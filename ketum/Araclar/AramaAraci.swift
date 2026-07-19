import Foundation
import FoundationModels
import CoreSpotlight

// AramaAraci — cihazdaki yerel Spotlight index'inde anahtar kelimeyle arama.
// Yerel RAG, yalnızca okuma. Ağ yok, yetki gerektirmez.
struct AramaAraci: KetumAraci {
    // Modele görünen ad BİLEREK "arama" değil (web-arama-spec §8.2 bu çakışmayı
    // öngörmüştü). Ölçülen hata: kullanıcı "internette ara" dediğinde model
    // `arama` adlı aracı görüp çağırıyor, Spotlight cihazdaki notlara bakıyor,
    // ve yanıt "Cihazında bulamadım" oluyor — kullanıcı web istemişken.
    // Açıklama "genel bilgi için kullanma" DİYORDU ve dinlenmedi: araç adı
    // açıklamadan daha güçlü bir sinyal. Ad artık ne aradığını söylüyor.
    // Swift tip adı (`AramaAraci`) ve profil anahtarı değişmez.
    let name = "not_arama"
    // TEK metin, iki durumu da idare eder (web-arama §3.4). Profil bileşimine
    // göre DEĞİŞMEZ: arama sunucusu varsa `web_arama` oturumdadır ve model onu
    // çağırır; yoksa araç ortada yoktur ve cümlenin ikinci yarısı bugünkü
    // dürüst yanıtı ("cihazında böyle bir bilgi yok") aynen sürdürür. Tanımı
    // profile göre değiştirmek, aynı aracın iki farklı davranışını ölçmeyi
    // imkânsız kılardı.
    let description = "Searches the user's OWN notes/files on the device (local Spotlight) by keyword. Only for personal-content requests like 'search my notes', 'find that note', in any language. Do NOT use for weather, general/world knowledge, or definitions — for those use the 'web_arama' tool if it is available; otherwise say there is no such info on the device."
    weak var raporlayici: (any AracRaporlayici)?

    @Generable struct Arguments {
        @Guide(description: "Keyword to search for, e.g. 'meeting notes'.")
        var anahtar: String
    }

    func call(arguments: Arguments) async -> String {
        await cipliCalis(ikon: "magnifyingglass",
                         calisiyorMetni: Yerel.notAraniyor,
                         hamGirdi: arguments.anahtar) {
            let basliklar = await Self.ara(arguments.anahtar)

            // Boş sonuç: hata değil. Model dürüstçe bulamadığını söylesin.
            // Yine de kullanıcının kendi içeriğinde arama YAPILDI — oturum
            // kirlenir (mcp §5.6); aranan kelime de kişisel veridir.
            if basliklar.isEmpty {
                return await kirletEgerBasarili(
                    AracSonucu(cipMetni: Yerel.notArandiYok,
                               durum: .okundu,
                               modeleDonen: "no_results_found on device"))
            }

            let liste = basliklar.enumerated()
                .map { "\($0.offset + 1). \($0.element)" }
                .joined(separator: "\n")

            return await kirletEgerBasarili(AracSonucu(
                cipMetni: Yerel.notArandi(basliklar.count),
                durum: .okundu,
                modeleDonen: "found \(basliklar.count) results: " + basliklar.joined(separator: ", "),
                hamCikti: liste
            ))
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
