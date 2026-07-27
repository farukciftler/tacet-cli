//
//  IntentInbox.swift
//  Tacet
//
//  THE single attachment point between the App Intents layer and the app (intent-plan §6.1).
//
//  Intents that open the app (class A) DO NOT make their own model calls: they leave a
//  request behind and the app runs it through the normal turn. The reason is in §1.1 of the
//  plan: a second ModelService set up in the background means a context-less session, an
//  unfed memory store and a screenless permission gate — it would produce a SILENTLY
//  different answer to the same question than the one in the app.
//
//  The inbox holds a single slot: a second request arriving right after the first overwrites
//  it. Keeping a queue would mean accumulating an order the user cannot see; someone asking
//  Siri two questions back to back is waiting for the answer to the second one.
//

import Foundation
import Observation

@MainActor
@Observable
final class IntentInbox {
    static let shared = IntentInbox()

    /// The request the app has to serve.
    enum Request: Equatable, Sendable {
        /// Run this prompt in a new chat (as if it had been typed on the keyboard).
        case ask(String)
        /// Open the past chat with this id.
        case openChat(UUID)
    }

    /// The pending request. `ContentView` observes this and acts when the value changes.
    private(set) var pending: Request?

    private init() {}

    func put(_ request: Request) {
        pending = request
    }

    /// A one-shot read: the same request is not run twice. On a cold launch both `.task` and
    /// `.onChange` can fire, so consuming it is mandatory.
    func consume() -> Request? {
        defer { pending = nil }
        return pending
    }
}
