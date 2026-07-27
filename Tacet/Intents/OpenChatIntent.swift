//
//  OpenChatIntent.swift
//  Tacet
//
//  CLASS A — an OPENING intent (intent-plan §1.3). Opens a past chat in the app.
//
//  IT HAS NO App Shortcut PHRASE and that is deliberate (§3): its parameter is an `AppEntity`,
//  and if it were embedded in a phrase the chat titles would be carried onto the Siri surface;
//  if it were embedded without the parameter, Siri would ask by listing the titles OUT LOUD —
//  the spoken version of the same leak. The intent is found from the Shortcuts action list.
//
//  NO model-availability pre-check IS DONE: reading history does not need the model, and it
//  would be a pointless obstacle.
//

import Foundation
import AppIntents

struct OpenChatIntent: AppIntent {
    nonisolated static var title: LocalizedStringResource { "Open a chat in Tacet" }

    nonisolated static var description: IntentDescription {
        IntentDescription("Opens the past chat you pick in Tacet.")
    }

    nonisolated static var openAppWhenRun: Bool { true }

    /// If left empty, the most recently updated chat is opened.
    @Parameter(title: "Chat")
    var chat: ChatEntity?

    nonisolated static var parameterSummary: some ParameterSummary {
        Summary("Open in Tacet: \(\.$chat)")
    }

    @MainActor
    func perform() async throws -> some IntentResult {
        let all = try ChatQuery.chats()   // throws `storeMissing` if there is no store

        // The id is verified AGAINST THE STORE: saying "opened" for a chat deleted after the
        // shortcut was built would be a silent lie.
        let target: Chat?
        if let selected = chat {
            target = all.first { $0.id == selected.id }
        } else {
            target = all.first
        }
        guard let target else { throw IntentError.chatMissing }

        IntentInbox.shared.put(.openChat(target.id))
        return .result()
    }
}
