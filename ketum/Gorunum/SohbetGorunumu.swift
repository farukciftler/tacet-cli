//
//  SohbetGorunumu.swift
//  ketum
//
//  Tek sürekli sohbet akışı (spec §8 v1). Bileşenleri, ModelServisi'ni ve
//  SwiftData geçmişini birleştirir. Araç çipleri ilgili yanıtın üstüne düşer.
//

import Foundation
import SwiftUI
import SwiftData
import UniformTypeIdentifiers

struct SohbetGorunumu: View {
    let sohbet: Sohbet
    let servis: ModelServisi
    var gecmisAc: () -> Void = {}
    var yeniSohbet: () -> Void = {}

    @Environment(\.modelContext) private var kayit
    @Environment(\.accessibilityReduceMotion) private var azHareket

    /// Kayıtlı MCP bağlantıları (mcp §5.4). Görünüm bunlarla hiçbir şey ÇİZMEZ —
    /// SwiftData'yı bilen katman burası olduğu için araçları servise buradan
    /// besliyoruz (`NobetBaglami.kayit` deseninin aynısı). Liste değişince
    /// (ekleme/silme/özet tazeleme) araçlar da tazelenir.
    @Query private var baglantilar: [Baglanti]

    @State private var soru = ""
    @State private var canliYanit = ""
    @State private var yaniyor = false
    @State private var belgeSecici = false
    @State private var onizleme: OnizlemeOgesi?
    /// Uçuştaki yanıt görevi. Sohbet değişince/görünüm kapanınca iptal edilir;
    /// aksi halde yapısız Task silinmiş bir Sohbet nesnesine yazmayı deneyebilir.
    @State private var yanitGorevi: Task<Void, Never>?
    /// Kullanıcıya gösterilecek işlem hatası (belge kopyalama, kayıt, dışa aktarma).
    /// Sessizce yutulan `try?`ların yerini alır.
    @State private var uyariMetni: String?
    /// Yanıt tamamlandığında haptik vermek için sayaç.
    @State private var tamamlanmaSayaci = 0
    @FocusState private var girisOdakli: Bool

    /// Süren turun seyri (seyir-spec §5.2). TEK KAYDEDİCİ: sahibi `ModelServisi`,
    /// görünüm onu yalnızca izler ve olay bağlar. Eskiden burada ayrı bir
    /// `@State` örnek vardı; servisin kaydedicisiyle ikisi paralel yaşıyor ve
    /// adımları ayrışabiliyordu (tek doğruluk kaynağı ihlali).
    private var kaydedici: SeyirKaydedici { servis.seyir }

    /// Bağlantı listesinin anlamlı imzası: kimlik + oturuma girebilecek araç
    /// adları + cihaz verisi ayarı. Yalnızca bunlar değişince araçlar yeniden
    /// kurulur; "son kullanım" gibi her çağrıda değişen alanlar imzaya girmez.
    /// Cihaz verisi ayarı imzada OLMAK ZORUNDA: kullanıcı BaglantiDetayi'nde
    /// "hiçbir zaman" ↔ "her seferinde sor" arasında geçiş yaptığında araçlar
    /// yeniden kurulmazsa eski MCPAraci örnekleri eski ayarla oturumda kalır.
    private var baglantiImzasi: [String] {
        baglantilar.map {
            "\($0.id)·\($0.cihazVerisi.rawValue)·\($0.kullanilabilirAraclar.map(\.ad).joined(separator: ","))"
        }
    }

    /// Şu an üretim sürüyor mu — servis tek doğruluk kaynağı, canlı akış yerel bayrak.
    private var uretimVar: Bool { servis.uretiyor || yaniyor }

    /// Aktif sohbetin zamana göre sıralı mesajları.
    private var mesajlar: [Mesaj] { sohbet.siraliMesajlar }

    /// QuickLook sheet'i için Identifiable URL sarmalayıcısı.
    private struct OnizlemeOgesi: Identifiable { let url: URL; var id: String { url.path } }

    /// Okuma/düzenleme için desteklenen belge türleri.
    private var belgeTurleri: [UTType] {
        [.pdf, .plainText, .text,
         UTType(filenameExtension: "xlsx") ?? .data,
         UTType(filenameExtension: "docx") ?? .data,
         UTType(filenameExtension: "md") ?? .plainText]
    }

    var body: some View {
        VStack(spacing: 0) {
            VStack(spacing: 0) {
                UstBar(durum: servis.durum, aciklama: servis.engelMesaji,
                       gecmisAc: gecmisAc, yeniSohbet: yeniSohbet)

                if mesajlar.isEmpty && !yaniyor {
                    BosDurum { ornek in
                        soru = ornek
                        girisOdakli = true
                    }
                    .frame(maxHeight: .infinity)
                } else {
                    akis
                }
            }
            // Giriş alanı dışında bir yere dokunmak klavyeyi kapatır; yazılan metin
            // `soru`da durduğu için kaybolmaz. Düğmeler öncelikli olduğundan
            // (simultaneous değil) örnek seçme ve çipler etkilenmez.
            .contentShape(Rectangle())
            .onTapGesture { girisOdakli = false }

            VStack(spacing: Olcek.s2) {
                if let belge = servis.belgeBaglami.aktifBelge {
                    EkliBelgeCipi(belge: belge) {
                        servis.belgeBaglami.belgeKaldir()
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, Olcek.s5)
                    .transition(azHareket ? .opacity : .opacity.combined(with: .offset(y: 2)))
                }
                GirisAlani(metin: $soru, gonder: gonder,
                           ekle: { belgeSecici = true },
                           uretiyor: uretimVar,
                           durdur: durdurBas)
                    .focused($girisOdakli)
            }
            .padding(.vertical, Olcek.s2)
        }
        .background(Renk.zemin)
        .task {
            servis.hazirla()   // sohbet görünür olunca modeli ısıt (prewarm, rapor §5.1)
            // MCP araçlarını oturuma bağla. Bağlantı yoksa ağa hiç çıkılmaz.
            servis.baglantilariTazele(baglantilar)
        }
        .onChange(of: baglantiImzasi) { _, _ in servis.baglantilariTazele(baglantilar) }
        // Yürütücüye düşen her yeni çip bir araç adımıdır (seyir-spec §5.2).
        // Araç katmanına dokunmadan, tek doğruluk kaynağını izleyerek bağlanır.
        // Bu, spec'in istediği OLAY tabanlı bağlamadır: `AracYurutucu.baslat`
        // çipi düşürdüğü an tetiklenir, parça sınırını beklemez.
        .onChange(of: servis.yurutucu.izler.map(\.id)) { _, _ in aracAdimlariEsitle() }
        // İlk parça geldiğinde yazım adımı açılır — parça başına adım yok.
        .onChange(of: canliYanit.isEmpty) { _, bosMu in
            if !bosMu { yazimAdimi() }
        }
        .onChange(of: sohbet.id) { _, _ in gorevIptal() }
        .onDisappear { gorevIptal() }
        .fileImporter(isPresented: $belgeSecici,
                      allowedContentTypes: belgeTurleri,
                      allowsMultipleSelection: false) { sonuc in
            belgeSecildi(sonuc)
        }
        .sheet(item: $onizleme, onDismiss: {
            // Sıfırlanmazsa aynı belge ikinci kez üretildiğinde onChange tetiklenmez.
            servis.belgeBaglami.onizlenecek = nil
        }) { oge in
            BelgeOnizlemeSheet(url: oge.url)
        }
        .onChange(of: servis.belgeBaglami.onizlenecek) { _, yeni in
            if let yeni { onizleme = OnizlemeOgesi(url: yeni) }
        }
        .sensoryFeedback(.success, trigger: tamamlanmaSayaci)
        .alert(Text("Bir sorun oldu"), isPresented: Binding(
            get: { uyariMetni != nil },
            set: { if !$0 { uyariMetni = nil } }
        )) {
            Button(role: .cancel) { uyariMetni = nil } label: { Text("Tamam") }
        } message: {
            if let uyariMetni { Text(uyariMetni) }
        }
    }

    /// SwiftData yazma hatasını kullanıcıya görünür kılar. `try? save()` sessizce
    /// yutulduğunda mesaj ekranda görünür ama uygulama kapanınca kaybolurdu.
    private func kaydet() {
        do {
            try kayit.save()
        } catch {
            uyariMetni = String(localized: "Sohbet kaydedilemedi: \(error.localizedDescription)")
        }
    }

    /// Seçilen belgeyi güvenli kapsamdan uygulama alanına kopyalayıp aktif eder.
    private func belgeSecildi(_ sonuc: Result<[URL], Error>) {
        guard case .success(let urller) = sonuc, let kaynak = urller.first else { return }
        let erisim = kaynak.startAccessingSecurityScopedResource()
        defer { if erisim { kaynak.stopAccessingSecurityScopedResource() } }

        // Kullanıcının KENDİ belgesi buradan geçiyor — en hassas yol. Alt klasör de
        // koruma sınıfıyla yaratılır ve yedekten çıkarılır; iOS miras verse de
        // kopyalanan dosyaya sınıf açıkça basılır (kaynak dosyanın öznitelikleri
        // kopyayla birlikte gelebilir).
        let hedefKlasor = BelgeBaglami.ciktiKlasoru().appendingPathComponent("Ekli", isDirectory: true)
        try? FileManager.default.createDirectory(
            at: hedefKlasor, withIntermediateDirectories: true,
            attributes: [.protectionKey: FileProtectionType.completeUnlessOpen])
        BelgeBaglami.korumayiUygula(hedefKlasor)
        let hedef = hedefKlasor.appendingPathComponent(kaynak.lastPathComponent)
        try? FileManager.default.removeItem(at: hedef)
        do {
            try FileManager.default.copyItem(at: kaynak, to: hedef)
            BelgeBaglami.korumayiUygula(hedef)
            withAnimation(azHareket ? nil : .easeOut(duration: 0.2)) {
                servis.belgeBaglami.belgeEkle(url: hedef)
            }
        } catch {
            // Sessizce geçilirse kullanıcı dosyayı eklediğini sanır — görünür hata ver.
            uyariMetni = String(localized: "Belge eklenemedi: \(error.localizedDescription)")
        }
    }

    // MARK: - Akış

    private var akis: some View {
        ScrollViewReader { okuyucu in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: Olcek.mesajAraligi) {
                    ForEach(Array(mesajlar.enumerated()), id: \.element.id) { indeks, mesaj in
                        if gunAyraciGerek(indeks) {
                            TarihAyraci(tarih: mesaj.olusturulma)
                                .frame(maxWidth: .infinity)
                        }
                        satir(mesaj)
                            .id(mesaj.id)
                    }

                    if yaniyor {
                        canliBlok
                            .id(canliKimlik)
                    }
                }
                .padding(.horizontal, Olcek.s5)
                .padding(.vertical, Olcek.s4)
            }
            .scrollDismissesKeyboard(.interactively)
            .onChange(of: mesajlar.count) { _, _ in dibeKay(okuyucu) }
            .onChange(of: canliYanit) { _, _ in dibeKay(okuyucu) }
            .onChange(of: yaniyor) { _, yeni in if yeni { dibeKay(okuyucu) } }
        }
    }

    @ViewBuilder
    private func satir(_ mesaj: Mesaj) -> some View {
        switch mesaj.rol {
        case .sen:
            KullaniciBalonu(metin: mesaj.icerik)
                .frame(maxWidth: .infinity, alignment: .trailing)
        case .ketum:
            // Adım verisi olan mesajda okuma çipleri katlamanın içine girer;
            // olmayanda (eski mesaj) çipler bugünkü gibi tümü görünür.
            let seyirVar = SeyirKatlama.satirGosterilirMi(mesaj.adimlar)
            VStack(alignment: .leading, spacing: Olcek.cipYanitAraligi) {
                if seyirVar {
                    SeyirCizgisi(adimlar: mesaj.adimlar, izler: mesaj.izler)
                }
                ForEach(YanitIzleri.cipler(mesaj.izler, seyirVar: seyirVar)) { iz in
                    AracCipi(iz: iz)
                        .transition(cipGecisi)
                }
                if !mesaj.icerik.isEmpty || !YanitIzleri.kartlar(mesaj.izler).isEmpty {
                    KetumYaniti(metin: mesaj.icerik,
                                tabloIndir: tabloIndir,
                                hataMi: mesaj.hataMi,
                                yenidenDene: mesaj.hataMi && mesaj.tekrarDenenebilir
                                    ? { yenidenDene(mesaj) } : nil,
                                dosyaIzleri: YanitIzleri.kartlar(mesaj.izler))
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    /// Canlı (streaming) asistan bloğu: yan etki çipleri + seyir şeridi + akan metin.
    ///
    /// Şerit önce yanıt balonunun YERİNDE durur (henüz metin yok); yazım başlayıp
    /// metin akmaya başlayınca aynı yerde kalarak yanıtın ÜSTÜNE çekilmiş olur
    /// (seyir-spec §3.1).
    private var canliBlok: some View {
        let izler = servis.yurutucu.izler
        let seritVar = !kaydedici.adimlar.isEmpty
        return VStack(alignment: .leading, spacing: Olcek.cipYanitAraligi) {
            ForEach(YanitIzleri.canliCipler(izler, seritVar: seritVar)) { iz in
                AracCipi(iz: iz, yurutucu: servis.yurutucu)
                    .transition(cipGecisi)
            }
            if seritVar {
                SeyirSeridi(adimlar: kaydedici.adimlar, izler: izler)
            }
            // Şerit varken ve henüz metin yokken nefes noktası çizilmez —
            // yanıtın yerini şerit tutuyor, iki canlı gösterge yan yana durmaz.
            if !seritVar || !canliYanit.isEmpty {
                KetumYaniti(metin: canliYanit, akiyorMu: true,
                            dosyaIzleri: YanitIzleri.kartlar(izler))
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private let canliKimlik = "canli-blok"

    // MARK: - Eylem

    private func gonder() {
        let metin = soru.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !metin.isEmpty, !uretimVar else { return }
        soru = ""

        let kullanici = Mesaj(rol: .sen, icerik: metin)
        kullanici.sohbet = sohbet
        withAnimation(azHareket ? nil : .easeOut(duration: 0.2)) {
            kayit.insert(kullanici)
        }
        // İlk kullanıcı mesajından sohbet başlığını türet (bayrak üstünden —
        // metin karşılaştırması çevirilerde bozuluyordu).
        sohbet.basligiTuret(metin)
        sohbet.guncelleme = Date()
        kaydet()

        uret(metin)
    }

    /// Hata balonundaki "Yeniden dene": hata kaydını silip aynı soruyu tekrar sorar.
    /// Kullanıcı balonu yeniden eklenmez.
    private func yenidenDene(_ hataMesaji: Mesaj) {
        guard !uretimVar else { return }
        let sirali = mesajlar
        guard let indeks = sirali.firstIndex(where: { $0.id == hataMesaji.id }) else { return }
        // SwiftData tuzağı: silmeden ÖNCE gereken değeri hesapla.
        guard let sorulan = sirali[..<indeks].last(where: { $0.rol == .sen })?.icerik else { return }

        withAnimation(azHareket ? nil : .easeOut(duration: 0.2)) {
            kayit.delete(hataMesaji)
        }
        kaydet()
        uret(sorulan)
    }

    /// Yanıt üretimini başlatır. Uçuştaki görev `yanitGorevi`de tutulur; sohbet
    /// değişince iptal edilir, aksi halde arkadaki görev eski sohbete yazar.
    private func uret(_ metin: String) {
        yaniyor = true
        canliYanit = ""
        kaydedici.sifirla()

        yanitGorevi?.cancel()
        yanitGorevi = Task {
            let sonuc = await servis.yanitSonucu(metin) { kismi in
                guard !Task.isCancelled else { return }
                canliYanit = kismi
            }
            // Yazmadan önce: görev iptal edildi mi, sohbet hâlâ geçerli mi?
            // Geçmiş temizlenmiş veya sohbet silinmişse sessizce çık — öksüz
            // ya da kısmi kayıt yazmak SwiftData'da ölümcül hataya yol açar.
            guard !Task.isCancelled, !sohbet.isDeleted, sohbet.modelContext != nil else {
                kaydedici.kes()
                yaniyor = false
                canliYanit = ""
                return
            }

            // Geçici durum bildirimi kalıcı yanıt değildir — kaydetme.
            if sonuc.geciciMi {
                kaydedici.kes()
                yaniyor = false
                canliYanit = ""
                return
            }

            kaydedici.bitir()
            // Hata bilgisi servisten BAYRAK olarak gelir; metin karşılaştırması yok.
            let yanit = Mesaj(rol: .ketum, icerik: sonuc.metin,
                              izler: sonuc.izler,
                              adimlar: SeyirDefteri.kalici(kaydedici.adimlar,
                                                           izler: sonuc.izler),
                              hataMi: sonuc.hataMi,
                              tekrarDenenebilir: sonuc.tekrarDenenebilir)
            yanit.sohbet = sohbet
            withAnimation(azHareket ? nil : .easeOut(duration: 0.2)) {
                kayit.insert(yanit)
                yaniyor = false
                canliYanit = ""
            }
            sohbet.guncelleme = Date()
            kaydet()
            tamamlanmaSayaci += 1
        }
    }

    /// Durdur düğmesi: üretimi iptal eder, o ana kadar akan metni kaybetmemek için
    /// kısmi yanıtı kaydeder.
    private func durdurBas() {
        servis.durdur()
        let kismi = canliYanit.trimmingCharacters(in: .whitespacesAndNewlines)
        // Kesinti sessizce kaybolmaz: son adım "yarıda kaldı" olarak kapanır ve
        // katlama satırında görünür (seyir-spec §3.4).
        kaydedici.kes()
        let adimlar = kaydedici.adimlar
        yanitGorevi?.cancel()
        yanitGorevi = nil
        withAnimation(azHareket ? nil : .easeOut(duration: 0.2)) {
            yaniyor = false
            canliYanit = ""
        }
        guard !kismi.isEmpty, !sohbet.isDeleted, sohbet.modelContext != nil else { return }
        let izler = servis.yurutucu.izler
        let yanit = Mesaj(rol: .ketum, icerik: kismi, izler: izler,
                          adimlar: SeyirDefteri.kalici(adimlar, izler: izler))
        yanit.sohbet = sohbet
        kayit.insert(yanit)
        sohbet.guncelleme = Date()
        kaydet()
    }

    /// Uçuştaki yanıt görevini iptal edip canlı akış durumunu sıfırlar.
    private func gorevIptal() {
        // Dış görevi iptal etmek yetmez: ModelServisi.uretimGorevi çalışmaya devam
        // eder, `uretiyor` true takılır ve yeni sohbette gönder düğmesi donardı.
        servis.durdur()
        // Kesilen tur kayda geçer; kaydedici kapanır ki yarım adım açık kalmasın.
        // Üretim yokken çağrılırsa (görünüm kapanışı) kesinti adımı uydurulmaz.
        if yaniyor { kaydedici.kes() }
        yanitGorevi?.cancel()
        yanitGorevi = nil
        yaniyor = false
        canliYanit = ""
    }

    // MARK: - Seyir

    /// Yürütücüye düşen yeni çipleri araç adımı olarak bağlar. Yalnızca EKLER —
    /// kaydedici geçmişi yeniden yazmaz; kaybolan iz (kabul edilen onay çipi)
    /// gösterim ve kalıcılık aşamasında elenir.
    ///
    /// `ModelServisi.seyriEsitle` aynı işi parça sınırlarında yapar (görünümsüz
    /// çağrılar için ağ emniyeti). İkisi de `izID`ye göre tekilleştirdiği ve
    /// AYNI kaydediciye yazdığı için çift adım oluşmaz.
    private func aracAdimlariEsitle() {
        guard yaniyor else { return }
        let bagli = Set(kaydedici.adimlar.compactMap(\.aracIziID))
        for iz in servis.yurutucu.izler where !bagli.contains(iz.id) {
            kaydedici.aracBagla(izID: iz.id)
        }
    }

    /// İlk metin parçası geldiğinde yazım adımını açar. Zaten açıksa dokunmaz —
    /// yazım tek adımdır, parça başına adım üretilmez (seyir-spec §5.4).
    private func yazimAdimi() {
        guard yaniyor else { return }
        guard kaydedici.adimlar.last?.tur != .yazim else { return }
        kaydedici.basla(tur: .yazim, metin: SeyirDefteri.yaziyorMetni)
    }

    /// Sohbet içi tablodan "Excel indir": tabloyu .xlsx üretip önizlemeyi açar.
    private func tabloIndir(_ tablo: Tablo) {
        do {
            let url = try ExcelMotor().yaz(dosyaAdi: "tablo", baslik: nil,
                                           govde: nil, tablo: tablo,
                                           klasor: BelgeBaglami.ciktiKlasoru())
            // Diğer üretim yollarıyla aynı koruma: kilitliyken okunamaz + yedek dışı.
            BelgeBaglami.korumayiUygula(url)
            onizleme = OnizlemeOgesi(url: url)
        } catch {
            // Sessizce dönülürse düğmeye basılır, hiçbir şey olmaz.
            uyariMetni = String(localized: "Tablo indirilemedi: \(error.localizedDescription)")
        }
    }

    // MARK: - Yardımcılar

    private func gunAyraciGerek(_ indeks: Int) -> Bool {
        guard indeks < mesajlar.count else { return false }
        if indeks == 0 { return true }
        let onceki = mesajlar[indeks - 1].olusturulma
        let simdi = mesajlar[indeks].olusturulma
        return !Calendar.current.isDate(onceki, inSameDayAs: simdi)
    }

    private var cipGecisi: AnyTransition {
        azHareket ? .opacity
                  : .opacity.combined(with: .offset(y: 2))
    }

    private func dibeKay(_ okuyucu: ScrollViewProxy) {
        let hedef: AnyHashable = yaniyor ? AnyHashable(canliKimlik) : AnyHashable(mesajlar.last?.id)
        withAnimation(azHareket ? nil : .easeOut(duration: 0.2)) {
            okuyucu.scrollTo(hedef, anchor: .bottom)
        }
    }
}

// MARK: - Seyir defteri (saf fonksiyonlar, OtoTest edilir)

/// Canlı adım listesini mesaja yazılacak kalıcı listeye çevirir.
enum SeyirDefteri {
    /// Canlı yazım adımının metni — şimdiki gerçek zaman.
    static var yaziyorMetni: String { String(localized: "yazıyor") }
    /// Bitmiş turda aynı adım geçmiş zamana döner; dramatize yok, yalnızca zaman.
    static var yazildiMetni: String { String(localized: "yazıldı") }

    /// - İzi kaybolan araç adımı düşer: kabul edilen onay çipini yürütücü akıştan
    ///   siler (mcp §3.3); metinsiz, tıklanamaz bir hayalet satır bırakmayız.
    /// - Yazım adımı geçmiş zamana çevrilir.
    static func kalici(_ adimlar: [SeyirAdimi], izler: [AracIzi]) -> [SeyirAdimi] {
        let mevcut = Set(izler.map(\.id))
        return adimlar.compactMap { adim in
            if adim.tur == .arac {
                guard let id = adim.aracIziID, mevcut.contains(id) else { return nil }
                return adim
            }
            if adim.tur == .yazim, adim.metin == yaziyorMetni {
                var kopya = adim
                kopya.metin = yazildiMetni
                return kopya
            }
            return adim
        }
    }
}

#Preview {
    let kap = try! ModelContainer(for: Sohbet.self, Mesaj.self, Baglanti.self,
                                  configurations: .init(isStoredInMemoryOnly: true))
    let s = Sohbet(baslik: "Örnek")
    kap.mainContext.insert(s)
    return SohbetGorunumu(sohbet: s, servis: ModelServisi())
        .modelContainer(kap)
}
