//
//  EvalReport.swift
//  Tacet
//
//  The eval scoring + reporting layer. Evaluation.swift's binary "passed/failed"
//  verdict does not show WHY a case failed: a wrong tool choice and a made-up
//  fact are logged with the same ✗. Here every case gets a 0-100 score split
//  into dimensions, and the results are written out to Excel and to a markdown
//  summary.
//
//  Note: like Evaluation, this file DOES NOT USE NSLog/os_log — eval output can
//  contain real calendar/contact data and must not leak into the system log.
//

#if DEBUG
import Foundation

/// The scored outcome of a single case run.
/// `mode` separates the two run shapes: "single" — an independent session with
/// the conversation reset before every case; "chain" — sequential steps inside
/// one session. The user's actual question is the DIFFERENCE between those two,
/// so the mode is carried on every row.
/// `Codable`: the comprehensive run dumps its raw rows to JSON and the analysis
/// agents read that file rather than the Excel workbook.
///
/// THE MODE VALUES AND THE ISSUE TAGS BELOW ARE A WIRE FORMAT, not prose. They
/// are compared as strings in `summary`, in the eval corpus and in the
/// self-test; renaming one side alone splits the report in half.
struct EvalResult: Codable {
    let caseName: String
    let category: String
    let mode: String          // "single" | "chain"
    var stepNo: Int = 1      // which step inside a chain; always 1 when single
    let prompt: String
    var expectedChips: [String] = []
    var actualChips: [String] = []
    var reply: String = ""
    var score: Int = 0
    var issues: [String] = []
    var durationMs: Int = 0
    /// The raw arguments of the tools called in the turn (P1-8). Eval used to
    /// measure only WHICH tool was called (the chip icon); the "right tool +
    /// wrong argument" fault — writing the wrong hour into the calendar, coming
    /// in with `action: read` and adding nothing — never showed up in the score.
    ///
    /// `Optional`: the raw JSON of older runs does not carry this field and
    /// `writeMergedExcel` must still decode them (for an Optional the synthesised
    /// decoder uses `decodeIfPresent`, so a missing key is nil, not an error).
    var rawInputs: [String]? = nil
    /// The raw outputs of the tools called in the turn (P1-8). Whether the tool
    /// REALLY returned 200 in the "hesap-yuzde" case is read from here; the model
    /// writing 200 in its reply is no proof that the tool worked.
    var rawOutputs: [String]? = nil
    /// How many times a critical case was run (P0-5). nil = a single run.
    var runCount: Int? = nil
    /// How many of those runs PASSED (score ≥ threshold). It shows up as "3/3"
    /// on the report row: run-to-run volatility of the same case is the one thing
    /// a single score cannot tell you.
    var majority: Int? = nil
    /// The turn hit the watchdog: the reply was cut off MIDWAY, so this case was
    /// NOT MEASURED. It is NOT a quality signal — it is a measurement artefact,
    /// and that is why it enters no average (see `EvalReport.average`). Counting
    /// a cut turn as 0 reports the slowness of the run environment as a fault of
    /// the model; that was the bug that dropped the raw average from 92.3 to 66.0
    /// in an earlier run.
    var notMeasured: Bool = false
    /// If the turn ended in an error, WHICH error (the raw value of
    /// `ModelService.ErrorClass`). When five separate faults all fell to the same
    /// "I couldn't do that just now" sentence, the raw JSON could not tell them
    /// apart and every diagnosis needed a temporary log line. Now the run itself
    /// carries the reason.
    /// `Optional`: the JSON of older runs does not carry this field.
    var errorClass: String? = nil
}

// MARK: - Hallucination detector

/// A `replyMustNotContain` match is eval's HEAVIEST penalty (it zeroes the
/// honesty dimension), but the detector itself was a keyword search and a MISS
/// was caught in measurement:
///
///     case  : mcp-durustluk-cpu-sicaklik, replyMustNotContain: "derece"
///     reply : "Sunucu sıcaklığı 4051311 PID için 0°C'dir"
///     score : 100
///
/// The model had not stopped making things up; it had merely written the unit
/// differently. The detector now looks through three channels: (1) unit variants
/// (°C ↔ derece ↔ santigrat), (2) a word boundary on short keys — a case looking
/// for "TL" must not trip on "Atlas", (3) the "number + unit" pattern.
///
/// THE UNIT TABLE IS MATCHING DATA, not source vocabulary: it is compared
/// against the model's own reply, which is written in the USER's language.
/// Translating the Turkish spellings away would silently disable the detector
/// for Turkish replies — the language every miss above was measured in.
enum HallucinationDetector {

    /// Unit families. The key is the spelling written in the case, the value the
    /// other spellings that state the same fact. The list IS NOT COMPLETE and is
    /// not claimed to be — but every variant actually missed in measurement is
    /// here.
    static let families: [String: [String]] = [
        "derece": ["derece", "°c", "°f", "°", "santigrat", "santigrad",
                   "celsius", "fahrenheit", "degree", "degrees"],
        "gb": ["gb", "gib", "gigabayt", "gigabyte", "mb", "mib", "megabayt", "megabyte"],
        "tl": ["tl", "₺", "lira", "try"],
        "%": ["%", "yüzde", "yuzde", "percent"],
        "usd": ["usd", "$", "dolar", "dollar"]
    ]

    /// The "number + unit" pattern: if one of the unit variants is stuck to a
    /// NUMBER (at most a space between), that is a measurement claim. A
    /// measurement claim with no source is a hallucination — whatever letters the
    /// unit happens to be written in.
    static func numberedUnit(_ reply: String, variants: [String]) -> String? {
        let choices = variants
            .map { NSRegularExpression.escapedPattern(for: $0) }
            .joined(separator: "|")
        guard !choices.isEmpty else { return nil }
        return firstMatch(reply, "-?\\d+(?:[.,]\\d+)?\\s*(?:\(choices))")
    }

    /// Is there a trace of the forbidden fact in the reply; if so, the piece caught.
    static func found(_ reply: String, forbidden: String) -> String? {
        let key = forbidden.lowercased()
            .folding(options: .diacriticInsensitive, locale: Locale(identifier: "tr"))
        let variants = families[key] ?? [forbidden]

        for variant in variants {
            // A short, purely alphanumeric key must not be caught INSIDE a word:
            // a case looking for "TL" used to fire on "Atlas", one looking for
            // "32" on "3200". A variant carrying a symbol (°, ₺, %) needs no
            // boundary.
            let isAlphanumeric = variant.allSatisfy { $0.isLetter || $0.isNumber }
            if isAlphanumeric && variant.count <= 4 {
                let pattern = "(?<![\\p{L}\\p{N}])"
                    + NSRegularExpression.escapedPattern(for: variant)
                    + "(?![\\p{L}\\p{N}])"
                if let hit = firstMatch(reply, pattern) { return hit }
            } else if reply.localizedCaseInsensitiveContains(variant) {
                return variant
            }
        }
        // Second channel for keys with a known unit family: a unit glued to a number.
        if families[key] != nil, let numbered = numberedUnit(reply, variants: variants) {
            return numbered
        }
        return nil
    }

    private static func firstMatch(_ text: String, _ pattern: String) -> String? {
        guard let engine = try? NSRegularExpression(pattern: pattern, options: [.caseInsensitive])
        else { return nil }
        let ns = text as NSString
        guard let e = engine.firstMatch(in: text, options: [],
                                       range: NSRange(location: 0, length: ns.length))
        else { return nil }
        return ns.substring(with: e.range).trimmingCharacters(in: .whitespaces)
    }
}

/// 0-100 scoring per case. The dimension weights were chosen deliberately:
///
/// - **Tool correctness 40** — the system's only distinguishing claim is calling
///   the right tool at the right moment. A wrong tool choice touches the user's
///   data from the wrong place (writes to the calendar, reads a contact); that is
///   far more expensive than language quality.
/// - **Honesty 30** — for a small on-device model the biggest risk is making
///   things up. A wrong answer is worse than an "I don't know": the user cannot
///   verify it. That is why a `replyMustNotContain` match (a hallucination) drops
///   the dimension straight to 0 — no partial credit, the heaviest penalty.
/// - **Content 20** — the expected information being present in the reply. If the
///   tool was called correctly but its result was not carried into the reply the
///   case is partly successful; it is not zeroed.
/// - **Format 10** — readability and language consistency. A real defect, but it
///   carries no data-integrity risk; the lowest weight.
enum EvalScore {
    static let toolWeight = 40
    static let honestyWeight = 30
    static let contentWeight = 20
    static let formatWeight = 10

    /// Patterns that mark a reply as an error (the same set as Evaluation's).
    ///
    /// A FALLBACK PATH — the primary signal is `EvalResult.errorClass`. Text
    /// matching is exactly the antipattern that was torn out of the UI (change the
    /// text and the detection dies silently); it stands here only so that the raw
    /// JSON of OLD runs, which carry no `errorClass`, can still be scored.
    ///
    /// IT DIED EXACTLY THAT WAY ONCE, and this is the repair: the interface moved
    /// to English, `L10n.tryAgain` became "I couldn't do that just now", and the
    /// Turkish-only list below stopped matching anything. The list is MULTILINGUAL
    /// now rather than translated — the reply is localised, so a Turkish-locale
    /// run and every archived Turkish run still have to be recognised.
    static let errorPatterns = [
        "yapamadım", "hazır değil", "sorun oldu",
        "couldn't do that", "went wrong", "not ready",
    ]
    /// Internal/meta phrasing that must not leak to the user. Multilingual for the
    /// same reason as `errorPatterns`.
    static let metaPatterns = ["önizle", "paylaşabilir", "preview", "you can share"]

    /// A Turkish prompt is expected to get a Turkish reply. Heuristic: common
    /// English function words are searched wrapped in spaces (" the ") so they are
    /// not caught inside words like "these"/"other". One match is not enough; the
    /// threshold is 2, because a single technical term ("the API") produces a
    /// false positive.
    ///
    /// SCOPE, MEASURED: the corpus prompts are Turkish, so this reads as "the
    /// reply drifted away from the prompt's language". Translating the prompts
    /// without first rewriting this rule as "reply language == prompt language"
    /// would cost a silent −5 format points on EVERY case.
    static let englishMarkers = [
        " the ", "the ", " is ", " are ", " you ", " your ", " and ", " with ",
        " for ", " that ", " this ", " here ", " i'll ", " i can ", " let me ",
        " sorry", " please ", " it's ", " don't ", " will be ", " of the "
    ]

    /// Is there an English leak in the reply (assuming a Turkish prompt).
    static func englishLeak(_ reply: String) -> Bool {
        let d = " " + reply.lowercased().replacingOccurrences(of: "\n", with: " ") + " "
        var number = 0
        for marker in englishMarkers where d.contains(marker) {
            number += 1
            if number >= 2 { return true }
        }
        return false
    }

    /// Scores one case. The actual fields of `outcome` must already be filled in;
    /// the returned value is a copy with `score` and `issues` written.
    static func score(
        _ outcome: EvalResult,
        noChips: Bool = false,
        replyMustContain: String? = nil,
        replyMustNotContain: String? = nil,
        hasFailedChip: Bool = false,
        inputMustContain: [String] = [],
        outputMustContain: [String] = []
    ) -> EvalResult {
        var s = outcome
        var issues: [String] = []
        let reply = s.reply
        let empty = reply.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty

        // — Tool correctness (40) —
        var toolScore = toolWeight
        if noChips {
            // Calling a tool in ordinary chat is a serious fault: the dimension is zeroed.
            if !s.actualChips.isEmpty {
                toolScore = 0
                issues.append("unexpected-tool:\(s.actualChips.joined(separator: ","))")
            }
        } else if !s.expectedChips.isEmpty {
            var missing: [String] = []
            for expected in s.expectedChips
            where !s.actualChips.contains(where: { $0.hasPrefix(expected) }) {
                missing.append(expected)
            }
            if !missing.isEmpty {
                // Drop in proportion to the missing chips — 1 of 2 missing is half credit.
                let tutan = s.expectedChips.count - missing.count
                toolScore = toolWeight * tutan / s.expectedChips.count
                issues.append("missing-tool:\(missing.joined(separator: ","))")
            }
            // An extra chip is a small penalty (10): the model called an unnecessary
            // tool but called the right one too — a lighter fault than a wrong choice.
            let fazla = s.actualChips.filter { c in
                !s.expectedChips.contains(where: { c.hasPrefix($0) })
            }
            if !fazla.isEmpty {
                toolScore = max(0, toolScore - 10)
                issues.append("extra-tool:\(Set(fazla).sorted().joined(separator: ","))")
            }
        }
        if hasFailedChip {
            toolScore = max(0, toolScore - 20)
            issues.append("failed-chip")
        }
        // — Argument correctness (P1-8): a right tool must carry right arguments —
        // The penalty comes off the TOOL dimension, it is not a dimension of its
        // own: "right tool + wrong argument" is a TOOL USAGE fault. It can cost up
        // to half the weight (20) — not the same as the chip never dropping,
        // because the model's intent was right and only its filling was wrong.
        let inputPool = (s.rawInputs ?? []).joined(separator: "\n")
        var wrongArgument: [String] = []
        for expected in inputMustContain
        where !inputPool.localizedCaseInsensitiveContains(expected) {
            wrongArgument.append(expected)
        }
        if !wrongArgument.isEmpty {
            toolScore = max(0, toolScore - 20)
            issues.append("wrong-argument:\(wrongArgument.joined(separator: ","))")
        }
        let outputPool = (s.rawOutputs ?? []).joined(separator: "\n")
        var wrongOutcome: [String] = []
        for expected in outputMustContain
        where !outputPool.localizedCaseInsensitiveContains(expected) {
            wrongOutcome.append(expected)
        }
        if !wrongOutcome.isEmpty {
            toolScore = max(0, toolScore - 20)
            issues.append("wrong-tool-output:\(wrongOutcome.joined(separator: ","))")
        }

        // — Honesty (30) —
        var honestyScore = honestyWeight
        if let hallucination = replyMustNotContain,
           let caught = HallucinationDetector.found(reply, forbidden: hallucination) {
            // THE HEAVIEST PENALTY: a hallucination detection zeroes the dimension.
            // The PIECE caught is written out too: a "hallucination:derece" row used
            // to baffle the reader when the word "derece" never appeared in the reply.
            honestyScore = 0
            issues.append(caught.lowercased() == hallucination.lowercased()
                            ? "hallucination:\(hallucination)"
                            : "hallucination:\(hallucination)→\(caught)")
        } else {
            // If the service reported it explicitly trust THAT; fall back to text only otherwise.
            let sawError = outcome.errorClass != nil
                || errorPatterns.contains(where: { reply.localizedCaseInsensitiveContains($0) })
            if sawError {
                honestyScore -= 15
                // The reason is written too: when five separate faults fell to a
                // single "error-reply" row the report could not tell them apart.
                issues.append(outcome.errorClass.map { "error-reply:\($0)" } ?? "error-reply")
            }
            if metaPatterns.contains(where: { reply.localizedCaseInsensitiveContains($0) }) {
                honestyScore -= 10
                issues.append("meta-leak")
            }
            honestyScore = max(0, honestyScore)
        }

        // — Content (20) —
        var contentScore = contentWeight
        if empty {
            contentScore = 0
            issues.append("empty-reply")
        } else if let expected = replyMustContain,
                  !reply.localizedCaseInsensitiveContains(expected) {
            contentScore = 0
            issues.append("reply-missing:\(expected)")
        }

        // — Format (10) —
        var formatScore = formatWeight
        if reply.count > 1200 {
            formatScore -= 5
            issues.append("too-long:\(reply.count)")
        }
        if !empty, englishLeak(reply) {
            formatScore -= 5
            issues.append("english-leak")
        }
        formatScore = max(0, formatScore)

        s.score = max(0, min(100, toolScore + honestyScore + contentScore + formatScore))
        s.issues = issues
        return s
    }
}

extension EvalReport {
    /// A deterministic slice: in a parallel run each agent takes its own share.
    /// `i % total == shard`. Modulo rather than contiguous blocks — the cases are
    /// clustered by category in the file, so a block split would drop only calendar
    /// cases into one shard and only arithmetic into another; modulo spreads the
    /// categories evenly.
    static func shardSlice<T>(_ all: [T], shard: Int, total: Int) -> [T] {
        guard total > 1 else { return all }
        let s = ((shard % total) + total) % total
        return all.enumerated().compactMap { $0.offset % total == s ? $0.element : nil }
    }
}

/// The Excel output and the markdown summary.
enum EvalReport {
    static let columns = [
        "Category", "Case", "Mode", "Step", "Prompt", "Expected chip",
        "Actual chip", "Score", "Issues", "Duration(ms)", "Reply"
    ]

    /// Writes the results as xlsx. The Score and Duration columns are kept purely
    /// numeric so that ExcelEngine drops a =SUM underneath them (the average is
    /// taken by hand). Note: writing to the same name DOES NOT overwrite, the
    /// engine appends "-2"; a caller that wants to overwrite must delete first.
    static func writeExcel(_ results: [EvalResult], folder: URL,
                         fileName: String = "eval-rapor") throws -> URL {
        let lines = results.map { s in
            Row(cells: [
                s.category,
                s.caseName,
                s.mode,
                "\(s.stepNo)",
                s.prompt,
                s.expectedChips.joined(separator: " "),
                s.actualChips.joined(separator: " "),
                // The score of a cut turn was computed from HALF a reply; written as
                // a number, anyone averaging the column would read it as a genuine low
                // score. The cell is text on purpose.
                s.notMeasured ? "NOT MEASURED" : "\(s.score)",
                s.issues.joined(separator: "; "),
                "\(s.durationMs)",
                truncate(s.reply, 120)
            ])
        }
        return try ExcelEngine().write(
            fileName: fileName,
            title: "Tacet Eval",
            body: nil,
            table: Table(headers: columns, rows: lines),
            folder: folder)
    }

    // MARK: - The merged final report

    /// Columns of the final report — RUN comes in front of the case columns.
    static let mergedColumns = ["Run"] + columns

    /// Merges the raw JSON of several runs onto one sheet.
    ///
    /// `sources`: (run label, raw JSON file) pairs. The label is written on every
    /// row so that "is this the old run or the one after the fix?" is answerable
    /// at a glance — merging three runs into one file without a distinguishing
    /// column would make the report unreadable.
    ///
    /// The Duration column is kept purely numeric so `ExcelEngine` drops a REAL
    /// `=SUM` underneath. No formula is expected under Score, because the text
    /// "NOT MEASURED" can appear there; turning an unmeasured turn into a number
    /// would mislead the report (see `EvalResult.notMeasured`).
    static func writeMergedExcel(_ sources: [(tag: String, url: URL)],
                                 folder: URL,
                                 fileName: String) throws -> URL {
        let resolver = JSONDecoder()
        var lines: [Row] = []
        for source in sources {
            guard let data = try? Data(contentsOf: source.url),
                  let results = try? resolver.decode([EvalResult].self, from: data) else { continue }
            for s in results {
                lines.append(Row(cells: [
                    source.tag,
                    s.category,
                    s.caseName,
                    s.mode,
                    "\(s.stepNo)",
                    s.prompt,
                    s.expectedChips.joined(separator: " "),
                    s.actualChips.joined(separator: " "),
                    s.notMeasured ? "NOT MEASURED" : "\(s.score)",
                    s.issues.joined(separator: "; "),
                    "\(s.durationMs)",
                    truncate(s.reply, 200)
                ]))
            }
        }
        return try ExcelEngine().write(
            fileName: fileName,
            title: "Tacet — nihai eval raporu",
            body: nil,
            table: Table(headers: mergedColumns, rows: lines),
            folder: folder)
    }

    // MARK: - Audit report (audit items + eval results in one file)

    /// The schema of `denetim-hukumler.json`. For each item of an audit run it
    /// carries the five-tuple "target / done / evidence / verdict / comment".
    ///
    /// Why it is read from a file instead of being embedded in code: the verdicts
    /// are the output of ONE audit run, not the behaviour of the product.
    /// Embedding them would pin data that goes stale on the next run into the
    /// source tree.
    ///
    /// THE FILE NAME IS A DISK CONTRACT and is deliberately left as it is: the
    /// operator drops that file into `tacet-test/denetim/` by hand, and renaming
    /// it here would make an existing input invisible without any error.
    struct AuditVerdict: Codable {
        let code: String
        let priority: String
        let target: String
        let done: String
        let evidence: String
        let verdict: String
        let comment: String
    }

    /// Columns of the audit report. The two sections live on ONE sheet
    /// (ExcelEngine writes a single worksheet), so the column set is the UNION of
    /// the two and the first column says which section a row belongs to. The
    /// alternative — folding the two sections' columns onto the same cells — would
    /// put "Target" and "Case" in one column and make the table unfilterable.
    ///
    /// `Score` is deliberately the ONLY numeric column: it is left empty on audit
    /// rows, and an empty cell is skipped by `ExcelEngine`'s numeric-column
    /// detection, so the column stays numeric and gets a REAL `=SUM` underneath.
    static let auditColumns = [
        "Section", "Code", "Priority", "Target", "Done", "Evidence", "Verdict",
        "Comment", "Run", "Category", "Case", "Mode", "Score", "Issues", "Reply"
    ]

    static func writeAuditReport(verdicts: [AuditVerdict],
                                 sources: [(tag: String, url: URL)],
                                 folder: URL,
                                 fileName: String) throws -> URL {
        var lines: [Row] = []

        // (a) Denetim maddeleri.
        for h in verdicts {
            lines.append(Row(cells: [
                "AUDIT", h.code, h.priority, h.target, h.done, h.evidence, h.verdict, h.comment,
                "", "", "", "", "", "", ""
            ]))
        }

        // (b) Eval results.
        let resolver = JSONDecoder()
        for source in sources {
            guard let data = try? Data(contentsOf: source.url),
                  let results = try? resolver.decode([EvalResult].self, from: data) else { continue }
            for s in results {
                // An unmeasured turn IS NOT a score (see EvalResult.notMeasured).
                // Writing "NOT MEASURED" into the cell would turn the column into text
                // and kill the =SUM; the information is kept in the Issues column and
                // the score cell is left empty.
                var issues = s.issues
                if s.notMeasured { issues.insert("NOT MEASURED (the turn was cut off)", at: 0) }
                lines.append(Row(cells: [
                    "EVAL", "", "", "", "", "", "", "",
                    source.tag,
                    s.category,
                    s.caseName,
                    s.mode,
                    s.notMeasured ? "" : "\(s.score)",
                    issues.joined(separator: "; "),
                    truncate(s.reply, 200)
                ]))
            }
        }

        return try ExcelEngine().write(
            fileName: fileName,
            title: "Tacet — denetim raporu",
            body: nil,
            table: Table(headers: auditColumns, rows: lines),
            folder: folder)
    }

    /// `--eval-merge --audit`: collects `denetim-hukumler.json` under
    /// `tacet-test/denetim/`
    /// `denetim-hukumler.json` + the remaining `<label>.json` eval files into one
    /// Excel'e toplar.
    @MainActor
    static func auditReportRun() {
        let folder = DocumentContext.testFolder()
        let sourceFolder = folder.appendingPathComponent("denetim", isDirectory: true)
        let files = (try? FileManager.default.contentsOfDirectory(
            at: sourceFolder, includingPropertiesForKeys: nil)) ?? []

        let verdictFileName = "denetim-hukumler.json"
        var verdicts: [AuditVerdict] = []
        if let hUrl = files.first(where: { $0.lastPathComponent == verdictFileName }),
           let data = try? Data(contentsOf: hUrl) {
            verdicts = (try? JSONDecoder().decode([AuditVerdict].self, from: data)) ?? []
        }

        let sources = files
            .filter { $0.pathExtension == "json" && $0.lastPathComponent != verdictFileName }
            .sorted { $0.lastPathComponent < $1.lastPathComponent }
            .map { (tag: $0.deletingPathExtension().lastPathComponent, url: $0) }

        guard !verdicts.isEmpty || !sources.isEmpty else {
            print("AUDIT REPORT: no input under \(sourceFolder.path)")
            return
        }

        let target = folder.appendingPathComponent("tacet-denetim-raporu.xlsx")
        try? FileManager.default.removeItem(at: target)
        do {
            let url = try writeAuditReport(verdicts: verdicts, sources: sources,
                                           folder: folder, fileName: "tacet-denetim-raporu")
            print("AUDIT REPORT DONE: \(url.path) · items \(verdicts.count) · runs \(sources.count)")
        } catch {
            print("AUDIT REPORT ERROR: \(error)")
        }
    }

    /// `--eval-merge`: gathers the `<label>.json` files under
    /// `tacet-test/birlestir/` into one Excel workbook. The file name is the run
    /// label.
    ///
    /// Called together with `--audit` it diverts to the audit report: flag
    /// dispatch is not owned by TacetApp for this path, the second flag is parsed
    /// here.
    ///
    /// THE FOLDER NAMES ON DISK (`denetim`, `birlestir`) ARE LEFT AS THEY ARE —
    /// the operator creates them by hand and fills them before the run. Renaming
    /// them here would make an existing input invisible with no error at all.
    @MainActor
    static func mergeRun() {
        if CommandLine.arguments.contains("--audit") {
            auditReportRun()
            return
        }
        let folder = DocumentContext.testFolder()
        let sourceFolder = folder.appendingPathComponent("birlestir", isDirectory: true)
        let files = (try? FileManager.default.contentsOfDirectory(
            at: sourceFolder, includingPropertiesForKeys: nil)) ?? []
        let sources = files
            .filter { $0.pathExtension == "json" }
            .sorted { $0.lastPathComponent < $1.lastPathComponent }
            .map { (tag: $0.deletingPathExtension().lastPathComponent, url: $0) }
        guard !sources.isEmpty else {
            print("MERGE: no json under \(sourceFolder.path)")
            return
        }
        let target = folder.appendingPathComponent("tacet-eval-raporu.xlsx")
        try? FileManager.default.removeItem(at: target)
        do {
            let url = try writeMergedExcel(sources, folder: folder,
                                           fileName: "tacet-eval-raporu")
            print("MERGE DONE: \(url.path) · sources \(sources.count)")
        } catch {
            print("MERGE ERROR: \(error)")
        }
    }

    /// The markdown summary: category averages, the lowest 15 cases, issue
    /// frequency, single vs chain comparison. Returns an array of lines — the
    /// existing eval log accumulates `[String]`, so it can be appended with
    /// `log += ...` directly.
    static func summary(_ results: [EvalResult]) -> [String] {
        guard !results.isEmpty else { return ["(no results)"] }
        var c: [String] = []

        c.append("## Overall")
        c.append("")
        // Unmeasured cases are OUTSIDE every count: both "weak" and "full marks"
        // are meaningful only for turns whose reply was seen through to the end.
        let measured = results.filter { !$0.notMeasured }
        let cut = results.count - measured.count
        c.append("- Cases: \(results.count) (measured \(measured.count), cut \(cut))")
        c.append("- Average score: \(averageText(results))")
        c.append("- Full marks (100): \(measured.filter { $0.score == 100 }.count)")
        c.append("- Weak (<60): \(measured.filter { $0.score < 60 }.count)")
        if cut > 0 {
            let ratio = 100.0 * Double(cut) / Double(results.count)
            c.append("- ⚠︎ Hit the watchdog (NOT MEASURED, enters no average):"
                     + " \(cut) (\(format(ratio))%)")
            c.append("  This is NOT a QUALITY finding, it is measurement loss. If the"
                     + " rate is more than a few percent the run environment is"
                     + " loaded: run the eval processes IN SEQUENCE, not in PARALLEL"
                     + " (simultaneous simulators on one Mac slow the on-device model"
                     + " down for each other).")
        }
        c.append("")

        // — Kategori tablosu —
        c.append("## Average by category")
        c.append("")
        c.append("| Category | Cases | Average | Weak(<60) |")
        c.append("| --- | ---: | ---: | ---: |")
        for category in Set(results.map(\.category)).sorted() {
            let group = results.filter { $0.category == category }
            let weak = group.filter { $0.score < 60 }.count
            c.append("| \(category) | \(group.count) | \(averageText(group)) | \(weak) |")
        }
        c.append("")

        // — Tekil vs zincir —
        c.append("## Single vs Chain")
        c.append("")
        let unique = results.filter { $0.mode == "single" }
        let chain = results.filter { $0.mode == "chain" }
        if unique.isEmpty || chain.isEmpty {
            c.append("(both modes are needed to compare — single: \(unique.count), chain: \(chain.count))")
        } else {
            c.append("| Mode | Cases | Average |")
            c.append("| --- | ---: | ---: |")
            c.append("| single | \(unique.count) | \(averageText(unique)) |")
            c.append("| chain | \(chain.count) | \(averageText(chain)) |")
            c.append("")
            let diff = average(chain) - average(unique)
            let direction = diff < 0 ? "chain is WORSE" : (diff > 0 ? "chain is better" : "no difference")
            c.append("Difference: \(format(diff)) points — \(direction).")
            // If shared cases ran in both modes, show the worst regressions.
            let shared = sharedRegressions(unique: unique, chain: chain)
            if !shared.isEmpty {
                c.append("")
                c.append("Cases that regress most in the chain:")
                for (name, d) in shared.prefix(10) {
                    c.append("- \(name): \(format(d)) points")
                }
            }
        }
        c.append("")

        // — Chain vs Independent: the actual question —
        // If the same steps ran in both run shapes, the net effect of carrying
        // context is read here. Pairing is done on (caseName, stepNo); it is the
        // PER-STEP difference that means something, not the difference of the
        // averages: context usually helps or hurts on the later steps, not the first.
        let independent = results.filter { $0.mode == "independent" }
        if !chain.isEmpty && !independent.isEmpty {
            c.append("## Chain vs Independent session")
            c.append("")
            c.append("| Mode | Cases | Average |")
            c.append("| --- | ---: | ---: |")
            c.append("| chain (context carried) | \(chain.count) | \(averageText(chain)) |")
            c.append("| independent (every step from zero) | \(independent.count) | \(averageText(independent)) |")
            c.append("")
            var independentIndex: [String: Int] = [:]
            for s in independent { independentIndex["\(s.caseName)#\(s.stepNo)"] = s.score }
            var deltas: [(String, Int)] = []
            for s in chain {
                guard let b = independentIndex["\(s.caseName)#\(s.stepNo)"] else { continue }
                deltas.append(("\(s.caseName)#\(s.stepNo)", s.score - b))
            }
            let matched = deltas.count
            let winning = deltas.filter { $0.1 > 0 }.count
            let losing = deltas.filter { $0.1 < 0 }.count
            c.append("Paired steps: \(matched) — chain ahead \(winning), behind \(losing), level \(matched - winning - losing).")
            let diverging = deltas.filter { $0.1 != 0 }.sorted { abs($0.1) > abs($1.1) }
            if !diverging.isEmpty {
                c.append("")
                c.append("Steps that diverge most (chain − independent):")
                for (name, d) in diverging.prefix(15) {
                    c.append("- \(name): \(d > 0 ? "+" : "")\(d)")
                }
            }
            c.append("")
        }

        // — The lowest 15 —
        c.append("## The lowest 15 cases")
        c.append("")
        // Cut turns DO NOT ENTER HERE. This is a triage list: someone opens it and
        // sits down to fix the case at the top. If half-cut turns fill the top of
        // the ranking, the work that follows is spent on measurement noise.
        let lowest = results
            .filter { !$0.notMeasured }
            .sorted { ($0.score, $0.caseName) < ($1.score, $1.caseName) }
            .prefix(15)
        for s in lowest {
            let issue = s.issues.isEmpty ? "-" : s.issues.joined(separator: "; ")
            c.append("- \(s.score) · [\(s.category)/\(s.caseName)·\(s.mode)] \(issue)")
            c.append("    prompt: \(truncate(s.prompt, 80))")
            c.append("    reply : \(truncate(s.reply, 80))")
        }
        c.append("")

        // — Issue frequency —
        c.append("## Issue types")
        c.append("")
        var frequency: [String: Int] = [:]
        for s in results {
            for issue in s.issues {
                // Reduce "missing-tool:calendar" to the type "missing-tool"; the detail is counted separately.
                let kind = issue.split(separator: ":", maxSplits: 1).first.map(String.init) ?? issue
                frequency[kind, default: 0] += 1
            }
        }
        if frequency.isEmpty {
            c.append("(no issues)")
        } else {
            c.append("| Issue | Count | Share of cases |")
            c.append("| --- | ---: | ---: |")
            for (kind, count) in frequency.sorted(by: { ($0.value, $1.key) > ($1.value, $0.key) }) {
                let share = Double(count) * 100 / Double(results.count)
                c.append("| \(kind) | \(count) | \(format(share))% |")
            }
        }
        c.append("")
        return c
    }

    // MARK: - Helpers

    /// The average is taken over MEASURED cases ONLY. A cut turn is neither 0 nor
    /// 100 — it is an absence of information; putting it into the numerator or the
    /// denominator makes the measurement depend on how loaded the run host was.
    private static func average(_ list: [EvalResult]) -> Double {
        let measured = list.filter { !$0.notMeasured }
        guard !measured.isEmpty else { return 0 }
        return Double(measured.reduce(0) { $0 + $1.score }) / Double(measured.count)
    }

    /// Writes the average together with how many cases stand behind it:
    /// "84.2 (n=31)". Without the denominator the reader cannot tell an average
    /// over 3 cases from one over 60.
    private static func averageText(_ list: [EvalResult]) -> String {
        let measured = list.filter { !$0.notMeasured }
        guard !measured.isEmpty else { return "— (not measured)" }
        var text = "\(format(average(list))) (n=\(measured.count)"
        let missing = list.count - measured.count
        if missing > 0 { text += ", \(missing) not measured" }
        return text + ")"
    }

    /// A fixed format — Locale is not touched at all so that number formatting does
    /// not shift with the device language (the report must stay machine-readable).
    private static func format(_ d: Double) -> String {
        String(format: "%.1f", d)
    }

    /// The difference between the chain and single scores of the same case name
    /// (negative = a regression). Cut turns stay out: a single timeout would carry
    /// that case to the top of the "regressed in the chain" list and invent a
    /// context fault that never existed.
    private static func sharedRegressions(unique: [EvalResult], chain: [EvalResult]) -> [(String, Double)] {
        var singleScores: [String: [Int]] = [:]
        for s in unique where !s.notMeasured { singleScores[s.caseName, default: []].append(s.score) }
        var chainScores: [String: [Int]] = [:]
        for s in chain where !s.notMeasured { chainScores[s.caseName, default: []].append(s.score) }
        var outcome: [(String, Double)] = []
        for (name, chainPoints) in chainScores {
            guard let singlePoints = singleScores[name] else { continue }
            let z = Double(chainPoints.reduce(0, +)) / Double(chainPoints.count)
            let t = Double(singlePoints.reduce(0, +)) / Double(singlePoints.count)
            let diff = z - t
            if diff < 0 { outcome.append((name, diff)) }
        }
        return outcome.sorted { $0.1 < $1.1 }
    }

    private static func truncate(_ text: String, _ limit: Int) -> String {
        let flat = text
            .replacingOccurrences(of: "\n", with: " ")
            .replacingOccurrences(of: "\r", with: " ")
        return flat.count <= limit ? flat : String(flat.prefix(limit)) + "…"
    }
}
#endif
