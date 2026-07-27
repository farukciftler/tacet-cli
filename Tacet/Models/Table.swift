//
//  Table.swift
//  Tacet
//
//  File generation pattern 1 (spec §7.3.2): the model produces a @Generable
//  Table → the generation tool writes the file. Type-safe rows; the model is
//  never made to parse free text. Practical limit ~50–100 rows (context window).
//  Bulk device data does not go in here.
//

import Foundation
import FoundationModels

/// A single row — its cells in the same order as the headers.
///
/// The @Guide texts are PROMPT TEXT the model reads. They were Turkish until the
/// English migration and the eval suite was measured against that wording, so the
/// numbers recorded elsewhere in this repo PREDATE this rewrite: the suite must be
/// re-measured before any of them is quoted as current.
@Generable
struct Row: Equatable {
    @Guide(description: "The cells of this row; same order and same count as the headers, as text.")
    var cells: [String]

    init(cells: [String]) { self.cells = cells }
}

/// A structured table — for the spreadsheet (xlsx) and for the markdown table.
@Generable
struct Table: Equatable {
    @Guide(description: "Column headers, left to right.")
    var headers: [String]

    @Guide(description: "Rows. Every row holds as many cells as there are headers.")
    var rows: [Row]

    init(headers: [String], rows: [Row]) {
        self.headers = headers
        self.rows = rows
    }

    /// Short plain-text summary returned to the model/reader.
    var summary: String {
        let head = headers.joined(separator: " | ")
        let first = rows.prefix(5).map { $0.cells.joined(separator: " | ") }.joined(separator: "\n")
        let more = rows.count > 5 ? "\n… (+\(rows.count - 5) rows)" : ""
        return "\(head)\n\(first)\(more)"
    }

    /// Markdown returned to the model — the row count is limited (4096 context
    /// budget). This is used instead of the plain `summary`: the model passes the
    /// text through almost verbatim, `fromMarkdown` parses it back, and a real
    /// table is drawn in the chat.
    func truncatedMarkdown(maxRows: Int) -> String {
        guard rows.count > maxRows else { return markdown }
        let short = Table(headers: headers, rows: Array(rows.prefix(maxRows)))
        // The note line comes after the table block; because it does not start
        // with "|" it does not break parsing.
        return short.markdown + "\n… (+\(rows.count - maxRows) more rows)"
    }

    // MARK: - Parsing (tolerant)

    /// The text split into table/text blocks.
    ///
    /// This is the single source of truth: both `markdownTables` and the chat
    /// body use it. That makes the "an unrecognized pipe row vanished silently"
    /// bug structurally impossible — every unrecognized line falls into a `.text`
    /// block (audit P1-5).
    enum TextBlock: Equatable {
        case text(String)
        case table(Table)
    }

    /// Splits the text into blocks while preserving order. NO line is dropped.
    static func blocks(_ text: String) -> [TextBlock] {
        let lines = text.components(separatedBy: "\n")
        var result: [TextBlock] = []
        var buffer: [String] = []
        var insideCodeBlock = false

        func drainText() {
            let t = buffer.joined(separator: "\n")
                .trimmingCharacters(in: .whitespacesAndNewlines)
            if !t.isEmpty { result.append(.text(t)) }
            buffer = []
        }

        var i = 0
        while i < lines.count {
            let s = lines[i].trimmingCharacters(in: .whitespaces)
            // ``` fences: a "|" inside a code block does not turn into a table.
            if s.hasPrefix("```") {
                insideCodeBlock.toggle()
                buffer.append(lines[i])
                i += 1
                continue
            }
            if !insideCodeBlock, let (table, end) = tryTable(lines, i) {
                drainText()
                result.append(.table(table))
                i = end
            } else {
                buffer.append(lines[i])
                i += 1
            }
        }
        drainText()
        return result
    }

    /// Extracts every markdown table in the text.
    static func markdownTables(_ text: String) -> [Table] {
        blocks(text).compactMap { if case .table(let t) = $0 { return t } else { return nil } }
    }

    /// The first markdown table in the text (nil if there is none).
    static func fromMarkdown(_ text: String) -> Table? {
        markdownTables(text).first
    }

    /// Is there a table starting at position `i`? If so, the table + the index of
    /// the line it ends on.
    ///
    /// ACCEPTANCE BOUNDARY (so that no false positive is produced):
    /// 1. The candidate line must have at least one `|` AND parse into at least
    ///    2 cells.
    /// 2. At least TWO consecutive candidate lines are required (header + data).
    ///    A single `|` in plain text is never a table.
    /// 3. The separator row (`|---|`, `|:--|--:|`) is OPTIONAL; it is skipped if
    ///    present.
    /// 4. The outer pipe (`|` at the start/end of the line) is OPTIONAL — but if
    ///    the outer pipe is ABSENT the tolerance narrows: every line in the block
    ///    must have EXACTLY the same cell count. This is the boundary that stops
    ///    free text (like "Ali | Veli") from accidentally turning into a table.
    private static func tryTable(_ lines: [String], _ i: Int) -> (Table, Int)? {
        // Run of consecutive candidate lines.
        var j = i
        while j < lines.count, isCandidateRow(lines[j]) { j += 1 }
        guard j - i >= 2 else { return nil }

        let headers = splitCells(lines[i])
        guard headers.count >= 2, headers.contains(where: { !$0.isEmpty }) else { return nil }

        // Outer-pipe consistency: do all of them start with "|"?
        let allOuterPiped = (i..<j).allSatisfy {
            lines[$0].trimmingCharacters(in: .whitespaces).hasPrefix("|")
        }
        if !allOuterPiped {
            // Loose form — the column count must match exactly.
            let holds = (i..<j).allSatisfy { splitCells(lines[$0]).count == headers.count }
            guard holds else { return nil }
        }

        // Skip the separator row if there is one.
        var dataStart = i + 1
        if isSeparatorRow(lines[dataStart]) { dataStart += 1 }
        guard dataStart < j else { return nil }

        var rowData: [Row] = []
        for k in dataStart..<j {
            // A second separator row that landed mid-body does not enter the data.
            if isSeparatorRow(lines[k]) { continue }
            let h = splitCells(lines[k])
            guard h.contains(where: { !$0.isEmpty }) else { continue }
            rowData.append(Row(cells: h))
        }
        guard !rowData.isEmpty else { return nil }
        return (Table(headers: headers, rows: rowData), j)
    }

    /// Table row candidate: contains `|` and splits into at least 2 cells.
    private static func isCandidateRow(_ line: String) -> Bool {
        let s = line.trimmingCharacters(in: .whitespaces)
        guard s.contains("|") else { return false }
        return splitCells(s).count >= 2
    }

    /// A separator row made of `---`, `:---`, `---:`, `:---:` cells.
    private static func isSeparatorRow(_ line: String) -> Bool {
        let h = splitCells(line).filter { !$0.isEmpty }
        guard !h.isEmpty else { return false }
        return h.allSatisfy { cell in
            var s = Substring(cell)
            if s.hasPrefix(":") { s = s.dropFirst() }
            if s.hasSuffix(":") { s = s.dropLast() }
            return !s.isEmpty && s.allSatisfy { $0 == "-" }
        }
    }

    private static func splitCells(_ line: String) -> [String] {
        var s = line.trimmingCharacters(in: .whitespaces)
        if s.hasPrefix("|") { s.removeFirst() }
        if s.hasSuffix("|") { s.removeLast() }
        return s.components(separatedBy: "|").map { $0.trimmingCharacters(in: .whitespaces) }
    }

    /// Markdown table rendering.
    var markdown: String {
        guard !headers.isEmpty else { return "" }
        let head = "| " + headers.joined(separator: " | ") + " |"
        let separator = "| " + headers.map { _ in "---" }.joined(separator: " | ") + " |"
        let body = rows.map { s in
            let h = (0..<headers.count).map { i in i < s.cells.count ? s.cells[i] : "" }
            return "| " + h.joined(separator: " | ") + " |"
        }.joined(separator: "\n")
        return ([head, separator] + [body]).joined(separator: "\n")
    }
}
