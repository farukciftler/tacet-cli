//
//  UserSkill.swift
//  Tacet
//
//  A skill the user wrote themselves (the equivalent of SKILL.md). Bundled
//  skills sit read-only inside Skills/*.md; this model stores what the user
//  added in SwiftData. The body is hard-limited to protect the 4096 token
//  window: a matching skill is injected into that session once, so every
//  character is budget.
//

import Foundation
import SwiftData

@Model
final class UserSkill {
    /// Upper bound of the body — the same order as the body of the bundled
    /// skills (~150 tokens).
    static let bodyLimit = 500
    /// Upper bound on triggers — limited because the match scan runs on every
    /// message.
    static let triggerLimit = 12

    var id: UUID = UUID()
    /// Display name; also used as the injection heading.
    var name: String = ""
    /// Raw trigger text ("invoice, expense, spending"). Parsing happens in
    /// `triggers`.
    var rawTriggers: String = ""
    /// Guide text — the instruction handed to the model.
    var body: String = ""
    var isActive: Bool = true
    var createdAt: Date = Date()

    init(name: String = "", rawTriggers: String = "", body: String = "") {
        self.id = UUID()
        self.name = name
        self.rawTriggers = rawTriggers
        self.body = body
        self.createdAt = Date()
    }

    /// Normalized triggers: lowercased, whitespace trimmed, empties dropped,
    /// limited.
    var triggers: [String] {
        rawTriggers
            .split(separator: ",")
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
            .filter { !$0.isEmpty }
            .prefix(Self.triggerLimit)
            .map { $0 }
    }

    /// Is it saveable — all three must be non-empty and the body within the limit.
    var isValid: Bool {
        !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !triggers.isEmpty
            && !body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && body.count <= Self.bodyLimit
    }

    /// Subtitle shown on the board: "invoice · expense · spending".
    var summary: String { triggers.joined(separator: " · ") }
}
