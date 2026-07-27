//
//  CreateDocumentTool.swift
//  Tacet
//
//  A production tool (spec §7.3, §7.3.2; code-spec §4). Produces an Excel/PDF/Word/Text file
//  or a single-file HTML page out of the chat data. HTML output is verified off-screen with
//  PageVerifier; a page that does not pass is not presented and the file is deleted. The
//  output gets a QuickLook preview + sharing + saving to Files. A write action → green chip.
//  No network.
//

import Foundation
import FoundationModels

struct CreateDocumentTool: TacetTool {
    let name = "create_document"
    let description = "Creates an Excel/PDF/Word/Markdown file or an HTML page. Call this IMMEDIATELY when the user asks for a file, table, list, report or web page ('make an excel/pdf/word/site', in any language) — do not ask or narrate. Write markdown into 'content'; for a table write a markdown table (| … |). For device data (e.g. calendar) pass 'sourceRef' instead of 'content'."

    weak var reporter: (any ToolReporter)?
    weak var context: DocumentContext?
    /// The bulk-data channel — with sourceRef the bulk data is pulled without passing through the model.
    weak var dataStore: DataStore?

    /// The format is NOT free text any more. `DocumentFormat(userText:)` used to resolve it
    /// with a fuzzy `.contains`, and every value that did not match silently became `.txt`:
    /// the user asked for "excel" and got a .txt. With an enum, constrained decoding makes an
    /// invalid value IMPOSSIBLE TO PRODUCE.
    @Generable
    enum Format: String, Equatable, CaseIterable {
        case excel
        case pdf
        case word
        case markdown
        case text
        case html

        var documentFormat: DocumentFormat {
            switch self {
            case .excel:    return .xlsx
            case .pdf:      return .pdf
            case .word:     return .docx
            case .markdown: return .md
            case .text:     return .txt
            case .html:     return .html
            }
        }
    }

    @Generable
    struct Arguments {
        @Guide(description: "File format: 'excel' (spreadsheet), 'pdf', 'word', 'markdown', 'text' (plain text) or 'html' (single-page website).")
        var format: Format
        @Guide(description: "File name without extension, e.g. 'july-meetings'.")
        var fileName: String
        @Guide(description: "Document title (optional).")
        var title: String?
        @Guide(description: "Document body as MARKDOWN. If a table is needed, write a markdown table: a | Header1 | Header2 | row, then | --- | --- |, then the data rows. Excel files are built from that table.")
        var content: String?
        @Guide(description: "Data reference returned by another tool (e.g. the calendar tool). If given, the full data is pulled from the store — leave 'content' empty.")
        var sourceRef: String?
    }

    func call(arguments: Arguments) async -> String {
        let format = arguments.format.documentFormat
        let input = "format: \(format.tag), name: \(arguments.fileName)"
            + (arguments.sourceRef.map { ", ref: \($0)" } ?? "")
        return await runWithChip(icon: format.icon,
                                 runningText: L10n.creatingDocument(format.tag),
                                 rawInput: input) {
            // The bulk-data channel: if there is a reference, pull the table from the store
            // (not from the model context).
            var table: Table?
            var body: String? = arguments.content
            let rawRef = arguments.sourceRef?.trimmingCharacters(in: .whitespacesAndNewlines)
            if let ref = rawRef, !ref.isEmpty {
                // IF THERE IS A REF, THE REF IS BINDING (P0-2). An unresolvable ref used to
                // silently fall back to `content`; since `content` was empty too, an EMPTY
                // file was written and "file_created" was reported — the user carried around
                // a file they believed was full. That was the most dangerous class of bug.
                // Now no file is written AT ALL and an explicit error is returned.
                if let storeTable = dataStore?.take(ref) {
                    table = storeTable
                    body = nil
                } else if let storeText = dataStore?.takeText(ref) {
                    // The plain body offloaded by read_document (P2-6). If Excel is requested,
                    // structure the markdown table inside it.
                    if format.isTableStructured, let parsed = Table.fromMarkdown(storeText) {
                        table = parsed
                        body = nil
                    } else {
                        table = nil
                        body = storeText
                    }
                } else {
                    // We do NOT build the sentence HERE: only the store knows the difference
                    // between "the ref never existed" and "the ref existed but was dropped for
                    // the memory cap" (`DataStore.refState`). Had we written our own text, a
                    // dropped ref would have been reported to the model as a hallucination.
                    let state = dataStore?.refState(ref)
                        ?? "unknown_data_ref: \"\(ref)\" (available: none)"
                    return ToolOutcome(
                        chipText: Self.sourceNotFound,
                        state: .failed(Self.sourceNotFound),
                        // Facts only: which ref was asked for, which ones are on hand, what
                        // happened. No imperative instruction (P2-4).
                        toModel: state + "; no file was created",
                        rawOutput: "sourceRef=\(ref)"
                    )
                }
            } else if format.isTableStructured, let c = arguments.content,
                      let parsed = Table.fromMarkdown(c) {
                // Excel is requested and the content holds a markdown table → convert it into
                // a structured table.
                table = parsed
                body = nil
            }
            let engine = DocumentEngines.engine(format)
            let url = try engine.write(fileName: arguments.fileName,
                                       title: arguments.title,
                                       body: body,
                                       table: table,
                                       folder: DocumentContext.outputFolder())
            // HTML verification (code-spec §4.3): the page is loaded in an off-screen
            // WKWebView; a page that does not load or that raises a script error is NOT
            // PRESENTED to the user — the file is deleted and a short cause is returned to the
            // model (the skill guide is what tells the model to simplify the content and try ONE more time).
            if format == .html {
                let verification = await PageVerifier.verify(url: url)
                if !verification.passed {
                    try? FileManager.default.removeItem(at: url)
                    let cause = verification.cause ?? L10n.pageNotVerified
                    return ToolOutcome(
                        chipText: L10n.pageNotVerified,
                        state: .failed(cause),
                        // The structural channel is `state: .failed(cause)`; only the fact goes
                        // to the model. Imperative instructions like "Simplify … try ONCE more"
                        // were removed (P2-4): a tool does not give the model orders, the
                        // retry instruction is the skill file's job.
                        toModel: "verification_failed: the page did not load cleanly; the file was discarded",
                        rawOutput: cause
                    )
                }
            }
            context?.outputAdded(url)
            // A file was created on the device; its content is the user's data (mcp §5.6).
            return await taintIfSucceeded(ToolOutcome(
                chipText: L10n.documentCreated(format.tag, url.lastPathComponent),
                state: .written,
                // Only the fact goes to the model; do not write UI instructions (preview/share) — the model parrots them.
                toModel: "file_created (\(format.tag)): \(url.lastPathComponent)",
                rawOutput: url.path,
                filePath: url.path
            ))
        }
    }

    // Note: in this phase L10n.swift belongs to another agent; the new key is defined here
    // with String(localized:) — it enters the String Catalog automatically.
    static var sourceNotFound: String { String(localized: "Source data not found") }
}
