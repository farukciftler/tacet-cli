//
//  SelfTest.swift
//  Tacet
//
//  The DEBUG self-test: it runs the real engine code (without a model) against
//  large data. This IS the large-file channel — the app layer hands the table
//  straight to the engine, the model is not involved. Opened with the "--selftest"
//  argument.
//

#if DEBUG
import Foundation

enum SelfTest {
    @MainActor
    static func run() {
        var record = ["=== TACET SELF-TEST ==="]
        let folder = DocumentContext.testFolder()

        // A large table: 500 rows, a numeric "Amount" column (triggers =SUM in Excel).
        var rows: [Row] = []
        var expectedTotal = 0
        for i in 1...500 {
            let holds = i * 7
            expectedTotal += holds
            rows.append(Row(cells: [
                "2026-07-\(String(format: "%02d", (i % 28) + 1))",
                "Item \(i)",
                "\(holds)",
            ]))
        }
        let table = Table(headers: ["Date", "Description", "Amount"], rows: rows)
        record.append("Table: \(table.rows.count) rows, expected Amount total=\(expectedTotal)")

        let jobs: [(DocumentFormat, String, Bool)] = [
            (.xlsx, "selftest-excel", true),
            (.docx, "selftest-word", false),
            (.pdf,  "selftest-pdf",  false),
            (.md,   "selftest-markdown", false),
            (.html, "selftest-page", false),   // code-spec §4: a page is a document channel too
        ]

        for (format, name, isTable) in jobs {
            do {
                let engine = DocumentEngines.engine(format)
                let body = isTable ? nil : "# Self-Test Report\n\n" + table.markdown
                let url = try engine.write(fileName: name,
                                           title: "Self Test",
                                           body: body,
                                           table: isTable ? table : nil,
                                           folder: folder)
                let attributes = try? FileManager.default.attributesOfItem(atPath: url.path)
                let size = (attributes?[.size] as? Int) ?? 0
                // Read it back (round-trip).
                let back = try engine.read(url: url)
                let lineCount = back.table?.rows.count ?? back.text.split(separator: "\n").count
                record.append("\(format.fileExtension): WROTE \(url.lastPathComponent) \(size)B · READ rows/blocks=\(lineCount)")
            } catch {
                record.append("\(format.fileExtension): ERROR \(error)")
            }
        }
        // The document context: a produced file must be readable/editable afterwards.
        // The behaviour the "make an excel" → "show it as a table" flow rests on.
        record.append("--- DOCUMENT CONTEXT ---")
        let context = DocumentContext()
        record.append("Runnable document at the start: \(context.runnableDocument == nil ? "none ✓" : "present ✗")")
        let produced = folder.appendingPathComponent("selftest-excel.xlsx")
        context.outputAdded(produced)
        let continued = context.runnableDocument
        record.append("After production: \(continued?.name ?? "none") \(continued?.name == "selftest-excel.xlsx" ? "✓" : "✗")")
        record.append("  format: \(continued?.format.tag ?? "-") \(continued?.format == .xlsx ? "✓" : "✗")")
        // A document the user attached must take precedence over the produced one.
        let attachedURL = folder.appendingPathComponent("selftest-pdf.pdf")
        context.addDocument(url: attachedURL)
        record.append("Attached document takes precedence: \(context.runnableDocument?.name ?? "none") "
                      + "\(context.runnableDocument?.name == "selftest-pdf.pdf" ? "✓" : "✗")")
        // Once the attachment is removed it must fall back to the produced one.
        context.removeDocument()
        record.append("Falls back to the produced one when the attachment is removed: "
                      + "\(context.runnableDocument?.name == "selftest-excel.xlsx" ? "✓" : "✗")")
        // New chat: the context must be cleared entirely, otherwise the old file leaks.
        context.forgetProduced()
        record.append("Cleared in a new chat: \(context.runnableDocument == nil ? "✓" : "✗")")

        // The table returned to the model must be VALID markdown — drawing a table in the
        // chat depends on `Table.fromMarkdown` being able to parse it.
        record.append("--- THE TABLE RETURNED TO THE MODEL ---")
        let fullMarkdown = table.truncatedMarkdown(maxRows: 30)
        let reparsed = Table.fromMarkdown(fullMarkdown)
        record.append("Markdown reparsed: \(reparsed != nil ? "✓" : "✗")")
        record.append("  rows: \(reparsed?.rows.count ?? 0)/30 "
                      + "\(reparsed?.rows.count == 30 ? "✓" : "✗")")
        record.append("  headers: \(reparsed?.headers.joined(separator: ",") ?? "-") "
                      + "\(reparsed?.headers == table.headers ? "✓" : "✗")")
        // THE NOTE TEXT COMES FROM `Table.truncatedMarkdown` — the two must be checked
        // together. It was a Turkish literal here while the producer had already moved to
        // English, so the assertion was silently red.
        record.append("  truncation note present: \(fullMarkdown.contains("+470 more rows") ? "✓" : "✗")")
        // A small table that needs no truncation must come back unchanged.
        let small = Table(headers: ["Day", "Meal"],
                          rows: [Row(cells: ["Monday", "Lentil soup"])])
        record.append("Small table not truncated: "
                      + "\(small.truncatedMarkdown(maxRows: 30) == small.markdown ? "✓" : "✗")")

        // The skill (SKILL.md) layer test.
        record.append("--- SKILLS (SKILL.md) ---")
        // With code + web-page activated (code-spec step 7) the package went to 10 skills.
        let packageCount = SkillStore.package.count
        record.append("Skills loaded from the bundle: \(packageCount) "
                      + "\(packageCount == 10 ? "✓" : "✗ (expected: 10)")")
        // THE PROBES FOLLOW THE TRIGGERS IN `Skills/*.md`. Both the phrases and the
        // expected names moved to English together with those files; a probe left in
        // Turkish matches nothing and turns the whole block red.
        let probes = [
            ("make me a weekly excel", "create-document"),
            ("what is on my calendar tomorrow", "calendar"),
            ("remind me to call at 6", "reminder"),
            ("how much is 125*8", "calc"),
            ("summarize this document", "read-document"),
            ("edit the friday row", "edit-document"),
            ("how are you", "none"),
            // Specificity: "as a table" (read-document) must beat the generic
            // "table" of create-document.
            ("show it as a table", "read-document"),
            // "make a table" NO LONGER triggers create-document, and that is deliberate:
            // the bare "table" trigger was removed, because a table is a DISPLAY request,
            // not a file request (Router.documentAddendum). The user was asking for a
            // table on screen and an .xlsx was being produced silently.
            ("make a weekly meal table", "none"),
            // When a file is asked for EXPLICITLY create-document must still fire — the
            // removal must not close that path (the real regression risk is here).
            ("make the weekly meal list an excel", "create-document"),
            ("show my calendar", "calendar"),
            // code-spec §8: the new skills. Without the "with python" trigger, calc's
            // "calculate" (9) would beat "python" (6) — the specificity rule at work.
            ("can you calculate it with python", "code"),
            ("make a site for my coffee shop", "web-page"),
        ]
        for (question, expected) in probes {
            let found = SkillStore.matching(question)?.name ?? "none"
            let marker = found == expected ? "✓" : "✗ (expected: \(expected))"
            record.append("  '\(question)' → \(found) \(marker)")
        }

        // The injection budget: even the longest bundled skill must not exceed the limit.
        let longest = SkillStore.package
            .map { (name: $0.name, length: SkillStore.injectionText($0).count) }
            .max { $0.length < $1.length }
        if let longest {
            // The limit + the fence text (~200 characters) must stay reasonable in total.
            let cap = SkillStore.injectionLimit + 250
            let marker = longest.length <= cap ? "✓" : "✗ (cap: \(cap))"
            record.append("Longest injection: \(longest.name) \(longest.length) chars \(marker)")
        }

        // A user skill: limit validation + beating the package on a match.
        record.append("--- USER SKILL ---")
        let longBody = String(repeating: "x", count: UserSkill.bodyLimit + 1)
        let oversized = UserSkill(name: "oversized", rawTriggers: "zzz", body: longBody)
        record.append("Body over the limit rejected: \(oversized.isValid ? "✗" : "✓")")
        let noTrigger = UserSkill(name: "empty", rawTriggers: " , ,", body: "something")
        record.append("Trigger-less skill rejected: \(noTrigger.isValid ? "✗" : "✓")")

        let custom = UserSkill(name: "invoice-tracking",
                               rawTriggers: "invoice, expense",
                               body: "When an invoice is asked about, call search first, then calculate.")
        SkillStore.reloadUser([custom])
        let customMatch = SkillStore.matching("what is my invoice this month")?.name ?? "none"
        record.append("  'invoice' → \(customMatch) \(customMatch == "invoice-tracking" ? "✓" : "✗")")
        // A closed skill must not go to the model. The expectation is "NOT
        // invoice-tracking" rather than "none": if the sentence also fits a bundled
        // skill's trigger, disabling the user skill falls back to the package one.
        custom.isActive = false
        SkillStore.reloadUser([custom])
        let closed = SkillStore.matching("what is my invoice this month")?.name ?? "none"
        record.append("  when closed → \(closed) \(closed != "invoice-tracking" ? "✓" : "✗")")
        SkillStore.reloadUser([])

        // The acceptance criteria of the four specs that need no model/network. These are
        // not "look at it by eye" but ASSERTs: a failure is marked on the line and counted
        // at the end.
        var ledger = SelfTestCases.run()
        // code-spec §8: the HtmlEngine cases (a pure engine, no model/network/WKWebView
        // needed).
        ledger.add(htmlCases(folder: folder))
        record.append(contentsOf: ledger.lines)
        record.append("=== ASSERTIONS: \(ledger.assertions) · FAILED: \(ledger.error) "
                      + "\(ledger.error == 0 ? "✓" : "✗") ===")

        record.append("=== DONE · folder: \(folder.path) ===")
        write(record, folder: folder)

        // Cases that require suspension (the approval gate) cannot run inside a
        // synchronous init; they run on the first run-loop turn and append their outcome
        // to the same file.
        Task { @MainActor in
            var extra = await SelfTestCases.runAsync()
            // code-spec §8: the CodeEngine + attempt-counter cases (they need await; the
            // timeout case takes ~3 s and does not fit the synchronous launch-time flow).
            extra.add(await codeCases())
            var full = record
            full.append(contentsOf: extra.lines)
            full.append("=== ASYNC ASSERTIONS: \(extra.assertions) · FAILED: \(extra.error) "
                        + "\(extra.error == 0 ? "✓" : "✗") ===")
            let totalErrors = ledger.error + extra.error
            full.append("=== TOTAL FAILED: \(totalErrors) "
                        + "\(totalErrors == 0 ? "✓ ALL PASSED" : "✗ FAILED") ===")
            write(full, folder: folder)
        }
    }

    // MARK: - code-spec §8: HtmlEngine

    /// Markdown → HTML emission + the `read` back-extraction. PageVerifier is DELIBERATELY
    /// not called (WKWebView is unreliable in the synchronous launch-time flow) — the
    /// assertions here statically exercise the very thing the verification protects
    /// (markdown parsing and the template's self-containedness).
    @MainActor
    private static func htmlCases(folder: URL) -> SelfTestLedger {
        var d = SelfTestLedger()
        d.title("HTML ENGINE (code-spec §4)")

        let markdown = """
        # Corner Coffee
        Freshly roasted & daily.

        ## Menu
        | Coffee | Price |
        | --- | --- |
        | Filter | 90 |
        | <Espresso> | 120 |

        ## Features
        - Fast service
        - **Fresh** beans

        ### Address
        3 Main Street
        """

        let engine = HtmlEngine()
        do {
            let url = try engine.write(fileName: "selftest-html-case", title: nil,
                                       body: markdown, table: nil, folder: folder)
            defer { try? FileManager.default.removeItem(at: url) }
            let raw = try String(contentsOf: url, encoding: .utf8)

            // Self-containedness: neither the template nor the output carries any trace of
            // an external request. A search for "http" must come back empty — the page
            // carries the network promise too (code-spec §4.2).
            d.check(!raw.contains("http"), "the produced HTML contains no 'http' (no external request)")
            d.check(!raw.contains("<script"), "the produced HTML contains no script")

            // Traces of the markdown → template mapping.
            d.check(raw.contains("<header class=\"hero\">"), "the first '# ' becomes the hero section")
            d.check(raw.contains("<h1>Corner Coffee</h1>"), "the hero carries an h1 title")
            d.check(raw.contains("class=\"tagline\""), "the paragraph under the hero becomes the tagline")
            d.check(raw.contains("<title>Corner Coffee</title>"), "the page title comes from the hero")
            d.check(raw.contains("<h2>Menu</h2>"), "'## ' becomes a section heading")
            d.check(raw.contains("<h3>Address</h3>"), "'### ' becomes a sub-heading inside the section")
            d.check(raw.contains("<table>"), "a markdown table becomes <table>")
            d.check(raw.contains("class=\"cards\""), "a '- ' list becomes the card grid")
            d.check(raw.contains("&lt;Espresso&gt;"), "< > in the content is escaped")
            d.check(raw.contains("&amp; daily"), "& in the content is escaped")
            d.check(raw.contains("<strong>Fresh</strong>"), "**bold** becomes strong")

            // Round-trip: `read` strips the tags and turns them back into markdown
            // prefixes — the "add a section to the site" flow (read_document →
            // edit_document) rests on this.
            let back = try engine.read(url: url).text
            d.check(back.contains("# Corner Coffee"), "read: the hero comes back with the '# ' prefix")
            d.check(back.contains("Freshly roasted & daily."), "read: the & escape is undone")
            d.check(back.contains("## Menu"), "read: the section comes back with the '## ' prefix")
            d.check(back.contains("### Address"), "read: the sub-heading comes back with the '### ' prefix")
            d.check(back.contains("| Filter | 90 |"), "read: the table comes back as a markdown row")
            d.check(back.contains("| <Espresso> | 120 |"), "read: the cell escapes are undone")
            d.check(back.contains("- Fast service"), "read: the card comes back as a '- ' item")
            d.check(!back.contains("<style"), "read: the style block is stripped entirely")
            d.check(!back.contains("</"), "read: no closing tag leaks through")
        } catch {
            d.check(false, "the HtmlEngine write/read round-trip", "\(error)")
        }
        return d
    }

    // MARK: - code-spec §8: CodeEngine + the attempt counter

    @MainActor
    private static func codeCases() async -> SelfTestLedger {
        var d = SelfTestLedger()
        d.title("CODE ENGINE (code-spec §5)")

        // print(6*7) → "42".
        switch await CodeEngine.run("print(6*7)") {
        case .succeeded(let output, _):
            d.equal(output, "42", "print(6*7) outputs 42")
        default:
            d.check(false, "print(6*7) outputs 42", "no successful outcome returned")
        }

        // If print was not called, the value of the last expression counts as the output.
        switch await CodeEngine.run("6*7") {
        case .succeeded(let output, _):
            d.equal(output, "42", "the last-expression form (6*7) also gives 42")
        default:
            d.check(false, "the last-expression form (6*7) also gives 42", "no successful outcome returned")
        }

        // A syntax error → error + line number.
        switch await CodeEngine.run("let a=1;\nlet x = ;") {
        case .error(let message):
            d.check(true, "a syntax error returns an error")
            d.check(message.contains("line"), "the error carries a line number", message)
        default:
            d.check(false, "a syntax error returns an error", "no error returned")
        }

        // An infinite loop → a timeout at ~3 s; no silent freeze.
        let start = Date()
        let loop = await CodeEngine.run("while(true){}")
        let elapsed = Date().timeIntervalSince(start)
        if case .timeout = loop {
            d.check(true, "an infinite loop returns a timeout")
        } else {
            d.check(false, "an infinite loop returns a timeout", "\(loop)")
        }
        d.check(elapsed >= 2.5 && elapsed < 10,
                "the timeout happens at ~3 s", String(format: "%.1f s", elapsed))

        // Output above 10k is clipped and the clipping is stated.
        switch await CodeEngine.run("let s='x'.repeat(20000); print(s)") {
        case .succeeded(let output, _):
            d.check(output.contains(L10n.codeOutputTruncated), "the truncation note is added to the output")
            d.check(output.count <= CodeEngine.outputCap + L10n.codeOutputTruncated.count + 1,
                    "the clipped output does not exceed the cap", "\(output.count)")
        default:
            d.check(false, "output above 10k comes back clipped", "no successful outcome returned")
        }

        // The sandbox: fetch/require are undefined — they return an error, they DO NOT RUN.
        switch await CodeEngine.run("fetch('https://example.com')") {
        case .error(let message):
            d.check(message.contains("fetch"), "fetch is undefined (no network bridge)", message)
        default:
            d.check(false, "fetch is undefined (no network bridge)", "no error returned")
        }
        switch await CodeEngine.run("require('fs')") {
        case .error(let message):
            d.check(message.contains("require"), "require is undefined (no file bridge)", message)
        default:
            d.check(false, "require is undefined (no file bridge)", "no error returned")
        }

        // The attempt counter (code-spec §5.4): the 3rd call is refused WITHOUT the engine
        // ever seeing it.
        d.title("CODE TOOL · ATTEMPT COUNTER (code-spec §5.4)")
        let state = CodeState()
        var tool = RunCodeTool()
        tool.state = state

        let a1 = await tool.call(arguments: .init(code: "print(1)"))
        d.check(a1.hasPrefix("ok"), "the 1st call runs", a1)
        let a2 = await tool.call(arguments: .init(code: "print(2)"))
        d.check(a2.hasPrefix("ok"), "the 2nd call runs", a2)
        d.equal(state.attempt, 2, "the counter counted two attempts")
        let a3 = await tool.call(arguments: .init(code: "print(3)"))
        d.check(a3.hasPrefix("error_final"), "the 3rd call is refused with error_final", a3)
        d.check(!a3.contains("ok ("), "the engine is not run on the 3rd call")

        // In production the reset lives in `ModelService`'s turn handling, the
        // recoverFromError retry branches and resetChat — the contract is verified here.
        state.newTurn()
        d.equal(state.attempt, 0, "newTurn() resets the counter")
        let a4 = await tool.call(arguments: .init(code: "print(4)"))
        d.check(a4.hasPrefix("ok"), "the call runs again in a new turn", a4)

        return d
    }

    /// Writes the outcome only to print and to the test file under Caches.
    /// NSLog is not used: personal data landing in the system log is permanent.
    @MainActor
    private static func write(_ record: [String], folder: URL) {
        let text = record.joined(separator: "\n")
        print(text)
        try? text.write(to: folder.appendingPathComponent("selftest-outcome.txt"),
                        atomically: true, encoding: .utf8)
    }
}
#endif
