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

    /// Modele giden TEK SATIRIN içerik tavanı (başlık — alan adı — özet birleşimi).
    /// Bütçe satır başına zorlanır: 5 satır × (~200 + önek) + üst satır ≈ 1150
    /// karakter ≈ 290 token (§5.5). Başlığı ya da özeti ayrı ayrı kırpmak yetmez —
    /// uzun başlık + uzun alan adı + tavan özet birlikte bütçeyi aşar; tek kapı
    /// satırın kendisidir. Ham çıktı (çip detayı) kırpılmadan kalır.
    static let satirTavani = 200

    /// Kırpılmış liste; hedef bütçe ≤ ~300 token. Sıfır sonuçta sabit `no_results`.
    static func modeleMetin(sorgu: String, sonuclar: [WebSonuc]) -> String {
        guard !sonuclar.isEmpty else { return "no_results" }
        let satirlar = sonuclar.enumerated().map { (i, s) -> String in
            let bas = s.bilgiKutusuMu ? "[infobox] " : ""
            let parcalar = kirp([s.baslik, s.alanAdi, s.ozet]
                .filter { !$0.isEmpty }
                .joined(separator: " — "), sinir: satirTavani)
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
    }

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
}
