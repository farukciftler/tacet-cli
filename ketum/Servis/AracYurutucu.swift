//
//  AracYurutucu.swift
//  ketum
//
//  Araç çipi ↔ tool eşlemesi (spec §7.4). Tek doğruluk kaynağı: aracın kendisi.
//  Tool çağrısı başlayınca "çalışıyor" çipi düşer; tool dönünce çip son
//  durumuna geçer. Çip metni araç tarafından üretilir, modele yazdırılmaz.
//

import Foundation
import Observation

/// Araçların çip yaşam döngüsünü bildirdiği arayüz. MainActor — UI durumu.
@MainActor
protocol AracRaporlayici: AnyObject, Sendable {
    /// "Çalışıyor" çipi düşürür, çip kimliğini döndürür.
    func baslat(ikon: String, metin: String) -> UUID
    /// Çipi son durumuna geçirir. Verilmeyen alanlar korunur.
    func guncelle(_ id: UUID, durum: AracDurumu, metin: String?, hamGirdi: String?, hamCikti: String?, dosyaYolu: String?)
}

/// Aktif turdaki araç çiplerini biriktiren yürütücü. ModelServisi sahibidir;
/// SohbetGorunumu canlı çipleri buradan gözlemler, tur bitince Mesaj'a taşınır.
@MainActor
@Observable
final class AracYurutucu: AracRaporlayici {
    /// Aktif turda düşen çipler, çağrı sırasına göre.
    private(set) var izler: [AracIzi] = []

    /// Bu turda "dünyayı değiştiren" (`.yazildi`) bir araç çalıştı mı — etkinlik
    /// yazıldı, hatırlatıcı kuruldu, belge üretildi/düzenlendi, nöbet kuruldu.
    ///
    /// YAPIŞKAN: hata kurtarma yolu retry'dan önce `yeniTur(yanEtkiyiUnut: false)`
    /// çağırıyor; bayrak orada sıfırlansaydı yan etkinin olup olmadığı bilgisi
    /// tam da ihtiyaç duyulduğu anda kaybolurdu. Yalnızca gerçek yeni tur
    /// (`yeniTur(yanEtkiyiUnut:)` varsayılanı) sıfırlar.
    private(set) var dunyaDegisti = false

    /// Bu turda UZAK (cihaz dışı) bir çağrı başarıyla döndü mü — yani
    /// kullanıcının kendi sunucusunda bir yan etki oluşmuş OLABİLİR.
    ///
    /// `dunyaDegisti`den AYRI bir eksendir ve ayrı olmak zorundadır: o bayrak
    /// yalnızca `.yazildi` çipiyle kurulur, uzak çağrı ise bilerek `.okundu`
    /// bırakılıyor (`MCPAraci.uzagaCagir`: `.yazildi` yerel geri alma/kurtarma
    /// mantığını tetikler ve uzak çağrı için yanlış olur). Sonuç: Jira issue'su
    /// açan bir çağrıdan SONRA oluşan genel bir hata `hataKurtar`ı retry'a
    /// sokuyor, AYNI istem ikinci kez gidiyor ve ikinci issue açılıyordu.
    ///
    /// `dunyaDegisti` gibi YAPIŞKAN: kurtarma yolu `yeniTur(yanEtkiyiUnut: false)`
    /// çağırdığında korunur, yalnız gerçek yeni tur sıfırlar.
    ///
    /// SINIF AYRIMI YAPILMAZ (salt-okuma/yazma). Sınıflandırma (`YanEtkiSinifi`)
    /// sunucunun ipuçlarına ve ad sezgisine dayanan bir TAHMİNDİR; "salt-okuma"
    /// sanılan bir uzak aracın kayıt tuttuğu, sayaç artırdığı, e-posta
    /// tetiklediği durumda geri alınamaz. Bir retry'ı fazladan engellemenin
    /// maliyeti bir hata balonu; kaçırmanın maliyeti çift dış yan etki.
    private(set) var disEtkiOlusabilir = false

    /// Uzak çağrı BAŞARIYLA döndükten sonra çağrılır. Tek yön; tur boyunca kalır.
    func disEtkiIsaretle() { disEtkiOlusabilir = true }

    /// Retry güvenli mi — `hataKurtar` tek karar noktası olarak bunu okur.
    /// İki eksen de temizse aynı istem ikinci kez gönderilebilir.
    var retryGuvenli: Bool { !dunyaDegisti && !disEtkiOlusabilir }

    // MARK: - Kirli oturum (mcp §5.6)

    /// Bu oturumda kişisel veri aracı (Takvim, Kişi, Arama/Spotlight, Belge*,
    /// Hatırlatıcı) en az bir kez BAŞARIYLA çalıştı mı.
    ///
    /// Oturum boyunca TEMİZLENMEZ ve `yeniTur()` bunu sıfırlamaz: bağlam
    /// özetlemesiyle yeni bir session açıldığında özetin kendisi kişisel veri
    /// taşıyabilir, dolayısıyla kirlilik de taşınır. Yalnızca gerçek sohbet
    /// sıfırlaması (`sohbetiSifirla()`) temizler.
    ///
    /// Modelin bu bayrağa erişimi yoktur; kapı kodda, modelde değil (mcp §2.2).
    private(set) var oturumKirli = false

    /// Kişisel veri aracının başarılı çağrısından sonra çağrılır. Tek yön.
    func kirlet() { oturumKirli = true }

    /// Bu oturumda kullanıcının paylaşımı reddettiği kaynaklar. Aynı kaynak
    /// için ikinci onay çipi üretilmez; sonraki denemeler sessizce aynı reddi
    /// alır — model ısrar döngüsüne giremez (mcp §3.3).
    private var reddedilenKaynaklar: Set<String> = []

    /// Şu an kullanıcı kararı bekleyen istek. Görünüm katmanı bunu gözler ve
    /// `OnaySayfasi`nı sunar; karar `onayKarariVer(_:)` ile geri gelir.
    private(set) var bekleyenOnay: OnayIstegi?

    /// Kararı bekleyen `onayIste` çağrısının devamı. Aynı anda en fazla bir tane.
    private var onayDevami: CheckedContinuation<Bool, Never>?

    /// Kullanıcıya sorulacak tek bir paylaşım isteği.
    struct OnayIstegi: Identifiable, Equatable {
        let id = UUID()
        /// Bağlantı/sunucu adı — "ev sunucusu", "arama sunucusu".
        let kaynak: String
        let aracAdi: String
        /// GÖNDERİLECEK ARGÜMANLARIN AYNISI. Kategori özeti değil.
        let icerik: String
        /// Bu isteğin akıştaki çipi.
        let izID: UUID
    }

    /// Cihaz dışına veri çıkarmadan önce çağrılır. `false` = gönderme.
    ///
    /// - Oturum kirli değilse SORMADAN `true` döner: onay nadirse okunur (mcp §2.4).
    /// - Kaynak bu oturumda bir kez reddedildiyse çip düşmeden `false` döner.
    /// - Aksi halde akışa "onay bekleniyor" çipi düşer ve kullanıcı kararı beklenir.
    ///
    /// - Parameter icerik: gönderilecek argümanların aynısı; kullanıcı onay
    ///   sayfasında birebir bunu görür.
    /// Cihaz verisi kapısı — çağrı yerlerinin çoğu bu kısa biçimi kullanır.
    func onayIste(kaynak: String, aracAdi: String, icerik: String) async -> Bool {
        await onayIste(kaynak: kaynak, aracAdi: aracAdi, icerik: icerik, zorunlu: false)
    }

    /// - Parameter zorunlu: true ise oturum temiz olsa da sorulur. Yıkıcı uzak
    ///   araçlar bunu kullanır: orada soru "cihazdan ne çıkıyor" değil,
    ///   "kullanıcının sunucusunda ne değişiyor" — kirlilikle ilgisiz.
    func onayIste(kaynak: String, aracAdi: String, icerik: String, zorunlu: Bool) async -> Bool {
        guard oturumKirli || zorunlu else { return true }
        if reddedilenKaynaklar.contains(kaynak) { return false }
        // Aynı anda ikinci bir kapı açmayız: üst üste binen istek sessizce
        // reddedilir, kullanıcı iki sheet arasında sıkışmaz.
        if bekleyenOnay != nil { return false }

        let izID = baslat(ikon: "hand.raised", metin: "\(kaynak) · onay bekleniyor")
        guncelle(izID, durum: .onayBekleniyor, hamGirdi: icerik)

        let kabul = await withCheckedContinuation { devam in
            onayDevami = devam
            bekleyenOnay = OnayIstegi(kaynak: kaynak, aracAdi: aracAdi,
                                      icerik: icerik, izID: izID)
        }

        if kabul {
            // Çip normal yaşam döngüsüne döner: aracın kendi çipi devralır,
            // bekleme çipi akışta iz bırakmaz.
            izler.removeAll { $0.id == izID }
        } else {
            reddedilenKaynaklar.insert(kaynak)
            guncelle(izID, durum: .gonderilmedi, metin: "\(kaynak) · gönderilmedi")
        }
        return kabul
    }

    /// Kullanıcının onay sayfasındaki kararı. Sayfayı kapatmak = `false`.
    func onayKarariVer(_ kabul: Bool) {
        guard let devam = onayDevami else { return }
        onayDevami = nil
        bekleyenOnay = nil
        devam.resume(returning: kabul)
    }

    /// Bekleyen bir onay varsa reddederek çözer — turun iptali askıda bir
    /// continuation bırakmasın (aksi halde araç sonsuza kadar askıda kalır).
    private func bekleyenOnayiCoz() {
        guard onayDevami != nil else { return }
        onayKarariVer(false)
    }

    // MARK: - Tur yaşam döngüsü

    /// Tur başına sıfırlanan dış sayaçların kancası (kod-spec §5.4: kod deneme
    /// sayacı `AracYurutucu.yeniTur`'da sıfırlanır). Sahibi (ModelServisi)
    /// kurar; jenerik tutulur — bu sınıf sayaç türlerini tanımaz. Kanca TEK
    /// noktadan tetiklendiği için `yeniTur` çağıran gelecekteki bir yol
    /// sayaç sıfırlamayı unutamaz.
    var turKancasi: (() -> Void)?

    /// Yeni tur — önceki turun çipleri Mesaj'a taşındıktan sonra sıfırlanır.
    ///
    /// `yanEtkiyiUnut: false` = "bu gerçek bir yeni tur DEĞİL, aynı turun
    /// kurtarma denemesi". O durumda ne yan etki bayrakları ne de ÇİPLER
    /// sıfırlanır (denetim P2-8).
    ///
    /// Çiplerin de korunması bilinçli bir geri dönüştür: eski davranış
    /// `izler = []`i koşulsuz yapıyordu ve kurtarma sonrası kullanıcı ilk
    /// denemede hangi aracın çalıştığını göremiyordu — tam da hata teşhisinin
    /// en çok ihtiyaç duyduğu bilgi, hata anında siliniyordu. İki deneme aynı
    /// aracı çalıştırırsa iki çip görünür; bu bir kusur değil, olan bitenin
    /// kendisidir ("bir kez denendi, hata aldı, yeniden denendi").
    ///
    /// Kirli oturum bayrağını ve ret önbelleğini BİLEREK sıfırlamaz — ikisi de
    /// oturum ömürlüdür (mcp §5.6, §3.3).
    func yeniTur(yanEtkiyiUnut: Bool = true) {
        bekleyenOnayiCoz()
        if yanEtkiyiUnut {
            izler = []
            dunyaDegisti = false
            disEtkiOlusabilir = false
        }
        turKancasi?()
    }

    /// Gerçek sohbet sıfırlaması: oturum ömürlü ne varsa burada biter —
    /// kirlilik ve ret önbelleği dahil. Yeni sohbet temiz başlar.
    func sohbetiSifirla() {
        yeniTur()
        oturumKirli = false
        reddedilenKaynaklar.removeAll()
    }

    func baslat(ikon: String, metin: String) -> UUID {
        let iz = AracIzi(ikon: ikon, metin: metin, durum: .calisiyor)
        izler.append(iz)
        return iz.id
    }

    func guncelle(_ id: UUID,
                  durum: AracDurumu,
                  metin: String? = nil,
                  hamGirdi: String? = nil,
                  hamCikti: String? = nil,
                  dosyaYolu: String? = nil) {
        if durum == .yazildi { dunyaDegisti = true }
        guard let i = izler.firstIndex(where: { $0.id == id }) else { return }
        izler[i].durum = durum
        if let metin { izler[i].metin = metin }
        if let hamGirdi { izler[i].hamGirdi = hamGirdi }
        if let hamCikti { izler[i].hamCikti = hamCikti }
        if let dosyaYolu { izler[i].dosyaYolu = dosyaYolu }
    }
}
