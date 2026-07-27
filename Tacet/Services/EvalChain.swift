//
//  EvalChain.swift
//  Tacet
//
//  "Chain session" eval infrastructure.
//
//  A discrete (single) run executes every case in a CLEAN session and for that
//  very reason can never see one class of bug: corruption of the state that
//  accumulates INSIDE a session. The context budget filling up and triggering
//  summarization, the session being rebuilt on a profile/signature change and
//  dropping what the previous turn knew, a reference to a document produced in
//  an earlier turn ("make that a pdf"), `DataStore` refs hitting the LRU cap
//  and vanishing between turns, memory extraction firing off accumulated
//  messages, the tool budget (code-spec §5.4) not resetting at the turn
//  boundary, `akanMetin` racing across turns — all of these arise ONLY in
//  back-to-back turns.
//
//  This file offers two things:
//   1. `ChainCase` / `ChainTurn` — the data types case authors will use. The
//      expectation vocabulary is IDENTICAL to `TestCase`; there is NO second
//      scoring language, scoring goes through `EvalScore.score`.
//   2. `Evaluation.runChain` — the executor that builds ONE session per case,
//      runs the turns in order, scores each turn separately and records the
//      chain-specific measurements.
//
//  Scoring semantics DO NOT CHANGE: pass mark `EvalGate.passMark` (80), run
//  threshold `EvalGate.threshold` (0.75), same hallucination detector.
//

#if DEBUG
import Foundation
import SwiftData

// MARK: - Case types

/// ONE turn of a chain. The field names and meanings are kept identical to
/// `TestCase` on purpose: the agent writing cases should not have to translate
/// mentally between two types, and no second path into the scorer
/// (`EvalScore.score`) should be opened.
struct ChainTurn {
    /// The prompt the user typed in this turn.
    let prompt: String
    /// Expected chip icon PREFIXES — all of them must appear. (The `doc` prefix
    /// also matches `doc.text.image`; a case that wants the format distinction
    /// must spell out the full name.)
    var icons: [String] = []
    /// No tool at all should be called in this turn (clarification / chat turns).
    var noChip = false
    /// Fragment that MUST appear in the reply.
    var replyContains: String? = nil
    /// Fragment that MUST NOT appear in the reply — hallucination detection
    /// (the heaviest penalty).
    var replyExcludes: String? = nil
    /// Fragments that must appear in the tool ARGUMENT. The chip icon tells you
    /// the right tool was called, it does NOT tell you the argument was right.
    var inputContains: [String] = []
    /// Fragments that must appear in the tool OUTPUT. The number has to come
    /// from the tool; the model writing it in the reply is no proof the tool
    /// ran correctly.
    var outputContains: [String] = []
}

/// A scenario run back-to-back in a single session.
///
/// CAUTION: `resetChat()` is NOT CALLED between turns — the state accumulating
/// in the session is precisely what is being measured. A chain's turns cannot
/// be split; sharding distributes a chain as a SINGLE ELEMENT (see
/// `EvalReport.shardSlice`).
struct ChainCase {
    /// The name shown in report rows. It is expected to start with the `zincir-`
    /// prefix so it does not get confused with a category (not mandatory, but
    /// it helps report readability).
    let name: String
    /// WHAT this chain measures — one sentence. Does not enter the report, it
    /// is for whoever reads the code.
    let description: String
    /// The turns, in order.
    let turns: [ChainTurn]
    /// Whether to attach the test document at the start of the chain
    /// (read/edit scenarios).
    var attachedDocument = false
    /// Whether to also perform the control run ("independent": reset before every
    /// turn).
    ///
    /// ON BY DEFAULT, because a chain score cannot be interpreted ON ITS OWN:
    /// is a turn's low score a flaw in carrying context, or was that prompt
    /// simply hard? The control run separates the two (the chain-vs-independent
    /// section of `EvalReport.ozet` is computed from that pair). Turn it off
    /// only for chains where the control is MEANINGLESS — e.g. if turn N's
    /// prompt is grammatically dependent on turn N-1's output ("move it to
    /// 10:00"), the independent run measures nothing and only burns time.
    var compare = true
    /// AFTER WHICH turn memory extraction should fire (1-based); nil = never.
    ///
    /// In production, extraction runs at the moment of `resetChat()`, i.e. it
    /// NEVER runs inside a single session. The chain imitates this: after the
    /// given turn the messages up to that point are written into an in-memory
    /// `Chat`, `MemoryService` is run, and the resulting notes are put into
    /// `MemoryStore` — so that LATER turns can see the note. The production
    /// store is restored at the end of the chain.
    var memoryExtractTurn: Int? = nil
}

extension ChainCase {
    /// The name fragment that marks a legacy chain as needing the test document
    /// attached. See `init(legacy:)` — this is the whole of that coupling.
    static let attachedDocumentNameFragment = "doc-read"

    /// Ports the old `EvalCases.EvalChain` corpus onto the new type.
    ///
    /// The corpus is NOT BEING DELETED: those 16 chains are already-measured
    /// data, and the price of moving to a single executor should be a
    /// converter, not copying them by hand. If `expected` is shorter than the
    /// step count, the remaining turns count as having no chip expectation
    /// (exactly the old runner's behaviour).
    init(legacy: EvalCases.EvalChain) {
        self.name = legacy.name
        self.description = legacy.description
        self.turns = legacy.steps.enumerated().map { i, prompt in
            ChainTurn(prompt: prompt,
                      icons: i < legacy.expected.count ? legacy.expected[i] : [])
        }
        // The old runner opened the attached document BY LOOKING AT THE NAME;
        // to preserve the behaviour the same rule lives here in one place.
        //
        // THIS COUPLING IS LOAD-BEARING: renaming the chain corpus without
        // touching this fragment silently turns every read/edit chain into a
        // chain with no document, and the chain still reports a score. The
        // fragment therefore lives in ONE constant, and `SelfTestCases`
        // asserts that at least one legacy chain still matches it.
        self.attachedDocument = legacy.name.contains(Self.attachedDocumentNameFragment)
        self.compare = true
        self.memoryExtractTurn = nil
    }
}

// MARK: - Chain-specific measurements

/// The measurements of a chain turn that are INDEPENDENT of the score.
///
/// These are regression signals for the mechanisms that change on turn 1
/// (summarization cache, session splitting, the `DataStore` LRU cap, the code
/// attempt counter, streaming performance). They DO NOT ENTER the score — a
/// mechanism that is not measured only gets noticed months after it breaks,
/// but tying the measurement to the score would be silently moving the bar.
struct ChainMeasurement {
    var turnNo: Int
    /// Whether any tool was called in the turn, and how many.
    var toolCount: Int
    /// The profile label read off the timeline ("everyday profile",
    /// "document profile" …).
    var profile: String
    /// The profile changed relative to the PREVIOUS turn — meaning the session
    /// was rebuilt (`ModelService.replyOutcome` calls `oturumKur` on a profile
    /// change).
    var sessionRebuilt: Bool
    /// A SECOND routing step within the same turn — in-turn profile recovery
    /// (P1-2) ran, i.e. the session was set up once more mid-turn.
    var inTurnRecovery: Bool
    /// SUSPICION of summarization. Not a definite signal (see the note at the
    /// end of the file): if no tool was called and the first token exceeds
    /// `summarySuspicionThresholdMs`, the elapsed time is preparation rather
    /// than generation, and the only long preparation step is the LLM call in
    /// `ozetle()`.
    var summarySuspected: Bool
    /// The moment the first stream chunk arrived (ms). nil if the stream never
    /// produced a chunk.
    var firstTokenMs: Int?
    var durationMs: Int
    /// `calisilabilirBelge` at the end of the turn — whether the "make that a
    /// pdf" reference survived is read from here.
    var documentName: String?
    /// The `DataStore` ref count at the end of the turn — if the new LRU cap
    /// (24 records) is dropping refs mid-chain, it shows up here.
    var refCount: Int
    /// The real number of code executions in the turn (code-spec §5.4 cap is 2).
    /// If it does not reset at the turn boundary, a second code turn is
    /// silently rejected.
    var codeAttempts: Int
    /// If memory extraction ran after this turn, the number of notes produced.
    var memoryNote: Int?
    var timedOut: Bool

    /// One-line machine-readable summary.
    ///
    /// `EvalResult` has NO FIELD for chain measurements and that file is not
    /// owned by this turn; the measurement is therefore appended to
    /// `rawOutputs` AFTER SCORING (so it cannot affect the score) and can be
    /// searched for under this prefix in the raw JSON.
    var line: String {
        var p = ["ZINCIR-OLCUM",
                 "tur=\(turnNo)",
                 "arac=\(toolCount)",
                 "profil=\(profile.isEmpty ? "?" : profile)",
                 "oturum-kuruldu=\(sessionRebuilt ? 1 : 0)",
                 "tur-ici-kurtarma=\(inTurnRecovery ? 1 : 0)",
                 "ozetleme-suphesi=\(summarySuspected ? 1 : 0)",
                 "ilk-token-ms=\(firstTokenMs.map(String.init) ?? "-")",
                 "sure-ms=\(durationMs)",
                 "belge=\(documentName ?? "-")",
                 "ref=\(refCount)",
                 "kod-deneme=\(codeAttempts)"]
        if let memoryNote { p.append("hafiza-notu=\(memoryNote)") }
        if timedOut { p.append("takildi=1") }
        return p.joined(separator: "|")
    }
}

// MARK: - Executor

@MainActor
extension Evaluation {

    /// In a turn where no tool was called, the first token being delayed this
    /// long says a long preparation step ran BEFORE generation. The only long
    /// preparation step is the separate LLM call in `ozetle()` (on a short
    /// history that call is never made, the raw queue is carried instead — so a
    /// turn that crosses the threshold is already a long-session turn). 2.5 s is
    /// many times the toolless-turn first-token latency measured in a
    /// single-process run (a few hundred ms).
    static var summarySuspicionThresholdMs: Int { 2500 }

    /// Runs one chain case in a SINGLE session, turns IN ORDER.
    ///
    /// - Even if a turn falls over (timeout, empty reply, missing tool) THE
    ///   REMAINING TURNS ARE RUN: where a chain breaks is only visible if what
    ///   comes after the break is measured too. A fallen turn is marked
    ///   `notMeasured` and does not enter the average.
    /// - Every turn is handed back to the caller IMMEDIATELY via `stepDone`;
    ///   the caller writes to disk. Even if the run is cut short, the turns in
    ///   hand are in the report.
    /// - `mode` is written straight into `EvalResult.mode` and MUST NOT take any
    ///   value other than "chain" | "independent": `EvalReport.ozet` does its
    ///   comparison with those two strings.
    static func runChain(_ service: ModelService,
                         testCase: ChainCase,
                         mode: String,
                         testDocument: URL?,
                         stepDone: (EvalResult, [String]) async -> Void) async {

        // — Memory setup (only meaningful in a real chain run) —
        // Extraction works against an in-memory store; production notes are put
        // back at the end of the chain. The container is kept ALIVE for the
        // whole chain: leaving a @Model object whose context has died in the
        // store is undefined behaviour in SwiftData.
        let needsMemory = testCase.memoryExtractTurn != nil && mode == "chain"
        let productionNotes = MemoryStore.notes
        var containers: [ModelContainer] = []
        var record: ModelContext?
        var chat: Chat?
        if needsMemory {
            let schema = Schema(versionedSchema: SchemaV1.self)
            let config = ModelConfiguration(schema: schema, isStoredInMemoryOnly: true)
            if let container = try? ModelContainer(for: schema, configurations: config) {
                containers.append(container)
                let k = container.mainContext
                let s = Chat()
                k.insert(s)
                try? k.save()
                record = k
                chat = s
            }
        }

        /// (Re)builds the session — in a chain only at the start, in the
        /// independent mode on every turn.
        func setUpSession() {
            service.resetChat()
            if testCase.attachedDocument, let testDocument {
                service.documentContext.addDocument(url: testDocument)
            }
        }

        if mode != "independent" { setUpSession() }

        var previousProfile = ""
        for (i, t) in testCase.turns.enumerated() {
            let turnNo = i + 1
            if mode == "independent" {
                setUpSession()
                // In the independent run there is no such thing as a "previous
                // turn"; the reference is reset so the profile-change
                // measurement does not become meaningless.
                previousProfile = ""
            }

            let turn = await runTurn(service, t.prompt)

            // — Session signals read off the timeline —
            // `timeline.steps` stays put until the next turn; reading it right
            // after the turn ends is safe. The profile name arrives in the form
            // "<routed> · X".
            let routings = service.timeline.steps
                .filter { $0.kind == .routing }
                .map(\.text)
            let profile = routings.last.map(Self.profileLabel) ?? ""
            let rebuilt = !previousProfile.isEmpty && !profile.isEmpty && profile != previousProfile
            if !profile.isEmpty { previousProfile = profile }

            let firstToken = turn.firstTokenMs
            let summarySuspected = turn.traces.isEmpty
                && (firstToken ?? 0) >= summarySuspicionThresholdMs

            var measure = ChainMeasurement(
                turnNo: turnNo,
                toolCount: turn.traces.count,
                profile: profile,
                sessionRebuilt: rebuilt,
                inTurnRecovery: routings.count > 1,
                summarySuspected: summarySuspected,
                firstTokenMs: firstToken,
                durationMs: turn.durationMs,
                documentName: service.documentContext.runnableDocument?.name,
                refCount: service.dataStore.refKeys.count,
                codeAttempts: service.codeState.attempt,
                memoryNote: nil,
                timedOut: turn.timedOut)

            // — Memory: accumulate messages, extract after the requested turn —
            if let record, let chat {
                let questionMessage = Message(role: .user, content: t.prompt)
                questionMessage.chat = chat
                record.insert(questionMessage)
                if !turn.text.isEmpty {
                    let replyMessage = Message(role: .tacet, content: turn.text)
                    replyMessage.chat = chat
                    record.insert(replyMessage)
                }
                try? record.save()
                if testCase.memoryExtractTurn == turnNo {
                    await MemoryService().extract(chat: chat, record: record)
                    let notes = ((try? record.fetch(FetchDescriptor<MemoryNote>())) ?? [])
                        .filter { !$0.isDeleted }
                    measure.memoryNote = notes.count
                    // Later turns MUST SEE the note: in production this is the
                    // injection store, and if the imitation falls short the
                    // "note was extracted but never applied" bug can never be
                    // measured.
                    MemoryStore.reload(notes)
                }
            }

            // — Scoring: the existing mechanism, NO new language —
            var s = EvalResult(caseName: testCase.name,
                              category: "chain",
                              mode: mode,
                              stepNo: turnNo,
                              prompt: t.prompt,
                              expectedChips: t.icons,
                              actualChips: turn.traces.map(\.icon),
                              reply: turn.text,
                              durationMs: turn.durationMs,
                              rawInputs: turn.traces.compactMap(\.rawInput),
                              rawOutputs: turn.traces.compactMap(\.rawOutput),
                              errorClass: turn.errorClass == .none ? nil : turn.errorClass.rawValue)
            s = EvalScore.score(s,
                                noChips: t.noChip,
                                replyMustContain: t.replyContains,
                                replyMustNotContain: t.replyExcludes,
                                hasFailedChip: turn.failedChip,
                                inputMustContain: t.inputContains,
                                outputMustContain: t.outputContains)
            if turn.timedOut { s.issues.append("zaman-asimi"); s.notMeasured = true }

            // The measurement line is appended AFTER SCORING: `rawOutputs` is
            // the scorer's `outputMustContain` pool, and if it were added first the
            // measurement text could accidentally satisfy an expectation.
            s.rawOutputs = (s.rawOutputs ?? []) + [measure.line]

            var rows = lines(s)
            rows.insert("    ⧉ \(measure.line)", at: max(0, rows.count - 1))
            await stepDone(s, rows)
            try? await Task.sleep(for: breather)
        }

        // — Put production memory back: eval does not change the user's notes —
        if needsMemory { MemoryStore.reload(productionNotes) }
        containers.removeAll()
    }

    /// "<routed> · document profile" → "document profile". The separator is
    /// the only thing parsed, so the localized prefix does not matter.
    /// If there is no separator the text itself is returned (so the measurement
    /// does not silently go empty when the localization changes).
    static func profileLabel(_ text: String) -> String {
        guard let separator = text.range(of: "·") else {
            return text.trimmingCharacters(in: .whitespaces)
        }
        return String(text[separator.upperBound...]).trimmingCharacters(in: .whitespaces)
    }
}

// MARK: - LIMITS OF THE MEASUREMENT (written down deliberately)
//
// `summarySuspected` is a SUSPICION, not a fact. A definite measurement would
// have to come from inside `ModelService`: if the `oturumKur` and `ozetle` call
// counters were exposed (`private(set) var`), this field would turn from a
// guess into a measurement. That file is not owned by this turn, so it is
// reported as a contract loose end.
//
// In the same way `sessionRebuilt` sees ONLY profile changes: a tool signature
// change, a language change and a budget overrun also rebuild the session but
// DO NOT drop a new `yonlendirme` step onto the timeline. So this field
// produces no false positives, it produces false NEGATIVES — it should be read
// knowing that it undercounts.
#endif
