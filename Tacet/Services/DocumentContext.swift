//
//  DocumentContext.swift
//  Tacet
//
//  The document context: the active document shared into the chat (for reading /
//  editing) and the produced files (QuickLook preview + sharing + saving to
//  Files). The tools reach in here; the UI previews from here. Same pattern as
//  ToolExecutor.
//

import Foundation
import Observation

/// A document attached to the chat (shared by the user).
struct AttachedDocument: Identifiable, Hashable {
    var id = UUID()
    var url: URL
    var name: String
    var format: DocumentFormat
}

@MainActor
@Observable
final class DocumentContext {
    /// The document currently active in the chat, readable/editable.
    var activeDocument: AttachedDocument?
    /// The files produced in this session (newest last).
    private(set) var produced: [URL] = []
    /// The last produced/requested file the UI will open with QuickLook.
    var toPreview: URL?
    /// The document Tacet has just produced. Even if the user attached nothing,
    /// follow-up requests such as "show it as a table" / "add a row" bind to this.
    private(set) var lastProduced: AttachedDocument?

    /// The document the tools will work on: the one the user attached if there is one,
    /// otherwise the last one produced in this chat. It never leaves a follow-up request
    /// without context.
    var runnableDocument: AttachedDocument? { activeDocument ?? lastProduced }

    /// The protection class used app-wide: unreadable while the device is locked, but
    /// not `.complete` so that new files can still be written while locked.
    nonisolated static let protectionClass = FileProtectionType.completeUnlessOpen

    /// The PATH of Documents/Tacet. A pure computation — it never touches the disk and
    /// never creates the folder. The read paths (size/listing) use this.
    nonisolated static func outputFolderPath() -> URL {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Tacet", isDirectory: true)
    }

    /// Folder setup happens ONCE per process. `static let` is lazy and initialised
    /// thread-safely; createDirectory/setResourceValues no longer run again on every size
    /// query (the Settings screen was calling this on every redraw).
    nonisolated private static let rootSetup: Void = {
        prepareFolder(outputFolderPath())
    }()

    /// The folder Tacet outputs are written to: Documents/Tacet.
    /// The promise "everything stays on this device" turns into code here: the folder is
    /// protected so it cannot be read while the device is locked, and it is excluded from
    /// the iCloud/iTunes backup.
    /// Only WRITE paths should call it — for reading there is `outputFolderPath()`.
    nonisolated static func outputFolder() -> URL {
        _ = rootSetup
        return outputFolderPath()
    }

    /// The folder for DEBUG test outputs: Caches/tacet-test. It is separate from the
    /// production folder — test logs can contain real calendar/contact answers, they must
    /// not sit among the user's documents and must not end up in a backup or a share.
    nonisolated static func testFolder() -> URL {
        _ = testSetup
        return FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("tacet-test", isDirectory: true)
    }

    nonisolated private static let testSetup: Void = {
        let cache = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
        prepareFolder(cache.appendingPathComponent("tacet-test", isDirectory: true))
    }()

    /// Excludes the given path from the iCloud/iTunes backup.
    nonisolated static func excludeFromBackup(_ url: URL) {
        var target = url
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try? target.setResourceValues(values)
    }

    /// Sets a FOLDER up (creating it if absent) with the protection class and excludes it
    /// from the backup. Everywhere that opens a subfolder under Tacet (e.g. "Attached/")
    /// must call this, so the subfolder is protected like the root. Returns the folder URL.
    @discardableResult
    nonisolated static func prepareFolder(_ folder: URL) -> URL {
        try? FileManager.default.createDirectory(
            at: folder, withIntermediateDirectories: true,
            attributes: [.protectionKey: protectionClass])
        applyProtection(folder)
        return folder
    }

    /// Opens a named subfolder under the Tacet root (e.g. `subfolder("Attached")`).
    /// It comes back with the protection + backup exclusion already applied.
    @discardableResult
    nonisolated static func subfolder(_ name: String) -> URL {
        prepareFolder(outputFolder().appendingPathComponent(name, isDirectory: true))
    }

    /// Applies the protection class + backup exclusion to a single file/folder.
    /// There is no need to call it directly for files: the `DocumentEngine.write(...)`
    /// wrapper applies it on the write path itself (so it cannot be forgotten when a new
    /// engine is added).
    nonisolated static func applyProtection(_ url: URL) {
        try? FileManager.default.setAttributes(
            [.protectionKey: protectionClass],
            ofItemAtPath: url.path)
        excludeFromBackup(url)
    }

    func addDocument(url: URL) {
        let format = DocumentFormat(fileExtension: url.pathExtension) ?? .txt
        activeDocument = AttachedDocument(url: url, name: url.lastPathComponent, format: format)
    }

    func removeDocument() { activeDocument = nil }

    func outputAdded(_ url: URL) {
        Self.applyProtection(url)
        produced.append(url)
        toPreview = url
        let format = DocumentFormat(fileExtension: url.pathExtension) ?? .txt
        lastProduced = AttachedDocument(url: url, name: url.lastPathComponent, format: format)
    }

    /// New chat: the production history is cleared too, otherwise the new chat reads the
    /// old file. Only the in-memory list is cleared — the files stay on disk, deleting
    /// them is the user's decision (see `deleteAllFiles()`).
    func forgetProduced() {
        produced.removeAll()
        lastProduced = nil
    }

    // MARK: - Disk management (for the Settings screen)

    /// Every file under Documents/Tacet (the produced ones + the copies the user attached),
    /// newest first. Folders are not listed, they are walked into.
    nonisolated static func filesOnDisk() -> [URL] {
        // A read path: it neither creates nor modifies the folder. If the folder does not
        // exist yet the list is empty.
        let root = outputFolderPath()
        guard let walker = FileManager.default.enumerator(
            at: root,
            includingPropertiesForKeys: [.isRegularFileKey, .contentModificationDateKey],
            options: [.skipsHiddenFiles]) else { return [] }

        var found: [(url: URL, date: Date)] = []
        for item in walker {
            guard let url = item as? URL,
                  let attributes = try? url.resourceValues(
                    forKeys: [.isRegularFileKey, .contentModificationDateKey]),
                  attributes.isRegularFile == true else { continue }
            found.append((url, attributes.contentModificationDate ?? .distantPast))
        }
        return found.sorted { $0.date > $1.date }.map(\.url)
    }

    /// The total size of every file under Documents/Tacet (bytes).
    nonisolated static func totalSize() -> Int64 {
        filesOnDisk().reduce(into: Int64(0)) { total, url in
            let attributes = try? url.resourceValues(forKeys: [.fileSizeKey])
            total += Int64(attributes?.fileSize ?? 0)
        }
    }

    /// Same as `totalSize()`; the name the Settings screen expects.
    nonisolated static func outputSize() -> Int64 { totalSize() }

    /// Same as `deleteAllFiles()`; the name the Settings screen expects.
    /// It throws nothing and returns no count — it clears silently.
    func deleteOutputs() { deleteAllFiles() }

    /// Deletes a single produced file.
    nonisolated static func deleteFile(_ url: URL) {
        try? FileManager.default.removeItem(at: url)
    }

    /// The number of plain files under a path (1 if the path is itself a file).
    /// Called BEFORE deletion — it gives the real number of files deleted.
    nonisolated private static func fileCount(_ url: URL) -> Int {
        let attributes = try? url.resourceValues(forKeys: [.isDirectoryKey])
        guard attributes?.isDirectory == true else { return 1 }
        guard let walker = FileManager.default.enumerator(
            at: url, includingPropertiesForKeys: [.isRegularFileKey]) else { return 0 }
        var counter = 0
        for item in walker {
            guard let lower = item as? URL,
                  let o = try? lower.resourceValues(forKeys: [.isRegularFileKey]),
                  o.isRegularFile == true else { continue }
            counter += 1
        }
        return counter
    }

    /// Deletes the contents of Documents/Tacet entirely and clears the in-memory context
    /// too. IRREVERSIBLE. Returns the real number of files deleted (including the ones
    /// inside subfolders). The Settings screen calls it after a confirmation.
    ///
    /// Scope safety: only the DIRECT contents of `Documents/Tacet` are deleted; the root
    /// itself stays and under no condition is a directory above it reached.
    @discardableResult
    func deleteAllFiles() -> Int {
        let fm = FileManager.default
        let documents = fm.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let root = Self.outputFolderPath()
        // Is the root really Documents/Tacet? If not, delete nothing.
        guard root.lastPathComponent == "Tacet",
              root.deletingLastPathComponent().standardizedFileURL == documents.standardizedFileURL
        else { return 0 }

        var counter = 0
        // A single pass: walk the root's direct contents, count each item before deleting
        // it, then delete.
        if let content = try? fm.contentsOfDirectory(
            at: root, includingPropertiesForKeys: [.isDirectoryKey], options: []) {
            for item in content {
                let count = Self.fileCount(item)
                if (try? fm.removeItem(at: item)) != nil { counter += count }
            }
        }

        // No context may be left pointing at a deleted file.
        activeDocument = nil
        toPreview = nil
        forgetProduced()
        return counter
    }
}
