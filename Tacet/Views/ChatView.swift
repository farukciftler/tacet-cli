//
//  ChatView.swift
//  Tacet
//
//  The single continuous chat stream (spec §8 v1). It joins the components, the
//  ModelService and the SwiftData history. Tool chips land above their reply.
//

import Foundation
import SwiftUI
import SwiftData
import UniformTypeIdentifiers

struct ChatView: View {
    let chat: Chat
    let service: ModelService
    var openHistory: () -> Void = {}
    var newChat: () -> Void = {}

    @Environment(\.modelContext) private var record
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// The saved MCP connections (mcp §5.4). The view DRAWS nothing with these — since
    /// this is the layer that knows SwiftData, the tools are fed to the service from here.
    /// When the list changes (add/delete/summary refresh) the tools are refreshed too.
    @Query private var connections: [Connection]

    @State private var question = ""
    /// The live stream state. The text does NOT sit at the ROOT as `@State`: on every
    /// token chunk the top bar, the input field and all visible history rows in the
    /// LazyVStack were re-evaluated. Now only `LiveBlock` observes this object; the root's
    /// body never touches the stream text.
    @State private var live = LiveStream()
    @State private var answering = false
    @State private var documentPicker = false
    @State private var preview: PreviewItem?
    /// The in-flight reply task. It is cancelled when the chat changes / the view closes;
    /// otherwise an unstructured Task could try to write to a deleted Chat object.
    @State private var replyTask: Task<Void, Never>?
    /// The operation error shown to the user (copying a document, saving, exporting).
    /// It replaces the silently swallowed `try?`s.
    @State private var warningText: String?
    /// A counter used to fire a haptic when the reply completes.
    @State private var completionCounter = 0
    @FocusState private var inputFocused: Bool

    /// The timeline of the running turn (timeline-spec §5.2). ONE RECORDER: it is owned by
    /// `ModelService`, the view only observes it and binds events. There used to be a
    /// separate `@State` instance here; it lived in parallel with the service's recorder
    /// and their steps could diverge (a single-source-of-truth violation).
    private var recorder: TimelineRecorder { service.timeline }

    /// The meaningful signature of the connection list: id + the tool names that could
    /// enter the session + the device-data setting. Only when these change are the tools
    /// rebuilt; fields that change on every call, such as "last used", do not enter the
    /// signature. The device-data setting MUST BE in the signature: when the user switches
    /// between "never" ↔ "ask every time" in ConnectionDetail, if the tools are not
    /// rebuilt the old MCPTool instances stay in the session with the old setting.
    private var connectionSignature: [String] {
        connections.map {
            "\($0.id)·\($0.deviceData.rawValue)·\($0.availableTools.map(\.name).joined(separator: ","))"
        }
    }

    /// Is production running right now — the service is the single source of truth, the
    /// live stream is a local flag.
    private var isProducing: Bool { service.isProducing || answering }

    /// The "close to the bottom" distance at which auto-scrolling kicks in (points).
    /// Above that, the user is deliberately reading the history.
    private let bottomThreshold: CGFloat = 96

    /// An Identifiable URL wrapper for the QuickLook sheet.
    private struct PreviewItem: Identifiable { let url: URL; var id: String { url.path } }

    /// The document types supported for reading/editing.
    private var documentTypes: [UTType] {
        [.pdf, .plainText, .text,
         UTType(filenameExtension: "xlsx") ?? .data,
         UTType(filenameExtension: "docx") ?? .data,
         UTType(filenameExtension: "md") ?? .plainText]
    }

    var body: some View {
        core
        .sheet(item: $preview, onDismiss: {
            // If it is not reset, onChange does not fire when the same document is
            // produced a second time.
            service.documentContext.toPreview = nil
        }) { item in
            DocumentPreviewSheet(url: item.url)
        }
        .onChange(of: service.documentContext.toPreview) { _, new in
            if let new { preview = PreviewItem(url: new) }
        }
        .sensoryFeedback(.success, trigger: completionCounter)
        .issueBanner($warningText)
    }

    /// Split out so the type checker does not have to solve the whole modifier chain
    /// as one expression (it timed out otherwise).
    @ViewBuilder private var core: some View {
        VStack(spacing: 0) {
            VStack(spacing: 0) {
                TopBar(state: service.state, description: service.blockMessage,
                       openHistory: openHistory, newChat: newChat)

                // `isEmpty` DOES NOT sort — we have no business with order here.
                if chat.isEmpty && !answering {
                    EmptyState { example in
                        question = example
                        inputFocused = true
                    }
                    .frame(maxHeight: .infinity)
                } else {
                    stream
                }
            }
            // Tapping anywhere outside the input field dismisses the keyboard; the typed
            // text is not lost because it lives in `question`. Because buttons take
            // priority (not simultaneous), picking an example and the chips are unaffected.
            .contentShape(Rectangle())
            .onTapGesture { inputFocused = false }

            inputBar
        }
        .background(Palette.background)
        .task {
            service.prepare()   // warm the model once the chat is visible (prewarm, report §5.1)
            // Bind the MCP tools to the session. With no connection, the network is never
            // touched.
            service.refreshConnections(connections)
        }
        .onChange(of: connectionSignature) { _, _ in service.refreshConnections(connections) }
        // Every new chip that lands in the executor is a tool step (timeline-spec §5.2).
        // It is bound by observing the single source of truth, without touching the tool
        // layer. This is the EVENT-based binding the spec asks for: it fires the moment
        // `ToolExecutor.begin` drops the chip, it does not wait for a chunk boundary.
        .onChange(of: service.executor.traces.map(\.id)) { _, _ in syncToolSteps() }
        // NOTE: the writing step used to be opened here with
        // `onChange(of: liveReply.isEmpty)`. That meant the root OBSERVED the stream text
        // (the whole view was re-evaluated on every chunk). The trigger was moved into the
        // stream closure inside `generate(_:)` — same behaviour, no cost.
        .onChange(of: chat.id) { _, _ in cancelTask() }
        .onDisappear { cancelTask() }
        .fileImporter(isPresented: $documentPicker,
                      allowedContentTypes: documentTypes,
                      allowsMultipleSelection: false) { result in
            documentSelected(result)
        }
    }

    /// The attached-document chip + the input field. Split out of `core` so the type
    /// checker does not solve one oversized expression (it timed out otherwise).
    @ViewBuilder private var inputBar: some View {
            VStack(spacing: Spacing.s2) {
                if let document = service.documentContext.activeDocument {
                    AttachedDocumentChip(document: document) {
                        service.documentContext.removeDocument()
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, Spacing.s5)
                    .transition(reduceMotion ? .opacity : .opacity.combined(with: .offset(y: 2)))
                }
                InputField(text: $question, send: send,
                           add: { documentPicker = true },
                           isProducing: isProducing,
                           stop: stopPressed)
                    .focused($inputFocused)
            }
            .padding(.vertical, Spacing.s2)
    }

    /// Makes a SwiftData write failure visible to the user. When `try? save()` was
    /// swallowed silently, the message appeared on screen but vanished when the app closed.
    /// NO rollback: let the message stay in memory so the user sees what they wrote.
    private func save() {
        record.boardSave(String(localized: "The chat could not be saved"), warning: $warningText)
    }

    /// Copies the selected document from the security scope into the app's area and makes
    /// it active.
    private func documentSelected(_ result: Result<[URL], Error>) {
        guard case .success(let urls) = result, let source = urls.first else { return }
        let access = source.startAccessingSecurityScopedResource()
        defer { if access { source.stopAccessingSecurityScopedResource() } }

        // The user's OWN document passes through here — the most sensitive path. The
        // subfolder is created with a protection class too and excluded from backups; even
        // though iOS would inherit it, the class is stamped explicitly on the copied file
        // (the source file's attributes can travel with the copy).
        let targetFolder = DocumentContext.outputFolder().appendingPathComponent("Attached", isDirectory: true)
        try? FileManager.default.createDirectory(
            at: targetFolder, withIntermediateDirectories: true,
            attributes: [.protectionKey: FileProtectionType.completeUnlessOpen])
        DocumentContext.applyProtection(targetFolder)
        let target = targetFolder.appendingPathComponent(source.lastPathComponent)
        try? FileManager.default.removeItem(at: target)
        do {
            try FileManager.default.copyItem(at: source, to: target)
            DocumentContext.applyProtection(target)
            withAnimation(reduceMotion ? nil : .easeOut(duration: 0.2)) {
                service.documentContext.addDocument(url: target)
            }
        } catch {
            // If this were passed over silently the user would think the file was
            // attached — give a visible error.
            warningText = String(localized: "Couldn’t attach the document: \(error.localizedDescription)")
        }
    }

    // MARK: - Stream

    private var stream: some View {
        // ONE sort. `daySeparatorNeeded` used to touch `orderedMessages` three times per
        // row on top of the `ForEach`: (row count × 3 + 1) O(n log n) sorts on every body
        // evaluation.
        let ordered = chat.orderedMessages
        return ScrollViewReader { reader in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: Spacing.messageGap) {
                    ForEach(Array(ordered.enumerated()), id: \.element.id) { index, message in
                        // The day-separator decision by index: the array is not rescanned.
                        if daySeparatorNeeded(ordered, index) {
                            DateSeparator(date: message.createdAt)
                                .frame(maxWidth: .infinity)
                        }
                        row(message)
                            .id(message.id)
                    }

                    if answering {
                        LiveBlock(live: live,
                                  executor: service.executor,
                                  recorder: recorder,
                                  reduceMotion: reduceMotion,
                                  textStreamed: { scrollToBottom(reader, force: false) })
                            .id(liveID)
                    }
                }
                .padding(.horizontal, Spacing.s5)
                .padding(.vertical, Spacing.s4)
            }
            .scrollDismissesKeyboard(.interactively)
            // Is the user close to the bottom? The flag is `@ObservationIgnored` — the body
            // is not rebuilt during scrolling, only the scroll decision reads it.
            .onScrollGeometryChange(for: Bool.self) { geometry in
                let remaining = geometry.contentSize.height + geometry.contentInsets.bottom
                    - geometry.containerSize.height - geometry.contentOffset.y
                return remaining <= bottomThreshold
            } action: { _, near in
                live.nearBottom = near
            }
            // A new message was added: if the user is reading further up, do not disturb them.
            .onChange(of: chat.messages.count) { _, _ in scrollToBottom(reader, force: false) }
            // The user sent a new question — going to the bottom here is what they want.
            .onChange(of: answering) { _, new in if new { scrollToBottom(reader, force: true) } }
        }
    }

    /// The row of a finished message. `message.traces`/`message.steps` are read ONCE
    /// (they used to be read 4-5 times per row, each a separate JSON decode).
    private func row(_ message: Message) -> some View {
        HistoryRow(id: message.id,
                   role: message.role,
                   content: message.content,
                   traces: message.traces,
                   steps: message.steps,
                   isError: message.isError,
                   reduceMotion: reduceMotion,
                   downloadTable: downloadTable,
                   retry: message.isError && message.isRetryable
                       ? { retry(message) } : nil)
            // The BODY of a row whose value has not changed is not rebuilt: while the
            // stream runs, the markdown/table parsing of the visible history is not
            // repeated for nothing.
            .equatable()
    }

    private let liveID = "live-block"

    // MARK: - Action

    private func send() {
        let text = question.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, !isProducing else { return }
        question = ""

        let userMessage = Message(role: .user, content: text)
        userMessage.chat = chat
        withAnimation(reduceMotion ? nil : .easeOut(duration: 0.2)) {
            record.insert(userMessage)
        }
        // Derive the chat title from the first user message (via the flag — comparing text
        // broke under translation).
        chat.deriveTitle(text)
        chat.updatedAt = Date()
        save()

        generate(text)
    }

    /// "Try again" in the error block: deletes the error record and asks the same question
    /// once more. The user bubble is not added again.
    private func retry(_ errorMessage: Message) {
        guard !isProducing else { return }
        let ordered = chat.orderedMessages
        guard let index = ordered.firstIndex(where: { $0.id == errorMessage.id }) else { return }
        // The SwiftData trap: compute the value you need BEFORE deleting.
        guard let asked = ordered[..<index].last(where: { $0.role == .user })?.content else { return }

        withAnimation(reduceMotion ? nil : .easeOut(duration: 0.2)) {
            record.delete(errorMessage)
        }
        save()
        generate(asked)
    }

    /// Starts reply production. The in-flight task is held in `replyTask`; it is cancelled
    /// when the chat changes, otherwise the background task writes to the old chat.
    private func generate(_ text: String) {
        answering = true
        live.text = ""
        recorder.reset()

        replyTask?.cancel()
        replyTask = Task {
            let outcome = await service.replyOutcome(text) { partial in
                guard !Task.isCancelled else { return }
                // The first text chunk opens the writing step (timeline-spec §5.4).
                // Since the root no longer observes the stream text, the trigger is here.
                if live.text.isEmpty && !partial.isEmpty { writingStep() }
                live.text = partial
            }
            // Before writing: was the task cancelled, is the chat still valid?
            // If the history was cleared or the chat deleted, exit silently — writing an
            // orphaned or partial record leads to a fatal error in SwiftData.
            guard !Task.isCancelled, !chat.isDeleted, chat.modelContext != nil else {
                recorder.interrupt()
                answering = false
                live.text = ""
                return
            }

            // A transient status notice is not a persistent reply — do not save it.
            if outcome.isTransient {
                recorder.interrupt()
                answering = false
                live.text = ""
                return
            }

            recorder.finish()
            // The error information comes from the service as a FLAG; there is no text
            // comparison.
            let reply = Message(role: .tacet, content: outcome.text,
                                traces: outcome.traces,
                                steps: TimelineLedger.persistent(recorder.steps,
                                                                 traces: outcome.traces),
                                isError: outcome.isError,
                                isRetryable: outcome.isRetryable)
            reply.chat = chat
            withAnimation(reduceMotion ? nil : .easeOut(duration: 0.2)) {
                record.insert(reply)
                answering = false
                live.text = ""
            }
            chat.updatedAt = Date()
            save()
            completionCounter += 1
        }
    }

    /// The stop button: cancels production and saves the partial reply so the text that has
    /// streamed so far is not lost.
    private func stopPressed() {
        service.stop()
        let partial = live.text.trimmingCharacters(in: .whitespacesAndNewlines)
        // The interruption does not vanish silently: the last step is closed as "left
        // halfway" and shows up in the folding row (timeline-spec §3.4).
        recorder.interrupt()
        let steps = recorder.steps
        replyTask?.cancel()
        replyTask = nil
        withAnimation(reduceMotion ? nil : .easeOut(duration: 0.2)) {
            answering = false
            live.text = ""
        }
        guard !partial.isEmpty, !chat.isDeleted, chat.modelContext != nil else { return }
        let traces = service.executor.traces
        let reply = Message(role: .tacet, content: partial, traces: traces,
                            steps: TimelineLedger.persistent(steps, traces: traces))
        reply.chat = chat
        record.insert(reply)
        chat.updatedAt = Date()
        save()
    }

    /// Cancels the in-flight reply task and resets the live stream state.
    private func cancelTask() {
        // Cancelling the outer task is not enough: ModelService.productionTask keeps
        // running, `isProducing` stays stuck at true, and the send button would freeze in
        // the new chat.
        service.stop()
        // The interrupted turn is recorded; the recorder is closed so no half-open step is
        // left. If it is called while nothing is being produced (view teardown), an
        // interruption step is not invented.
        if answering { recorder.interrupt() }
        replyTask?.cancel()
        replyTask = nil
        answering = false
        live.text = ""
    }

    // MARK: - Timeline

    /// Binds the new chips landing in the executor as tool steps. It only ADDS — the
    /// recorder does not rewrite history; a trace that disappears (an approved approval
    /// chip) is filtered out at the display and persistence stage.
    ///
    /// `ModelService.syncTimeline` does the same job at chunk boundaries (a safety net for
    /// view-less calls). Because both deduplicate by `traceID` and write to the SAME
    /// recorder, no duplicate step is created.
    private func syncToolSteps() {
        guard answering else { return }
        let bound = Set(recorder.steps.compactMap(\.toolTraceID))
        for trace in service.executor.traces where !bound.contains(trace.id) {
            recorder.bindTool(traceID: trace.id)
        }
    }

    /// Opens the writing step when the first text chunk arrives. If it is already open it
    /// is left alone — writing is a single step, no step is produced per chunk
    /// (timeline-spec §5.4).
    private func writingStep() {
        guard answering else { return }
        guard recorder.steps.last?.kind != .writing else { return }
        recorder.begin(kind: .writing, text: TimelineLedger.writingText)
    }

    /// "Download Excel" from an in-chat table: produces the table as .xlsx and opens the
    /// preview.
    private func downloadTable(_ table: Table) {
        do {
            let url = try ExcelEngine().write(fileName: "table", title: nil,
                                              body: nil, table: table,
                                              folder: DocumentContext.outputFolder())
            // The same protection as the other production paths: unreadable while locked +
            // excluded from backups.
            DocumentContext.applyProtection(url)
            preview = PreviewItem(url: url)
        } catch {
            // If it returned silently, the button would be pressed and nothing would happen.
            warningText = String(localized: "Couldn’t export the table: \(error.localizedDescription)")
        }
    }

    // MARK: - Helpers

    /// The day-separator decision: it compares only against the PREVIOUS element. The
    /// sorted array comes from outside — no re-sorting or rescanning happens here.
    private func daySeparatorNeeded(_ ordered: [Message], _ index: Int) -> Bool {
        guard index < ordered.count else { return false }
        if index == 0 { return true }
        let previous = ordered[index - 1].createdAt
        let current = ordered[index].createdAt
        return !Calendar.current.isDate(previous, inSameDayAs: current)
    }

    /// The id of the last message, used as the scroll target. It DOES NOT do a full sort —
    /// it is called at action time and never enters body evaluation.
    private var lastMessageID: UUID? {
        var last: Message?
        for message in chat.messages {
            if let current = last, current.createdAt > message.createdAt { continue }
            last = message
        }
        return last?.id
    }

    /// Pulls the stream to the bottom — BUT only if the user is already at the bottom.
    ///
    /// An animated `scrollTo` used to be called on every token chunk: overlapping
    /// animations produced stutter, and worse, while the user scrolled up to read an old
    /// message every chunk dragged them back down. Now stream scrolling is unanimated and
    /// at most every ~0.1 s; `force` is used only for the user's own action (sending a
    /// question).
    private func scrollToBottom(_ reader: ScrollViewProxy, force: Bool) {
        guard force || live.nearBottom else { return }
        let now = Date()
        if !force {
            guard now.timeIntervalSince(live.lastScroll) >= 0.1 else { return }
        }
        live.lastScroll = now
        // `AnyHashable(lastMessageID)` WAS WRONG: it wraps `UUID?` and produces an
        // `Optional<UUID>` box, while the rows' identity is `UUID`. Because the two never
        // matched, scrolling silently did nothing for the message that arrived after the
        // stream finished. The Optional must be UNWRAPPED before wrapping.
        let target: AnyHashable? = answering ? AnyHashable(liveID)
                                             : lastMessageID.map(AnyHashable.init)
        guard let target else { return }
        if force && !reduceMotion {
            withAnimation(.easeOut(duration: 0.2)) {
                reader.scrollTo(target, anchor: .bottom)
            }
        } else {
            reader.scrollTo(target, anchor: .bottom)
        }
    }
}

// MARK: - Live stream

/// The carrier of the streaming reply. Instead of `@State` at the root, ONLY `LiveBlock`
/// observes this reference object; that way the top bar, the input field and the visible
/// history rows are not re-evaluated on every token chunk.
@MainActor
@Observable
final class LiveStream {
    /// The streaming reply text. The ONLY observed field.
    var text: String = ""
    /// Is the user close to the bottom. Not observed: there is no point rebuilding the body
    /// on every scroll gesture, the value is only read in the scroll decision.
    @ObservationIgnored var nearBottom = true
    /// The moment of the last automatic scroll — the throttle for stream scrolling.
    @ObservationIgnored var lastScroll: Date = .distantPast
}

/// The live (streaming) assistant block: side-effect chips + the timeline ribbon + the
/// streaming text.
///
/// The ribbon first sits IN THE PLACE of the reply bubble (there is no text yet); once
/// writing starts and text begins to stream, by staying in the same place it ends up pulled
/// ABOVE the reply (timeline-spec §3.1).
///
/// Being a SEPARATE view is deliberate: this is the only place that observes the stream
/// text, so invalidation is confined here.
private struct LiveBlock: View {
    let live: LiveStream
    let executor: ToolExecutor
    let recorder: TimelineRecorder
    let reduceMotion: Bool
    /// Called as the text streams; the scroll decision is made in the parent view.
    let textStreamed: () -> Void

    private var hasRibbon: Bool { !recorder.steps.isEmpty }

    private var chipTransition: AnyTransition {
        reduceMotion ? .opacity : .opacity.combined(with: .offset(y: 2))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.chipReplyGap) {
            ForEach(ReplyTraces.liveChips(executor.traces, hasRibbon: hasRibbon)) { trace in
                ToolChip(trace: trace, executor: executor)
                    .transition(chipTransition)
            }
            // That the chip can be tapped is explained once. It is NOT drawn while there is
            // NO trace: without the thing it describes on screen, the hint has no referent.
            if !executor.traces.isEmpty {
                ToolHint()
            }
            if hasRibbon {
                TimelineRibbon(steps: recorder.steps, traces: executor.traces)
            }
            // While the ribbon is present and there is still no text, the breath dot is not
            // drawn — the ribbon is holding the reply's place, and two live indicators do
            // not stand side by side.
            if !hasRibbon || !live.text.isEmpty {
                TacetReply(text: live.text, isStreaming: true,
                           fileTraces: ReplyTraces.cards(executor.traces))
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .onChange(of: live.text) { _, _ in textStreamed() }
    }
}

// MARK: - History row

/// The row of a finished message. Being `Equatable` is the whole point: even if the parent
/// view is re-evaluated while the stream runs, the BODY of a row whose value has not
/// changed is not built — markdown and table parsing are not repeated for nothing.
///
/// It carries resolved values, NOT the model object: because `Message` is a reference,
/// identity comparison does not mean "the content did not change", and this way
/// `traces`/`steps` are read once per row.
///
/// The closures do not take part in the comparison: because they are recreated on every
/// body evaluation they would never compare equal and would defeat the cache entirely.
private struct HistoryRow: View, Equatable {
    let id: UUID
    let role: Role
    let content: String
    let traces: [ToolTrace]
    let steps: [TimelineStep]
    let isError: Bool
    let reduceMotion: Bool
    let downloadTable: (Table) -> Void
    let retry: (() -> Void)?

    static func == (left: HistoryRow, right: HistoryRow) -> Bool {
        left.id == right.id
            && left.role == right.role
            && left.content == right.content
            && left.isError == right.isError
            && left.reduceMotion == right.reduceMotion
            && (left.retry == nil) == (right.retry == nil)
            && left.traces == right.traces
            && left.steps == right.steps
    }

    private var chipTransition: AnyTransition {
        reduceMotion ? .opacity : .opacity.combined(with: .offset(y: 2))
    }

    var body: some View {
        switch role {
        case .user:
            UserBubble(text: content)
                .frame(maxWidth: .infinity, alignment: .trailing)
        case .tacet:
            // In a message that has step data, the read chips go inside the fold; in one
            // that does not (an old message) all the chips are visible as they are today.
            let hasTimeline = TimelineFolding.showsRow(steps)
            let cards = ReplyTraces.cards(traces)
            VStack(alignment: .leading, spacing: Spacing.chipReplyGap) {
                if hasTimeline {
                    TimelineLine(steps: steps, traces: traces)
                }
                ForEach(ReplyTraces.chips(traces, hasTimeline: hasTimeline)) { trace in
                    ToolChip(trace: trace)
                        .transition(chipTransition)
                }
                if !content.isEmpty || !cards.isEmpty {
                    TacetReply(text: content,
                               downloadTable: downloadTable,
                               isError: isError,
                               retry: retry,
                               fileTraces: cards)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

// MARK: - Timeline ledger (pure functions, covered by SelfTest)

/// Turns the live step list into the persistent list written to the message.
enum TimelineLedger {
    /// The text of the live writing step — the present, real time.
    static var writingText: String { String(localized: "writing") }
    /// In a finished turn the same step goes to the past tense; no drama, only time.
    static var writtenText: String { String(localized: "written") }

    /// - A tool step whose trace disappeared is dropped: the executor removes an approved
    ///   approval chip from the stream (mcp §3.3); we do not leave a textless, untappable
    ///   ghost row behind.
    /// - The writing step is turned into the past tense.
    static func persistent(_ steps: [TimelineStep], traces: [ToolTrace]) -> [TimelineStep] {
        let present = Set(traces.map(\.id))
        return steps.compactMap { step in
            if step.kind == .tool {
                guard let id = step.toolTraceID, present.contains(id) else { return nil }
                return step
            }
            if step.kind == .writing, step.text == writingText {
                var copy = step
                copy.text = writtenText
                return copy
            }
            return step
        }
    }
}

#Preview {
    let container = try! ModelContainer(for: Chat.self, Message.self, Connection.self,
                                        configurations: .init(isStoredInMemoryOnly: true))
    let chat = Chat(title: "Example")
    container.mainContext.insert(chat)
    return ChatView(chat: chat, service: ModelService())
        .modelContainer(container)
}
