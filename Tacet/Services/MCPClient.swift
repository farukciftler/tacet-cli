//
//  MCPIstemcisi.swift
//  Tacet
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
nonisolated indirect enum JSONValue: Codable, Hashable, Sendable {
    case none
    case mantik(Bool)
    case number(Double)
    case text(String)
    case array([JSONValue])
    case object([String: JSONValue])

    init(from decoder: Decoder) throws {
        let k = try decoder.singleValueContainer()
        if k.decodeNil() { self = .none }
        else if let v = try? k.decode(Bool.self) { self = .mantik(v) }
        else if let v = try? k.decode(Double.self) { self = .number(v) }
        else if let v = try? k.decode(String.self) { self = .text(v) }
        else if let v = try? k.decode([JSONValue].self) { self = .array(v) }
        else if let v = try? k.decode([String: JSONValue].self) { self = .object(v) }
        else { self = .none }
    }

    func encode(to encoder: Encoder) throws {
        var k = encoder.singleValueContainer()
        switch self {
        case .none:          try k.encodeNil()
        case .mantik(let v): try k.encode(v)
        case .number(let v):   try k.encode(v)
        case .text(let v):  try k.encode(v)
        case .array(let v):   try k.encode(v)
        case .object(let v):  try k.encode(v)
        }
    }

    // Gezinme kolaylıkları — MCP yanıtları elle okunur.
    subscript(_ key: String) -> JSONValue? {
        if case .object(let s) = self { return s[key] }
        return nil
    }
    var metinMi: String? { if case .text(let v) = self { return v }; return nil }
    var diziMi: [JSONValue]? { if case .array(let v) = self { return v }; return nil }
    var mantikMi: Bool? { if case .mantik(let v) = self { return v }; return nil }

    /// Düz metin gösterimi — onay sayfasında kullanıcıya gösterilecek argüman
    /// metni ve çip ham girdisi bundan üretilir.
    var duzMetin: String {
        let kodlayici = JSONEncoder()
        kodlayici.outputFormatting = [.prettyPrinted, .sortedKeys]
        guard let data = try? kodlayici.encode(self),
              let s = String(data: data, encoding: .utf8) else { return "" }
        return s
    }

    /// JSON metninden ayrıştırır (MCPAraci modelin ürettiği argüman metnini böyle verir).
    static func parse(_ text: String) -> JSONValue? {
        guard let data = text.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(JSONValue.self, from: data)
    }
}

// MARK: - İstemci

/// Tek bir MCP sunucusuyla konuşan istemci. Bağlantı başına bir örnek; oturum
/// kimliği (`Mcp-Session-Id`) örnek içinde yaşar.
actor MCPClient {

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
    nonisolated struct ToolSpec: Sendable, Hashable {
        let name: String
        /// Ham açıklama — 100–500 token olabilir; bağlama BU HÂLİYLE GİRMEZ,
        /// `BaglantiServisi` özetler (§5.3).
        let description: String
        /// `inputSchema` (JSON Şeması). MCPAraci bunu çalışma anında
        /// `DynamicGenerationSchema`ya çevirir.
        let schema: JSONValue?
        /// MCP `annotations.readOnlyHint` — sunucu "bu araç hiçbir şey
        /// değiştirmez" diyorsa true. Sunucu söylemediyse nil.
        let readOnlyHint: Bool?
        /// MCP `annotations.destructiveHint` — sunucu "bu araç yıkıcı" diyorsa
        /// true. İPUCUDUR, güvence değil: sunucu yalan söyleyebilir ya da hiç
        /// bildirmeyebilir, o yüzden `MCPAraci` ayrıca ad sözlüğüne bakar.
        let destructiveHint: Bool?

        init(name: String, description: String, schema: JSONValue?,
             readOnlyHint: Bool? = nil, destructiveHint: Bool? = nil) {
            self.name = name
            self.description = description
            self.schema = schema
            self.readOnlyHint = readOnlyHint
            self.destructiveHint = destructiveHint
        }
    }

    /// Hata yolları düz dille ayrışır (§3.1): kullanıcıya neden söylenir.
    /// `nonisolated` — ağ tarafında fırlatılır, MainActor'da gösterilir.
    nonisolated enum MCPError: Error, Equatable, Sendable {
        case zamanAsimi
        case yetki                  // 401/403 — anahtar yok ya da geçersiz
        case tls                    // sertifika/TLS el sıkışması
        case unreachable            // ağ yok, sunucu kapalı, DNS
        case server(String)         // JSON-RPC error ya da HTTP 4xx/5xx
        case bicimsiz               // yanıt MCP'ye uymuyor
        case cancelled                  // uygulama arka plana gitti / kullanıcı durdurdu

        /// Kullanıcıya gösterilecek cümle. Dramatize etmez, ne olduğunu söyler.
        var description: String {
            switch self {
            case .zamanAsimi:  return String(localized: "timed out")
            case .yetki:       return String(localized: "access key was rejected")
            case .tls:         return String(localized: "couldn’t establish a secure connection")
            case .unreachable: return String(localized: "couldn’t reach the server")
            case .server(let m):
                return m.isEmpty ? String(localized: "server returned an error")
                                 : String(localized: "server returned an error: \(m)")
            case .bicimsiz:    return String(localized: "couldn’t make sense of the server’s response")
            case .cancelled:       return String(localized: "stopped partway")
            }
        }
    }

    /// Sunucu `Mcp-Session-Id`mizi tanımadı (HTTP 404). Yalnızca bu dosyanın
    /// içinde yaşayan işaret: `MCPHatasi` kullanıcıya gösterilen sözleşmedir,
    /// bu ise taşıma katmanının kendi kendine toparlanma sinyali.
    private nonisolated struct SessionDropped: Error {}

    /// Oturum düştüğünde ÇAĞRISI TEKRARLANABİLEN metotlar. Ölçüt yan etkidir:
    /// bu ikisi sunucuda hiçbir şey değiştirmez, ikinci kez çalışmaları zararsız.
    /// `tools/call` bilerek DIŞARIDA — istek sunucuya ulaşıp yan etkisini
    /// bırakmış da olabilir (yanıt yolda kaybolmuş olabilir), tekrar göndermek
    /// e-postayı iki kez yollardı. Dünya değiştiyse retry yok.
    private static let yenidenDenenebilir: Set<String> = ["initialize", "tools/list"]

    private let url: URL
    /// Bearer anahtarı — Keychain'den alınmış kopya; belleğin dışına yazılmaz.
    private let key: String?
    private let session: URLSession

    /// `initialize` yanıtındaki `Mcp-Session-Id` başlığı; varsa sonraki her
    /// istekte taşınır. Sunucu oturum tutmuyorsa nil kalır (geçerli davranış).
    private var oturumKimligi: String?
    /// Sunucuyla uzlaşılan sürüm — el sıkışmadan sonra başlıkta gider.
    private var uzlasilanSurum: String = MCPClient.protokolSurumu
    private var elSikisildi = false
    private var counter = 0

    init(url: URL, key: String?) {
        self.url = url
        self.key = key
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = MCPClient.zamanAsimi
        config.timeoutIntervalForResource = MCPClient.zamanAsimi
        // Çerez/önbellek yok: uzak sunucuyla ilişkimiz istekten ibaret.
        config.httpCookieAcceptPolicy = .never
        config.httpShouldSetCookies = false
        config.requestCachePolicy = .reloadIgnoringLocalAndRemoteCacheData
        self.session = URLSession(configuration: config)
    }

    // MARK: - Üç metot (v1 kapsamı)

    /// El sıkışma. Bir kez yapılır; sonraki çağrılar dokunmaz.
    func elSikis() async throws {
        guard !elSikisildi else { return }
        let outcome = try await invoke(metot: "initialize", parametre: .object([
            "protocolVersion": .text(Self.protokolSurumu),
            "capabilities": .object(["tools": .object([:])]),
            "clientInfo": .object(["name": .text("tacet"), "version": .text("1.0")]),
        ]))
        if let s = outcome["protocolVersion"]?.metinMi, !s.isEmpty { uzlasilanSurum = s }
        elSikisildi = true
        // MCP el sıkışmasının ikinci yarısı: sunucuya hazır olduğumuzu bildiren
        // bildirim (yanıt beklenmez). Bunu atlarsak katı sunucular tools/list'i
        // reddediyor — ayrı bir "metot" değil, initialize'ın parçası.
        try? await notify(metot: "notifications/initialized")
    }

    /// Sunucunun araçları. Sayfalama (`nextCursor`) sonuna kadar döner.
    func tools() async throws -> [ToolSpec] {
        try await elSikis()
        var total: [ToolSpec] = []
        var caret: String?
        var sheet = 0
        repeat {
            sheet += 1
            var parametre: [String: JSONValue] = [:]
            if let caret { parametre["cursor"] = .text(caret) }
            let outcome = try await invoke(metot: "tools/list", parametre: .object(parametre))
            for item in outcome["tools"]?.diziMi ?? [] {
                guard let name = item["name"]?.metinMi, !name.isEmpty else { continue }
                let notlar = item["annotations"]
                total.append(ToolSpec(name: name,
                                         description: item["description"]?.metinMi ?? "",
                                         schema: item["inputSchema"],
                                         readOnlyHint: notlar?["readOnlyHint"]?.mantikMi,
                                         destructiveHint: notlar?["destructiveHint"]?.mantikMi))
            }
            let next = outcome["nextCursor"]?.metinMi
            // Aynı imleci tekrar veren sunucu döngüye sokmasın.
            caret = (next == caret) ? nil : next
        } while caret != nil && sheet < Self.enFazlaSayfa && total.count < Self.enFazlaArac

        return Array(total.prefix(Self.enFazlaArac))
    }

    /// Aracı çağırır ve içeriği düz metne indirger.
    ///
    /// ÖNEMLİ: onay kapısı bu çağrıdan ÖNCE, `AracYurutucu.onayIste` ile geçilir.
    /// Buraya gelen her şey kullanıcının gördüğü ve onayladığı şeydir.
    /// - Returns: (metin, sunucuHatasi) — `isError` sunucunun kendi hatasıdır
    ///   (komut başarısız), taşıma hatası değildir; model onu okuyup anlatır.
    func aracCagir(name: String, argumanlar: JSONValue) async throws -> (text: String, hataliMi: Bool) {
        try await elSikis()
        let outcome = try await invoke(metot: "tools/call", parametre: .object([
            "name": .text(name),
            "arguments": argumanlar,
        ]))
        let parcalar: [String] = (outcome["content"]?.diziMi ?? []).compactMap { item in
            if let t = item["text"]?.metinMi { return t }
            // Metin dışı içerik (görsel/ses) v1'de taşınmaz; sessizce yutmayalım.
            if let kind = item["type"]?.metinMi, kind != "text" {
                return "[\(kind) içeriği — gösterilemiyor]"
            }
            return nil
        }
        let text = parcalar.joined(separator: "\n")
        return (text, outcome["isError"]?.mantikMi ?? false)
    }

    // MARK: - JSON-RPC taşıması

    private func sonrakiKimlik() -> Int { counter += 1; return counter }

    private func istekKur(body: Data) -> URLRequest {
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.timeoutInterval = Self.zamanAsimi
        request.httpBody = body
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        // İki biçimi de kabul ederiz; sunucu hangisini seçerse ona göre okuruz.
        request.setValue("application/json, text/event-stream", forHTTPHeaderField: "Accept")
        if elSikisildi {
            request.setValue(uzlasilanSurum, forHTTPHeaderField: "MCP-Protocol-Version")
        }
        if let oturumKimligi {
            request.setValue(oturumKimligi, forHTTPHeaderField: "Mcp-Session-Id")
        }
        if let key, !key.isEmpty {
            request.setValue("Bearer \(key)", forHTTPHeaderField: "Authorization")
        }
        return request
    }

    /// Yanıt beklemeyen bildirim (JSON-RPC notification: `id` yok).
    private func notify(metot: String) async throws {
        let body: [String: JSONValue] = ["jsonrpc": .text("2.0"), "method": .text(metot)]
        guard let data = try? JSONEncoder().encode(JSONValue.object(body)) else { return }
        _ = try? await session.data(for: istekKur(body: data))
    }

    /// Tek JSON-RPC çağrısı — `result` nesnesini döndürür.
    ///
    /// - Parameter yenidenDenendi: tek denemelik bayrak. Oturum toparlama
    ///   yolunda `true` geçilir; ikinci bir 404 artık toparlanmaz, düz hata
    ///   döner. Özyineleme böylece sonsuza gitmez.
    private func invoke(metot: String, parametre: JSONValue,
                       yenidenDenendi: Bool = false) async throws -> JSONValue {
        let identity = sonrakiKimlik()
        let body: [String: JSONValue] = [
            "jsonrpc": .text("2.0"),
            "id": .number(Double(identity)),
            "method": .text(metot),
            "params": parametre,
        ]
        guard let data = try? JSONEncoder().encode(JSONValue.object(body)) else {
            throw MCPError.bicimsiz
        }
        let request = istekKur(body: data)

        // URLSession'ın kendi zaman aşımı SSE akışında güvenilir tetiklenmiyor
        // (bayt gelmeye devam ettikçe sayaç sıfırlanır); üst sınırı biz koyarız.
        let reply: JSONValue
        do {
            reply = try await zamanSinirli(Self.zamanAsimi) { [self] in
                try await self.akisOku(request: request, identity: identity)
            }
        } catch is SessionDropped {
            // Sunucu yeniden başlamış ya da oturumun süresi dolmuş. Yerel durumu
            // sıfırlamazsak ölü kimlik her istekte tekrar gönderilir ve bağlantı
            // uygulama yeniden açılana dek HER çağrıda düşer.
            oturumKimligi = nil
            elSikisildi = false
            guard !yenidenDenendi, Self.yenidenDenenebilir.contains(metot) else {
                throw MCPError.server("HTTP 404")
            }
            // El sıkışma yeniden kurulur; `initialize`ın kendisi için gereksiz —
            // çağrının kendisi zaten o.
            if metot != "initialize" { try await elSikis() }
            return try await invoke(metot: metot, parametre: parametre,
                                   yenidenDenendi: true)
        }

        if let error = reply["error"] {
            let message = error["message"]?.metinMi ?? ""
            throw MCPError.server(message)
        }
        guard let outcome = reply["result"] else { throw MCPError.bicimsiz }
        return outcome
    }

    /// İsteği gönderir, gövdeyi biçimine göre okur ve bizim `id`li yanıtı döndürür.
    private func akisOku(request: URLRequest, identity: Int) async throws -> JSONValue {
        let baytlar: URLSession.AsyncBytes
        let reply: URLResponse
        do {
            (baytlar, reply) = try await session.bytes(for: request)
        } catch {
            throw Self.hataCevir(error)
        }

        guard let http = reply as? HTTPURLResponse else { throw MCPError.bicimsiz }
        // Oturum kimliği el sıkışmada gelir; sonraki isteklerde taşınır.
        if let kimlikBasligi = http.value(forHTTPHeaderField: "Mcp-Session-Id"), !kimlikBasligi.isEmpty {
            oturumKimligi = kimlikBasligi
        }
        switch http.statusCode {
        case 200..<300: break
        case 401, 403:  throw MCPError.yetki
        // Spec: sunucu tanımadığı oturum kimliği için 404 döner ve istemciden
        // yeniden `initialize` bekler. Yanlış adres de 404 verir; o durumda
        // toparlanma bir kez daha deneyip aynı hataya düşer (bir fazladan
        // istek), kullanıcının gördüğü sonuç değişmez.
        case 404:       throw SessionDropped()
        default:        throw MCPError.server("HTTP \(http.statusCode)")
        }

        let kind = (http.value(forHTTPHeaderField: "Content-Type") ?? "").lowercased()

        if kind.contains("text/event-stream") {
            return try await sseOku(baytlar, identity: identity)
        }

        // Düz JSON: satırları toplayıp tek gövde olarak çöz.
        var body = Data()
        do {
            for try await byte in baytlar {
                try Task.checkCancellation()
                body.append(byte)
            }
        } catch {
            throw Self.hataCevir(error)
        }
        guard let resolve = try? JSONDecoder().decode(JSONValue.self, from: body) else {
            throw MCPError.bicimsiz
        }
        // Sunucu toplu (batch) dizi döndürmüş olabilir — bizim id'yi seç.
        if let array = resolve.diziMi {
            guard let benim = array.first(where: { Self.kimlikEsit($0, identity) }) else {
                throw MCPError.bicimsiz
            }
            return benim
        }
        return resolve
    }

    /// SSE ayrıştırması: `data:` satırları biriktirilir, BOŞ SATIRDA olay tamamlanır.
    /// Bizim `id`mize ait yanıt gelene kadar okumaya devam eder (sunucu araya
    /// ilerleme/log olayları koyabilir).
    private func sseOku(_ baytlar: URLSession.AsyncBytes, identity: Int) async throws -> JSONValue {
        var buffer: [String] = []

        func olayiCoz() -> JSONValue? {
            guard !buffer.isEmpty else { return nil }
            let gövde = buffer.joined(separator: "\n")
            buffer.removeAll()
            guard let data = gövde.data(using: .utf8),
                  let resolve = try? JSONDecoder().decode(JSONValue.self, from: data) else { return nil }
            if let array = resolve.diziMi {
                return array.first(where: { Self.kimlikEsit($0, identity) })
            }
            return Self.kimlikEsit(resolve, identity) ? resolve : nil
        }

        do {
            for try await line in baytlar.lines {
                try Task.checkCancellation()
                if line.isEmpty {
                    if let event = olayiCoz() { return event }
                    continue
                }
                if line.hasPrefix(":") { continue }        // yorum/heartbeat
                guard line.hasPrefix("data:") else { continue } // event:/id:/retry: ilgilendirmez
                var chunk = String(line.dropFirst(5))
                if chunk.hasPrefix(" ") { chunk.removeFirst() }
                buffer.append(chunk)
            }
        } catch {
            throw Self.hataCevir(error)
        }
        // Akış kapandı: son olay boş satırla kapanmamış olabilir.
        if let event = olayiCoz() { return event }
        throw MCPError.bicimsiz
    }

    private nonisolated static func kimlikEsit(_ object: JSONValue, _ identity: Int) -> Bool {
        guard let field = object["id"] else { return false }
        switch field {
        case .number(let v): return Int(v) == identity
        case .text(let v): return Int(v) == identity
        default: return false
        }
    }

    /// URLError/CancellationError → düz dille ayrışan MCPHatasi (§3.1).
    private nonisolated static func hataCevir(_ error: Error) -> MCPError {
        if error is CancellationError { return .cancelled }
        if let mcp = error as? MCPError { return mcp }
        guard let u = error as? URLError else { return .unreachable }
        switch u.code {
        case .timedOut:
            return .zamanAsimi
        case .cancelled:
            return .cancelled
        case .secureConnectionFailed, .serverCertificateHasBadDate,
             .serverCertificateUntrusted, .serverCertificateHasUnknownRoot,
             .serverCertificateNotYetValid, .clientCertificateRejected,
             .clientCertificateRequired, .appTransportSecurityRequiresSecureConnection:
            return .tls
        case .userAuthenticationRequired:
            return .yetki
        default:
            return .unreachable
        }
    }
}

// MARK: - Süre sınırı

/// İşi verilen süre içinde bitirir; bitmezse `zamanAsimi` fırlatır ve işi iptal eder.
/// Dışarıdan gelen iptal (uygulama arka plana gitti) aynen aşağı geçer.
///
/// `nonisolated` — varsayılan MainActor izolasyonu bu yardımcıyı arayüz kuyruğuna
/// bağlardı; 120 saniyelik bir ağ beklemesinin orada işi yok.
private nonisolated func zamanSinirli<T: Sendable>(_ second: TimeInterval,
                                       _ is: @escaping @Sendable () async throws -> T) async throws -> T {
    try await withThrowingTaskGroup(of: T.self) { grup in
        grup.addTask { try await `is`() }
        grup.addTask {
            try await Task.sleep(nanoseconds: UInt64(second * 1_000_000_000))
            throw MCPClient.MCPError.zamanAsimi
        }
        guard let first = try await grup.next() else { throw MCPClient.MCPError.bicimsiz }
        grup.cancelAll()
        return first
    }
}
