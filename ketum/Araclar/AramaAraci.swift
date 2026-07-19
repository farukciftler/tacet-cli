import Foundation
import FoundationModels
import CoreSpotlight

// AramaAraci — cihazdaki yerel Spotlight index'inde anahtar kelimeyle arama.
// Yerel RAG, yalnızca okuma. Ağ yok, yetki gerektirmez.
struct AramaAraci: KetumAraci {
    let name = "arama"
    let description = "Searches the user's OWN notes/files on the device (local Spotlight) by keyword. Only for personal-content requests like 'search my notes', 'find that note', in any language. Do NOT use for weather, general/world knowledge, or definitions — for those, say there is no such info on the device."
    weak var raporlayici: (any AracRaporlayici)?

    @Generable struct Arguments {
        @Guide(description: "Aranacak anahtar kelime, örn 'toplantı notları'")
        var anahtar: String
    }

    func call(arguments: Arguments) async -> String {
        await cipliCalis(ikon: "magnifyingglass",
                         calisiyorMetni: Yerel.notAraniyor,
                         hamGirdi: arguments.anahtar) {
            let basliklar = await Self.ara(arguments.anahtar)

            // Boş sonuç: hata değil. Model dürüstçe bulamadığını söylesin.
            if basliklar.isEmpty {
                return AracSonucu(cipMetni: Yerel.notArandiYok,
                                  durum: .okundu,
                                  modeleDonen: "no_results_found on device")
            }

            let liste = basliklar.enumerated()
                .map { "\($0.offset + 1). \($0.element)" }
                .joined(separator: "\n")

            return AracSonucu(
                cipMetni: Yerel.notArandi(basliklar.count),
                durum: .okundu,
                modeleDonen: "found \(basliklar.count) results: " + basliklar.joined(separator: ", "),
                hamCikti: liste
            )
        }
    }

    // Spotlight sorgusunu async köprüyle çalıştırır; en çok 10 başlık toplar.
    // Hata olursa boş liste döner (çip .basarisiz olmaz).
    private static func ara(_ anahtar: String) async -> [String] {
        // Özel karakterleri temizle; sorgu string'ini güvenli kur.
        let temiz = anahtar.replacingOccurrences(of: "\"", with: "")
        guard !temiz.trimmingCharacters(in: .whitespaces).isEmpty else { return [] }

        let sorgu = "title == \"*\(temiz)*\"cd || textContent == \"*\(temiz)*\"cd"

        return await withCheckedContinuation { devam in
            var baslıklar: [String] = []
            var tamamlandi = false

            let baglam = CSSearchQueryContext()
            baglam.fetchAttributes = ["title", "displayName"]

            let query = CSSearchQuery(queryString: sorgu, queryContext: baglam)

            // Sonuçlar parça parça gelir; en çok 10 başlık biriktir.
            query.foundItemsHandler = { ogeler in
                for oge in ogeler where baslıklar.count < 10 {
                    let ad = oge.attributeSet.title
                        ?? oge.attributeSet.displayName
                        ?? oge.uniqueIdentifier
                    baslıklar.append(ad)
                }
            }

            query.completionHandler = { _ in
                guard !tamamlandi else { return }
                tamamlandi = true
                devam.resume(returning: Array(baslıklar.prefix(10)))
            }

            query.start()
        }
    }
}
