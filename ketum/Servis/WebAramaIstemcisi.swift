//
//  WebAramaIstemcisi.swift
//  ketum
//
//  Uygulamadaki TEK ağ kodu (web-arama-spec §2.5; MCP istemcisi gelene dek
//  tek, ondan sonra iki). Başka hiçbir katman URLSession'a dokunmaz — bu kural
//  OtoTest'te statik taramayla doğrulanır (§8).
//
//  Kullanıcının kendi SearXNG örneğine tek bir GET atar. Dışarı çıkan tek veri
//  arama sorgusudur. Ayrıştırma ve filtreler (5 sonuç / 200 karakter / alan
//  adı) SAF FONKSİYONDUR: ağsız, fixture JSON ile test edilebilir (§6).
//

import Foundation
import NaturalLanguage

/// Modele ve çipe giden tek sonuç satırı.
struct WebSonuc: Equatable, Identifiable {
    var id: String { tamAdres.isEmpty ? baslik : tamAdres }

    var baslik: String
    /// Modele giden kısaltılmış adres — yalnızca alan adı ("www.mgm.gov.tr").
    var alanAdi: String
    /// Tam URL — çip detayında durur, modele GİTMEZ (halüsinasyonlu link riski).
    var tamAdres: String
    /// 200 karakterde kelime sınırında kırpılmış özet.
    var ozet: String
    /// SearXNG infobox'ından geldiyse true — listede ilk sırada durur.
    var bilgiKutusuMu: Bool = false
}

/// Arama hataları. `LocalizedError` olduğu için `KetumAraci.kisaHata`
/// bunları olduğu gibi çipe yazar; ham `NSURLErrorDomain` metni ekrana çıkmaz.
enum WebAramaHatasi: LocalizedError, Equatable {
    /// Sunucu tanımsız ya da arama kapalı — ağ hiç denenmez.
    case sunucuYok
    /// Ağ katmanı hatası: zaman aşımı, adres bulunamadı, bağlantı kesildi.
    case ulasilamadi
    /// HTTP ≠ 200.
    case sunucuHatasi(Int)
    /// Gövde JSON değil ya da beklenen alanlar yok — SearXNG'de `formats: json`
    /// kapalıysa tipik olarak HTML döner ve buraya düşer.
    case bicimAnlasilmadi

    var errorDescription: String? {
        switch self {
        case .sunucuYok:
            return String(localized: "Arama sunucusu tanımlı değil.")
        case .ulasilamadi, .sunucuHatasi:
            return String(localized: "Aramaya şu an ulaşılamadı.")
        case .bicimAnlasilmadi:
            return String(localized: "Arama sunucusu JSON döndürmedi.")
        }
    }
}

enum WebAramaIstemcisi {

    /// Arama uzun sürmez. MCP'nin 120 sn'si build içindir, buraya taşınmaz (§5.3).
    static let zamanAsimi: TimeInterval = 15

    /// Modele giden sonuç tavanı (bilgi kutusu dahil).
    static let sonucTavani = 5
    /// Sonuç başına özet karakter tavanı.
    static let ozetTavani = 200

    // MARK: - Ağ

    /// Kök adrese `GET /search?q=…&format=json` atar, filtrelenmiş sonuç döner.
    /// - Parameter kok: `nil` ise ayardan okunur.
    static func ara(_ sorgu: String, kok: URL? = nil) async throws -> (sonuclar: [WebSonuc], istekURL: URL) {
        guard let kokURL = kok ?? WebAramaAyari.kokURL else { throw WebAramaHatasi.sunucuYok }
        let dil = await dilSec(sorgu: sorgu)
        guard let url = istekURL(kok: kokURL, sorgu: sorgu, dil: dil) else {
            throw WebAramaHatasi.sunucuYok
        }

        var istek = URLRequest(url: url)
        istek.timeoutInterval = zamanAsimi
        istek.httpMethod = "GET"
        istek.setValue("application/json", forHTTPHeaderField: "Accept")

        let ayar = URLSessionConfiguration.ephemeral
        ayar.timeoutIntervalForRequest = zamanAsimi
        ayar.timeoutIntervalForResource = zamanAsimi
        // Sorgu ve yanıt diskte iz bırakmasın: arama sorgusu kişisel bilgi
        // taşıyabilir (§2.2), URL önbelleğinde saklanmasının gereği yok.
        ayar.urlCache = nil
        ayar.requestCachePolicy = .reloadIgnoringLocalCacheData
        let oturum = URLSession(configuration: ayar)

        let veri: Data
        let yanit: URLResponse
        do {
            (veri, yanit) = try await oturum.data(for: istek)
        } catch {
            // Ham NSError dışarı sızmaz; çipe insan cümlesi çıkar.
            throw WebAramaHatasi.ulasilamadi
        }

        if let http = yanit as? HTTPURLResponse, http.statusCode != 200 {
            throw WebAramaHatasi.sunucuHatasi(http.statusCode)
        }

        return (try ayristir(veri), url)
    }

    /// İstek URL'i. Sorgu yüzde-kodlaması `URLComponents`e bırakılır.
    static func istekURL(kok: URL, sorgu: String, dil: String?) -> URL? {
        let temizSorgu = sorgu.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !temizSorgu.isEmpty else { return nil }

        // Kök adres "…/searxng/" ya da "…/searxng" olabilir; ikisi de çalışsın.
        let taban = kok.appendingPathComponent("search")
        guard var parca = URLComponents(url: taban, resolvingAgainstBaseURL: false) else { return nil }

        var ogeler = [
            URLQueryItem(name: "q", value: temizSorgu),
            URLQueryItem(name: "format", value: "json"),
            URLQueryItem(name: "safesearch", value: "1"),
        ]
        // Dil bilinmiyorsa parametre HİÇ gönderilmez — yanlış dil zorlamaktansa
        // sunucunun kendi varsayılanı iyidir (§5.3).
        if let dil, !dil.isEmpty {
            ogeler.append(URLQueryItem(name: "language", value: dil))
        }
        parca.queryItems = ogeler
        return parca.url
    }

    /// Sorgu dili: önce kullanıcının açık tercihi, sonra metinden tahmin, yoksa nil.
    @MainActor
    static func dilSec(sorgu: String) -> String? {
        let tercih = DilTercihi.paylasilan.yanitDili
        if !tercih.isEmpty { return tercih }
        return tahminEt(sorgu: sorgu)
    }

    /// `NLLanguageRecognizer` tahmini — cihaz üstü, ağsız. Kısa/kararsız
    /// sorgularda nil döner (yanlış dil zorlamaktan iyidir).
    static func tahminEt(sorgu: String) -> String? {
        let temiz = sorgu.trimmingCharacters(in: .whitespacesAndNewlines)
        guard temiz.count >= 4 else { return nil }
        let taniyici = NLLanguageRecognizer()
        taniyici.processString(temiz)
        guard let dil = taniyici.dominantLanguage, dil != .undetermined else { return nil }
        // Güven eşiği: zayıf tahmin parametre göndermeye değmez.
        let guven = taniyici.languageHypotheses(withMaximum: 1)[dil] ?? 0
        guard guven >= 0.5 else { return nil }
        return dil.rawValue
    }

    // MARK: - Ayrıştırma + filtreler (saf; fixture ile test edilir)

    /// SearXNG JSON gövdesini `WebSonuc` listesine çevirir ve uygulama katmanı
    /// filtrelerini uygular. Model çıktısına/girdisine güvenilmez: tavan, kırpma
    /// ve alan adı indirgemesi burada zorlanır, çağıranın insafına bırakılmaz.
    static func ayristir(_ veri: Data) throws -> [WebSonuc] {
        guard let kok = try? JSONSerialization.jsonObject(with: veri) as? [String: Any] else {
            throw WebAramaHatasi.bicimAnlasilmadi
        }

        var sonuclar: [WebSonuc] = []

        // Bilgi kutusu varsa ilk sırada (§5.3).
        if let kutular = kok["infoboxes"] as? [[String: Any]],
           let ilk = kutular.first {
            let icerik = (ilk["content"] as? String) ?? ""
            if !icerik.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                let adres = (ilk["urls"] as? [[String: Any]])?.first?["url"] as? String
                    ?? (ilk["id"] as? String) ?? ""
                sonuclar.append(WebSonuc(
                    baslik: (ilk["infobox"] as? String) ?? "",
                    alanAdi: alanAdiCikar(adres),
                    tamAdres: adres,
                    ozet: kirp(icerik),
                    bilgiKutusuMu: true))
            }
        }

        // `results` yoksa bu geçerli ama boş bir yanıttır; bozuk JSON değildir.
        let ham = (kok["results"] as? [[String: Any]]) ?? []
        for oge in ham {
            if sonuclar.count >= sonucTavani { break }
            let baslik = ((oge["title"] as? String) ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            let adres = ((oge["url"] as? String) ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            let icerik = (oge["content"] as? String) ?? ""
            guard !baslik.isEmpty || !adres.isEmpty else { continue }
            sonuclar.append(WebSonuc(
                baslik: baslik,
                alanAdi: alanAdiCikar(adres),
                tamAdres: adres,
                ozet: kirp(icerik)))
        }

        return Array(sonuclar.prefix(sonucTavani))
    }

    /// Özeti kelime sınırında kırpar. Sınırın ortasında kelime bölünmez.
    static func kirp(_ metin: String, sinir: Int = ozetTavani) -> String {
        let tek = metin
            .replacingOccurrences(of: "\n", with: " ")
            .replacingOccurrences(of: "\r", with: " ")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard tek.count > sinir else { return tek }

        let dilim = tek.prefix(sinir)
        if let bosluk = dilim.lastIndex(of: " "), bosluk > dilim.startIndex {
            let kesik = String(dilim[dilim.startIndex..<bosluk])
                .trimmingCharacters(in: .whitespacesAndNewlines)
            if !kesik.isEmpty { return kesik + "…" }
        }
        return String(dilim) + "…"
    }

    /// Adresi alan adına indirger. Modele tam URL gitmez: token bütçesi ve
    /// halüsinasyonlu link riski birlikte düşer (§5.3).
    static func alanAdiCikar(_ adres: String) -> String {
        guard let url = URL(string: adres), let konak = url.host, !konak.isEmpty else {
            return ""
        }
        return konak
    }

    // MARK: - Modele dönen metin (4096 bypass — §5.5)

    /// Modele giden TEK SATIRIN içerik tavanı. Bütçe satır başına zorlanır:
    /// 5 satır × (~180 + önek) + üst satır + kaynak kuralı ≈ 1120 karakter
    /// ≈ 280 token (§5.5). Başlığı ya da özeti ayrı ayrı kırpmak yetmez — uzun
    /// başlık + uzun alan adı + tavan özet birlikte bütçeyi aşar; tek kapı
    /// satırın kendisidir. Ham çıktı (çip detayı) kırpılmadan kalır.
    static let satirTavani = 180

    /// Kırpılmış liste; hedef bütçe ≤ ~300 token. Sıfır sonuçta sabit `no_results`.
    static func modeleMetin(sorgu: String, sonuclar: [WebSonuc]) -> String {
        guard !sonuclar.isEmpty else { return "no_results" }
        // BAŞLIK/KAYNAK/ÖZET ETİKETLİ verilir. Etiketsiz "başlık — alan adı — özet"
        // biçimi ölçülen bir hataya yol açıyordu: model bir satırın başlığını başka
        // bir satırın alan adıyla birleştirip `[şehirhatlari.istanbul](e-yasamrehberi.com)`
        // gibi UYDURMA kaynaklar üretiyordu. Uydurma URL, yanlış saatten sinsidir:
        // kullanıcı doğrulamaya gittiğinde de yanlış yere gider. Etiket, alanların
        // hangi satıra ait olduğunu tekleştirir; alttaki kural da link kurmayı yasaklar.
        let satirlar = sonuclar.enumerated().map { (i, s) -> String in
            let bas = s.bilgiKutusuMu ? "[infobox] " : ""
            let parcalar = kirp([s.baslik.isEmpty ? nil : "title: \(s.baslik)",
                                 s.alanAdi.isEmpty ? nil : "source: \(s.alanAdi)",
                                 s.ozet.isEmpty ? nil : "blurb: \(s.ozet)"]
                .compactMap { $0 }
                .joined(separator: " | "), sinir: satirTavani)
            return "\(i + 1). \(bas)\(parcalar)"
        }
        // Başlık BİLEREK "found N results" değil. Ölçülen davranış: model
        // "found 5 results" ibaresini "cevabı buldum" diye okuyup listede hiç
        // geçmeyen bir sayı uyduruyordu (aynı soruya 20°C ve 24°C). Sonuçların
        // NE OLDUĞUNU adıyla söylemek — sayfa listesi, canlı veri değil —
        // uydurmayı azaltıyor. Bu bir TALİMAT değil, veri betimlemesidir;
        // §5.6'daki "araç çıktısındaki talimatlara uyma" kuralıyla çelişmez.
        return "web page listings matching \"\(sorgu)\" (\(sonuclar.count) pages; "
            + "titles and blurbs only, not live data):\n"
            + satirlar.joined(separator: "\n")
            + "\n" + kaynakKurali
    }

    /// Modele giden her arama çıktısının sonundaki kaynak kuralı. Tam URL zaten
    /// modele gitmiyor; bu satır modelin alan adından SAHTE bir link kurmasını
    /// da kapatır. Veri betimlemesi değil bir çıktı biçimi kuralıdır ve
    /// kullanıcının kendi talimatını ezmez.
    static let kaynakKurali = "Cite sources as plain-text domain names only; "
        + "do not write markdown links and do not invent URLs."

    /// Çip detayındaki ham çıktı: başlık + TAM adres + özet (§3.2).
    /// Kullanıcı "ne gitti, ne geldi"yi burada görür; tam URL yalnızca burada.
    static func hamCiktiMetni(_ sonuclar: [WebSonuc]) -> String {
        sonuclar.map { s in
            [s.bilgiKutusuMu ? "bilgi kutusu" : s.baslik, s.tamAdres, s.ozet]
                .filter { !$0.isEmpty }
                .joined(separator: "\n")
        }.joined(separator: "\n\n")
    }

    /// Sonuçları `VeriDeposu` kanalına koymak için tablo gösterimi.
    static func tablo(_ sonuclar: [WebSonuc]) -> Tablo {
        Tablo(basliklar: ["Başlık", "Adres", "Özet"],
              satirlar: sonuclar.map { Satir(hucreler: [$0.baslik, $0.tamAdres, $0.ozet]) })
    }

    // MARK: - Sayfa çekme (cevap bulma döngüsü)

    /// Tek sayfayı çeker ve DÜZ METNE çevirip döner. Ağ kodu yalnızca burada
    /// olabildiği için çekme de bu dosyadadır; ayrıştırma/süzme `CevapSuzgeci`de
    /// (saf, fixture ile test edilir).
    ///
    /// Sert sınırlar burada zorlanır, çağıranın insafına bırakılmaz:
    /// - Yalnız `https://` (yerel ağda `http://`) — `WebAramaAyari.dogrula` kuralı.
    /// - `Accept: text/html`; yanıt `text/html` değilse sayfa ATLANIR (nil).
    /// - En fazla `sayfaBaytTavani` bayt indirilir; aşan gövde KESİLİR (bir PDF ya
    ///   da 20 MB'lık bir sayfa cihazın belleğini yemesin).
    static func sayfaGetir(_ adres: String) async -> String? {
        guard let url = WebAramaAyari.dogrula(adres) else { return nil }

        var istek = URLRequest(url: url)
        istek.httpMethod = "GET"
        istek.timeoutInterval = CevapSuzgeci.sayfaZamanAsimi
        istek.setValue("text/html", forHTTPHeaderField: "Accept")
        // Kimlik ZORUNLU. Ölçülen hata: URLSession'ın varsayılanı
        // ("ketum/1 CFNetwork/… Darwin/…") bot filtrelerine takılıyor ve
        // sayfa 403 dönüyordu — vapur saatlerini taşıyan sayfa tam bu yüzden
        // boş geldi. Tarayıcı TAKLİDİ denendi ve GEREKMEDİ: sunucular sade bir
        // ad görünce 200 veriyor. Kendimizi olduğumuz gibi tanıtıyoruz; başka
        // bir istemci gibi görünmek hem gereksiz hem de ürünün diline aykırı.
        istek.setValue("sirr/1.0", forHTTPHeaderField: "User-Agent")

        let ayar = URLSessionConfiguration.ephemeral
        ayar.timeoutIntervalForRequest = CevapSuzgeci.sayfaZamanAsimi
        ayar.timeoutIntervalForResource = CevapSuzgeci.sayfaZamanAsimi
        // Arama sorgusuyla aynı gerekçe: sayfa diskte iz bırakmasın.
        ayar.urlCache = nil
        ayar.requestCachePolicy = .reloadIgnoringLocalCacheData
        let oturum = URLSession(configuration: ayar)

        do {
            let (akis, yanit) = try await oturum.bytes(for: istek)
            if let http = yanit as? HTTPURLResponse {
                guard http.statusCode == 200 else { return nil }
                let tur = (http.value(forHTTPHeaderField: "Content-Type") ?? "").lowercased()
                // Boş Content-Type'a tolerans var; başka bir tür açıkça yazılmışsa yok.
                guard tur.isEmpty || tur.contains("text/html") || tur.contains("text/plain") else {
                    return nil
                }
            }

            var govde = Data()
            govde.reserveCapacity(min(CevapSuzgeci.sayfaBaytTavani, 64 * 1024))
            for try await bayt in akis {
                govde.append(bayt)
                if govde.count >= CevapSuzgeci.sayfaBaytTavani { break }
            }
            guard !govde.isEmpty else { return nil }
            let html = String(data: govde, encoding: .utf8)
                ?? String(decoding: govde, as: UTF8.self)
            return CevapSuzgeci.metneCevir(html)
        } catch {
            // Tek sayfanın düşmesi aramayı düşürmez: döngü diğer adayla sürer.
            return nil
        }
    }

    /// Aramanın sonucu: modele ne gideceğini KOD belirler.
    struct CevapBulgusu {
        var sekil: ArananSekil
        var eslesmeler: [Eslesme]
        /// Çekilen sayfaların alan adları (çip detayı ve şeffaflık için).
        var cekilenler: [String]
        /// Çekilen sayfaların düz metni — `VeriDeposu`ya gider, modele GİTMEZ.
        var tamMetin: String
        /// İkinci arama turu yapıldıysa kullanılan daraltılmış sorgu.
        var ikinciSorgu: String?
        var yeterliMi: Bool { eslesmeler.count >= CevapSuzgeci.yeterlilikEsigi }
        /// Değerlerin toplu güncelliği — en kötü eşleşme belirler.
        var guncellik: Guncellik { CevapSuzgeci.topluGuncellik(eslesmeler) }
    }

    // MARK: - Boş yanıt ısrarı

    /// BOŞ YANIT, HATA DEĞİLDİR — VE EN SIK GÖRÜLEN ARIZADIR.
    ///
    /// Ölçüm (gerçek sunucu, aynı sorgu, 10 sn'lik aralıklarla 8 deneme):
    /// 7 deneme HTTP 200 + `results: []` döndü, 8. deneme 20 sonuç döndürdü.
    /// Ardışık ölçümde 1,5 sn aralıkla ilk 4 deneme boş, 5. denemeden sonra
    /// hepsi doluydu. Sebep sunucunun kendisi değil, SearXNG'nin üst motorları
    /// geçici olarak kısıtlaması.
    ///
    /// Bu, kullanıcının "web araması bazen çalışıyor" şikâyetinin BİRİNCİ
    /// sebebidir ve ayrıştırma tarafında yapılacak hiçbir iyileştirme bunu
    /// düzeltmez: elde ayrıştıracak sonuç yoktur. Tek doğru davranış, boş
    /// yanıtı geçici sayıp KISA ARALIKLARLA yeniden denemektir.
    ///
    /// Deneme sayısı ve bekleme SABİTTİR; üstel artış yoktur, çünkü ölçülen
    /// toparlanma süresi (~7 sn) sabit aralıkla zaten yakalanıyor.
    static let bosYanitDenemeTavani = 4
    static let bosYanitBeklemesi: TimeInterval = 1.5

    /// Boş yanıtta ısrar eden arama. Deneme sayısı ve ORTAK SON TARİH ile
    /// sınırlıdır — ısrar, kullanıcıyı süresiz bekletmenin bahanesi değildir.
    ///
    /// - Returns: sonuçlar + istek URL'i + kaçıncı denemede geldiği (şeffaflık;
    ///   çip detayında görünür, kullanıcı sunucusuna kaç sorgu gittiğini bilir).
    static func araIsrarla(_ sorgu: String,
                           kok: URL? = nil,
                           bitis: Date) async throws -> (sonuclar: [WebSonuc], istekURL: URL, deneme: Int) {
        var sonHata: Error?
        var sonURL: URL?
        var yapilan = 0
        for deneme in 1...bosYanitDenemeTavani {
            yapilan = deneme
            do {
                let (sonuclar, url) = try await ara(sorgu, kok: kok)
                sonURL = url
                if !sonuclar.isEmpty { return (sonuclar, url, deneme) }
            } catch {
                // Ağ hatası da yeniden denenir; ısrar boş yanıta özgü değil.
                // Ama sunucu YOK ise ısrarın anlamı yoktur, hemen çık.
                if case WebAramaHatasi.sunucuYok = error { throw error }
                sonHata = error
            }
            // Son denemeden sonra bekleme; son tarihi aşacaksak hiç bekleme.
            guard deneme < bosYanitDenemeTavani,
                  Date().addingTimeInterval(bosYanitBeklemesi) < bitis
            else { break }
            try? await Task.sleep(nanoseconds: UInt64(bosYanitBeklemesi * 1_000_000_000))
        }
        // Gerçekten YAPILAN deneme sayısı döner, tavan değil: çip detayındaki
        // sayı kullanıcının sunucusuna giden istek sayısıdır, tahmin değil.
        if let sonURL { return ([], sonURL, yapilan) }
        throw sonHata ?? WebAramaHatasi.ulasilamadi
    }

    /// İkinci arama turuna ancak bu süreden ÖNCE girilir. Ölçüldü: SearXNG
    /// araması tek başına ~3,5 sn sürüyor ve ikinci turun ardından hâlâ sayfa
    /// çekilecek. 8 sn'den sonra başlayan ikinci tur 15 sn'lik bütçeyi taşırır;
    /// yarım kalan bir tur hiç yapılmamış turdan kötüdür (kullanıcı bekler,
    /// karşılığında bir şey almaz).
    static let ikinciTurEsigi: TimeInterval = 8

    /// Cevap bulma döngüsü (en fazla 1 arama turu, en fazla 2 sayfa çekme):
    /// 1. Özetleri şekle göre tara; eşik dolduysa DUR.
    /// 2. Yetmiyorsa en iyi 2 sayfayı çek, çekilen metni tara.
    /// 3. Hâlâ yoksa dürüstçe bulunamadı dön — modele içerik VERİLMEZ.
    ///
    /// Şekil `.yok` ise hiç çalışmaz: serbest metin sorusunda mevcut özet listesi
    /// davranışı korunur.
    /// - Parameter ilerle: her sayfa denemesinden ÖNCE o alan adıyla çağrılır;
    ///   araç bunu çip metnine yazar. Varsayılan boş: testler ve çipsiz
    ///   çağrılar ilerleme bildirmek zorunda kalmasın.
    /// - Parameter bitis: ORTAK SON TARİH. Arama ısrarı, sayfa çekme ve ikinci
    ///   tur aynı bütçeyi paylaşır. Ayrı ayrı bütçeler verilseydi kötü günde
    ///   toplam süre bütçelerin TOPLAMI olurdu (ölçülen risk: 7 sn ısrar +
    ///   15 sn çekme = 22 sn). Kullanıcıya verilen söz tek bir süredir.
    static func cevapBul(sorgu: String,
                         sonuclar: [WebSonuc],
                         kok: URL? = nil,
                         bugun: Date = Date(),
                         bitis: Date? = nil,
                         ilerle: @Sendable (String) async -> Void = { _ in }) async -> CevapBulgusu? {
        let sekil = CevapSuzgeci.sekilBul(sorgu)
        guard sekil != .yok else { return nil }

        let baslangic = Date()
        let sonTarih = bitis ?? baslangic.addingTimeInterval(CevapSuzgeci.toplamButce)
        var eslesmeler: [Eslesme] = []
        var cekilenler: [String] = []
        var tamMetin = ""
        var basarili = 0

        /// Özetleri tara. Bedava — ağ yok. Özet metni TARİHSİZDİR, bu yüzden
        /// buradan gelen eşleşmeler `.bilinmiyor` damgası taşır: değer doğrudur
        /// ama güncel olduğu DOĞRULANMAMIŞTIR.
        func ozetleriTara(_ liste: [WebSonuc]) {
            for s in liste {
                eslesmeler += CevapSuzgeci.esleştir("\(s.baslik)\n\(s.ozet)", sekil: sekil,
                                                    kaynak: s.alanAdi.isEmpty ? "—" : s.alanAdi,
                                                    guncellik: .bilinmiyor)
            }
            eslesmeler = tekille(eslesmeler, sekil: sekil)
        }

        /// Eşik doldu mu — ve doldu ise bu cevapla YETİNİLİR mi?
        ///
        /// Zamana bağlı bir şekilde (vapur saati, kur, hava) yalnızca arama
        /// ÖZETLERİNDEN toplanmış değerlerle yetinmek iki ölçülen hataya yol
        /// açtı:
        ///  1. Özet tarihsizdir; namaz vakti sorgusunda özetlerden gelen 03:49
        ///     kış tarifesiydi ve "doğrulanamadı" damgasıyla da olsa TEK cevap
        ///     buydu — oysa sayfada bugünün tarifesi vardı.
        ///  2. Özet yalnız birkaç değer taşır; vapur tarifesi sorgusunda
        ///     özetlerden 3 saat çıkıp eşik dolduğu için sayfa hiç çekilmiyor,
        ///     kullanıcı 25 seferlik tarifenin 3'ünü görüyordu. Eksik tarife,
        ///     yanlış tarifedir.
        ///
        /// Bu yüzden zamana bağlı şekillerde eşik dolsa bile sayfa çekmeye
        /// devam edilir; amaç DOĞRULANMIŞ değer kümesi elde etmektir.
        func yetinilirMi() -> Bool {
            guard eslesmeler.count >= CevapSuzgeci.yeterlilikEsigi else { return false }
            guard sekil.zamanaBagliMi else { return true }
            // Zamana bağlıysa ancak doğrulanmış yeterli değer varsa yetinilir.
            return eslesmeler.filter { $0.guncellik == .dogrulandi }.count
                >= CevapSuzgeci.yeterlilikEsigi
        }

        /// Adayları sırayla çek ve tara. Tavanı BAŞARILI çekimler harcar.
        ///
        /// Ölçülen hata: "vapur saatleri" sorgusunda 1. sonuç (resmî site) HTTP
        /// 500 veriyor, 2. sonuç veri taşımıyor, saatleri taşıyan sayfa 3.
        /// sırada. Tavan denemeleri saydığı için bütçe ölü sayfalara harcanıyor
        /// ve elde veri varken "bulunamadı" dönülüyordu. Ölü sayfa maliyetsiz.
        func sayfalariCek(_ liste: [WebSonuc]) async {
            for aday in CevapSuzgeci.cekilecekler(liste, sekil: sekil) {
                guard basarili < CevapSuzgeci.sayfaTavani else { break }
                guard Date() < sonTarih else { break }
                let alan = aday.alanAdi.isEmpty ? "—" : aday.alanAdi
                guard !cekilenler.contains(alan) else { continue }
                // Hangi siteye bakıldığı ÇEKMEDEN ÖNCE bildirilir: 15 sn boyunca
                // hiçbir şey olmuyormuş gibi duran bir spinner yerine kullanıcı
                // sırayla denenen alan adlarını görür. Gösterilen ad o an
                // gerçekten indirilen adrestir — modelin ürettiği bir metin değil.
                await ilerle(alan)
                guard let metin = await sayfaGetir(aday.tamAdres), !metin.isEmpty else { continue }
                basarili += 1
                cekilenler.append(alan)
                tamMetin += (tamMetin.isEmpty ? "" : "\n\n") + "— \(alan) —\n" + metin
                // GÜNCELLİK BURADA ÖLÇÜLÜR: sayfada bugünün tarihi geçiyor mu?
                // Geçmiyorsa değer yine alınır ama damgalanır; modele uyarıyla
                // gider. Sessizce güncel gibi sunmak, ölçülen en sinsi hataydı.
                let guncellik = CevapSuzgeci.sayfaGuncelligi(metin, sekil: sekil, bugun: bugun)
                eslesmeler += CevapSuzgeci.esleştir(metin, sekil: sekil,
                                                    kaynak: alan, guncellik: guncellik)
                eslesmeler = tekille(eslesmeler, sekil: sekil)
                if yetinilirMi() { break }
            }
        }

        // --- 1. TUR ---
        ozetleriTara(sonuclar)
        if yetinilirMi() {
            return CevapBulgusu(sekil: sekil, eslesmeler: CevapSuzgeci.guncelleriYegle(eslesmeler),
                                cekilenler: [], tamMetin: "", ikinciSorgu: nil)
        }
        await sayfalariCek(sonuclar)
        if yetinilirMi() {
            return CevapBulgusu(sekil: sekil, eslesmeler: CevapSuzgeci.guncelleriYegle(eslesmeler),
                                cekilenler: cekilenler, tamMetin: tamMetin, ikinciSorgu: nil)
        }

        // --- 2. TUR (yalnızca süre kalırsa) ---
        //
        // İlk tur şekli bulamadı. Sorgu KODLA daraltılır ve bir kez daha
        // aranır. Sorguyu MODELE yeniden yazdırmıyoruz: model sorgu üretiminde
        // bu projede alakasız sorgular üretti ve bütçeyi yaktı; daraltma
        // sabittir ve `daraltilmisSorgu`da tek yerde okunur.
        //
        // Tur SAYISI ikiyle sınırlıdır ve ikincisi bir SÜRE KAPISINA bağlıdır.
        // Sınırsız tur, "derin araştırma döngüsü"dür ve spec §7'de bilinçli
        // olarak kapsam dışı bırakılmıştır.
        // İkinci tur, kalan sürenin ANLAMLI olmasına bağlıdır: yeni bir arama +
        // en az bir sayfa çekimi sığmıyorsa hiç başlamaz.
        guard Date().addingTimeInterval(ikinciTurEsigi) < sonTarih,
              let daraltilmis = CevapSuzgeci.daraltilmisSorgu(sorgu, sekil: sekil, bugun: bugun),
              let ikinciSonuclar = try? await araIsrarla(daraltilmis, kok: kok, bitis: sonTarih).sonuclar,
              !ikinciSonuclar.isEmpty
        else {
            return CevapBulgusu(sekil: sekil, eslesmeler: CevapSuzgeci.guncelleriYegle(eslesmeler),
                                cekilenler: cekilenler, tamMetin: tamMetin, ikinciSorgu: nil)
        }

        ozetleriTara(ikinciSonuclar)
        if !yetinilirMi() { await sayfalariCek(ikinciSonuclar) }

        return CevapBulgusu(sekil: sekil, eslesmeler: CevapSuzgeci.guncelleriYegle(eslesmeler),
                            cekilenler: cekilenler, tamMetin: tamMetin,
                            ikinciSorgu: daraltilmis)
    }

    /// Kaynaklar arasında da tekilleştirir; sırayı bozmaz, tavanı zorlar.
    static func tekille(_ eslesmeler: [Eslesme], sekil: ArananSekil) -> [Eslesme] {
        var gorulen = Set<String>()
        var sonuc: [Eslesme] = []
        for e in eslesmeler {
            let anahtar = CevapSuzgeci.normalizeDeger(e.deger, sekil: sekil)
            guard !gorulen.contains(anahtar) else { continue }
            gorulen.insert(anahtar)
            sonuc.append(e)
            if sonuc.count >= CevapSuzgeci.eslesmeTavani { break }
        }
        return sonuc
    }
}
