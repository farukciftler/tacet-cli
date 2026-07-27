//
//  ModelService.swift
//  Tacet
//
//  The model layer (spec §7.1, §7.2). Only the on-device SystemLanguageModel;
//  Private Cloud Compute is not used. Availability is treated like a feature flag.
//  The context window (4096) is actively managed; an overflow is recovered silently.
//

import Foundation
import Observation
import FoundationModels
import NaturalLanguage
import SwiftData

@MainActor
@Observable
final class ModelService {

    /// The session state. There is no coloured indicator on screen; the state is told
    /// in words only.
    enum State: Equatable {
        case ready                    // silent — there is nothing to tell
        case preparing             // grey · "preparing…"
        case unavailable(String)     // gri · neden

        var tag: String {
            switch self {
            case .ready: return String(localized: "On your device")
            case .preparing: return String(localized: "preparing…")
            case .unavailable(let cause):
                return cause.isEmpty ? String(localized: "not available on this device") : cause
            }
        }
        var isReady: Bool { self == .ready }
    }

    /// Why we cannot produce an answer — kept so we can tell the user "what happened +
    /// what to do". `State.tag` is the short badge text; this one picks the full sentence
    /// that lands in the chat.
    enum Block { case device, closed, preparing }

    /// The outcome of a turn. Whether it is an error IS NOT DERIVED FROM THE TEXT — the
    /// service states it explicitly. (The UI used to compare the returned text against
    /// known error strings; every time the text changed the error bubble died silently.)
    /// WHY the turn failed. A single "I could not do that right now" sentence was
    /// covering three separate faults (measured: 5 cases, all the same text, none the same
    /// cause). The class does two jobs at once: it picks the right sentence for the user
    /// (`L10n`) and it is written into the eval raw JSON — the next diagnosis happens
    /// without adding a log.
    ///
    /// IT LEAKS NO INTERNAL DETAIL: the class NAME is logged, and the user sees only the
    /// plain sentence corresponding to it (no error text, line number or model name).
    enum ErrorClass: String, Codable, Sendable {
        case none
        /// The model spoke but only an unparsed tool call was left behind.
        case emptyReply
        /// A tool failed in the turn and the model produced no text on top of it.
        case toolFailed
        /// The context window overflowed; summarising and rebuilding did not hold either.
        case contextOverflow
        /// Guardrail — unrecoverable, no retry.
        case outOfBounds
        /// The language is not supported — unrecoverable, no retry.
        case unsupportedLanguage
        /// An error after a side effect occurred; the retry was deliberately not made.
        case afterWrite
        /// Other generation errors; a retry with a fresh session did not hold either.
        case generationError
    }

    struct ReplyOutcome {
        let text: String
        let traces: [ToolTrace]
        /// Should it be drawn as an error bubble? A CANCELLATION IS NOT AN ERROR (false).
        var isError: Bool = false
        /// Can the same prompt safely be sent again? False if a side effect occurred.
        var isRetryable: Bool = false
        /// Not persistent, just a momentary status notice — it must not be written to
        /// SwiftData.
        var isTransient: Bool = false
        /// The error class — always `.none` when `isError` is false.
        var errorClass: ErrorClass = .none
    }

    private(set) var state: State = .preparing
    private(set) var block: Block? = .preparing

    /// The explanation that lands in the chat while the model is unavailable. It varies by
    /// state so that the "preparing" and "not ready on this device" messages do not
    /// contradict each other.
    var blockMessage: String {
        switch block {
        case .preparing: return L10n.modelPreparing
        case .closed:       return L10n.appleIntelligenceClosed
        case .device, nil:   return L10n.deviceNotEligible
        }
    }
    /// The tool chip is the single source of truth — tools report here, the UI reads here.
    let executor = ToolExecutor()
    /// Is the block temporary (is waiting and retrying meaningful)? Not if the device is
    /// not eligible.
    var blockIsRetryable: Bool {
        switch block {
        case .preparing, .closed: return true
        case .device, nil:            return false
        }
    }
    /// Documents shared into / produced in the chat — the document tools reach here, the
    /// UI previews from here.
    let documentContext = DocumentContext()
    /// The large-data transport channel (spec §7.3.2) — bulk data moves between tools
    /// without passing through the model.
    let dataStore = DataStore()
    /// The code-execution attempt counter (code-spec §5.4): at most 2 real runs per turn.
    /// The counter lives here rather than in the tool and is wired to `executor.turnHook`
    /// in `init` — the reset happens where the spec says, at a SINGLE point inside
    /// ToolExecutor.newTurn; there is no manual pairing at the call sites, so a forgotten
    /// path cannot make the cap session-lived.
    let codeState = CodeState()

    /// The timeline of the turn (timeline-spec §5.2). A PURE OBSERVER: no text here enters
    /// the prompt or the instruction, and no status report is ever requested from the
    /// model. With and without the recorder the model output is bit-identical (the §6
    /// acceptance criterion) — which is why every call is a one-way notification.
    let timeline = TimelineRecorder()

    /// The hook that triggers memory extraction the moment the chat is reset
    /// (memory-spec §4.1). `ModelService` has access to neither `Chat` nor `ModelContext`;
    /// the layer that knows both (ContentView) wires this up.
    var memoryTrigger: (() -> Void)?

    /// The MCP tools that enter the session in the connection profile (mcp §5.4).
    ///
    /// They are not built here: an MCP tool schema arrives from the server at run time and
    /// `setUpSession` is synchronous. The layer that knows the connection hands over the
    /// ready tools via `setConnectionTools`; if it is empty the profile is never selected.
    private var mcpTools: [MCPTool] = []
    /// The name of the selected connection — for the routing signal and the timeline row.
    private var connectionName: String = ""

    /// Prepares the selected connection's tools for the session. An empty array = no
    /// connection; the connection profile becomes unselectable at that moment (the tool is
    /// never visible to the model).
    ///
    /// If the active session is in the connection profile, changing the list invalidates
    /// the session: it is rebuilt on the next turn with the new tool set.
    func setConnectionTools(_ tools: [MCPTool], name: String = "") {
        mcpTools = tools
        connectionName = name
        if activeProfile == .connection { session = nil }
    }

    /// How many MCP tools enter the session at most (mcp §5.4: "the first 4–6 if needed").
    /// Together with Calculate + Time the 6–8 budget is preserved.
    private static let mcpToolCap = 6

    /// The pool the relevance ranking selects from. Wider than the slot, but not unlimited
    /// because schema conversion is not free (mcp §5.2).
    private static let mcpToolPool = 24

    /// The generation options (the missing half of audit P0-5).
    ///
    /// THE PRODUCTION DEFAULT DOES NOT CHANGE: `GenerationOptions()` IS today's behaviour;
    /// touching it changes the answers the user sees, and that is a separate decision. The
    /// variable exists ONLY for the eval.
    ///
    /// WHY IT IS NEEDED (measured, 20 July 2026): with the same pair and no code change,
    /// 27% of the cases changed score between two runs, the average swing among the
    /// changed ones was 21.8 points, and category averages drifted by ±15 points on their
    /// own. Against that noise floor, whether a fix worked CANNOT BE SAID from a single
    /// run — every "improvement" claim stays weak.
    ///
    /// `.greedy` is stricter than temperature 0: temperature 0 can still sample, greedy
    /// takes the most likely token at every step.
    ///
    /// BOUND TO MainActor: the reads already come from here (`consumeStream`), and the
    /// writes only from DEBUG eval paths (`Evaluation`, `SelfTestCases` — both MainActor).
    /// `nonisolated(unsafe)` kept that invariant true but HIDDEN from the compiler; the
    /// first nonisolated write would have been a silent data race.
    @MainActor static var generationOptions = GenerationOptions()

    /// Turns sampling off for eval runs. Called only from DEBUG paths; there is no caller
    /// in the production pair.
    static func disableSampling() {
        generationOptions = GenerationOptions(sampling: .greedy)
    }

    /// The raw prompt of this turn — the only input of the slot relevance ranking (P1-6).
    /// Because session setup is synchronous, the question travels through a field.
    private var lastQuestion: String = ""

    /// The MCP tools that enter the slot: chosen from the pool by RELEVANCE to the user's
    /// request in that turn (P1-6). The old behaviour was `prefix(cap)`, i.e. the slots
    /// filled BLINDLY in the server's `tools/list` order; on a 20-tool server an "open an
    /// issue" request never reached the table if `create_issue` was 14th.
    ///
    /// `rank` is stable: if the question carries no signal at all the result is IDENTICAL
    /// to the old blind prefix — that is a regression guarantee.
    private func selectedMCPTools() -> [MCPTool] {
        Array(ToolRelevance.sort(mcpTools, question: lastQuestion,
                               name: \.remoteName, summary: \.summary)
            .prefix(Self.mcpToolCap))
    }

    // MARK: - The connection bridge (mcp §5.4 — plugging in)

    /// The remote-call path and client ownership. `MCPTool` sees only this contract; the
    /// network code is still in one place (`MCPClient`).
    let connectionBridge = MCPToolBridge()

    /// The signature of which connection list the tools were built from. It prevents going
    /// to the network a second time for the same list; if the list (name/address/tool
    /// names) changes it is refreshed.
    private var connectionSignature = ""
    /// The tool-building task in flight — cancelled when the list changes.
    private var connectionTask: Task<Void, Never>?

    /// Builds the MCP tools from the saved connections and feeds them into the session
    /// (mcp §5.4).
    ///
    /// Because the schema arrives from the server at run time, the setup is ASYNCHRONOUS:
    /// until the tools are ready `mcpTools` is empty, so the connection profile cannot be
    /// selected and today's behaviour continues unchanged. With no connection at all the
    /// network is never touched (§2.1).
    ///
    /// The selected connection: the most recently used, or the most recently added if none
    /// was ever used. There is no separate selection surface in v1; because only one
    /// server's tools fit in a 4096 window at a time, a single connection is selected.
    func refreshConnections(_ connections: [Connection]) {
        let live = connections.filter { !$0.isDeleted && $0.isValid }
        let selected = live
            .sorted { ($0.lastUsed ?? $0.createdAt) > ($1.lastUsed ?? $1.createdAt) }
            .first { !$0.availableTools.isEmpty }

        guard let selected, let url = selected.url else {
            // No connection (or none of them has a supported tool): the profile becomes
            // unselectable again and the clients are released.
            connectionTask?.cancel()
            connectionTask = nil
            connectionSignature = ""
            connectionBridge.forget()
            setConnectionTools([], name: "")
            return
        }

        // A SwiftData trap: the object is touched BEFORE any await.
        let identity = selected.id
        let name = selected.name
        let deviceData = selected.deviceData
        // Tools with an unsupported schema are already eliminated here (§5.2).
        let summaries = selected.availableTools
        let key = selected.keyRef.flatMap { Keychain.read(ref: $0) }
        // The signature INCLUDES deviceData: when the user changes the setting in
        // ConnectionDetail the tools must be rebuilt, so the new setting does not wait for
        // the rest of the session.
        let signature = "\(identity)|\(name)|\(url.absoluteString)|\(deviceData.rawValue)|\(summaries.map(\.name).joined(separator: ","))"
        guard signature != connectionSignature else { return }
        connectionSignature = signature
        connectionBridge.save(identity: identity, url: url, key: key)

        connectionTask?.cancel()
        connectionTask = Task { [weak self] in
            guard let self else { return }
            let tools = await connectionBridge.setUpTools(
                connectionID: identity, name: name, summaries: summaries,
                pool: Self.mcpToolPool, deviceData: deviceData,
                gate: executor, reporter: executor)
            guard !Task.isCancelled, connectionSignature == signature else { return }
            // If the server could not be reached, drop the signature: let the next refresh
            // try again, so the user does not have to restart the app when the network
            // comes back.
            if tools.isEmpty { connectionSignature = "" }
            setConnectionTools(tools, name: name)
        }
    }

    /// The tool profile (spec §7.3.1): at most 6–8 tools enter a session in a 4096 window.
    ///
    /// `search` and `connection` are the profiles that leave the device and they
    /// DELIBERATELY exclude the personal-data tools (web-search §5.4, mcp §5.4): a
    /// structural defence against the possibility of the model writing personal data into
    /// an argument — if the tool is not in the session, the data cannot leave either.
    enum Profile {
        case everyday, document, search, connection

        /// The name shown in the timeline row. The profile is not an internal decision the
        /// user cannot see — it determines which tools are on the table, so it is told.
        var timelineName: String {
            switch self {
            case .everyday: return String(localized: "everyday profile")
            case .document:    return String(localized: "document profile")
            case .search:    return String(localized: "search profile")
            case .connection: return String(localized: "connection profile")
            }
        }
    }
    private var activeProfile: Profile = .everyday
    /// The signature of the current session's tool set. THE PROFILE NAME IS NOT ENOUGH:
    /// the content of the everyday set changes within a turn (the Contact ↔ web swap, the
    /// hiding of Spotlight), and had the rebuild condition looked only at
    /// `wanted != activeProfile` that swap would NEVER have been applied after the first
    /// turn — the mechanism would be defined but dead. The measured path: turn 1 "set a
    /// reminder" (the set is built with web), turn 2 "my mother's phone number" → because
    /// the profile was still .everyday the session was kept and ContactTool never entered.
    private var sessionToolSignature = ""
    /// The language name embedded in the session (the reply-language anchor). Updated once
    /// the user's language is detected.
    private var activeLanguage: String = ""
    /// The language the current session was built with — prevents a needless rebuild.
    private var sessionLanguage: String = ""
    /// Does the active language come from the user's explicit choice (rather than from
    /// detection)? It changes the instruction text.
    private var languageChosen: Bool = false
    /// The choice state at the moment the session was built — tracked because the
    /// instruction text is written according to it.
    private var sessionLanguageChosen: Bool = false


    private let models = SystemLanguageModel.default
    private var session: LanguageModelSession?

    /// The context budget threshold: 80% of contextSize (research report §5.2).
    private let thresholdRatio = 0.80

    init() {
        // Remote output is processed against the 4096 budget too (§5.5): a large result is
        // put into the data store without passing through the model, and a summary +
        // sourceRef go to the model.
        connectionBridge.dataStore = dataStore
        // P0-3: the side that will set the flag closing the retry gate once a remote call
        // returns successfully. The bridge is a SINGLE funnel: every remote call passes
        // through here, so if a new MCP call path is added it is impossible to forget to
        // set the flag.
        connectionBridge.executor = executor
        // The code attempt counter is wired to the turn lifecycle here (code-spec §5.4:
        // the reset lives in ToolExecutor.newTurn). Because resetChat calls newTurn
        // internally too, every path goes through the single hook.
        executor.turnHook = { [codeState] in codeState.newTurn() }
        checkAvailability()
    }

    // MARK: - Availability

    func checkAvailability() {
        let wasReady = state.isReady
        switch models.availability {
        case .available:
            state = .ready
            block = nil
            // If we were already ready DO NOT TOUCH the session: rebuilding would silently
            // delete the transcript (i.e. the conversation context) and break the lazy
            // setup of `resetChat`. Build only on the not-ready → ready transition.
            if !wasReady { setUpSession(profile: activeProfile) }
        case .unavailable(let cause):
            switch cause {
            case .deviceNotEligible:
                state = .unavailable(String(localized: "not available on this device"))
                block = .device
            case .appleIntelligenceNotEnabled:
                state = .unavailable(String(localized: "Apple Intelligence is off"))
                block = .closed
            case .modelNotReady:
                state = .preparing
                block = .preparing
            @unknown default:
                state = .unavailable(String(localized: "not available on this device"))
                block = .device
            }
        }
    }

    /// Re-reads availability. On a device where the model download finished or Apple
    /// Intelligence was switched on afterwards, it opens the assistant without restarting
    /// the app. Called when the scene returns (TopBar/ContentView) and at the start of
    /// every request. If it is already ready it resets nothing — it can safely be called
    /// often.
    func reloadAvailability() { checkAvailability() }

    // MARK: - Oturum ve profiller

    /// The everyday profile (spec §8, v1): Calendar, Reminder, Contact/Search, Calculate,
    /// Time + Code (code-spec §7). The cap is 6–8 tools (spec §7.3); if the device
    /// measurement comes out badly, `time` is dropped or the code intent becomes a separate
    /// profile (code-spec §9, open question 2).
    private func everydayTools() -> [any Tool] {
        var calendar = CalendarTool();          calendar.reporter = executor; calendar.dataStore = dataStore
        var reminder = ReminderTool(); reminder.reporter = executor; reminder.dataStore = dataStore
        var calc = CalcTool();            calc.reporter = executor
        var time = TimeTool();            time.reporter = executor
        var code = RunCodeTool();        code.reporter = executor; code.state = codeState
        var tools: [any Tool] = [calendar, reminder, calc, time, code]

        // THE CONTACT ↔ WEB SEARCH SWAP. The two DO NOT STAND in the same set, for two
        // reasons:
        //
        // 1) Budget: the 6–8 tool cap (spec §7.3). Both at once strains the cap.
        // 2) Safety: web-search §5.4 does not want the personal-data tools in the same
        //    profile as the web — the model could write what it read from the address book
        //    into a query.
        //
        // WHICH one enters is decided by THE QUESTION: if the sentence carries an
        // address-book signal, Contact; otherwise web search. That way "my mother's number"
        // works and so does "prayer times" — and the two are never side by side.
        if hasContactSignal {
            var contact = ContactTool(); contact.reporter = executor
            tools.insert(contact, at: 2)
        } else if searchAvailable {
            var web = WebSearchTool()
            web.reporter = executor; web.executor = executor; web.dataStore = dataStore
            tools.insert(web, at: 2)
        }

        // The Spotlight search DOES NOT ENTER the session AT ALL if the user EXPLICITLY
        // asked for the web and web search is off.
        //
        // Measured bug: on "search the internet" the model called `search_notes` as a
        // substitute, the result came back empty and the answer became "I searched the
        // internet and could not find it on the device" — claiming to have done work that
        // WAS NOT DONE, the worst output it can produce. A ban written into the instruction
        // and into the skill body was tried TWICE and did not hold; on a 3B model a ban in
        // text does not control behaviour. Taking the capability away does.
        if !hideLocalSearch {
            var search = SearchNotesTool(); search.reporter = executor
            tools.insert(search, at: 3)
        }
        return tools
    }

    /// Should the Spotlight search be hidden in this turn — `IntentPicker.select`
    /// refreshes it every turn.
    private var hideLocalSearch = false
    /// Is this turn an address-book question — it decides the Contact ↔ web search swap.
    private var hasContactSignal = false

    /// The document/production profile (spec §7.3.1): Create, Read, Edit + the data source
    /// and the helpers.
    ///
    /// While a document is attached, `intentProfile` LOCKS the profile to .document; that
    /// is why a tool absent from this set becomes completely unreachable for the user.
    /// Because "remind me about this tomorrow" / "search my notes" are the two most
    /// frequent requests while a document is attached, Reminder and Search were brought in
    /// here. To fit the 6–8 tool budget ContactTool was taken out: in document production a
    /// contact lookup is the least-used path (the document content already comes from the
    /// attached document or from the calendar/data store), and because it needs the
    /// Contacts permission it came back empty on most turns anyway.
    private func documentTools() -> [any Tool] {
        var create = CreateDocumentTool(); create.reporter = executor; create.context = documentContext; create.dataStore = dataStore
        var read = ReadDocumentTool();         read.reporter = executor;      read.context = documentContext
        var edit = EditDocumentTool(); edit.reporter = executor;  edit.context = documentContext
        var calendar = CalendarTool();        calendar.reporter = executor;   calendar.dataStore = dataStore
        var reminder = ReminderTool(); reminder.reporter = executor; reminder.dataStore = dataStore
        var search = SearchNotesTool();          search.reporter = executor
        var calc = CalcTool();          calc.reporter = executor
        var time = TimeTool();          time.reporter = executor
        return [create, read, edit, calendar, reminder, search, calc, time]
    }

    /// The search profile (web-search §5.4): web_search + Time. NO CALCULATE.
    ///
    /// Calendar/Contact/Search(Spotlight)/Document/Reminder are DELIBERATELY absent. The
    /// model writes the query itself; with a personal-data tool standing in the same
    /// session, "search for the address in my notes" could leak outside in a single step.
    /// Mixed work needing personal data flows over two turns, and because the second turn
    /// has tainted the session it falls to the approval gate.
    ///
    /// THE CALCULATE TOOL WAS REMOVED (audit cluster 1 — live-value invention). Three
    /// measured cases showed that in this profile the sole function of `calculate` was to
    /// GIVE INVENTION A LEGITIMATE COVER — the model made up the live value it had searched
    /// for and not found, had the tool do the arithmetic, and presented the resulting number
    /// as "it came from a tool":
    ///
    ///   web-euro   → calculate("(1.00 / 0.85) * 100") → "the euro is 117.6471 TL"
    ///   web-petrol → calculate("(1.60 * 1.20)")       → "petrol is 1.92 TL"
    ///   web-league → calculate("(139+30)*1.20")       → "Beşiktaş leads with 202.8 points"
    ///
    /// In all three the input numbers come from NO search result at all; the invention has
    /// already happened in the `expression` field. The tool works correctly and the result
    /// is still a lie. Closing this with an instruction means relying on the model's good
    /// intentions (the same lesson was learned twice with `hideLocalSearch`: on a 3B model a
    /// ban in text does not control behaviour, taking the capability away does).
    ///
    /// THE COST and why it is acceptable: a chain such as "how many TL is 100 dollars"
    /// spreads over two turns — the rate comes from the web in this turn, and the
    /// multiplication happens on the next turn in the everyday profile with `calculate`. THE
    /// CALCULATION ESCAPE in `intentProfile` routes that second turn to everyday, so the
    /// chain does not break, it only lengthens. A one-turn delay in the arithmetic is better
    /// in every case than an invented rate presented instantly.
    private func searchTools() -> [any Tool] {
        var web = WebSearchTool(); web.reporter = executor; web.executor = executor; web.dataStore = dataStore
        var time = TimeTool();  time.reporter = executor
        return [web, time]
    }

    /// The connection profile (mcp §5.4): the selected connection's tools + Calculate +
    /// Time. The personal-data tools are absent for exactly the same reason as in the
    /// search profile.
    private func connectionTools() -> [any Tool] {
        var calc = CalcTool(); calc.reporter = executor
        var time = TimeTool(); time.reporter = executor
        return selectedMCPTools().map { $0 as any Tool } + [calc, time]
    }

    /// Summarises the tool set to be built in a single string. Only the inputs that REALLY
    /// change the set are written; otherwise every turn would pay the cost of a needless
    /// rebuild (and a summarisation turn).
    private func toolSignature(_ profile: Profile) -> String {
        switch profile {
        case .everyday:
            let swap = hasContactSignal ? "contact" : (searchAvailable ? "web" : "none")
            return "everyday|\(swap)|\(hideLocalSearch ? "nospot" : "spot")"
        case .document:    return "document"
        case .search:    return "search"
        // THE COUNT IS NOT ENOUGH: the relevance ranking can pick a DIFFERENT six from turn
        // to turn while the count stays the same. The signature writes the selected NAMES;
        // otherwise the mechanism would be defined but never run, because after the first
        // turn the session would never be rebuilt (the same trap as the Contact ↔ web swap
        // in the everyday set).
        case .connection: return "connection|" + selectedMCPTools().map(\.name).joined(separator: ",")
        }
    }

    private func buildTools(_ profile: Profile) -> [any Tool] {
        switch profile {
        case .everyday:  return everydayTools()
        case .document:     return documentTools()
        case .search:     return searchTools()
        case .connection:  return connectionTools()
        }
    }

    // MARK: - Beceriler (progressive disclosure)

    /// Per-turn prompt enrichment (a memory note + the skill guide + data references + the
    /// attached-document line + the language reminder). The logic and the session-lived
    /// injection state live in `PromptEnricher`; it is only triggered from here and cleared
    /// at the session boundary with `reset()`.
    private let enricher = PromptEnricher()

    /// The tool names in the current session. The SINGLE source of truth of the skill gate
    /// (audit P1-4): a guide is not injected if the tool it commands IS NOT in the session —
    /// otherwise the model would be steered towards calling a tool that does not exist (in
    /// the document lock a "write code…" request triggers code.md while `run_code` is not in
    /// the document set).
    ///
    /// There used to be a hand-written `[skill: Set<Profile>]` map here. That map WAS A COPY
    /// OF THE TOOL SET and, being a copy, it went stale silently: when a tool moved between
    /// profiles (as Reminder and Search moved into the document set), forgetting to update
    /// the map produced no compile error at all. The gate now reads from the session's REAL
    /// tool names; a drift between the map and the set is conceptually impossible.
    private var sessionToolNames: Set<String> = []

    /// Feeds the enricher with this turn's state and opens the timeline row.
    /// Because the timeline recorder stayed in `ModelService`, the skill name comes back as
    /// feedback; we open the row here (the order does not change: routing → skill).
    private func enrichPrompt(_ question: String) -> String {
        let outcome = enricher.enrich(
            question,
            sessionToolNames: sessionToolNames,
            dataRefs: dataStore.references,
            attachedDocumentName: documentContext.runnableDocument?.name,
            activeLanguage: activeLanguage)
        if let name = outcome.injectedSkill {
            timeline.begin(kind: .enrichment, text: Self.skillAdded(name))
        }
        return outcome.prompt
    }

    private func setUpSession(profile: Profile, resuming summary: String? = nil) {
        activeProfile = profile
        // New session = new context: the injected skills and notes are no longer in the
        // transcript, so both must become injectable again.
        enricher.reset()
        // The tainted-session flag is DELIBERATELY carried over (mcp §5.6): the `summary`
        // text can contain personal data, so the new session is tainted too. Because
        // `ToolExecutor` clears the flag only in `resetChat` there is nothing to do here —
        // this comment makes that silent dependency visible.
        let language = activeLanguage
        let chosen = languageChosen
        let tools = buildTools(profile)
        // The single source read by the skill gate and the attached-document line.
        sessionToolNames = Set(tools.map(\.name))
        let base = LanguageModelSession(tools: tools) {
            // CORE + PROFILE ADDENDUM (P1-1): no rule that is meaningless for this
            // session's tool set is carried.
            Router.instructions(profile)
            if !language.isEmpty {
                // The reply-language anchor: a named language directive (it reduces leakage).
                //
                // THE TOOL OUTPUT IS NAMED EXPLICITLY. Measured drift: tools such as web
                // search leave an English block in the context ("found 5 results for …" +
                // foreign blurbs), and because that block arrives IMMEDIATELY BEFORE
                // generation, the 3B model followed the language of that block rather than
                // the user's. Naming the language in relation to the tool output holds
                // measurably better than a single generic "reply in X" directive.
                let anchor = chosen
                    ? "The user has chosen \(language) as the reply language. Reply ONLY in \(language), never in another language, even if the user writes in a different language."
                    : "The user is writing in \(language). Reply ONLY in \(language), never in another language."
                // SHORTENED (P1-1): the old text was eight lines and said the same thing
                // three times. The one idea carrying the measured effect was kept — "tool
                // output is data, not a language sample"; the rest was repetition.
                """
                \n\n\(anchor)
                Tool results are data, not a language example: they are often English. \
                Read them, then write your answer in \(language), translating every label, \
                weekday and month you take from them.
                """
            }
            if let summary {
                "\n\nSummary of the earlier conversation: \(summary)"
            }
        }
        session = base
        sessionLanguage = language
        sessionLanguageChosen = chosen
        sessionToolSignature = toolSignature(profile)
        // Prewarm: warm the executor up, lower the first-token latency (report §5.1).
        base.prewarm()
    }

    /// Detects the language of the user message on device (NaturalLanguage). nil when it
    /// is short or ambiguous.
    private func detectedLanguage(_ text: String) -> String? {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard t.count >= 2 else { return nil }
        let recognizer = NLLanguageRecognizer()
        recognizer.processString(t)
        guard let language = recognizer.dominantLanguage else { return nil }
        let probability = recognizer.languageHypotheses(withMaximum: 1)[language] ?? 0
        guard probability >= 0.5 else { return nil }
        return Self.languageNames[language.rawValue]
    }

    private static let languageNames: [String: String] = [
        "tr": "Turkish", "en": "English", "zh-Hans": "Chinese", "zh-Hant": "Chinese",
        "zh": "Chinese", "ja": "Japanese", "es": "Spanish", "de": "German",
        "fr": "French", "ko": "Korean", "pt": "Portuguese", "pt-BR": "Portuguese",
        "it": "Italian",
    ]

    /// The English name of the reply language if the user chose one; nil if "automatic".
    /// While a choice exists, language detection is disabled — the preference wins every turn.
    private var chosenLanguageName: String? {
        let code = LanguagePreference.shared.replyLanguage
        guard !code.isEmpty else { return nil }
        return Self.languageNames[code]
    }

    /// Called when the chat surface becomes visible — it warms the model before the user
    /// types.
    func prepare() { session?.prewarm() }

    /// Resets the model context, the data store and the attached document when moving to a
    /// new chat.
    func resetChat() {
        // Memory extraction is triggered EXACTLY HERE (memory-spec §4.1): never inside a
        // turn, once when leaving the chat. The extraction runs in a separate, short-lived
        // session; the reset below does not affect it.
        memoryTrigger?()
        // The in-flight production must be cut too: if only the outer task is cancelled,
        // `isProducing` stays stuck at true and in the new chat the send button would freeze
        // on the stop icon.
        stop()
        dataStore.clear()
        documentContext.removeDocument()
        documentContext.forgetProduced()
        documentContext.toPreview = nil
        // A REAL chat reset: not `newTurn()`. The tainted-session flag and the refusal cache
        // are session-lived (mcp §5.6, §3.3) and end only here; calling `newTurn()` would
        // start the new chat with the old chat's taint.
        executor.resetChat()   // turnHook resets the code attempt counter too
        timeline.reset()
        // The reset forgets the detected language but DOES NOT OVERRIDE the user's explicit
        // choice.
        activeLanguage = chosenLanguageName ?? ""
        languageChosen = chosenLanguageName != nil
        enricher.reset()
        sessionToolNames = []
        // The new chat's transcript is a different one: the old summary cache is not kept.
        summaryCache = nil
        lastBudgetMeasureError = nil
        // The routing signals are session-lived too: a new chat must not start in an
        // off-device profile because of the previous chat's search/connection chips.
        previousTurnSearch = false
        previousTurnConnection = false
        activeProfile = .everyday
        // Lazy: do not build the session now. On the first message the language is detected
        // and it is built in one go (this avoids a double setup per new chat).
        session = nil
        sessionLanguage = ""
        sessionLanguageChosen = false
        sessionToolSignature = ""
    }

    /// Gathers all the inputs of the routing decision from the current state. The decision
    /// logic lives in `IntentPicker` and DOES NOT READ instance state; this function is the
    /// SINGLE bridge between the state and the pure picker.
    private func intentInput(_ question: String, available: Profile) -> IntentPicker.Input {
        IntentPicker.Input(question: question,
                          currentProfile: available,
                          hasDocument: documentContext.runnableDocument != nil,
                          searchAvailable: searchAvailable,
                          connectionAvailable: connectionAvailable,
                          connectionName: connectionName,
                          previousTurnSearch: previousTurnSearch,
                          previousTurnConnection: previousTurnConnection)
    }

    /// Intent classification (spec §7.3.1). The decision is in `IntentPicker.select`; here
    /// only the two flags that shape the everyday tool set are written onto the instance.
    /// The flags are set BEFORE the return: `toolSignature` reads both and signs the set
    /// according to this turn's decision.
    private func intentProfile(_ question: String, available: Profile) -> Profile {
        let outcome = IntentPicker.select(intentInput(question, available: available))
        hideLocalSearch = outcome.hideLocalSearch
        hasContactSignal = outcome.hasContactSignal
        return outcome.profile
    }

    // MARK: - In-turn profile recovery (audit P1-2)

    /// The SECOND profile to try when the deterministic picker was wrong
    /// (`IntentPicker.secondPass`). `hasContactSignal` is the value computed in this turn's
    /// first selection — it is passed to the picker as a parameter.
    private func secondProfile(_ question: String, first: Profile) -> Profile? {
        IntentPicker.secondPass(intentInput(question, available: first),
                           first: first,
                           hasContactSignal: hasContactSignal)
    }

    /// "I cannot do that" patterns — a CAPABILITY refusal in 8 languages. Deliberately
    /// NARROW: a factual "I could not find it" (searched but no result) DOES NOT BELONG
    /// HERE, because that is a correct and honest answer; re-running would take the model
    /// to the same empty result a second time and do nothing but burn battery.
    ///
    /// THE PATTERNS ARE USER-FACING MODEL OUTPUT in 8 languages, not identifiers: they must
    /// stay in their own languages.
    private static let cannotDoTraces = [
        "yapamıyorum", "yapamam", "yapamadım", "edemiyorum", "edemem",
        "bu konuda yardımcı olamıyorum", "yardımcı olamam", "erişimim yok",
        "yeteneğim yok", "elimde böyle bir araç", "bir aracım yok",
        "i can't", "i cannot", "i'm unable", "i am unable", "i don't have access",
        "i don't have the ability", "i'm not able",
        "no puedo", "ich kann nicht", "je ne peux pas", "não posso",
        "できません", "无法", "할 수 없",
    ]

    /// The in-turn recovery trigger. THREE conditions at once:
    /// 1. No tool ran at all — if a tool ran the profile was right, and the content of the
    ///    answer may be debatable but there is no CAPABILITY gap.
    /// 2. The text carries a capability-refusal pattern.
    /// 3. (on the caller side) no recovery has been tried in this turn yet.
    ///
    /// Pure and static: tested without a model.
    static func recoveryNeeded(traces: [ToolTrace], text: String) -> Bool {
        guard traces.isEmpty else { return false }
        let m = text.lowercased()
        return cannotDoTraces.contains(where: m.contains)
    }

    /// Classifies a turn that ended without text: if a tool failed during the turn the fault
    /// is IN THE TOOL (`.toolFailed`), otherwise the model never formed a sentence
    /// (`.emptyReply`). Pure and static: tested without a model.
    static func failureClass(traces: [ToolTrace]) -> ErrorClass {
        let hasFailure = traces.contains {
            if case .failed = $0.state { return true }
            return false
        }
        return hasFailure ? .toolFailed : .emptyReply
    }

    /// The single mapping point from a class to a user sentence. The text choice stands in
    /// ONE place so that a new class left without a sentence shows up in the compiler.
    /// Pure: tested without a model.
    static func failureText(_ errorClass: ErrorClass) -> String {
        switch errorClass {
        case .toolFailed:      return L10n.toolFailedReply
        case .contextOverflow:  return L10n.conversationTooLong
        case .outOfBounds:      return L10n.outOfBounds
        case .unsupportedLanguage:        return L10n.languageUnsupported
        case .afterWrite:   return L10n.errorAfterWrite
        case .emptyReply:       return L10n.answerNotAssembled
        case .generationError, .none: return L10n.tryAgain
        }
    }

    /// Is a search server configured and on? `isActive` already returns false if the address
    /// is invalid — there is no "on but not working" intermediate state.
    private var searchAvailable: Bool { WebSearchSetting.isActive }

    /// Is a connection selected and can at least one of its tools enter the session?
    private var connectionAvailable: Bool { !mcpTools.isEmpty }

    /// Did a search chip drop in the previous turn (the web-search §5.4 signal)? A follow-up
    /// question ("and tomorrow?") contains no keyword at all.
    private var previousTurnSearch = false
    /// Did an MCP chip drop in the previous turn (the mcp §5.4 signal)?
    private var previousTurnConnection = false

    /// An EXPLICIT arithmetic intent (audit cluster 1). The logic moved to `IntentPicker`;
    /// this forwarding point preserves the call shape used by `SelfTestCases`.
    static func calcIntent(_ s: String) -> Bool { IntentPicker.calcIntent(s) }

    /// Derives the next turn's routing signals from this turn's chips. The icon belongs to
    /// the tool that dropped the chip and IS NOT text produced by the model — which is why
    /// it is a signal closed to hallucination.
    private func updateTurnSignals() {
        previousTurnSearch = executor.traces.contains { $0.icon == "globe" }
        previousTurnConnection = executor.traces.contains { $0.icon == "arrow.up.forward.app" }
    }

    // MARK: - Timeline row texts (timeline-spec §2.4)
    //
    // Lowercase, real time, deterministic events. No invented status verb: only what
    // REALLY happens in the code is written.

    private static func routedTo(_ profile: Profile) -> String {
        String(localized: "routed · \(profile.timelineName)")
    }
    private static func skillAdded(_ name: String) -> String {
        String(localized: "skill added · \(name)")
    }
    /// The text of the live writing step. PRESENT tense: when the step opens the writing is
    /// still going. The place that converts it to past tense is `TimelineLog.persist`, and it
    /// does that conversion by looking at the text — which is why this string must stay
    /// identical to `TimelineLog.writingText`.
    private static var writingText: String { String(localized: "writing") }

    // MARK: - Production / cancellation

    /// The live stream task. If the user says "stop" this is cancelled.
    private var productionTask: Task<String, Error>?

    /// The summarisation task in flight. `stop()` cancels THIS TOO: `summarize()` is a full
    /// LLM call, and had only `productionTask` been cancelled, a "stop" pressed during
    /// summarisation would cut nothing (measured wait: seconds).
    private var summaryTask: Task<String?, Never>?

    /// The text accumulated so far in a turn's stream — when cancelled it IS NOT HIDDEN from
    /// the user, the half answer stays on screen (deleting it silently would take back what
    /// the user has read).
    ///
    /// A NEW INSTANCE PER TURN, NOT an instance field. As a field, every `consumeStream` was
    /// resetting it: when a cancelled old turn's `recoverFromError` fell into
    /// `cancelledOutcome()`, it read the partial text (or the empty text) of the NEW turn
    /// that had started meanwhile and wrote it into the old turn's Message. The
    /// `productionNumber` generation counter separates which turn is speaking; this box
    /// separates which text belongs to that turn.
    private final class StreamBuffer { var text = "" }

    /// Is there live production — ChatView draws the send/stop button from this.
    private(set) var isProducing: Bool = false

    /// The identity of the running turn. `stop()` advances it, so that the late `defer` of a
    /// cancelled turn cannot lower the `isProducing` flag of the NEW turn started meanwhile.
    private var productionNumber = 0

    /// Cancels production. The text streamed so far is preserved; `respond` returns normally
    /// with the half text (a cancellation, not an error).
    ///
    /// `isProducing` becomes false IMMEDIATELY: waiting for the underlying stream to die was
    /// freezing the send button on the stop icon for seconds after the user pressed "stop"
    /// and silently refusing the new request.
    func stop() {
        productionTask?.cancel()
        productionTask = nil
        // The summarisation is cut too: what "stop" cuts must be the WHOLE turn, not just
        // the stream (see `summaryTask`).
        summaryTask?.cancel()
        summaryTask = nil
        productionNumber &+= 1          // the old turn's defer can no longer touch the flag
        isProducing = false
        // THE PENDING APPROVAL IS RESOLVED TOO. `ToolExecutor.approvalContinuation` is a
        // `withCheckedContinuation` and is NOT AFFECTED by task cancellation: cancelling the
        // task alone left the approval sheet on screen and leaked the tool task in suspension
        // until the next message. The decision is `.cancelled`, not a refusal — the source is
        // not blacklisted (see `requestApprovalDecision`).
        executor.resolvePendingApproval()
        // No silent disappearance (timeline-spec §3.4): the steps so far stay, and a closed
        // "stopped partway" row is appended at the end.
        syncTimeline()
        timeline.interrupt()
    }

    // MARK: - The timeline bridge

    /// Reflects the chips in `ToolExecutor` into the timeline as tool steps.
    ///
    /// The right place is `ToolExecutor.start` (timeline-spec §5.2: "the single extra line
    /// into the timeline is to open a step at the moment of `start`") — because that file
    /// belongs to another agent in this phase, the bridge was built here. `ChatView` does the
    /// EVENT-based binding (`onChange(of: executor.traces)`), so on screen the step opens
    /// instantly; the polling here is a safety net for view-less callers (LanguageTest,
    /// Evaluation). The step order is preserved: tool calls are resolved BETWEEN stream
    /// chunks and `syncTimeline` runs on every chunk. Because it deduplicates by `traceID`,
    /// this bridge becomes a no-op by itself once a direct call is added inside `start` — it
    /// produces no duplicate step and does not need deleting.
    private func syncTimeline() {
        let bound = Set(timeline.steps.compactMap(\.toolTraceID))
        for trace in executor.traces where !bound.contains(trace.id) {
            timeline.bindTool(traceID: trace.id)
        }
    }

    // MARK: - Reply (streaming)

    /// The legacy call shape (LanguageTest/Evaluation): text + chips only.
    func respond(_ question: String, stream: @escaping (String) -> Void) async -> (text: String, traces: [ToolTrace]) {
        let s = await replyOutcome(question, stream: stream)
        return (s.text, s.traces)
    }

    /// Produces a streamed answer to the user's question. `stream` is called with every
    /// partial text. The return carries the error/retry flags ALONGSIDE the text — the UI
    /// does not compare text.
    func replyOutcome(_ question: String, stream: @escaping (String) -> Void) async -> ReplyOutcome {
        // If we are not ready, look again quietly: the model download may have finished or
        // the user may have switched Apple Intelligence on in Settings (without restarting
        // the app).
        if !state.isReady { reloadAvailability() }
        guard state.isReady else {
            return ReplyOutcome(text: blockMessage, traces: [],
                               isError: true, isRetryable: blockIsRetryable)
        }
        // The parallel-request lock (report §5.1). It rests on our own flag: `isResponding`
        // stays true for a while after a cancellation and was locking the user out for
        // nothing.
        if isProducing {
            return ReplyOutcome(text: L10n.previousFinishing, traces: [], isTransient: true)
        }
        productionNumber &+= 1
        let myTurn = productionNumber
        // This turn's stream buffer. It is born with the turn and dies with the turn.
        let buffer = StreamBuffer()
        isProducing = true
        // The late-finishing defer of a cancelled turn (one whose productionNumber has
        // advanced) must not lower the flag of the new turn started meanwhile.
        defer { if productionNumber == myTurn { isProducing = false } }
        // The signals are read from the PREVIOUS turn's chips, before the chips are reset.
        updateTurnSignals()
        // The input of the slot relevance ranking (P1-6). Written BEFORE the profile
        // decision: `toolSignature` will write the tools selected for this turn's question,
        // and if the selection changed the session will be rebuilt.
        lastQuestion = question
        executor.newTurn()   // turnHook resets the code attempt counter too (code-spec §5.4)
        timeline.reset()

        // Profile + language routing: if there is no session, or the profile/language
        // changed, build it in one go.
        let wanted = intentProfile(question, available: activeProfile)
        timeline.begin(kind: .routing, text: Self.routedTo(wanted))
        // If there is a preference, detection does not run; when the preference changes
        // activeLanguage changes too and the `activeLanguage != sessionLanguage` condition
        // below rebuilds the session with the new language.
        if let secilen = chosenLanguageName {
            activeLanguage = secilen
            languageChosen = true
        } else {
            // On returning from an explicit choice to Automatic, the forced language must not
            // stay stuck: let detection start from a clean slate, otherwise on short inputs
            // that do not pass the threshold the answer would keep coming in the previously
            // chosen language.
            if languageChosen { activeLanguage = "" }
            languageChosen = false
            activeLanguage = detectedLanguage(question) ?? activeLanguage
        }
        if session == nil || wanted != activeProfile || toolSignature(wanted) != sessionToolSignature
            || activeLanguage != sessionLanguage || languageChosen != sessionLanguageChosen {
            setUpSession(profile: wanted, resuming: await summarize())
        }
        await checkBudget()
        // Setup + summarisation + measurement are all awaits: a "stop" pressed in this window
        // used to cut nothing and production started anyway. A cancellation is not an error —
        // whatever text has streamed (empty if none yet) is what comes back.
        guard productionNumber == myTurn else { return cancelledOutcome(buffer.text) }
        guard let session else {
            return ReplyOutcome(text: blockMessage, traces: [],
                               isError: true, isRetryable: blockIsRetryable)
        }

        // The matching skill guide + memory notes are attached to this turn's prompt (both of
        // them once per session).
        let prompt = enrichPrompt(question)
        do {
            let raw = try await consumeStream(session, question: prompt, buffer: buffer, stream: stream)
            // An unparsed tool call DOES NOT GO to the user.
            var finalText = Self.stripToolLeak(raw)

            // IN-TURN PROFILE RECOVERY (P1-2). NO tool trace + a "I cannot do that" pattern
            // PRESENT = the deterministic picker was most likely wrong and the required tool
            // is not in this session at all. It is retried once, with the second most likely
            // profile. It MUST happen BEFORE `timeline.finish()`: no new step can be opened
            // after the recorder is closed.
            if Self.recoveryNeeded(traces: executor.traces, text: finalText),
               productionNumber == myTurn,
               let secondPass = secondProfile(question, first: activeProfile) {
                finalText = await recoverProfile(secondPass, question: question, myTurn: myTurn,
                                              ilkMetin: finalText, stream: stream)
            }

            // Normal completion or a cancellation (half text) — neither is an error. In a
            // cancelled turn `stop()` has already cut the timeline; `finish()` does not touch
            // a closed recorder.
            syncTimeline()
            timeline.finish()
            // If only leakage is left, there is nothing to say in the turn: a retryable error
            // is more honest than showing half a JSON.
            //
            // BUT the "which error" distinction is made here: if a tool FAILED during the
            // turn this is not a generation fault but a tool fault, and the user already sees
            // it in the chip. Giving the same sentence to both was what dropped all five of
            // five measured cases onto the same text and made the cause invisible.
            if finalText.isEmpty, !raw.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                stream("")
                let errorClass = Self.failureClass(traces: executor.traces)
                return ReplyOutcome(text: Self.failureText(errorClass), traces: executor.traces,
                                   isError: true, isRetryable: executor.retryIsSafe,
                                   errorClass: errorClass)
            }
            if finalText != raw { stream(finalText) }
            return ReplyOutcome(text: finalText, traces: executor.traces)
        } catch {
            let outcome = await recoverFromError(error, question: question, myTurn: myTurn,
                                         buffer: buffer, stream: stream)
            syncTimeline()
            timeline.finish()
            return outcome
        }
    }

    /// Re-runs the turn ONCE with the second profile. If the recovery does not hold,
    /// `firstText` comes back — the attempt never makes the answer in the user's hands worse,
    /// at worst it spends one generation.
    ///
    /// Whichever text it returns, THAT text is written to the screen (`stream`), because the
    /// first attempt's text has been erased from the stream.
    ///
    /// The summary IS CARRIED OVER: dropping the conversation context would be a bigger loss
    /// than the recovery itself. There is a risk that an "I cannot do that" sentence entering
    /// the summary biases the second attempt; against that, on the second attempt the tool is
    /// REALLY in the session, so the model has a concrete path in front of it.
    private func recoverProfile(_ secondPass: Profile,
                              question: String,
                              myTurn: Int,
                              ilkMetin: String,
                              stream: @escaping (String) -> Void) async -> String {
        stream("")   // remove the first attempt's "I cannot do that" text from the screen
        // A recovery turn: the chips and the side-effect flags ARE PRESERVED (P2-8).
        executor.newTurn(forgetSideEffects: false)
        setUpSession(profile: secondPass, resuming: await summarize())
        // Summarisation is a long await: if "stop" was pressed during it, do not start the
        // second generation at all — it would be an invisible continuation of a cancelled turn.
        guard productionNumber == myTurn else {
            stream(ilkMetin)
            return ilkMetin
        }
        timeline.begin(kind: .routing, text: Self.routedTo(secondPass))
        // The prompt is rebuilt: the session changed, so the skill guide and the
        // attached-document line are the ones valid for this new set.
        if let new = session,
           let raw = try? await consumeStream(new, question: enrichPrompt(question),
                                        buffer: StreamBuffer(), stream: stream) {
            let text = Self.stripToolLeak(raw)
            // If it is empty or again "I cannot do that", the recovery did not hold.
            if !text.isEmpty,
               !Self.recoveryNeeded(traces: executor.traces, text: text) {
                stream(text)
                return text
            }
        }
        stream(ilkMetin)
        return ilkMetin
    }

    /// The error taxonomy (report §5.5): overflow → invisible recovery; guardrail/language →
    /// NO retry.
    private func recoverFromError(_ error: Error,
                            question: String,
                            myTurn: Int,
                            buffer: StreamBuffer,
                            stream: @escaping (String) -> Void) async -> ReplyOutcome {
        stream("")  // clear the half-streamed text
        // A retry sends THE SAME prompt a second time. If a tool has already changed the world
        // in this turn (an event was written, a reminder was set, a document was produced), the
        // second attempt REPEATS the same side effect — a duplicate event, a duplicate reminder.
        //
        // A LOCAL side effect is NOT the only axis (audit P0-3): a remote MCP write (a Jira
        // issue, a record, an email) drops no `.written` chip and never set the `worldChanged`
        // flag, so a general error occurring AFTER that side effect entered the retry and a
        // second issue was opened. `retryIsSafe` reads both axes at once.
        if !executor.retryIsSafe {
            // An error bubble yes; "try again" NO — the side effect would be repeated.
            return ReplyOutcome(text: L10n.errorAfterWrite, traces: executor.traces,
                               isError: true, isRetryable: false,
                               errorClass: .afterWrite)
        }
        if let g = error as? LanguageModelSession.GenerationError {
            switch g {
            case .guardrailViolation:
                // Unrecoverable — a single sentence without burning battery (no retry).
                return ReplyOutcome(text: L10n.outOfBounds, traces: executor.traces,
                                   isError: true, isRetryable: false,
                                   errorClass: .outOfBounds)
            case .unsupportedLanguageOrLocale:
                return ReplyOutcome(text: L10n.languageUnsupported, traces: executor.traces,
                                   isError: true, isRetryable: false,
                                   errorClass: .unsupportedLanguage)
            case .exceededContextWindowSize:
                // Recoverable: summarise, rebuild the session, try once.
                guard productionNumber == myTurn else { return cancelledOutcome(buffer.text) }
                executor.newTurn(forgetSideEffects: false)
                setUpSession(profile: activeProfile, resuming: await summarize())
                // "Stop" may have been pressed while the summarisation was running.
                guard productionNumber == myTurn else { return cancelledOutcome(buffer.text) }
                if let new = session,
                   let m = try? await consumeStream(new, question: question,
                                              buffer: StreamBuffer(), stream: stream) {
                    return ReplyOutcome(text: m, traces: executor.traces)
                }
                return ReplyOutcome(text: L10n.conversationTooLong, traces: executor.traces,
                                   isError: true, isRetryable: true,
                                   errorClass: .contextOverflow)
            default:
                break
            }
        }
        // Other transient errors: try once more with a fresh session.
        guard productionNumber == myTurn else { return cancelledOutcome(buffer.text) }
        executor.newTurn(forgetSideEffects: false)
        setUpSession(profile: activeProfile)
        if let new = session,
           let m = try? await consumeStream(new, question: question,
                                      buffer: StreamBuffer(), stream: stream) {
            return ReplyOutcome(text: m, traces: executor.traces)
        }
        // The retry did not hold either. If a tool failed during the turn, THAT is the fault
        // the user sees (the chip is already there); otherwise the fault is on the generation
        // side.
        let errorClass: ErrorClass =
            Self.failureClass(traces: executor.traces) == .toolFailed ? .toolFailed : .generationError
        return ReplyOutcome(text: Self.failureText(errorClass), traces: executor.traces,
                           isError: true, isRetryable: true,
                           errorClass: errorClass)
    }

    /// A CANCELLED TURN IS NOT RECOVERED (audit P1-7).
    ///
    /// `recoverFromError` used to ignore `productionNumber` entirely: an error arriving after
    /// the user pressed "stop" started a new `setUpSession` + `consumeStream`, i.e. opened an
    /// invisible production race. Worse, because `setUpSession` writes `activeProfile`, it
    /// could overwrite the session of the NEW turn started meanwhile.
    ///
    /// A cancellation IS NOT an error: `isError` false, `isRetryable` false — the half text
    /// stays on screen, and the user already knows they stopped it themselves.
    ///
    /// The text IS A PARAMETER: a copy of the buffer captured at the start of the turn is
    /// passed in. Had it been read from a shared field, the race of writing the new turn's
    /// text into the old turn's outcome would stay open.
    private func cancelledOutcome(_ partialText: String) -> ReplyOutcome {
        ReplyOutcome(text: partialText, traces: executor.traces)
    }

    private func consumeStream(_ session: LanguageModelSession,
                         question: String,
                         buffer: StreamBuffer,
                         stream: @escaping (String) -> Void) async throws -> String {
        buffer.text = ""
        // The stream runs in a separate Task so that `stop()` can cancel it.
        let task = Task { @MainActor [weak self] () throws -> String in
            var last = ""
            // The stream can take a long time without producing a chunk; honour a
            // cancellation even before the first chunk (with checkCancellation only inside
            // the loop, "stop" was ineffective until the first token arrived).
            try Task.checkCancellation()
            let responseStream = session.streamResponse(to: question, options: Self.generationOptions)
            var firstChunk = true
            for try await chunk in responseStream {
                // If the user said "stop" we leave here; the buffer holds the last text it
                // saw and the catch above returns it.
                try Task.checkCancellation()
                // Tool steps are caught at chunk boundaries: tool calls are resolved BETWEEN
                // stream chunks, so their order is correct here.
                self?.syncTimeline()
                if firstChunk {
                    // A single step — NOT per chunk. Once, the moment writing starts.
                    firstChunk = false
                    self?.timeline.begin(kind: .writing, text: ModelService.writingText)
                }
                last = chunk.content
                // The buffer belongs to THE TURN; it is not shared through `self`.
                buffer.text = last
                stream(last)
            }
            return last
        }
        productionTask = task
        defer { productionTask = nil }
        do {
            return try await task.value
        } catch is CancellationError {
            // A cancellation is not an error: let the half answer stay on screen for the user
            // to read.
            return buffer.text
        }
    }

    // MARK: - The context budget (report §5.2 — a real token measurement)

    /// The diagnostic channel filled when the measurement fails. When the budget check was
    /// skipped silently, the overflow blew up one turn later as `exceededContextWindowSize`
    /// and no trace was left to answer the question "did the measurement fail or is the
    /// threshold wrong?". IT IS NOT SHOWN to the user (that decision stands) — only the
    /// eval/DEBUG reads it.
    private(set) var lastBudgetMeasureError: String?

    /// Measures the transcript's real token count with `tokenCount`; when it exceeds 80% of
    /// contextSize it rebuilds the session with a summary. A measurement, not an estimate.
    private func checkBudget() async {
        let kind = productionNumber
        guard let session else { return }
        // contextSize exists in 26.0, tokenCount arrived in 26.4. Because the deployment
        // target is 26.0 the measurement sits behind a version gate: on 26.0–26.3 the budget
        // check cannot be made and the overflow is reported by FoundationModels' own error.
        guard #available(iOS 26.4, *) else {
            lastBudgetMeasureError = "tokenCount requires iOS 26.4"
            return
        }
        let threshold = Int(Double(models.contextSize) * thresholdRatio)
        let number: Int
        do {
            number = try await models.tokenCount(for: session.transcript)
        } catch {
            // It was `try?`: a failing measurement was SILENTLY skipping the budget check.
            // The decision is the same (do not show the user an error, let the turn continue)
            // but it now leaves a trace.
            lastBudgetMeasureError = String(describing: error)
            #if DEBUG
            print("Tacet: the token measurement failed, the budget check was skipped — \(error)")
            #endif
            return
        }
        lastBudgetMeasureError = nil
        guard number > threshold else { return }
        // The measurement is an await too: "stop" may have been pressed during it, in which
        // case summarising and rebuilding (seconds) has no recipient at all.
        guard productionNumber == kind else { return }
        setUpSession(profile: activeProfile, resuming: await summarize())
    }

    /// How many raw turns are carried into the new session on a budget overflow (spec §146:
    /// the last 4–6 turns are preserved).
    private let keptTurnCount = 6

    /// The input and the outcome of the last summary. The same transcript IS NOT SUMMARISED
    /// twice.
    ///
    /// A single user turn can build a session several times (a profile/language/signature
    /// change, a budget overflow, in-turn profile recovery, error recovery) and every setup
    /// meant a full LLM summary call. If the input is identical the outcome is identical too;
    /// the second call only adds to the first-token latency.
    private var summaryCache: (input: String, outcome: String)?

    /// Has the old history summarised into a single paragraph; if the summarisation fails it
    /// returns at least the raw text of the last turns. Under no condition does it drop the
    /// context silently — the assistant losing its memory is an invisible fault for the user,
    /// the worst kind of error.
    private func summarize() async -> String? {
        let kind = productionNumber
        guard let session else { return nil }
        let dump = session.transcript.compactMap(Self.turnText)
        guard !dump.isEmpty else { return nil }

        // The raw text of the last turns: both an input to the summary prompt and a fallback
        // context.
        let lastTurns = dump.suffix(keptTurnCount).joined(separator: "\n")
        let rawTail = "Recent conversation:\n" + Self.truncate(lastTurns, 2000)

        // NO SUMMARY CALL IS MADE ON A SHORT HISTORY. Everything up to the kept-turn count is
        // already carried VERBATIM; taking an LLM summary on top of that would produce the
        // same information a second time, worse (a 3B model, numbers get rounded) and at the
        // cost of seconds. For a user moving from topic to topic the session is rebuilt often
        // and those sessions are exactly the short ones — this was the main source of
        // first-token latency.
        guard dump.count > keptTurnCount else { return rawTail }

        let history = Self.truncate(dump.suffix(24).joined(separator: "\n"), 4000)
        if let cache = summaryCache, cache.input == history { return cache.outcome }

        // The summary is produced in a SEPARATE, short session. It used to add one more prompt
        // to a session whose budget was already full — the summary itself was enlarging the
        // overflow.
        //
        // IN A SEPARATE TASK: so that `stop()` can cancel it. While only `productionTask` was
        // cancelled, the seconds waited here flowed on despite the "stop".
        let task = Task { @MainActor () -> String? in
            let summarizer = LanguageModelSession {
                "You summarize conversations. Reply with ONE short paragraph, no preamble."
            }
            guard !Task.isCancelled,
                  let summary = try? await summarizer.respond(
                      to: "Conversation:\n\(history)\n\nSummarize it in one short paragraph.").content,
                  !Task.isCancelled,
                  !summary.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            else { return nil }
            return summary
        }
        summaryTask = task
        let summary = await task.value
        if productionNumber == kind { summaryTask = nil }

        // If it was cancelled or the summarisation failed, THE CONTEXT IS NOT DROPPED
        // SILENTLY: the raw tail is carried under every condition (the assistant losing its
        // memory is an invisible fault for the user, the worst kind of error).
        guard let summary, productionNumber == kind else { return rawTail }

        // THE SUMMARY IS NOT CARRIED ALONE. The one producing the summary is a 3B model too,
        // and entrusting facts to it violates lesson 1 of this project: NUMBERS such as a
        // prayer time, an exchange rate or a departure time get rounded, dropped, or turned
        // into another plausible-looking number while being summarised — and the profile
        // change happens on exactly those turns ("find the times" → "make a table").
        // The raw text of the last turns is appended verbatim: the summary carries the
        // context, the raw tail carries the facts. Because `turnText` does not take tool
        // output into the context, the numbers stand only in the assistant's own sentences;
        // that was exactly what was being lost. The cost is ~1200 characters (≈400 tokens),
        // bounded.
        let outcome = summary + "\n\nRecent turns (verbatim — use exactly as written):\n"
            + Self.truncate(lastTurns, 1200)
        summaryCache = (history, outcome)
        return outcome
    }

    /// Extracts plain text from a transcript entry. Only user/assistant turns are taken; tool
    /// calls and outputs are not carried as context (they are bulky and can be called again
    /// in the new session).
    private static func turnText(_ input: Transcript.Entry) -> String? {
        func plain(_ segments: [Transcript.Segment]) -> String {
            segments.compactMap { if case .text(let t) = $0 { return t.content } else { return nil } }
                .joined(separator: " ")
                .trimmingCharacters(in: .whitespacesAndNewlines)
        }
        switch input {
        case .prompt(let p):
            let m = plain(p.segments)
            return m.isEmpty ? nil : "User: " + truncate(m, 400)
        case .response(let r):
            // The leakage must not enter THE SUMMARY either: if it does, the model takes it
            // for valid output in its own history and repeats it in the next session.
            let m = stripToolLeak(plain(r.segments))
            return m.isEmpty ? nil : "Assistant: " + truncate(m, 400)
        default:
            return nil
        }
    }

    private static func truncate(_ text: String, _ limit: Int) -> String {
        text.count <= limit ? text : String(text.suffix(limit))
    }

    // MARK: - The tool-call leakage filter

    /// Strips an unparsed tool-call payload out of the text.
    ///
    /// When FoundationModels cannot recognise a call block it passes it through as a plain
    /// TEXT segment; without the filter `{"name": "calculate", …}<executable_end>` went
    /// straight to the user. Worse, it fed itself: because the leakage is a `.response` text,
    /// `summarize()` carried it into the summary, and in a new session the model saw
    /// tool-call syntax in its own history as "valid assistant output" and copied IRRELEVANT
    /// payloads (a `calculate` for a lactose question, a `contact("Ali")` for a privacy
    /// question). That is why the filter is applied both to the text going to the user and to
    /// `turnText`.
    ///
    /// It can return empty — the caller must treat that as "the turn came to nothing".
    static func stripToolLeak(_ text: String) -> String {
        var m = text
        // 1) ```function … ``` / … <executable_end> blocks (together with their body).
        m = delete("(?s)```[ \\t]*function\\b.*?(?:```|<executable_end>|\\z)", m)
        // 2) A bare JSON tool call, single or inside a [ … ] array.
        m = delete("(?s)\\[?\\s*\\{\\s*\"name\"\\s*:\\s*\"[^\"]*\"\\s*,\\s*\"arguments\"\\s*:\\s*\\{.*?\\}\\s*\\}\\s*\\]?", m)
        m = delete("<executable_(?:end|start)>", m)
        // 3) Orphan closing leftovers are cleaned ONLY if a call was really stripped above.
        //    Applied unconditionally it would also delete the closing ``` line of a LEGITIMATE
        //    markdown code block and corrupt the answer — the code skill produces exactly such
        //    blocks.
        if m != text {
            m = delete("(?m)^\\s*(?:\\]|```)\\s*$", m)
        }
        return m.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func delete(_ pattern: String, _ text: String) -> String {
        guard let re = try? NSRegularExpression(pattern: pattern) else { return text }
        return re.stringByReplacingMatches(in: text,
                                           range: NSRange(text.startIndex..., in: text),
                                           withTemplate: "")
    }
}
