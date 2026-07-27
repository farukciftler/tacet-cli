//
//  AskTacetIntent.swift
//  Tacet
//
//  CLASS A — an OPENING intent (intent-plan §1.1). The app opens, no data flows to Shortcuts
//  (there is no return value).
//
//  THE TYPE NAME IS A PERMANENT IDENTITY: it is embedded into the compiled Metadata.appintents
//  and the shortcuts the user builds bind to it; renaming after release would break those
//  shortcuts. This type carried the previous brand name; because the migration was completed
//  before release, no shortcut is broken. The old name lives only in the git history and is
//  deliberately not written here.
//
//  WHY IT DOES NOT RETURN THE ANSWER (the plan's five points, all of them binding):
//   1. `ModelService` is `@State` on `ContentView`; there is no process-wide reachable
//      instance. Setting up a second one in the background means a context-less session, an
//      unfed `MemoryStore` and an empty `DataStore` — it would produce a SILENTLY different
//      answer to the same question than the one in the app.
//   2. Most tools ask for permission; `requestFullAccess…` cannot show a prompt while the app
//      is not in the foreground, and the model would then report to the user a refusal THEY
//      NEVER GAVE.
//   3. `ToolExecutor.requestApprovalDecision` waits for an `ApprovalSheet` on screen.
//   4. A turn includes context measurement, summarization, profile recovery and retry — it
//      exceeds the execution budget of an intent started in the background.
//   5. PRODUCT: the tool chips would drop while nobody was looking. "It does not tell, it
//      shows" would break exactly here.
//
//  NOT INTO THE CURRENT CHAT, INTO A NEW ONE: what is asked through Siri is a separate topic;
//  producing an answer on top of a context you cannot see (an old transcript + injected notes)
//  means an input the user cannot audit.
//

import Foundation
import AppIntents

struct AskTacetIntent: AppIntent {
    nonisolated static var title: LocalizedStringResource { "Ask Tacet" }

    nonisolated static var description: IntentDescription {
        IntentDescription("Passes your question to Tacet. The answer is produced in the app, in front of your eyes.")
    }

    /// The turn runs on screen — the whole of this intent is the justification for this line.
    nonisolated static var openAppWhenRun: Bool { true }

    @Parameter(title: "Question", requestValueDialog: "What would you like to ask?")
    var prompt: String

    nonisolated static var parameterSummary: some ParameterSummary {
        Summary("Ask Tacet: \(\.$prompt)")
    }

    @MainActor
    func perform() async throws -> some IntentResult {
        // THE FIRST LINE: if the model is blocked, NOTHING is put into the inbox — otherwise a
        // prompt that cannot be answered would land there when the app opens.
        if let block = IntentError.modelBlock() { throw block }

        let text = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { throw IntentError.emptyPrompt }

        IntentInbox.shared.put(.ask(text))
        return .result()
    }
}
