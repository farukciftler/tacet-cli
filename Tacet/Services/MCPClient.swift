//
//  MCPClient.swift
//  Tacet
//
//  The ONLY network code in the app (mcp-connection-spec §5.1, §2.1). No other
//  layer touches URLSession. If no connection has been added, this is never called
//  and the app's network traffic is zero.
//
//  HAND-WRITTEN — the official `modelcontextprotocol/swift-sdk` WAS NOT USED (a
//  deliberate deviation from spec §5.2). The project has zero SPM packages; zero
//  dependencies is the product's identity (even the OOXML zip is pure Swift). What
//  v1 needs is three methods: initialize, tools/list, tools/call. Transport:
//  Streamable HTTP — JSON-RPC 2.0 bodies go over HTTP POST, and the response can be
//  `application/json` or `text/event-stream` (SSE).
//
//  Timeout 120 s (§5.7): the remote side can do long work such as a build.
//  Cancellation: if the calling Task is cancelled (the app went to the background)
//  the request drops and `.cancelled` is returned — the chip becomes "stopped
//  partway", there is no silent disappearance.
//

import Foundation

// MARK: - JSON value

/// Schema-less JSON. The smallest representation needed, because MCP tool schemas and
/// arguments are not known at compile time. (There is no other JSON type in the code
/// base.)
///
/// `nonisolated` IS DELIBERATE: the project builds with
/// `SWIFT_DEFAULT_ACTOR_ISOLATION = MainActor`, so every type whose isolation is not
/// stated binds to MainActor by default. Binding a pure value type to the UI queue
/// makes no sense: `MCPClient` is an actor and decodes network responses off
/// MainActor, where touching members like `asText`/`asArray` would be an ERROR in the
/// Swift 6 language mode. The type is already `Sendable` — leaving isolation is safe.
nonisolated indirect enum JSONValue: Codable, Hashable, Sendable {
    case none
    case bool(Bool)
    case number(Double)
    case text(String)
    case array([JSONValue])
    case object([String: JSONValue])

    init(from decoder: Decoder) throws {
        let k = try decoder.singleValueContainer()
        if k.decodeNil() { self = .none }
        else if let v = try? k.decode(Bool.self) { self = .bool(v) }
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
        case .bool(let v):   try k.encode(v)
        case .number(let v):   try k.encode(v)
        case .text(let v):  try k.encode(v)
        case .array(let v):   try k.encode(v)
        case .object(let v):  try k.encode(v)
        }
    }

    // Navigation conveniences — MCP responses are read by hand.
    subscript(_ key: String) -> JSONValue? {
        if case .object(let s) = self { return s[key] }
        return nil
    }
    var asText: String? { if case .text(let v) = self { return v }; return nil }
    var asArray: [JSONValue]? { if case .array(let v) = self { return v }; return nil }
    var asBool: Bool? { if case .bool(let v) = self { return v }; return nil }

    /// The plain-text rendering — the argument text shown to the user on the approval
    /// sheet and the chip's raw input are produced from this.
    var plainText: String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        guard let data = try? encoder.encode(self),
              let s = String(data: data, encoding: .utf8) else { return "" }
        return s
    }

    /// Parses from JSON text (this is how MCPTool hands over the argument text the model
    /// produced).
    static func parse(_ text: String) -> JSONValue? {
        guard let data = text.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(JSONValue.self, from: data)
    }
}

// MARK: - Client

/// The client that talks to a single MCP server. One instance per connection; the
/// session id (`Mcp-Session-Id`) lives inside the instance.
actor MCPClient {

    /// The MCP version this client speaks. If the server proposes a different version,
    /// its choice is followed (MCP version negotiation).
    private static let protocolVersion = "2025-06-18"

    /// §5.7 — for long work such as a build. Applied both to URLSession and to the SSE
    /// read.
    static let timeout: TimeInterval = 120

    /// The upper bound of the `tools/list` pagination loop — a badly behaved server must
    /// not spin the app by returning an endless cursor.
    private static let maxPages = 20

    /// The tool-count cap: only 6–8 tools fit into the 4096 window anyway; there is no
    /// point loading thousands of tools into memory, and the import would drown in
    /// summarisation too.
    private static let maxTools = 200

    /// A single tool spec coming from the server.
    /// `nonisolated` — even though it is declared inside the actor it is pure data; it
    /// must travel freely both on the network side and in `ConnectionService` on
    /// MainActor.
    nonisolated struct ToolSpec: Sendable, Hashable {
        let name: String
        /// The raw description — it can be 100–500 tokens; it DOES NOT ENTER the context
        /// IN THIS FORM, `ConnectionService` summarises it (§5.3).
        let description: String
        /// `inputSchema` (JSON Schema). MCPTool converts it to a
        /// `DynamicGenerationSchema` at run time.
        let schema: JSONValue?
        /// MCP `annotations.readOnlyHint` — true if the server says "this tool changes
        /// nothing". nil if the server said nothing.
        let readOnlyHint: Bool?
        /// MCP `annotations.destructiveHint` — true if the server says "this tool is
        /// destructive". It IS A HINT, not a guarantee: the server may lie or may not
        /// report at all, which is why `MCPTool` also consults a name dictionary.
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

    /// The error paths separate in plain language (§3.1): the user is told the reason.
    /// `nonisolated` — thrown on the network side, displayed on MainActor.
    nonisolated enum MCPError: Error, Equatable, Sendable {
        case timedOut
        case unauthorized           // 401/403 — no key, or an invalid one
        case tls                    // certificate / TLS handshake
        case unreachable            // no network, server down, DNS
        case server(String)         // a JSON-RPC error or HTTP 4xx/5xx
        case malformed              // the response does not conform to MCP
        case cancelled              // the app went to the background / the user stopped

        /// The sentence shown to the user. It does not dramatise, it says what happened.
        var description: String {
            switch self {
            case .timedOut:     return String(localized: "timed out")
            case .unauthorized: return String(localized: "access key was rejected")
            case .tls:         return String(localized: "couldn’t establish a secure connection")
            case .unreachable: return String(localized: "couldn’t reach the server")
            case .server(let m):
                return m.isEmpty ? String(localized: "server returned an error")
                                 : String(localized: "server returned an error: \(m)")
            case .malformed:    return String(localized: "couldn’t make sense of the server’s response")
            case .cancelled:    return String(localized: "stopped partway")
            }
        }
    }

    /// The server did not recognise our `Mcp-Session-Id` (HTTP 404). A marker that lives
    /// only inside this file: `MCPError` is the contract shown to the user, whereas this
    /// is the transport layer's own self-recovery signal.
    private nonisolated struct SessionDropped: Error {}

    /// The methods whose CALL CAN BE REPEATED when the session drops. The criterion is the
    /// side effect: these two change nothing on the server, so running them a second time
    /// is harmless. `tools/call` is deliberately LEFT OUT — the request may have reached
    /// the server and left its side effect (the response may have been lost on the way),
    /// and sending it again would send the email twice. If the world changed, no retry.
    private static let retryable: Set<String> = ["initialize", "tools/list"]

    private let url: URL
    /// The bearer key — a copy taken from the Keychain; it is never written outside memory.
    private let key: String?
    private let session: URLSession

    /// The `Mcp-Session-Id` header from the `initialize` response; if present it is
    /// carried on every later request. It stays nil if the server keeps no session (valid
    /// behaviour).
    private var sessionID: String?
    /// The version negotiated with the server — it goes in the header after the handshake.
    private var negotiatedVersion: String = MCPClient.protocolVersion
    private var didHandshake = false
    private var counter = 0

    init(url: URL, key: String?) {
        self.url = url
        self.key = key
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = MCPClient.timeout
        config.timeoutIntervalForResource = MCPClient.timeout
        // No cookies/cache: our relationship with the remote server is the request, nothing more.
        config.httpCookieAcceptPolicy = .never
        config.httpShouldSetCookies = false
        config.requestCachePolicy = .reloadIgnoringLocalAndRemoteCacheData
        self.session = URLSession(configuration: config)
    }

    // MARK: - The three methods (v1 scope)

    /// The handshake. Done once; later calls do not touch it.
    func handshake() async throws {
        guard !didHandshake else { return }
        let outcome = try await invoke(method: "initialize", params: .object([
            "protocolVersion": .text(Self.protocolVersion),
            "capabilities": .object(["tools": .object([:])]),
            "clientInfo": .object(["name": .text("tacet"), "version": .text("1.0")]),
        ]))
        if let s = outcome["protocolVersion"]?.asText, !s.isEmpty { negotiatedVersion = s }
        didHandshake = true
        // The second half of the MCP handshake: the notification telling the server we are
        // ready (no response is expected). If we skip it, strict servers refuse tools/list
        // — it is not a separate "method", it is part of initialize.
        try? await notify(method: "notifications/initialized")
    }

    /// The server's tools. Pagination (`nextCursor`) is followed to the end.
    func tools() async throws -> [ToolSpec] {
        try await handshake()
        var total: [ToolSpec] = []
        var caret: String?
        var page = 0
        repeat {
            page += 1
            var params: [String: JSONValue] = [:]
            if let caret { params["cursor"] = .text(caret) }
            let outcome = try await invoke(method: "tools/list", params: .object(params))
            for item in outcome["tools"]?.asArray ?? [] {
                guard let name = item["name"]?.asText, !name.isEmpty else { continue }
                let annotations = item["annotations"]
                total.append(ToolSpec(name: name,
                                      description: item["description"]?.asText ?? "",
                                      schema: item["inputSchema"],
                                      readOnlyHint: annotations?["readOnlyHint"]?.asBool,
                                      destructiveHint: annotations?["destructiveHint"]?.asBool))
            }
            let next = outcome["nextCursor"]?.asText
            // A server repeating the same cursor must not put us in a loop.
            caret = (next == caret) ? nil : next
        } while caret != nil && page < Self.maxPages && total.count < Self.maxTools

        return Array(total.prefix(Self.maxTools))
    }

    /// Calls the tool and reduces the content to plain text.
    ///
    /// IMPORTANT: the approval gate is passed BEFORE this call, via
    /// `ToolExecutor.requestApprovalDecision`. Everything that arrives here is what the
    /// user saw and approved.
    /// - Returns: (text, serverError) — `isError` is the server's own error (the command
    ///   failed), not a transport error; the model reads it and reports it.
    func callTool(name: String, arguments: JSONValue) async throws -> (text: String, isError: Bool) {
        try await handshake()
        let outcome = try await invoke(method: "tools/call", params: .object([
            "name": .text(name),
            "arguments": arguments,
        ]))
        let parts: [String] = (outcome["content"]?.asArray ?? []).compactMap { item in
            if let t = item["text"]?.asText { return t }
            // Non-text content (image/audio) is not carried in v1; let us not swallow it
            // silently. THIS STRING GOES TO THE MODEL — it stays English.
            if let kind = item["type"]?.asText, kind != "text" {
                return "[\(kind) content — cannot be displayed]"
            }
            return nil
        }
        let text = parts.joined(separator: "\n")
        return (text, outcome["isError"]?.asBool ?? false)
    }

    // MARK: - The JSON-RPC transport

    private func nextIdentity() -> Int { counter += 1; return counter }

    private func buildRequest(body: Data) -> URLRequest {
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.timeoutInterval = Self.timeout
        request.httpBody = body
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        // We accept both formats; whichever the server picks is how we read it.
        request.setValue("application/json, text/event-stream", forHTTPHeaderField: "Accept")
        if didHandshake {
            request.setValue(negotiatedVersion, forHTTPHeaderField: "MCP-Protocol-Version")
        }
        if let sessionID {
            request.setValue(sessionID, forHTTPHeaderField: "Mcp-Session-Id")
        }
        if let key, !key.isEmpty {
            request.setValue("Bearer \(key)", forHTTPHeaderField: "Authorization")
        }
        return request
    }

    /// A notification expecting no response (JSON-RPC notification: no `id`).
    private func notify(method: String) async throws {
        let body: [String: JSONValue] = ["jsonrpc": .text("2.0"), "method": .text(method)]
        guard let data = try? JSONEncoder().encode(JSONValue.object(body)) else { return }
        _ = try? await session.data(for: buildRequest(body: data))
    }

    /// A single JSON-RPC call — returns the `result` object.
    ///
    /// - Parameter retried: a one-shot flag. `true` is passed on the session recovery
    ///   path; a second 404 is no longer recovered from and a plain error is returned. The
    ///   recursion therefore never goes on forever.
    private func invoke(method: String, params: JSONValue,
                        retried: Bool = false) async throws -> JSONValue {
        let identity = nextIdentity()
        let body: [String: JSONValue] = [
            "jsonrpc": .text("2.0"),
            "id": .number(Double(identity)),
            "method": .text(method),
            "params": params,
        ]
        guard let data = try? JSONEncoder().encode(JSONValue.object(body)) else {
            throw MCPError.malformed
        }
        let request = buildRequest(body: data)

        // URLSession's own timeout does not fire reliably on an SSE stream (the counter
        // resets as long as bytes keep arriving); we impose the upper bound ourselves.
        let reply: JSONValue
        do {
            reply = try await withTimeLimit(Self.timeout) { [self] in
                try await self.readStream(request: request, identity: identity)
            }
        } catch is SessionDropped {
            // The server restarted or the session expired. If we do not reset the local
            // state, the dead id is sent again on every request and the connection drops on
            // EVERY call until the app is reopened.
            sessionID = nil
            didHandshake = false
            guard !retried, Self.retryable.contains(method) else {
                throw MCPError.server("HTTP 404")
            }
            // The handshake is re-established; unnecessary for `initialize` itself — that
            // call already is the handshake.
            if method != "initialize" { try await handshake() }
            return try await invoke(method: method, params: params, retried: true)
        }

        if let error = reply["error"] {
            let message = error["message"]?.asText ?? ""
            throw MCPError.server(message)
        }
        guard let outcome = reply["result"] else { throw MCPError.malformed }
        return outcome
    }

    /// Sends the request, reads the body according to its format and returns the response
    /// carrying our `id`.
    private func readStream(request: URLRequest, identity: Int) async throws -> JSONValue {
        let bytes: URLSession.AsyncBytes
        let reply: URLResponse
        do {
            (bytes, reply) = try await session.bytes(for: request)
        } catch {
            throw Self.convertError(error)
        }

        guard let http = reply as? HTTPURLResponse else { throw MCPError.malformed }
        // The session id arrives with the handshake; it is carried on later requests.
        if let idHeader = http.value(forHTTPHeaderField: "Mcp-Session-Id"), !idHeader.isEmpty {
            sessionID = idHeader
        }
        switch http.statusCode {
        case 200..<300: break
        case 401, 403:  throw MCPError.unauthorized
        // Spec: the server returns 404 for a session id it does not recognise and expects
        // the client to `initialize` again. A wrong address also gives 404; in that case
        // the recovery tries once more and falls into the same error (one extra request),
        // and the outcome the user sees does not change.
        case 404:       throw SessionDropped()
        default:        throw MCPError.server("HTTP \(http.statusCode)")
        }

        let kind = (http.value(forHTTPHeaderField: "Content-Type") ?? "").lowercased()

        if kind.contains("text/event-stream") {
            return try await readSSE(bytes, identity: identity)
        }

        // Plain JSON: collect the lines and decode as a single body.
        var body = Data()
        do {
            for try await byte in bytes {
                try Task.checkCancellation()
                body.append(byte)
            }
        } catch {
            throw Self.convertError(error)
        }
        guard let decoded = try? JSONDecoder().decode(JSONValue.self, from: body) else {
            throw MCPError.malformed
        }
        // The server may have returned a batch array — pick out our id.
        if let array = decoded.asArray {
            guard let mine = array.first(where: { Self.identityMatches($0, identity) }) else {
                throw MCPError.malformed
            }
            return mine
        }
        return decoded
    }

    /// SSE parsing: `data:` lines are accumulated and the event completes ON AN EMPTY LINE.
    /// It keeps reading until the response carrying our `id` arrives (the server may
    /// interleave progress/log events).
    private func readSSE(_ bytes: URLSession.AsyncBytes, identity: Int) async throws -> JSONValue {
        var buffer: [String] = []

        func resolveEvent() -> JSONValue? {
            guard !buffer.isEmpty else { return nil }
            let body = buffer.joined(separator: "\n")
            buffer.removeAll()
            guard let data = body.data(using: .utf8),
                  let decoded = try? JSONDecoder().decode(JSONValue.self, from: data) else { return nil }
            if let array = decoded.asArray {
                return array.first(where: { Self.identityMatches($0, identity) })
            }
            return Self.identityMatches(decoded, identity) ? decoded : nil
        }

        do {
            for try await line in bytes.lines {
                try Task.checkCancellation()
                if line.isEmpty {
                    if let event = resolveEvent() { return event }
                    continue
                }
                if line.hasPrefix(":") { continue }        // comment/heartbeat
                guard line.hasPrefix("data:") else { continue } // event:/id:/retry: are of no interest
                var chunk = String(line.dropFirst(5))
                if chunk.hasPrefix(" ") { chunk.removeFirst() }
                buffer.append(chunk)
            }
        } catch {
            throw Self.convertError(error)
        }
        // The stream closed: the last event may not have been closed by an empty line.
        if let event = resolveEvent() { return event }
        throw MCPError.malformed
    }

    private nonisolated static func identityMatches(_ object: JSONValue, _ identity: Int) -> Bool {
        guard let field = object["id"] else { return false }
        switch field {
        case .number(let v): return Int(v) == identity
        case .text(let v): return Int(v) == identity
        default: return false
        }
    }

    /// URLError/CancellationError → an MCPError that separates in plain language (§3.1).
    private nonisolated static func convertError(_ error: Error) -> MCPError {
        if error is CancellationError { return .cancelled }
        if let mcp = error as? MCPError { return mcp }
        guard let u = error as? URLError else { return .unreachable }
        switch u.code {
        case .timedOut:
            return .timedOut
        case .cancelled:
            return .cancelled
        case .secureConnectionFailed, .serverCertificateHasBadDate,
             .serverCertificateUntrusted, .serverCertificateHasUnknownRoot,
             .serverCertificateNotYetValid, .clientCertificateRejected,
             .clientCertificateRequired, .appTransportSecurityRequiresSecureConnection:
            return .tls
        case .userAuthenticationRequired:
            return .unauthorized
        default:
            return .unreachable
        }
    }
}

// MARK: - Time limit

/// Finishes the work within the given time; if it does not finish it throws `timedOut`
/// and cancels the work. A cancellation coming from outside (the app went to the
/// background) passes straight down.
///
/// `nonisolated` — the default MainActor isolation would bind this helper to the UI
/// queue; a 120-second network wait has no business there.
private nonisolated func withTimeLimit<T: Sendable>(_ seconds: TimeInterval,
                                                    _ work: @escaping @Sendable () async throws -> T) async throws -> T {
    try await withThrowingTaskGroup(of: T.self) { group in
        group.addTask { try await work() }
        group.addTask {
            try await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
            throw MCPClient.MCPError.timedOut
        }
        guard let first = try await group.next() else { throw MCPClient.MCPError.malformed }
        group.cancelAll()
        return first
    }
}
