//
//  WebAramaAraci.swift
//  ketum
//
//  Web araması aracı (web-arama-spec §5.2). Kullanıcının KENDİ arama sunucusuna
//  tek bir sorgu gönderir. Dışarı çıkan tek veri sorgudur ve sorgu her zaman
//  çip metnindedir (§2.2) — kullanıcı ne gittiğini bakmadan görür.
//
//  Kirli oturumda (daha önce kişisel veri aracı çalıştıysa) sorgu, ortak onay
//  kapısından geçer: `AracYurutucu.onayIste`. Kapı koddadır, modelde değil.
//
//  Ağ kodu burada DEĞİL, `WebAramaIstemcisi`dedir; bu araç yalnızca çip yaşam
//  döngüsünü, onay kapısını ve 4096 bypass kanalını (VeriDeposu) yönetir.
//

import Foundation
import FoundationModels

struct WebAramaAraci: KetumAraci {
    let name = "web_arama"
    let description = "Searches the web via the user's own search server. Use for weather, news, prices, current events, and general/world knowledge the device cannot know. NOT for the user's personal notes/files."

    weak var raporlayici: (any AracRaporlayici)?
    /// Onay kapısı için — `AracRaporlayici` protokolünde olmayan `onayIste`
    /// buradan çağrılır. `ModelServisi` araç kurulumunda bağlar.
    weak var yurutucu: AracYurutucu?
    /// Toplu veri kanalı: tüm sonuçlar depoya, modele kısa liste.
    weak var veriDeposu: VeriDeposu?

    @Generable struct Arguments {
        @Guide(description: "Short web search query in the user's language, e.g. 'istanbul weather tomorrow'.")
        var sorgu: String
    }

    func call(arguments: Arguments) async -> String {
        let sorgu = arguments.sorgu.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !sorgu.isEmpty else {
            return "error: empty_query. Call the tool again with a short search query."
        }

        // Sunucu tanımsız/kapalıysa araç zaten profile girmemeli; yine de
        // savunmacı dur — ağ katmanına hiç dokunma.
        guard WebAramaAyari.aktifMi else {
            return "search_unavailable: no search server is configured. Tell the user, in their own language, that this information is not on the device."
        }

        // ONAY KAPISI — çipten ÖNCE. Temiz oturumda sormadan geçer (§2.4);
        // kirli oturumda kullanıcı giden sorgunun aynısını görür (§3.3).
        if let yurutucu {
            let izinli = await yurutucu.onayIste(kaynak: Yerel.aramaSunucusu,
                                                 aracAdi: name,
                                                 icerik: sorgu)
            guard izinli else {
                // Ret çipini kapı düşürdü; burada ikinci çip üretilmez.
                return "user_declined_search: the user chose not to send this query. Do not retry; tell the user briefly, in their own language, that the search was not sent."
            }
        }

        return await cipliCalis(ikon: "globe",
                                calisiyorMetni: Yerel.araniyor(sorgu),
                                hamGirdi: sorgu) {
            let (sonuclar, istekURL) = try await WebAramaIstemcisi.ara(sorgu)

            // Çip detayı: ne gitti (tam istek URL'i) + ne geldi (başlık/adres/özet).
            let ham = "GET \(istekURL.absoluteString)\n\n"
                + (sonuclar.isEmpty ? "—" : WebAramaIstemcisi.hamCiktiMetni(sonuclar))

            guard !sonuclar.isEmpty else {
                return AracSonucu(cipMetni: Yerel.arandi(0),
                                  durum: .okundu,
                                  modeleDonen: "no_results",
                                  hamCikti: ham)
            }

            // 4096 bypass: tam liste depoya, modele kırpılmış özet (≤ ~300 token).
            var modeleDonen = WebAramaIstemcisi.modeleMetin(sorgu: sorgu, sonuclar: sonuclar)
            if let veriDeposu {
                let ref = await veriDeposu.koy(WebAramaIstemcisi.tablo(sonuclar), etiket: "arama")
                modeleDonen += "\n(data_ref=\(ref))"
            }

            return AracSonucu(cipMetni: Yerel.arandi(sonuclar.count),
                              durum: .okundu,
                              modeleDonen: modeleDonen,
                              hamCikti: ham)
        }
    }
}

// Çip metinleri. `Yerel` başka bir ajanın dosyası olduğu için burada
// genişletiliyor — tek yerelleştirme noktası korunur, dosya sınırı da.
extension Yerel {
    /// Sorgu çip metnindedir (§2.2) — kategori özeti değil, giden metnin aynısı.
    static func araniyor(_ sorgu: String) -> String { String(localized: "Aranıyor · \(sorgu)") }
    static func arandi(_ n: Int) -> String { String(localized: "Arandı · \(n) sonuç") }
    static var aramaUlasilamadi: String { String(localized: "Aramaya ulaşılamadı") }
    /// Onay kapısında ve ret çipinde görünen kaynak adı.
    static var aramaSunucusu: String { String(localized: "arama sunucusu") }
}
