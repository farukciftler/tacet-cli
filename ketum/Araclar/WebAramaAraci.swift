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
    let description = "Searches the web via the user's own search server and, when the question asks for a concrete value (times, prices, temperatures, dates), extracts that value from the pages. Use for weather, news, prices, timetables, current events, and general/world knowledge the device cannot know. NOT for the user's personal notes/files. Cite sources as plain-text domain names only; never write markdown links or full URLs."

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
            return "search_unavailable: no search server is configured; the answer is not available on this device"
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
                                hamGirdi: sorgu) { ilerle in
            // ORTAK SON TARİH: arama ısrarı + sayfa çekme + ikinci tur hepsi bu
            // tek bütçeyi paylaşır (bkz. `cevapBul(bitis:)`).
            let bitis = Date().addingTimeInterval(CevapSuzgeci.toplamButce)

            // BOŞ YANITTA ISRAR. Ölçüldü: bu sunucu aynı sorguya arka arkaya
            // 7 kez HTTP 200 + boş liste, 8. kez 20 sonuç döndürüyor (üst motor
            // kısıtlaması). Tek denemede vazgeçmek, aramanın "bazen çalışıyor"
            // görünmesinin birinci sebebiydi.
            let (sonuclar, istekURL, deneme) = try await WebAramaIstemcisi.araIsrarla(
                sorgu, bitis: bitis)

            // Çip detayı: ne gitti (tam istek URL'i) + ne geldi (başlık/adres/özet).
            // Kaç kez sorulduğu da yazılır: kullanıcının kendi sunucusuna giden
            // istek sayısı gizlenmez (§2.2 şeffaflık).
            let denemeNotu = deneme > 1 ? " (\(deneme) deneme)" : ""
            let ham = "GET \(istekURL.absoluteString)\(denemeNotu)\n\n"
                + (sonuclar.isEmpty ? "—" : WebAramaIstemcisi.hamCiktiMetni(sonuclar))

            guard !sonuclar.isEmpty else {
                return AracSonucu(cipMetni: Yerel.arandi(0),
                                  durum: .okundu,
                                  modeleDonen: "no_results",
                                  hamCikti: ham)
            }

            // CEVAP BULMA DÖNGÜSÜ. Sorgu somut bir değer istiyorsa (saat, fiyat,
            // sıcaklık, tarih) link listesi yetmez: değerin KENDİSİ getirilir.
            // Yeterlilik kararını kod verir (regex), model değil — model yargısına
            // bırakıldığı her yerde uydurma değer üretti.
            if let bulgu = await WebAramaIstemcisi.cevapBul(
                sorgu: sorgu, sonuclar: sonuclar, bitis: bitis,
                ilerle: { alan in await ilerle(Yerel.okunuyor(alan)) }) {
                let cekmeNotu = bulgu.cekilenler.isEmpty
                    ? ""
                    : "\n\nOKUNAN SAYFALAR: " + bulgu.cekilenler.joined(separator: ", ")
                // İkinci tur yapıldıysa GİDEN İKİNCİ SORGU da çip detayında
                // görünür. "Dışarı çıkan tek veri sorgudur ve sorgu her zaman
                // görünür" vaadi (spec §2.2) kodun ürettiği sorgu için de geçerli.
                let ikinciNotu = bulgu.ikinciSorgu.map { "\n\nİKİNCİ ARAMA: \($0)" } ?? ""
                let hamGenis = ham + ikinciNotu + cekmeNotu
                    + (bulgu.tamMetin.isEmpty ? "" : "\n\n" + bulgu.tamMetin)

                guard bulgu.yeterliMi else {
                    // Eşik altında: modele İÇERİK VERİLMEZ. Sayfa metnini "belki
                    // model bir şey bulur" diye göndermek, uydurmanın davetiyesidir.
                    return AracSonucu(cipMetni: Yerel.cevapBulunamadi,
                                      durum: .okundu,
                                      modeleDonen: CevapSuzgeci.bulunamadiMetni,
                                      hamCikti: hamGenis)
                }

                var modeleDonen = CevapSuzgeci.modeleMetin(sorgu: sorgu,
                                                           sekil: bulgu.sekil,
                                                           eslesmeler: bulgu.eslesmeler,
                                                           guncellik: bulgu.guncellik)
                // Düzenli eşleşmeler tablo olur; model tabloyu kendi yazmaz.
                if let veriDeposu,
                   let tablo = CevapSuzgeci.tablo(bulgu.eslesmeler, sekil: bulgu.sekil) {
                    let ref = await veriDeposu.koy(tablo, etiket: "arama")
                    modeleDonen += "\n(data_ref=\(ref))"
                }
                // Çip de dürüst olmalı: uyarıyı yalnız modele verip kullanıcıya
                // "Bulundu · 6 değer" demek, modelin uyarıyı aktarmasına
                // güvenmek olurdu. Bu projede model yargısına bırakılan her şey
                // düştü; kullanıcıya giden ikinci kanal koddan gelir.
                let cipMetni = bulgu.guncellik == .dogrulandi
                    ? Yerel.cevapBulundu(bulgu.eslesmeler.count)
                    : Yerel.cevapBulunduGuncellikSuphesi(bulgu.eslesmeler.count)
                return AracSonucu(cipMetni: cipMetni,
                                  durum: .okundu,
                                  modeleDonen: modeleDonen,
                                  hamCikti: hamGenis)
            }

            // Şekil belirlenemedi (serbest metin sorusu) → mevcut davranış:
            // 4096 bypass, tam liste depoya, modele kırpılmış özet (≤ ~300 token).
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
    /// Sayfa çekilirken canlı çip metni. Süslü fiil yok, alan adının kendisi.
    static func okunuyor(_ alan: String) -> String { String(localized: "Okunuyor · \(alan)") }
    /// Şekil aranıp bulunduğunda: sonuç sayısı değil, DEĞER sayısı gösterilir.
    static func cevapBulundu(_ n: Int) -> String { String(localized: "Bulundu · \(n) değer") }
    /// Değer bulundu ama sayfada bugünün tarihi görünmüyor. Kullanıcı bunu
    /// modelin anlatmasına kalmadan çipte görür — dramatize yok, düz bilgi.
    static func cevapBulunduGuncellikSuphesi(_ n: Int) -> String {
        String(localized: "Bulundu · \(n) değer · güncelliği doğrulanmadı")
    }
    /// Şekil arandı ama eşik altında kalındı — dürüst çip, sessiz başarı yok.
    static var cevapBulunamadi: String { String(localized: "Aranan bilgi bulunamadı") }
    static var aramaUlasilamadi: String { String(localized: "Aramaya ulaşılamadı") }
    /// Onay kapısında ve ret çipinde görünen kaynak adı.
    static var aramaSunucusu: String { String(localized: "arama sunucusu") }
}
