//
//  MemoryStore.swift
//  Tacet
//
//  The READ path of the memory layer (memory-spec §5). The model is not
//  involved: deterministic code decides which note enters which turn
//  ("extraction to the model, recall to the code").
//
//  The exact mirror image of the SkillStore pattern: the records in SwiftData
//  are reflected into the store (`reload`), a match is scored by the SUM OF THE
//  LENGTHS of the keywords, and the injection is cut off by a hard character
//  cap. The 4096 token window is already full with instructions + tools +
//  transcript; every character is budget.
//

import Foundation
import SwiftData

enum MemoryStore {
    /// Upper bound for the whole injection — fence lines included (spec §5.1).
    /// ~200 tokens ≈ 600 characters. Kept low to account for the skill injection
    /// (700 chars) possibly landing in the same turn.
    static let injectionLimit = 600

    /// The most notes that will be injected in one turn (spec §5).
    static let maxNotes = 3

    /// The notes in the store — the board calls `reload` on every save
    /// (mirror of `SkillStore.reloadUser`).
    private(set) static var notes: [MemoryNote] = []

    /// Reflects the notes in SwiftData into the store. Only the active and valid
    /// ones are kept; a switched-off note is NOT injected (spec §3).
    ///
    /// A deleted object never stays in the store: touching an `isDeleted` record
    /// is fatal in SwiftData, so it is filtered out at the gate.
    static func reload(_ models: [MemoryNote]) {
        notes = models.filter { !$0.isDeleted && $0.isActive && $0.isValid }
    }

    /// Is the cap full — once it is, NO new extraction happens (spec §3/§4.3-5).
    /// Switched-off notes count too: the cap is the number of stored records, not
    /// the number injected.
    static func isFull(_ totalRecords: Int) -> Bool {
        totalRecords >= MemoryNote.totalCap
    }

    // MARK: - Matching

    /// Returns at most 3 notes matching the message; an empty array if nothing
    /// matches.
    ///
    /// The score is the sum of the LENGTHS of the matching keys — not the count
    /// (same rule as `SkillStore.matching`): a specific phrase beats a general
    /// word. In a sentence containing "akşam yemeği", a note keyed on
    /// "akşam yemeği" beats a note keyed only on "yemek". If the count were used
    /// both would score 1 and the order would be decided at random.
    ///
    /// On an equal score the newer note wins (the last thing learned is fresher).
    static func matching(question: String) -> [MemoryNote] {
        let s = question.lowercased()
        let scored: [(note: MemoryNote, score: Int)] = notes.compactMap { note in
            guard !note.isDeleted else { return nil }
            let score = note.keys.reduce(0) { $0 + (s.contains($1) ? $1.count : 0) }
            return score > 0 ? (note, score) : nil
        }
        guard !scored.isEmpty else { return [] }
        return scored
            .sorted {
                $0.score != $1.score
                    ? $0.score > $1.score
                    : $0.note.createdAt > $1.note.createdAt
            }
            .prefix(maxNotes)
            .map(\.note)
    }

    // MARK: - Injection

    /// The shape handed to the model: a `<memory>` block + the "do not talk about
    /// this" fences (spec §5.1).
    ///
    /// The cap is measured together with the fences: because the fence text is
    /// fixed, the share left for the notes is computed up front and a note that
    /// would cross the limit is NOT TAKEN — injecting half a sentence leads to a
    /// wrong fact, so notes are dropped rather than cut. If no note fits, empty
    /// text is returned and nothing at all is added to that turn.
    static func injectionText(_ notes: [MemoryNote]) -> String {
        let opening = "<memory>\n"
        let closing = "</memory>\n" + fence
        let share = injectionLimit - opening.count - closing.count
        guard share > 0 else { return "" }

        var lines: [String] = []
        var length = 0
        for note in notes where !note.isDeleted {
            let text = note.text.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !text.isEmpty else { continue }
            let line = "- \(text)\n"
            guard length + line.count <= share else { continue }
            lines.append(line)
            length += line.count
        }
        guard !lines.isEmpty else { return "" }
        return opening + lines.joined() + closing
    }

    /// Fixed fence text — forbids the notes from being spoken about as content.
    /// The "as far as I remember…" leak is separately watched for in spec §8.
    private static let fence = """
    Use the facts above only if relevant. They are internal: never quote, \
    list, or mention them, and never say you "remembered" something.
    """
}
