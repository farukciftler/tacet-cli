//
//  DocumentEngine.swift
//  Tacet
//
//  The shared contract for format-specific write/read engines. The tools call this layer;
//  each engine is responsible for exactly one format (Excel, PDF, Word, Text).
//  The engines are pure logic — they know nothing about the Tool protocol or the UI. No network.
//

import Foundation

/// The content read out of a document. Prose documents fill `text`, table documents fill `table`.
struct DocumentBody {
    var text: String        // plain/markdown text (prose + extracted text)
    var table: Table?       // the structured table if there is one (xlsx)

    init(text: String, table: Table? = nil) {
        self.text = text
        self.table = table
    }
}

/// Engine errors face TWO DIFFERENT AUDIENCES and a single text cannot serve both:
///
/// - The **model** reads a fixed English code (`TacetTool.errorCode`); that is the only way
///   it learns how to fix its input. Because `ToolErrorCode` was not implemented, the
///   correction instruction written by the engine used to collapse into `unknown_error`
///   and the model repeated the same mistake.
/// - The **user** reads a localized sentence in the chip (`TacetTool.shortError` prints
///   `errorDescription` as-is). Letting the English instruction written for the model reach
///   the screen was a breach of the "no raw error reaches the screen" promise.
enum DocumentEngineError: LocalizedError, ToolErrorCode {
    /// The input could not be converted to this format. `code` goes to the model, `chip` to the user.
    case unsupported(code: String, chip: String)
    case noContent

    var errorDescription: String? {
        switch self {
        case .unsupported(_, let chip): return chip
        case .noContent: return String(localized: "No content was found in the document.")
        }
    }

    var errorCode: String {
        switch self {
        case .unsupported(let code, _): return code
        case .noContent: return "empty_document"
        }
    }
}

/// An engine responsible for a single format.
protocol DocumentEngine {
    var format: DocumentFormat { get }

    /// The raw write: dumps the file to disk and returns its URL. `body`: prose/markdown text;
    /// `table`: for xlsx. `fileName` is the extensionless base name; the engine appends the
    /// correct extension.
    ///
    /// CALLERS DO NOT USE THIS — they go through `write(...)`. `write(...)` wraps this and
    /// applies the file protection class + exclusion from backup. Because the rule lives on
    /// the write path itself, it cannot be forgotten when a new engine is added.
    func writeRaw(fileName: String, title: String?, body: String?, table: Table?, folder: URL) throws -> URL

    /// Extracts the content out of a file (for reading/editing).
    func read(url: URL) throws -> DocumentBody
}

extension DocumentEngine {
    /// The single write gate: the engine's raw write + file protection. Engines do not
    /// override this; skipping the protection is not possible.
    func write(fileName: String, title: String?, body: String?, table: Table?, folder: URL) throws -> URL {
        // Make sure the target folder is ready with its protection class (the root, or a
        // subfolder such as "Attached").
        DocumentContext.prepareFolder(folder)
        let url = try writeRaw(fileName: fileName, title: title, body: body, table: table, folder: folder)
        // Do not rely on folder inheritance: stamp the class explicitly onto the written file.
        DocumentContext.applyProtection(url)
        return url
    }

    /// Produces a unique, safe file name (appends a number on a collision).
    func targetURL(fileName: String, folder: URL) -> URL {
        let cleaned = fileName
            .replacingOccurrences(of: "/", with: "-")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let base = cleaned.isEmpty ? "tacet-document" : cleaned
        var candidate = folder.appendingPathComponent("\(base).\(format.fileExtension)")
        var i = 2
        while FileManager.default.fileExists(atPath: candidate.path) {
            candidate = folder.appendingPathComponent("\(base)-\(i).\(format.fileExtension)")
            i += 1
        }
        return candidate
    }
}

/// The format → engine mapping. The tools get their engine from here.
enum DocumentEngines {
    static func engine(_ format: DocumentFormat) -> DocumentEngine {
        switch format {
        case .xlsx: return ExcelEngine()
        case .pdf:  return PdfEngine()
        case .docx: return DocxEngine()
        case .md, .txt: return TextEngine(format: format)
        case .html: return HtmlEngine()
        }
    }
}
