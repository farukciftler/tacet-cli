import SwiftUI
import UIKit

// MARK: - Çip/kart ayrımı (saf fonksiyonlar, OtoTest edilir)

/// Bir turun izleri üç yere dağılır: dosya kartı, görünür çip, katlanmış seyir.
/// Karar tek yerde verilir ki canlı blok ile geçmiş mesaj aynı kuralı uygulasın.
/// Tek doğruluk kaynağı yine `AracIzi`dir — kart da çip de aynı izi anlatır.
enum YanitIzleri {

    /// Dosya üreten iz: kart olarak çizilir, çip listesinden ÇIKARILIR
    /// (seyir-spec §9.4). Başarısız iz kart olmaz; ortada dosya yoktur, hatanın
    /// yeri çiptir.
    static func kartlikMi(_ iz: AracIzi) -> Bool {
        guard let yol = iz.dosyaYolu, !yol.isEmpty else { return false }
        if case .basarisiz = iz.durum { return false }
        return true
    }

    /// Kart olarak çizilecek izler.
    static func kartlar(_ izler: [AracIzi]) -> [AracIzi] {
        izler.filter { kartlikMi($0) }
    }

    /// Yanıt bitince görünen çipler.
    ///
    /// - `seyirVar == false` (eski mesaj, adım verisi yok): çipler BUGÜNKÜ gibi
    ///   tümü görünür — geriye dönük seyir üretilmez (seyir-spec §5.1).
    /// - `seyirVar == true`: yalnızca okuma çipleri katlamaya girer; `yazildi`
    ///   ve `basarisiz` yerinde kalır (§2.2).
    static func cipler(_ izler: [AracIzi], seyirVar: Bool) -> [AracIzi] {
        let kartsiz = izler.filter { !kartlikMi($0) }
        return seyirVar ? SeyirKatlama.katlamaDisi(kartsiz) : kartsiz
    }

    /// Üretim sürerken görünen çipler. Süren adım zaten canlı şeritte yazılıdır;
    /// aynı şeyi iki kez söylememek için `calisiyor` çipi şerit varken çizilmez.
    static func canliCipler(_ izler: [AracIzi], seritVar: Bool) -> [AracIzi] {
        let kartsiz = izler.filter { !kartlikMi($0) }
        guard seritVar else { return kartsiz }
        return kartsiz.filter { iz in
            if iz.durum == .calisiyor { return false }
            return !SeyirKatlama.katlanirMi(iz.durum)
        }
    }
}

// Asistan yanıtı: balonsuz, sola hizalı serif metin.
// Metin akarken (streaming) tek sakin yanıp sönen nokta gösterilir.
struct KetumYaniti: View {
    let metin: String
    var akiyorMu: Bool = false
    /// Sohbet içi tablodan "Excel indir" isteği.
    var tabloIndir: (Tablo) -> Void = { _ in }
    /// Bu bir hata bildirimi mi (gerçek yanıt değil)?
    var hataMi: Bool = false
    /// Hata balonundaki "Yeniden dene" eylemi. nil ise düğme gösterilmez.
    var yenidenDene: (() -> Void)? = nil
    /// Bu yanıtın ürettiği dosyaların izleri — gövdenin altında kart olarak
    /// çizilir (seyir-spec §9.4). Çağıran taraf `YanitIzleri.kartlar` ile ayırır.
    var dosyaIzleri: [AracIzi] = []

    // Reduce Motion açıksa animasyon kapalı, nokta sabit kalır.
    @Environment(\.accessibilityReduceMotion) private var hareketiAzalt

    /// Kopyalama haptiği için tetikleyici.
    @State private var kopyaSayaci = 0

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: Olcek.cipYanitAraligi) {
                govde
                // Dosya kartı gövdenin ALTINDA, balonla aynı hizada (§9.2).
                ForEach(dosyaIzleri) { iz in
                    if let kart = DosyaKarti(iz: iz) { kart }
                }
            }
            // Genişlik, taşıyıcı (satır) genişliğinin %88'i — spec §4.3.
            .containerRelativeFrame(.horizontal, alignment: .leading) { genislik, _ in
                genislik * Olcek.ketumYanitGenislik
            }
            Spacer(minLength: 0)
        }
        .sensoryFeedback(.success, trigger: kopyaSayaci)
    }

    @ViewBuilder
    private var govde: some View {
        if hataMi {
            hataBlogu
        } else {
            icerik
                .contextMenu {
                    if !metin.isEmpty {
                        Button {
                            UIPasteboard.general.string = metin
                            kopyaSayaci += 1
                        } label: {
                            Label("Kopyala", systemImage: "doc.on.doc")
                        }
                    }
                }
        }
    }

    /// Hata bildirimi: gerçek yanıttan ayrışsın diye kenarlıklı, hata renginde
    /// küçük bir blok. Vurgu rengi kullanılmaz — palet mürekkep/gri + Renk.hata.
    private var hataBlogu: some View {
        VStack(alignment: .leading, spacing: Olcek.s2) {
            HStack(alignment: .firstTextBaseline, spacing: Olcek.s2) {
                Image(systemName: "exclamationmark.triangle")
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.hata)
                Text(metin)
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.hata)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            if let yenidenDene {
                Button(action: yenidenDene) {
                    Text("Yeniden dene")
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.murekkep)
                        .padding(.horizontal, Olcek.s3)
                        .padding(.vertical, Olcek.s1)
                        .overlay(
                            RoundedRectangle(cornerRadius: Olcek.cipKose)
                                .stroke(Renk.cizgi, lineWidth: Olcek.hairline)
                        )
                }
                .buttonStyle(.plain)
            }
        }
        .padding(Olcek.s3)
        .overlay(
            RoundedRectangle(cornerRadius: Olcek.s3)
                .stroke(Renk.hata.opacity(0.4), lineWidth: Olcek.hairline)
        )
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(Text("Hata: \(metin)"))
    }

    // Metin boşsa ve akıyorsa nokta; akarken düz metin; bitince tabloları render et.
    @ViewBuilder
    private var icerik: some View {
        if metin.isEmpty && akiyorMu {
            NefesNoktasi(hareketiAzalt: hareketiAzalt)
        } else if akiyorMu {
            // Akış sırasında tablo yarım olabilir — düz metin göster (titremesin).
            metinGovde(metin)
        } else {
            VStack(alignment: .leading, spacing: Olcek.s3) {
                ForEach(Array(Tablo.bloklara(metin).enumerated()), id: \.offset) { _, blok in
                    switch blok {
                    case .metin(let t): metinGovde(t)
                    case .tablo(let tb): SohbetTablo(tablo: tb, indir: tabloIndir)
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func metinGovde(_ t: String) -> some View {
        Text(bicimli(t))
            .font(Yazi.ketum())
            .foregroundStyle(Renk.murekkep)
            .textSelection(.enabled)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// Satır içi markdown'ı çözer: `**kalın**`, `*italik*`, `` `kod` ``.
    ///
    /// `Text(String)` markdown'ı ÇÖZMEZ — SwiftUI bunu yalnızca derleme anında
    /// bilinen `LocalizedStringKey` literalleri için yapar. Model çıktısı çalışma
    /// anı String olduğu için yıldızlar ekranda ham görünüyordu ("**İmsak:** 05:04").
    ///
    /// `inlineOnlyPreservingWhitespace` bilinçli: satır sonlarını KORUR. Tam
    /// markdown ayrıştırması paragrafları birleştirip listeyi tek satıra indirirdi.
    /// Blok yapısı (tablolar) zaten `bloklar(_:)` tarafından ayrı ele alınıyor.
    ///
    /// Ayrıştırma düşerse ham metne dönülür: yarım kalmış bir `**` yüzünden
    /// (akış sırasında olur) yanıtın kaybolması kabul edilemez.
    private func bicimli(_ ham: String) -> AttributedString {
        (try? AttributedString(
            markdown: ham,
            options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)
        )) ?? AttributedString(ham)
    }

    // Blok ayrımı `Tablo.bloklara` içinde — tek doğruluk kaynağı orası.
    //
    // Eski yerel ayrıştırıcı, ayraç satırı ("|---|") ZORUNLU sayıyordu ve
    // `markdownDan` nil dönünce toplanan pipe satırlarını hiçbir bloğa
    // koymadan DÜŞÜRÜYORDU — model geçerli ama ayraçsız bir tablo yazdığında
    // içerik ekrandan sessizce kayboluyordu (denetim P1-5). `Tablo.bloklara`
    // tanımadığı her satırı `.metin` olarak geri verdiği için bu mümkün değil.
}

// Tek nokta, yavaşça nefes alır gibi yanıp söner.
private struct NefesNoktasi: View {
    let hareketiAzalt: Bool
    @State private var parlak = false

    var body: some View {
        Circle()
            .fill(Renk.gri)
            .frame(width: 6, height: 6)
            .opacity(hareketiAzalt ? 0.6 : (parlak ? 1.0 : 0.3))
            .onAppear {
                guard !hareketiAzalt else { return }
                withAnimation(
                    .easeInOut(duration: 0.9).repeatForever(autoreverses: true)
                ) {
                    parlak = true
                }
            }
            .accessibilityLabel("sirr yazıyor")
    }
}

#Preview("Yanıt") {
    VStack(alignment: .leading, spacing: Olcek.mesajAraligi) {
        KetumYaniti(metin: "Bugün üç toplantın var. İlki saat onda başlıyor.")
        KetumYaniti(metin: "", akiyorMu: true)
        KetumYaniti(metin: "Model bu cihazda hazır değil.",
                    hataMi: true, yenidenDene: {})
        KetumYaniti(metin: "Tabloyu hazırladım.",
                    dosyaIzleri: [AracIzi(ikon: "tablecells", metin: "tablo yazıldı",
                                          durum: .yazildi,
                                          dosyaYolu: "/tmp/Yıldızlar keşif soruları.xlsx")])
    }
    .padding(Olcek.s5)
}
