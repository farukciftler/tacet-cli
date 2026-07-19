import SwiftUI

// Asistan yanıtı: balonsuz, sola hizalı serif metin.
// Metin akarken (streaming) tek sakin yanıp sönen nokta gösterilir.
struct KetumYaniti: View {
    let metin: String
    var akiyorMu: Bool = false
    /// Sohbet içi tablodan "Excel indir" isteği.
    var tabloIndir: (Tablo) -> Void = { _ in }

    // Reduce Motion açıksa animasyon kapalı, nokta sabit kalır.
    @Environment(\.accessibilityReduceMotion) private var hareketiAzalt

    var body: some View {
        HStack {
            icerik
                // Genişlik, taşıyıcı (satır) genişliğinin %88'i — spec §4.3.
                .containerRelativeFrame(.horizontal, alignment: .leading) { genislik, _ in
                    genislik * Olcek.ketumYanitGenislik
                }
            Spacer(minLength: 0)
        }
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
                ForEach(Array(bloklar(metin).enumerated()), id: \.offset) { _, blok in
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
        Text(t)
            .font(Yazi.ketum())
            .foregroundStyle(Renk.murekkep)
            .textSelection(.enabled)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    // Metni metin/tablo bloklarına böler (markdown tablolarını ayırır).
    private enum Blok { case metin(String); case tablo(Tablo) }

    private func bloklar(_ ham: String) -> [Blok] {
        let satirlar = ham.components(separatedBy: "\n")
        var sonuc: [Blok] = []
        var metinTampon: [String] = []
        func metniBosalt() {
            let t = metinTampon.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
            if !t.isEmpty { sonuc.append(.metin(t)) }
            metinTampon = []
        }
        var i = 0
        while i < satirlar.count {
            let s = satirlar[i].trimmingCharacters(in: .whitespaces)
            let sonraki = i + 1 < satirlar.count ? satirlar[i + 1].trimmingCharacters(in: .whitespaces) : ""
            if s.hasPrefix("|"), sonraki.hasPrefix("|"), sonraki.contains("-") {
                // Tablo bloğu başladı — tampon metni boşalt, tablo satırlarını topla.
                metniBosalt()
                var j = i
                var tabloSatir: [String] = []
                while j < satirlar.count, satirlar[j].trimmingCharacters(in: .whitespaces).hasPrefix("|") {
                    tabloSatir.append(satirlar[j])
                    j += 1
                }
                if let tb = Tablo.markdownDan(tabloSatir.joined(separator: "\n")) {
                    sonuc.append(.tablo(tb))
                }
                i = j
            } else {
                metinTampon.append(satirlar[i])
                i += 1
            }
        }
        metniBosalt()
        return sonuc
    }
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
    }
    .padding(Olcek.s5)
}
