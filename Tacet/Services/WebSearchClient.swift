//
//  WebSearchClient.swift
//  Tacet
//
//  The ONLY network code in the app (web-search-spec §2.5; one until the MCP
//  client arrived, two after that). No other layer touches URLSession — that rule
//  is verified with a static scan in SelfTest (§8).
//
//  It makes a single GET to the user’s own SearXNG instance. The only data that
//  leaves is the search query. Parsing and the filters (5 results / 200 characters
//  / domain name) ARE PURE FUNCTIONS: testable without a network, with fixture
//  JSON (§6).

import Foundation
import NaturalLanguage

/// A single result row that goes to the model and to the chip.
///
/// `nonisolated`: a pure value type. Taken out of isolation so that parsing and
/// filtering can run off the main actor (see the `AnswerFilter` rationale).
nonisolated struct WebResult: Equatable, Identifiable, Sendable {
    var id: String { fullAddress.isEmpty ? title : fullAddress }

    var title: String
    /// The shortened address that goes to the model — the domain name only
    /// ("www.mgm.gov.tr").
    var domain: String
    /// The full URL — it stays in the chip detail and DOES NOT GO to the model (the
    /// risk of hallucinated links).
    var fullAddress: String
    /// The blurb, clipped at a word boundary at 200 characters.
    var summary: String
    /// True if it came from a SearXNG infobox — it stands first in the list.
    var isInfobox: Bool = false
}

/// Search errors. Because it is a `LocalizedError`, `TacetTool.shortError` writes them
/// into the chip as they are; the raw `NSURLErrorDomain` text never reaches the screen.
nonisolated enum WebSearchError: LocalizedError, Equatable, Sendable {
    /// No server configured or search is off — the network is never attempted.
    case noServer
    /// A network-layer error: timeout, address not found, connection dropped.
    case unreachable
    /// HTTP ≠ 200.
    case serverError(Int)
    /// The body is not JSON, or the expected fields are missing — if `formats: json` is
    /// off in SearXNG it typically returns HTML and lands here.
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

/// `nonisolated` IS DELIBERATE: parsing, clipping and text production are pure
/// functions and there is no reason for them to stay bound to the main actor (see
/// `AnswerFilter`). The members that touch the SETTING or the @Generable `Table` —
/// `search`, `searchPersistently`, `findAnswer`, `fetchPage`, `pickLanguage`, `table` —
/// are explicitly marked `@MainActor`; no caller’s contract changes.
nonisolated enum WebSearchClient {

    /// A search does not take long. MCP’s 120 s is for a build and is not carried
    /// over here (§5.3).
    static let timeout: TimeInterval = 15

    /// The cap on results that go to the model (the infobox included).
    static let resultCap = 5
    /// The blurb character cap per result.
    static let summaryCap = 200

    // MARK: - Shared sessions

    /// SHARED SESSIONS — no `URLSession` is produced per call.
    ///
    /// Every search and every page fetch used to build its own session, and none of
    /// them was `invalidate`d. A `URLSession` that is not invalidated does not release
    /// its delegate queue, its connection pool or its configuration copy; in a single
    /// search round (4 persistence attempts + 2 pages) more than six sessions were left
    /// behind. A session is an expensive object designed to be shared.
    ///
    /// THERE ARE TWO SEPARATE sessions because `timeoutIntervalForResource` is held at
    /// SESSION level and cannot be overridden per request: 15 s for search, 5 s for a
    /// page fetch (that split is a measured decision, see `AnswerFilter.pageTimeout`).
    static let searchSession = Self.makeSession(Self.timeout)
    static let pageSession = Self.makeSession(AnswerFilter.pageTimeout)

    /// The shared session configuration. `ephemeral` + `urlCache = nil`: the query and
    /// the page must leave no trace on disk — a search query can carry personal
    /// information (§2.2).
    private static func makeSession(_ duration: TimeInterval) -> URLSession {
        let setting = URLSessionConfiguration.ephemeral
        setting.timeoutIntervalForRequest = duration
        setting.timeoutIntervalForResource = duration
        setting.urlCache = nil
        setting.requestCachePolicy = .reloadIgnoringLocalCacheData
        return URLSession(configuration: setting)
    }

    // MARK: - Network

    /// Sends `GET /search?q=…&format=json` to the root address and returns the filtered
    /// results.
    /// - Parameter root: read from the setting if `nil`.
    ///
    /// `@MainActor`: `WebSearchSetting` reads the UserDefaults setting in the default
    /// isolation and belongs to another file. The network itself already flows inside
    /// `URLSession`, off the main actor; what is held here is only the setting read.
    @MainActor
    static func search(_ query: String, root: URL? = nil) async throws -> (results: [WebResult], requestURL: URL) {
        guard let rootURL = root ?? WebSearchSetting.rootURL else { throw WebSearchError.noServer }
        let language = pickLanguage(query: query)
        guard let url = requestURL(root: rootURL, query: query, language: language) else {
            throw WebSearchError.noServer
        }

        var request = URLRequest(url: url)
        request.timeoutInterval = timeout
        request.httpMethod = "GET"
        request.setValue("application/json", forHTTPHeaderField: "Accept")

        let data: Data
        let reply: URLResponse
        do {
            (data, reply) = try await searchSession.data(for: request)
        } catch {
            // The raw NSError never leaks out; a human sentence reaches the chip.
            throw WebSearchError.unreachable
        }

        if let http = reply as? HTTPURLResponse, http.statusCode != 200 {
            throw WebSearchError.serverError(http.statusCode)
        }

        return (try parse(data), url)
    }

    /// The request URL. Percent-encoding of the query is left to `URLComponents`.
    static func requestURL(root: URL, query: String, language: String?) -> URL? {
        let cleanQuery = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !cleanQuery.isEmpty else { return nil }

        // The root address may be "…/searxng/" or "…/searxng"; both must work.
        let base = root.appendingPathComponent("search")
        guard var chunk = URLComponents(url: base, resolvingAgainstBaseURL: false) else { return nil }

        var items = [
            URLQueryItem(name: "q", value: cleanQuery),
            URLQueryItem(name: "format", value: "json"),
            URLQueryItem(name: "safesearch", value: "1"),
        ]
        // If the language is unknown the parameter is NOT sent at all — the server’s
        // own default is better than forcing the wrong language (§5.3).
        if let language, !language.isEmpty {
            items.append(URLQueryItem(name: "language", value: language))
        }
        chunk.queryItems = items
        return chunk.url
    }

    /// The query language: the user’s explicit preference first, then a guess from the
    /// text, otherwise nil.
    @MainActor
    static func pickLanguage(query: String) -> String? {
        let preference = LanguagePreference.shared.replyLanguage
        if !preference.isEmpty { return preference }
        return guessLanguage(query: query)
    }

    /// An `NLLanguageRecognizer` guess — on device, no network. It returns nil for short
    /// or undecided queries (better than forcing the wrong language).
    static func guessLanguage(query: String) -> String? {
        let clean = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard clean.count >= 4 else { return nil }
        let recognizer = NLLanguageRecognizer()
        recognizer.processString(clean)
        guard let language = recognizer.dominantLanguage, language != .undetermined else { return nil }
        // The confidence threshold: a weak guess is not worth sending a parameter for.
        let confidence = recognizer.languageHypotheses(withMaximum: 1)[language] ?? 0
        guard confidence >= 0.5 else { return nil }
        return language.rawValue
    }

    // MARK: - Parsing + filters (pure; tested with fixtures)

    /// Converts the SearXNG JSON body into a list of `WebResult` and applies the
    /// app-layer filters. Model output/input is not trusted: the cap, the clipping and
    /// the domain reduction are enforced here, not left to the caller’s mercy.
    static func parse(_ data: Data) throws -> [WebResult] {
        guard let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw WebSearchError.formatNotUnderstood
        }

        var results: [WebResult] = []

        // If there is an infobox it comes first (§5.3).
        if let boxes = root["infoboxes"] as? [[String: Any]],
           let first = boxes.first {
            let content = (first["content"] as? String) ?? ""
            if !content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                let address = (first["urls"] as? [[String: Any]])?.first?["url"] as? String
                    ?? (first["id"] as? String) ?? ""
                results.append(WebResult(
                    title: (first["infobox"] as? String) ?? "",
                    domain: domainOf(address),
                    fullAddress: address,
                    summary: truncate(content),
                    isInfobox: true))
            }
        }

        // If `results` is absent this is a valid but empty response; it is not broken JSON.
        let raw = (root["results"] as? [[String: Any]]) ?? []
        for item in raw {
            if results.count >= resultCap { break }
            let title = ((item["title"] as? String) ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            let address = ((item["url"] as? String) ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            let content = (item["content"] as? String) ?? ""
            guard !title.isEmpty || !address.isEmpty else { continue }
            results.append(WebResult(
                title: title,
                domain: domainOf(address),
                fullAddress: address,
                summary: truncate(content)))
        }

        return Array(results.prefix(resultCap))
    }

    /// Clips the blurb at a word boundary. No word is split in the middle of the limit.
    static func truncate(_ text: String, limit: Int = summaryCap) -> String {
        let single = text
            .replacingOccurrences(of: "\n", with: " ")
            .replacingOccurrences(of: "\r", with: " ")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard single.count > limit else { return single }

        let slice = single.prefix(limit)
        if let space = slice.lastIndex(of: " "), space > slice.startIndex {
            let clipped = String(slice[slice.startIndex..<space])
                .trimmingCharacters(in: .whitespacesAndNewlines)
            if !clipped.isEmpty { return clipped + "…" }
        }
        return String(slice) + "…"
    }

    /// Reduces the address to its domain name. The full URL does not go to the model:
    /// the token budget and the hallucinated-link risk drop together (§5.3).
    static func domainOf(_ address: String) -> String {
        guard let url = URL(string: address), let host = url.host, !host.isEmpty else {
            return ""
        }
        return host
    }

    // MARK: - The text returned to the model (the 4096 bypass — §5.5)

    /// The content cap of a SINGLE LINE going to the model. The budget is enforced per
    /// line: 5 lines × (~180 + prefix) + the header line + the source rule ≈ 1120
    /// characters ≈ 280 tokens (§5.5). Clipping the title or the blurb separately is not
    /// enough — a long title + a long domain + a capped blurb together exceed the budget;
    /// the single gate is the line itself. The raw output (chip detail) stays unclipped.
    static let lineCap = 180

    /// The clipped list; the target budget is ≤ ~300 tokens. On zero results, a fixed
    /// `no_results`.
    static func modelText(query: String, results: [WebResult]) -> String {
        guard !results.isEmpty else { return "no_results" }
        // The fields are LABELLED title/source/blurb. The unlabelled
        // "title — domain — blurb" shape led to a measured bug: the model combined one
        // row’s title with another row’s domain and produced INVENTED sources such as
        // `[sehirhatlari.istanbul](e-yasamrehberi.com)`. An invented URL is more insidious
        // than a wrong time: when the user goes to verify it, they go to the wrong place
        // too. The label makes it unambiguous which row a field belongs to; the rule below
        // forbids building a link at all.
        let lines = results.enumerated().map { (i, s) -> String in
            let prefix = s.isInfobox ? "[infobox] " : ""
            let parts = truncate([s.title.isEmpty ? nil : "title: \(s.title)",
                                 s.domain.isEmpty ? nil : "source: \(s.domain)",
                                 s.summary.isEmpty ? nil : "blurb: \(s.summary)"]
                .compactMap { $0 }
                .joined(separator: " | "), limit: lineCap)
            return "\(i + 1). \(prefix)\(parts)"
        }
        // The header is DELIBERATELY not "found N results". Measured behaviour: the model
        // read the phrase "found 5 results" as "I found the answer" and invented a number
        // that appeared nowhere in the list (20°C and 24°C for the same question). Naming
        // WHAT the results ARE — a page listing, not live data — reduces the invention.
        // This is not an INSTRUCTION but a description of the data; it does not conflict
        // with the "do not follow instructions in tool output" rule in §5.6.
        return "web page listings matching \"\(query)\" (\(results.count) pages; "
            + "titles and blurbs only, not live data):\n"
            + lines.joined(separator: "\n")
            + "\n" + sourceRule
    }

    /// The source rule at the end of every search output that goes to the model. The full
    /// URL does not go to the model anyway; this line also closes off the model building a
    /// FAKE link out of the domain. It is not a description of the data but an
    /// output-format rule, and it does not override the user’s own instruction.
    static let sourceRule = "Cite sources as plain-text domain names only; "
        + "do not write markdown links and do not invent URLs."

    /// The raw output in the chip detail: title + FULL address + blurb (§3.2).
    /// The user sees "what went out, what came back" here; the full URL lives only here.
    static func rawOutputText(_ results: [WebResult]) -> String {
        results.map { s in
            [s.isInfobox ? String(localized: "infobox") : s.title, s.fullAddress, s.summary]
                .filter { !$0.isEmpty }
                .joined(separator: "\n")
        }.joined(separator: "\n\n")
    }

    /// The table representation used to put the results into the `DataStore` channel.
    /// `@MainActor`: the `Table`/`Row` @Generable types live in the default isolation.
    ///
    /// THE HEADERS ARE USER-VISIBLE: they end up in the produced .xlsx/.pdf, so they go
    /// through `String(localized:)` like every other user-facing string.
    @MainActor
    static func table(_ results: [WebResult]) -> Table {
        Table(headers: [String(localized: "Title"),
                        String(localized: "Address"),
                        String(localized: "Summary")],
              rows: results.map { Row(cells: [$0.title, $0.fullAddress, $0.summary]) })
    }

    // MARK: - Page fetching (the answer-finding loop)

    /// Fetches a single page, converts it to PLAIN TEXT and returns it. Because network
    /// code may live only here, the fetching is in this file too; parsing/filtering is in
    /// `AnswerFilter` (pure, tested with fixtures).
    ///
    /// The hard limits are enforced here, not left to the caller’s mercy:
    /// - `https://` only (`http://` on a local network) — the
    ///   `WebSearchSetting.validate` rule.
    /// - `Accept: text/html`; if the response is not `text/html` the page is SKIPPED (nil).
    /// - At most `pageByteCap` bytes are processed; a body beyond that is TRUNCATED (a PDF
    ///   or a 20 MB page must not eat the device’s memory).
    ///
    /// `@MainActor` only for the `WebSearchSetting.validate` setting read; the network
    /// runs inside `URLSession` and the HTML→text conversion on the global executor.
    @MainActor
    static func fetchPage(_ address: String) async -> String? {
        guard let url = WebSearchSetting.validate(address) else { return nil }

        var request = URLRequest(url: url)
        request.httpMethod = "GET"
        request.timeoutInterval = AnswerFilter.pageTimeout
        request.setValue("text/html", forHTTPHeaderField: "Accept")
        // The identity is MANDATORY. Measured bug: URLSession’s default
        // ("Tacet/1 CFNetwork/… Darwin/…") tripped bot filters and the page returned 403 —
        // the page carrying the ferry times came back empty for exactly this reason.
        // IMITATING a browser was tried and WAS NOT NEEDED: servers return 200 when they
        // see a plain name. We identify ourselves as what we are; looking like another
        // client is both unnecessary and against the product’s language.
        request.setValue("Tacet/1.0", forHTTPHeaderField: "User-Agent")

        do {
            // `data(for:)` instead of `bytes(for:)`: consuming the byte stream with
            // `for try await` put every single byte through a suspension point on the main
            // actor, and a 400 KB page locked the interface for the whole search.
            // `data(for:)` collects the body on URLSession’s own queue and returns it to
            // the main actor in one piece.
            let (raw, reply) = try await pageSession.data(for: request)
            if let http = reply as? HTTPURLResponse {
                guard http.statusCode == 200 else { return nil }
                let kind = (http.value(forHTTPHeaderField: "Content-Type") ?? "").lowercased()
                // An empty Content-Type is tolerated; another type stated explicitly is not.
                guard kind.isEmpty || kind.contains("text/html") || kind.contains("text/plain") else {
                    return nil
                }
                // If the declared size is many times the cap, do not process the page at all.
                let declared = http.expectedContentLength
                guard declared <= 0
                        || declared <= Int64(AnswerFilter.pageByteCap) * 8
                else { return nil }
            }

            // THE SIZE CAP: the body converted to text is still truncated at `pageByteCap`
            // as before. The only difference is that the truncation happens after the
            // download rather than during it — `data(for:)` takes the body into memory as
            // a whole. That is why, IF the server DECLARES a size and it is many times the
            // cap, the page is not processed at all (it is skipped, just as with a wrong
            // Content-Type): downloading a 20 MB PDF to use 400 KB of it helps nobody. If
            // no size is declared, the only brake is `timeoutIntervalForResource`.
            let body = raw.count > AnswerFilter.pageByteCap
                ? Data(raw.prefix(AnswerFilter.pageByteCap))
                : raw
            guard !body.isEmpty else { return nil }

            // HTML→TEXT DOES NOT RUN ON THE MAIN ACTOR. Entity resolution walks the whole
            // text dozens of times and line simplification once more; the measured effect
            // was a chip animation frozen for the duration of the search round. Because
            // `AnswerFilter` is pure and `nonisolated`, the work can move to the global
            // executor.
            return await Task.detached(priority: .userInitiated) {
                let html = String(data: body, encoding: .utf8)
                    ?? String(decoding: body, as: UTF8.self)
                let text = AnswerFilter.toText(html)
                return text.isEmpty ? nil : text
            }.value
        } catch {
            // One page failing does not fail the search: the loop continues with the next
            // candidate.
            return nil
        }
    }

    /// The outcome of a search: THE CODE decides what goes to the model.
    struct AnswerFinding: Sendable {
        var shape: SoughtShape
        var matches: [Match]
        /// The domain names of the pages fetched (for the chip detail and transparency).
        var fetched: [String]
        /// The plain text of the pages fetched — it goes to `DataStore`, NOT to the model.
        var fullText: String
        /// The narrowed query used if a second search round happened.
        var secondQuery: String?
        var isSufficient: Bool { matches.count >= AnswerFilter.sufficiencyThreshold }
        /// The overall freshness of the values — the worst match decides.
        var freshness: Freshness { AnswerFilter.overallFreshness(matches) }
    }

    // MARK: - Empty-response persistence

    /// AN EMPTY RESPONSE IS NOT AN ERROR — AND IT IS THE MOST FREQUENT FAULT.
    ///
    /// Measurement (a real server, the same query, 8 attempts at 10 s intervals): 7
    /// attempts returned HTTP 200 + `results: []`, the 8th returned 20 results. In a
    /// consecutive measurement at 1.5 s intervals the first 4 attempts were empty and
    /// everything after the 5th was full. The cause is not the server itself but
    /// SearXNG’s upstream engines rate-limiting temporarily.
    ///
    /// This is the FIRST cause of the user’s complaint that "web search works
    /// sometimes", and no improvement on the parsing side fixes it: there is no result in
    /// hand to parse. The only correct behaviour is to treat an empty response as
    /// temporary and RETRY AT SHORT INTERVALS.
    ///
    /// The attempt count and the wait are FIXED; there is no exponential back-off,
    /// because the measured recovery time (~7 s) is already caught by a fixed interval.
    static let emptyReplyAttemptCap = 4
    static let emptyReplyWait: TimeInterval = 1.5

    /// A search that persists on an empty response. It is bounded by the attempt count
    /// and by the SHARED DEADLINE — persistence is not an excuse for making the user wait
    /// indefinitely.
    ///
    /// - Returns: the results + the request URL + which attempt they arrived on
    ///   (transparency; it is visible in the chip detail, so the user knows how many
    ///   queries went to their server).
    @MainActor
    static func searchPersistently(_ query: String,
                           root: URL? = nil,
                           end: Date) async throws -> (results: [WebResult], requestURL: URL, attempt: Int) {
        var lastError: Error?
        var lastURL: URL?
        var made = 0
        for attempt in 1...emptyReplyAttemptCap {
            made = attempt
            do {
                let (results, url) = try await search(query, root: root)
                lastURL = url
                if !results.isEmpty { return (results, url, attempt) }
            } catch {
                // A network error is retried too; the persistence is not specific to an
                // empty response. But if there IS NO server, persisting is meaningless —
                // leave at once.
                if case WebSearchError.noServer = error { throw error }
                lastError = error
            }
            // Do not wait after the last attempt; and do not wait at all if it would
            // exceed the deadline.
            guard attempt < emptyReplyAttemptCap,
                  Date().addingTimeInterval(emptyReplyWait) < end
            else { break }
            try? await Task.sleep(nanoseconds: UInt64(emptyReplyWait * 1_000_000_000))
        }
        // The number of attempts ACTUALLY MADE is returned, not the cap: the number in the
        // chip detail is the count of requests that went to the user’s server, not a guess.
        if let lastURL { return ([], lastURL, made) }
        throw lastError ?? WebSearchError.unreachable
    }

    /// The second search round is entered only BEFORE this much time has passed.
    /// Measured: a SearXNG search alone takes ~3.5 s, and after the second round pages
    /// still have to be fetched. A second round starting after 8 s overflows the 15 s
    /// budget; a round left half-done is worse than a round never started (the user waits
    /// and gets nothing in return).
    static let secondRoundThreshold: TimeInterval = 8

    /// The answer-finding loop (at most 1 extra search round, at most 2 page fetches):
    /// 1. Scan the blurbs by shape; if the threshold is met, STOP.
    /// 2. If not enough, fetch the best 2 pages and scan the fetched text.
    /// 3. If it is still absent, honestly return not-found — NO content is given to the
    ///    model.
    ///
    /// If the shape is `.none` it does not run at all: for a free-text question the
    /// existing blurb-list behaviour is preserved.
    /// - Parameter advance: called BEFORE every page attempt with that domain name; the
    ///   tool writes it into the chip text. Empty by default, so tests and chip-less calls
    ///   are not forced to report progress.
    /// - Parameter end: the SHARED DEADLINE. The search persistence, the page fetching and
    ///   the second round share one budget. Had they been given separate budgets, on a bad
    ///   day the total time would be the SUM of the budgets (the measured risk: 7 s of
    ///   persistence + 15 s of fetching = 22 s). The promise given to the user is a single
    ///   duration.
    @MainActor
    static func findAnswer(query: String,
                         results: [WebResult],
                         root: URL? = nil,
                         today: Date = Date(),
                         end: Date? = nil,
                         advance: @Sendable (String) async -> Void = { _ in }) async -> AnswerFinding? {
        let shape = AnswerFilter.findShape(query)
        guard shape != .none else { return nil }

        let start = Date()
        let deadline = end ?? start.addingTimeInterval(AnswerFilter.totalBudget)
        var matches: [Match] = []
        var fetched: [String] = []
        var fullText = ""
        var succeeded = 0

        /// Scan the blurbs. Free — no network. Blurb text is UNDATED, so matches coming
        /// from here carry the `.unknown` stamp: the value is correct but its freshness is
        /// NOT VERIFIED.
        func scanSummaries(_ list: [WebResult]) {
            for s in list {
                matches += AnswerFilter.match("\(s.title)\n\(s.summary)", shape: shape,
                                                    source: s.domain.isEmpty ? "—" : s.domain,
                                                    freshness: .unknown)
            }
            matches = dedupe(matches, shape: shape)
        }

        /// Is the threshold met — and if it is, IS this answer GOOD ENOUGH?
        ///
        /// For a time-dependent shape (ferry times, exchange rates, weather), settling for
        /// values gathered only from the search BLURBS led to two measured bugs:
        ///  1. A blurb is undated; in the prayer-times query the 03:49 coming from the
        ///     blurbs was the winter timetable and, even with the "not verified" stamp, it
        ///     was the ONLY answer — while the page carried today’s timetable.
        ///  2. A blurb carries only a few values; in the ferry timetable query 3 times came
        ///     out of the blurbs, the threshold was met, no page was fetched at all, and
        ///     the user saw 3 of a 25-sailing timetable. An incomplete timetable is a wrong
        ///     timetable.
        ///
        /// So for time-dependent shapes page fetching continues even when the threshold is
        /// met; the goal is to obtain a set of VERIFIED values.
        func isSatisfied() -> Bool {
            guard matches.count >= AnswerFilter.sufficiencyThreshold else { return false }
            guard shape.isTimeDependent else { return true }
            // If it is time-dependent, we settle only when there are enough verified values.
            return matches.filter { $0.freshness == .verified }.count
                >= AnswerFilter.sufficiencyThreshold
        }

        /// Fetch and scan the candidates in order. Only SUCCESSFUL fetches spend the cap.
        ///
        /// Measured bug: in the "ferry times" query the 1st result (the official site)
        /// returned HTTP 500, the 2nd carried no data, and the page carrying the times was
        /// 3rd. Because the cap counted attempts, the budget was spent on dead pages and
        /// "not found" was returned while the data was right there. A dead page is free.
        func fetchPages(_ list: [WebResult]) async {
            for candidate in AnswerFilter.candidatesToFetch(list, shape: shape) {
                guard succeeded < AnswerFilter.pageCap else { break }
                guard Date() < deadline else { break }
                let field = candidate.domain.isEmpty ? "—" : candidate.domain
                guard !fetched.contains(field) else { continue }
                // Which site is being looked at is reported BEFORE the fetch: instead of a
                // spinner that looks like nothing is happening for 15 s, the user sees the
                // domain names being tried in order. The name shown is the address actually
                // being downloaded at that moment — not text produced by the model.
                await advance(field)
                guard let text = await fetchPage(candidate.fullAddress), !text.isEmpty else { continue }
                succeeded += 1
                fetched.append(field)
                fullText += (fullText.isEmpty ? "" : "\n\n") + "— \(field) —\n" + text
                // FRESHNESS IS MEASURED HERE: does today’s date appear on the page? If it
                // does not, the value is still taken but stamped; it goes to the model with
                // a warning. Presenting it silently as current was the most insidious bug
                // measured.
                //
                // The scan IS NOT DONE on the main actor: `match` runs the regex engine on
                // every line of the page and page text can be hundreds of thousands of
                // characters. `scanPage` returns the freshness stamp and the matches in a
                // single pass — one executor hop is enough.
                let scan = await Task.detached(priority: .userInitiated) {
                    AnswerFilter.scanPage(text, shape: shape, source: field, today: today)
                }.value
                matches += scan.matches
                matches = dedupe(matches, shape: shape)
                if isSatisfied() { break }
            }
        }

        // --- ROUND 1 ---
        scanSummaries(results)
        if isSatisfied() {
            return AnswerFinding(shape: shape, matches: AnswerFilter.preferFresh(matches),
                                fetched: [], fullText: "", secondQuery: nil)
        }
        await fetchPages(results)
        if isSatisfied() {
            return AnswerFinding(shape: shape, matches: AnswerFilter.preferFresh(matches),
                                fetched: fetched, fullText: fullText, secondQuery: nil)
        }

        // --- ROUND 2 (only if time remains) ---
        //
        // The first round did not find the shape. The query is narrowed IN CODE and
        // searched once more. We do not have the MODEL rewrite the query: in this project
        // the model produced irrelevant queries and burned the budget; the narrowing is
        // fixed and read in one place, in `narrowedQuery`.
        //
        // The NUMBER of rounds is capped at two and the second is gated on TIME. Unlimited
        // rounds would be a "deep research loop" and spec §7 deliberately puts that out of
        // scope.
        // The second round depends on the remaining time being MEANINGFUL: if a new search
        // + at least one page fetch does not fit, it never starts.
        guard Date().addingTimeInterval(secondRoundThreshold) < deadline,
              let narrowed = AnswerFilter.narrowedQuery(query, shape: shape, today: today),
              let secondResults = try? await searchPersistently(narrowed, root: root, end: deadline).results,
              !secondResults.isEmpty
        else {
            return AnswerFinding(shape: shape, matches: AnswerFilter.preferFresh(matches),
                                fetched: fetched, fullText: fullText, secondQuery: nil)
        }

        scanSummaries(secondResults)
        if !isSatisfied() { await fetchPages(secondResults) }

        return AnswerFinding(shape: shape, matches: AnswerFilter.preferFresh(matches),
                            fetched: fetched, fullText: fullText,
                            secondQuery: narrowed)
    }

    /// Deduplicates across sources too; it does not disturb the order and enforces the cap.
    static func dedupe(_ matches: [Match], shape: SoughtShape) -> [Match] {
        var seen = Set<String>()
        var outcome: [Match] = []
        for e in matches {
            let key = AnswerFilter.normalizeValue(e.value, shape: shape)
            guard !seen.contains(key) else { continue }
            seen.insert(key)
            outcome.append(e)
            if outcome.count >= AnswerFilter.matchCap { break }
        }
        return outcome
    }
}
