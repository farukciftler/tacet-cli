//
//  AddNoteIntent.swift
//  Tacet
//
//  CLASS B — a SHOWING intent (intent-plan §1.4). The app DOES NOT OPEN, and the return is
//  `ProvidesDialog & ShowsSnippetView`; there is NO `ReturnsValue`, i.e. no variable that a
//  Shortcut could capture is produced. The note appears on screen, it cannot enter a pipeline.
//
//  WHY IT CAN RUN SCREENLESS: no model, no permission, no network, no data leaving — only the
//  user's own sentence being written to their own device. The "no silent learning" principle
//  is not violated: the user ordered the learning.
//  A VISIBLE TRACE: the returned snippet shows the note's text and its keys; the note also
//  sits on the Memory board, where it can be turned off/deleted.
//
//  `ToolExecutor` is never touched: none of the `sessionTainted`, `requestApprovalDecision`,
//  `mayHaveSideEffects` paths come into play (§4.4).
//

import Foundation
import AppIntents
import SwiftData
import SwiftUI

struct AddNoteIntent: AppIntent {
    nonisolated static var title: LocalizedStringResource { "Add a note to Tacet" }

    nonisolated static var description: IntentDescription {
        IntentDescription("Adds a one-sentence note for Tacet to remember. The note stays on the device.")
    }

    /// The app does not open — see the file header.
    nonisolated static var openAppWhenRun: Bool { false }

    @Parameter(title: "Note", requestValueDialog: "What should be remembered?")
    var text: String

    /// Comma-separated. If left empty, they are derived deterministically from the text.
    @Parameter(title: "Keywords")
    var keys: String?

    nonisolated static var parameterSummary: some ParameterSummary {
        Summary("Add a note to Tacet: \(\.$text)")
    }

    @MainActor
    func perform() async throws -> some IntentResult & ProvidesDialog & ShowsSnippetView {
        guard let context = StoreGate.context else { throw IntentError.storeMissing }

        // 1. Size — the same limits as `MemoryNote.isValid`.
        let clean = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard clean.count >= 10, clean.count <= MemoryNote.textLimit else {
            throw IntentError.noteSize
        }

        // 2. The cap — when it is full there is NO AUTOMATIC EVICTION (memory-spec §3).
        let existing: [MemoryNote]
        do {
            existing = try context.fetch(FetchDescriptor<MemoryNote>())
        } catch {
            throw IntentError.storeMissing
        }
        guard !MemoryStore.isFull(existing.count) else { throw IntentError.memoryFull }

        // 3. Deduplication: if the same note exists it is NOT AN ERROR — automations must be
        //    idempotent, and a shortcut running every evening does not go red on day two.
        let normalized = clean.lowercased()
        if let found = existing.first(where: { !$0.isDeleted && $0.normalizedText == normalized }) {
            return .result(
                dialog: "That note was already there.",
                view: NoteCard(text: found.text, keys: found.keys))
        }

        // 4. The keys: the parameter if it was given, otherwise derived from the text.
        let selected = Self.resolveKeys(parameter: keys, text: clean)
        guard !selected.isEmpty else { throw IntentError.noteSize }

        // 5. Write. `sourceChatID: nil` — the model did not extract the note, the user dictated
        //    it; showing "no source" on the board is CORRECT. Adding a new stored field would
        //    have meant a schema migration (rule 6).
        let note = MemoryNote(text: clean,
                              kind: .fact,
                              rawKeys: selected.joined(separator: ", "),
                              sourceChatID: nil)
        context.insert(note)
        do {
            try context.save()
        } catch {
            // Showing a note that was not written to disk as "added" is a silent lie.
            context.rollback()
            throw IntentError.couldNotWrite
        }

        // 6. Keep the read path current within the same turn: if the app is in the foreground,
        //    `ContentView` only refreshes the store on a scene change.
        MemoryStore.reload((try? context.fetch(FetchDescriptor<MemoryNote>())) ?? [])

        return .result(
            dialog: "Note added.",
            view: NoteCard(text: clean, keys: selected))
    }

    // MARK: - Key derivation

    /// The maximum number of derived keys stored in one turn.
    static let derivationCap = 5
    /// The shortest word that counts as a key.
    static let shortestKey = 4

    /// Normalizes the parameter if it was given; if it was not (or everything was filtered
    /// out), derives from the text.
    ///
    /// MainActor: `MemoryNote.keyLimit` is a member of the @Model class.
    @MainActor
    static func resolveKeys(parameter: String?, text: String) -> [String] {
        if let parameter {
            let manual = parameter
                .split(separator: ",")
                .map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
                .filter { !$0.isEmpty }
                .prefix(MemoryNote.keyLimit)
                .map { $0 }
            if !manual.isEmpty { return manual }
        }
        return deriveKeys(text)
    }

    /// Deterministic derivation: split the text on non-letters, lowercase it, take the DISTINCT
    /// words of at least `shortestKey` letters, and pick THE LONGEST five.
    ///
    /// `MemoryStore.matching` already scores by key LENGTH; the longest words are already the
    /// most distinctive ones. The model IS NOT INVOLVED — remembering belongs to the code,
    /// extraction to the model (memory-spec §5).
    static func deriveKeys(_ text: String) -> [String] {
        let lowered = text.lowercased(with: Locale(identifier: "tr_TR"))
        var seen = Set<String>()
        var candidates: [String] = []
        for part in lowered.split(whereSeparator: { !$0.isLetter && !$0.isNumber }) {
            let word = String(part)
            guard word.count >= shortestKey, !seen.contains(word) else { continue }
            seen.insert(word)
            candidates.append(word)
        }
        // By length; on a tie the order in the text is preserved (the index is the secondary
        // criterion so the sort is stable).
        return candidates.enumerated()
            .sorted { left, right in
                left.element.count != right.element.count
                    ? left.element.count > right.element.count
                    : left.offset < right.offset
            }
            .prefix(derivationCap)
            .map(\.element)
    }
}
