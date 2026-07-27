//
//  DemoSeed.swift
//  Tacet
//
//  DEBUG-only store seeding, for App Store screenshots and for looking at the
//  interface without a model.
//
//  WHY THIS EXISTS: the simulator has no Apple Intelligence, so
//  `SystemLanguageModel.availability` reports unavailable and the chat stays
//  empty. Screenshots taken there would show the failure state, not the product.
//  Seeding writes REAL records through the REAL models, so what gets captured is
//  the actual interface rendering actual data — not a mockup.
//
//  THE RULE FOR WHAT MAY GO IN HERE: only states the app can genuinely produce.
//  A chip that no tool emits, a format no engine writes, a permission flow that
//  does not exist — none of those belong here, because a screenshot of them
//  would be a claim the product cannot keep.
//

#if DEBUG
import Foundation
import SwiftData

enum DemoSeed {
    /// Seeds three chats, each one built for a single screenshot.
    ///
    /// Idempotent by title: launching twice does not stack duplicates, so the
    /// capture script can run the app repeatedly without a clean install.
    static func run(_ container: ModelContainer) {
        let context = container.mainContext
        let existing = (try? context.fetch(FetchDescriptor<Chat>())) ?? []
        let titles = Set(existing.map(\.title))

        for chat in [documentsChat(), calculationChat(), approvalChat()]
        where !titles.contains(chat.title) {
            insert(chat, into: context)
        }
        try? context.save()
    }

    private static func insert(_ seed: SeededChat, into context: ModelContext) {
        let chat = Chat(title: seed.title, titleIsAutomatic: false)
        chat.createdAt = seed.date
        chat.updatedAt = seed.date
        context.insert(chat)
        for (offset, message) in seed.messages.enumerated() {
            message.chat = chat
            message.createdAt = seed.date.addingTimeInterval(Double(offset) * 12)
            context.insert(message)
        }
    }

    private struct SeededChat {
        let title: String
        let date: Date
        let messages: [Message]
    }

    // A fixed reference point: screenshots taken on different days must not
    // differ, otherwise the set cannot be regenerated identically.
    private static let reference = Date(timeIntervalSince1970: 1_774_000_000)

    // MARK: - 1 · A document, and the table drawn back into the chat

    private static func documentsChat() -> SeededChat {
        let read = ToolTrace(
            icon: "doc",
            text: String(localized: "Document read · 3 sheets"),
            state: .readOk,
            rawInput: #"{"path":"expenses.xlsx","focus":"summary"}"#,
            rawOutput: "3 sheets · 412 rows · columns: date, category, amount")
        let create = ToolTrace(
            icon: "doc",
            text: String(localized: "Spreadsheet written · summary.xlsx"),
            state: .written,
            rawInput: #"{"format":"excel","name":"summary","source_ref":"document#1"}"#,
            rawOutput: "summary.xlsx · 4 rows · =SUM(B2:B4)")
        // NO `filePath` ON PURPOSE. A file card resolves its path at draw time;
        // seeded records point at a file that was never written, so the card
        // renders "no longer on this device" — true, but it would be the one
        // negative sentence in a screenshot. The chip already says the
        // spreadsheet was written, and that part is honest without a fake file.

        return SeededChat(
            title: String(localized: "Expense summary"),
            date: reference,
            messages: [
                Message(role: .user, content: String(localized: "Read expenses.xlsx and total it by category")),
                Message(role: .tacet,
                        content: String(localized: """
                        Here is the total by category:

                        | Category | Amount |
                        | --- | --- |
                        | Rent | 24,000 |
                        | Groceries | 8,450 |
                        | Transport | 3,120 |

                        The spreadsheet carries a live `=SUM()` formula, not a frozen number.
                        """),
                        traces: [read, create]),
            ])
    }

    // MARK: - 2 · Arithmetic is solved in code, never in the model

    private static func calculationChat() -> SeededChat {
        let calculate = ToolTrace(
            icon: "function",
            text: "18% of 2,450 = 441",
            state: .readOk,
            rawInput: #"{"expression":"2450 * 0.18"}"#,
            rawOutput: "441")
        let remember = ToolTrace(
            icon: "curlybraces",
            text: String(localized: "Noted · stays on this device"),
            state: .written,
            rawInput: #"{"action":"save","note":"VAT rate is 18%"}"#,
            rawOutput: "saved")

        return SeededChat(
            title: String(localized: "VAT"),
            date: reference.addingTimeInterval(-3600),
            messages: [
                Message(role: .user, content: String(localized: "What is 18% of 2,450? Remember the rate.")),
                Message(role: .tacet,
                        content: String(localized: "441. I noted the rate — it stays on this device."),
                        traces: [calculate, remember]),
            ])
    }

    // MARK: - 3 · The approval gate: the moment data would leave the device

    private static func approvalChat() -> SeededChat {
        let waiting = ToolTrace(
            icon: "hand.raised",
            text: String(localized: "your server · awaiting approval"),
            state: .awaitingApproval,
            rawInput: #"{"query":"train times Ankara Istanbul"}"#,
            rawOutput: nil)

        return SeededChat(
            title: String(localized: "Train times"),
            date: reference.addingTimeInterval(-7200),
            messages: [
                Message(role: .user, content: String(localized: "Look up the evening train times")),
                Message(role: .tacet,
                        content: String(localized: """
                        This one needs the web. Your notes were read in this session, so I am asking first: \
                        the text above is exactly what would be sent to your own search server.
                        """),
                        traces: [waiting]),
            ])
    }
}
#endif
