//
//  ContentView.swift
//  Tacet
//
//  Root view. Manages the active chat, the shared ModelService and the history
//  list. Opening a new chat and reaching the old ones happens here (spec §4.7).
//

import SwiftUI
import SwiftData

struct ContentView: View {
    @Environment(\.modelContext) private var record
    @Environment(\.scenePhase) private var scenePhase
    @Query(sort: \Chat.updatedAt, order: .reverse) private var chats: [Chat]

    @Query private var skills: [UserSkill]
    /// Memory notes (memory-spec §5): the read path refreshes the store from this array.
    @Query private var notes: [MemoryNote]

    @State private var service = ModelService()
    /// Extraction (the write path) goes through a single instance: at most one
    /// session opening on back-to-back triggers depends on the guard inside this
    /// instance (memory-spec §4.1).
    @State private var memory = MemoryService()
    /// The counterpart of the contract the connection board expects — see end of file.
    @State private var connectionBridge = ConnectionBridge()
    @State private var activeID: UUID?
    /// A single presentation channel: only one sheet can be open at a time. With
    /// separate `isPresented` flags, "close + open" landed in the same runloop turn
    /// and the second presentation was silently dropped.
    @State private var sheet: Sheet?
    /// The user-visible counterpart of a write/run failure. Nil means no warning.
    @State private var warningText: String?
    /// Store state: the malformed-store backup notice can be read once and dismissed.
    @State private var backupNoticeDismissed = false
    /// The job picked in the welcome screen. Single shot: nil'ed the moment it is delivered.
    @State private var launchPrompt: String?

    /// @MainActor: StoreState lives on the main actor; a View's non-body members are
    /// not isolated, so the helpers that reach it are marked explicitly.
    @MainActor private var store: StoreState { StoreState.shared }

    /// The active chat — if none is selected, the most recently updated one.
    private var active: Chat? {
        if let activeID, let found = chats.first(where: { $0.id == activeID }) {
            return found
        }
        return chats.first
    }

    var body: some View {
        VStack(spacing: 0) {
            // Unusual store conditions sit ABOVE the chat: the user must know whether
            // the session is being saved before starting to write.
            storeWarnings
            Group {
                if let active {
                    ChatView(
                        chat: active,
                        service: service,
                        openHistory: { sheet = .history },
                        newChat: startNewChat
                    )
                    .id(active.id)   // when the chat changes, view state (question, stream) resets
                } else {
                    Color.clear.onAppear(perform: ensureFirstChat)
                }
            }
        }
        .background(Palette.background)
        .task {
            // Introduce the user's skills to the model at launch (also refreshed as the
            // board saves).
            SkillStore.reloadUser(skills)
            // The connection board writes to SwiftData through the bridge.
            connectionBridge.record = record
            // The store is empty at launch: unless it is filled, the first turn would
            // pass without injection.
            MemoryStore.reload(notes)
            service.reloadAvailability()
            // A request coming from Siri/Shortcuts is handled BEFORE the welcome screen:
            // the user asked for a concrete job, onboarding must not cut in.
            let intentArrived = handleIntent()
            // The welcome screen opens through the single presentation channel. If some
            // other sheet is already open (deep link, coming back), it does not cut in.
            if sheet == nil, !intentArrived, WelcomeSetting.showAtLaunch {
                sheet = .welcome
            }
        }
        // On a cold launch `perform()` usually runs AFTER `.task`; that is why the inbox
        // is also observed. Since `consume()` is single shot, the two paths do not collide.
        .onChange(of: IntentInbox.shared.pending) { _, new in
            guard new != nil else { return }
            _ = handleIntent()
        }
        .onChange(of: scenePhase) { _, new in
            guard new == .active else {
                // Going to the background is one of extraction's two triggers
                // (memory-spec §4.1). It NEVER runs INSIDE a chat turn; this place is
                // outside the turn.
                memory.trigger(chat: active, record: record)
                return
            }
            // Refresh when the app comes back to the front too — .task only runs the
            // first time. If the model download finished in the background, be ready
            // without a restart.
            service.reloadAvailability()
            // Let notes extracted in the background enter the read path.
            MemoryStore.reload(notes)
        }
        .sheet(item: $sheet) { open in
            switch open {
            case .history:
                ChatList(
                    chats: chats,
                    activeID: active?.id,
                    select: { c in
                        // A chat switch is extraction's other trigger: the chat being
                        // LEFT is processed, not the one being selected. Order matters —
                        // before activeID changes.
                        memory.trigger(chat: active, record: record)
                        activeID = c.id
                        service.resetChat()
                        sheet = nil
                    },
                    delete: deleteChat,
                    new: {
                        sheet = nil
                        startNewChat()
                    },
                    // A single assignment: the list closes and the target sheet is
                    // presented in the same transition.
                    openSkills: { sheet = .skills },
                    openMemory: { sheet = .memory },
                    openSettings: { sheet = .settings }
                )
            case .skills:
                SkillBoard()
            case .memory:
                MemoryBoard()
            case .connections:
                ConnectionBoard(service: connectionBridge)
            case .settings:
                Settings(clearHistory: clearHistory,
                         documentContext: service.documentContext,
                         openConnections: { sheet = .connections },
                         openWelcome: { sheet = .welcome })
            case .welcome:
                // State/block are passed BY VALUE; since the service is @Observable, the
                // sheet refreshes if the model download finishes while it is open.
                Welcome(state: service.state,
                        block: service.block,
                        jobSelected: { launchPrompt = $0 })
            }
        }
        // The picked job lands in the chat. It is not sent directly: while the model is
        // not ready, the reply would be dropped without being saved and the user would
        // experience "I tapped it and nothing happened". The job is written into the
        // input field through the SAME path as the empty state's example chips; the user
        // decides whether to send.
        .onChange(of: launchPrompt) { _, new in
            guard let new, !new.isEmpty else { return }
            launchPrompt = nil
            WelcomeBridge.shared.release(new)
            // If the empty state is not on screen the job would vanish silently: an
            // empty chat is guaranteed (if the active chat is already empty, no new one
            // is opened).
            startNewChat()
        }
        .issueBanner($warningText)
    }

    // MARK: - Store warnings

    /// Two conditions around the persistent store. No badges or dots: the state is told
    /// in words.
    @MainActor @ViewBuilder
    private var storeWarnings: some View {
        // PERSISTENT warning: this session is not being written to disk. It cannot be
        // dismissed, because it is a fact the user has to know at every moment —
        // everything they write will be lost when they close.
        if store.sessionNotPersistent {
            storeRibbon(
                title: Text("This session isn’t being saved."),
                description: Text("The store couldn’t be opened. What you write stays only on screen; when you close Tacet, these conversations are gone."),
                emphasised: true
            )
        }

        // Informational: the old data was not deleted, it was backed up. Read once and
        // dismissed.
        if let backup = store.backedUpStore, !backupNoticeDismissed {
            storeRibbon(
                title: Text("Your old data was backed up."),
                description: Text("The previous store couldn’t be opened, so it was kept as “\(backup)” and Tacet started with a clean store. Nothing was deleted."),
                emphasised: false,
                close: { backupNoticeDismissed = true }
            )
        }
    }

    @MainActor private func storeRibbon(title: Text,
                                        description: Text,
                                        emphasised: Bool,
                                        close: (() -> Void)? = nil) -> some View {
        HStack(alignment: .top, spacing: Spacing.s3) {
            VStack(alignment: .leading, spacing: Spacing.s1) {
                title
                    .font(Typography.user())
                    .foregroundStyle(emphasised ? Palette.error : Palette.ink)
                description
                    .font(Typography.chip())
                    .foregroundStyle(Palette.grey)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            if let close {
                Button(action: close) {
                    Text("Close")
                        .font(Typography.chip())
                        .foregroundStyle(Palette.grey)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, Spacing.s5)
        .padding(.vertical, Spacing.s3)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Palette.background)
        .overlay(alignment: .bottom) {
            Rectangle()
                .fill(Palette.divider)
                .frame(height: Spacing.hairline)
        }
        .accessibilityElement(children: .combine)
    }

    // MARK: - App Intents connection

    /// Applies the pending request in `IntentInbox`. This is the App Intents layer's
    /// ONLY attachment point to the app: opening intents (class A) do not make their own
    /// model calls, they leave a request here.
    ///
    /// Returns: whether a request was handled. On a cold launch the caller decides
    /// whether the welcome screen cuts in by looking at this value.
    @discardableResult
    @MainActor private func handleIntent() -> Bool {
        guard let request = IntentInbox.shared.consume() else { return false }
        // The user asked Siri for a concrete job: if the welcome screen is open it is
        // pulled away.
        if sheet == .welcome { sheet = nil }
        switch request {
        case .ask(let prompt):
            // The SAME path as the welcome job: the prompt is written into the input
            // field, the decision to send is the user's. Sending it directly meant
            // nothing happening on screen while the model was not ready.
            launchPrompt = prompt
        case .openChat(let id):
            // The chat may have been deleted after the intent ran; if it is gone, do
            // not touch it.
            guard chats.contains(where: { $0.id == id }) else { return true }
            // A chat switch is an extraction trigger: the chat being LEFT is processed.
            memory.trigger(chat: active, record: record)
            activeID = id
            service.resetChat()
        }
        return true
    }

    /// The sheets that can be presented from the root. All go through one channel with
    /// `.sheet(item:)`.
    private enum Sheet: Identifiable {
        case history, skills, memory, connections, settings, welcome
        var id: Self { self }
    }

    // MARK: - Chat management

    private func ensureFirstChat() {
        guard chats.isEmpty else { return }
        openEmptyChat()
    }

    /// Writes an empty chat and makes it active. Going through one place matters: after
    /// the history is cleared there must still be a valid chat on screen.
    private func openEmptyChat() {
        let new = Chat()
        record.insert(new)
        // If the write fails it is not rolled back: let the chat stay in memory so the
        // user is not left on a blank screen. Had it been rolled back, ensureFirstChat
        // would fire again and show the same error in an endless loop. That it was not
        // saved is said in the warning.
        save(String(localized: "Couldn’t save the new conversation"))
        activeID = new.id
    }

    private func startNewChat() {
        // The extraction trigger before leaving (memory-spec §4.1). An empty chat has no
        // message to process, the service returns early on its own.
        memory.trigger(chat: active, record: record)
        // If the active chat is already empty, do not open a new one — a single empty
        // chat is enough.
        if let active, active.isEmpty {
            activeID = active.id
            service.resetChat()
            return
        }
        let new = Chat()
        record.insert(new)
        save(String(localized: "Couldn’t save the new conversation"))
        activeID = new.id
        service.resetChat()
    }

    private func deleteChat(_ chat: Chat) {
        // Take the id before deleting — reaching a deleted object is a fatal error. The
        // next active one is also computed BEFORE the delete: the @Query array may not
        // have refreshed at the moment of deletion, and walking it afterwards touches a
        // deleted instance.
        let deletedID = chat.id
        let deletingActive = (deletedID == active?.id)
        let nextID = chats.first(where: { $0.id != deletedID })?.id
        record.delete(chat)
        do {
            try record.save()
        } catch {
            // The delete did not reach disk: we roll the context back so the list shows
            // the truth — the user must not think it was deleted and believe the chat
            // is lost.
            record.rollback()
            warningText = String(localized: "Couldn’t delete the conversation: \(error.localizedDescription) The conversation is still there.")
            return
        }
        // The chat is gone from disk; its memory caret now points at nothing. A BULK
        // reset would be wrong here — the other chats' carets are still standing.
        MemoryService.deleteCaret(chatID: deletedID)
        if deletingActive {
            activeID = nextID
            service.resetChat()
        }
    }

    /// Deletes the whole chat history. Generated documents and MEMORY remain — deleting
    /// those is the user's separate decision (memory-spec §7).
    /// Deleting memory is the Memory board's job; notes are not touched here.
    private func clearHistory() {
        // The objects to be deleted are held BEFORE deleting: the live @Query array may
        // not have refreshed at the moment of deletion, and walking it afterwards
        // touches a deleted instance (fatal). Messages go along through the cascade rule
        // on Chat.messages.
        let toDelete = chats
        activeID = nil
        service.resetChat()
        for chat in toDelete { record.delete(chat) }
        do {
            try record.save()
        } catch {
            // This was the most critical silence: with a failed write the user would
            // think their history had been deleted. We roll back and state the situation
            // plainly.
            record.rollback()
            warningText = String(localized: "Couldn’t clear the history: \(error.localizedDescription) Your conversations are untouched.")
            return
        }
        // The chats are gone; the processed-message carets now point at nothing. This
        // DOES NOT TOUCH the notes, it only empties the caret dictionary.
        MemoryService.resetCarets()
        // An empty chat is opened right away so a valid state stays on screen.
        openEmptyChat()
    }

    /// Makes a SwiftData write failure visible to the user (same pattern as
    /// ChatView.save). When `try? save()` was swallowed silently, data loss went
    /// unnoticed.
    private func save(_ cause: String) {
        record.boardSave(cause, warning: $warningText)
    }
}

// MARK: - Connection bridge

/// `ConnectionBoard` imposes the `ConnectionProbe` contract (returning an async result)
/// on the view layer; `ConnectionService` instead runs the attempt through @Observable
/// state (`attempt`) and asks for a `ModelContext` for `add/delete`. The two signatures
/// do not match and both files belong to other agents in this phase — so the adapter
/// layer sits next to the root view, in one place.
///
/// The service makes no decision of its own: it only translates the call shape.
@MainActor
final class ConnectionBridge: ConnectionProbe {
    /// The root view provides it at launch; without it no write is attempted (no silent
    /// loss).
    var record: ModelContext?

    private let service = ConnectionService()

    /// The service returns the result directly: this place used to poll `@Observable`
    /// state with 120 ms sleeps — burning energy and delaying the result by a turn. The
    /// upper bound is now `MCPClient`'s own timeout.
    func probe(name: String, rawURL: String, key: String?) async -> ConnectionProbeOutcome {
        switch await service.probeAndWait(rawURL: rawURL, key: key) {
        case .succeeded(let tools):
            return .succeeded(tools)
        case .failed(let cause):
            return .failed(cause)
        case .pending, .probing:
            // Unreachable: `probeAndWait` only returns a settled state.
            return .failed(ConnectionService.cancelSentence)
        }
    }

    func probe(_ connection: Connection) async -> ConnectionProbeOutcome {
        // Touching a deleted record is fatal; the fields are read BEFORE the await.
        guard !connection.isDeleted else {
            return .failed(String(localized: "The connection no longer exists."))
        }
        let name = connection.name
        let rawURL = connection.rawURL
        let key = connection.keyRef.flatMap { Keychain.read(ref: $0) }
        return await probe(name: name, rawURL: rawURL, key: key)
    }

    func add(name: String,
             rawURL: String,
             key: String?,
             deviceData: DeviceDataSetting,
             tools: [ToolSummary]) throws -> Connection {
        guard let record else { throw BridgeError.contextMissing }
        // `tools` is ignored: the service keeps the RAW specs it read during the probe
        // and does the summarising itself in the background. Writing the view's coarse
        // summaries back would put the old ones on top of the summarised version.
        return try service.add(name: name,
                               rawURL: rawURL,
                               deviceData: deviceData,
                               key: key,
                               context: record)
    }

    func removeKey(_ connection: Connection) {
        // Deleting is the board's job; here ONLY the Keychain record goes.
        guard !connection.isDeleted, let ref = connection.keyRef else { return }
        _ = Keychain.delete(ref: ref)
    }

    enum BridgeError: LocalizedError {
        case contextMissing
        var errorDescription: String? {
            String(localized: "Couldn’t save the connection: the store isn’t ready.")
        }
    }
}

#Preview {
    ContentView()
        .modelContainer(for: [Chat.self, Message.self,
                              UserSkill.self, MemoryNote.self, Connection.self],
                        inMemory: true)
}
