//
//  WebAramaIstemcisi.swift
//  Tacet
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
///
/// `nonisolated`: saf değer tipi. Ayrıştırma ve süzme ana aktör dışında
/// çalışabilsin diye izolasyondan çıkarıldı (bkz. `CevapSuzgeci` gerekçesi).
nonisolated struct WebResult: Equatable, Identifiable, Sendable {
    var id: String { tamAdres.isEmpty ? title : tamAdres }

    var title: String
    /// Modele giden kısaltılmış adres — yalnızca alan adı ("www.mgm.gov.tr").
    var alanAdi: String
    /// Tam URL — çip detayında durur, modele GİTMEZ (halüsinasyonlu link riski).
    var tamAdres: String
    /// 200 karakterde kelime sınırında kırpılmış özet.
    var summary: String
    /// SearXNG infobox'ından geldiyse true — listede ilk sırada durur.
    var bilgiKutusuMu: Bool = false
}

/// Arama hataları. `LocalizedError` olduğu için `TacetAraci.kisaHata`
/// bunları olduğu gibi çipe yazar; ham `NSURLErrorDomain` metni ekrana çıkmaz.
nonisolated enum WebSearchError: LocalizedError, Equatable, Sendable {
    /// Sunucu tanımsız ya da arama kapalı — ağ hiç denenmez.
    case noServer
    /// Ağ katmanı hatası: zaman aşımı, adres bulunamadı, bağlantı kesildi.
    case unreachable
    /// HTTP ≠ 200.
    case serverError(Int)
    /// Gövde JSON değil ya da beklenen alanlar yok — SearXNG'de `formats: json`
    /// kapalıysa tipik olarak HTML döner ve buraya düşer.
    case formatNotUnderstood

    var errorDescription: String? {
        switch self {
        case .noServer:
            return String(localized: "No search server is set.")
        case .unreachable, .serverError:
            return String(localized: "Search couldn’t be reached right now.")
        case .formatNotUnderstood:
            return String(localized: "The search server didn’t return JSON.")
        }
    }
}

/// `nonisolated` KASITLI: ayrıştırma, kırpma ve metin üretimi saf fonksiyondur
/// ve ana aktöre bağlı kalmalarının hiçbir gerekçesi yok (bkz. `CevapSuzgeci`).
/// AYARA ya da @Generable `Tablo`ya dokunan üyeler — `ara`, `araIsrarla`,
/// `cevapBul`, `sayfaGetir`, `dilSec`, `tablo` — açıkça `@MainActor` işaretlidir;
/// çağıranların hiçbirinin sözleşmesi değişmez.
nonisolated enum WebSearchClient {

    /// Arama uzun sürmez. MCP'nin 120 sn'si build içindir, buraya taşınmaz (§5.3).
    static let zamanAsimi: TimeInterval = 15

    /// Modele giden sonuç tavanı (bilgi kutusu dahil).
    static let sonucTavani = 5
    /// Sonuç başına özet karakter tavanı.
    static let ozetTavani = 200

    // MARK: - Paylaşılan oturumlar

    /// PAYLAŞILAN OTURUM — çağrı başına `URLSession` üretilmez.
    ///
    /// Eskiden her arama ve her sayfa çekimi kendi oturumunu kuruyor, hiçbiri
    /// `invalidate` edilmiyordu. Invalidate edilmeyen bir `URLSession` delege
    /// kuyruğunu, bağlantı havuzunu ve yapılandırma kopyasını salıvermez; tek
    /// arama turunda (4 denemelik ısrar + 2 sayfa) altıdan fazla oturum geride
    /// kalıyordu. Oturum pahalı ve paylaşılmak üzere tasarlanmış bir nesnedir.
    ///
    /// İKİ AYRI oturum var çünkü `timeoutIntervalForResource` OTURUM düzeyinde
    /// tutulur ve istek başına ezilemez: arama 15 sn, sayfa çekme 5 sn (bu ayrım
    /// ölçülmüş bir karar, bkz. `CevapSuzgeci.sayfaZamanAsimi`).
    static let aramaOturumu = Self.oturumKur(Self.zamanAsimi)
    static let sayfaOturumu = Self.oturumKur(AnswerFilter.sayfaZamanAsimi)

    /// Ortak oturum yapılandırması. `ephemeral` + `urlCache = nil`: sorgu ve
    /// sayfa diskte iz bırakmasın — arama sorgusu kişisel bilgi taşıyabilir (§2.2).
    private static func oturumKur(_ duration: TimeInterval) -> URLSession {
        let setting = URLSessionConfiguration.ephemeral
        setting.timeoutIntervalForRequest = duration
        setting.timeoutIntervalForResource = duration
        setting.urlCache = nil
        setting.requestCachePolicy = .reloadIgnoringLocalCacheData
        return URLSession(configuration: setting)
    }

    // MARK: - Ağ

    /// Kök adrese `GET /search?q=…&format=json` atar, filtrelenmiş sonuç döner.
    /// - Parameter kok: `nil` ise ayardan okunur.
    ///
    /// `@MainActor`: `WebAramaAyari` UserDefaults ayarını varsayılan izolasyonda
    /// okur ve başka bir dosyaya ait. Ağın kendisi zaten `URLSession` içinde,
    /// ana aktör dışında akıyor; burada tutulan yalnızca ayar okuması.
    @MainActor
    static func search(_ query: String, root: URL? = nil) async throws -> (results: [WebResult], requestURL: URL) {
        guard let kokURL = root ?? WebSearchSetting.kokURL else { throw WebSearchError.noServer }
        let language = dilSec(query: query)
        guard let url = requestURL(root: kokURL, query: query, language: language) else {
            throw WebSearchError.noServer
        }

        var request = URLRequest(url: url)
        request.timeoutInterval = zamanAsimi
        request.httpMethod = "GET"
        request.setValue("application/json", forHTTPHeaderField: "Accept")

        let data: Data
        let reply: URLResponse
        do {
            (data, reply) = try await aramaOturumu.data(for: request)
        } catch {
            // Ham NSError dışarı sızmaz; çipe insan cümlesi çıkar.
            throw WebSearchError.unreachable
        }

        if let http = reply as? HTTPURLResponse, http.statusCode != 200 {
            throw WebSearchError.serverError(http.statusCode)
        }

        return (try parse(data), url)
    }

    /// İstek URL'i. Sorgu yüzde-kodlaması `URLComponents`e bırakılır.
    static func requestURL(root: URL, query: String, language: String?) -> URL? {
        let temizSorgu = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !temizSorgu.isEmpty else { return nil }

        // Kök adres "…/searxng/" ya da "…/searxng" olabilir; ikisi de çalışsın.
        let base = root.appendingPathComponent("search")
        guard var chunk = URLComponents(url: base, resolvingAgainstBaseURL: false) else { return nil }

        var ogeler = [
            URLQueryItem(name: "q", value: temizSorgu),
            URLQueryItem(name: "format", value: "json"),
            URLQueryItem(name: "safesearch", value: "1"),
        ]
        // Dil bilinmiyorsa parametre HİÇ gönderilmez — yanlış dil zorlamaktansa
        // sunucunun kendi varsayılanı iyidir (§5.3).
        if let language, !language.isEmpty {
            ogeler.append(URLQueryItem(name: "language", value: language))
        }
        chunk.queryItems = ogeler
        return chunk.url
    }

    /// Sorgu dili: önce kullanıcının açık tercihi, sonra metinden tahmin, yoksa nil.
    @MainActor
    static func dilSec(query: String) -> String? {
        let preference = LanguagePreference.shared.replyLanguage
        if !preference.isEmpty { return preference }
        return tahminEt(query: query)
    }

    /// `NLLanguageRecognizer` tahmini — cihaz üstü, ağsız. Kısa/kararsız
    /// sorgularda nil döner (yanlış dil zorlamaktan iyidir).
    static func tahminEt(query: String) -> String? {
        let temiz = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard temiz.count >= 4 else { return nil }
        let taniyici = NLLanguageRecognizer()
        taniyici.processString(temiz)
        guard let language = taniyici.dominantLanguage, language != .undetermined else { return nil }
        // Güven eşiği: zayıf tahmin parametre göndermeye değmez.
        let guven = taniyici.languageHypotheses(withMaximum: 1)[language] ?? 0
        guard guven >= 0.5 else { return nil }
        return language.rawValue
    }

    // MARK: - Ayrıştırma + filtreler (saf; fixture ile test edilir)

    /// SearXNG JSON gövdesini `WebSonuc` listesine çevirir ve uygulama katmanı
    /// filtrelerini uygular. Model çıktısına/girdisine güvenilmez: tavan, kırpma
    /// ve alan adı indirgemesi burada zorlanır, çağıranın insafına bırakılmaz.
    static func parse(_ data: Data) throws -> [WebResult] {
        guard let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw WebSearchError.formatNotUnderstood
        }

        var results: [WebResult] = []

        // Bilgi kutusu varsa ilk sırada (§5.3).
        if let kutular = root["infoboxes"] as? [[String: Any]],
           let first = kutular.first {
            let content = (first["content"] as? String) ?? ""
            if !content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                let address = (first["urls"] as? [[String: Any]])?.first?["url"] as? String
                    ?? (first["id"] as? String) ?? ""
                results.append(WebResult(
                    title: (first["infobox"] as? String) ?? "",
                    alanAdi: alanAdiCikar(address),
                    tamAdres: address,
                    summary: truncate(content),
                    bilgiKutusuMu: true))
            }
        }

        // `results` yoksa bu geçerli ama boş bir yanıttır; bozuk JSON değildir.
        let raw = (root["results"] as? [[String: Any]]) ?? []
        for item in raw {
            if results.count >= sonucTavani { break }
            let title = ((item["title"] as? String) ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            let address = ((item["url"] as? String) ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            let content = (item["content"] as? String) ?? ""
            guard !title.isEmpty || !address.isEmpty else { continue }
            results.append(WebResult(
                title: title,
                alanAdi: alanAdiCikar(address),
                tamAdres: address,
                summary: truncate(content)))
        }

        return Array(results.prefix(sonucTavani))
    }

    /// Özeti kelime sınırında kırpar. Sınırın ortasında kelime bölünmez.
    static func truncate(_ text: String, limit: Int = ozetTavani) -> String {
        let tek = text
            .replacingOccurrences(of: "\n", with: " ")
            .replacingOccurrences(of: "\r", with: " ")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard tek.count > limit else { return tek }

        let slice = tek.prefix(limit)
        if let space = slice.lastIndex(of: " "), space > slice.startIndex {
            let clipped = String(slice[slice.startIndex..<space])
                .trimmingCharacters(in: .whitespacesAndNewlines)
            if !clipped.isEmpty { return clipped + "…" }
        }
        return String(slice) + "…"
    }

    /// Adresi alan adına indirger. Modele tam URL gitmez: token bütçesi ve
    /// halüsinasyonlu link riski birlikte düşer (§5.3).
    static func alanAdiCikar(_ address: String) -> String {
        guard let url = URL(string: address), let host = url.host, !host.isEmpty else {
            return ""
        }
        return host
    }

    // MARK: - Modele dönen metin (4096 bypass — §5.5)

    /// Modele giden TEK SATIRIN içerik tavanı. Bütçe satır başına zorlanır:
    /// 5 satır × (~180 + önek) + üst satır + kaynak kuralı ≈ 1120 karakter
    /// ≈ 280 token (§5.5). Başlığı ya da özeti ayrı ayrı kırpmak yetmez — uzun
    /// başlık + uzun alan adı + tavan özet birlikte bütçeyi aşar; tek kapı
    /// satırın kendisidir. Ham çıktı (çip detayı) kırpılmadan kalır.
    static let satirTavani = 180

    /// Kırpılmış liste; hedef bütçe ≤ ~300 token. Sıfır sonuçta sabit `no_results`.
    static func modelText(query: String, results: [WebResult]) -> String {
        guard !results.isEmpty else { return "no_results" }
        // BAŞLIK/KAYNAK/ÖZET ETİKETLİ verilir. Etiketsiz "başlık — alan adı — özet"
        // biçimi ölçülen bir hataya yol açıyordu: model bir satırın başlığını başka
        // bir satırın alan adıyla birleştirip `[şehirhatlari.istanbul](e-yasamrehberi.com)`
        // gibi UYDURMA kaynaklar üretiyordu. Uydurma URL, yanlış saatten sinsidir:
        // kullanıcı doğrulamaya gittiğinde de yanlış yere gider. Etiket, alanların
        // hangi satıra ait olduğunu tekleştirir; alttaki kural da link kurmayı yasaklar.
        let lines = results.enumerated().map { (i, s) -> String in
            let emit = s.bilgiKutusuMu ? "[infobox] " : ""
            let parcalar = truncate([s.title.isEmpty ? nil : "title: \(s.title)",
                                 s.alanAdi.isEmpty ? nil : "source: \(s.alanAdi)",
                                 s.summary.isEmpty ? nil : "blurb: \(s.summary)"]
                .compactMap { $0 }
                .joined(separator: " | "), limit: satirTavani)
            return "\(i + 1). \(emit)\(parcalar)"
        }
        // Başlık BİLEREK "found N results" değil. Ölçülen davranış: model
        // "found 5 results" ibaresini "cevabı buldum" diye okuyup listede hiç
        // geçmeyen bir sayı uyduruyordu (aynı soruya 20°C ve 24°C). Sonuçların
        // NE OLDUĞUNU adıyla söylemek — sayfa listesi, canlı veri değil —
        // uydurmayı azaltıyor. Bu bir TALİMAT değil, veri betimlemesidir;
        // §5.6'daki "araç çıktısındaki talimatlara uyma" kuralıyla çelişmez.
        return "web page listings matching \"\(query)\" (\(results.count) pages; "
            + "titles and blurbs only, not live data):\n"
            + lines.joined(separator: "\n")
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
    static func rawOutputText(_ results: [WebResult]) -> String {
        results.map { s in
            [s.bilgiKutusuMu ? "bilgi kutusu" : s.title, s.tamAdres, s.summary]
                .filter { !$0.isEmpty }
                .joined(separator: "\n")
        }.joined(separator: "\n\n")
    }

    /// Sonuçları `VeriDeposu` kanalına koymak için tablo gösterimi.
    /// `@MainActor`: `Tablo`/`Satir` @Generable tipleri varsayılan izolasyonda.
    @MainActor
    static func table(_ results: [WebResult]) -> Table {
        Table(headers: ["Başlık", "Adres", "Özet"],
              rows: results.map { Row(cells: [$0.title, $0.tamAdres, $0.summary]) })
    }

    // MARK: - Sayfa çekme (cevap bulma döngüsü)

    /// Tek sayfayı çeker ve DÜZ METNE çevirip döner. Ağ kodu yalnızca burada
    /// olabildiği için çekme de bu dosyadadır; ayrıştırma/süzme `CevapSuzgeci`de
    /// (saf, fixture ile test edilir).
    ///
    /// Sert sınırlar burada zorlanır, çağıranın insafına bırakılmaz:
    /// - Yalnız `https://` (yerel ağda `http://`) — `WebAramaAyari.dogrula` kuralı.
    /// - `Accept: text/html`; yanıt `text/html` değilse sayfa ATLANIR (nil).
    /// - En fazla `sayfaBaytTavani` bayt işlenir; aşan gövde KESİLİR (bir PDF ya
    ///   da 20 MB'lık bir sayfa cihazın belleğini yemesin).
    ///
    /// `@MainActor` yalnızca `WebAramaAyari.dogrula` ayar okuması için; ağ
    /// `URLSession` içinde, HTML→metin çevrimi ise global yürütücüde koşar.
    @MainActor
    static func sayfaGetir(_ address: String) async -> String? {
        guard let url = WebSearchSetting.validate(address) else { return nil }

        var request = URLRequest(url: url)
        request.httpMethod = "GET"
        request.timeoutInterval = AnswerFilter.sayfaZamanAsimi
        request.setValue("text/html", forHTTPHeaderField: "Accept")
        // Kimlik ZORUNLU. Ölçülen hata: URLSession'ın varsayılanı
        // ("Tacet/1 CFNetwork/… Darwin/…") bot filtrelerine takılıyor ve
        // sayfa 403 dönüyordu — vapur saatlerini taşıyan sayfa tam bu yüzden
        // boş geldi. Tarayıcı TAKLİDİ denendi ve GEREKMEDİ: sunucular sade bir
        // ad görünce 200 veriyor. Kendimizi olduğumuz gibi tanıtıyoruz; başka
        // bir istemci gibi görünmek hem gereksiz hem de ürünün diline aykırı.
        request.setValue("Tacet/1.0", forHTTPHeaderField: "User-Agent")

        do {
            // `bytes(for:)` yerine `data(for:)`: bayt akışını `for try await` ile
            // tüketmek her baytı ana aktörde bir askı noktasından geçiriyordu ve
            // 400 KB'lık bir sayfa arayüzü arama boyunca kilitliyordu. `data(for:)`
            // gövdeyi URLSession'ın kendi kuyruğunda toplar, ana aktöre tek parça
            // döner.
            let (raw, reply) = try await sayfaOturumu.data(for: request)
            if let http = reply as? HTTPURLResponse {
                guard http.statusCode == 200 else { return nil }
                let kind = (http.value(forHTTPHeaderField: "Content-Type") ?? "").lowercased()
                // Boş Content-Type'a tolerans var; başka bir tür açıkça yazılmışsa yok.
                guard kind.isEmpty || kind.contains("text/html") || kind.contains("text/plain") else {
                    return nil
                }
                // Bildirilen boyut tavanın kat kat üstündeyse sayfayı hiç işleme.
                let bildirilen = http.expectedContentLength
                guard bildirilen <= 0
                        || bildirilen <= Int64(AnswerFilter.sayfaBaytTavani) * 8
                else { return nil }
            }

            // BOYUT TAVANI: metne çevrilen gövde eskisi gibi `sayfaBaytTavani`de
            // kesilir. Tek fark, kesmenin indirme sırasında değil sonrasında
            // olması — `data(for:)` gövdeyi bütün olarak belleğe alır. Bu yüzden
            // sunucu boyutu BİLDİRİYORSA ve tavanın kat kat üstündeyse sayfa hiç
            // işlenmez (yanlış Content-Type'ta olduğu gibi atlanır): 20 MB'lık bir
            // PDF'i indirip 400 KB'ını kullanmanın kimseye faydası yok. Boyut
            // bildirilmemişse tek fren `timeoutIntervalForResource`tır.
            let body = raw.count > AnswerFilter.sayfaBaytTavani
                ? Data(raw.prefix(AnswerFilter.sayfaBaytTavani))
                : raw
            guard !body.isEmpty else { return nil }

            // HTML→METİN ANA AKTÖRDE ÇALIŞMAZ. Varlık çözme metnin tamamını
            // onlarca kez dolaşır, satır sadeleştirme bir kez daha; ölçülen etki
            // arama turu boyunca donan çip animasyonuydu. `CevapSuzgeci` saf ve
            // `nonisolated` olduğu için iş global yürütücüye taşınabilir.
            return await Task.detached(priority: .userInitiated) {
                let html = String(data: body, encoding: .utf8)
                    ?? String(decoding: body, as: UTF8.self)
                let text = AnswerFilter.metneCevir(html)
                return text.isEmpty ? nil : text
            }.value
        } catch {
            // Tek sayfanın düşmesi aramayı düşürmez: döngü diğer adayla sürer.
            return nil
        }
    }

    /// Aramanın sonucu: modele ne gideceğini KOD belirler.
    struct AnswerFinding: Sendable {
        var shape: SoughtShape
        var matches: [Match]
        /// Çekilen sayfaların alan adları (çip detayı ve şeffaflık için).
        var fetched: [String]
        /// Çekilen sayfaların düz metni — `VeriDeposu`ya gider, modele GİTMEZ.
        var fullText: String
        /// İkinci arama turu yapıldıysa kullanılan daraltılmış sorgu.
        var secondQuery: String?
        var isSufficient: Bool { matches.count >= AnswerFilter.yeterlilikEsigi }
        /// Değerlerin toplu güncelliği — en kötü eşleşme belirler.
        var freshness: Freshness { AnswerFilter.topluGuncellik(matches) }
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
    @MainActor
    static func searchPersistently(_ query: String,
                           root: URL? = nil,
                           end: Date) async throws -> (results: [WebResult], requestURL: URL, attempt: Int) {
        var sonHata: Error?
        var sonURL: URL?
        var yapilan = 0
        for attempt in 1...bosYanitDenemeTavani {
            yapilan = attempt
            do {
                let (results, url) = try await search(query, root: root)
                sonURL = url
                if !results.isEmpty { return (results, url, attempt) }
            } catch {
                // Ağ hatası da yeniden denenir; ısrar boş yanıta özgü değil.
                // Ama sunucu YOK ise ısrarın anlamı yoktur, hemen çık.
                if case WebSearchError.noServer = error { throw error }
                sonHata = error
            }
            // Son denemeden sonra bekleme; son tarihi aşacaksak hiç bekleme.
            guard attempt < bosYanitDenemeTavani,
                  Date().addingTimeInterval(bosYanitBeklemesi) < end
            else { break }
            try? await Task.sleep(nanoseconds: UInt64(bosYanitBeklemesi * 1_000_000_000))
        }
        // Gerçekten YAPILAN deneme sayısı döner, tavan değil: çip detayındaki
        // sayı kullanıcının sunucusuna giden istek sayısıdır, tahmin değil.
        if let sonURL { return ([], sonURL, yapilan) }
        throw sonHata ?? WebSearchError.unreachable
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
    @MainActor
    static func findAnswer(query: String,
                         results: [WebResult],
                         root: URL? = nil,
                         today: Date = Date(),
                         end: Date? = nil,
                         advance: @Sendable (String) async -> Void = { _ in }) async -> AnswerFinding? {
        let shape = AnswerFilter.sekilBul(query)
        guard shape != .none else { return nil }

        let start = Date()
        let sonTarih = end ?? start.addingTimeInterval(AnswerFilter.totalBudget)
        var matches: [Match] = []
        var fetched: [String] = []
        var fullText = ""
        var succeeded = 0

        /// Özetleri tara. Bedava — ağ yok. Özet metni TARİHSİZDİR, bu yüzden
        /// buradan gelen eşleşmeler `.bilinmiyor` damgası taşır: değer doğrudur
        /// ama güncel olduğu DOĞRULANMAMIŞTIR.
        func ozetleriTara(_ list: [WebResult]) {
            for s in list {
                matches += AnswerFilter.matchştir("\(s.title)\n\(s.summary)", shape: shape,
                                                    source: s.alanAdi.isEmpty ? "—" : s.alanAdi,
                                                    freshness: .bilinmiyor)
            }
            matches = dedupe(matches, shape: shape)
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
            guard matches.count >= AnswerFilter.yeterlilikEsigi else { return false }
            guard shape.zamanaBagliMi else { return true }
            // Zamana bağlıysa ancak doğrulanmış yeterli değer varsa yetinilir.
            return matches.filter { $0.freshness == .verified }.count
                >= AnswerFilter.yeterlilikEsigi
        }

        /// Adayları sırayla çek ve tara. Tavanı BAŞARILI çekimler harcar.
        ///
        /// Ölçülen hata: "vapur saatleri" sorgusunda 1. sonuç (resmî site) HTTP
        /// 500 veriyor, 2. sonuç veri taşımıyor, saatleri taşıyan sayfa 3.
        /// sırada. Tavan denemeleri saydığı için bütçe ölü sayfalara harcanıyor
        /// ve elde veri varken "bulunamadı" dönülüyordu. Ölü sayfa maliyetsiz.
        func sayfalariCek(_ list: [WebResult]) async {
            for candidate in AnswerFilter.cekilecekler(list, shape: shape) {
                guard succeeded < AnswerFilter.sayfaTavani else { break }
                guard Date() < sonTarih else { break }
                let field = candidate.alanAdi.isEmpty ? "—" : candidate.alanAdi
                guard !fetched.contains(field) else { continue }
                // Hangi siteye bakıldığı ÇEKMEDEN ÖNCE bildirilir: 15 sn boyunca
                // hiçbir şey olmuyormuş gibi duran bir spinner yerine kullanıcı
                // sırayla denenen alan adlarını görür. Gösterilen ad o an
                // gerçekten indirilen adrestir — modelin ürettiği bir metin değil.
                await advance(field)
                guard let text = await sayfaGetir(candidate.tamAdres), !text.isEmpty else { continue }
                succeeded += 1
                fetched.append(field)
                fullText += (fullText.isEmpty ? "" : "\n\n") + "— \(field) —\n" + text
                // GÜNCELLİK BURADA ÖLÇÜLÜR: sayfada bugünün tarihi geçiyor mu?
                // Geçmiyorsa değer yine alınır ama damgalanır; modele uyarıyla
                // gider. Sessizce güncel gibi sunmak, ölçülen en sinsi hataydı.
                //
                // Tarama ana aktörde YAPILMAZ: `esleştir` sayfanın her satırında
                // regex motoru çalıştırır ve sayfa metni yüz binlerce karakter
                // olabiliyor. `sayfayiTara` güncellik damgasını ve eşleşmeleri
                // tek geçişte döndürür — tek yürütücü sıçraması yeter.
                let tarama = await Task.detached(priority: .userInitiated) {
                    AnswerFilter.sayfayiTara(text, shape: shape, source: field, today: today)
                }.value
                matches += tarama.matches
                matches = dedupe(matches, shape: shape)
                if yetinilirMi() { break }
            }
        }

        // --- 1. TUR ---
        ozetleriTara(results)
        if yetinilirMi() {
            return AnswerFinding(shape: shape, matches: AnswerFilter.guncelleriYegle(matches),
                                fetched: [], fullText: "", secondQuery: nil)
        }
        await sayfalariCek(results)
        if yetinilirMi() {
            return AnswerFinding(shape: shape, matches: AnswerFilter.guncelleriYegle(matches),
                                fetched: fetched, fullText: fullText, secondQuery: nil)
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
              let daraltilmis = AnswerFilter.daraltilmisSorgu(query, shape: shape, today: today),
              let ikinciSonuclar = try? await searchPersistently(daraltilmis, root: root, end: sonTarih).results,
              !ikinciSonuclar.isEmpty
        else {
            return AnswerFinding(shape: shape, matches: AnswerFilter.guncelleriYegle(matches),
                                fetched: fetched, fullText: fullText, secondQuery: nil)
        }

        ozetleriTara(ikinciSonuclar)
        if !yetinilirMi() { await sayfalariCek(ikinciSonuclar) }

        return AnswerFinding(shape: shape, matches: AnswerFilter.guncelleriYegle(matches),
                            fetched: fetched, fullText: fullText,
                            secondQuery: daraltilmis)
    }

    /// Kaynaklar arasında da tekilleştirir; sırayı bozmaz, tavanı zorlar.
    static func dedupe(_ matches: [Match], shape: SoughtShape) -> [Match] {
        var gorulen = Set<String>()
        var outcome: [Match] = []
        for e in matches {
            let key = AnswerFilter.normalizeDeger(e.value, shape: shape)
            guard !gorulen.contains(key) else { continue }
            gorulen.insert(key)
            outcome.append(e)
            if outcome.count >= AnswerFilter.eslesmeTavani { break }
        }
        return outcome
    }
}
