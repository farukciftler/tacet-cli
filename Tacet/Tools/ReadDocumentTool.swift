//
//  ReadDocumentTool.swift
//  Tacet
//
//  A perception tool (spec §7.3). Reads the document the user shared into the chat
//  (Excel/PDF/Word/Text) and returns its content to the model as a summary. Reading → grey chip.
//

import Foundation
import FoundationModels

struct ReadDocumentTool: TacetTool {
    let name = "read_document"
    let description = "Reads the document in play — the one attached to the chat, or the file you just created. Call this IMMEDIATELY when the user asks about it ('summarize', 'what's in it', 'show it as a table', in any language); read before describing. Never say you need an attachment before calling this."

    weak var reporter: (any ToolReporter)?
    weak var context: DocumentContext?
    /// The bulk-data channel (P2-6). The FULL content that was read is put here; only a short
    /// summary + a ref goes back to the model. The content used to be truncated at 1500
    /// characters and printed straight into the model: it both ate ~375 tokens and lost the
    /// truncated part IRRECOVERABLY — create_document could never reach that data again.
    weak var dataStore: DataStore?

    @Generable
    struct Arguments {
        @Guide(description: "Optional: the topic or focus the user is interested in. If empty, the whole document is read.")
        var focus: String?
    }

    func call(arguments: Arguments) async -> String {
        let attached = context?.runnableDocument
        guard let attached else {
            return await runWithChip(icon: "doc", runningText: L10n.lookingForDocument) {
                ToolOutcome(chipText: L10n.noSharedDocument,
                            state: .readOk,
                            toModel: "no_document_attached (ask the user to attach a document first)")
            }
        }
        return await runWithChip(icon: attached.format.icon,
                                 runningText: L10n.readingDocument(attached.format.tag),
                                 rawInput: attached.name) {
            let engine = DocumentEngines.engine(attached.format)
            let content = try engine.read(url: attached.url)
            // For a table document a MARKDOWN table goes to the model: the model can pass it
            // through almost verbatim and it gets drawn as a real table in the chat.
            // The plain `summary` (no pipes, cut at 5 rows) forced the model to rebuild the
            // table; the small model did not do that and just said "shown".
            // The bulk-data channel (P2-6): the FULL content goes to the store, a summary + ref
            // to the model. Truncation is no longer DATA LOSS, only a window decision: the
            // truncated part reaches create_document intact through the ref.
            var ref: String?
            if let store = dataStore {
                if let table = content.table, table.rows.count > 1 {
                    ref = store.put(table, tag: "document")
                } else if content.text.count > 1500 {
                    ref = store.putText(content.text, tag: "document")
                }
            }

            // The preview row count DEPENDS on the offload. Once the full data is
            // recoverable, printing 30 rows of markdown into the window has no justification
            // left; when it is not recoverable (no store attached) the old 30 rows are kept —
            // let us not open a new path to loss.
            let previewRows = ref == nil ? 30 : 10
            let body = content.table?.truncatedMarkdown(maxRows: previewRows) ?? content.text
            let truncated = body.count > 1500 ? String(body.prefix(1500)) + "…" : body

            // The user's document was genuinely read → the session is tainted (mcp §5.6).
            // The "no attached document" path above does not come here: since no data was
            // touched, tainting the session there would be wrong.
            let extraText = ref.map { " (full content ready, data_ref=\($0))" } ?? ""
            return await taintIfSucceeded(ToolOutcome(
                chipText: L10n.documentRead(attached.format.tag, attached.name),
                state: .readOk,
                toModel: (truncated.isEmpty ? "The document appears to be empty." : truncated) + extraText,
                // The chip detail shows the FULL content (the second layer of transparency).
                // The model's window is truncated; what the user sees is not.
                rawOutput: content.table?.markdown ?? content.text
            ))
        }
    }
}
