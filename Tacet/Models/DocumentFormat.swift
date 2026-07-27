//
//  DocumentFormat.swift
//  Tacet
//
//  Document formats (spec §7.3, generation tools). All of them are produced/read
//  on device; no network. .xlsx and .docx are OOXML (zip) — packed/unpacked with
//  ZipStore.
//

import Foundation

enum DocumentFormat: String, CaseIterable, Sendable {
    case xlsx   // Excel
    case pdf    // PDF
    case docx   // Word
    case md     // Markdown
    case txt    // plain text
    case html   // single-file web page (code-spec §4)

    var fileExtension: String { rawValue }

    /// Chip/tag name: "Excel created · …" (spec §4.4).
    var tag: String {
        switch self {
        case .xlsx: return "Excel"
        case .pdf:  return "PDF"
        case .docx: return "Word"
        case .md:   return "Note"
        case .txt:  return "Metin"
        case .html: return "Sayfa"
        }
    }

    /// SF Symbol (outline). The chip icon.
    var icon: String {
        switch self {
        case .xlsx: return "tablecells"
        case .pdf:  return "doc.richtext"
        case .docx: return "doc.text"
        case .md:   return "text.alignleft"
        case .txt:  return "doc.plaintext"
        case .html: return "doc.text.image" // not "globe" — that suggests the network (code-spec §4.2)
        }
    }

    /// Does this format rest on a table (xlsx)? The others are plain/markdown text.
    var isTableStructured: Bool { self == .xlsx }

    /// Map the model's free text ("excel", "word", "pdf", "markdown"…) to a format.
    ///
    /// The match words stay Turkish on purpose: they are matched against what
    /// the model writes, which is Turkish in a Turkish session. Translating them
    /// would silently stop matching.
    init(userText raw: String) {
        let s = raw.lowercased().trimmingCharacters(in: .whitespaces)
        switch true {
        case s.contains("xls"), s.contains("excel"), s.contains("tablo"), s.contains("sheet"):
            self = .xlsx
        case s.contains("pdf"):
            self = .pdf
        case s.contains("doc"), s.contains("word"):
            self = .docx
        case s.contains("html"), s.contains("site"), s.contains("sayfa"), s.contains("web"):
            self = .html
        case s.contains("md"), s.contains("markdown"):
            self = .md
        default:
            self = .txt
        }
    }

    /// Format from a file extension (for reading/editing).
    init?(fileExtension raw: String) {
        switch raw.lowercased() {
        case "xlsx": self = .xlsx
        case "pdf":  self = .pdf
        case "docx": self = .docx
        case "md", "markdown": self = .md
        case "txt", "text": self = .txt
        case "html", "htm": self = .html
        default: return nil
        }
    }
}
