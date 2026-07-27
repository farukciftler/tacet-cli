//
//  ProvideDocumentIntent.swift
//  Tacet
//
//  CLASS C — a PROVIDING intent (intent-plan §1.5). THE ONLY surface that takes REAL data out
//  of the app; that is why the only real gate is here too: `ShortcutSetting.exportEnabled` is
//  OFF BY DEFAULT and can only be turned on from inside the app.
//
//  WHY THE EXCEPTION: this is the only usable shape of the "ask someone for an Excel, save it
//  to Files / attach it to an email" chain. A document surface that returns no file would be
//  left half-finished in Shortcuts.
//
//  SCOPE: the file is picked ONLY from the `DocumentContext.filesOnDisk()` list, i.e. from
//  under Documents/Tacet. An arbitrary path is never accepted — a `TacetDocument` id is a
//  relative path and `TacetDocument.url(_:)` eliminates anything that escapes the root.
//

import Foundation
import AppIntents
import SwiftUI
import UniformTypeIdentifiers

struct ProvideDocumentIntent: AppIntent {
    nonisolated static var title: LocalizedStringResource { "Give Tacet’s last document" }

    nonisolated static var description: IntentDescription {
        IntentDescription("Gives a file Tacet created to Shortcuts. The file can leave the device, so you have to grant permission separately inside Tacet.")
    }

    /// The app does not open: no screen is needed to hand over a file, and consent was already
    /// taken beforehand and inside the app (§4.3).
    nonisolated static var openAppWhenRun: Bool { false }

    /// The largest file that can be given to Shortcuts.
    static let sizeLimit: Int64 = 25 * 1024 * 1024

    /// If left empty, the most recently modified file.
    @Parameter(title: "Document")
    var document: TacetDocument?

    nonisolated static var parameterSummary: some ParameterSummary {
        Summary("Give to Shortcuts: \(\.$document)")
    }

    @MainActor
    func perform() async throws -> some IntentResult & ReturnsValue<IntentFile> & ProvidesDialog & ShowsSnippetView {
        // 1. The gate. While it is closed it DOES NOT SILENTLY RETURN EMPTY: it shows an
        //    explicit error and describes how to open it.
        guard ShortcutSetting.exportEnabled else { throw IntentError.exportDisabled }

        // 2. The pick — the source is only Documents/Tacet.
        let target: TacetDocument
        if let document {
            guard let found = DocumentQuery.all().first(where: { $0.id == document.id }) else {
                throw IntentError.fileMissing
            }
            target = found
        } else {
            guard let last = DocumentQuery.all().first else { throw IntentError.documentMissing }
            target = last
        }
        guard let url = TacetDocument.url(target.id),
              FileManager.default.fileExists(atPath: url.path) else {
            throw IntentError.fileMissing
        }

        // 3. Size.
        guard target.size <= Self.sizeLimit else { throw IntentError.fileTooLarge }

        // 4. Readability pre-check (a single byte). Because the protection class is
        //    `.completeUnlessOpen`, an automation triggered on a LOCKED device gets an honest
        //    error here instead of being silently handed an empty file.
        try Self.checkReadable(url)

        // 5. Give the file.
        let file = IntentFile(fileURL: url,
                              filename: target.name,
                              type: TacetDocument.typeIdentifier(url))

        // 6. The record: Settings shows this on the "last given" row. What went out must stay
        //    visible in the app after the fact (§4.3).
        ShortcutSetting.recordExport(name: target.name, date: Date())

        return .result(
            value: file,
            dialog: "\(target.name) was given.",
            view: DocumentCard(name: target.name, subtitle: target.subtitle, icon: target.icon))
    }

    /// Tries to read a single byte from the file. If that fails, the device is locked (or the
    /// file really cannot be read).
    static func checkReadable(_ url: URL) throws {
        guard let handle = try? FileHandle(forReadingFrom: url) else {
            throw IntentError.locked
        }
        defer { try? handle.close() }
        do {
            _ = try handle.read(upToCount: 1)
        } catch {
            throw IntentError.locked
        }
    }
}
