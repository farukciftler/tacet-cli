//
//  Chat.swift
//  Tacet
//
//  A chat session (spec §4.7). History is kept on the device with SwiftData;
//  the user can open a new chat and reach the old ones. The messages of each
//  chat hang off a relationship (deleting the chat deletes the messages too).
//

import Foundation
import SwiftData

@Model
final class Chat {
    var id: UUID = UUID()
    var title: String = "New chat"
    var createdAt: Date = Date()
    var updatedAt: Date = Date()
    /// Is the title still the default one (not yet set by the user or by the
    /// first message)? We do NOT compare against the text: once "New chat" is
    /// translated the literal comparison breaks and the title would never be
    /// updated again. Because it carries a default value, old records still
    /// open through lightweight migration.
    var titleIsAutomatic: Bool = true

    @Relationship(deleteRule: .cascade, inverse: \Message.chat)
    var messages: [Message] = []

    init(title: String = "New chat", titleIsAutomatic: Bool = true) {
        self.id = UUID()
        self.title = title
        self.titleIsAutomatic = titleIsAutomatic
        self.createdAt = Date()
        self.updatedAt = Date()
    }

    /// Derives the title from the first user message. Does nothing if the title
    /// was already set by hand or automatically.
    func deriveTitle(_ text: String) {
        guard titleIsAutomatic else { return }
        let summary = text.trimmingCharacters(in: .whitespacesAndNewlines).prefix(40)
        guard !summary.isEmpty else { return }
        title = String(summary)
        titleIsAutomatic = false
    }

    /// Messages ordered by time (the relationship may come back unordered).
    ///
    /// O(n log n) — THE CALLER MUST TAKE IT ONCE AND CARRY IT. Called per row
    /// inside a view body the cost is multiplied by the number of rows
    /// (ChatView.stream keeps it in a single local variable for this reason).
    var orderedMessages: [Message] {
        messages.sorted { $0.createdAt < $1.createdAt }
    }

    var isEmpty: Bool { messages.isEmpty }

    /// Preview of the last line, shown in the list.
    ///
    /// Does NOT sort: the list row read this more than once, so the sort cost
    /// was making the list stutter. The newest message is picked in a single
    /// pass; on an equal timestamp the last one wins — same as
    /// `orderedMessages.last`.
    var preview: String {
        var last: Message?
        for message in messages {
            if let current = last, current.createdAt > message.createdAt { continue }
            last = message
        }
        return last?.content ?? "No messages yet"
    }
}
