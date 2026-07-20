//
//  ModelServisi.swift
//  ketum
//
//  Model katmanı (spec §7.1, §7.2). Yalnızca on-device SystemLanguageModel;
//  Private Cloud Compute kullanılmaz. Availability bir feature flag gibi ele
//  alınır. Bağlam penceresi (4096) aktif yönetilir; taşma sessizce kurtarılır.
//

import Foundation
import Observation
import FoundationModels
import NaturalLanguage
import SwiftData

@MainActor
@Observable
final class ModelServisi {

    /// Oturumun durumu. Ekranda renkli gösterge yoktur; durum yalnızca sözle anlatılır.
    enum Durum: Equatable {
        case hazir                    // sessiz — anlatılacak bir şey yok
        case hazirlaniyor             // gri · "hazırlanıyor…"
        case kullanilamaz(String)     // gri · neden

        var etiket: String {
            switch self {
            case .hazir: return String(localized: "Cihazında")
            case .hazirlaniyor: return String(localized: "hazırlanıyor…")
            case .kullanilamaz(let neden):
                return neden.isEmpty ? String(localized: "bu cihazda kullanılamıyor") : neden
            }
        }
        var hazirMi: Bool { self == .hazir }
    }

    /// Neden yanıt üretemiyoruz — kullanıcıya "ne oldu + ne yapmalı" demek için tutulur.
    /// `Durum.etiket` kısa rozet metnidir; bu ise sohbete düşen tam cümleyi seçer.
    enum Engel { case cihaz, kapali, hazirlaniyor }

    /// Bir turun sonucu. Hata olup olmadığı METİNDEN ÇIKARILMAZ — servis açıkça
    /// bildirir. (Eskiden UI, dönen metni bilinen hata dizgileriyle karşılaştırıyordu;
    /// metin her değiştiğinde hata balonu sessizce ölüyordu.)
    /// Turun NEDEN düştüğü. Tek bir "Şu an bunu yapamadım" cümlesi üç ayrı
    /// arızayı örtüyordu (ölçüm: 5 vaka, hepsi aynı metin, hiçbiri aynı sebep).
    /// Sınıf iki işi birden görür: kullanıcıya doğru cümleyi seçer (`Yerel`)
    /// ve eval ham JSON'una yazılır — bir sonraki teşhis log eklemeden yapılır.
    ///
    /// İÇ AYRINTI SIZDIRMAZ: sınıf ADI loglanır, kullanıcı yalnız ona karşılık
    /// gelen sade cümleyi görür (hata metni, satır no, model adı geçmez).
    enum HataSinifi: String, Codable, Sendable {
        case yok
        /// Model konuştu ama geriye yalnız ayrıştırılamamış araç çağrısı kaldı.
        case bosYanit
        /// Turda bir araç düştü ve model üstüne söyleyecek metin üretmedi.
        case aracDustu
        /// Bağlam penceresi taştı; özetleyip yeniden kurma da tutmadı.
        case baglamTasmasi
        /// Guardrail — kurtarılamaz, retry yok.
        case sinirDisi
        /// Dil desteklenmiyor — kurtarılamaz, retry yok.
        case dilDisi
        /// Yan etki oluştuktan sonraki hata; retry bilerek yapılmadı.
        case yazmaSonrasi
        /// Diğer üretim hataları; taze oturumla retry da tutmadı.
        case uretimHatasi
    }

    struct YanitSonucu {
        let metin: String
        let izler: [AracIzi]
        /// Hata balonu olarak çizilsin mi. İPTAL HATA DEĞİLDİR (false).
        var hataMi: Bool = false
        /// Aynı istem güvenle tekrar gönderilebilir mi. Yan etki oluşmuşsa false.
        var tekrarDenenebilir: Bool = false
        /// Kalıcı değil, yalnızca anlık durum bildirimi — SwiftData'ya yazılmamalı.
        var geciciMi: Bool = false
        /// Hata sınıfı — `hataMi` false ise daima `.yok`.
        var hataSinifi: HataSinifi = .yok
    }

    private(set) var durum: Durum = .hazirlaniyor
    private(set) var engel: Engel? = .hazirlaniyor

    /// Model kullanılamazken sohbete düşecek açıklama. Duruma göre değişir ki
    /// "hazırlanıyor" ile "bu cihazda hazır değil" mesajları çelişmesin.
    var engelMesaji: String {
        switch engel {
        case .hazirlaniyor: return Yerel.modelHazirlaniyor
        case .kapali:       return Yerel.appleIntelligenceKapali
        case .cihaz, nil:   return Yerel.cihazUygunDegil
        }
    }
    /// Araç çipi tek doğruluk kaynağı — tools buraya rapor eder, UI buradan okur.
    let yurutucu = AracYurutucu()
    /// Engel geçici mi (bekleyip yeniden denemek anlamlı mı) — cihaz uygun değilse değil.
    var engelTekrarDenenebilir: Bool {
        switch engel {
        case .hazirlaniyor, .kapali: return true
        case .cihaz, nil:            return false
        }
    }
    /// Sohbete paylaşılan/üretilen belgeler — belge araçları buraya erişir, UI önizler.
    let belgeBaglami = BelgeBaglami()
    /// Büyük veri taşıma kanalı (spec §7.3.2) — toplu veri modelden geçmeden araçlar arası taşınır.
    let veriDeposu = VeriDeposu()
    /// Nöbet (zamanlanmış ajan) kurma bağlamı — NobetAraci buraya erişir.
    let nobetBaglami = NobetBaglami()
    /// Kod çalıştırma deneme sayacı (kod-spec §5.4): tur başına en fazla 2
    /// gerçek çalıştırma. Sayaç araçta değil burada yaşar ve `init` içinde
    /// `yurutucu.turKancasi`na bağlanır — sıfırlama spec'in dediği yerde,
    /// AracYurutucu.yeniTur içinde TEK noktadan olur; çağrı noktalarında elle
    /// eşleme yoktur, unutulan bir yol tavanı oturum ömürlü yapamaz.
    let kodDurumu = KodDurumu()

    /// Turun seyri (seyir-spec §5.2). SALT GÖZLEMCİ: buradaki hiçbir metin
    /// isteme ya da talimata girmez, modelden hiçbir durum bildirimi istenmez.
    /// Kaydedicinin varlığıyla yokluğu arasında model çıktısı bit düzeyinde
    /// aynıdır (§6 kabul ölçütü) — bu yüzden tüm çağrılar tek yönlü bildirimdir.
    let seyir = SeyirKaydedici()

    /// Sohbet sıfırlandığı anda hafıza ayıklamasını tetikleyen kanca
    /// (hafiza-spec §4.1). `ModelServisi`nin ne `Sohbet` ne `ModelContext`
    /// erişimi vardır; ikisini de bilen katman (ContentView) bunu bağlar.
    var hafizaTetigi: (() -> Void)?

    /// Bağlantı profilinde oturuma girecek MCP araçları (mcp §5.4).
    ///
    /// Burada kurulmazlar: MCP aracının şeması sunucudan çalışma anında gelir
    /// ve `oturumKur` senkrondur. Bağlantıyı bilen katman hazır araçları
    /// `baglantiAraclariniAyarla` ile verir; boşsa profil hiç seçilmez.
    private var mcpAraclari: [MCPAraci] = []
    /// Seçili bağlantının adı — yönlendirme sinyali ve seyir satırı için.
    private var baglantiAdi: String = ""

    /// Seçili bağlantının araçlarını oturuma hazırlar. Boş dizi = bağlantı yok;
    /// bağlantı profili o an seçilemez hâle gelir (araç modele hiç görünmez).
    ///
    /// Aktif oturum bağlantı profilindeyse liste değiştiğinde oturum
    /// geçersizleşir: bir sonraki turda yeni araç setiyle yeniden kurulur.
    func baglantiAraclariniAyarla(_ araclar: [MCPAraci], ad: String = "") {
        mcpAraclari = araclar
        baglantiAdi = ad
        if aktifProfil == .baglanti { oturum = nil }
    }

    /// En fazla kaç MCP aracı oturuma girer (mcp §5.4: "gerekirse ilk 4–6").
    /// Hesap + Zaman ile birlikte 6–8 bütçesi korunur.
    private static let mcpAracTavani = 6

    /// Alaka sıralamasının içinden seçtiği havuz. Yuvadan geniş, ama şema
    /// çevirisi bedava olmadığı için sınırsız değil (mcp §5.2).
    private static let mcpAracHavuzu = 24

    /// Üretim seçenekleri (denetim P0-5'in eksik yarısı).
    ///
    /// ÜRETİMDE VARSAYILAN DEĞİŞMEZ: `GenerationOptions()` bugünkü davranışın
    /// ta kendisidir; buraya dokunmak kullanıcının gördüğü yanıtları değiştirir
    /// ve bu ayrı bir karardır. Değişken YALNIZCA eval için vardır.
    ///
    /// NEDEN GEREKLİ (ölçüldü, 20 Temmuz 2026): aynı ikiliyle kod değişmeden
    /// iki koşum arasında vakaların %27'si puan değiştirdi, değişenlerde
    /// ortalama oynama 21.8 puan, kategori ortalamaları kendiliğinden ±15 puan
    /// kaydı. Bu gürültü tabanında bir düzeltmenin işe yarayıp yaramadığı tek
    /// koşumla SÖYLENEMEZ — her "iyileştirme" iddiası zayıf kalır.
    ///
    /// `.greedy` sıcaklık 0'dan daha kesindir: sıcaklık 0 hâlâ örnekleme
    /// yapabilir, greedy her adımda en olası jetonu alır.
    nonisolated(unsafe) static var uretimSecenekleri = GenerationOptions()

    /// Eval koşumları için örneklemeyi kapatır. Yalnız DEBUG yollarından
    /// çağrılır; üretim ikilisinde çağıran yoktur.
    static func orneklemeyiKapat() {
        uretimSecenekleri = GenerationOptions(sampling: .greedy)
    }

    /// Bu turun ham istemi — yuva alaka sıralamasının tek girdisi (P1-6).
    /// Oturum kurulumu senkron olduğu için soru bir alan üzerinden taşınır.
    private var sonSoru: String = ""

    /// Yuvaya girecek MCP araçları: havuzdan, kullanıcının o turki isteğine
    /// ALAKAYA göre seçilir (P1-6). Eski davranış `prefix(tavan)` idi, yani
    /// yuvalar sunucunun `tools/list` sırasına göre KÖR doluyordu; 20 araçlı
    /// bir sunucuda "issue aç" isteği, `issue_olustur` 14. sıradaysa masaya
    /// hiç gelmiyordu.
    ///
    /// `sirala` kararlıdır: soru hiçbir sinyal taşımıyorsa sonuç eski kör
    /// prefix'in AYNISI — bu bir gerileme güvencesidir.
    private func secilenMCPAraclari() -> [MCPAraci] {
        Array(AracAlaka.sirala(mcpAraclari, soru: sonSoru,
                               ad: \.uzakAd, ozet: \.ozet)
            .prefix(Self.mcpAracTavani))
    }

    // MARK: - Bağlantı köprüsü (mcp §5.4 — fişe takma)

    /// Uzak çağrı yolu ve istemci sahipliği. `MCPAraci` yalnızca bu sözleşmeyi
    /// görür; ağ kodu hâlâ tek yerde (`MCPIstemcisi`).
    let baglantiKopru = MCPAracKoprusu()

    /// Araçları hangi bağlantı listesinden kurduğumuzun imzası. Aynı listede
    /// ikinci kez ağa çıkmayı önler; liste (ad/adres/araç adları) değişirse
    /// tazelenir.
    private var baglantiImzasi = ""
    /// Süren araç kurma görevi — liste değişince eskisi iptal edilir.
    private var baglantiGorevi: Task<Void, Never>?

    /// Kayıtlı bağlantılardan MCP araçlarını kurup oturuma besler (mcp §5.4).
    ///
    /// Şema sunucudan çalışma anında geldiği için kurulum ASENKRONDUR: araçlar
    /// hazır olana kadar `mcpAraclari` boştur, yani bağlantı profili seçilemez ve
    /// bugünkü davranış aynen sürer. Hiç bağlantı yoksa ağa hiç çıkılmaz (§2.1).
    ///
    /// Seçili bağlantı: en son kullanılan, hiç kullanılmadıysa en son eklenen.
    /// v1'de ayrı bir seçim yüzeyi yok; 4096 pencerede aynı anda tek sunucunun
    /// araçları taşınabildiği için tek bağlantı seçilir.
    func baglantilariTazele(_ baglantilar: [Baglanti]) {
        let canli = baglantilar.filter { !$0.isDeleted && $0.gecerliMi }
        let secili = canli
            .sorted { ($0.sonKullanim ?? $0.olusturulma) > ($1.sonKullanim ?? $1.olusturulma) }
            .first { !$0.kullanilabilirAraclar.isEmpty }

        guard let secili, let url = secili.url else {
            // Bağlantı yok (ya da hiçbirinin desteklenen aracı yok): profil hiç
            // seçilemez hâle döner, istemciler bırakılır.
            baglantiGorevi?.cancel()
            baglantiGorevi = nil
            baglantiImzasi = ""
            baglantiKopru.unut()
            baglantiAraclariniAyarla([], ad: "")
            return
        }

        // SwiftData tuzağı: nesneye await'ten ÖNCE dokunulur.
        let kimlik = secili.id
        let ad = secili.ad
        let cihazVerisi = secili.cihazVerisi
        // Desteklenmeyen şemalı araçlar zaten burada eleniyor (§5.2).
        let ozetler = secili.kullanilabilirAraclar
        let anahtar = secili.anahtarRefi.flatMap { AnahtarKasasi.oku(ref: $0) }
        // İmza cihazVerisi'ni İÇERİR: kullanıcı ayarı BaglantiDetayi'nde
        // değiştirdiğinde araçlar yeniden kurulsun, yeni ayar oturum boyunca
        // beklemesin.
        let imza = "\(kimlik)|\(ad)|\(url.absoluteString)|\(cihazVerisi.rawValue)|\(ozetler.map(\.ad).joined(separator: ","))"
        guard imza != baglantiImzasi else { return }
        baglantiImzasi = imza
        baglantiKopru.kaydet(kimlik: kimlik, url: url, anahtar: anahtar)

        baglantiGorevi?.cancel()
        baglantiGorevi = Task { [weak self] in
            guard let self else { return }
            let araclar = await baglantiKopru.araclariKur(
                baglantiID: kimlik, ad: ad, ozetler: ozetler,
                havuz: Self.mcpAracHavuzu, cihazVerisi: cihazVerisi,
                kapi: yurutucu, raporlayici: yurutucu)
            guard !Task.isCancelled, baglantiImzasi == imza else { return }
            // Sunucuya erişilemediyse imzayı düşür: bir sonraki tazelemede
            // yeniden denensin, kullanıcı ağ dönünce uygulamayı yeniden
            // başlatmak zorunda kalmasın.
            if araclar.isEmpty { baglantiImzasi = "" }
            baglantiAraclariniAyarla(araclar, ad: ad)
        }
    }

    /// Araç profili (spec §7.3.1): 4096 pencerede oturuma en fazla 6–8 araç verilir.
    ///
    /// `arama` ve `baglanti` cihaz DIŞINA çıkan profillerdir ve kişisel veri
    /// araçlarını BİLEREK içermez (web-arama §5.4, mcp §5.4): modelin bir
    /// argümana kişisel veri yazması ihtimaline karşı yapısal savunma —
    /// araç oturumda yoksa veri de çıkamaz.
    enum Profil {
        case gundelik, belge, arama, baglanti

        /// Seyir satırında görünen ad. Profil, kullanıcının göremediği bir iç
        /// karar değil — hangi araçların masada olduğunu belirler, o yüzden
        /// anlatılır.
        var seyirAdi: String {
            switch self {
            case .gundelik: return String(localized: "gündelik profil")
            case .belge:    return String(localized: "belge profili")
            case .arama:    return String(localized: "arama profili")
            case .baglanti: return String(localized: "bağlantı profili")
            }
        }
    }
    private var aktifProfil: Profil = .gundelik
    /// Mevcut oturumun araç setinin imzası. PROFİL ADI YETMEZ: gündelik setin
    /// içeriği tur içinde değişiyor (Kişi ↔ web takası, Spotlight'ın gizlenmesi)
    /// ve yeniden kurma koşulu yalnızca `istenen != aktifProfil`e baksaydı bu
    /// takas ilk turdan sonra HİÇ uygulanmazdı — mekanizma tanımlı olur ama
    /// çalışmazdı. Ölçülen yol: 1. tur "hatırlatıcı kur" (set web'li kurulur),
    /// 2. tur "annemin telefon numarası" → profil hâlâ .gundelik olduğu için
    /// oturum korunuyor ve KisiAraci oturuma hiç girmiyordu.
    private var oturumAracImzasi = ""
    /// Oturuma gömülü dil adı (yanıt-dili çapası). Kullanıcının dili saptanınca güncellenir.
    private var aktifDil: String = ""
    /// Mevcut oturumun kurulduğu dil — gereksiz yeniden kurmayı önler.
    private var oturumDili: String = ""
    /// Aktif dil kullanıcının açık seçiminden mi geliyor (saptamadan değil) — talimat metnini değiştirir.
    private var dilSecildi: Bool = false
    /// Oturumun kurulduğu andaki seçim durumu — talimat metni buna göre yazıldığı için izlenir.
    private var oturumDilSecildi: Bool = false


    private let model = SystemLanguageModel.default
    private var oturum: LanguageModelSession?

    /// Bağlam bütçesi eşiği: contextSize'ın %80'i (araştırma raporu §5.2).
    private let esikOran = 0.80

    init() {
        // Uzak çıktı da 4096 bütçesine göre işlenir (§5.5): büyük sonuç modelden
        // geçmeden veri deposuna konur, modele özet + kaynakRef gider.
        baglantiKopru.veriDeposu = veriDeposu
        // P0-3: uzak çağrı başarıyla dönünce retry kapısını kapatan bayrağı
        // kuracak olan taraf. Köprü TEK huni: her uzak çağrı buradan geçer,
        // dolayısıyla yeni bir MCP çağrı yolu eklenirse bayrağı kurmayı
        // unutmak mümkün değil.
        baglantiKopru.yurutucu = yurutucu
        // Kod deneme sayacı tur yaşam döngüsüne buradan bağlanır (kod-spec
        // §5.4: sıfırlama AracYurutucu.yeniTur'dadır). sohbetiSifirla da
        // yeniTur'u içeriden çağırdığı için tüm yollar tek kancadan geçer.
        yurutucu.turKancasi = { [kodDurumu] in kodDurumu.yeniTur() }
        availabilityKontrol()
    }

    // MARK: - Availability

    func availabilityKontrol() {
        let oncekiHazir = durum.hazirMi
        switch model.availability {
        case .available:
            durum = .hazir
            engel = nil
            // Zaten hazırdıysak oturuma DOKUNMA: yeniden kurmak transcript'i (yani
            // sohbetin bağlamını) sessizce silerdi ve `sohbetiSifirla`nın tembel
            // kurulumunu bozardı. Yalnızca hazır-değil → hazır geçişinde kur.
            if !oncekiHazir { oturumKur(profil: aktifProfil) }
        case .unavailable(let neden):
            switch neden {
            case .deviceNotEligible:
                durum = .kullanilamaz(String(localized: "bu cihazda kullanılamıyor"))
                engel = .cihaz
            case .appleIntelligenceNotEnabled:
                durum = .kullanilamaz(String(localized: "Apple Intelligence kapalı"))
                engel = .kapali
            case .modelNotReady:
                durum = .hazirlaniyor
                engel = .hazirlaniyor
            @unknown default:
                durum = .kullanilamaz(String(localized: "bu cihazda kullanılamıyor"))
                engel = .cihaz
            }
        }
    }

    /// Availability'yi yeniden okur. Model indirmesi biten ya da Apple Intelligence
    /// sonradan açılan cihazda uygulamayı yeniden başlatmadan asistanı açar.
    /// Sahneye dönüşte (UstBar/ContentView) ve her istek başında çağrılır.
    /// Zaten hazırsa hiçbir şeyi sıfırlamaz — güvenle sık çağrılabilir.
    func availabilityYenile() { availabilityKontrol() }

    // MARK: - Oturum ve profiller

    /// Gündelik profil (spec §8, v1): Takvim, Hatırlatıcı, Kişi, Arama, Hesap,
    /// Zaman, Nöbet + Kod (kod-spec §7). 8 araçla tavan ZORLANIYOR — cihaz
    /// ölçümü kötü çıkarsa `zaman` düşürülür ya da kod niyeti ayrı profil olur
    /// (kod-spec §9 açık soru 2).
    private func gundelikAraclar() -> [any Tool] {
        var takvim = TakvimAraci();          takvim.raporlayici = yurutucu; takvim.veriDeposu = veriDeposu
        var hatirlatici = HatirlaticiAraci(); hatirlatici.raporlayici = yurutucu; hatirlatici.veriDeposu = veriDeposu
        var hesap = HesapAraci();            hesap.raporlayici = yurutucu
        var zaman = ZamanAraci();            zaman.raporlayici = yurutucu
        var nobet = NobetAraci();            nobet.raporlayici = yurutucu; nobet.baglam = nobetBaglami
        var kod = KodCalistirAraci();        kod.raporlayici = yurutucu; kod.durum = kodDurumu
        var araclar: [any Tool] = [takvim, hatirlatici, hesap, zaman, nobet, kod]

        // KİŞİ ↔ WEB ARAMASI TAKASI. İkisi aynı sette DURMAZ ve bu iki sebepten:
        //
        // 1) Bütçe: 6–8 araç tavanı (spec §7.3). İkisi birden 9 eder.
        // 2) Güvenlik: web-arama §5.4 kişisel veri araçlarını web ile aynı
        //    profilde istemez — model rehberden okuduğunu sorguya yazabilir.
        //
        // Hangisinin gireceğini SORU belirler: cümlede rehber sinyali varsa
        // Kişi, yoksa web araması. Böylece "annemin numarası" da çalışır,
        // "namaz vakitleri" de — ve ikisi asla yan yana gelmez.
        if kisiSinyaliVar {
            var kisi = KisiAraci(); kisi.raporlayici = yurutucu
            araclar.insert(kisi, at: 2)
        } else if aramaKullanilabilir {
            var web = WebAramaAraci()
            web.raporlayici = yurutucu; web.yurutucu = yurutucu; web.veriDeposu = veriDeposu
            araclar.insert(web, at: 2)
        }

        // Spotlight araması, kullanıcı AÇIKÇA web istediyse ve web araması
        // kapalıysa oturuma HİÇ GİRMEZ.
        //
        // Ölçülen hata: "internette ara" dendiğinde model `not_arama`yı yedek
        // olarak çağırıyor, sonuç boş dönüyor ve yanıt "internette arama
        // yapıldı, cihazda bulunamadı" oluyordu — YAPILMAMIŞ bir işi yaptım
        // demek, üretebileceği en kötü çıktı. Talimata ve beceri gövdesine
        // yazılan yasak İKİ KEZ denendi ve tutmadı; 3B modelde yasak metni
        // davranışı kontrol etmiyor. Yeteneği elinden almak kontrol ediyor.
        if !yerelAramayiGizle {
            var arama = AramaAraci(); arama.raporlayici = yurutucu
            araclar.insert(arama, at: 3)
        }
        return araclar
    }

    /// Bu turda Spotlight araması gizlensin mi — `niyetProfili` her turda tazeler.
    private var yerelAramayiGizle = false
    /// Bu turda rehber sorusu mu — Kişi ↔ web araması takasını belirler.
    private var kisiSinyaliVar = false

    /// Rehber niyeti. Web araması gündelik sete girdiği için gerekli: hangisinin
    /// oturuma alınacağına bu karar verir.
    private static let kisiIzleri = [
        "kişi", "kisi", "rehber", "numara", "telefonu", "telefon numar",
        "mail adresi", "e-posta adres", "eposta adres", "adresi ne",
        "contact", "phone number", "email address", "in numarası",
    ]

    /// Belge kilidinden kaçış izleri (denetim P2-1). Rehber ve açık web ayrı
    /// hesaplanır; burası yalnızca belge setinde HİÇ karşılığı olmayan iki
    /// yetenek: nöbet (zamanlanmış ajan) ve kod çalıştırma.
    ///
    /// Liste bilerek çok DAR ve hep ÇOK SÖZCÜKLÜ/ayırt edici: kaçışın yanlış
    /// pozitifi, kullanıcının ekli belgesi üzerinde çalışmayı reddetmek demek
    /// — kilidin kendisinden daha kötü bir arıza. Çıplak "kod" yok ("kodu",
    /// "barkod", "kodlanmış" içinde geçer); çıplak "ajan" yok.
    private static let kilitKacisIzleri = [
        "nöbet", "nobet kur", "her sabah", "her akşam",                      // tr — nobet_kur
        "kod çalıştır", "kodu çalıştır", "javascript", "js ile hesapla",     // tr — kod_calistir
        "run this code", "execute this code", "in javascript",               // en
    ]

    /// Belge/üretim profili (spec §7.3.1): Oluştur, Oku, Düzenle + veri kaynağı ve yardımcılar.
    ///
    /// Bir belge ekliyken `niyetProfili` profili .belge'ye KİLİTLER; bu yüzden bu sette
    /// olmayan bir araç kullanıcı için tamamen erişilemez hâle gelir. "Bunu yarın
    /// hatırlat" / "notlarımda ara" belge ekliyken en sık gelen iki istek olduğu için
    /// Hatırlatıcı ve Arama buraya alındı. 6–8 araç bütçesine sığmak adına KisiAraci
    /// çıkarıldı: belge üretiminde kişi araması en az kullanılan yol (belge içeriği
    /// zaten ekli belgeden ya da takvim/veri deposundan geliyor), ayrıca Kişiler izni
    /// gerektirdiği için çoğu turda zaten sonuçsuz dönüyordu.
    private func belgeAraclar() -> [any Tool] {
        var olustur = BelgeOlusturAraci(); olustur.raporlayici = yurutucu; olustur.baglam = belgeBaglami; olustur.veriDeposu = veriDeposu
        var oku = BelgeOkuAraci();         oku.raporlayici = yurutucu;      oku.baglam = belgeBaglami
        var duzenle = BelgeDuzenleAraci(); duzenle.raporlayici = yurutucu;  duzenle.baglam = belgeBaglami
        var takvim = TakvimAraci();        takvim.raporlayici = yurutucu;   takvim.veriDeposu = veriDeposu
        var hatirlatici = HatirlaticiAraci(); hatirlatici.raporlayici = yurutucu; hatirlatici.veriDeposu = veriDeposu
        var arama = AramaAraci();          arama.raporlayici = yurutucu
        var hesap = HesapAraci();          hesap.raporlayici = yurutucu
        var zaman = ZamanAraci();          zaman.raporlayici = yurutucu
        return [olustur, oku, duzenle, takvim, hatirlatici, arama, hesap, zaman]
    }

    /// Arama profili (web-arama §5.4): web_arama + Zaman. HESAP YOK.
    ///
    /// Takvim/Kişi/Arama(Spotlight)/Belge/Hatırlatıcı BİLEREK YOK. Model
    /// sorguyu kendi üretir; kişisel veri aracı aynı oturumda dururken
    /// "notlarımdaki adresi ara" tek adımda dışarı sızabilirdi. Kişisel veri
    /// gerektiren karma iş iki turda akar ve ikinci tur oturumu kirletmiş
    /// olduğu için onay kapısına düşer.
    ///
    /// HESAP ARACI ÇIKARILDI (denetim küme 1 — canlı veri uydurması). Ölçülen
    /// üç vaka, bu profilde `hesapla`nın tek işlevinin UYDURMAYA MEŞRU KILIF
    /// olduğunu gösterdi — model arayıp bulamadığı canlı değeri kendi kafasından
    /// atıp aritmetiği araca yaptırıyor, çıkan sayıyı "araçtan geldi" diye
    /// sunuyordu:
    ///
    ///   web-euro    → hesapla("(1.00 / 0.85) * 100") → "Euro 117,6471 TL"
    ///   web-benzin  → hesapla("(1.60 * 1.20)")       → "Benzin 1.92 TL"
    ///   web-lig     → hesapla("(139+30)*1.20")       → "202.8 puanla lider Beşiktaş"
    ///
    /// Üçünde de girdi sayıları HİÇBİR arama sonucundan gelmiyor; uydurma zaten
    /// `ifade` alanında olup bitiyor. Araç doğru çalışıyor, sonuç yine de yalan.
    /// Bunu talimatla kapatmak modelin iyi niyetine bel bağlamaktır (aynı ders
    /// `yerelAramayiGizle`de iki kez alındı: 3B modelde yasak metni davranışı
    /// kontrol etmiyor, yeteneği elinden almak kontrol ediyor).
    ///
    /// BEDELİ ve neden kabul edilebilir olduğu: "100 dolar kaç TL" gibi zincir
    /// iki tura yayılır — kur bu turda web'den gelir, çarpma sonraki turda
    /// gündelik profilde `hesapla` ile yapılır. `niyetProfili`ndeki HESAP KAÇIŞI
    /// o ikinci turu gündeliğe yönlendirir, yani zincir kopmaz, yalnız uzar.
    /// Aritmetiğin bir tur gecikmesi, uydurulmuş bir kurun anında sunulmasından
    /// her koşulda iyidir.
    private func aramaAraclar() -> [any Tool] {
        var web = WebAramaAraci(); web.raporlayici = yurutucu; web.yurutucu = yurutucu; web.veriDeposu = veriDeposu
        var zaman = ZamanAraci();  zaman.raporlayici = yurutucu
        return [web, zaman]
    }

    /// Bağlantı profili (mcp §5.4): seçili bağlantının araçları + Hesap + Zaman.
    /// Kişisel veri araçları arama profilindeki gerekçenin aynısıyla yoktur.
    private func baglantiAraclar() -> [any Tool] {
        var hesap = HesapAraci(); hesap.raporlayici = yurutucu
        var zaman = ZamanAraci(); zaman.raporlayici = yurutucu
        return secilenMCPAraclari().map { $0 as any Tool } + [hesap, zaman]
    }

    /// Kurulacak araç setini tek dizgede özetler. Yalnızca seti GERÇEKTEN
    /// değiştiren girdiler yazılır; aksi halde her turda gereksiz yeniden
    /// kurma (ve bir özetleme turu) maliyeti çıkardı.
    private func aracImzasi(_ profil: Profil) -> String {
        switch profil {
        case .gundelik:
            let takas = kisiSinyaliVar ? "kisi" : (aramaKullanilabilir ? "web" : "yok")
            return "gundelik|\(takas)|\(yerelAramayiGizle ? "spotsuz" : "spot")"
        case .belge:    return "belge"
        case .arama:    return "arama"
        // SAYI YETMEZ: alaka sıralaması turdan tura FARKLI altılıyı seçebilir,
        // sayı ise aynı kalır. İmza seçilen ADLARI yazar; aksi halde mekanizma
        // tanımlı olur ama ilk turdan sonra oturum hiç yeniden kurulmadığı için
        // hiç çalışmazdı (gündelik setteki Kişi ↔ web takasının aynı tuzağı).
        case .baglanti: return "baglanti|" + secilenMCPAraclari().map(\.name).joined(separator: ",")
        }
    }

    private func araclariYap(_ profil: Profil) -> [any Tool] {
        switch profil {
        case .gundelik:  return gundelikAraclar()
        case .belge:     return belgeAraclar()
        case .arama:     return aramaAraclar()
        case .baglanti:  return baglantiAraclar()
        }
    }

    // MARK: - Beceriler (progressive disclosure)

    /// Beceri kılavuzları oturum TALİMATINA gömülmez: ölçümler, küçük on-device
    /// modelin fazla sabit talimatla araçları çağırmak yerine "anlatmaya" başladığını
    /// gösterdi. Bunun yerine Claude'un SKILL.md mantığı uygulanır — yalnızca o
    /// mesaja uyan TEK beceri, o oturuma BİR KEZ, o turun istemine iliştirilir.
    /// Böylece hem sabit talimat kısa kalır hem rehberlik gerektiği anda gelir.
    /// Beceri enjeksiyonunun MESAFELİ durumu (denetim P2-2). Eski `Set<String>`
    /// "bir kez enjekte edildi, bir daha asla" demekti; kılavuz uzun turda
    /// pencereden kayınca geri gelmiyordu. Mantık `BeceriDeposu`da, durum burada.
    private var beceriDurumu = BeceriDeposu.EnjeksiyonDurumu()

    /// Bu oturuma hâlihazırda enjekte edilmiş hafıza notları (hafiza-spec §5.1).
    /// `enjekteBeceriler` simetriği: aynı not aynı oturuma bir kez girer, oturum
    /// yeniden kurulunca (transcript sıfırlanınca) temizlenir.
    private var enjekteNotlar: Set<UUID> = []

    /// Beceri kılavuzu + hafıza notları AYNI YERDE, o turun isteminin başına
    /// (hafiza-spec §5.1). Talimat sistemine gömülmezler: sabit talimat kısa kalır.
    ///
    /// İkisi aynı tura denk gelirse ikisi birden girer — beceri 700, hafıza 600
    /// karakter tavanlı, toplam ~1500 karakter. Bu, 4096 pencerede bilinçli
    /// kabul edilmiş en kötü hâldir.
    ///
    /// Sıra: önce hafıza (kullanıcı hakkında olgular), sonra beceri (nasıl
    /// yapılacağı), en sonda sorunun kendisi — soru istemin SONUNDA kalır,
    /// küçük modelde son bloğun ağırlığı en yüksektir.
    /// Mevcut oturumdaki araç adları. Beceri kapısının TEK doğruluk kaynağı
    /// (denetim P1-4): kılavuz, emrettiği araç oturumda YOKSA enjekte edilmez —
    /// aksi halde model var olmayan bir aracı çağırmaya yönlendirilirdi
    /// (belge kilidindeyken "kodla…" isteği kod.md'yi tetikliyor ama
    /// `kod_calistir` belge setinde yok).
    ///
    /// Eskiden burada elle yazılmış bir `[beceri: Set<Profil>]` haritası vardı.
    /// O harita ARAÇ SETİNİN KOPYASIYDI ve kopya olduğu için sessizce
    /// eskiyordu: bir araç profiller arasında taşındığında (Hatırlatıcı ve
    /// Arama'nın belge setine alınması gibi) haritayı güncellemeyi unutmak
    /// hiçbir derleme hatası vermiyordu. Artık kapı, oturumun GERÇEK araç
    /// adlarından okunuyor; harita ile set arasında sapma kavramsal olarak
    /// imkânsız.
    private var oturumAracAdlari: Set<String> = []

    private func istemZenginlestir(_ soru: String) -> String {
        var bloklar: [String] = []

        let notlar = HafizaDeposu.eslesen(soru: soru).filter { !enjekteNotlar.contains($0.id) }
        if !notlar.isEmpty {
            let metin = HafizaDeposu.enjeksiyonMetni(notlar)
            // Tavana hiçbir not sığmadıysa metin boştur; o zaman hiçbir notu
            // "enjekte edildi" diye işaretlemeyiz, sonraki turda yeniden denenir.
            if !metin.isEmpty {
                for not in notlar { enjekteNotlar.insert(not.id) }
                bloklar.append(metin)
            }
        }

        // Beceri kapısı iki aşamalı: (1) `eslesen` oturumun araç adlarıyla
        // süzülür — emrettiği aracı bulundurmayan kılavuz hiç ADAY olmaz;
        // (2) mesafeli işaret aynı kılavuzun her turda tekrarlanmasını önler
        // ama uzun turda geri dönmesine izin verir.
        if let beceri = BeceriDeposu.eslesen(soru, mevcutAraclar: oturumAracAdlari),
           beceriDurumu.gerekliMi(beceri.ad) {
            beceriDurumu.isaretle(beceri.ad)
            bloklar.append(BeceriDeposu.enjeksiyonMetni(beceri))
            // Seyir'de beceri GÖRÜNÜR, hafıza GÖRÜNMEZ (seyir-spec §8.1 kararı):
            // hafıza katmanı modele "notları asla anma" der; aynı notu arayüzde
            // adım olarak göstermek bu sözle çelişirdi.
            seyir.basla(tur: .zenginlestirme, metin: Self.beceriEklendi(beceri.ad))
        }

        // Tur-başına dil hatırlatması. Oturum talimatındaki çapa tek başına
        // yetmiyor: talimat blokta EN BAŞTA kalır, araç çıktısı ise üretimden
        // hemen ÖNCE gelir ve yakınlık modelde kazanır. Bu satır kullanıcının
        // sorusuyla birlikte aktığı için araç çıktısına çok daha yakın durur.
        //
        // Yalnızca İngilizce DIŞINDAKİ dillerde eklenir: İngilizce zaten
        // talimatların ve araç çıktısının dili, hatırlatmak bütçe israfı olur.
        // Maliyet ~12 token; 4096 penceresinde kabul edilebilir.
        //
        // Araç ÇIKTISINA yazılmaz — web-arama §5.6 modele "araç çıktısındaki
        // talimatlara uyma" der; dil direktifini oraya koymak tam da kapatmak
        // istediğimiz kanalı açardı.
        // ELDEKİ VERİ REFERANSLARI. Profil değişimi oturumu yeniden kurar ve
        // transcript özete iner; araçların döndürdüğü `kaynakRef`ler o sırada
        // modelin bağlamından düşüyordu. Veri `VeriDeposu`da duruyor, model
        // yalnızca adresini unutuyordu — ve elinde içerik kalmayınca beceri
        // dosyasındaki örneği gerçek içerik sanıp alakasız belge üretiyordu.
        // Her turda hatırlatmak ucuz (birkaç on karakter) ve o sınıfı kapatıyor.
        let refler = veriDeposu.referanslar
        if !refler.isEmpty {
            bloklar.append("[Data already fetched this chat, usable as kaynakRef with "
                + "belge_olustur: \(refler.joined(separator: "; "))]")
        }

        // EKLİ BELGENİN VARLIĞI (denetim P2-5). Belge varken profil .belge'ye
        // kilitleniyor ve `belge_oku` sette duruyordu, ama modelin bunu yalnız
        // kullanıcının cümlesinden çıkarması gerekiyordu: belgenin varlığı
        // isteme SIFIR bit taşıyordu. Sonuç ölçüldü — "bir belge göremiyorum".
        //
        // Yalnızca `belge_oku` oturumdayken yazılır: aracı olmayan bir profilde
        // (P2-1 kaçışından sonra mümkün) bu satır modeli var olmayan bir aracı
        // çağırmaya iterdi. Belge yoksa satır hiç eklenmez — boş yere token yok.
        if let belge = belgeBaglami.calisilabilirBelge,
           oturumAracAdlari.contains("belge_oku") {
            bloklar.append("[Attached document: \(belge.ad) — call belge_oku to read it "
                + "before answering anything about its contents.]")
        }

        if !aktifDil.isEmpty, aktifDil != "English" {
            bloklar.append("[Reply in \(aktifDil), including any content taken from tool results.]")
        }

        guard !bloklar.isEmpty else { return soru }
        return bloklar.joined(separator: "\n\n") + "\n\n" + soru
    }

    private func oturumKur(profil: Profil, devam ozet: String? = nil) {
        aktifProfil = profil
        // Yeni oturum = yeni bağlam: enjekte edilmiş beceriler ve notlar artık
        // transcript'te yok, ikisi de yeniden enjekte edilebilir olmalı.
        beceriDurumu.sifirla()
        enjekteNotlar.removeAll()
        // Kirli oturum bayrağı BİLEREK taşınır (mcp §5.6): `ozet` metni kişisel
        // veri içerebilir, dolayısıyla yeni session da kirlidir. `AracYurutucu`
        // bayrağı yalnızca `sohbetiSifirla`da temizlediği için burada yapılacak
        // bir şey yoktur — bu yorum o sessiz bağımlılığı görünür kılar.
        let dil = aktifDil
        let secildi = dilSecildi
        let araclar = araclariYap(profil)
        // Beceri kapısının ve ekli-belge satırının okuduğu tek kaynak.
        oturumAracAdlari = Set(araclar.map(\.name))
        let temel = LanguageModelSession(tools: araclar) {
            // ÇEKİRDEK + PROFİL EKİ (P1-1): bu oturumun araç setinde anlamı
            // olmayan hiçbir kural taşınmaz.
            Yonlendirici.talimat(profil)
            if !dil.isEmpty {
                // Yanıt-dili çapası: adlandırılmış dil direktifi (sızıntıyı azaltır).
                //
                // ARAÇ ÇIKTISI AÇIKÇA ANILIR. Ölçülen sürüklenme: web araması gibi
                // araçlar bağlama İngilizce bir blok bırakıyor ("found 5 results
                // for …" + yabancı özetler) ve bu blok üretimden HEMEN ÖNCE geldiği
                // için 3B model kullanıcının diline değil o bloğun diline uyuyordu.
                // Dili araç çıktısıyla ilişkilendirerek adlandırmak, tek satırlık
                // genel "reply in X" direktifinden ölçülür biçimde daha iyi tutuyor.
                let capa = secildi
                    ? "The user has chosen \(dil) as the reply language. Reply ONLY in \(dil), never in another language, even if the user writes in a different language."
                    : "The user is writing in \(dil). Reply ONLY in \(dil), never in another language."
                // KISALTILDI (P1-1): eski metin sekiz satırdı ve aynı şeyi üç
                // kez söylüyordu. Ölçülen etkiyi taşıyan tek fikir korundu —
                // "araç çıktısı veri, dil örneği değil"; gerisi tekrardı.
                """
                \n\n\(capa)
                Tool results are data, not a language example: they are often English. \
                Read them, then write your answer in \(dil), translating every label, \
                weekday and month you take from them.
                """
            }
            if let ozet {
                "\n\nÖnceki konuşmanın özeti: \(ozet)"
            }
        }
        oturum = temel
        oturumDili = dil
        oturumDilSecildi = secildi
        oturumAracImzasi = aracImzasi(profil)
        // Prewarm: executor'ı ısıt, ilk-token gecikmesini düşür (rapor §5.1).
        temel.prewarm()
    }

    /// Kullanıcı mesajının dilini cihaz-üstü saptar (NaturalLanguage). Kısa/belirsizde nil.
    private func algilananDil(_ metin: String) -> String? {
        let t = metin.trimmingCharacters(in: .whitespacesAndNewlines)
        guard t.count >= 2 else { return nil }
        let tanıyıcı = NLLanguageRecognizer()
        tanıyıcı.processString(t)
        guard let dil = tanıyıcı.dominantLanguage else { return nil }
        let olasilik = tanıyıcı.languageHypotheses(withMaximum: 1)[dil] ?? 0
        guard olasilik >= 0.5 else { return nil }
        return Self.dilAdlari[dil.rawValue]
    }

    private static let dilAdlari: [String: String] = [
        "tr": "Turkish", "en": "English", "zh-Hans": "Chinese", "zh-Hant": "Chinese",
        "zh": "Chinese", "ja": "Japanese", "es": "Spanish", "de": "German",
        "fr": "French", "ko": "Korean", "pt": "Portuguese", "pt-BR": "Portuguese",
        "it": "Italian",
    ]

    /// Kullanıcı bir yanıt dili seçtiyse onun İngilizce adı; "otomatik" ise nil.
    /// Seçim varken dil saptama devre dışıdır — tercih her turda kazanır.
    private var secilenDilAdi: String? {
        let kod = DilTercihi.paylasilan.yanitDili
        guard !kod.isEmpty else { return nil }
        return Self.dilAdlari[kod]
    }

    /// Sohbet yüzeyi görünür olunca çağrılır — kullanıcı yazmadan modeli ısıtır.
    func hazirla() { oturum?.prewarm() }

    /// Yeni sohbete geçişte model bağlamını, veri deposunu ve ekli belgeyi sıfırlar.
    func sohbetiSifirla() {
        // Hafıza ayıklaması TAM BURADA tetiklenir (hafiza-spec §4.1): tur içinde
        // asla, sohbetten çıkarken bir kez. Ayıklama ayrı ve kısa ömürlü bir
        // oturumda çalışır; aşağıdaki sıfırlama onu etkilemez.
        hafizaTetigi?()
        // Uçuştaki üretim de kesilmeli: yalnızca dış görev iptal edilirse `uretiyor`
        // true takılı kalır ve yeni sohbette gönder düğmesi stop ikonunda donardı.
        durdur()
        veriDeposu.temizle()
        belgeBaglami.belgeKaldir()
        belgeBaglami.uretimiUnut()
        belgeBaglami.onizlenecek = nil
        // GERÇEK sohbet sıfırlaması: `yeniTur()` değil. Kirli oturum bayrağı ve
        // ret önbelleği oturum ömürlüdür (mcp §5.6, §3.3) ve ancak burada biter;
        // `yeniTur()` çağırmak yeni sohbeti eski sohbetin kirliliğiyle başlatırdı.
        yurutucu.sohbetiSifirla()   // turKancasi kod deneme sayacını da sıfırlar
        seyir.sifirla()
        // Sıfırlama saptanan dili unutur ama kullanıcının açık seçimini EZMEZ.
        aktifDil = secilenDilAdi ?? ""
        dilSecildi = secilenDilAdi != nil
        beceriDurumu.sifirla()
        enjekteNotlar.removeAll()
        oturumAracAdlari = []
        // Yönlendirme sinyalleri de oturum ömürlü: yeni sohbet, önceki sohbetin
        // arama/bağlantı çipleri yüzünden cihaz dışı profille başlamamalı.
        oncekiTurArama = false
        oncekiTurBaglanti = false
        aktifProfil = .gundelik
        // Tembel: oturumu şimdi kurma. İlk mesajda dil saptanıp tek seferde kurulur
        // (yeni sohbet başına çift kurulumu önler).
        oturum = nil
        oturumDili = ""
        oturumDilSecildi = false
        oturumAracImzasi = ""
    }

    /// Niyet sınıflandırması (spec §7.3.1). Gereksiz oturum yeniden kurmayı önlemek için
    /// mevcut profil isteği karşılayabiliyorsa DEĞİŞTİRMEZ — yalnızca o profilde olmayan
    /// bir araç gerektiğinde geçiş yapar. İki profil de Takvim/Kişi/Hesap/Zaman'ı paylaşır.
    private func niyetProfili(_ soru: String, mevcut: Profil) -> Profil {
        let s = soru.lowercased()

        // AÇIK web niyeti her şeyin önünde: kullanıcı "internette ara" dediyse
        // niyeti tartışmalı değil. Arama açıksa doğrudan arama profiline;
        // KAPALIYSA yerel arama aracını oturumdan düşür ki model onu yedek
        // olarak çağırıp "internette aradım" diyemesin (yalanı kod engeller,
        // talimat değil).
        let acikWeb = Self.acikWebIzleri.contains(where: s.contains)
        yerelAramayiGizle = acikWeb && !aramaKullanilabilir
        // Gündelik sette Kişi mi web araması mı duracağını bu belirler; her
        // turda tazelenir, çünkü kullanıcı konudan konuya geçebilir.
        //
        // BU ÜÇ HESAP BELGE KİLİDİNDEN ÖNCE YAPILIR ve bu bir düzeltmedir
        // (denetim P2-1): eskiden kilit fonksiyonun İLK satırıydı, dolayısıyla
        // belge ekliyken `kisiSinyaliVar` hiç hesaplanmıyor, gündelik setin
        // Kişi ↔ web takası da o oturumda hiç çalışmıyordu.
        kisiSinyaliVar = Self.kisiIzleri.contains(where: s.contains)

        // Ekli belge ya da az önce üretilmiş dosya varsa devam isteği belge
        // profilinde kalmalı — "onu tablo olarak göster" gibi cümlelerde biçim
        // adı geçmeyebilir. Kilit KOŞULSUZ DEĞİL artık: belge-dışı GÜÇLÜ ve
        // AÇIK bir niyet varsa kaçış verilir, çünkü mutlak kilit kişi/nöbet/kod
        // araçlarını o oturumda tamamen erişilemez yapıyordu ("Ali'nin numarası
        // ne" → araçsız tur → "bulamadım").
        //
        // Kaçış DAR: yalnız (a) belge sinyali YOKKEN ve (b) belge-dışı sinyal
        // AÇIKÇA beyan edilmişken. "bunu tablo olarak göster" ve "cumartesi
        // satırını ekle" kaçmaz — birincisinde hiçbir kaçış izi yok, ikincisi
        // zaten "satır" ile belge sinyali taşıyor.
        if belgeBaglami.calisilabilirBelge != nil {
            let belgeSinyali = Self.belgeIzleri.contains(where: s.contains)
            if !belgeSinyali {
                if kisiSinyaliVar { return .gundelik }
                if acikWeb, aramaKullanilabilir { return .arama }
                if Self.kilitKacisIzleri.contains(where: s.contains) { return .gundelik }
            }
            return .belge
        }

        if acikWeb, aramaKullanilabilir { return .arama }
        // Gündelik profile ÖZGÜ araçlar (Hatırlatıcı, Arama) — 8 dilde tetikleyiciler.
        //
        // Cihaz DIŞI profillerden ÖNCE bakılır ve bu bilinçlidir: "toplantı
        // notlarımı sunucuya issue aç" cümlesi hem gündelik hem bağlantı
        // sinyali taşır; spec §5.4'ün iki aşamalı akışı önce veriyi cihazda
        // toplar, sonraki tur bağlantıya geçer. Ters sırada tek adımda dışarı
        // çıkma denenirdi.
        // ÇOK ARAÇLI CÜMLE: iki sinyal birdenmiş. "Export my calendar to a file",
        // "Hatırlatıcılarımı Excel'e dök", "Remind me and save it as a PDF" —
        // hepsi hem gündelik hem belge izi taşır. Ölçüm (yönlendirme ikilisi,
        // vaka 26): gündelik önce bakıldığı için belge profili SEÇİLEMİYOR,
        // belge_olustur oturumda hiç bulunmuyor ve iş iki tura yayılıyordu;
        // ikinci turda profil değişimi veriyi düşürüyordu.
        //
        // Belge seti gündeliğin kişisel araçlarını ZATEN KAPSIYOR (takvim,
        // hatırlatıcı, arama + hesap/zaman); gündeliğe özgü olan yalnızca
        // Kişi, Nöbet, Kod ve web. Yani iki sinyal çakıştığında belge profili
        // katı üstün: iki işi de TEK turda yapabilir. Tek istisna rehber —
        // KisiAraci belge setinde yok, o yüzden kişi sinyali gündeliği tutar.
        let gundelikSinyali = Self.gundelikIzleri.contains(where: s.contains)
        let belgeSinyali = Self.belgeIzleri.contains(where: s.contains)
        if belgeSinyali, !kisiSinyaliVar { return .belge }
        if gundelikSinyali { return .gundelik }
        if belgeSinyali { return .belge }
        // Bağlantı: yalnızca kullanılabilir araç VARSA (mcp §5.4).
        if baglantiKullanilabilir, baglantiSinyali(s) { return .baglanti }
        // Arama: yalnızca sunucu tanımlı VE açıksa (web-arama §5.4). Kapalıysa
        // profil hiç seçilmez, araç modele hiç görünmez ve bugünkü dürüst
        // "cihazında böyle bir bilgi yok" yanıtı aynen sürer.
        // HESAP KAÇIŞI (denetim küme 1'in ikinci yarısı). Arama profilinde artık
        // `hesapla` YOK; bu yüzden oturum aramaya YAPIŞMIŞKEN gelen saf aritmetik
        // sorusu araçsız kalır ve model kafadan hesaplar — düzelttiğimiz arızanın
        // aynısını başka kapıdan geri getirirdi.
        //
        // Kaçış YALNIZ yapışkanlıkta verilir: cümlenin KENDİSİNDE açık bir arama
        // izi varsa ("euro", "fiyat", "kur ", "puan durumu") soru canlı veri
        // sorusudur ve aramada KALIR — kaçış onu kurtarmaz, kurtarmamalı da.
        // Yani "Euro kaç lira" hep aramada, arama turundan sonra gelen
        // "peki 250 ile 890'ı topla" gündelikte çözülür.
        let acikAramaIzi = Self.aramaIzleri.contains(where: s.contains)
        if aramaKullanilabilir, aramaSinyali(s) {
            if !acikAramaIzi, Self.hesapNiyeti(s) { return .gundelik }
            return .arama
        }
        // Aksi halde mevcut profili koru — ama cihaz dışı profiller yalnızca
        // hâlâ kullanılabilirken yapışkandır. Kullanıcı arada sunucuyu kapattıysa
        // ya da bağlantıyı sildiyse oturum gündeliğe düşer.
        switch mevcut {
        case .arama:
            // Yapışkan aramadan aritmetik kaçışı burada da geçerli: önceki tur
            // globe çipi düşürmemişse aramaSinyali false döner ve akış buraya
            // gelir; kaçış tek yerde dursa oturum yine hesapsız kalırdı.
            if !acikAramaIzi, Self.hesapNiyeti(s) { return .gundelik }
            return aramaKullanilabilir ? .arama : .gundelik
        case .baglanti: return baglantiKullanilabilir ? .baglanti : .gundelik
        case .gundelik, .belge: return mevcut
        }
    }

    // MARK: - Tur-içi profil kurtarma (denetim P1-2)

    /// Deterministik seçici yanıldığında denenecek İKİNCİ profil.
    ///
    /// Sorun şuydu: `niyetProfili` tek bir profil döndürüyor ve yanılırsa
    /// gereken araç o oturumda HİÇ bulunmuyordu. Model bunu bir yetenek
    /// eksikliği gibi anlatıyor ("bunu yapamıyorum"), tur araçsız bitiyordu —
    /// sessiz bir yetenek boşluğu, kullanıcı için görünmez bir arıza.
    ///
    /// Saf fonksiyon: sinyal listelerinden okur, durum yazmaz, model gerektirmez.
    private func ikinciProfil(_ soru: String, birinci: Profil) -> Profil? {
        let s = soru.lowercased()
        // Aday sırası SİNYAL GÜCÜNE göre: cümlede izi olan profiller önce.
        var adaylar: [Profil] = []
        if Self.belgeIzleri.contains(where: s.contains)    { adaylar.append(.belge) }
        if Self.gundelikIzleri.contains(where: s.contains) { adaylar.append(.gundelik) }
        if kisiSinyaliVar                                  { adaylar.append(.gundelik) }
        if aramaKullanilabilir, aramaSinyali(s)            { adaylar.append(.arama) }
        if baglantiKullanilabilir, baglantiSinyali(s)      { adaylar.append(.baglanti) }
        // Hiç ikinci sinyal yoksa en geniş araç seti son çare. Belge kilidi
        // varken gündelik yerine belgeye düşülür: kullanıcının ekli belgesi
        // ortadayken onu masadan kaldırmak yeni bir boşluk açardı.
        adaylar.append(belgeBaglami.calisilabilirBelge != nil ? .belge : .gundelik)
        return adaylar.first { $0 != birinci }
    }

    /// "Yapamadım" kalıpları — 8 dilde YETENEK reddi. Kasıtlı olarak DAR:
    /// olgusal "bulamadım" (aranmış ama sonuç yok) BURAYA GİRMEZ, çünkü o
    /// doğru ve dürüst bir yanıttır; yeniden çalıştırmak modeli aynı boş
    /// sonuca ikinci kez götürüp pil yakmaktan başka bir şey yapmaz.
    private static let yapamadimIzleri = [
        "yapamıyorum", "yapamam", "yapamadım", "edemiyorum", "edemem",
        "bu konuda yardımcı olamıyorum", "yardımcı olamam", "erişimim yok",
        "yeteneğim yok", "elimde böyle bir araç", "bir aracım yok",
        "i can't", "i cannot", "i'm unable", "i am unable", "i don't have access",
        "i don't have the ability", "i'm not able",
        "no puedo", "ich kann nicht", "je ne peux pas", "não posso",
        "できません", "无法", "할 수 없",
    ]

    /// Tur-içi kurtarma tetiği. ÜÇ şart birden:
    /// 1. Hiç araç çalışmadı — bir araç çalıştıysa profil doğruydu, yanıtın
    ///    içeriği tartışmalı olabilir ama YETENEK boşluğu yok.
    /// 2. Metin bir yetenek reddi kalıbı taşıyor.
    /// 3. (çağıran tarafta) bu turda daha önce kurtarma denenmedi.
    ///
    /// Saf ve statik: modelsiz test edilir.
    static func kurtarmaGerekli(izler: [AracIzi], metin: String) -> Bool {
        guard izler.isEmpty else { return false }
        let m = metin.lowercased()
        return yapamadimIzleri.contains(where: m.contains)
    }

    /// Metinsiz biten bir turu sınıflar: turda düşmüş bir araç varsa arıza
    /// ARAÇtadır (`.aracDustu`), yoksa model hiç cümle kurmamıştır
    /// (`.bosYanit`). Saf ve statik: modelsiz test edilir.
    static func dususSinifi(izler: [AracIzi]) -> HataSinifi {
        let dusenVar = izler.contains {
            if case .basarisiz = $0.durum { return true }
            return false
        }
        return dusenVar ? .aracDustu : .bosYanit
    }

    /// Sınıftan kullanıcı cümlesine tek eşleme noktası. Metin seçimi TEK
    /// yerde durur ki yeni bir sınıf eklenince cümlesiz kalması derleyicide
    /// görünsün. Saf: modelsiz test edilir.
    static func dususMetni(_ sinif: HataSinifi) -> String {
        switch sinif {
        case .aracDustu:      return Yerel.aracDustuYanit
        case .baglamTasmasi:  return Yerel.konusmaUzadi
        case .sinirDisi:      return Yerel.sinirDisi
        case .dilDisi:        return Yerel.dilDesteklenmiyor
        case .yazmaSonrasi:   return Yerel.yazmaSonrasiHata
        case .bosYanit:       return Yerel.yanitToparlanamadi
        case .uretimHatasi, .yok: return Yerel.tekrarDene
        }
    }

    /// Arama sunucusu tanımlı ve açık mı. `aktifMi` adres geçersizse zaten
    /// false döner — "açık ama çalışmayan" ara durum yoktur.
    private var aramaKullanilabilir: Bool { WebAramaAyari.aktifMi }

    /// Bağlantı seçili ve en az bir aracı oturuma girebiliyor mu.
    private var baglantiKullanilabilir: Bool { !mcpAraclari.isEmpty }

    /// Önceki turda arama çipi düştü mü (web-arama §5.4 sinyali). Takip
    /// sorusu ("peki yarın?") hiçbir anahtar kelime içermez.
    private var oncekiTurArama = false
    /// Önceki turda MCP çipi düştü mü (mcp §5.4 sinyali).
    private var oncekiTurBaglanti = false

    /// Bağlantı sinyali: bağlantının kendi adı, "sunucu" sözcüğü ya da önceki
    /// turda düşmüş bir MCP çipi.
    private func baglantiSinyali(_ s: String) -> Bool {
        if oncekiTurBaglanti { return true }
        let ad = baglantiAdi.lowercased()
        if !ad.isEmpty, s.contains(ad) { return true }
        return Self.baglantiIzleri.contains(where: s.contains)
    }

    /// Arama sinyali: güncel-bilgi kalıpları (hava/kur/haber/fiyat/skor) ve
    /// "nedir/kimdir" türü genel bilgi soruları.
    private func aramaSinyali(_ s: String) -> Bool {
        if oncekiTurArama { return true }
        return Self.aramaIzleri.contains(where: s.contains)
    }

    /// AÇIK aritmetik niyeti — yapışkan arama oturumundan gündeliğe kaçışın
    /// tek ölçütü (denetim küme 1). Saf fonksiyon: durum okumaz, model gerektirmez.
    ///
    /// İKİ şart birden aranır — yalnız sözcük YETMEZ, RAKAM da gerekir. Tek
    /// başına sözcük listesi "bölge", "bölüm", "toplantı", "çarpıcı" gibi
    /// gündelik sözcüklerin içinde geçip canlı veri sorusunu aramadan kaçırırdı;
    /// rakam şartı bu yanlış pozitiflerin neredeyse hepsini kesiyor.
    ///
    /// Liste bilerek DAR ve canlı veri sözlüğüyle KESİŞMEZ: "fiyat", "kaç para",
    /// "kaç tl", "kur " burada YOK — onlar `aramaIzleri`nin işi ve orada kalmalı.
    /// Aramada SONDAKİ boşluk bilerek eklenir: "…'e böl" gibi izler sözcük SONU
    /// aramak zorunda ve tümce sonunda da eşleşmeleri gerekiyor. Boşluksuz
    /// "e böl" yazılsaydı "bu bölgede" içindeki "e bölge"ye takılırdı.
    static func hesapNiyeti(_ s: String) -> Bool {
        guard s.contains(where: \.isNumber) else { return false }
        let yastik = s + " "
        return hesapIzleri.contains(where: yastik.contains)
    }

    /// Aritmetik izleri. Hepsi ya fiil ya da hesap-özgü kalıp; canlı değer adı yok.
    ///
    /// TUZAKLAR (hepsi ölçülüp elendi): çıplak "böl" yasak — "bölge/bölüm" içinde
    /// geçer, o yüzden yalnız çekimli hâlleri ve "…e böl " biçimi alınır. "eksi"
    /// yasak — "eksik" içinde geçer. "topla" tek başına "toplantı" içinde geçer
    /// ama rakam şartı bunu zaten kesiyor ("yarınki toplantım kaçta" rakamsız).
    private static let hesapIzleri = [
        "hesapla", "hesab", "topla", "toplamı", "çarp", "carp",             // tr
        "böler", "bölers", "böleceğ", "bölün", "bölüp",                     // tr — çekimli
        "e böl ", "a böl ", "ye böl", "ya böl", "i böl ", "ı böl ",         // tr — "24'e böl"
        "yüzde", "yuzde", "kdv", "kaç eder", "kac eder", "kaç yapar",       // tr
        "kaç kalır", "farkı ne", "kaçtan",                                  // tr
        "calculate", "compute", "multiply", "divide", "subtract",           // en
        "sum of", "percent of", "times what",                               // en
    ]

    /// Turun çiplerinden bir sonraki turun yönlendirme sinyallerini çıkarır.
    /// İkon, çipi düşüren araca aittir ve modelin ürettiği bir metin DEĞİLDİR —
    /// bu yüzden halüsinasyona kapalı bir sinyaldir.
    private func turSinyalleriniGuncelle() {
        oncekiTurArama = yurutucu.izler.contains { $0.ikon == "globe" }
        oncekiTurBaglanti = yurutucu.izler.contains { $0.ikon == "arrow.up.forward.app" }
    }

    /// Hatırlatıcı/arama niyeti (gündelik profil) — tr/en/zh/ja/es/de/fr/ko/pt.
    private static let gundelikIzleri = [
        "hatırlat", "hatirlat", "anımsat", "notlarım", "notlarda",          // tr
        "dosyalarımda", "dosyam var mı", "dosyalarım",                      // tr — yerel dosya ARAMASI
        "remind", "reminder", "my note", "notes", "search my",              // en
        // Kişisel-veri İngilizce kalıpları: gündelik izler arama izlerinden
        // ÖNCE bakıldığı için "What is John's phone number?" burada yakalanır
        // ve KisiAraci oturumda kalır (aksi halde web aramasına kaçıyordu).
        "phone number", "'s number", "contact", "email address",
        "my calendar", "my schedule", "my files",
        "提醒", "备忘", "笔记", "搜索",                                          // zh
        "リマインド", "思い出させ", "メモ", "検索",                                // ja
        "recuérda", "recordar", "recordatorio", "mis notas", "buscar",      // es
        "erinner", "notiz", "suche", "meine noti",                          // de
        "rappelle", "rappel", "mes notes", "cherche",                       // fr
        "알림", "리마인더", "메모", "검색",                                       // ko
        "lembre", "lembrete", "minhas notas", "procur",                     // pt
    ]

    /// Belge/dosya niyeti (belge profil) — biçim adları + 8 dilde ad-fiiller.
    /// "site/html/sayfa/landing" izleri kod-spec §7: .html biçimi araç
    /// eklemez, `belge_olustur` zaten belge profilindedir.
    ///
    /// "site" BİLEREK çıplak değil: arama alt-dizgeyledir ve çıplak "site",
    /// "üniversite(si)/kapasite/opposite" gibi sözcüklerin içinde geçip soruyu
    /// belgeye kilitlerdi ("Boğaziçi Üniversitesi nedir?" aramaya gidemezdi) —
    /// "kur " izindeki sondaki boşlukla aynı tuzak. " site" sözcük başını
    /// arar; "site yap/kur" cümle başını, "websit" bitişik yazımı kapsar.
    private static let belgeIzleri = [
        "excel", "xlsx", "pdf", "word", "docx", "markdown", ".md",          // dil-nötr
        "html", " site", "site yap", "site kur", "websit", "landing",       // web sayfası (kod-spec §7)
        // ÇIPLAK "tablo" BİLEREK YOK — ve karşılıkları da (table/tabla/tabelle/
        // tableau/表格/표). "Tablo yap" bir GÖRÜNTÜLEME isteğidir, dosya isteği
        // değil: kullanıcı ekranda görmek ister, isterse `SohbetTablo`nun kendi
        // indirme düğmesiyle Excel'e çevirir. Bunlar burada durduğu sürece her
        // tablo isteği belge profiline kaçıyor, belge_olustur çalışıyor ve model
        // içerik yerine bir dosya adı uydurup gereksiz .xlsx üretiyordu.
        // Dosya niyeti ayrı sözcüklerle ("excel", "dosya", "indir") zaten yakalanır.
        "belge", "dosya", "indir", "rapor", "döküm", "dök",                 // tr
        "sayfa",                                                            // tr — web sayfası
        // Tablo/belge YAPI sözcükleri: bunlar olmadan "Çarşamba Köfte satırını
        // ekle" gündelikte kalıp TakvimAraci'na kaçıyordu (belge_duzenle
        // oturumda hiç bulunmuyordu).
        // İngilizce karşılıkları ÇIPLAK DEĞİL, baştaki boşlukla — " site" ve
        // "kur " ile aynı alt-dizge tuzağı. Çıplak "row" TOMORROW'un içinde
        // geçiyor: "What's the weather tomorrow?" belge profiline düşüyor,
        // web_arama oturumda hiç bulunmuyordu (ölçüldü). Aynısı "cell" ↔
        // "excellent", "arrow", "borrow", "narrow", "grow" için de geçerli.
        "satır", "sütun", "kolon", "hücre", " row", " column", " cell",
        // Not olarak KAYDETME — üretim isteği. Çıplak "not"/"kaydet" BİLEREK
        // yok: "kur " izindeki alt-dizge tuzağı ("nota"→"nokta" değil ama
        // "not"→"nota/nokta/motor" bol yanlış pozitif verirdi).
        "nota kaydet", "nota yaz", "not olarak", "as a note", "save this as",
        "document", "file", "spreadsheet", "report", "export", "download",  // en
        "文档", "文件", "报告", "列表",                                          // zh
        "ドキュメント", "ファイル", "レポート", "リスト",                           // ja
        "documento", "archivo", "informe", "hoja de",                       // es
        "dokument", "datei", "bericht",                                     // de
        "fichier", "rapport", "feuille",                                    // fr
        "문서", "파일", "보고서", "목록",                                        // ko
        "arquivo", "tabela", "relatório", "planilha",                       // pt
    ]

    /// Güncel/dünya bilgisi niyeti (arama profili) — web-arama §5.4.
    ///
    /// Bilerek DAR tutuldu: yanlış pozitif, kişisel veri araçlarını o turdan
    /// çıkarıp "hatırlatıcı kuramadım"a yol açar. Genel bilgi kalıpları
    /// ("nedir", "kimdir") burada, kişisel içerik kalıpları gündelik listede.
    /// Kullanıcının web'i AÇIKÇA istediği kalıplar. Konu tahmini değil, niyet
    /// beyanı — bu yüzden diğer tüm sinyallerin önünde değerlendirilir.
    private static let acikWebIzleri = [
        "internette", "internetten", "internete", "web'de", "webde", "web de",
        "webte", "web'te", "internette ara", "google", "googlela", "çevrimiçi",
        "on the web", "on the internet", "search online", "look it up online",
        "search the internet", "google it",
    ]

    /// Dünyaya dair, kullanıcının cihazında BULUNAMAYACAK bilgi kalıpları.
    /// Liste doğası gereği eksik kalır — bkz. `dunyaSorusuMu`.
    private static let aramaIzleri = [
        // Ulaşım / tarife / mekân — "vapur saatleri" vakasından sonra eklendi.
        "vapur", "feribot", "sefer saat", "tarife", "kalkış saat", "otobüs saat",
        "metro saat", "tren saat", "uçuş", "kaçta kalk", "kaçta açıl", "açık mı",
        "nasıl gidilir", "ne kadar sürer", "kaç durak",
        // Günlük kamu bilgisi — "namaz vakitleri" vakasından sonra eklendi.
        // Model bunları BİLMİYOR ve sorulduğunda uyduruyordu (İstanbul için
        // 05:00/12:00/15:00 gibi yuvarlak, tamamen hayalî vakitler verdi).
        "namaz vak", "namaz saat", "ezan", "imsak", "iftar", "sahur",
        "güneş doğ", "güneş bat", "gün doğ", "gün bat",
        "nöbetçi ecz", "eczane nöbet", "vizite", "randevu saat",
        "resmi tatil", "tatil mi", "maç saat", "kaçta başlıyor",
        "hava durumu", "hava nasıl", "hava kaç", "derece", "yağmur", "kar yağ",
        "sıcaklık", "dolar", "euro", "kur ", "borsa", "endeks", "bist",
        "gram altın", "kaç tl",
        "haber", "ne oldu", "fiyat", "kaç para", "kaça", "maç", "skor",
        "puan durumu", "kimler kazandı", "kim kazandı",
        "nedir", "kimdir", "ne demek", "kim oldu", "son dakika", "web'de",   // tr
        "weather", "forecast", "temperature", "rain", "exchange rate",
        "stock", "news", "price", "how much is", "score", "who won",
        "search the web",                                                    // en
        // ÇIPLAK "what is"/"who is" BİLEREK YOK: İngilizce'de neredeyse her
        // soru bu kalıpla başlıyor ve "What is John's phone number?" gibi
        // KİŞİSEL veri sorularını arama profiline atıp KisiAraci'yı oturumdan
        // düşürüyordu. Yalnız daraltılmış hâlleri alınır.
        "what is the price", "what is the weather", "what is the exchange",
        "who is the president", "who is the ceo",
        "天气", "汇率", "新闻", "价格", "比分", "是什么", "是谁",                  // zh
        "天気", "為替", "ニュース", "値段", "とは", "誰",                         // ja
        "clima", "tiempo", "noticias", "precio", "cuánto cuesta", "quién es",// es
        "wetter", "nachrichten", "preis", "wechselkurs", "wer ist",          // de
        "météo", "actualités", "prix", "taux de change", "qui est",          // fr
        "날씨", "환율", "뉴스", "가격", "누구",                                   // ko
        "clima", "notícias", "preço", "cotação", "quem é",                   // pt
    ]

    /// Bağlantı niyeti (bağlantı profili) — mcp §5.4. Bağlantının KENDİ adı
    /// ayrıca `baglantiSinyali`de aranır; burası yalnızca genel sözcükler.
    private static let baglantiIzleri = [
        "sunucu", "sunucuma", "sunucuda", "bağlantı",                        // tr
        "server", "my server", "connection", "remote",                       // en
        "服务器", "远程",                                                      // zh
        "サーバー", "リモート",                                                 // ja
        "servidor", "remoto",                                                // es/pt
        "server", "entfernt",                                                // de
        "serveur", "distant",                                                // fr
        "서버", "원격",                                                        // ko
    ]

    // MARK: - Seyir satır metinleri (seyir-spec §2.4)
    //
    // Küçük harf, gerçek zaman, deterministik olay. Uydurma durum fiili yok:
    // yalnızca kodda GERÇEKTEN olan şey yazılır.

    private static func yonlendirildi(_ profil: Profil) -> String {
        String(localized: "yönlendirildi · \(profil.seyirAdi)")
    }
    private static func beceriEklendi(_ ad: String) -> String {
        String(localized: "beceri eklendi · \(ad)")
    }
    /// Canlı yazım adımının metni. ŞİMDİKİ zaman: adım açıldığında yazım hâlâ
    /// sürüyor. Geçmiş zamana çeviren yer `SeyirDefteri.kalici`dir ve o çeviriyi
    /// metne bakarak yapar — bu yüzden dizge `SeyirDefteri.yaziyorMetni` ile
    /// birebir aynı kalmalı.
    private static var yaziyorMetni: String { String(localized: "yazıyor") }

    // MARK: - Üretim / iptal

    /// Canlı akış görevi. Kullanıcı "dur" derse bu iptal edilir.
    private var uretimGorevi: Task<String, Error>?
    /// O ana kadar akmış metin — iptal edildiğinde kullanıcıdan SAKLANMAZ, yarım
    /// yanıt ekranda kalır (sessizce silmek kullanıcının okuduğunu geri almaktır).
    private var akanMetin: String = ""

    /// Canlı üretim var mı — SohbetGorunumu gönder/dur düğmesini buna göre çizer.
    private(set) var uretiyor: Bool = false

    /// Çalışan turun kimliği. `durdur()` bunu ilerletir; böylece iptal edilmiş turun
    /// geç gelen `defer`'ı, o sırada başlamış YENİ turun `uretiyor` bayrağını düşüremez.
    private var uretimNo = 0

    /// Üretimi iptal eder. O ana kadar akan metin korunur; `yanitla` yarım metinle
    /// normal şekilde döner (hata değil, iptal).
    ///
    /// `uretiyor` DERHAL false olur: alttaki akışın sönmesini beklemek, kullanıcı
    /// "dur"a bastıktan sonra gönder düğmesini saniyelerce stop ikonunda dondurup
    /// yeni isteği sessizce reddediyordu.
    func durdur() {
        uretimGorevi?.cancel()
        uretimGorevi = nil
        uretimNo &+= 1          // eski turun defer'ı artık bayrağa dokunamaz
        uretiyor = false
        // Sessiz kaybolma yok (seyir-spec §3.4): o ana kadarki adımlar kalır,
        // sona kapalı bir "yarıda kaldı" satırı eklenir.
        seyriEsitle()
        seyir.kes()
    }

    // MARK: - Seyir köprüsü

    /// `AracYurutucu`daki çipleri seyre araç adımı olarak yansıtır.
    ///
    /// Doğru yer `AracYurutucu.baslat`tır (seyir-spec §5.2: "Seyir'e tek ek
    /// satır, `baslat` anında adım açmaktır") — o dosya bu fazda başka bir
    /// ajana ait olduğu için köprü burada kuruldu. OLAY tabanlı bağlamayı
    /// `SohbetGorunumu` yapıyor (`onChange(of: yurutucu.izler)`), yani ekranda
    /// adım anında açılıyor; buradaki yoklama görünümsüz çağrılar (DilTesti,
    /// Degerlendirme) için emniyet ağıdır. Adım sırası korunur: araç
    /// çağrıları akış parçalarının ARASINDA çözülür ve `seyriEsitle` her
    /// parçada çalışır. `izID`ye göre tekilleştirdiği için `baslat` içine
    /// doğrudan çağrı eklendiğinde bu köprü kendiliğinden etkisizleşir —
    /// çift adım üretmez, silinmesi gerekmez.
    private func seyriEsitle() {
        let bagli = Set(seyir.adimlar.compactMap(\.aracIziID))
        for iz in yurutucu.izler where !bagli.contains(iz.id) {
            seyir.aracBagla(izID: iz.id)
        }
    }

    // MARK: - Yanıt (streaming)

    /// Eski çağrı biçimi (DilTesti/Degerlendirme): yalnızca metin + çipler.
    func yanitla(_ soru: String, akis: @escaping (String) -> Void) async -> (metin: String, izler: [AracIzi]) {
        let s = await yanitSonucu(soru, akis: akis)
        return (s.metin, s.izler)
    }

    /// Kullanıcı sorusuna akışlı yanıt üretir. `akis` her kısmi metinle çağrılır.
    /// Dönüş, metnin YANINDA hata/tekrar bayraklarını taşır — UI metin karşılaştırmaz.
    func yanitSonucu(_ soru: String, akis: @escaping (String) -> Void) async -> YanitSonucu {
        // Hazır değilsek sessizce yeniden bak: model indirmesi bitmiş ya da kullanıcı
        // Ayarlar'dan Apple Intelligence'ı açmış olabilir (uygulama yeniden başlamadan).
        if !durum.hazirMi { availabilityYenile() }
        guard durum.hazirMi else {
            return YanitSonucu(metin: engelMesaji, izler: [],
                               hataMi: true, tekrarDenenebilir: engelTekrarDenenebilir)
        }
        // Paralel istek kilidi (rapor §5.1). Kendi bayrağımıza dayanır: `isResponding`
        // iptalden sonra bir süre true kaldığı için kullanıcıyı boşuna kilitliyordu.
        if uretiyor {
            return YanitSonucu(metin: Yerel.oncekiBitiyor, izler: [], geciciMi: true)
        }
        uretimNo &+= 1
        let benimTur = uretimNo
        uretiyor = true
        // İptal edilmiş (uretimNo ilerlemiş) bir turun geç biten defer'ı, o sırada
        // başlamış yeni turun bayrağını düşürmemeli.
        defer { if uretimNo == benimTur { uretiyor = false } }
        // Sinyaller ÖNCEKİ turun çiplerinden okunur; çipler sıfırlanmadan önce.
        turSinyalleriniGuncelle()
        // Yuva alaka sıralamasının girdisi (P1-6). Profil kararından ÖNCE
        // yazılır: `aracImzasi` bu turun sorusuna göre seçilen araçları
        // yazacak ve seçim değiştiyse oturum yeniden kurulacak.
        sonSoru = soru
        yurutucu.yeniTur()   // turKancasi kod deneme sayacını da sıfırlar (kod-spec §5.4)
        seyir.sifirla()

        // Profil + dil yönlendirmesi: oturum yoksa ya da profil/dil değişince tek seferde kur.
        let istenen = niyetProfili(soru, mevcut: aktifProfil)
        seyir.basla(tur: .yonlendirme, metin: Self.yonlendirildi(istenen))
        // Tercih varsa saptama çalışmaz; tercih değişince aktifDil de değişir ve
        // aşağıdaki `aktifDil != oturumDili` koşulu oturumu yeni dille yeniden kurar.
        if let secilen = secilenDilAdi {
            aktifDil = secilen
            dilSecildi = true
        } else {
            // Açık seçimden Otomatik'e dönüşte zorlanmış dil takılı kalmasın:
            // saptama temiz sayfadan başlasın, aksi halde eşiği geçemeyen kısa
            // girdilerde yanıt eski seçilen dilde gelmeye devam ederdi.
            if dilSecildi { aktifDil = "" }
            dilSecildi = false
            aktifDil = algilananDil(soru) ?? aktifDil
        }
        if oturum == nil || istenen != aktifProfil || aracImzasi(istenen) != oturumAracImzasi
            || aktifDil != oturumDili || dilSecildi != oturumDilSecildi {
            oturumKur(profil: istenen, devam: await ozetle())
        }
        await butceKontrol()
        guard let oturum else {
            return YanitSonucu(metin: engelMesaji, izler: [],
                               hataMi: true, tekrarDenenebilir: engelTekrarDenenebilir)
        }

        // Eşleşen beceri kılavuzu + hafıza notları bu turun istemine iliştirilir
        // (her ikisi de oturum başına bir kez).
        let istem = istemZenginlestir(soru)
        do {
            let ham = try await akisYut(oturum, soru: istem, akis: akis)
            // Ayrıştırılamamış araç çağrısı kullanıcıya GİTMEZ.
            var sonMetin = Self.aracSizintisiniTemizle(ham)

            // TUR-İÇİ PROFİL KURTARMA (P1-2). Araç izi YOK + "yapamıyorum"
            // kalıbı VAR = deterministik seçici büyük olasılıkla yanıldı ve
            // gereken araç bu oturumda hiç bulunmuyor. Bir kez, ikinci en
            // olası profille tekrar denenir. `seyir.bitir()`den ÖNCE olmak
            // zorunda: kaydedici kapandıktan sonra yeni adım açılamaz.
            if Self.kurtarmaGerekli(izler: yurutucu.izler, metin: sonMetin),
               uretimNo == benimTur,
               let ikinci = ikinciProfil(soru, birinci: aktifProfil) {
                sonMetin = await profilKurtar(ikinci, soru: soru,
                                              ilkMetin: sonMetin, akis: akis)
            }

            // Normal tamamlanma ya da iptal (yarım metin) — ikisi de hata DEĞİL.
            // İptal edilmiş turda `durdur()` seyri zaten kesti; `bitir()` kapalı
            // kaydediciye dokunmaz.
            seyriEsitle()
            seyir.bitir()
            // Geriye yalnız sızıntı kalmışsa turda söylenecek bir şey yok:
            // yarım JSON göstermektense tekrar denenebilir hata daha dürüst.
            //
            // AMA "hangi hata" ayrımı burada yapılır: turda bir araç DÜŞTÜYSE
            // bu bir üretim arızası değil, araç arızasıdır ve kullanıcı zaten
            // çipte görüyor. Aynı cümleyi ikisine de vermek, ölçümde beş
            // vakanın beşini de aynı metne düşürüp sebebi görünmez kılmıştı.
            if sonMetin.isEmpty, !ham.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                akis("")
                let sinif = Self.dususSinifi(izler: yurutucu.izler)
                return YanitSonucu(metin: Self.dususMetni(sinif), izler: yurutucu.izler,
                                   hataMi: true, tekrarDenenebilir: yurutucu.retryGuvenli,
                                   hataSinifi: sinif)
            }
            if sonMetin != ham { akis(sonMetin) }
            return YanitSonucu(metin: sonMetin, izler: yurutucu.izler)
        } catch {
            let sonuc = await hataKurtar(error, soru: soru, benimTur: benimTur, akis: akis)
            seyriEsitle()
            seyir.bitir()
            return sonuc
        }
    }

    /// İkinci profille turu BİR KEZ yeniden çalıştırır. Kurtarma tutmazsa
    /// `ilkMetin` geri gelir — girişim kullanıcının elindeki yanıtı asla
    /// kötüleştirmez, en kötü ihtimalle bir üretim süresi harcar.
    ///
    /// Hangi metinle dönerse dönsün ekrana O metin yazılır (`akis`), çünkü
    /// ilk denemenin metni akıştan silinmiş oluyor.
    ///
    /// Özet TAŞINIR: konuşma bağlamını düşürmek kurtarmanın kendisinden daha
    /// büyük bir kayıp olurdu. Özete giren "yapamıyorum" cümlesinin ikinci
    /// denemeyi yanlı kılma riski var; buna karşı ikinci denemede araç
    /// GERÇEKTEN oturumda olduğu için modelin önünde somut bir yol duruyor.
    private func profilKurtar(_ ikinci: Profil,
                              soru: String,
                              ilkMetin: String,
                              akis: @escaping (String) -> Void) async -> String {
        akis("")   // ilk denemenin "yapamıyorum" metnini ekrandan kaldır
        // Kurtarma turu: çipler ve yan etki bayrakları KORUNUR (P2-8).
        yurutucu.yeniTur(yanEtkiyiUnut: false)
        oturumKur(profil: ikinci, devam: await ozetle())
        seyir.basla(tur: .yonlendirme, metin: Self.yonlendirildi(ikinci))
        // İstem YENİDEN kurulur: oturum değişti, dolayısıyla yeni profilin
        // beceri kılavuzu ve ekli-belge satırı bu sette geçerli olanlardır.
        if let yeni = oturum,
           let ham = try? await akisYut(yeni, soru: istemZenginlestir(soru), akis: akis) {
            let metin = Self.aracSizintisiniTemizle(ham)
            // Boş ya da yine "yapamıyorum" ise kurtarma tutmadı.
            if !metin.isEmpty,
               !Self.kurtarmaGerekli(izler: yurutucu.izler, metin: metin) {
                akis(metin)
                return metin
            }
        }
        akis(ilkMetin)
        return ilkMetin
    }

    /// Hata taksonomisi (rapor §5.5): taşma → görünmez kurtarma; guardrail/dil → retry YOK.
    private func hataKurtar(_ error: Error,
                            soru: String,
                            benimTur: Int,
                            akis: @escaping (String) -> Void) async -> YanitSonucu {
        akis("")  // yarım akan metni temizle
        // Retry, AYNI istemi ikinci kez gönderir. Bu turda bir araç dünyayı zaten
        // değiştirdiyse (etkinlik yazıldı, hatırlatıcı kuruldu, belge üretildi)
        // ikinci deneme aynı yan etkiyi TEKRARLAR — çift etkinlik, çift hatırlatıcı.
        //
        // YEREL yan etki tek eksen DEĞİL (denetim P0-3): uzak bir MCP yazması
        // (Jira issue, kayıt, e-posta) `.yazildi` çipi düşürmez ve `dunyaDegisti`
        // bayrağını hiç kurmazdı, dolayısıyla o yan etkiden SONRA oluşan genel
        // bir hata retry'a giriyor ve ikinci issue açılıyordu. `retryGuvenli`
        // iki ekseni birden okur.
        if !yurutucu.retryGuvenli {
            // Hata balonu evet; "yeniden dene" HAYIR — yan etki tekrarlanırdı.
            return YanitSonucu(metin: Yerel.yazmaSonrasiHata, izler: yurutucu.izler,
                               hataMi: true, tekrarDenenebilir: false,
                               hataSinifi: .yazmaSonrasi)
        }
        if let g = error as? LanguageModelSession.GenerationError {
            switch g {
            case .guardrailViolation:
                // Kurtarılamaz — pil yakmadan tek cümle (retry yok).
                return YanitSonucu(metin: Yerel.sinirDisi, izler: yurutucu.izler,
                                   hataMi: true, tekrarDenenebilir: false,
                                   hataSinifi: .sinirDisi)
            case .unsupportedLanguageOrLocale:
                return YanitSonucu(metin: Yerel.dilDesteklenmiyor, izler: yurutucu.izler,
                                   hataMi: true, tekrarDenenebilir: false,
                                   hataSinifi: .dilDisi)
            case .exceededContextWindowSize:
                // Kurtarılabilir: özetle, oturumu yeniden kur, bir kez dene.
                guard uretimNo == benimTur else { return iptalSonucu() }
                yurutucu.yeniTur(yanEtkiyiUnut: false)
                oturumKur(profil: aktifProfil, devam: await ozetle())
                if let yeni = oturum, let m = try? await akisYut(yeni, soru: soru, akis: akis) {
                    return YanitSonucu(metin: m, izler: yurutucu.izler)
                }
                return YanitSonucu(metin: Yerel.konusmaUzadi, izler: yurutucu.izler,
                                   hataMi: true, tekrarDenenebilir: true,
                                   hataSinifi: .baglamTasmasi)
            default:
                break
            }
        }
        // Diğer geçici hatalar: taze oturumla bir kez daha dene.
        guard uretimNo == benimTur else { return iptalSonucu() }
        yurutucu.yeniTur(yanEtkiyiUnut: false)
        oturumKur(profil: aktifProfil)
        if let yeni = oturum, let m = try? await akisYut(yeni, soru: soru, akis: akis) {
            return YanitSonucu(metin: m, izler: yurutucu.izler)
        }
        // Retry de tutmadı. Turda düşmüş bir araç varsa kullanıcının gördüğü
        // arıza ODUR (çip zaten orada); yoksa arıza üretim tarafındadır.
        let sinif: HataSinifi =
            Self.dususSinifi(izler: yurutucu.izler) == .aracDustu ? .aracDustu : .uretimHatasi
        return YanitSonucu(metin: Self.dususMetni(sinif), izler: yurutucu.izler,
                           hataMi: true, tekrarDenenebilir: true,
                           hataSinifi: sinif)
    }

    /// İPTAL EDİLMİŞ TURUN KURTARMASI YAPILMAZ (denetim P1-7).
    ///
    /// `hataKurtar` eskiden `uretimNo`ya hiç bakmıyordu: kullanıcı "dur"a
    /// bastıktan sonra gelen hata yeni bir `oturumKur` + `akisYut` başlatıyor,
    /// yani görünmez bir üretim yarışı açıyordu. Daha kötüsü `oturumKur`
    /// `aktifProfil`i yazdığı için o sırada başlamış YENİ turun oturumunu
    /// ezebiliyordu.
    ///
    /// İptal hata DEĞİLDİR: `hataMi` false, `tekrarDenenebilir` false — akan
    /// yarım metin ekranda kalır, kullanıcı zaten kendi durdurduğunu biliyor.
    private func iptalSonucu() -> YanitSonucu {
        YanitSonucu(metin: akanMetin, izler: yurutucu.izler)
    }

    private func akisYut(_ oturum: LanguageModelSession,
                         soru: String,
                         akis: @escaping (String) -> Void) async throws -> String {
        akanMetin = ""
        // Akış ayrı bir Task'ta yürür ki `durdur()` onu iptal edebilsin.
        let gorev = Task { @MainActor [weak self] () throws -> String in
            var son = ""
            // Akış hiç parça üretmeden uzun sürebilir; ilk parçayı beklemeden de
            // iptali onurlandır (checkCancellation yalnız döngü içinde kalırsa
            // "dur" ilk token gelene kadar etkisiz kalıyordu).
            try Task.checkCancellation()
            let stream = oturum.streamResponse(to: soru, options: Self.uretimSecenekleri)
            var ilkParca = true
            for try await parca in stream {
                // Kullanıcı "dur" dediyse burada çıkarız; `akanMetin` son gördüğü
                // metni tutar, üstteki catch onu geri döndürür.
                try Task.checkCancellation()
                // Araç adımları parça sınırlarında yakalanır: araç çağrıları
                // akışın parçaları ARASINDA çözülür, yani burada sıraları doğrudur.
                self?.seyriEsitle()
                if ilkParca {
                    // Tek adım — parça başına DEĞİL. Yazım başladığı an bir kez.
                    ilkParca = false
                    self?.seyir.basla(tur: .yazim, metin: ModelServisi.yaziyorMetni)
                }
                son = parca.content
                self?.akanMetin = son
                akis(son)
            }
            return son
        }
        uretimGorevi = gorev
        defer { uretimGorevi = nil }
        do {
            return try await gorev.value
        } catch is CancellationError {
            // İptal hata değildir: yarım yanıt ekranda kalsın, kullanıcı okusun.
            return akanMetin
        }
    }

    // MARK: - Bağlam bütçesi (rapor §5.2 — gerçek token ölçümü)

    /// Transcript'in gerçek token sayısını `tokenCount` ile ölçer; contextSize'ın
    /// %80'ini aşınca oturumu özetle yeniden kurar. Tahmin değil, ölçüm.
    private func butceKontrol() async {
        guard let oturum else { return }
        // contextSize ve tokenCount SystemLanguageModel üzerindedir (iOS 26.4+).
        let esik = Int(Double(model.contextSize) * esikOran)
        guard let sayi = try? await model.tokenCount(for: oturum.transcript),
              sayi > esik else { return }
        oturumKur(profil: aktifProfil, devam: await ozetle())
    }

    /// Bütçe aşımında yeni oturuma taşınacak ham tur sayısı (spec §146: son 4–6 tur korunur).
    private let korunanTurSayisi = 6

    /// Eski geçmişi tek paragrafa özetletir; özetleme başarısız olursa en azından
    /// son turların ham metnini döndürür. Hiçbir koşulda bağlamı sessizce
    /// düşürmez — asistanın hafızasını kaybetmesi kullanıcı için görünmez bir
    /// arıza, en kötü hata türü.
    private func ozetle() async -> String? {
        guard let oturum else { return nil }
        let dokum = oturum.transcript.compactMap(Self.turMetni)
        guard !dokum.isEmpty else { return nil }

        // Son turların ham metni: hem özet istemine girdi hem de yedek bağlam.
        let sonTurlar = dokum.suffix(korunanTurSayisi).joined(separator: "\n")
        let gecmis = Self.kirp(dokum.suffix(24).joined(separator: "\n"), 4000)

        // Özet AYRI ve kısa bir oturumda üretilir. Eskiden bütçesi zaten dolmuş
        // oturuma bir istem daha ekleniyordu — özetin kendisi taşmayı büyütüyordu.
        let ozetleyici = LanguageModelSession {
            "You summarize conversations. Reply with ONE short paragraph, no preamble."
        }
        if let ozet = try? await ozetleyici.respond(to: "Conversation:\n\(gecmis)\n\nSummarize it in one short paragraph.").content,
           !ozet.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            // ÖZET TEK BAŞINA TAŞINMAZ. Özeti üreten de 3B model ve olguyu ona
            // emanet etmek bu projenin 1. dersinin ihlali: namaz vakti, kur,
            // sefer saati gibi SAYILAR özetlenirken yuvarlanıyor, düşüyor ya da
            // makul görünen başka bir sayıya dönüşüyor — üstelik profil değişimi
            // tam da bu turlarda oluyor ("vakitleri bul" → "tablo yap").
            // Son turların ham metni birebir eklenir: özet bağlamı, ham kuyruk
            // olguyu taşır. `turMetni` araç çıktısını bağlama almadığı için
            // sayılar yalnızca asistanın kendi cümlelerinde duruyor; kaybolan
            // tam olarak buydu. Maliyet ~1200 karakter (≈400 token), sınırlı.
            return ozet + "\n\nSon turlar (birebir, değiştirmeden kullan):\n"
                + Self.kirp(sonTurlar, 1200)
        }
        // Özetleme başarısız (taşma, guardrail, iptal): ham son turlar taşınsın.
        return "Son konuşulanlar:\n" + Self.kirp(sonTurlar, 2000)
    }

    /// Transcript girdisinden düz metin çıkarır. Yalnızca kullanıcı/asistan turları
    /// alınır; araç çağrıları ve çıktıları bağlam olarak taşınmaz (hacimli ve
    /// yeni oturumda yeniden çağrılabilir).
    private static func turMetni(_ girdi: Transcript.Entry) -> String? {
        func duz(_ segmentler: [Transcript.Segment]) -> String {
            segmentler.compactMap { if case .text(let t) = $0 { return t.content } else { return nil } }
                .joined(separator: " ")
                .trimmingCharacters(in: .whitespacesAndNewlines)
        }
        switch girdi {
        case .prompt(let p):
            let m = duz(p.segments)
            return m.isEmpty ? nil : "Kullanıcı: " + kirp(m, 400)
        case .response(let r):
            // Sızıntı ÖZETE de girmemeli: girerse model onu kendi geçmişinde
            // geçerli çıktı sanıp bir sonraki oturumda tekrarlıyor.
            let m = aracSizintisiniTemizle(duz(r.segments))
            return m.isEmpty ? nil : "Asistan: " + kirp(m, 400)
        default:
            return nil
        }
    }

    private static func kirp(_ metin: String, _ sinir: Int) -> String {
        metin.count <= sinir ? metin : String(metin.suffix(sinir))
    }

    // MARK: - Araç-çağrısı sızıntısı süzgeci

    /// Ayrıştırılamamış araç-çağrısı yükünü metinden ayıklar.
    ///
    /// FoundationModels çağrı bloğunu tanıyamadığında onu düz METİN segmenti
    /// olarak geçiriyor; süzgeç olmadan `{"name": "hesapla", …}<executable_end>`
    /// doğrudan kullanıcıya gidiyordu. Daha kötüsü kendini besliyordu: sızıntı
    /// bir `.response` metni olduğu için `ozetle()` onu özete taşıyor, model
    /// yeni oturumda kendi geçmişinde araç-çağrısı sözdizimini "geçerli asistan
    /// çıktısı" olarak görüp ALAKASIZ yükleri kopyalıyordu (laktoz sorusuna
    /// `hesapla`, gizlilik sorusuna `kisi("Ali")`). Bu yüzden süzgeç hem
    /// kullanıcıya giden metne hem `turMetni`ye uygulanır.
    ///
    /// Boş dönebilir — çağıran bunu "tur boşa gitti" olarak ele almalı.
    static func aracSizintisiniTemizle(_ metin: String) -> String {
        var m = metin
        // 1) ```function … ``` / … <executable_end> blokları (gövdesiyle birlikte).
        m = sil("(?s)```[ \\t]*function\\b.*?(?:```|<executable_end>|\\z)", m)
        // 2) Çıplak JSON araç çağrısı, tekil ya da [ … ] dizisi içinde.
        m = sil("(?s)\\[?\\s*\\{\\s*\"name\"\\s*:\\s*\"[^\"]*\"\\s*,\\s*\"arguments\"\\s*:\\s*\\{.*?\\}\\s*\\}\\s*\\]?", m)
        m = sil("<executable_(?:end|start)>", m)
        // 3) Yetim kapanış artıkları YALNIZCA yukarıda gerçekten bir çağrı
        //    soyulduysa temizlenir. Koşulsuz uygulanırsa MEŞRU bir markdown
        //    kod bloğunun kapanış ``` satırını da silip yanıtı bozardı —
        //    kod becerisi tam olarak böyle bloklar üretiyor.
        if m != metin {
            m = sil("(?m)^\\s*(?:\\]|```)\\s*$", m)
        }
        return m.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func sil(_ desen: String, _ metin: String) -> String {
        guard let re = try? NSRegularExpression(pattern: desen) else { return metin }
        return re.stringByReplacingMatches(in: metin,
                                           range: NSRange(metin.startIndex..., in: metin),
                                           withTemplate: "")
    }
}

// MARK: - MCP araç köprüsü (mcp §5.2, §5.4, §5.5)

/// Kayıtlı bir bağlantıyı çalışır `MCPAraci` örneklerine çeviren ve uzak çağrıyı
/// yürüten katman. `MCPAraci` ağ API'sine dokunmaz, yalnızca `MCPCagirici`yi
/// çağırır; ağ hâlâ tek yerde (`MCPIstemcisi`).
///
/// İstemci bağlantı başına TEK örnektir: MCP oturum kimliği (`Mcp-Session-Id`)
/// istemcinin içinde yaşıyor, her çağrıda yeni istemci kurmak her çağrıda yeni
/// el sıkışma demek olurdu.
@MainActor
final class MCPAracKoprusu: MCPCagirici {

    private struct UcNokta: Equatable { let url: URL; let anahtar: String? }

    private var ucNoktalar: [UUID: UcNokta] = [:]
    private var istemciler: [UUID: MCPIstemcisi] = [:]

    /// §5.5 sonuç işleme için — büyük çıktı modelden geçmeden buraya konur.
    weak var veriDeposu: VeriDeposu?

    /// Dış yan etki bayrağının sahibi (denetim P0-3). Uzak çağrı sunucuya
    /// ULAŞTIĞI anda `disEtkiIsaretle()` çağrılır ve o turda retry kapanır.
    weak var yurutucu: AracYurutucu?

    init() {}

    /// Bağlantının adresini kaydeder. Adres ya da anahtar değiştiyse istemci
    /// atılır: eski oturum kimliğiyle yeni sunucuya konuşulmaz.
    func kaydet(kimlik: UUID, url: URL, anahtar: String?) {
        let yeni = UcNokta(url: url, anahtar: anahtar)
        guard ucNoktalar[kimlik] != yeni else { return }
        ucNoktalar[kimlik] = yeni
        istemciler[kimlik] = nil
    }

    /// Tüm bağlantılar gitti (silindi / hiç yok): istemcileri bırak.
    func unut() {
        ucNoktalar.removeAll()
        istemciler.removeAll()
    }

    private func istemci(_ kimlik: UUID) -> MCPIstemcisi? {
        if let mevcut = istemciler[kimlik] { return mevcut }
        guard let uc = ucNoktalar[kimlik] else { return nil }
        let yeni = MCPIstemcisi(url: uc.url, anahtar: uc.anahtar)
        istemciler[kimlik] = yeni
        return yeni
    }

    // MARK: - Araç kurulumu

    /// Sunucudan şemaları okur ve oturuma girecek araçları üretir.
    ///
    /// Modele giden tanım SUNUCUNUN HAM AÇIKLAMASI DEĞİL, ekleme anında
    /// önbelleklenen özettir (§5.3) — ham açıklama 4096 pencereyi tek araçla
    /// doldurabilir. Önbellekte olmayan araç (yeni eklenmiş, henüz özetlenmemiş)
    /// bu turda atlanır; özet tazelenince gelir.
    ///
    /// Ağ erişilemezse boş dizi döner — bağlantı profili seçilemez, bugünkü
    /// davranış sürer. Uydurma araç üretilmez.
    func araclariKur(baglantiID: UUID,
                     ad: String,
                     ozetler: [AracOzeti],
                     havuz: Int,
                     cihazVerisi: CihazVerisiAyari,
                     kapi: (any OnayKapisi)?,
                     raporlayici: (any AracRaporlayici)?) async -> [MCPAraci] {
        guard let istemci = istemci(baglantiID), !ozetler.isEmpty else { return [] }
        guard let tanimlar = try? await istemci.araclar() else { return [] }

        var ozetSozlugu: [String: String] = [:]
        for ozet in ozetler where !ozet.desteklenmiyor { ozetSozlugu[ozet.ad] = ozet.ozet }

        // Sunucu sırası korunur (deterministik), önbellekte olmayan elenir ve
        // HAVUZ tavanı ÇEVİRİDEN ÖNCE uygulanır: 200 araçlık sunucuda 200 şema
        // çevirmenin anlamı yok. Havuz, oturum yuvasından (6) BİLEREK geniştir:
        // yuvayı hangi araçların dolduracağına artık burada değil, kullanıcının
        // o turki isteğine bakan `AracAlaka` karar veriyor (P1-6) ve bunun için
        // seçebileceği bir havuz gerek. Havuz da 6 olsaydı alaka sıralaması
        // sunucunun ilk 6'sını kendi içinde yeniden dizmekten öteye gidemezdi.
        // Sınıflandırma tavandan ÖNCE: 200 araçlık bir sunucuda ilk 6 araç
        // `komut_calistir`, `dosya_sil` olabilir ve saf sunucu sırası bu
        // durumda oturumu yıkıcı araçlarla doldurup salt-okumaları dışarıda
        // bırakır. Kararlı sıralama (`enumerate` ile bağ bozma) sunucu sırasını
        // sınıf içinde korur, yani davranış hâlâ deterministik.
        let suzulmus = tanimlar.filter { ozetSozlugu[$0.ad] != nil }
        let siniflar = suzulmus.map {
            YanEtkiSinifi.sinifla(ad: $0.ad,
                                  ozet: ozetSozlugu[$0.ad] ?? "",
                                  saltOkumaIpucu: $0.saltOkumaIpucu,
                                  yikiciIpucu: $0.yikiciIpucu)
        }
        let adaylar = zip(suzulmus, siniflar).enumerated()
            .sorted { sol, sag in
                let solYikici = sol.element.1.onayZorunluMu
                let sagYikici = sag.element.1.onayZorunluMu
                if solYikici != sagYikici { return !solYikici }
                return sol.offset < sag.offset
            }
            .map(\.element)
            .prefix(havuz)

        // Ad çakışması koleksiyon düzeyinde çözülür (P2-9). "get-user" ve
        // "get_user" ikisi de `get_user`a indirgeniyordu ve model iki araçtan
        // hangisini çağırdığını bilmiyordu; `adlariCoz` sırayı bozmadan
        // ikisine FARKLI ad verir.
        let cozulmusAdlar = MCPAraci.adlariCoz(adaylar.map { (uzakAd: $0.0.ad, sunucu: ad) })

        var araclar: [MCPAraci] = []
        for (sira, (tanim, yanEtki)) in adaylar.enumerated() {
            // Şema çalışma anında çevrilir; çevrilemeyen araç ATLANIR (§5.2) —
            // yanlış argüman üretmektense araç hiç olmasın.
            guard let sema = try? MCPSemaCevirici.cevir(tanim: Self.tanimaCevir(tanim)) else { continue }
            araclar.append(MCPAraci(baglantiID: baglantiID,
                                    baglantiAdi: ad,
                                    uzakAd: tanim.ad,
                                    ozet: ozetSozlugu[tanim.ad] ?? "",
                                    parameters: sema,
                                    cagirici: self,
                                    cihazVerisi: cihazVerisi,
                                    yanEtki: yanEtki,
                                    kapi: kapi,
                                    raporlayici: raporlayici,
                                    cozulmusAd: cozulmusAdlar[sira]))
        }
        return araclar
    }

    /// İstemci tanımı → şema çevirisinin beklediği ham tanım.
    /// Şemasız araç = argümansız araç: boş nesne şeması verilir, araç düşmez.
    private static func tanimaCevir(_ tanim: MCPIstemcisi.AracTanimi) -> MCPAracTanimi {
        let bosNesne = Data(#"{"type":"object","properties":{}}"#.utf8)
        var veri = bosNesne
        if let sema = tanim.sema, case .nesne = sema,
           let kodlanmis = try? JSONEncoder().encode(sema) {
            veri = kodlanmis
        }
        return MCPAracTanimi(ad: tanim.ad, aciklama: tanim.aciklama, girdiSemasiJSON: veri)
    }

    // MARK: - Uzak çağrı (MCPCagirici)

    /// Onay kapısı bu çağrıdan ÖNCE `MCPAraci.call` içinde geçildi; buraya gelen
    /// her şey kullanıcının gördüğü şeydir.
    func cagir(baglantiID: UUID, aracAdi: String, argumanlarJSON: String) async throws -> MCPSonucu {
        guard let istemci = istemci(baglantiID) else {
            throw MCPIstemcisi.MCPHatasi.erisilemedi
        }
        // Model şemaya uygun JSON üretir; yine de ayrıştırılamayan girdiyi
        // uydurmayız — argümansız çağrıya ineriz.
        let argumanlar = JSONDeger.ayristir(argumanlarJSON) ?? .nesne([:])
        let (metin, hataliMi) = try await istemci.aracCagir(ad: aracAdi, argumanlar: argumanlar)

        // BURASI P0-3'ÜN TEK NOKTASI. Çağrı DÖNDÜ demek, istek sunucuya ulaştı
        // ve sunucu onu işledi demektir — issue açıldı, kayıt yazıldı, e-posta
        // gitti olabilir. Bundan sonra bu turda AYNI istemi ikinci kez
        // göndermek geri alınamaz bir tekrar üretir.
        //
        // `hataliMi` AYIRMAZ ve ayırmamalı: MCP'de `isError` sunucunun aracın
        // sonucu hakkındaki yorumudur, işlemin hiç gerçekleşmediğinin kanıtı
        // değil ("issue açıldı ama alan doğrulaması geçmedi" de isError döner).
        // Yalnız `throw` eden yol (taşıma hatası) bayrağı kurmaz, çünkü orada
        // istek sunucuya ulaşmamıştır.
        yurutucu?.disEtkiIsaretle()

        // §5.5: ham çıktı modele girmez; özet + kaynakRef gider, tamamı çipte kalır.
        let islenmis = BaglantiServisi.sonucIsle(metin, aracAdi: aracAdi, veriDeposu: veriDeposu)
        let govde = islenmis.modeleDonen.trimmingCharacters(in: .whitespacesAndNewlines)

        if hataliMi {
            // Sunucunun KENDİ hatası (komut başarısız) — taşıma hatası değil.
            // Model bunu okuyup anlatır; çip de "hata döndü" der, sessiz geçilmez.
            return MCPSonucu(
                cipDetayi: String(localized: "\(aracAdi) hata döndü"),
                modeleDonen: govde.isEmpty
                    ? "remote_tool_error: the tool failed on the user's server without a message. Say this in one sentence."
                    : "remote_tool_error: \(govde)",
                hamCikti: islenmis.hamCikti)
        }
        return MCPSonucu(
            cipDetayi: String(localized: "\(aracAdi) tamam"),
            modeleDonen: govde.isEmpty
                ? "remote_tool_empty: the tool ran but returned nothing. Say this in one sentence; do not invent a result."
                : govde,
            hamCikti: islenmis.hamCikti)
    }
}
