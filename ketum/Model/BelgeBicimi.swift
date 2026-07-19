//
//  BelgeBicimi.swift
//  ketum
//
//  Belge biçimleri (spec §7.3 üretim araçları). Tümü cihaz-üstü üretilir/okunur;
//  ağ yok. .xlsx ve .docx OOXML (zip) — ZipDeposu ile paketlenir/açılır.
//

import Foundation

enum BelgeBicimi: String, CaseIterable, Sendable {
    case xlsx   // Excel
    case pdf    // PDF
    case docx   // Word
    case md     // Markdown
    case txt    // düz metin
    case html   // tek dosyalık web sayfası (kod-spec §4)

    var uzanti: String { rawValue }

    /// Çip/etiket adı: "Excel oluşturuldu · …" (spec §4.4).
    var etiket: String {
        switch self {
        case .xlsx: return "Excel"
        case .pdf:  return "PDF"
        case .docx: return "Word"
        case .md:   return "Not"
        case .txt:  return "Metin"
        case .html: return "Sayfa"
        }
    }

    /// SF Symbol (outline). Çip ikonu.
    var ikon: String {
        switch self {
        case .xlsx: return "tablecells"
        case .pdf:  return "doc.richtext"
        case .docx: return "doc.text"
        case .md:   return "text.alignleft"
        case .txt:  return "doc.plaintext"
        case .html: return "doc.text.image" // "globe" değil — ağ çağrışımı yapar (kod-spec §4.2)
        }
    }

    /// Bu biçim tabloya mı dayanır (xlsx)? Diğerleri düz/markdown metindir.
    var tabloYapisi: Bool { self == .xlsx }

    /// Modelin serbest metninden ("excel", "word", "pdf", "markdown"…) biçime eşle.
    init(kullaniciMetni ham: String) {
        let s = ham.lowercased().trimmingCharacters(in: .whitespaces)
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

    /// Dosya uzantısından biçim (okuma/düzenleme için).
    init?(uzanti ham: String) {
        switch ham.lowercased() {
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
