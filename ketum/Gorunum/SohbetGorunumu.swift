//
//  SohbetGorunumu.swift
//  ketum
//
//  Tek sürekli sohbet akışı (spec §8 v1). Bileşenleri, ModelServisi'ni ve
//  SwiftData geçmişini birleştirir. Araç çipleri ilgili yanıtın üstüne düşer.
//

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

    @State private var soru = ""
    @State private var canliYanit = ""
    @State private var yaniyor = false
    @State private var belgeSecici = false
    @State private var onizleme: OnizlemeOgesi?
    /// Uçuştaki yanıt görevi. Sohbet değişince/görünüm kapanınca iptal edilir;
    /// aksi halde yapısız Task silinmiş bir Sohbet nesnesine yazmayı deneyebilir.
    @State private var yanitGorevi: Task<Void, Never>?
    @FocusState private var girisOdakli: Bool

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
                UstBar(durum: servis.durum, gecmisAc: gecmisAc, yeniSohbet: yeniSohbet)

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
                GirisAlani(metin: $soru, gonder: gonder, ekle: { belgeSecici = true })
                    .focused($girisOdakli)
            }
            .padding(.vertical, Olcek.s2)
        }
        .background(Renk.zemin)
        .task { servis.hazirla() }   // sohbet görünür olunca modeli ısıt (prewarm, rapor §5.1)
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
    }

    /// Seçilen belgeyi güvenli kapsamdan uygulama alanına kopyalayıp aktif eder.
    private func belgeSecildi(_ sonuc: Result<[URL], Error>) {
        guard case .success(let urller) = sonuc, let kaynak = urller.first else { return }
        let erisim = kaynak.startAccessingSecurityScopedResource()
        defer { if erisim { kaynak.stopAccessingSecurityScopedResource() } }

        let hedefKlasor = BelgeBaglami.ciktiKlasoru().appendingPathComponent("Ekli", isDirectory: true)
        try? FileManager.default.createDirectory(at: hedefKlasor, withIntermediateDirectories: true)
        let hedef = hedefKlasor.appendingPathComponent(kaynak.lastPathComponent)
        try? FileManager.default.removeItem(at: hedef)
        do {
            try FileManager.default.copyItem(at: kaynak, to: hedef)
            withAnimation(azHareket ? nil : .easeOut(duration: 0.2)) {
                servis.belgeBaglami.belgeEkle(url: hedef)
            }
        } catch {
            // Kopyalanamadıysa sessizce geç — kullanıcı yeniden deneyebilir.
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
            VStack(alignment: .leading, spacing: Olcek.cipYanitAraligi) {
                ForEach(mesaj.izler) { iz in
                    AracCipi(iz: iz)
                        .transition(cipGecisi)
                }
                if !mesaj.icerik.isEmpty {
                    KetumYaniti(metin: mesaj.icerik, tabloIndir: tabloIndir)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    /// Canlı (streaming) asistan bloğu: aktif çipler + akan serif metin.
    private var canliBlok: some View {
        VStack(alignment: .leading, spacing: Olcek.cipYanitAraligi) {
            ForEach(servis.yurutucu.izler) { iz in
                AracCipi(iz: iz)
                    .transition(cipGecisi)
            }
            KetumYaniti(metin: canliYanit, akiyorMu: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private let canliKimlik = "canli-blok"

    // MARK: - Eylem

    private func gonder() {
        let metin = soru.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !metin.isEmpty, !yaniyor else { return }
        soru = ""

        let kullanici = Mesaj(rol: .sen, icerik: metin)
        kullanici.sohbet = sohbet
        withAnimation(azHareket ? nil : .easeOut(duration: 0.2)) {
            kayit.insert(kullanici)
        }
        // İlk kullanıcı mesajından sohbet başlığını türet.
        if sohbet.baslik == "Yeni sohbet" {
            sohbet.baslik = String(metin.prefix(40))
        }
        sohbet.guncelleme = Date()
        try? kayit.save()

        yaniyor = true
        canliYanit = ""

        yanitGorevi?.cancel()
        yanitGorevi = Task {
            let sonuc = await servis.yanitla(metin) { kismi in
                guard !Task.isCancelled else { return }
                canliYanit = kismi
            }
            // Yazmadan önce: görev iptal edildi mi, sohbet hâlâ geçerli mi?
            // Geçmiş temizlenmiş veya sohbet silinmişse sessizce çık — öksüz
            // ya da kısmi kayıt yazmak SwiftData'da ölümcül hataya yol açar.
            guard !Task.isCancelled, !sohbet.isDeleted, sohbet.modelContext != nil else {
                yaniyor = false
                canliYanit = ""
                return
            }

            let yanit = Mesaj(rol: .ketum, icerik: sonuc.metin, izler: sonuc.izler)
            yanit.sohbet = sohbet
            withAnimation(azHareket ? nil : .easeOut(duration: 0.2)) {
                kayit.insert(yanit)
                yaniyor = false
                canliYanit = ""
            }
            sohbet.guncelleme = Date()
            try? kayit.save()
        }
    }

    /// Uçuştaki yanıt görevini iptal edip canlı akış durumunu sıfırlar.
    private func gorevIptal() {
        yanitGorevi?.cancel()
        yanitGorevi = nil
        yaniyor = false
        canliYanit = ""
    }

    /// Sohbet içi tablodan "Excel indir": tabloyu .xlsx üretip önizlemeyi açar.
    private func tabloIndir(_ tablo: Tablo) {
        guard let url = try? ExcelMotor().yaz(dosyaAdi: "tablo", baslik: nil,
                                              govde: nil, tablo: tablo,
                                              klasor: BelgeBaglami.ciktiKlasoru()) else { return }
        onizleme = OnizlemeOgesi(url: url)
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

#Preview {
    let kap = try! ModelContainer(for: Sohbet.self, Mesaj.self,
                                  configurations: .init(isStoredInMemoryOnly: true))
    let s = Sohbet(baslik: "Örnek")
    kap.mainContext.insert(s)
    return SohbetGorunumu(sohbet: s, servis: ModelServisi())
        .modelContainer(kap)
}
