import Foundation

// .md ve .txt için sade metin motoru.
struct MetinMotor: BelgeMotoru {
    var bicim: BelgeBicimi

    func yaz(dosyaAdi: String, baslik: String?, govde: String?, tablo: Tablo?, klasor: URL) throws -> URL {
        let markdownMi = bicim == .md
        var parcalar: [String] = []

        // Başlık: markdown ise "# ", düz metin ise sade satır.
        if let baslik, !baslik.isEmpty {
            parcalar.append(markdownMi ? "# \(baslik)" : baslik)
        }

        // Tablo: md için markdown, txt için özet ya da satırlar.
        if let tablo {
            if markdownMi {
                parcalar.append(tablo.markdown)
            } else {
                let ozet = tablo.ozet
                if ozet.isEmpty {
                    parcalar.append(txtTablo(tablo))
                } else {
                    parcalar.append(ozet)
                }
            }
        }

        // Gövde metni.
        if let govde, !govde.isEmpty {
            parcalar.append(govde)
        }

        var icerik = parcalar.joined(separator: "\n\n")
        // Boş dosya üretme: en azından bir işaret bırak.
        if icerik.isEmpty {
            icerik = baslik?.isEmpty == false ? baslik! : " "
        }

        let url = hedefURL(dosyaAdi: dosyaAdi, klasor: klasor)
        try Data(icerik.utf8).write(to: url)
        return url
    }

    func oku(url: URL) throws -> BelgeIcerik {
        let icerik: String
        if let utf8 = try? String(contentsOf: url, encoding: .utf8) {
            icerik = utf8
        } else {
            icerik = (try? String(contentsOf: url, encoding: .isoLatin1)) ?? ""
        }
        return BelgeIcerik(metin: icerik)
    }

    // Özet yoksa satırları TAB ile düz metne dök.
    private func txtTablo(_ tablo: Tablo) -> String {
        var satirlar: [String] = []
        if !tablo.basliklar.isEmpty {
            satirlar.append(tablo.basliklar.joined(separator: "\t"))
        }
        for satir in tablo.satirlar {
            satirlar.append(satir.hucreler.joined(separator: "\t"))
        }
        return satirlar.joined(separator: "\n")
    }
}
