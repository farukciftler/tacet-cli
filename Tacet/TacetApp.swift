//
//  TacetApp.swift
//  Tacet
//

import SwiftUI
import SwiftData

@main
struct TacetApp: App {
    /// `nil` = no store at all could be opened (neither disk nor memory). CRASHING
    /// at launch is not an option: the whole recovery chain right below was written
    /// as "instead of crashing", and having the last step be `try!` made a liar of
    /// that rationale.
    let container: ModelContainer?

    init() {
        #if DEBUG
        if CommandLine.arguments.contains("--selftest") {
            SelfTest.run()
        }
        if CommandLine.arguments.contains("--test") {
            Evaluation.run()
        }
        // Final report: collects the raw JSON of separate runs into one Excel file.
        // The report is produced with the app's OWN ExcelEngine (so real =SUM lands).
        if CommandLine.arguments.contains("--eval-merge") {
            EvalReport.mergeRun()
        }
        if CommandLine.arguments.contains("--language") {
            LanguageTest.run()
        }
        // Comprehensive eval: "--eval [--shard N] [--shards M]". Parallel run agents
        // launch the same binary with different shards; the slicing is deterministic.
        if CommandLine.arguments.contains("--eval") {
            let arguments = CommandLine.arguments
            func number(_ flag: String, _ defaultValue: Int) -> Int {
                guard let index = arguments.firstIndex(of: flag),
                      index + 1 < arguments.count,
                      let value = Int(arguments[index + 1]) else { return defaultValue }
                return value
            }
            Evaluation.runComprehensive(shard: number("--shard", 0),
                                        total: number("--shards", 1))
        }
        // MCP (connection) eval: "--eval-mcp". A separate flag, because this run
        // REALLY goes out to the user's own server; "--eval" does not.
        if CommandLine.arguments.contains("--eval-mcp") {
            Evaluation.runMCP()
        }
        #endif
        container = Self.setupContainer()
        // The intents' (App Intents) gate to the store. Screenless intents
        // (`AddNoteIntent`, `OpenChatIntent`) cannot reach @Environment; without the
        // store gate they cannot run at all. NO SECOND CONTAINER IS OPENED — the
        // `StoreGate` below only shares this instance's mainContext.
        StoreGate.setup(container)
        // A Keychain record outlives the SwiftData store: after a reinstall or a store
        // move the tokens stay behind with no `Connection` pointing at them.
        Self.purgeOrphanKeys(container)
        #if DEBUG
        // Store seeding for App Store captures. Separate from `--table-demo`,
        // which seeds exactly one table and exists to check the markdown-to-table
        // chain; this one seeds the set the screenshots are taken from.
        if CommandLine.arguments.contains("--demo-seed"), let container {
            DemoSeed.run(container)
        }
        if CommandLine.arguments.contains("--table-demo"), let container {
            let chat = Chat(title: "Table demo")
            container.mainContext.insert(chat)
            let userMessage = Message(role: .user, content: "Could you make a weekly meal plan?")
            userMessage.chat = chat
            let reply = Message(role: .tacet, content: """
            Here is your weekly meal plan:

            | Day | Lunch | Dinner |
            | --- | --- | --- |
            | Monday | Lentil soup | Grilled chicken |
            | Tuesday | Ezogelin soup | Stuffed aubergine |
            | Wednesday | Tomato soup | Fish |

            Enjoy. Tell me if you want a change.
            """)
            reply.chat = chat
            container.mainContext.insert(userMessage)
            container.mainContext.insert(reply)
            try? container.mainContext.save()
        }
        #endif
    }

    var body: some Scene {
        WindowGroup {
            // The single source of truth for the language is AppleLanguages: whichever
            // .lproj Bundle.main picked at process start is what both Text(...) and
            // String(localized:) use. Overriding the locale here splits those two paths
            // and leaves the screen half-translated; that is why there is NO override.
            // A language change is announced through
            // LanguagePreference.uiRestartPending.
            if let container {
                ContentView()
                    .modelContainer(container)
            } else {
                // No store could be opened at all. ContentView cannot work without a
                // modelContext; instead of a blank screen or a crash we tell the
                // situation IN WORDS.
                StoreUnavailableScreen()
            }
        }
    }

    /// Drops the Keychain records no connection points at any more.
    ///
    /// IT ONLY RUNS ON A HEALTHY STORE, and every clause of the guard below is there to
    /// stop the same accident: purging against a list that is not the user's real list.
    /// If the store could not be opened at all, if the session fell back to memory, or if
    /// a malformed store was backed up, then the connections on hand are NOT the
    /// connections the user has — their owners are sitting in the store we set aside, and
    /// deleting those keys would be final, because a token is never shown again. In those
    /// states the leak is the cheaper of the two.
    ///
    /// A FAILED FETCH IS NOT AN EMPTY LIST: `try?` collapsing to nil must not become "no
    /// connections, delete everything", which is why the fetch is bound rather than
    /// defaulted. An empty list that the fetch really did return is the legitimate case —
    /// the user deleted every connection — and its keys should go.
    @MainActor
    private static func purgeOrphanKeys(_ container: ModelContainer?) {
        guard let container,
              !StoreState.shared.sessionNotPersistent,
              StoreState.shared.backedUpStore == nil,
              let connections = try? container.mainContext.fetch(FetchDescriptor<Connection>())
        else { return }
        Keychain.purgeOrphans(valid: Set(connections.compactMap(\.keyRef)))
    }

    /// Container with the Chat + Message schema. If it cannot open: in DEBUG it
    /// resets, in production it NEVER deletes user data — it backs the malformed
    /// store up and retries.
    @MainActor
    private static func setupContainer() -> ModelContainer? {
        let schema = Schema(versionedSchema: SchemaV1.self)
        let config = ModelConfiguration(schema: schema)
        do {
            return try ModelContainer(for: schema,
                                      migrationPlan: MigrationPlan.self,
                                      configurations: config)
        } catch {
            #if DEBUG
            // Development only: a clean start while the schema is still moving.
            try? FileManager.default.removeItem(at: config.url)
            for suffix in ["-wal", "-shm"] {
                try? FileManager.default.removeItem(
                    at: config.url.appendingPathExtension(suffix))
            }
            #else
            // No deleting in production: we rename and keep the malformed store so
            // the user's data stays on disk and can be recovered later.
            StoreState.shared.backedUpStore = backUpStore(config.url)
            #endif

            if let clean = try? ModelContainer(for: schema,
                                               migrationPlan: MigrationPlan.self,
                                               configurations: config) {
                return clean
            }
            // The disk still will not open: instead of crashing at launch, carry this
            // session in memory. Nothing is saved; chats do not survive the app
            // closing — the UI must warn.
            let inMemory = ModelConfiguration(schema: schema, isStoredInMemoryOnly: true)
            if let temp = try? ModelContainer(for: schema, configurations: inMemory) {
                StoreState.shared.sessionNotPersistent = true
                return temp
            }
            // The in-memory store did not open either. A state that should not be seen
            // in practice, but `try!` would turn it into A CRASH AT LAUNCH — whereas
            // every step of the graceful recovery up to here was written with the
            // rationale "instead of crashing", and what the user sees must be a
            // diagnosable screen.
            return nil
        }
    }

    /// Renames the malformed store and its side files to `<name>.malformed-<n>`.
    /// Returns the backup's file name on success. Nothing is deleted.
    private static func backUpStore(_ url: URL) -> String? {
        let fm = FileManager.default
        guard fm.fileExists(atPath: url.path) else { return nil }
        // Find a counter that does not collide.
        var counter = 1
        var target = url.appendingPathExtension("malformed-\(counter)")
        while fm.fileExists(atPath: target.path) && counter < 100 {
            counter += 1
            target = url.appendingPathExtension("malformed-\(counter)")
        }
        do {
            try fm.moveItem(at: url, to: target)
        } catch {
            return nil
        }
        // Back the WAL/SHM side files up too; if they are left behind they corrupt
        // the new store.
        for suffix in ["-wal", "-shm"] {
            let sideFile = url.appendingPathExtension(suffix)
            guard fm.fileExists(atPath: sideFile.path) else { continue }
            try? fm.moveItem(at: sideFile, to: target.appendingPathExtension(suffix))
        }
        return target.lastPathComponent
    }
}

// MARK: - Intent store gate

/// The intents' (App Intents) gate to the store.
///
/// It is the SAME `mainContext` the view layer receives through
/// `@Environment(\.modelContext)`. NO SECOND CONTAINER IS OPENED: opening the same
/// store with two containers leads to corruption in SwiftData. That is why, if
/// `context` is nil, an intent does NOT set up its own container — it throws
/// `IntentError.storeMissing`, an honest error instead of a silent void.
///
/// Screenless intents (`AddNoteIntent`, `OpenChatIntent`) run without bringing the
/// app to the front; since `@main App.init()` runs in that case too, the gate ends
/// up filled. An unfilled state is not a fault but a diagnosable condition, and the
/// user is told to "open the app once".
@MainActor
enum StoreGate {
    private(set) static var container: ModelContainer?

    static func setup(_ c: ModelContainer?) { container = c }

    static var context: ModelContext? { container?.mainContext }
}

// MARK: - Schema versions

/// v1 — the first schema. The version identifier is deliberately held at 1.0.0.
///
/// RATIONALE: the app has not shipped yet, meaning there is no store in the field
/// carrying a DIFFERENT set of fields under the 1.0.0 tag. That makes it safe to pull
/// fields added during development — such as `Message.isError` and
/// `Chat.titleAutomatic` — and models added and then removed before ever shipping,
/// such as `MemoryNote` and `Connection`, into v1 without bumping the version;
/// bumping it would create a pointless migration stage. v1 = "the first shipped state".
///
/// AFTER SHIPPING THIS FREEDOM ENDS. The moment the first release is in the store this
/// enum MUST BE FROZEN: if a field is added or removed, a new `SchemaV2` must be
/// written, added to `MigrationPlan.schemas`, and `stages` must gain
/// `.lightweight(fromVersion: SchemaV1.self, toVersion: SchemaV2.self)` (or a custom
/// stage). Editing v1 after shipping makes a liar of the "from" schema and the user's
/// store will not open.
enum SchemaV1: VersionedSchema {
    static var versionIdentifier: Schema.Version { Schema.Version(1, 0, 0) }
    static var models: [any PersistentModel.Type] {
        [Chat.self, Message.self, UserSkill.self,
         MemoryNote.self, Connection.self]
    }
}

/// The migration plan. There is only one version right now and the stage list is
/// empty; the skeleton stays here so v2 can be wired in with a single line.
enum MigrationPlan: SchemaMigrationPlan {
    static var schemas: [any VersionedSchema.Type] { [SchemaV1.self] }
    static var stages: [MigrationStage] { [] }
}

// MARK: - Store state

/// Unusual conditions around the persistent store that the UI has to report to the
/// user. On a normal launch both are off and nothing is shown.
@MainActor
@Observable
final class StoreState {
    static let shared = StoreState()

    /// The disk store could not be opened, the session is in memory only. Everything
    /// in this session is lost when the app closes — the user must be warned.
    var sessionNotPersistent = false

    /// If a malformed store was backed up, the backup's file name. The old data was
    /// not lost but is not visible in this session; the situation can be reported to
    /// the user.
    var backedUpStore: String?

    // Note: NO separate flag is KEPT for "neither disk nor memory could be opened".
    // It was a flag that was written and never read: the screen decision is made by
    // `container == nil` (see `TacetApp.body`) and that condition already carries the
    // same fact.

    private init() {}
}

// MARK: - Last-resort screen

/// The screen shown when no store could be opened at all. Its only job is to tell the
/// situation honestly: showing an empty chat screen would give the user the feeling of
/// "it works but it is broken", and crashing would tell nothing.
///
/// Palette rule: ink/grey only; `Palette.error` appears in the title alone, to say the
/// state really is a fault.
private struct StoreUnavailableScreen: View {
    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.s3) {
            Text("Tacet could not start.")
                .font(Typography.brand())
                .foregroundStyle(Palette.error)
            Text("The store where your chats are kept could not be opened on this device, so Tacet cannot start a conversation. Your data has not been deleted.")
                .font(Typography.tacet())
                .foregroundStyle(Palette.ink)
            Text("Try closing Tacet completely and opening it again. If the problem continues, make sure there is free space on the device.")
                .font(Typography.user())
                .foregroundStyle(Palette.grey)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .center)
        .padding(.horizontal, Spacing.s5)
        .background(Palette.background)
    }
}
