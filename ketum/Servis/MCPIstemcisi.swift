//
//  MCPIstemcisi.swift
//  ketum
//
//  Uygulamadaki TEK ağ kodu (mcp-baglanti-spec §5.1, §2.1). Başka hiçbir katman
//  URLSession'a dokunmaz. Hiç bağlantı eklenmemişse burası hiç çağrılmaz ve
//  uygulamanın ağ trafiği sıfırdır.
//
//  ELLE YAZILDI — resmî `modelcontextprotocol/swift-sdk` KULLANILMADI (spec §5.2'den
//  bilinçli sapma). Projede sıfır SPM paketi var; sıfır bağımlılık ürünün kimliği
//  (OOXML zip'i bile saf Swift). v1'in ihtiyacı üç metot: initialize, tools/list,
//  tools/call. Taşıma: Streamable HTTP — JSON-RPC 2.0 gövdeleri HTTP POST ile
//  gider, yanıt `application/json` ya da `text/event-stream` (SSE) olabilir.
//
//  Zaman aşımı 120 sn (§5.7): uzak taraf build gibi uzun işler yapabilir.
//  İptal: çağıran Task iptal edilirse (uygulama arka plana gitti) istek düşer ve
//  `.iptal` döner — çip "yarıda kaldı" olur, sessiz kaybolma yoktur.
//

import Foundation

// MARK: - JSON değeri

/// Şemasız JSON. MCP araç şemaları ve argümanları derleme anında bilinmediği için
/// gereken en küçük gösterim. (Kod tabanında başka JSON tipi yok.)
///
/// `nonisolated` KASITLI: proje `SWIFT_DEFAULT_ACTOR_ISOLATION = MainActor` ile
/// derleniyor, yani izolasyonu belirtilmemiş her tip varsayılan olarak MainActor'a
/// bağlanır. Saf bir değer tipinin arayüz kuyruğuna bağlanmasının anlamı yok:
/// `MCPIstemcisi` bir actor ve ağ yanıtlarını MainActor dışında çözüyor, orada
/// `metinMi`/`diziMi` gibi üyelere dokunmak Swift 6 dilinde HATA olurdu.
/// Tip zaten `Sendable` — izolasyondan çıkması güvenli.
nonisolated indirect enum JSONDeger: Codable, Hashable, Sendable {
    case yok
    case mantik(Bool)
    case sayi(Double)
    case metin(String)
    case dizi([JSONDeger])
    case nesne([String: JSONDeger])

    init(from decoder: Decoder) throws {
        let k = try decoder.singleValueContainer()
        if k.decodeNil() { self = .yok }
        else if let v = try? k.decode(Bool.self) { self = .mantik(v) }
        else if let v = try? k.decode(Double.self) { self = .sayi(v) }
        else if let v = try? k.decode(String.self) { self = .metin(v) }
        else if let v = try? k.decode([JSONDeger].self) { self = .dizi(v) }
        else if let v = try? k.decode([String: JSONDeger].self) { self = .nesne(v) }
        else { self = .yok }
    }

    func encode(to encoder: Encoder) throws {
        var k = encoder.singleValueContainer()
        switch self {
        case .yok:          try k.encodeNil()
        case .mantik(let v): try k.encode(v)
        case .sayi(let v):   try k.encode(v)
        case .metin(let v):  try k.encode(v)
        case .dizi(let v):   try k.encode(v)
        case .nesne(let v):  try k.encode(v)
        }
    }

    // Gezinme kolaylıkları — MCP yanıtları elle okunur.
    subscript(_ anahtar: String) -> JSONDeger? {
        if case .nesne(let s) = self { return s[anahtar] }
        return nil
    }
    var metinMi: String? { if case .metin(let v) = self { return v }; return nil }
    var diziMi: [JSONDeger]? { if case .dizi(let v) = self { return v }; return nil }
    var mantikMi: Bool? { if case .mantik(let v) = self { return v }; return nil }

    /// Düz metin gösterimi — onay sayfasında kullanıcıya gösterilecek argüman
    /// metni ve çip ham girdisi bundan üretilir.
    var duzMetin: String {
        let kodlayici = JSONEncoder()
        kodlayici.outputFormatting = [.prettyPrinted, .sortedKeys]
        guard let veri = try? kodlayici.encode(self),
              let s = String(data: veri, encoding: .utf8) else { return "" }
        return s
    }

    /// JSON metninden ayrıştırır (MCPAraci modelin ürettiği argüman metnini böyle verir).
    static func ayristir(_ metin: String) -> JSONDeger? {
        guard let veri = metin.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(JSONDeger.self, from: veri)
    }
}

// MARK: - İstemci

/// Tek bir MCP sunucusuyla konuşan istemci. Bağlantı başına bir örnek; oturum
/// kimliği (`Mcp-Session-Id`) örnek içinde yaşar.
actor MCPIstemcisi {

    /// Bu istemcinin konuştuğu MCP sürümü. Sunucu farklı bir sürüm önerirse
    /// onunkine uyulur (MCP sürüm anlaşması).
    private static let protokolSurumu = "2025-06-18"

    /// §5.7 — build gibi uzun işler için. Hem URLSession'a hem de SSE okumasına uygulanır.
    static let zamanAsimi: TimeInterval = 120

    /// `tools/list` sayfalama döngüsünün üst sınırı — kötü davranan sunucu
    /// sonsuz cursor döndürüp uygulamayı döndürmesin.
    private static let enFazlaSayfa = 20

    /// Araç sayısı tavanı: 4096 pencereye zaten 6–8 araç giriyor; binlerce aracı
    /// belleğe almanın anlamı yok, içe aktarma da özetlemede boğulur.
    private static let enFazlaArac = 200

    /// Sunucudan gelen tek araç tanımı.
    /// `nonisolated` — actor içinde tanımlı olsa da saf veri; hem ağ tarafında
    /// hem MainActor'daki `BaglantiServisi`nde serbestçe gezmeli.
    nonisolated struct AracTanimi: Sendable, Hashable {
        let ad: String
        /// Ham açıklama — 100–500 token olabilir; bağlama BU HÂLİYLE GİRMEZ,
        /// `BaglantiServisi` özetler (§5.3).
        let aciklama: String
        /// `inputSchema` (JSON Şeması). MCPAraci bunu çalışma anında
        /// `DynamicGenerationSchema`ya çevirir.
        let sema: JSONDeger?
        /// MCP `annotations.readOnlyHint` — sunucu "bu araç hiçbir şey
        /// değiştirmez" diyorsa true. Sunucu söylemediyse nil.
        let saltOkumaIpucu: Bool?
        /// MCP `annotations.destructiveHint` — sunucu "bu araç yıkıcı" diyorsa
        /// true. İPUCUDUR, güvence değil: sunucu yalan söyleyebilir ya da hiç
        /// bildirmeyebilir, o yüzden `MCPAraci` ayrıca ad sözlüğüne bakar.
        let yikiciIpucu: Bool?

        init(ad: String, aciklama: String, sema: JSONDeger?,
             saltOkumaIpucu: Bool? = nil, yikiciIpucu: Bool? = nil) {
            self.ad = ad
            self.aciklama = aciklama
            self.sema = sema
            self.saltOkumaIpucu = saltOkumaIpucu
            self.yikiciIpucu = yikiciIpucu
        }
    }

    /// Hata yolları düz dille ayrışır (§3.1): kullanıcıya neden söylenir.
    /// `nonisolated` — ağ tarafında fırlatılır, MainActor'da gösterilir.
    nonisolated enum MCPHatasi: Error, Equatable, Sendable {
        case zamanAsimi
        case yetki                  // 401/403 — anahtar yok ya da geçersiz
        case tls                    // sertifika/TLS el sıkışması
        case erisilemedi            // ağ yok, sunucu kapalı, DNS
        case sunucu(String)         // JSON-RPC error ya da HTTP 4xx/5xx
        case bicimsiz               // yanıt MCP'ye uymuyor
        case iptal                  // uygulama arka plana gitti / kullanıcı durdurdu

        /// Kullanıcıya gösterilecek cümle. Dramatize etmez, ne olduğunu söyler.
        var aciklama: String {
            switch self {
            case .zamanAsimi:  return String(localized: "zaman aşımı")
            case .yetki:       return String(localized: "erişim anahtarı kabul edilmedi")
            case .tls:         return String(localized: "güvenli bağlantı kurulamadı")
            case .erisilemedi: return String(localized: "sunucuya erişilemedi")
            case .sunucu(let m):
                return m.isEmpty ? String(localized: "sunucu hata döndü")
                                 : String(localized: "sunucu hata döndü: \(m)")
            case .bicimsiz:    return String(localized: "sunucunun yanıtı anlaşılmadı")
            case .iptal:       return String(localized: "yarıda kaldı")
            }
        }
    }

    private let url: URL
    /// Bearer anahtarı — Keychain'den alınmış kopya; belleğin dışına yazılmaz.
    private let anahtar: String?
    private let oturum: URLSession

    /// `initialize` yanıtındaki `Mcp-Session-Id` başlığı; varsa sonraki her
    /// istekte taşınır. Sunucu oturum tutmuyorsa nil kalır (geçerli davranış).
    private var oturumKimligi: String?
    /// Sunucuyla uzlaşılan sürüm — el sıkışmadan sonra başlıkta gider.
    private var uzlasilanSurum: String = MCPIstemcisi.protokolSurumu
    private var elSikisildi = false
    private var sayac = 0

    init(url: URL, anahtar: String?) {
        self.url = url
        self.anahtar = anahtar
        let yapilandirma = URLSessionConfiguration.ephemeral
        yapilandirma.timeoutIntervalForRequest = MCPIstemcisi.zamanAsimi
        yapilandirma.timeoutIntervalForResource = MCPIstemcisi.zamanAsimi
        // Çerez/önbellek yok: uzak sunucuyla ilişkimiz istekten ibaret.
        yapilandirma.httpCookieAcceptPolicy = .never
        yapilandirma.httpShouldSetCookies = false
        yapilandirma.requestCachePolicy = .reloadIgnoringLocalAndRemoteCacheData
        self.oturum = URLSession(configuration: yapilandirma)
    }

    // MARK: - Üç metot (v1 kapsamı)

    /// El sıkışma. Bir kez yapılır; sonraki çağrılar dokunmaz.
    func elSikis() async throws {
        guard !elSikisildi else { return }
        let sonuc = try await cagir(metot: "initialize", parametre: .nesne([
            "protocolVersion": .metin(Self.protokolSurumu),
            "capabilities": .nesne(["tools": .nesne([:])]),
            "clientInfo": .nesne(["name": .metin("sirr"), "version": .metin("1.0")]),
        ]))
        if let s = sonuc["protocolVersion"]?.metinMi, !s.isEmpty { uzlasilanSurum = s }
        elSikisildi = true
        // MCP el sıkışmasının ikinci yarısı: sunucuya hazır olduğumuzu bildiren
        // bildirim (yanıt beklenmez). Bunu atlarsak katı sunucular tools/list'i
        // reddediyor — ayrı bir "metot" değil, initialize'ın parçası.
        try? await bildir(metot: "notifications/initialized")
    }

    /// Sunucunun araçları. Sayfalama (`nextCursor`) sonuna kadar döner.
    func araclar() async throws -> [AracTanimi] {
        try await elSikis()
        var toplam: [AracTanimi] = []
        var imlec: String?
        var sayfa = 0
        repeat {
            sayfa += 1
            var parametre: [String: JSONDeger] = [:]
            if let imlec { parametre["cursor"] = .metin(imlec) }
            let sonuc = try await cagir(metot: "tools/list", parametre: .nesne(parametre))
            for oge in sonuc["tools"]?.diziMi ?? [] {
                guard let ad = oge["name"]?.metinMi, !ad.isEmpty else { continue }
                let notlar = oge["annotations"]
                toplam.append(AracTanimi(ad: ad,
                                         aciklama: oge["description"]?.metinMi ?? "",
                                         sema: oge["inputSchema"],
                                         saltOkumaIpucu: notlar?["readOnlyHint"]?.mantikMi,
                                         yikiciIpucu: notlar?["destructiveHint"]?.mantikMi))
            }
            let sonraki = sonuc["nextCursor"]?.metinMi
            // Aynı imleci tekrar veren sunucu döngüye sokmasın.
            imlec = (sonraki == imlec) ? nil : sonraki
        } while imlec != nil && sayfa < Self.enFazlaSayfa && toplam.count < Self.enFazlaArac

        return Array(toplam.prefix(Self.enFazlaArac))
    }

    /// Aracı çağırır ve içeriği düz metne indirger.
    ///
    /// ÖNEMLİ: onay kapısı bu çağrıdan ÖNCE, `AracYurutucu.onayIste` ile geçilir.
    /// Buraya gelen her şey kullanıcının gördüğü ve onayladığı şeydir.
    /// - Returns: (metin, sunucuHatasi) — `isError` sunucunun kendi hatasıdır
    ///   (komut başarısız), taşıma hatası değildir; model onu okuyup anlatır.
    func aracCagir(ad: String, argumanlar: JSONDeger) async throws -> (metin: String, hataliMi: Bool) {
        try await elSikis()
        let sonuc = try await cagir(metot: "tools/call", parametre: .nesne([
            "name": .metin(ad),
            "arguments": argumanlar,
        ]))
        let parcalar: [String] = (sonuc["content"]?.diziMi ?? []).compactMap { oge in
            if let t = oge["text"]?.metinMi { return t }
            // Metin dışı içerik (görsel/ses) v1'de taşınmaz; sessizce yutmayalım.
            if let tur = oge["type"]?.metinMi, tur != "text" {
                return "[\(tur) içeriği — gösterilemiyor]"
            }
            return nil
        }
        let metin = parcalar.joined(separator: "\n")
        return (metin, sonuc["isError"]?.mantikMi ?? false)
    }

    // MARK: - JSON-RPC taşıması

    private func sonrakiKimlik() -> Int { sayac += 1; return sayac }

    private func istekKur(govde: Data) -> URLRequest {
        var istek = URLRequest(url: url)
        istek.httpMethod = "POST"
        istek.timeoutInterval = Self.zamanAsimi
        istek.httpBody = govde
        istek.setValue("application/json", forHTTPHeaderField: "Content-Type")
        // İki biçimi de kabul ederiz; sunucu hangisini seçerse ona göre okuruz.
        istek.setValue("application/json, text/event-stream", forHTTPHeaderField: "Accept")
        if elSikisildi {
            istek.setValue(uzlasilanSurum, forHTTPHeaderField: "MCP-Protocol-Version")
        }
        if let oturumKimligi {
            istek.setValue(oturumKimligi, forHTTPHeaderField: "Mcp-Session-Id")
        }
        if let anahtar, !anahtar.isEmpty {
            istek.setValue("Bearer \(anahtar)", forHTTPHeaderField: "Authorization")
        }
        return istek
    }

    /// Yanıt beklemeyen bildirim (JSON-RPC notification: `id` yok).
    private func bildir(metot: String) async throws {
        let govde: [String: JSONDeger] = ["jsonrpc": .metin("2.0"), "method": .metin(metot)]
        guard let veri = try? JSONEncoder().encode(JSONDeger.nesne(govde)) else { return }
        _ = try? await oturum.data(for: istekKur(govde: veri))
    }

    /// Tek JSON-RPC çağrısı — `result` nesnesini döndürür.
    private func cagir(metot: String, parametre: JSONDeger) async throws -> JSONDeger {
        let kimlik = sonrakiKimlik()
        let govde: [String: JSONDeger] = [
            "jsonrpc": .metin("2.0"),
            "id": .sayi(Double(kimlik)),
            "method": .metin(metot),
            "params": parametre,
        ]
        guard let veri = try? JSONEncoder().encode(JSONDeger.nesne(govde)) else {
            throw MCPHatasi.bicimsiz
        }
        let istek = istekKur(govde: veri)

        // URLSession'ın kendi zaman aşımı SSE akışında güvenilir tetiklenmiyor
        // (bayt gelmeye devam ettikçe sayaç sıfırlanır); üst sınırı biz koyarız.
        let yanit = try await zamanSinirli(Self.zamanAsimi) { [self] in
            try await self.akisOku(istek: istek, kimlik: kimlik)
        }

        if let hata = yanit["error"] {
            let mesaj = hata["message"]?.metinMi ?? ""
            throw MCPHatasi.sunucu(mesaj)
        }
        guard let sonuc = yanit["result"] else { throw MCPHatasi.bicimsiz }
        return sonuc
    }

    /// İsteği gönderir, gövdeyi biçimine göre okur ve bizim `id`li yanıtı döndürür.
    private func akisOku(istek: URLRequest, kimlik: Int) async throws -> JSONDeger {
        let baytlar: URLSession.AsyncBytes
        let yanit: URLResponse
        do {
            (baytlar, yanit) = try await oturum.bytes(for: istek)
        } catch {
            throw Self.hataCevir(error)
        }

        guard let http = yanit as? HTTPURLResponse else { throw MCPHatasi.bicimsiz }
        // Oturum kimliği el sıkışmada gelir; sonraki isteklerde taşınır.
        if let kimlikBasligi = http.value(forHTTPHeaderField: "Mcp-Session-Id"), !kimlikBasligi.isEmpty {
            oturumKimligi = kimlikBasligi
        }
        switch http.statusCode {
        case 200..<300: break
        case 401, 403:  throw MCPHatasi.yetki
        default:        throw MCPHatasi.sunucu("HTTP \(http.statusCode)")
        }

        let tur = (http.value(forHTTPHeaderField: "Content-Type") ?? "").lowercased()

        if tur.contains("text/event-stream") {
            return try await sseOku(baytlar, kimlik: kimlik)
        }

        // Düz JSON: satırları toplayıp tek gövde olarak çöz.
        var govde = Data()
        do {
            for try await bayt in baytlar {
                try Task.checkCancellation()
                govde.append(bayt)
            }
        } catch {
            throw Self.hataCevir(error)
        }
        guard let coz = try? JSONDecoder().decode(JSONDeger.self, from: govde) else {
            throw MCPHatasi.bicimsiz
        }
        // Sunucu toplu (batch) dizi döndürmüş olabilir — bizim id'yi seç.
        if let dizi = coz.diziMi {
            guard let benim = dizi.first(where: { Self.kimlikEsit($0, kimlik) }) else {
                throw MCPHatasi.bicimsiz
            }
            return benim
        }
        return coz
    }

    /// SSE ayrıştırması: `data:` satırları biriktirilir, BOŞ SATIRDA olay tamamlanır.
    /// Bizim `id`mize ait yanıt gelene kadar okumaya devam eder (sunucu araya
    /// ilerleme/log olayları koyabilir).
    private func sseOku(_ baytlar: URLSession.AsyncBytes, kimlik: Int) async throws -> JSONDeger {
        var tampon: [String] = []

        func olayiCoz() -> JSONDeger? {
            guard !tampon.isEmpty else { return nil }
            let gövde = tampon.joined(separator: "\n")
            tampon.removeAll()
            guard let veri = gövde.data(using: .utf8),
                  let coz = try? JSONDecoder().decode(JSONDeger.self, from: veri) else { return nil }
            if let dizi = coz.diziMi {
                return dizi.first(where: { Self.kimlikEsit($0, kimlik) })
            }
            return Self.kimlikEsit(coz, kimlik) ? coz : nil
        }

        do {
            for try await satir in baytlar.lines {
                try Task.checkCancellation()
                if satir.isEmpty {
                    if let olay = olayiCoz() { return olay }
                    continue
                }
                if satir.hasPrefix(":") { continue }        // yorum/heartbeat
                guard satir.hasPrefix("data:") else { continue } // event:/id:/retry: ilgilendirmez
                var parca = String(satir.dropFirst(5))
                if parca.hasPrefix(" ") { parca.removeFirst() }
                tampon.append(parca)
            }
        } catch {
            throw Self.hataCevir(error)
        }
        // Akış kapandı: son olay boş satırla kapanmamış olabilir.
        if let olay = olayiCoz() { return olay }
        throw MCPHatasi.bicimsiz
    }

    private nonisolated static func kimlikEsit(_ nesne: JSONDeger, _ kimlik: Int) -> Bool {
        guard let alan = nesne["id"] else { return false }
        switch alan {
        case .sayi(let v): return Int(v) == kimlik
        case .metin(let v): return Int(v) == kimlik
        default: return false
        }
    }

    /// URLError/CancellationError → düz dille ayrışan MCPHatasi (§3.1).
    private nonisolated static func hataCevir(_ hata: Error) -> MCPHatasi {
        if hata is CancellationError { return .iptal }
        if let mcp = hata as? MCPHatasi { return mcp }
        guard let u = hata as? URLError else { return .erisilemedi }
        switch u.code {
        case .timedOut:
            return .zamanAsimi
        case .cancelled:
            return .iptal
        case .secureConnectionFailed, .serverCertificateHasBadDate,
             .serverCertificateUntrusted, .serverCertificateHasUnknownRoot,
             .serverCertificateNotYetValid, .clientCertificateRejected,
             .clientCertificateRequired, .appTransportSecurityRequiresSecureConnection:
            return .tls
        case .userAuthenticationRequired:
            return .yetki
        default:
            return .erisilemedi
        }
    }
}

// MARK: - Süre sınırı

/// İşi verilen süre içinde bitirir; bitmezse `zamanAsimi` fırlatır ve işi iptal eder.
/// Dışarıdan gelen iptal (uygulama arka plana gitti) aynen aşağı geçer.
///
/// `nonisolated` — varsayılan MainActor izolasyonu bu yardımcıyı arayüz kuyruğuna
/// bağlardı; 120 saniyelik bir ağ beklemesinin orada işi yok.
private nonisolated func zamanSinirli<T: Sendable>(_ saniye: TimeInterval,
                                       _ is: @escaping @Sendable () async throws -> T) async throws -> T {
    try await withThrowingTaskGroup(of: T.self) { grup in
        grup.addTask { try await `is`() }
        grup.addTask {
            try await Task.sleep(nanoseconds: UInt64(saniye * 1_000_000_000))
            throw MCPIstemcisi.MCPHatasi.zamanAsimi
        }
        guard let ilk = try await grup.next() else { throw MCPIstemcisi.MCPHatasi.bicimsiz }
        grup.cancelAll()
        return ilk
    }
}
