import Foundation
import FoundationModels
import CoreSpotlight

// SearchNotesTool — keyword search over the device's local Spotlight index.
// Local RAG, read-only. No network, needs no authorization.
struct SearchNotesTool: TacetTool {
    // The name the model sees is DELIBERATELY not plain "search" (web-search-spec §8.2 had
    // foreseen this clash). Measured bug: when the user said "search the internet" the model
    // saw a tool called `search` and called it, Spotlight looked at notes on the device, and
    // the answer became "I couldn't find it on your device" — while the user had asked for
    // the web. The description DID SAY "don't use this for general knowledge" and was ignored:
    // the tool name is a stronger signal than the description. The name now says what it searches.
    // The Swift type name (`SearchNotesTool`) and the profile key do not change.
    let name = "search_notes"
    // ONE text handles both situations (web-search §3.4). It does NOT CHANGE with the profile
    // composition: if a search server is configured then `web_search` is in the session and the
    // model calls it; if not, the tool is not there at all and the second half of the sentence
    // carries today's honest answer ("there is no such info on your device") through unchanged.
    // Varying the description by profile would have made it impossible to measure two different
    // behaviors of the same tool.
    let description = "Searches the user's OWN notes/files on the device (local Spotlight) by keyword. Only for personal-content requests like 'search my notes', 'find that note', in any language. Do NOT use for weather, general/world knowledge, or definitions — for those use the 'web_search' tool if it is available; otherwise say there is no such info on the device."
    weak var reporter: (any ToolReporter)?

    @Generable struct Arguments {
        @Guide(description: "Keyword to search for, e.g. 'meeting notes'.")
        var keyword: String
    }

    func call(arguments: Arguments) async -> String {
        await runWithChip(icon: "magnifyingglass",
                          runningText: L10n.searchingNotes,
                          rawInput: arguments.keyword) {
            let titles = await Self.search(arguments.keyword)

            // An empty result is not an error. Let the model honestly say it found nothing.
            // A search WAS still PERFORMED over the user's own content — the session is
            // tainted (mcp §5.6); the searched word is personal data too.
            if titles.isEmpty {
                return await taintIfSucceeded(
                    ToolOutcome(chipText: L10n.notesSearchedNone,
                                state: .readOk,
                                toModel: "no_results_found on device"))
            }

            let list = titles.enumerated()
                .map { "\($0.offset + 1). \($0.element)" }
                .joined(separator: "\n")

            return await taintIfSucceeded(ToolOutcome(
                chipText: L10n.notesSearched(titles.count),
                state: .readOk,
                toModel: "found \(titles.count) results: " + titles.joined(separator: ", "),
                rawOutput: list
            ))
        }
    }

    /// How many titles are collected at most — the context budget (spec §7.2).
    private static let cap = 10

    /// The Spotlight budget (seconds). A stuck `CSSearchQuery` can end up never calling its
    /// `completionHandler`; in that case the continuation waited FOREVER and the turn locked
    /// up. As with every other external component, the time here is bounded; when it runs out
    /// the titles collected so far are returned (a partial result, not a silent lock-up).
    private static let timeout: TimeInterval = 3

    // Runs the Spotlight query through an async bridge; collects at most 10 titles.
    // On an error it returns an empty list (the chip does not become .failed).
    private static func search(_ keyword: String) async -> [String] {
        // Characters that could break the query string: `"` closes the string early, and a
        // trailing `\` escapes the closing quote and makes the query invalid.
        let cleaned = keyword
            .replacingOccurrences(of: "\"", with: "")
            .replacingOccurrences(of: "\\", with: "")
        guard !cleaned.trimmingCharacters(in: .whitespaces).isEmpty else { return [] }

        let query = "title == \"*\(cleaned)*\"cd || textContent == \"*\(cleaned)*\"cd"

        return await withCheckedContinuation { continuation in
            let box = QueryBox(continuation: continuation, cap: cap)

            let context = CSSearchQueryContext()
            context.fetchAttributes = ["title", "displayName"]

            let searchQuery = CSSearchQuery(queryString: query, queryContext: context)
            box.bindQuery(searchQuery)

            // Results arrive in chunks; once the cap is reached the query is cancelled —
            // otherwise the rest would keep running for nothing.
            searchQuery.foundItemsHandler = { items in box.add(items) }
            searchQuery.completionHandler = { _ in box.finish() }

            searchQuery.start()

            DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + timeout) {
                box.finish()
            }
        }
    }
}

/// A locked box that guarantees the continuation is resumed EXACTLY ONCE.
///
/// Three paths race: the result cap filling up, the query finishing on its own, and the
/// timeout. Resuming a `CheckedContinuation` twice CRASHES the app; the flag is read and
/// written under the lock, the first arrival wins, later ones drop silently.
/// The query is cancelled at the same time — no query keeps running behind a finished turn.
private final class QueryBox: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<[String], Never>?
    private var titles: [String] = []
    private var query: CSSearchQuery?
    private let cap: Int

    init(continuation: CheckedContinuation<[String], Never>, cap: Int) {
        self.continuation = continuation
        self.cap = cap
    }

    func bindQuery(_ q: CSSearchQuery) {
        lock.lock(); defer { lock.unlock() }
        query = q
    }

    func add(_ items: [CSSearchableItem]) {
        lock.lock()
        for item in items where titles.count < cap {
            titles.append(item.attributeSet.title
                          ?? item.attributeSet.displayName
                          ?? item.uniqueIdentifier)
        }
        let isFull = titles.count >= cap
        lock.unlock()
        if isFull { finish() }
    }

    func finish() {
        lock.lock()
        let c = continuation
        let result = Array(titles.prefix(cap))
        let q = query
        if c != nil { continuation = nil; query = nil }
        lock.unlock()

        guard let c else { return }
        q?.cancel()
        c.resume(returning: result)
    }
}
