//
//  MemoryNote.swift
//  Tacet
//
//  The single persistent record of the memory layer (memory-spec §3). Same
//  pattern as UserSkill: the limit constants live on the model, validation is
//  gathered in `isValid`, and the comma-separated raw text is normalized in
//  `keys`.
//
//  The text is hard-limited because matching notes are injected into the 4096
//  token window; every character is budget. When the cap is reached there is NO
//  AUTOMATIC EVICTION — deleting silently is the mirror image of the "no silent
//  learning" principle (spec §3).
//

import Foundation
import SwiftData

/// The kind of the note. Stored as a raw String (simple, for SwiftData enum
/// support). In v1 it is only a tag on the board; it plays no part in injection
/// priority (spec §9/2).
enum MemoryKind: String, Codable, CaseIterable {
    case identity
    case preference
    case relation
    case fact
}

@Model
final class MemoryNote {
    /// Upper bound of the text — roughly 40 tokens (spec §3).
    static let textLimit = 160
    /// The most notes that can be stored. Once full, no new extraction happens.
    static let totalCap = 50
    /// Upper bound on keys — limited because the match scan runs on every message.
    static let keyLimit = 8

    var id: UUID = UUID()
    /// A single-sentence fact; in the user's own words.
    var text: String = ""
    /// Raw `MemoryKind` value. An unknown value read back counts as `.fact`.
    private var rawKind: String = MemoryKind.fact.rawValue
    /// Raw trigger text ("food, restaurant, evening"). Parsing happens in `keys`.
    var rawKeys: String = ""
    /// Which chat it was extracted from (transparency; shown on the board).
    /// If the source chat is deleted the note stays and this field falls empty
    /// (spec §7).
    var sourceChatID: UUID?
    var createdAt: Date = Date()
    /// The user can switch it off; a switched-off note is not injected.
    var isActive: Bool = true

    init(text: String = "",
         kind: MemoryKind = .fact,
         rawKeys: String = "",
         sourceChatID: UUID? = nil) {
        self.id = UUID()
        self.text = text
        self.rawKind = kind.rawValue
        self.rawKeys = rawKeys
        self.sourceChatID = sourceChatID
        self.createdAt = Date()
        self.isActive = true
    }

    var kind: MemoryKind {
        get { MemoryKind(rawValue: rawKind) ?? .fact }
        set { rawKind = newValue.rawValue }
    }

    /// Normalized keys: lowercased, whitespace trimmed, empties dropped, limited.
    var keys: [String] {
        rawKeys
            .split(separator: ",")
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
            .filter { !$0.isEmpty }
            .prefix(Self.keyLimit)
            .map { $0 }
    }

    /// Dedup key (spec §4.3/4). Turkish ı/İ is mapped by hand: with
    /// `lowercased()` alone "İstanbul'da yaşıyorum" and "Istanbul'da yaşıyorum"
    /// produced SEPARATE keys and both were being saved to the board (İ folds to
    /// a combining dotted i, I folds to a plain i). The single source of truth is
    /// `MemoryService.dedupKey` — the filter uses it too.
    var normalizedText: String {
        MemoryService.dedupKey(text)
    }

    /// Is it saveable — text non-empty and within the limit, at least one key.
    /// The 10-character lower bound is the same as the filter in spec §4.3/1.
    var isValid: Bool {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.count >= 10
            && trimmed.count <= Self.textLimit
            && !keys.isEmpty
    }

    /// Subtitle shown on the board: "food · restaurant · evening".
    var summary: String { keys.joined(separator: " · ") }
}
