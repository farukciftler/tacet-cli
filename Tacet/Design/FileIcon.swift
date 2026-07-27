//
//  FileIcon.swift
//  Tacet
//
//  File-kind icons and kind labels — timeline-spec §9.3.
//
//  The icon set lives in the asset catalog (Assets.xcassets/FileIcons) as
//  single-colour template vectors: a shared 24×24 page silhouette + an inner
//  mark. There are no third-party brand colours (Excel green, PDF red); the icon
//  always takes the colour of the text above it.
//
//  The two functions here are PURE: the same input always gives the same output,
//  they have no side effects and never touch the file system. SelfTest verifies
//  them with 20 kinds + synonyms + fallback cases.
//

import SwiftUI
import UniformTypeIdentifiers

enum FileIcon {

    // MARK: - Set

    /// The 20 kinds from §9.3. The asset name is "file-" + this value.
    static let knownKinds: [String] = [
        "pdf", "docx", "md", "txt", "rtf",       // document
        "xlsx", "csv", "json",                   // table / data
        "pptx",                                  // presentation
        "png", "jpg", "heic", "gif", "svg",      // image
        "mp3", "m4a", "wav",                     // audio
        "mp4", "mov",                            // video
        "zip"                                    // archive
    ]

    /// Generic fallback — every extension not in the list lands here.
    /// A card is never drawn without an icon.
    /// NOTE: this is an asset-catalog key, not user-visible text — it stays as is.
    static let genericKind = "document"

    /// Other spellings of the same content. They need no new icon, they fold into
    /// the table. Old Office formats (xls/doc/ppt) also use the icon of their
    /// modern counterparts: to the user both are a "spreadsheet", and a separate
    /// drawing would be brand noise.
    private static let synonyms: [String: String] = [
        "jpeg": "jpg", "jpe": "jpg",
        "markdown": "md", "mdown": "md", "mkd": "md",
        "text": "txt",
        "heif": "heic",
        "tsv": "csv",
        "xls": "xlsx", "doc": "docx", "ppt": "pptx",
        "m4v": "mp4", "qt": "mov",
        "wave": "wav", "aac": "m4a",
        "zipx": "zip"
    ]

    // MARK: - Normalisation

    /// Simplifies the extension: a leading dot is dropped, whitespace trimmed,
    /// everything lowercased. Even if a full file name ("report.xlsx") is given,
    /// the last component is taken.
    static func normalExtension(_ raw: String) -> String {
        var e = raw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if let last = e.split(separator: ".").last, !e.isEmpty {
            e = String(last)
        }
        return e
    }

    /// Extension → kind within the set. Everything unknown falls to the generic kind.
    static func kind(extension ext: String) -> String {
        let e = normalExtension(ext)
        let folded = synonyms[e] ?? e
        return knownKinds.contains(folded) ? folded : genericKind
    }

    /// The asset name in the asset catalog.
    /// NOTE: the "file-" prefix is an asset-catalog key owned by Assets.xcassets;
    /// renaming it here without renaming the imagesets breaks every file icon.
    static func assetName(extension ext: String) -> String {
        "file-" + kind(extension: ext)
    }

    // MARK: - Public interface

    /// The template vector icon for the extension. The colour comes from the
    /// caller (`.foregroundStyle`); the icon has no colour of its own.
    static func icon(extension ext: String) -> Image {
        Image(assetName(extension: ext))
            .renderingMode(.template)
    }

    /// The kind label — it comes from the `UTType` localisation, not from the
    /// extension ("Spreadsheet", "PNG image"). If the system cannot resolve it,
    /// the extension alone is written in uppercase.
    static func kindLabel(extension ext: String) -> String {
        let e = normalExtension(ext)
        guard !e.isEmpty else { return "" }
        let upper = e.uppercased()
        guard let type = UTType(filenameExtension: e),
              let description = type.localizedDescription?
                  .trimmingCharacters(in: .whitespacesAndNewlines),
              !description.isEmpty
        else { return upper }
        return capitalisedFirst(description)
    }

    /// System descriptions sometimes start lowercase ("PDF document" /
    /// "spreadsheet"); the label starts a line, so the first letter is
    /// uppercased. Abbreviations that already start uppercase (PDF, JPEG) are
    /// left alone.
    private static func capitalisedFirst(_ text: String) -> String {
        guard let first = text.first, first.isLowercase else { return text }
        return first.uppercased() + text.dropFirst()
    }
}
