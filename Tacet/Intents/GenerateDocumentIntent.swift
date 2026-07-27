//
//  GenerateDocumentIntent.swift
//  Tacet
//
//  CLASS A — an OPENING intent (intent-plan §1.2). The templated version of `AskTacetIntent`;
//  the reason it is a separate intent is that in Shortcuts the format is a DROP-DOWN MENU and
//  it can be embedded in a Siri phrase: `AppEnum` is one of the parameter types that can enter
//  a phrase, a free `String` cannot.
//
//  IT DOES NOT RETURN THE FILE. `openAppWhenRun` + `ReturnsValue` is technically possible, but
//  `perform()` would have to wait for the turn to finish — a continuation left hanging on every
//  one of the approval-sheet, permission-prompt, cancellation and user-leaves-the-app paths.
//  The product reason weighs more: DATA LEAVING MUST BE A SEPARATE AND DELIBERATE ACT — if the
//  user wants to take the file into a pipeline, they add `ProvideDocumentIntent` to their
//  shortcut separately, and that intent asks for explicit consent inside the app.
//

import Foundation
import AppIntents

// MARK: - Format

/// The single parameter embedded in the Siri phrase (§3).
///
/// `.txt` IS DELIBERATELY ABSENT: "Create Text with Tacet" is a meaningless sentence and
/// `markdown` already covers plain text.
nonisolated enum IntentDocumentFormat: String, CaseIterable, AppEnum {
    case excel
    case word
    case pdf
    case markdown
    case html

    static var typeDisplayRepresentation: TypeDisplayRepresentation {
        TypeDisplayRepresentation(name: "Document format")
    }

    static var caseDisplayRepresentations: [IntentDocumentFormat: DisplayRepresentation] {
        [.excel: "Excel",
         .word: "Word",
         .pdf: "PDF",
         .markdown: "Markdown note",
         .html: "Web page"]
    }

    /// The word written into the prompt. These match the roots inside
    /// `IntentPicker.documentTraces` ("excel", "word", "pdf", "markdown", "web page") → the
    /// turn is routed to the `.document` profile and `create_document` enters the session.
    /// `CreateDocumentTool.Format` IS NOT SET directly: constrained decoding already makes
    /// producing an invalid format impossible.
    ///
    /// THESE ARE NOT UI TEXT, THEY ARE ROUTING DATA. The router matches on substrings, so a
    /// word here is only a signal if `documentTraces` carries it. "Web page" was "Web sayfası"
    /// and was matched by the trace "sayfa"; the English trace "web page" was added to the list
    /// IN THE SAME STEP. Change one of the two without the other and the routing breaks
    /// SILENTLY — the request still runs, just in the wrong profile and without
    /// `create_document` in the session.
    var promptWord: String {
        switch self {
        case .excel:    return "Excel"
        case .word:     return "Word"
        case .pdf:      return "PDF"
        case .markdown: return "Markdown note"
        case .html:     return "Web page"
        }
    }
}

// MARK: - Intent

struct GenerateDocumentIntent: AppIntent {
    nonisolated static var title: LocalizedStringResource { "Create a document with Tacet" }

    nonisolated static var description: IntentDescription {
        IntentDescription("Opens Tacet so it can prepare a document in the format you want.")
    }

    nonisolated static var openAppWhenRun: Bool { true }

    @Parameter(title: "Format")
    var format: IntentDocumentFormat

    @Parameter(title: "Topic", requestValueDialog: "What should it be about?")
    var topic: String

    nonisolated static var parameterSummary: some ParameterSummary {
        Summary("Create \(\.$format): \(\.$topic)")
    }

    @MainActor
    func perform() async throws -> some IntentResult {
        if let block = IntentError.modelBlock() { throw block }

        let cleanTopic = topic.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !cleanTopic.isEmpty else { throw IntentError.emptyPrompt }

        let prompt = String(localized: "Prepare as \(format.promptWord): \(cleanTopic)")
        IntentInbox.shared.put(.ask(prompt))
        return .result()
    }
}
