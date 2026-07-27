//
//  EditDocumentTool.swift
//  Tacet
//
//  A production tool — editing. Reads the document shared into the chat and writes it out as
//  a new version with the content the model supplied (read-assisted regeneration).
//  The original is kept; a new file named "… (edited)" is produced. Writing → green chip.
//
//  The flow: the model first gets the content with read_document, then calls this tool with
//  the edited body/table. For Excel it passes newTable, for prose documents newBody.
//

import Foundation
import FoundationModels

struct EditDocumentTool: TacetTool {
    let name = "edit_document"
    let description = "Edits the document in play — the one attached to the chat, or the file you just created — by writing a new version. Call this when the user asks to change it ('add this', 'delete that row', 'change the title', in any language). First call read_document to get the content, then pass the FULL edited content as 'newContent' (markdown; a markdown table for Excel files)."

    weak var reporter: (any ToolReporter)?
    weak var context: DocumentContext?

    @Generable
    struct Arguments {
        @Guide(description: "The FULL edited content as markdown (the whole document, not just the changed part). For an Excel document write a markdown table (| … |); for a text document write plain markdown.")
        var newContent: String
        @Guide(description: "Optional new title.")
        var title: String?
    }

    func call(arguments: Arguments) async -> String {
        let attached = context?.runnableDocument
        guard let attached else {
            return await runWithChip(icon: "doc", runningText: L10n.lookingForDocument) {
                ToolOutcome(chipText: L10n.noDocumentToEdit,
                            state: .readOk,
                            toModel: "no_document_attached (ask the user to attach a document first)")
            }
        }
        return await runWithChip(icon: attached.format.icon,
                                 runningText: L10n.editingDocument(attached.format.tag),
                                 rawInput: attached.name) {
            let engine = DocumentEngines.engine(attached.format)
            let base = attached.url.deletingPathExtension().lastPathComponent
            // For Excel, turn the markdown table into a structured table; otherwise plain text.
            let table = attached.format.isTableStructured ? Table.fromMarkdown(arguments.newContent) : nil
            let url = try engine.write(fileName: "\(base) (edited)",
                                       title: arguments.title,
                                       body: table == nil ? arguments.newContent : nil,
                                       table: table,
                                       folder: DocumentContext.outputFolder())
            context?.outputAdded(url)
            // The user's document was read and a new version was written (mcp §5.6).
            return await taintIfSucceeded(ToolOutcome(
                chipText: L10n.documentEdited(attached.format.tag, url.lastPathComponent),
                state: .written,
                // THE FORMAT IS STATED EXPLICITLY (measured: "convert this to word" → the tool
                // rewrote the .xlsx as .xlsx and the model said "converted to Word" — a silent
                // lie). This tool does NOT CONVERT formats; conversion happens through
                // read_document + create_document. Telling the model the fact is stronger than
                // repeating the rule in the instructions: the model faces the lie in its own output.
                toModel: "file_edited: \(url.lastPathComponent) "
                    + "(format unchanged: \(attached.format.fileExtension); this tool never converts "
                    + "format — to change format call create_document with the new format)",
                rawOutput: url.path,
                filePath: url.path
            ))
        }
    }
}
