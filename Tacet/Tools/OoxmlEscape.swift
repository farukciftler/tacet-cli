//
//  OoxmlEscape.swift
//  Tacet
//
//  OOXML (.xlsx/.docx) text escaping — a single copy. There used to be two separate
//  `xmlEscape` functions, one in ExcelEngine and one in DocxEngine, and both only escaped
//  `& < > " '`. Because XML 1.0 control characters (U+0000–U+0008, U+000B, U+000C,
//  U+000E–U+001F) were not filtered out, a single control character in the model's output
//  made the produced document IMPOSSIBLE TO OPEN.
//

import Foundation

enum OoxmlEscape {

    /// Drops the characters that are invalid in XML 1.0, then escapes the special ones.
    /// A single pass: re-escaping an escape sequence is impossible.
    nonisolated static func escape(_ text: String) -> String {
        var result = ""
        result.reserveCapacity(text.unicodeScalars.count)
        for u in text.unicodeScalars where isValid(u) {
            switch u {
            case "&":  result += "&amp;"
            case "<":  result += "&lt;"
            case ">":  result += "&gt;"
            case "\"": result += "&quot;"
            case "'":  result += "&apos;"
            default:   result.unicodeScalars.append(u)
            }
        }
        return result
    }

    /// The XML 1.0 `Char` production: #x9 | #xA | #xD | [#x20-#xD7FF] |
    /// [#xE000-#xFFFD] | [#x10000-#x10FFFF]. (Surrogate code points cannot occur in a
    /// Swift `Unicode.Scalar` in the first place.)
    nonisolated private static func isValid(_ u: Unicode.Scalar) -> Bool {
        switch u.value {
        case 0x09, 0x0A, 0x0D:  return true
        case 0x20...0xD7FF:     return true
        case 0xE000...0xFFFD:   return true
        case 0x10000...0x10FFFF: return true
        default:                return false
        }
    }
}
