//
//  ContentView.swift
//  ketum
//
//  Kök görünüm. Aktif sohbeti, paylaşılan ModelServisi'ni ve geçmiş listesini yönetir.
//  Yeni sohbet açma ve eskilere erişim buradan (spec §4.7).
//

import SwiftUI
import SwiftData

struct ContentView: View {
    @Environment(\.modelContext) private var kayit
    @Environment(\.scenePhase) private var sahne
    @Query(sort: \Sohbet.guncelleme, order: .reverse) private var sohbetler: [Sohbet]

    @Query private var nobetler: [Nobet]
    @Query private var beceriler: [KullaniciBecerisi]
    /// Hafıza notları (hafiza-spec §5): okuma yolu depoyu bu diziden tazeler.
    @Query private var notlar: [HafizaNotu]

    @State private var servis = ModelServisi()
    /// Ayıklama (yazma yolu) tek örnekten geçer: art arda tetiklerde en fazla
    /// bir oturum açılması bu örneğin içindeki korumaya bağlı (hafiza-spec §4.1).
    @State private var hafiza = HafizaServisi()
    /// Bağlantı panosunun beklediği sözleşmenin karşılığı — bkz. dosya sonu.
    @State private var baglantiKoprusu = BaglantiKoprusu()
    @State private var aktifID: UUID?
    /// Tek sunum kanalı: aynı anda yalnız bir sayfa açık olabilir. Ayrı ayrı
    /// `isPresented` bayrakları kullanıldığında "kapat + aç" aynı runloop turuna
    /// düştüğü için ikinci sunum sessizce düşüyordu.
    @State private var sayfa: Sayfa?
    /// Nöbet tazeleme sürüyor mu — eş zamanlı ikinci akışı engeller.
    @State private var tazeleniyor = false
    /// Yazma/çalıştırma hatasının kullanıcıya görünen karşılığı. Nil ise uyarı yok.
    @State private var uyariMetni: String?
    /// Depo durumu: bozuk mağaza yedeği bir kez okunup kapatılabilir.
    @State private var yedekBildirimiKapandi = false

    /// @MainActor: DepoDurumu ana aktörde yaşıyor; View'ın gövde dışı üyeleri
    /// izole değil, bu yüzden erişen yardımcılar açıkça işaretlendi.
    @MainActor private var depo: DepoDurumu { DepoDurumu.paylasilan }

    /// Aktif sohbet — seçili yoksa en son güncellenen.
    private var aktif: Sohbet? {
        if let aktifID, let bulunan = sohbetler.first(where: { $0.id == aktifID }) {
            return bulunan
        }
        return sohbetler.first
    }

    var body: some View {
        VStack(spacing: 0) {
            // Depoyla ilgili olağandışı durumlar sohbetin ÜSTÜNDE durur: kullanıcı
            // yazmaya başlamadan önce oturumun kaydedilip kaydedilmediğini bilmeli.
            depoUyarilari
            Group {
                if let aktif {
                    SohbetGorunumu(
                        sohbet: aktif,
                        servis: servis,
                        gecmisAc: { sayfa = .gecmis },
                        yeniSohbet: yeniSohbetBaslat
                    )
                    .id(aktif.id)   // sohbet değişince görünüm durumu (soru, akış) sıfırlanır
                } else {
                    Color.clear.onAppear(perform: ilkSohbetiSagla)
                }
            }
        }
        .background(Renk.zemin)
        .task {
            // Kullanıcı becerilerini modele açılışta tanıt (pano kaydettikçe de tazeler).
            BeceriDeposu.kullaniciyiYenile(beceriler)
            // Bağlantı panosu SwiftData'ya köprü üzerinden yazar.
            baglantiKoprusu.kayit = kayit
            // Depo açılışta boş: doldurulmazsa ilk tur enjeksiyonsuz geçerdi.
            HafizaDeposu.yenile(notlar)
            servis.availabilityYenile()
            await nobetleriTazele()
        }
        .onChange(of: sahne) { _, yeni in
            guard yeni == .active else {
                // Arka plana geçiş, ayıklamanın iki tetiğinden biri (hafiza-spec §4.1).
                // Sohbet turunun İÇİNDE asla çalışmaz; burası tur dışıdır.
                hafiza.tetikle(sohbet: aktif, kayit: kayit)
                return
            }
            // Uygulama arka plandan öne gelince de tazele — .task yalnız ilk kez çalışır.
            // Model indirmesi arka planda bittiyse yeniden başlatmadan hazır olsun.
            servis.availabilityYenile()
            // Arka planda ayıklanmış notlar okuma yoluna girsin.
            HafizaDeposu.yenile(notlar)
            Task { await nobetleriTazele() }
        }
        .sheet(item: $sayfa) { acik in
            switch acik {
            case .gecmis:
                SohbetListesi(
                    sohbetler: sohbetler,
                    aktifID: aktif?.id,
                    sec: { s in
                        // Sohbet değişimi ayıklamanın diğer tetiği: AYRILAN sohbet
                        // işlenir, seçilen değil. Sıra önemli — aktifID değişmeden.
                        hafiza.tetikle(sohbet: aktif, kayit: kayit)
                        aktifID = s.id
                        servis.sohbetiSifirla()
                        sayfa = nil
                    },
                    sil: silSohbet,
                    yeni: {
                        sayfa = nil
                        yeniSohbetBaslat()
                    },
                    // Tek atama: liste kapanır, hedef sayfa aynı geçişte sunulur.
                    nobetlerAc: { sayfa = .nobetler },
                    becerilerAc: { sayfa = .beceriler },
                    hafizaAc: { sayfa = .hafiza },
                    ayarlarAc: { sayfa = .ayarlar }
                )
            case .nobetler:
                NobetPanosu(servis: servis)
            case .beceriler:
                BeceriPanosu()
            case .hafiza:
                HafizaPanosu()
            case .baglantilar:
                BaglantiPanosu(servis: baglantiKoprusu)
            case .ayarlar:
                Ayarlar(gecmisiTemizle: gecmisiTemizle,
                        belgeBaglami: servis.belgeBaglami,
                        baglantilarAc: { sayfa = .baglantilar })
            }
        }
        .alert(Text("Bir sorun oldu"), isPresented: Binding(
            get: { uyariMetni != nil },
            set: { if !$0 { uyariMetni = nil } }
        )) {
            Button(role: .cancel) { uyariMetni = nil } label: { Text("Tamam") }
        } message: {
            if let uyariMetni { Text(uyariMetni) }
        }
    }

    // MARK: - Depo uyarıları

    /// Kalıcı depoyla ilgili iki durum. Rozet/nokta yok: durum sözle anlatılır.
    @MainActor @ViewBuilder
    private var depoUyarilari: some View {
        // KALICI uyarı: bu oturum diske yazılmıyor. Kapatılamaz, çünkü kullanıcının
        // her an bilmesi gereken bir gerçek — kapanınca yazdığı her şey kaybolacak.
        if depo.oturumKaliciDegil {
            depoSeridi(
                baslik: Text("Bu oturum kaydedilmiyor."),
                aciklama: Text("Depo açılamadı. Yazdıkların yalnızca ekranda duruyor; sirr'i kapatınca bu sohbetler kaybolur."),
                vurgulu: true
            )
        }

        // Bilgilendirme: eski veri silinmedi, yedeklendi. Bir kez okunup kapatılır.
        if let yedek = depo.yedeklenenMagaza, !yedekBildirimiKapandi {
            depoSeridi(
                baslik: Text("Eski verilerin yedeklendi."),
                aciklama: Text("Önceki depo açılamadığı için “\(yedek)” adıyla saklandı ve sirr temiz bir depoyla başladı. Hiçbir şey silinmedi."),
                vurgulu: false,
                kapat: { yedekBildirimiKapandi = true }
            )
        }
    }

    @MainActor private func depoSeridi(baslik: Text,
                            aciklama: Text,
                            vurgulu: Bool,
                            kapat: (() -> Void)? = nil) -> some View {
        HStack(alignment: .top, spacing: Olcek.s3) {
            VStack(alignment: .leading, spacing: Olcek.s1) {
                baslik
                    .font(Yazi.kullanici())
                    .foregroundStyle(vurgulu ? Renk.hata : Renk.murekkep)
                aciklama
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.gri)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            if let kapat {
                Button(action: kapat) {
                    Text("Kapat")
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.gri)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, Olcek.s5)
        .padding(.vertical, Olcek.s3)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Renk.zemin)
        .overlay(alignment: .bottom) {
            Rectangle()
                .fill(Renk.cizgi)
                .frame(height: Olcek.hairline)
        }
        .accessibilityElement(children: .combine)
    }

    /// Kökten sunulabilen sayfalar. `.sheet(item:)` ile tek kanaldan gider.
    private enum Sayfa: Identifiable {
        case gecmis, nobetler, beceriler, hafiza, baglantilar, ayarlar
        var id: Self { self }
    }

    /// Nöbet bağlamına SwiftData'yı verir ve bugün çalışmamış nöbetleri hazırlar.
    /// Aynı anda tek akış çalışır: `gerekliyseCalistir` "bugün çalıştı mı" kontrolünden
    /// sonra beklediği için (izin diyaloğu dahil), koruma olmadan ikinci bir çağrı da
    /// guard'ı geçip aynı gün için ikinci kayıt yazabilirdi. Sürerken gelen çağrı atlanır.
    private func nobetleriTazele() async {
        guard !tazeleniyor else { return }
        tazeleniyor = true
        defer { tazeleniyor = false }
        servis.nobetBaglami.kayit = kayit
        do {
            try await servis.nobetBaglami.servis.gerekliyseCalistir(nobetler, kayit: kayit)
        } catch {
            uyariMetni = String(localized: "Nöbet günlüğü yazılamadı: \(error.localizedDescription)")
        }
    }

    // MARK: - Sohbet yönetimi

    private func ilkSohbetiSagla() {
        guard sohbetler.isEmpty else { return }
        bosSohbetAc()
    }

    /// Boş bir sohbet yazar ve aktif yapar. Tek yerden geçmesi önemli:
    /// geçmiş temizlendikten sonra da ekranda geçerli bir sohbet kalmalı.
    private func bosSohbetAc() {
        let yeni = Sohbet()
        kayit.insert(yeni)
        // Yazma başarısızsa geri alınmaz: sohbet bellekte kalsın ki kullanıcı
        // boş ekranda kalmasın. Geri alınsaydı ilkSohbetiSagla yeniden tetiklenip
        // aynı hatayı sonsuz döngüde gösterirdi. Kaydedilmediği uyarıyla söylenir.
        kaydet(String(localized: "Yeni sohbet kaydedilemedi"))
        aktifID = yeni.id
    }

    private func yeniSohbetBaslat() {
        // Ayrılmadan önce ayıklama tetiği (hafiza-spec §4.1). Boş sohbette
        // işlenecek mesaj yok, servis kendi içinde erken döner.
        hafiza.tetikle(sohbet: aktif, kayit: kayit)
        // Aktif sohbet zaten boşsa yeni açma — tek boş sohbet yeter.
        if let aktif, aktif.bosMu {
            aktifID = aktif.id
            servis.sohbetiSifirla()
            return
        }
        let yeni = Sohbet()
        kayit.insert(yeni)
        kaydet(String(localized: "Yeni sohbet kaydedilemedi"))
        aktifID = yeni.id
        servis.sohbetiSifirla()
    }

    private func silSohbet(_ sohbet: Sohbet) {
        // Kimliği silmeden önce al — silinen nesneye erişmek ölümcül hata.
        // Sıradaki aktif de silmeden ÖNCE hesaplanır: @Query dizisi silme anında
        // henüz tazelenmemiş olabilir, sonradan gezmek silinmiş örneğe dokunur.
        let silinenID = sohbet.id
        let siliniyorAktif = (silinenID == aktif?.id)
        let sonrakiID = sohbetler.first(where: { $0.id != silinenID })?.id
        kayit.delete(sohbet)
        do {
            try kayit.save()
        } catch {
            // Silme diske işlenmedi: bağlamı geri alıyoruz ki liste gerçeği
            // göstersin — kullanıcı silindi sanıp sohbeti kaybettiğini düşünmesin.
            kayit.rollback()
            uyariMetni = String(localized: "Sohbet silinemedi: \(error.localizedDescription) Sohbet duruyor.")
            return
        }
        if siliniyorAktif {
            aktifID = sonrakiID
            servis.sohbetiSifirla()
        }
    }

    /// Tüm sohbet geçmişini siler. Nöbetler, nöbet kayıtları, üretilmiş belgeler
    /// ve HAFIZA kalır — onları silmek kullanıcının ayrı kararı (hafiza-spec §7).
    /// Hafızayı silmek Hafıza panosunun işidir; burada notlara dokunulmaz.
    private func gecmisiTemizle() {
        // Silinecek nesneler silmeden ÖNCE tutulur: canlı @Query dizisi silme anında
        // tazelenmemiş olabilir, sonradan gezmek silinmiş örneğe dokunur (ölümcül).
        // Mesajlar Sohbet.mesajlar üzerindeki cascade kuralıyla birlikte gider.
        let silinecekler = sohbetler
        aktifID = nil
        servis.sohbetiSifirla()
        for sohbet in silinecekler { kayit.delete(sohbet) }
        do {
            try kayit.save()
        } catch {
            // En kritik sessizlik buydu: yazma başarısızken kullanıcı geçmişinin
            // silindiğini sanırdı. Geri alıp durumu açıkça söylüyoruz.
            kayit.rollback()
            uyariMetni = String(localized: "Geçmiş temizlenemedi: \(error.localizedDescription) Sohbetlerin olduğu gibi duruyor.")
            return
        }
        // Sohbetler gitti; işlenmiş mesaj imleçleri artık hiçbir şeye işaret
        // etmiyor. Notlara DOKUNMAZ, yalnızca imleç sözlüğünü boşaltır.
        HafizaServisi.imlecleriSifirla()
        // Ekranda geçerli bir durum kalsın diye hemen boş bir sohbet açılır.
        bosSohbetAc()
    }

    /// SwiftData yazma hatasını kullanıcıya görünür kılar (SohbetGorunumu.kaydet ile
    /// aynı desen). `try? save()` sessizce yutulduğunda veri kaybı fark edilmiyordu.
    private func kaydet(_ neden: String) {
        do {
            try kayit.save()
        } catch {
            uyariMetni = "\(neden): \(error.localizedDescription)"
        }
    }
}

// MARK: - Bağlantı köprüsü

/// `BaglantiPanosu` görünüm katmanına `BaglantiDeneyici` (async sonuç dönen)
/// sözleşmesini dayatır; `BaglantiServisi` ise denemeyi @Observable durum
/// (`deneme`) üzerinden yürütür ve `ekle/sil` için `ModelContext` ister.
/// İki imza uyuşmuyor ve iki dosya da bu fazda başka ajanlara ait — uyum
/// katmanı bu yüzden kök görünümün yanında, tek yerde duruyor.
///
/// Servis kendi başına bir karar vermez: yalnızca çağrı biçimini çevirir.
@MainActor
final class BaglantiKoprusu: BaglantiDeneyici {
    /// Kök görünüm açılışta verir; yoksa yazma denenmez (sessiz kayıp olmaz).
    var kayit: ModelContext?

    private let servis = BaglantiServisi()

    /// Deneme sonucunu beklerken tur başına uyku ve üst sınır. Sınır MCP
    /// istemcisinin kendi zaman aşımından uzun: normalde istemci daha önce
    /// biter, bu yalnızca "hiç sonuçlanmadı" halinde kilitlenmeyi keser.
    private static let yoklamaAraligi = Duration.milliseconds(120)
    private static let bekleyisTavani: Duration = .seconds(180)

    func dene(ad: String, urlHam: String, anahtar: String?) async -> BaglantiDenemeSonucu {
        servis.dene(urlHam: urlHam, anahtar: anahtar)
        return await sonucuBekle()
    }

    func dene(_ baglanti: Baglanti) async -> BaglantiDenemeSonucu {
        // Silinmiş kayda dokunmak ölümcül; alanlar await'ten ÖNCE okunur.
        guard !baglanti.isDeleted else {
            return .basarisiz(String(localized: "Bağlantı artık yok."))
        }
        let ad = baglanti.ad
        let urlHam = baglanti.urlHam
        let anahtar = baglanti.anahtarRefi.flatMap { AnahtarKasasi.oku(ref: $0) }
        return await dene(ad: ad, urlHam: urlHam, anahtar: anahtar)
    }

    func ekle(ad: String,
              urlHam: String,
              anahtar: String?,
              cihazVerisi: CihazVerisiAyari,
              araclar: [AracOzeti]) throws -> Baglanti {
        guard let kayit else { throw KopruHatasi.baglamYok }
        // `araclar` yok sayılır: servis, denemede okuduğu HAM tanımları saklar ve
        // özetlemeyi arka planda kendisi yapar. Görünümün elindeki kaba özetleri
        // geri yazmak, özetlenmiş sürümün üzerine eskisini koyardı.
        return try servis.ekle(ad: ad,
                           urlHam: urlHam,
                           cihazVerisi: cihazVerisi,
                           anahtar: anahtar,
                           baglam: kayit)
    }

    func anahtariKaldir(_ baglanti: Baglanti) {
        // Silme panonun işi; burada YALNIZCA Keychain kaydı kalkar.
        guard !baglanti.isDeleted, let ref = baglanti.anahtarRefi else { return }
        _ = AnahtarKasasi.sil(ref: ref)
    }

    enum KopruHatasi: LocalizedError {
        case baglamYok
        var errorDescription: String? {
            String(localized: "Bağlantı kaydedilemedi: depo hazır değil.")
        }
    }

    private func sonucuBekle() async -> BaglantiDenemeSonucu {
        var gecen: Duration = .zero
        while gecen < Self.bekleyisTavani {
            switch servis.deneme {
            case .basarili(let araclar):
                return .basarili(araclar)
            case .basarisiz(let neden):
                return .basarisiz(neden)
            case .bekliyor, .deneniyor:
                do { try await Task.sleep(for: Self.yoklamaAraligi) }
                catch { return .basarisiz(String(localized: "Deneme yarıda kaldı.")) }
                gecen += Self.yoklamaAraligi
            }
        }
        return .basarisiz(String(localized: "Sunucu yanıt vermedi."))
    }
}

#Preview {
    ContentView()
        .modelContainer(for: [Sohbet.self, Mesaj.self, Nobet.self, NobetKaydi.self,
                              KullaniciBecerisi.self, HafizaNotu.self, Baglanti.self],
                        inMemory: true)
}
