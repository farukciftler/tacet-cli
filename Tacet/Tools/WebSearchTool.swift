//
//  WebSearchTool.swift
//  Tacet
//
//  The web search tool (web-search-spec §5.2). Sends a single query to the user's OWN search
//  server. The only data that leaves is the query, and the query is always in the chip text
//  (§2.2) — the user sees what went out without having to look for it.
//
//  In a tainted session (if a personal-data tool ran earlier) the query goes through the
//  shared approval gate: `ToolExecutor.requestApprovalDecision`. The gate is in the code, not in the model.
//
//  The network code is NOT here, it is in `WebSearchClient`; this tool only manages the chip
//  lifecycle, the approval gate and the 4096 bypass channel (DataStore).
//

import Foundation
import FoundationModels

struct WebSearchTool: TacetTool {
    let name = "web_search"
    let description = "Searches the web via the user's own search server and, when the question asks for a concrete value (times, prices, temperatures, dates), extracts that value from the pages. Use for weather, news, prices, timetables, current events, and general/world knowledge the device cannot know. NOT for the user's personal notes/files. Cite sources as plain-text domain names only; never write markdown links or full URLs."

    weak var reporter: (any ToolReporter)?
    /// For the approval gate — `requestApprovalDecision`, which is not part of the `ToolReporter`
    /// protocol, is called from here. `ModelService` wires it during tool setup.
    weak var executor: ToolExecutor?
    /// The bulk-data channel: all results into the store, a short list to the model.
    weak var dataStore: DataStore?

    @Generable struct Arguments {
        @Guide(description: "Short web search query in the user's language, e.g. 'istanbul weather tomorrow'.")
        var query: String
    }

    func call(arguments: Arguments) async -> String {
        let query = arguments.query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else {
            return "error: empty_query. Call the tool again with a short search query."
        }

        // If the server is undefined/off the tool should not have entered the profile at all;
        // still, stop defensively — do not touch the network layer.
        guard WebSearchSetting.isActive else {
            return "search_unavailable: no search server is configured; the answer is not available on this device"
        }

        // THE APPROVAL GATE — BEFORE the chip. In a clean session it passes without asking
        // (§2.4); in a tainted session the user sees exactly the query that is going out (§3.3).
        if let executor {
            let decision = await executor.requestApprovalDecision(source: L10n.searchServer,
                                                                  toolName: name,
                                                                  content: query)
            // The gate already dropped the refusal chip; no second chip is produced here.
            // Refusal and "could never be asked" MUST be separate sentences: in the second
            // case the user made no decision, and telling the model otherwise would be a lie.
            switch decision {
            case .accepted:
                break
            case .denied:
                return "user_declined_search: the user chose not to send this query; the search was not sent"
            case .busy, .cancelled:
                return decision.toModel ?? ""
            }
        }

        return await runWithChip(icon: "globe",
                                 runningText: L10n.searching(query),
                                 rawInput: query) { advance in
            // A SHARED DEADLINE: the search persistence + page fetching + the second pass all
            // share this one budget (see `findAnswer(end:)`).
            let end = Date().addingTimeInterval(AnswerFilter.totalBudget)

            // PERSISTING THROUGH EMPTY RESPONSES. Measured: this server returns HTTP 200 + an
            // empty list 7 times in a row for the same query and 20 results on the 8th (an
            // upstream engine restriction). Giving up after a single attempt was the number
            // one reason search looked like it "sometimes works".
            let (results, requestURL, attempt) = try await WebSearchClient.searchPersistently(
                query, end: end)

            // The chip detail: what went out (the full request URL) + what came back
            // (title/address/summary). The number of attempts is written down too: the number
            // of requests going to the user's own server is not hidden (§2.2 transparency).
            let attemptNote = attempt > 1 ? " (\(attempt) attempts)" : ""
            let raw = "GET \(requestURL.absoluteString)\(attemptNote)\n\n"
                + (results.isEmpty ? "—" : WebSearchClient.rawOutputText(results))

            guard !results.isEmpty else {
                return ToolOutcome(chipText: L10n.searched(0),
                                   state: .readOk,
                                   toModel: "no_results",
                                   rawOutput: raw)
            }

            // THE ANSWER-FINDING LOOP. If the query asks for a concrete value (a time, a price,
            // a temperature, a date) a list of links is not enough: the value ITSELF is
            // fetched. The sufficiency decision is made by code (regex), not by the model —
            // everywhere it was left to the model's judgement it produced a fabricated value.
            if let finding = await WebSearchClient.findAnswer(
                query: query, results: results, end: end,
                advance: { domain in await advance(L10n.reading(domain)) }) {
                let fetchNote = finding.fetched.isEmpty
                    ? ""
                    : "\n\nPAGES READ: " + finding.fetched.joined(separator: ", ")
                // If a second pass happened, THE SECOND QUERY THAT WENT OUT also appears in the
                // chip detail. The promise "the only data that leaves is the query, and the
                // query is always visible" (spec §2.2) holds for the query the code produced too.
                let secondNote = finding.secondQuery.map { "\n\nSECOND SEARCH: \($0)" } ?? ""
                let wideRaw = raw + secondNote + fetchNote
                    + (finding.fullText.isEmpty ? "" : "\n\n" + finding.fullText)

                guard finding.isSufficient else {
                    // Below the threshold: NO CONTENT IS GIVEN to the model. Sending the page
                    // text on the chance that "maybe the model finds something" is an
                    // invitation to fabricate.
                    return ToolOutcome(chipText: L10n.answerNotFound,
                                       state: .readOk,
                                       toModel: AnswerFilter.notFoundText,
                                       rawOutput: wideRaw)
                }

                var toModel = AnswerFilter.modelText(query: query,
                                                     shape: finding.shape,
                                                     matches: finding.matches,
                                                     freshness: finding.freshness)
                // Regular matches become a table; the model does not write the table itself.
                if let dataStore,
                   let table = AnswerFilter.table(finding.matches, shape: finding.shape) {
                    let ref = dataStore.put(table, tag: "search")
                    toModel += "\n(data_ref=\(ref))"
                }
                // The chip has to be honest too: giving the warning only to the model and
                // telling the user "Found · 6 values" would mean trusting the model to relay
                // the warning. In this project everything left to the model's judgement failed;
                // the second channel that reaches the user comes from the code.
                let chipText = finding.freshness == .verified
                    ? L10n.answerFound(finding.matches.count)
                    : L10n.answerFoundFreshnessDoubt(finding.matches.count)
                return ToolOutcome(chipText: chipText,
                                   state: .readOk,
                                   toModel: toModel,
                                   rawOutput: wideRaw)
            }

            // The shape could not be determined (a free-text question) → the existing behavior:
            // the 4096 bypass, the full list into the store, a truncated summary (≤ ~300 tokens)
            // to the model.
            var toModel = WebSearchClient.modelText(query: query, results: results)
            if let dataStore {
                let ref = dataStore.put(WebSearchClient.table(results), tag: "search")
                toModel += "\n(data_ref=\(ref))"
            }

            return ToolOutcome(chipText: L10n.searched(results.count),
                               state: .readOk,
                               toModel: toModel,
                               rawOutput: raw)
        }
    }
}

// Chip texts. Because `L10n` is another agent's file, it is extended here — the single
// localization point is preserved, and so is the file boundary.
extension L10n {
    /// The query is in the chip text (§2.2) — not a category summary, the exact text going out.
    static func searching(_ query: String) -> String { String(localized: "Searching · \(query)") }
    static func searched(_ n: Int) -> String { String(localized: "Searched · \(n) results") }
    /// The live chip text while a page is being fetched. No fancy verb, the domain name itself.
    static func reading(_ domain: String) -> String { String(localized: "Reading · \(domain)") }
    /// When a shape was sought and found: the number of VALUES is shown, not the number of results.
    static func answerFound(_ n: Int) -> String { String(localized: "Found · \(n) values") }
    /// A value was found but today's date is not visible on the page. The user sees this in the
    /// chip without waiting for the model to explain it — no dramatization, plain information.
    static func answerFoundFreshnessDoubt(_ n: Int) -> String {
        String(localized: "Found · \(n) values · recency unverified")
    }
    /// A shape was sought but it stayed below the threshold — an honest chip, no silent success.
    static var answerNotFound: String { String(localized: "Requested information not found") }
    static var searchUnreachable: String { String(localized: "Search unreachable") }
    /// The source name shown at the approval gate and on the refusal chip.
    static var searchServer: String { String(localized: "search server") }
}
