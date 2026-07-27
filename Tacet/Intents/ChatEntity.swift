//
//  ChatEntity.swift
//  Tacet
//
//  The chat that can be picked inside Shortcuts (intent-plan §2.1).
//
//  THE LEAK BOUNDARY — three bans, the easiest point of the plan to violate:
//   1. `IndexedEntity` IS NOT CONFORMED TO, `CSSearchableItem` IS NOT DONATED. Chat titles do
//      not enter the system Spotlight database (§7).
//   2. This entity IS NOT embedded in ANY `AppShortcut` phrase. If it were, the results of
//      `suggestedEntities()` would turn into phrases Siri reads out and the titles would be
//      carried onto the voice-assistant surface.
//   3. NO donation IS MADE via `IntentDonationManager`.
//  The only remaining surface: the parameter picker the user opens by hand in Shortcuts.
//  Seeing titles there means looking at your own list on your own device.
//
//  The ONLY field shown is `Chat.title`. `preview` (the content of the last message) is never
//  shown — the title is already the first 40 characters of the first user message, and there
//  is no need to leak more than that.
//

import Foundation
import AppIntents
import SwiftData

nonisolated struct ChatEntity: AppEntity, Identifiable {
    let id: UUID
    let title: String
    let updatedAt: Date

    static var typeDisplayRepresentation: TypeDisplayRepresentation {
        TypeDisplayRepresentation(name: "Chat")
    }

    var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "\(title)", subtitle: "\(Self.dateText(updatedAt))")
    }

    /// `let` — it satisfies the protocol's get-only requirement and leaves no process-wide
    /// mutable static state behind.
    static let defaultQuery = ChatQuery()

    /// The subtitle in the picker. The device's own format is used; there is no separate string.
    static func dateText(_ date: Date) -> String {
        date.formatted(date: .abbreviated, time: .shortened)
    }
}

// MARK: - Query

/// The chat query. On failure it DOES NOT RETURN AN EMPTY ARRAY: it throws `storeMissing` — a
/// silent blank is the lie "you have no chats" (§5).
nonisolated struct ChatQuery: EntityQuery, EntityStringQuery {

    /// The maximum number of results returned by a search by name.
    private static let searchCap = 20
    /// The maximum number of chats suggested in the picker.
    private static let suggestionCap = 10

    @MainActor
    func entities(for identifiers: [ChatEntity.ID]) async throws -> [ChatEntity] {
        let set = Set(identifiers)
        // Collection membership inside a Predicate is fragile in SwiftData; since the chat
        // count is small anyway, the filtering is done in memory.
        return try Self.chats().filter { set.contains($0.id) }.map { Self.entity($0) }
    }

    @MainActor
    func entities(matching string: String) async throws -> [ChatEntity] {
        let needle = string.trimmingCharacters(in: .whitespacesAndNewlines)
        let all = try Self.chats()
        guard !needle.isEmpty else {
            return all.prefix(Self.searchCap).map { Self.entity($0) }
        }
        return all
            .filter { $0.title.localizedStandardContains(needle) }
            .prefix(Self.searchCap)
            .map { Self.entity($0) }
    }

    @MainActor
    func suggestedEntities() async throws -> [ChatEntity] {
        try Self.chats().prefix(Self.suggestionCap).map { Self.entity($0) }
    }

    // MARK: - Store

    /// Most recently updated first. If there is no store, `storeMissing` — intents DO NOT set up
    /// THEIR OWN containers (§RISK 1: opening the same store with two containers breaks it).
    @MainActor
    static func chats() throws -> [Chat] {
        guard let context = StoreGate.context else { throw IntentError.storeMissing }
        var descriptor = FetchDescriptor<Chat>(
            sortBy: [SortDescriptor(\Chat.updatedAt, order: .reverse)])
        descriptor.fetchLimit = 200
        do {
            return try context.fetch(descriptor)
        } catch {
            throw IntentError.storeMissing
        }
    }

    @MainActor
    static func entity(_ chat: Chat) -> ChatEntity {
        ChatEntity(id: chat.id, title: chat.title, updatedAt: chat.updatedAt)
    }
}
