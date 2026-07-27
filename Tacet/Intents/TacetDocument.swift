//
//  TacetDocument.swift
//  Tacet
//
//  A file Tacet produced that can be picked inside Shortcuts (intent-plan §2.2).
//
//  `id` IS NOT AN ABSOLUTE PATH, it is a path RELATIVE to Documents/Tacet: the UUID of the app
//  container changes on update, and a shortcut storing an absolute path would silently break at
//  the first update. A relative path can always be looked up again; if it is not found, an
//  honest "This file no longer exists." error appears.
//
//  The same three bans are shared with `ChatEntity`: no indexing, no embedding into an App
//  Shortcut phrase, no donation.
//
//  THE TYPE NAME IS A PERMANENT IDENTITY: this struct's name is embedded into the compiled
//  Metadata.appintents and the shortcuts the user builds bind to it. Renaming it AFTER release
//  breaks those shortcuts — this struct carried the previous brand name, so the migration was
//  completed before release. The old name lives only in the git history and is deliberately not
//  written here.
//
//  ISOLATION NOTE: `displayRepresentation` is a NON-isolated protocol requirement, while
//  `DocumentFormat` is on the MainActor (because of the project-wide MainActor default). That
//  is why the two texts derived from the format — the subtitle and the icon — are computed and
//  stored WHILE THE ENTITY IS BEING BUILT. No new dictionary is opened: the values still come
//  from `DocumentFormat.tag` / `.icon`.
//

import Foundation
import AppIntents
import UniformTypeIdentifiers

nonisolated struct TacetDocument: AppEntity, Identifiable {
    /// The path relative to Documents/Tacet ("report.xlsx", "Attached/deck.pdf").
    let id: String
    let name: String
    /// "Excel · 24 KB · 3 Jul 2026" — `DocumentFormat.tag` + size + date.
    let subtitle: String
    /// SF Symbol; the same source as `DocumentFormat.icon`.
    let icon: String
    let size: Int64
    let date: Date

    static var typeDisplayRepresentation: TypeDisplayRepresentation {
        TypeDisplayRepresentation(name: "Tacet document")
    }

    var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "\(name)", subtitle: "\(subtitle)")
    }

    static let defaultQuery = DocumentQuery()

    // MARK: - Disk mapping

    /// The Documents/Tacet root. A read path: it does not create the folder.
    static var root: URL { DocumentContext.outputFolderPath() }

    /// The relative id from an absolute URL. If the file is not UNDER the root, `nil` — an
    /// arbitrary path never turns into an entity.
    static func relativePath(_ url: URL) -> String? {
        let rootPath = root.standardizedFileURL.path
        let filePath = url.standardizedFileURL.path
        guard filePath.hasPrefix(rootPath + "/") else { return nil }
        return String(filePath.dropFirst(rootPath.count + 1))
    }

    /// The absolute URL from a relative id. Ids that escape the scope ("../") are eliminated
    /// because after standardization they do not stay under the root.
    static func url(_ relative: String) -> URL? {
        guard !relative.isEmpty else { return nil }
        let candidate = root.appendingPathComponent(relative).standardizedFileURL
        guard candidate.path.hasPrefix(root.standardizedFileURL.path + "/") else { return nil }
        return candidate
    }

    /// An entity from a file on disk. A file out of scope is `nil`.
    /// MainActor: `DocumentFormat` members are touched here (see the isolation note).
    @MainActor
    static func entity(_ url: URL) -> TacetDocument? {
        guard let relative = relativePath(url) else { return nil }
        let values = try? url.resourceValues(forKeys: [.fileSizeKey, .contentModificationDateKey])
        let format = DocumentFormat(fileExtension: url.pathExtension) ?? .txt
        let size = Int64(values?.fileSize ?? 0)
        let date = values?.contentModificationDate ?? .distantPast
        let measure = ByteCountFormatter.string(fromByteCount: size, countStyle: .file)
        let day = date.formatted(date: .abbreviated, time: .omitted)
        return TacetDocument(id: relative,
                             name: url.lastPathComponent,
                             subtitle: "\(format.tag) · \(measure) · \(day)",
                             icon: format.icon,
                             size: size,
                             date: date)
    }

    /// The type identifier of the file handed to Shortcuts. An unknown extension is `.data`.
    static func typeIdentifier(_ url: URL) -> UTType {
        UTType(filenameExtension: url.pathExtension) ?? .data
    }
}

// MARK: - Query

/// The document query. The source is `DocumentContext.filesOnDisk()` — only under
/// Documents/Tacet; the SwiftData store IS NOT REQUIRED, so this works correctly even when the
/// store could never be opened.
nonisolated struct DocumentQuery: EntityQuery, EntityStringQuery {

    private static let searchCap = 20
    private static let suggestionCap = 10

    @MainActor
    func entities(for identifiers: [TacetDocument.ID]) async throws -> [TacetDocument] {
        let set = Set(identifiers)
        return Self.all().filter { set.contains($0.id) }
    }

    @MainActor
    func entities(matching string: String) async throws -> [TacetDocument] {
        let needle = string.trimmingCharacters(in: .whitespacesAndNewlines)
        let list = Self.all()
        guard !needle.isEmpty else { return Array(list.prefix(Self.searchCap)) }
        return Array(list.filter { $0.name.localizedStandardContains(needle) }
            .prefix(Self.searchCap))
    }

    @MainActor
    func suggestedEntities() async throws -> [TacetDocument] {
        Array(Self.all().prefix(Self.suggestionCap))
    }

    /// Newest first (`filesOnDisk` is already sorted by modification date).
    @MainActor
    static func all() -> [TacetDocument] {
        DocumentContext.filesOnDisk().compactMap { TacetDocument.entity($0) }
    }
}
