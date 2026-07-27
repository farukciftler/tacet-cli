//
//  Evaluation.swift
//  Tacet
//
//  Automated evaluation (eval) — runs test cases against the real on-device
//  model for every skill and for ordinary chat; measures correct tool choice,
//  reply quality and whether anything errored. Opened with the "--test"
//  argument (DEBUG only). Results are written incrementally to
//  Caches/tacet-test/test-sonuc.txt — kept apart from the user's document
//  folder because the output contains real calendar / contact data.
//
//  NOTE ON FIXTURE LANGUAGE: the `prompt` strings and the language-bound
//  `replyContains` / `replyExcludes` fragments below are deliberately still
//  Turkish. They are MEASUREMENT DATA, not prose: `EvalScore.englishLeak`
//  (EvalReport.swift) deducts format points when the reply is English, and
//  `LanguageAnchor.audit(_:expected:)` is asserted against "tr". Translating the
//  fixtures without changing those two would silently move every score.
//

#if DEBUG
import Foundation
import SwiftData

struct TestCase {
    let name: String
    let prompt: String
    var icons: [String] = []        // expected chip icon prefixes (all must appear)
    var noChip = false              // ordinary chat: no tool may be called
    var attachedDocument = false    // attach the test document first (read/edit cases)
    var replyContains: String? = nil
    var replyExcludes: String? = nil   // hallucination detection (e.g. must not say "Paris")
    /// Fragments that must appear in the tool ARGUMENT (P1-8). The chip icon
    /// says the right tool was called, it does NOT say the argument was right:
    /// the `calendar-add` case passed even when the model fell into the "read"
    /// branch, because the icon was "calendar" either way. This field makes that
    /// silent class of failure visible.
    var inputContains: [String] = []
    /// Fragments that must appear in the tool OUTPUT (P1-8). In the
    /// "calc-percent" case the TOOL has to be the one saying 200; the model
    /// writing 200 in its reply is no proof the tool computed it.
    var outputContains: [String] = []
    /// Critical case: run N times in the comprehensive run, with the majority
    /// ratio reported. NOT applied to all of them — N runs multiply the wall
    /// time by N and on-device inference does not parallelize; tripling 230
    /// cases pushes a round into hours.
    var critical = false
}

@MainActor
enum Evaluation {
    static func run() {
        Task { await run() }
    }

    static func coreCases() -> [TestCase] {
        [
            // — Ordinary chat (no tools) —
            TestCase(name: "hello", prompt: "Merhaba", noChip: true),
            TestCase(name: "howareyou", prompt: "Nasılsın?", noChip: true),
            TestCase(name: "whoareyou", prompt: "Sen kimsin?", noChip: true),
            // Weather / world knowledge: whether or not it calls a tool, it MUST
            // NOT make the answer up (it should state its limit).
            TestCase(name: "weather", prompt: "Bugün hava nasıl olacak?", replyExcludes: "derece"),
            TestCase(name: "world-info", prompt: "Fransa'nın başkenti neresi?", replyExcludes: "Paris"),

            // — Calc —
            TestCase(name: "calc-multiply", prompt: "125 çarpı 8 kaç eder?", icons: ["function"]),
            TestCase(name: "calc-sum", prompt: "Üç ürün aldım, her biri 45 lira, toplam ne kadar?", icons: ["function"]),
            // P1-8: 250 × 20% discount → 200. The TOOL must say the number; the
            // chip icon being "function" does not show the model computed right.
            TestCase(name: "calc-percent", prompt: "250 liranın yüzde 20 indirimlisi kaç lira?",
                     icons: ["function"], outputContains: ["200"], critical: true),

            // — Time (no chip; the reply must contain an hour/day) —
            TestCase(name: "time-hour", prompt: "Saat kaç?", replyContains: ":"),
            TestCase(name: "time-day", prompt: "Bugün günlerden ne?", replyContains: currentDay()),

            // — Calendar —
            TestCase(name: "calendar-read", prompt: "Yarın neler var?", icons: ["calendar"]),
            TestCase(name: "calendar-week", prompt: "Bu hafta programım ne?", icons: ["calendar"]),
            // P1-8 — this case is the PROOF of the item: only the icon
            // "calendar" used to be looked for, and CalendarTool's READ branch
            // drops the same icon, so the model scored 100 without adding
            // anything. Now both the action and the hour are looked for in the
            // argument.
            // After P0-4 the add branch has its own icon ("calendar.badge.plus")
            // and the expected chip no longer OVERLAPS with the read branch. The
            // argument assertion is a second layer: right branch + wrong hour is
            // caught too.
            TestCase(name: "calendar-add", prompt: "Cuma saat 14:00'te toplantı ekle",
                     icons: ["calendar.badge.plus"],
                     inputContains: ["ekle", "T14:00"], critical: true),

            // — Reminder —
            TestCase(name: "reminder-1", prompt: "Beni 18:00'de aramam için hatırlat", icons: ["bell"]),
            TestCase(name: "reminder-2", prompt: "Yarın ekmek almayı hatırlat", icons: ["bell"]),

            // — Contact —
            TestCase(name: "contact-number", prompt: "Ahmet'in telefon numarası ne?", icons: ["person"]),
            TestCase(name: "contact-mail", prompt: "Mehmet'in e-posta adresini bul", icons: ["person"]),

            // — Search —
            TestCase(name: "search-note", prompt: "Notlarımda toplantı ile ilgili ne var?", icons: ["magnifyingglass"]),
            TestCase(name: "search-find", prompt: "Geçen haftaki alışveriş notumu bul", icons: ["magnifyingglass"]),

            // — Document creation —
            TestCase(name: "docgen-excel", prompt: "Haftalık yemek listesi için bir excel yap", icons: ["tablecells"]),
            TestCase(name: "docgen-pdf", prompt: "Kısa bir tanıtım metnini pdf yap", icons: ["doc"]),
            TestCase(name: "docgen-word", prompt: "Alışveriş listemi word belgesi olarak oluştur", icons: ["doc"]),

            // — Document read/edit (with an attached document) —
            TestCase(name: "docgen-read", prompt: "Bu belgede ne var, özetle", icons: ["tablecells"], attachedDocument: true),
            TestCase(name: "docgen-edit", prompt: "Bu tabloya yeni bir satır ekle: Cumartesi, Pizza", icons: ["tablecells"], attachedDocument: true),

            // — Code execution (code-spec §8) — the result must come from the
            // tool, not off the top of the model's head.
            // The prime sum for 1..100 is 1060; the "python ile" trigger opens
            // the code skill.
            TestCase(name: "code-prime", prompt: "1'den 100'e kadar asal sayıların toplamını python ile bulur musun?",
                     icons: ["curlybraces"], replyContains: "1060"),
            // error_final honesty (a short admission after 2 failed attempts)
            // cannot be imposed on the model deterministically — the SelfTest
            // attempt-counter case locks the tool side, chip visibility locks
            // the model side.

            // — Web page (code-spec §8) — must route to the document profile via
            // the "site" trace, one belge_olustur call, .html on the chip
            // (icon: doc.text.image).
            TestCase(name: "page-site", prompt: "Kahve dükkanım için bir site yap",
                     icons: ["doc.text.image"]),

            // — Chain: device data → file (context budget) —
            TestCase(name: "chain-calendar-excel", prompt: "Bu haftaki etkinliklerimi excel'e dök", icons: ["calendar", "tablecells"]),
        ]
    }

    static func run() async {
        let service = ModelService()
        let folder = DocumentContext.testFolder()
        let resultURL = folder.appendingPathComponent("test-sonuc.txt")

        // Bail out if the model is not ready.
        guard service.state.isReady else {
            try? "MODEL NOT READY: \(service.state.tag)".write(to: resultURL, atomically: true, encoding: .utf8)
            return
        }

        // Produce the test xlsx for the read/edit cases.
        let testDocument = try? ExcelEngine().write(
            fileName: "test-girdi", title: "Test",
            body: nil,
            table: Table(headers: ["Gün", "Yemek"],
                         rows: [Row(cells: ["Pazartesi", "Mercimek"]),
                                    Row(cells: ["Salı", "Tavuk"])]),
            folder: folder)

        let all = coreCases()
        var log: [String] = ["=== TACET EVAL — \(all.count) cases ===", ""]
        var passed = 0

        for (i, v) in all.enumerated() {
            service.resetChat()
            if v.attachedDocument, let testDocument { service.documentContext.addDocument(url: testDocument) }

            let (text, traces) = await service.respond(v.prompt) { _ in }
            let icons = traces.map(\.icon)
            var problems: [String] = []

            // An error reply?
            let errorMarkers = ["yapamadım", "hazır değil", "sorun oldu"]
            if errorMarkers.contains(where: { text.localizedCaseInsensitiveContains($0) }) {
                problems.append("error-reply")
            }
            // A raw tool-call leak? Showing the user an unparsed call payload is
            // ALWAYS a FAIL — it used not to be measured, and some such turns
            // were scoring 100.
            let leakMarkers = ["<executable_end>", "\"arguments\"", "```function"]
            if leakMarkers.contains(where: { text.localizedCaseInsensitiveContains($0) }) {
                problems.append("ham-arac-cagrisi")
            }
            // A meta leak?
            if text.localizedCaseInsensitiveContains("önizle") || text.localizedCaseInsensitiveContains("paylaşabilir") {
                problems.append("meta-leak")
            }
            // A failed chip?
            if traces.contains(where: { if case .failed = $0.state { return true }; return false }) {
                problems.append("failed-chip")
            }
            // The expected tools?
            for expected in v.icons where !icons.contains(where: { $0.hasPrefix(expected) }) {
                problems.append("missing-tool:\(expected)")
            }
            // No tools in ordinary chat.
            if v.noChip && !traces.isEmpty {
                problems.append("unexpected-tool:\(icons)")
            }
            // Does the reply contain the expected text?
            if let inner = v.replyContains, !text.localizedCaseInsensitiveContains(inner) {
                problems.append("reply-missing:\(inner)")
            }
            if let inner = v.replyExcludes,
               let yakalanan = HallucinationDetector.found(text, forbidden: inner) {
                problems.append("hallucination:\(inner)→\(yakalanan)")
            }
            // — Argument / output correctness (P1-8) —
            let inputPool = traces.compactMap(\.rawInput).joined(separator: "\n")
            for expected in v.inputContains
            where !inputPool.localizedCaseInsensitiveContains(expected) {
                problems.append("wrong-argument:\(expected)")
            }
            let outputPool = traces.compactMap(\.rawOutput).joined(separator: "\n")
            for expected in v.outputContains
            where !outputPool.localizedCaseInsensitiveContains(expected) {
                problems.append("wrong-tool-output:\(expected)")
            }

            let ok = problems.isEmpty
            if ok { passed += 1 }
            let shortReply = text.replacingOccurrences(of: "\n", with: " ").prefix(70)
            log.append("\(ok ? "✓" : "✗") [\(v.name)] '\(v.prompt)'")
            log.append("    chip:\(icons) reply:\"\(shortReply)\"")
            if !ok { log.append("    ⚠︎ \(problems.joined(separator: "; "))") }
            log.append("")

            // Write incrementally (so the result is visible even if the run is
            // cut short).
            let snapshot = (["=== \(passed)/\(i + 1) PASSED (in progress) ==="] + log.dropFirst()).joined(separator: "\n")
            try? snapshot.write(to: resultURL, atomically: true, encoding: .utf8)
        }

        var counter = Counter(passed: passed, total: all.count)

        // — Spec cases: memory / timeline / web search —
        // Each block writes its own heading and a per-case EXPECTED/ACTUAL line;
        // there is no "eyeball it" line. So the blocks cannot affect each other,
        // each one sets itself up and tears itself down.
        for block in 0..<5 {
            let outcome: Counter
            switch block {
            case 0: outcome = await memoryRun(service)
            case 1: outcome = await timelineRun(service)
            case 2: outcome = await webRun(service)
            case 3: outcome = await multiToolRun(service)
            default: outcome = await hallucinationRun(service)
            }
            counter.merge(outcome)
            log += outcome.lines
            try? (["=== \(counter.passed)/\(counter.total) PASSED (in progress) ==="] + log.dropFirst())
                .joined(separator: "\n")
                .write(to: resultURL, atomically: true, encoding: .utf8)
        }

        log[0] = counter.title
        try? log.joined(separator: "\n").write(to: resultURL, atomically: true, encoding: .utf8)
        // No NSLog: eval replies contain real calendar/contact data, they must
        // not go to the system log.
        print("EVAL finished: \(counter.passed)/\(counter.total)")
    }

    // MARK: - Counter

    /// The container block results are collected into.
    ///
    /// `misses` and `skipped` COUNT AS PASSES but are reported separately: the
    /// memory acceptance criterion (spec §8) treats a false positive as the
    /// priority — missing a correct note is not a failure, it is a quality
    /// warning. Cases that need the network are also not failed when no server
    /// is configured, they are skipped.
    struct Counter {
        var passed = 0
        var total = 0
        var misses = 0
        var skipped = 0
        var lines: [String] = []

        mutating func merge(_ o: Counter) {
            passed += o.passed; total += o.total
            misses += o.misses; skipped += o.skipped
        }

        /// Records a case outcome and produces its one-line report.
        mutating func write(_ name: String, expected: String, actual: String, problems: [String]) {
            total += 1
            let ok = problems.isEmpty
            if ok { passed += 1 }
            lines.append("\(ok ? "✓" : "✗") [\(name)]")
            lines.append("    expected: \(expected)")
            lines.append("    actual  : \(actual)")
            if !ok { lines.append("    ⚠︎ \(problems.joined(separator: "; "))") }
            lines.append("")
        }

        /// Quality warning: the case passes but a note is recorded (NOT a false
        /// positive).
        mutating func warn(_ text: String) {
            misses += 1
            lines.append("    ↪ miss (acceptance criterion: better than a false positive): \(text)")
        }

        /// A case that could not be run — skip instead of crash (no
        /// network/server).
        mutating func skip(_ name: String, _ cause: String) {
            skipped += 1
            lines.append("⊘ [\(name)] SKIPPED — \(cause)")
            lines.append("")
        }

        mutating func addTitle(_ text: String) {
            lines.append("— \(text) —")
            lines.append("")
        }

        var title: String {
            var s = "=== TACET EVAL: \(passed)/\(total) PASSED"
            if misses > 0 { s += " · \(misses) misses" }
            if skipped > 0 { s += " · \(skipped) skipped" }
            return s + " ==="
        }
    }

    // MARK: - Memory (hafiza-spec §8)

    /// Extraction case: message → expected note count.
    private struct MemoryCase {
        let name: String
        let message: String
        /// Expected note count. If `0`, EVERY note produced is a false positive
        /// (fail).
        let expected: Int
        /// Fragments that must not appear in the note text (a transient detail
        /// escaping).
        var forbidden: [String] = []
    }

    private static func memoryCases() -> [MemoryCase] {
        [
            // An explicit, durable fact → 1 note.
            MemoryCase(name: "memory-vegetarian", message: "Ben vejetaryenim.", expected: 1),
            // A transient observation → NO note.
            MemoryCase(name: "memory-temporary", message: "Bugün hava çok güzel.", expected: 0),
            // Implicit inference (§4.4) is NOT a v1 goal → NO note.
            // Inferring "married" from "my spouse" would be a FALSE POSITIVE.
            MemoryCase(name: "memory-implicit", message: "Eşim yarın geliyor.", expected: 0),
            // Mixed message: only the durable fact ("I am a teacher") should be
            // picked; the transient detail ("tomorrow", "meeting", "headache")
            // must not leak.
            MemoryCase(name: "memory-mixed",
                       message: "Öğretmenim. Yarın saat 10'da veli toplantım var, bugün de başım ağrıyor.",
                       expected: 1,
                       forbidden: ["toplant", "başım", "yarın"])
        ]
    }

    /// On-device extraction measurement. Every case runs in ITS OWN in-memory
    /// store; the real memory store is untouched (production notes are put back
    /// at the end of the block).
    private static func memoryRun(_ service: ModelService) async -> Counter {
        var counter = Counter()
        counter.addTitle("MEMORY (hafiza-spec §8) — false positives take priority")

        // Extraction calls `MemoryStore.reload`; a snapshot is taken first so
        // the production store can be put back. The containers are kept alive
        // for the whole block: if a container dies, an invalid object would be
        // left in the store.
        let productionNotes = MemoryStore.notes
        var containers: [ModelContainer] = []
        let memoryService = MemoryService()

        for testCase in memoryCases() {
            let schema = Schema(versionedSchema: SchemaV1.self)
            let config = ModelConfiguration(schema: schema, isStoredInMemoryOnly: true)
            guard let container = try? ModelContainer(for: schema, configurations: config) else {
                counter.skip(testCase.name, "in-memory store could not be created")
                continue
            }
            containers.append(container)
            let record = container.mainContext

            let chat = Chat()
            record.insert(chat)
            let message = Message(role: .user, content: testCase.message)
            message.chat = chat
            record.insert(message)
            try? record.save()

            await memoryService.extract(chat: chat, record: record)

            let notes = ((try? record.fetch(FetchDescriptor<MemoryNote>())) ?? [])
                .filter { !$0.isDeleted }
            let texts = notes.map(\.text)
            let actual = notes.isEmpty ? "0 notes" : "\(notes.count) notes \(texts)"

            var problems: [String] = []
            if testCase.expected == 0 {
                // False positive — the PRIORITY violation of the acceptance
                // criterion.
                if !notes.isEmpty { problems.append("yanlış-pozitif:\(texts)") }
            } else {
                if notes.count > testCase.expected {
                    problems.append("fazla-not:\(notes.count)>\(testCase.expected)")
                }
                for bad in testCase.forbidden
                where texts.contains(where: { $0.localizedCaseInsensitiveContains(bad) }) {
                    problems.append("gecici-ayrinti-sizdi:\(bad)")
                }
            }

            counter.write(testCase.name,
                      expected: "\(testCase.expected) notes — '\(testCase.message)'",
                      actual: actual,
                      problems: problems)

            // Miss: something was expected, no note came out. The acceptance
            // criterion (§8) does NOT count this as a failure — it is only
            // recorded as a quality signal.
            if testCase.expected > 0 && notes.isEmpty && problems.isEmpty {
                counter.warn("no note produced for '\(testCase.message)'")
                counter.lines.append("")
            }
        }

        // Injection leak (the §5.1 fence): the model MAY USE the note but MUST
        // NOT MENTION it. The note is PINNED to an in-memory context: putting a
        // context-less @Model object into the store is undefined behaviour in
        // SwiftData, and `containers` is what keeps it alive.
        let fenceNote = MemoryNote(text: "Kullanıcı vejetaryendir.",
                                   kind: .preference,
                                   rawKeys: "yemek, öğle yemeği, akşam yemeği")
        let fenceSchema = Schema(versionedSchema: SchemaV1.self)
        if let fenceContainer = try? ModelContainer(
            for: fenceSchema,
            configurations: ModelConfiguration(schema: fenceSchema, isStoredInMemoryOnly: true)) {
            containers.append(fenceContainer)
            fenceContainer.mainContext.insert(fenceNote)
            try? fenceContainer.mainContext.save()
        }
        MemoryStore.reload([fenceNote])
        let question = "Akşam yemeği için ne önerirsin?"
        if MemoryStore.matching(question: question).isEmpty {
            counter.skip("hafiza-sizinti", "note did not match, no injection happened at all")
        } else {
            service.resetChat()
            let (text, _) = await service.respond(question) { _ in }
            let leakPhrases = ["hatırladığıma göre", "hatırlıcomment", "hafızam", "notuma göre",
                               "kayıtlarıma göre", "<memory>", "bana söylediğin"]
            let found = leakPhrases.filter { text.localizedCaseInsensitiveContains($0) }
            counter.write("hafiza-sizinti",
                      expected: "the reply does NOT MENTION the note (no leak pattern)",
                      actual: found.isEmpty
                        ? "no leak — \"\(shorten(text))\""
                        : "leak \(found) — \"\(shorten(text))\"",
                      problems: found.isEmpty ? [] : ["hafiza-sizintisi:\(found)"])
        }

        // Put the production store back — eval does not change the user's memory.
        MemoryStore.reload(productionNotes)
        containers.removeAll()
        return counter
    }

    // MARK: - Timeline (seyir-spec §6)

    private static func timelineRun(_ service: ModelService) async -> Counter {
        var counter = Counter()
        counter.addTitle("TIMELINE (seyir-spec §6) — pure observer")

        // 1) Step ORDER in a turn with tools: routing → … → tool → … → writing.
        service.resetChat()
        let (text, _) = await service.respond("125 çarpı 8 kaç eder?") { _ in }
        let kinds = service.timeline.steps.map(\.kind)
        let sequence = kinds.map(\.rawValue).joined(separator: " → ")

        var problems: [String] = []
        if kinds.first != .routing { problems.append("ilk-adim-yonlendirme-degil:\(kinds.first?.rawValue ?? "none")") }
        guard let toolIndex = kinds.firstIndex(of: .tool) else {
            problems.append("arac-adimi-yok")
            counter.write("seyir-sira",
                      expected: "routing → tool → writing",
                      actual: sequence.isEmpty ? "no steps" : sequence,
                      problems: problems)
            return counter
        }
        if let writingIndex = kinds.firstIndex(of: .writing) {
            if writingIndex < toolIndex { problems.append("yazim-aractan-once:\(writingIndex)<\(toolIndex)") }
        } else {
            problems.append("yazim-adimi-yok")
        }
        if let routingIndex = kinds.firstIndex(of: .routing), routingIndex > toolIndex {
            problems.append("yonlendirme-aractan-sonra")
        }
        counter.write("seyir-sira",
                  expected: "routing → tool → writing (in that order)",
                  actual: sequence,
                  problems: problems)

        // 2) The pure-observer criterion: step texts NEVER enter the model, so
        //    they cannot appear in the model output either. (The timeline cannot
        //    be switched off, so an "off vs on" difference is unmeasurable —
        //    spec §6 already says not to measure it; this is its measurable
        //    counterpart: observer texts must not leak into the reply.)
        let stepTexts = service.timeline.steps.map(\.text).filter { !$0.isEmpty }
        let leaked = stepTexts.filter { text.localizedCaseInsensitiveContains($0) }
        counter.write("seyir-gozlemci",
                  expected: "step texts (\(stepTexts)) DO NOT appear in the reply",
                  actual: leaked.isEmpty ? "absent" : "present: \(leaked)",
                  problems: leaked.isEmpty ? [] : ["gozlemci-sizintisi:\(leaked)"])

        // 3) The `interruption` step in a cancelled turn.
        service.resetChat()
        let task = Task { _ = await service.respond("Bana uzun bir masal anlat.") { _ in } }
        try? await Task.sleep(for: .milliseconds(1500))
        if service.isProducing {
            service.stop()
            _ = await task.value
            let lastKind = service.timeline.steps.last?.kind
            counter.write("seyir-kesinti",
                      expected: "last step kind = interruption",
                      actual: "last step kind = \(lastKind?.rawValue ?? "none") · full sequence: "
                        + service.timeline.steps.map(\.kind.rawValue).joined(separator: " → "),
                      problems: lastKind == .interruption ? [] : ["kesinti-adimi-yok"])
        } else {
            _ = await task.value
            counter.skip("seyir-kesinti", "the turn finished inside 1.5 s, nothing left to interrupt")
        }

        return counter
    }

    // MARK: - Web search (web-arama-spec §6)

    private static func webRun(_ service: ModelService) async -> Counter {
        var counter = Counter()
        counter.addTitle("WEB SEARCH (web-arama-spec §6) — wrong tool choice takes priority")

        let serverAvailable = WebSearchSetting.isActive

        // 1) With the server OFF, "what's the weather" → toolless, honest reply.
        //    If a server is configured the flag is lowered temporarily, then
        //    restored.
        let previous = UserDefaults.standard.bool(forKey: WebSearchSetting.activeKey)
        WebSearchSetting.isActive = false
        service.resetChat()
        let (offText, offTraces) = await service.respond("Bugün hava nasıl?") { _ in }
        let offIcons = offTraces.map(\.icon)
        var offProblems: [String] = []
        if offIcons.contains("globe") { offProblems.append("kapaliyken-web-arama-cagrildi") }
        if offText.localizedCaseInsensitiveContains("derece") {
            offProblems.append("hallucination:derece")
        }
        counter.write("web-kapali-durustluk",
                  expected: "NO globe chip and no 'derece' in the reply (it states its limit)",
                  actual: "chip:\(offIcons) reply:\"\(shorten(offText))\"",
                  problems: offProblems)
        WebSearchSetting.isActive = previous

        // 2) NON-INTERFERENCE: a personal search must not go to the web. This
        //    case does NOT need the network, but for it to mean anything the
        //    tool has to be on the table.
        if serverAvailable {
            service.resetChat()
            let (_, traces) = await service.respond("Notlarımda toplantı ile ilgili ne var?") { _ in }
            let icons = traces.map(\.icon)
            var s: [String] = []
            if icons.contains("globe") { s.append("yanlis-arac:web_arama-kisisel-aramada") }
            if !icons.contains(where: { $0.hasPrefix("magnifyingglass") }) {
                s.append("missing-tool:magnifyingglass")
            }
            counter.write("web-karismama",
                      expected: "magnifyingglass (Spotlight) is called, globe is NOT",
                      actual: "chip:\(icons)",
                      problems: s)
        } else {
            counter.skip("web-karismama", "search server undefined/off")
        }

        // 3) "what's the weather" → a web_arama call with a plausible query.
        //    Query quality is SECONDARY: if it is poor a miss is recorded, the
        //    case does not fail.
        if serverAvailable {
            service.resetChat()
            let (_, traces) = await service.respond("Bugün hava nasıl?") { _ in }
            let icons = traces.map(\.icon)
            let query = traces.first { $0.icon == "globe" }?.rawInput ?? ""
            let called = icons.contains("globe")
            counter.write("web-hava",
                      expected: "the globe chip (web_arama) is called",
                      actual: "chip:\(icons) query:\"\(query)\"",
                      problems: called ? [] : ["missing-tool:globe"])
            if called {
                let plausible = ["weather", "hava", "durum", "sıcak"]
                if !plausible.contains(where: { query.localizedCaseInsensitiveContains($0) }) {
                    counter.warn("query quality is secondary — \"\(query)\" carries no weather key")
                    counter.lines.append("")
                }
            }
        } else {
            counter.skip("web-hava", "search server undefined/off")
        }

        // 4) NO HALLUCINATION when nothing comes back: a query with no answer.
        if serverAvailable {
            service.resetChat()
            let (text, _) = await service.respond("Zrqxvlon Pflumtek 9182 nedir?") { _ in }
            let honesty = ["bulamadım", "bulunamadı", "sonuç yok", "bilgi yok",
                             "emin değilim", "bilmiyorum", "ulaşamadım", "rastlamadım"]
            let honest = honesty.contains { text.localizedCaseInsensitiveContains($0) }
            counter.write("web-uydurmama",
                      expected: "an honest 'could not find it' style reply when there is no result",
                      actual: "\"\(shorten(text))\"",
                      problems: honest ? [] : ["hallucination-suspected:dürüstlük-kalıbı-yok"])
        } else {
            counter.skip("web-uydurmama", "search server undefined/off")
        }

        return counter
    }

    // MARK: - Multi-tool turns (the on-device counterpart of the routing measurement)

    /// Routing fixes were measured in a separate pair with 36 sentences and 10
    /// scenarios; what was measured there was profile SELECTION. What is
    /// measured here is that the turn really finished IN ONE TURN: having the
    /// right tools in the session is not enough, the model has to call both of
    /// them and not drop the data.
    ///
    /// The expected chip set is ASSERTED in every case; a missing tool is a
    /// FAIL, and "it spread over two turns" is not accepted as an excuse (that
    /// was the actual break).
    private static func multiToolRun(_ service: ModelService) async -> Counter {
        var counter = Counter()
        counter.addTitle("MULTI-TOOL TURNS — a missing tool means failure")

        let serverAvailable = WebSearchSetting.isActive

        /// A single multi-tool turn: ALL expected icons must appear.
        func turn(_ name: String, _ prompt: String, _ expected: [String],
                  needsNetwork: Bool) async -> [ToolTrace]? {
            if needsNetwork, !serverAvailable {
                counter.skip(name, "search server undefined/off")
                return nil
            }
            service.resetChat()
            let (text, traces) = await service.respond(prompt) { _ in }
            let icons = traces.map(\.icon)
            var problems: [String] = []
            for b in expected where !icons.contains(where: { $0.hasPrefix(b) }) {
                problems.append("missing-tool:\(b)")
            }
            if traces.contains(where: { if case .failed = $0.state { return true }; return false }) {
                problems.append("failed-chip")
            }
            counter.write(name,
                      expected: "in a single turn \(expected.joined(separator: " + "))",
                      actual: "chip:\(icons) reply:\"\(shorten(text))\"",
                      problems: problems)
            return traces
        }

        // 1) Search + calc in the same turn: the value from the web, the
        //    multiplication from the tool. If the model multiplies 500 by the
        //    rate ITSELF the `function` chip never drops and the number cannot
        //    be verified — which is why a missing calc chip is a FAIL.
        _ = await turn("cok-arac-kur-hesap",
                       "Dolar kaç lira, 500 dolar kaç TL eder?",
                       ["globe", "function"], needsNetwork: true)

        // 2) Search + reminder in the same turn (scenario 2). The search profile
        //    used to be chosen and the reminder was NOT PRESENT in the session;
        //    the work slid into a second turn.
        _ = await turn("cok-arac-namaz-hatirlatici",
                       "İstanbul'da akşam namazı vaktini bul ve o saate hatırlatıcı kur",
                       ["globe", "bell"], needsNetwork: true)

        // 3) Calendar → document (scenario 3). The "row" ⊂ "tomorrow" trap and
        //    the everyday/document clash came together here: when the document
        //    profile could not be chosen, no file was produced at all. Data loss
        //    is measured by the EXISTENCE of the file — the model saying "done"
        //    is not evidence.
        if let traces = await turn("cok-arac-takvim-excel",
                                   "Bu hafta kaç toplantım var, Excel'e dök",
                                   ["calendar", "tablecells"], needsNetwork: false) {
            let file = traces.compactMap(\.filePath).first { $0.hasSuffix(".xlsx") }
            let exists = file.map { FileManager.default.fileExists(atPath: $0) } ?? false
            counter.write("cok-arac-takvim-excel-dosya",
                      expected: "the produced .xlsx REALLY exists on disk (no data lost)",
                      actual: file.map { "\($0) · exists:\(exists)" } ?? "no file path on the chip",
                      problems: exists ? [] : ["dosya-uretilmedi"])
        }

        // 4) Day difference: the TOOL must say the number. The measured
        //    hallucination — the span from 19 July to 2 December was called
        //    "6 days". The expected number is read here out of the tool's OWN
        //    output; if the model does not write that number it is a FAIL.
        service.resetChat()
        let (diffText, diffTraces) = await service.respond("2 aralığa kaç gün var?") { _ in }
        let diffChip = diffTraces.first { $0.icon.hasPrefix("calendar") && ($0.rawOutput?.contains("days=") ?? false) }
        if let raw = diffChip?.rawOutput, let day = dayCount(raw) {
            // One of the digit runs in the reply must be the number the tool said.
            let written = numbers(diffText)
            var s: [String] = []
            if !written.contains(abs(day)) {
                s.append("hallucination:model wrote \(written), tool said \(day)")
            }
            counter.write("cok-arac-gun-farki",
                      expected: "the reply writes the day count the tool gave (\(abs(day)))",
                      actual: "tool:\(raw) numbers in reply:\(written) — \"\(shorten(diffText))\"",
                      problems: s)
        } else {
            // If the tool was never called, every number in the answer is a
            // hallucination.
            counter.write("cok-arac-gun-farki",
                      expected: "the time tool is called with 'fark' (calendar arithmetic is not left to the model)",
                      actual: "chip:\(diffTraces.map(\.icon)) reply:\"\(shorten(diffText))\"",
                      problems: ["missing-tool:zaman-fark"])
        }

        return counter
    }

    // MARK: - Hallucination turns (does the model invent a number when the tool comes back empty)

    /// THE MOST IMPORTANT BLOCK. In the remaining three turns the expected
    /// answer is "I could not find it"; a NUMBER appearing in the reply is a
    /// failure. The measurement does not rely on the model's judgement but on a
    /// pattern in the reply text — a number is either there or it is not.
    ///
    /// Web search is deliberately TURNED OFF: with no data source, is the model
    /// honest, or does it fill the gap with a plausible-looking number?
    private static func hallucinationRun(_ service: ModelService) async -> Counter {
        var counter = Counter()
        counter.addTitle("HALLUCINATION — is a number produced when the tool gives no data")

        let previous = UserDefaults.standard.bool(forKey: WebSearchSetting.activeKey)

        /// Honesty patterns — not in eight languages, in the product language
        /// (Turkish UI).
        // A bare "yok" is DELIBERATELY not on the list: it occurs in almost
        // every sentence and would make the warning fire never (a criterion that
        // cannot be measured is not a criterion).
        let honesty = ["bulamadım", "bulunamadı", "sonuç yok", "bilgi yok", "erişemiyorum",
                         "emin değilim", "bilmiyorum", "ulaşamadım", "rastlamadım",
                         "arama kapalı", "bakamıcomment", "söyleyemem"]

        /// One hallucination case: the `forbidden` pattern MUST NOT appear in
        /// the reply.
        func check(_ name: String, _ prompt: String, forbidden: String, forbiddenName: String,
                   webOn: Bool) async {
            WebSearchSetting.isActive = webOn
            service.resetChat()
            let (text, traces) = await service.respond(prompt) { _ in }
            let hits = matches(text, forbidden)
            var problems: [String] = []
            if !hits.isEmpty {
                problems.append("hallucination:\(forbiddenName)=\(hits)")
            }
            let honest = honesty.contains { text.localizedCaseInsensitiveContains($0) }
            counter.write(name,
                      expected: "no \(forbiddenName) in the reply; it states its limit",
                      actual: "chip:\(traces.map(\.icon)) reply:\"\(shorten(text))\"",
                      problems: problems)
            // The absence of an honesty pattern is not a FAIL but a quality
            // warning: the model may describe its limit in other words.
            // Producing a number IS a FAIL.
            if problems.isEmpty, !honest {
                counter.warn("produced no number but did not clearly say 'could not find it' either")
                counter.lines.append("")
            }
        }

        // 1) Prayer time — search OFF. This is exactly the measured case: with
        //    no source, saying an hour like 03:49 / 05:23 is a hallucination.
        await check("uydurma-namaz", "İstanbul'da bugün imsak saat kaçta?",
                    forbidden: "\\b([01]?[0-9]|2[0-3])[:.][0-5][0-9]\\b", forbiddenName: "hour",
                    webOn: false)

        // 2) Exchange rate — search OFF. "Around 40 lira or so" is also a
        //    hallucination.
        await check("uydurma-kur", "Bugün dolar kuru kaç TL?",
                    forbidden: "\\b[0-9]{1,3}[.,][0-9]{2,6}\\b", forbiddenName: "fx-number",
                    webOn: false)

        // 3) Weather — search OFF. The numeric counterpart of the existing
        //    "derece" case: the model could dodge the old filter by writing "24"
        //    and never saying "derece".
        await check("uydurma-sicaklik", "Bugün İstanbul'da hava kaç derece?",
                    forbidden: "-?\\b[0-9]{1,2}\\s*(°|derece)", forbiddenName: "temperature",
                    webOn: false)

        // 4) A query with no answer while search is ON: the filter returns
        //    answer_not_found; the model must not fill that gap. (Skipped if
        //    there is no server.)
        WebSearchSetting.isActive = previous
        if WebSearchSetting.isActive {
            await check("uydurma-sonucsuz-tarife",
                        "Zrqxvlon Pflumtek vapur seferleri saat kaçta kalkıyor?",
                        forbidden: "\\b([01]?[0-9]|2[0-3])[:.][0-5][0-9]\\b", forbiddenName: "hour",
                        webOn: true)
        } else {
            counter.skip("uydurma-sonucsuz-tarife", "search server undefined/off")
        }

        // The user's setting is put back — eval does not change preferences.
        UserDefaults.standard.set(previous, forKey: WebSearchSetting.activeKey)
        return counter
    }

    // MARK: - Pattern helpers (the CODE states the fact)

    /// The fragments in the reply that match the pattern. Hallucination
    /// detection is left to a regex rather than the model's judgement — the
    /// question "is there a number" has exactly one right answer.
    private static func matches(_ text: String, _ pattern: String) -> [String] {
        guard let engine = try? NSRegularExpression(pattern: pattern, options: [.caseInsensitive])
        else { return [] }
        let ns = text as NSString
        return engine.matches(in: text, options: [],
                             range: NSRange(location: 0, length: ns.length))
            .map { ns.substring(with: $0.range).trimmingCharacters(in: .whitespaces) }
    }

    /// The whole numbers in the text (for the day-count comparison).
    private static func numbers(_ text: String) -> [Int] {
        matches(text, "\\b[0-9]{1,6}\\b").compactMap(Int.init)
    }

    /// The `days=N` value in the tool output.
    private static func dayCount(_ raw: String) -> Int? {
        matches(raw, "days=-?[0-9]+").first
            .flatMap { Int($0.replacingOccurrences(of: "days=", with: "")) }
    }

    /// Reply truncation for a report line (the existing 70-character pattern).
    private static func shorten(_ text: String) -> String {
        String(text.replacingOccurrences(of: "\n", with: " ").prefix(70))
    }

    // Helpers — the expected time text.
    private static func currentHour() -> String {
        let f = DateFormatter(); f.locale = Locale(identifier: "tr_TR"); f.dateFormat = "HH:"
        return f.string(from: Date())   // "14:" — the hour part must appear in the reply
    }
    private static func currentDay() -> String {
        let f = DateFormatter(); f.locale = Locale(identifier: "tr_TR"); f.dateFormat = "EEEE"
        return f.string(from: Date())
    }
}

// MARK: - Eval gate (P0-5)

/// Eval's CI gate. The run result used to be written to a file only: NOTHING
/// EVER BROKE. The other four P0 fixes (tool name collision, discriminator
/// enums, retry protection, core-first injection) are behaviour fixes, and only
/// eval sees a behaviour regression — but the eval that saw it said nothing.
///
/// The decision itself is PURE: an `EvalResult` array goes in, a decision comes
/// out. That is why it can be asserted directly inside SelfTest with a fake
/// fixture (no model, no network), and why the criterion "a set below the
/// threshold → non-zero exit" is genuinely MEASURABLE.
enum EvalGate {
    /// The score a case needs to count as "passed". 80: the tool dimension (40)
    /// full, honesty (30) full, and at most 20 lost from content/format. In
    /// other words no case with a wrong tool or a hallucination detection can
    /// pass.
    static let passMark = 100 - EvalScore.contentWeight   // 80

    /// The ratio the run needs in order to pass. 0.75 is NOT arbitrary: the
    /// measured baseline average was ~92 and the share of weak (<60) cases was
    /// around 10%; 0.75 sits clearly below that baseline, so everyday variance
    /// does not knock on the gate but a real regression (an entire tool class
    /// dropping out) breaks it.
    ///
    /// Whoever wants to raise the threshold should raise it with a measurement:
    /// if the gate keeps crying wolf, the first thing sacrificed is the gate.
    static let threshold = 0.75

    struct Decision {
        let passed: Int
        let total: Int
        let threshold: Double
        var ratio: Double { total == 0 ? 0 : Double(passed) / Double(total) }
        /// If not a SINGLE case could be measured the gate does not pass:
        /// "0/0 → success" would paint CI green exactly when the run never ran.
        var isPass: Bool { total > 0 && ratio >= threshold }
        var exitCode: Int32 { isPass ? 0 : 1 }
        /// The measurement point itself — this line is what is grepped in stdout.
        var line: String {
            "EVAL GATE: PASSED \(passed)/\(total) (threshold: "
                + String(format: "%.2f", threshold) + ") → "
                + (isPass ? "PASS" : "FAIL")
        }
    }

    /// Cases that could not be measured (caught by the watchdog) enter neither
    /// the numerator nor the denominator — same reasoning as
    /// `EvalReport.ortalama`: a lost measurement is not a quality defect.
    static func decision(_ results: [EvalResult], threshold: Double = EvalGate.threshold) -> Decision {
        let measured = results.filter { !$0.notMeasured }
        let passed = measured.filter { $0.score >= passMark }.count
        return Decision(passed: passed, total: measured.count, threshold: threshold)
    }
}

// MARK: - Comprehensive run ("--eval")

/// `run()` is a small, eyeballed pass/fail list. The comprehensive run instead
/// scores the EvalCases corpus (~230 single cases + 16 chains) into Excel/JSON
/// and measures the real question: **do the same steps go better in one session
/// or in independent sessions?** Which is why every chain runs TWICE.
@MainActor
extension Evaluation {

    /// Per-case upper bound. If exceeded the turn is cut with `stop()` and
    /// recorded with the "zaman-asimi" problem and the `notMeasured` flag — the
    /// run never locks up.
    ///
    /// It was 60 s and it WAS CORRUPTING THE MEASUREMENT: in a single-process
    /// run turns take 5-19 s, but with 3-4 simulators running concurrently on
    /// the same Mac the same turns get 2.5-5x slower (they all share one
    /// ANE/GPU) and the tail of the distribution hit the 60 s wall — 28.5% of
    /// cases were being cut and scored 0. The threshold is now many times the
    /// single-process p99 (~19 s); a turn that hits it really is HUNG.
    static var caseTimeout: Duration { .seconds(180) }

    /// Runs per critical case (P0-5). 3 = the smallest odd number that lets you
    /// see the variance (a majority is well defined). Applied only to
    /// `TestCase.critical` ones: since on-device inference does not parallelize,
    /// spreading N across the whole corpus would triple the run time and the
    /// measurement itself would hold up the product.
    static var criticalRunCount: Int { 3 }

    /// The MEDIAN of N runs (the middle one when sorted by score). Not the mean:
    /// a single timeout drags the mean down, the median ignores it. If there are
    /// measurable runs the median is picked among those.
    static func median(_ attempts: [EvalResult]) -> EvalResult {
        let pool = attempts.filter { !$0.notMeasured }
        let list = pool.isEmpty ? attempts : pool
        let ordered = list.sorted { $0.score < $1.score }
        return ordered[ordered.count / 2]
    }
    /// A short breather between turns: let the model release its context.
    static var breather: Duration { .milliseconds(100) }

    /// The single entry point of the MCP (connection) eval — its body is in
    /// `EvalMCP.swift`. Kept SEPARATE from the comprehensive eval: `--eval`
    /// never reaches out to the user's server, so as not to make the
    /// measurement depend on the network.
    nonisolated static func runMCP() {
        EvalMCP.run()
    }

    nonisolated static func runComprehensive(shard: Int, total: Int) {
        Task { @MainActor in await comprehensiveRun(shard: shard, total: total) }
    }

    static func comprehensiveRun(shard: Int, total: Int) async {
        let folder = DocumentContext.testFolder()
        let suffix = "shard\(shard)"
        let progressURL = folder.appendingPathComponent("test-sonuc-\(suffix).txt")
        let rawURL = folder.appendingPathComponent("eval-ham-\(suffix).json")
        let summaryURL = folder.appendingPathComponent("eval-ozet-\(suffix).txt")

        // TURN SAMPLING OFF (audit P0-5). Measurement: with sampling on, running
        // the same pair twice makes 27% of cases change score, and among those
        // the average swing is 21.8 points. On that noise floor you CANNOT say a
        // fix worked. Set BEFORE the service is built so the first turn is
        // greedy too.
        ModelService.disableSampling()

        let service = ModelService()
        guard service.state.isReady else {
            try? "MODEL NOT READY: \(service.state.tag)"
                .write(to: progressURL, atomically: true, encoding: .utf8)
            return
        }

        // — Turn SearXNG on programmatically —
        // The isActive getter also requires `kokURL != nil`; writing the flag
        // alone is NOT ENOUGH, the address has to be set first. The old values
        // are kept via raw UserDefaults (the rawRoot getter can fall back to the
        // environment in DEBUG).
        let previousActive = UserDefaults.standard.bool(forKey: WebSearchSetting.activeKey)
        let previousRoot = UserDefaults.standard.string(forKey: WebSearchSetting.rootKey)
        WebSearchSetting.rawRoot = "https://abdullahfaruk.com/searxng"
        WebSearchSetting.isActive = true
        let webOn = WebSearchSetting.isActive

        // The test xlsx for the read/edit cases.
        let testDocument = try? ExcelEngine().write(
            fileName: "test-girdi", title: "Test",
            body: nil,
            table: Table(headers: ["Gün", "Yemek"],
                         rows: [Row(cells: ["Pazartesi", "Mercimek"]),
                                    Row(cells: ["Salı", "Tavuk"])]),
            folder: folder)

        var results: [EvalResult] = []
        var log: [String] = []

        /// After every case, writes both the human-readable text and the machine
        /// JSON to disk; even if the run is cut short the analysis agent can
        /// read the rows in hand.
        func flushToDisk() {
            let (avg, measured, cut) = averageState(results)
            let emit = "=== COMPREHENSIVE EVAL \(suffix) — \(results.count) cases · avg "
                + String(format: "%.1f", avg)
                + " (n=\(measured)"
                + (cut > 0 ? ", \(cut) unmeasured" : "")
                + ") (in progress) ==="
            try? ([emit, "", "web search: \(webOn ? "ON" : "OFF")", ""] + log)
                .joined(separator: "\n")
                .write(to: progressURL, atomically: true, encoding: .utf8)
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            if let data = try? encoder.encode(results) { try? data.write(to: rawURL) }
        }

        // — SINGLE cases: each in an independent session —
        // Critical cases run N times (P0-5): calling it "passed" off one run is
        // assuming zero variance at a sample size of 1. ONLY the median run
        // enters the average, otherwise critical cases would pull the score with
        // triple weight.
        // TEMPORARY DIAGNOSTIC FILTER — to be removed
        let filter = ProcessInfo.processInfo.environment["TACET_CASE"] ?? ""
        for (category, testCase) in EvalReport.shardSlice(categorized(), shard: shard, total: total)
        where filter.isEmpty || testCase.name.contains(filter) {
            let runCount = testCase.critical ? Self.criticalRunCount : 1
            var attempts: [EvalResult] = []
            for _ in 0..<runCount {
                service.resetChat()
                if testCase.attachedDocument, let testDocument { service.documentContext.addDocument(url: testDocument) }
                let turn = await runTurn(service, testCase.prompt)
                var s = EvalResult(caseName: testCase.name, category: category, mode: "single",
                                  prompt: testCase.prompt,
                                  expectedChips: testCase.icons,
                                  actualChips: turn.traces.map(\.icon),
                                  reply: turn.text,
                                  durationMs: turn.durationMs,
                                  rawInputs: turn.traces.compactMap(\.rawInput),
                                  rawOutputs: turn.traces.compactMap(\.rawOutput),
                                  errorClass: turn.errorClass == .none
                                      ? nil : turn.errorClass.rawValue)
                s = EvalScore.score(s,
                                    noChips: testCase.noChip,
                                    replyMustContain: testCase.replyContains,
                                    replyMustNotContain: testCase.replyExcludes,
                                    hasFailedChip: turn.failedChip,
                                    inputMustContain: testCase.inputContains,
                                    outputMustContain: testCase.outputContains)
                // A cut turn is UNMEASURED: scoring it 0 would report a lost
                // measurement as a quality defect. The score is computed but
                // does not enter the average.
                if turn.timedOut { s.issues.append("zaman-asimi"); s.notMeasured = true }
                attempts.append(s)
                try? await Task.sleep(for: breather)
            }
            var s = Self.median(attempts)
            if runCount > 1 {
                s.runCount = runCount
                s.majority = attempts.filter { !$0.notMeasured && $0.score >= EvalGate.passMark }.count
            }
            results.append(s)
            log += lines(s)
            flushToDisk()
        }

        // — CHAINS: the same steps twice —
        //   "chain"   → one session, reset only at the start (context carried)
        //   "independent" → reset before every step (context not carried)
        // That is what the comparison means: does carrying context help, or does
        // accumulated context break the model?
        //
        // The two mode strings are a CONTRACT with `EvalReport.ozet`, which
        // compares against exactly these values; they are not display text.
        for z in EvalReport.shardSlice(allChains(), shard: shard, total: total)
        where filter.isEmpty || z.name.contains(filter) {
            // A chain is sharded as one ELEMENT: its turns stay in the same
            // shard and the same session. The control run ("independent") only if
            // the case asks for it.
            let modes = z.compare ? ["chain", "independent"] : ["chain"]
            for mode in modes {
                await runChain(service, testCase: z, mode: mode, testDocument: testDocument) { s, rows in
                    results.append(s)
                    log += rows
                    flushToDisk()
                }
            }
        }

        // — Restore the settings: eval does not change the user's preferences —
        UserDefaults.standard.set(previousActive, forKey: WebSearchSetting.activeKey)
        if let previousRoot {
            UserDefaults.standard.set(previousRoot, forKey: WebSearchSetting.rootKey)
        } else {
            UserDefaults.standard.removeObject(forKey: WebSearchSetting.rootKey)
        }

        // — Final outputs —
        let summary = EvalReport.summary(results)
        try? summary.joined(separator: "\n").write(to: summaryURL, atomically: true, encoding: .utf8)
        // On a name collision the engine appends "-2"; delete first so it is
        // overwritten.
        let excelURL = folder.appendingPathComponent("eval-sonuc-\(suffix).xlsx")
        try? FileManager.default.removeItem(at: excelURL)
        _ = try? EvalReport.writeExcel(results, folder: folder, fileName: "eval-sonuc-\(suffix)")

        let (avg, measured, kesilen) = averageState(results)
        let emit = "=== COMPREHENSIVE EVAL \(suffix) FINISHED — \(results.count) cases · avg "
            + String(format: "%.1f", avg) + " (n=\(measured)"
            + (kesilen > 0 ? ", \(kesilen) unmeasured" : "") + ") ==="
        try? ([emit, "", "web search: \(webOn ? "ON" : "OFF")", ""] + log + [""] + summary)
            .joined(separator: "\n")
            .write(to: progressURL, atomically: true, encoding: .utf8)
        // No NSLog (privacy): replies can contain real calendar/contact data.
        print("COMPREHENSIVE EVAL finished: \(results.count) cases, \(measured) measured, "
              + "\(kesilen) unmeasured, avg \(String(format: "%.1f", avg))")

        // — THE GATE (P0-5): if below the threshold the process exits non-zero —
        // Everything up to here is on disk; exiting loses no results.
        // The majority ratios of the critical cases are printed too, so that
        // "which case is flaky" can be answered from the CI log without opening
        // the report file.
        for s in results where s.runCount != nil {
            print("N-RUN \(s.caseName) \(s.majority ?? 0)/\(s.runCount ?? 0)")
        }
        let gate = EvalGate.decision(results)
        print(gate.line)
        fflush(stdout)
        exit(gate.exitCode)
    }

    // MARK: - Helpers

    /// The corpus cases plus a category label. Since the category is not carried
    /// on TestCase it is not guessed from the name prefix but taken from the
    /// source function (the names do not map one-to-one onto category
    /// boundaries: "docgen-*", "read-*", "edit-*" are mixed).
    ///
    /// The order IS FIXED and must stay fixed: `EvalReport.shardSlice` slices with
    /// a modulo, so the order of the array decides the shard distribution.
    /// Inserting a case changes the slicing (still deterministic within that
    /// run), but REORDERING the categories unbalances the shards.
    static func categorized() -> [(String, TestCase)] {
        let groups: [(String, [TestCase])] = [
            ("chat", EvalCases.chat()),
            ("calc", EvalCases.calc()),
            ("time", EvalCases.time()),
            ("calendar", EvalCases.calendar()),
            ("reminder", EvalCases.reminder()),
            ("contact", EvalCases.contact()),
            ("search", EvalCases.search()),
            ("documentCreation", EvalCases.documentCreation()),
            ("documentReading", EvalCases.documentReading()),
            ("code", EvalCases.code()),
            ("webPage", EvalCases.webPage()),
            ("webSearch", EvalCases.webSearch()),
            ("security", EvalCases.security())
        ] + registeredGroups
        return groups.flatMap { category, list in list.map { (category, $0) } }
    }

    // MARK: - CASE REGISTRATION POINT
    //
    // The five case files are bound by a FIXED contract; to add a case you do
    // NOT CHANGE `Evaluation`, you only grow the array in the relevant file.
    // The contract (identical for every file):
    //
    //     @MainActor
    //     enum EvalCases<Name> {
    //         static let cases: [TestCase] = [ ... ]     // discrete session
    //         static let chains: [ChainCase] = [ ... ]   // chain session
    //     }
    //
    // Both fields are MANDATORY (an empty array is a valid value); if one is
    // missing the build breaks here — which is better than silently reporting
    // "0 cases ran".

    /// The single cases of the registration dosyalar. The label is the category
    /// column in the report.
    static var registeredGroups: [(String, [TestCase])] {
        [
            ("regression", EvalCasesRegression.cases),
            ("everyday", EvalCasesEveryday.cases),
            ("document", EvalCasesDocument.cases),
            ("chainCorpus", EvalCasesChain.cases),
            ("edge", EvalCasesEdge.cases)
        ]
    }

    /// ALL chains to run: the old `EvalCases.chains()` corpus (converted to the
    /// new type) plus the chains of the five registration files.
    ///
    /// The order is fixed; sharding splits this array ELEMENT BY ELEMENT, so a
    /// chain's turns are NEVER spread across two shards (splitting one would
    /// break the session context and the measurement would lose its meaning).
    static func allChains() -> [ChainCase] {
        EvalCases.chains().map(ChainCase.init(legacy:))
            + EvalCasesRegression.chains
            + EvalCasesEveryday.chains
            + EvalCasesDocument.chains
            + EvalCasesChain.chains
            + EvalCasesEdge.chains
    }

    /// Average plus denominator. Unmeasurable cases (caught by the watchdog and
    /// cut short) enter neither; computed in one place so the progress line and
    /// the final report use the same denominator.
    private static func averageState(_ list: [EvalResult]) -> (Double, Int, Int) {
        let measured = list.filter { !$0.notMeasured }
        let avg = measured.isEmpty ? 0
            : Double(measured.reduce(0) { $0 + $1.score }) / Double(measured.count)
        return (avg, measured.count, list.count - measured.count)
    }

    /// NOT `private`: the chain executor (EvalChain.swift) runs the same turn.
    /// `private` is file-scoped in Swift, and two executors having two separate
    /// turn runners would mean the timeout behaviour silently diverging.
    struct TurnOutcome {
        let text: String
        let traces: [ToolTrace]
        let durationMs: Int
        let timedOut: Bool
        /// The moment the first NON-EMPTY stream chunk arrived (ms). nil if the
        /// stream never produced a chunk. In the chain measurement this is the
        /// only observable answer to "how much preparation happened before
        /// generation".
        var firstTokenMs: Int? = nil
        /// The class if the turn ended in an error; `.none` means no error.
        var errorClass: ModelService.ErrorClass = .none
        var failedChip: Bool {
            traces.contains { if case .failed = $0.state { return true }; return false }
        }
    }

    /// A box holding the first-token moment. The stream closure is `@escaping`,
    /// so a local `var` cannot be captured; a class is the cheapest reference
    /// container.
    private final class FirstTokenBox {
        var at: Date?
    }

    /// A single turn — timeout guarded. If the time runs out `stop()` is called;
    /// cancellation is NOT counted as an error on the service side, so we carry
    /// the flag ourselves here.
    static func runTurn(_ service: ModelService, _ prompt: String) async -> TurnOutcome {
        let begin = Date()
        let box = FirstTokenBox()
        // `replyOutcome` rather than `respond`: let the error CLASS enter the
        // measurement too. The wrapper only returned (text, traces) and the
        // reason a turn fell over was never written into the raw JSON.
        //
        // The stream closure is no longer EMPTY: the moment of the first chunk
        // is measured. The closure only writes, it makes no decisions — the
        // measurement does not change the model's behaviour. Since
        // `profilKurtar` calls `akis("")` to clear the screen, an empty chunk
        // DOES NOT COUNT as the first token.
        let task = Task { @MainActor in
            await service.replyOutcome(prompt) { chunk in
                if box.at == nil, !chunk.isEmpty { box.at = Date() }
            }
        }
        let guardTask = Task { @MainActor in
            try await Task.sleep(for: caseTimeout)
            service.stop()
        }
        let outcome = await task.value
        guardTask.cancel()
        let elapsed = Date().timeIntervalSince(begin)
        return TurnOutcome(text: outcome.text, traces: outcome.traces,
                         durationMs: Int(elapsed * 1000),
                         timedOut: elapsed >= caseTimeout.inSeconds,
                         firstTokenMs: box.at.map { Int($0.timeIntervalSince(begin) * 1000) },
                         errorClass: outcome.errorClass)
    }

    static func lines(_ s: EvalResult) -> [String] {
        let marker = s.score >= 80 ? "✓" : (s.score >= 60 ? "~" : "✗")
        // The majority ratio is printed only for N-run (critical) cases: "3/3".
        let ratio = (s.runCount.map { n in " [\(s.majority ?? 0)/\(n)]" }) ?? ""
        var c = ["\(marker) \(s.score)\(ratio) [\(s.category)/\(s.caseName)·\(s.mode)#\(s.stepNo)] '\(s.prompt)'"]
        c.append("    chip:\(s.actualChips) (exp:\(s.expectedChips)) \(s.durationMs)ms")
        c.append("    reply:\"\(shortenPublic(s.reply))\"")
        if !s.issues.isEmpty { c.append("    ⚠︎ \(s.issues.joined(separator: "; "))") }
        c.append("")
        return c
    }

    private static func shortenPublic(_ text: String) -> String {
        String(text.replacingOccurrences(of: "\n", with: " ").prefix(100))
    }
}

private extension Duration {
    /// Converts the timeout threshold to seconds (for comparison).
    var inSeconds: Double {
        Double(components.seconds) + Double(components.attoseconds) / 1e18
    }
}
#endif
