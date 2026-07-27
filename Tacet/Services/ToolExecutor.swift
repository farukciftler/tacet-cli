//
//  ToolExecutor.swift
//  Tacet
//
//  The tool chip ↔ tool mapping (spec §7.4). The single source of truth: the tool
//  itself. When a tool call starts a "running" chip drops; when the tool returns
//  the chip moves to its final state. The chip text is produced by the tool, it is
//  never written by the model.
//

import Foundation
import Observation

/// The outcome of an approval request. `Bool` WAS NOT ENOUGH: "the user declined" and
/// "the user was NEVER asked" both fell to the same `false`, and the tool reported both
/// to the model as a refusal — so the model said "you declined to share". But on the
/// contention path the user never even saw that question; what is being said is A LIE
/// about something the user did.
///
/// The type is AT FILE LEVEL, not inside `ToolExecutor`: the `ApprovalGate` contract
/// (Tools/MCPTool.swift) returns it, and the purpose of that protocol is to keep the
/// tools independent of the concrete executor.
enum ApprovalDecision: Equatable {
    /// Approved — or never asked, because the session was clean.
    case accepted
    /// The user said "do not send" (or this source had already been declined this session).
    case denied
    /// Another approval was pending at the same time; this request was never shown to the
    /// user. It IS NOT a refusal: it can be retried on the next turn.
    case busy
    /// The turn was cancelled ("stop") or a new turn started; no decision was ever taken.
    case cancelled

    /// The English FACT sentence returned to the model; `nil` = the gate is open and the
    /// flow continues.
    ///
    /// All four are SEPARATE sentences: "declined" is something the user did, the other
    /// two are not. If the model cannot tell them apart it attributes to the user a
    /// decision they never made.
    var toModel: String? {
        switch self {
        case .accepted:
            nil
        case .denied:
            "user_declined: the user chose not to send this data; nothing was sent"
        case .busy:
            "approval_unavailable: another approval was already pending, so the user "
                + "was never asked; nothing was sent"
        case .cancelled:
            "turn_cancelled: the turn was stopped before the user answered; nothing was sent"
        }
    }
}

/// The interface through which tools report the chip lifecycle. MainActor — UI state.
@MainActor
protocol ToolReporter: AnyObject, Sendable {
    /// Drops a "running" chip and returns the chip id.
    func start(icon: String, text: String) -> UUID
    /// Moves the chip to its final state. Fields not supplied are preserved.
    func update(_ id: UUID, state: ToolState, text: String?, rawInput: String?, rawOutput: String?, filePath: String?)
}

/// The executor that accumulates the tool chips of the active turn. ModelService owns
/// it; ChatView observes the live chips from here, and at the end of the turn they move
/// into Message.
@MainActor
@Observable
final class ToolExecutor: ToolReporter {
    /// The chips dropped in the active turn, in call order.
    private(set) var traces: [ToolTrace] = []

    /// Did a tool that "changes the world" (`.written`) run in this turn — an event was
    /// written, a reminder was set, a document was produced/edited.
    ///
    /// STICKY: the error recovery path calls `newTurn(forgetSideEffects: false)` before a
    /// retry; had the flag been reset there, the information about whether a side effect
    /// happened would be lost exactly when it is needed. Only a real new turn (the default
    /// of `newTurn(forgetSideEffects:)`) resets it.
    private(set) var worldChanged = false

    /// Did a REMOTE (off-device) call return successfully in this turn — i.e. a side
    /// effect MAY have occurred on the user's own server.
    ///
    /// It is a SEPARATE axis from `worldChanged` and it has to be: that flag is set only
    /// by a `.written` chip, whereas a remote call is deliberately left `.read`
    /// (`MCPTool.callRemote`: `.written` triggers the local undo/recovery logic and would
    /// be wrong for a remote call). The consequence: a general error occurring AFTER a
    /// call that opened a Jira issue put `recoverFromError` into a retry, the SAME prompt
    /// went a second time, and a second issue was opened.
    ///
    /// STICKY like `worldChanged`: it is preserved when the recovery path calls
    /// `newTurn(forgetSideEffects: false)`, and only a real new turn resets it.
    ///
    /// NO CLASS DISTINCTION IS MADE (read-only/write). The classification
    /// (`SideEffectClass`) is a GUESS resting on the server's hints and name intuition; if
    /// a remote tool believed to be "read-only" keeps a record, increments a counter or
    /// triggers an email, that cannot be undone. The cost of blocking one retry too many
    /// is an error bubble; the cost of missing one is a doubled external side effect.
    private(set) var mayHaveExternalEffect = false

    /// Called after a remote call returned SUCCESSFULLY. One-way; it stays for the turn.
    func markSideEffect() { mayHaveExternalEffect = true }

    /// Is a retry safe — `recoverFromError` reads this as the single decision point.
    /// If both axes are clean, the same prompt can be sent a second time.
    var retryIsSafe: Bool { !worldChanged && !mayHaveExternalEffect }

    // MARK: - Tainted session (mcp §5.6)

    /// Has a personal-data tool (Calendar, Contact, Search/Spotlight, Document*,
    /// Reminder) run SUCCESSFULLY at least once in this session?
    ///
    /// It IS NOT CLEARED during the session and `newTurn()` does not reset it: when a new
    /// session is opened through context summarisation, the summary itself can carry
    /// personal data, so the taint carries too. Only a real chat reset (`resetChat()`)
    /// clears it.
    ///
    /// The model has no access to this flag; the gate is in code, not in the model (mcp §2.2).
    private(set) var sessionTainted = false

    /// Called after a successful call of a personal-data tool. One-way.
    func taint() { sessionTainted = true }

    /// The sources for which the user declined sharing in this session. No second approval
    /// chip is produced for the same source; later attempts silently receive the same
    /// refusal — the model cannot enter an insistence loop (mcp §3.3).
    private var declinedSources: Set<String> = []

    /// The request currently awaiting a user decision. The view layer observes this and
    /// presents the approval sheet; the decision comes back via `decideApproval(_:)`.
    private(set) var pendingApproval: ApprovalRequest?

    /// The continuation of the `requestApprovalDecision` call awaiting a decision. At most
    /// one at a time.
    private var approvalContinuation: CheckedContinuation<ApprovalDecision, Never>?

    /// A single sharing request to be put to the user.
    struct ApprovalRequest: Identifiable, Equatable {
        let id = UUID()
        /// The connection/server name — "home server", "search server".
        let source: String
        let toolName: String
        /// EXACTLY THE ARGUMENTS THAT WILL BE SENT. Not a category summary.
        let content: String
        /// This request's chip in the flow.
        let traceID: UUID
    }

    /// Called before any data leaves the device. Nothing is sent other than on `.accepted`.
    ///
    /// - If the session is not tainted it returns `.accepted` WITHOUT ASKING: an approval
    ///   is read only if it is rare (mcp §2.4).
    /// - If the source was declined once in this session it returns `.denied` without
    ///   dropping a chip.
    /// - If another gate is open it returns `.busy` — this IS NOT a refusal, the question
    ///   was never asked.
    /// - Otherwise an "awaiting approval" chip drops into the flow and the user's decision
    ///   is awaited.
    ///
    /// - Parameter content: exactly the arguments that will be sent; the user sees this
    ///   verbatim on the approval sheet.
    ///
    /// The FULL outcome of the approval gate — the SINGLE entry point. The short forms
    /// returning `Bool` were removed: there, refusal, contention and cancellation all fell
    /// to the same `false`, and a new caller silently losing that distinction was only a
    /// matter of time.
    ///
    /// - Parameter required: if true the question is asked even when the session is clean.
    ///   Destructive remote tools use it: there the question is not "what leaves the
    ///   device" but "what changes on the user's server" — unrelated to the taint.
    func requestApprovalDecision(source: String,
                          toolName: String,
                          content: String,
                          required: Bool = false) async -> ApprovalDecision {
        guard sessionTainted || required else { return .accepted }
        if declinedSources.contains(source) { return .denied }
        // We do not open a second gate at the same time — the user must not be trapped
        // between two sheets. But this IS NOT a refusal: the question was never asked and
        // no chip drops.
        if pendingApproval != nil { return .busy }

        let traceID = start(icon: "hand.raised", text: L10n.connectionAwaitingApproval(source))
        update(traceID, state: .awaitingApproval, rawInput: content)

        let decision: ApprovalDecision = await withCheckedContinuation { continuation in
            approvalContinuation = continuation
            pendingApproval = ApprovalRequest(source: source, toolName: toolName,
                                      content: content, traceID: traceID)
        }

        switch decision {
        case .accepted:
            // The chip returns to the normal lifecycle: the tool's own chip takes over and
            // the waiting chip leaves no trace in the flow.
            traces.removeAll { $0.id == traceID }
        case .denied:
            declinedSources.insert(source)
            update(traceID, state: .notSent, text: L10n.connectionNotSent(source))
        case .cancelled, .busy:
            // CANCELLATION IS NOT A REFUSAL: the source IS NOT blacklisted for the
            // session. Never being able to send data to a server again because the user
            // pressed "stop" would load permanence onto a decision they never made. The
            // chip still stays "not sent" — nothing left.
            update(traceID, state: .notSent, text: L10n.connectionNotSent(source))
        }
        return decision
    }

    /// The user's decision on the approval sheet. Dismissing the sheet = `false`.
    func decideApproval(_ accepted: Bool) {
        resolveApproval(accepted ? .accepted : .denied)
    }

    /// If an approval is pending, resolves it as CANCELLED — a turn cancellation must not
    /// leave a suspended continuation behind (otherwise the tool task leaks in suspension
    /// until the next message and the approval sheet stays on screen).
    ///
    /// `newTurn()` and `ModelService.stop()` call it; both must be able to reach it.
    func resolvePendingApproval() { resolveApproval(.cancelled) }

    /// The continuation is resumed EXACTLY ONCE: the field is nil'ed BEFORE the resume, so
    /// a second call falls out at the `guard`. (A double resume is a run-time crash, and
    /// `stop()` can race the user's decision on the approval sheet.)
    private func resolveApproval(_ decision: ApprovalDecision) {
        guard let continuation = approvalContinuation else { return }
        approvalContinuation = nil
        pendingApproval = nil
        continuation.resume(returning: decision)
    }

    // MARK: - Turn lifecycle

    /// The hook for external counters reset per turn (code-spec §5.4: the code attempt
    /// counter is reset in `ToolExecutor.newTurn`). The owner (ModelService) installs it;
    /// it is kept generic — this class knows nothing about counter types. Because the hook
    /// fires from a SINGLE point, a future path calling `newTurn` cannot forget to reset
    /// the counter.
    var turnHook: (() -> Void)?

    /// A new turn — reset after the previous turn's chips have been moved into Message.
    ///
    /// `forgetSideEffects: false` = "this is NOT a real new turn, it is a recovery attempt
    /// of the same turn". In that case neither the side-effect flags nor the CHIPS are
    /// reset (audit P2-8).
    ///
    /// Preserving the chips too is a deliberate reversal: the old behaviour did
    /// `traces = []` unconditionally and after a recovery the user could not see which
    /// tool had run on the first attempt — exactly the information error diagnosis needs
    /// most was being deleted at the moment of the error. If two attempts run the same
    /// tool, two chips appear; that is not a defect, it is what actually happened ("it was
    /// tried once, it errored, it was tried again").
    ///
    /// It DELIBERATELY does not reset the tainted-session flag or the refusal cache — both
    /// are session-lived (mcp §5.6, §3.3).
    func newTurn(forgetSideEffects: Bool = true) {
        resolvePendingApproval()
        if forgetSideEffects {
            traces = []
            worldChanged = false
            mayHaveExternalEffect = false
        }
        turnHook?()
    }

    /// A real chat reset: everything session-lived ends here — the taint and the refusal
    /// cache included. A new chat starts clean.
    func resetChat() {
        newTurn()
        sessionTainted = false
        declinedSources.removeAll()
    }

    func start(icon: String, text: String) -> UUID {
        let trace = ToolTrace(icon: icon, text: text, state: .running)
        traces.append(trace)
        return trace.id
    }

    func update(_ id: UUID,
                  state: ToolState,
                  text: String? = nil,
                  rawInput: String? = nil,
                  rawOutput: String? = nil,
                  filePath: String? = nil) {
        if state == .written { worldChanged = true }
        guard let i = traces.firstIndex(where: { $0.id == id }) else { return }
        traces[i].state = state
        if let text { traces[i].text = text }
        if let rawInput { traces[i].rawInput = rawInput }
        if let rawOutput { traces[i].rawOutput = rawOutput }
        if let filePath { traces[i].filePath = filePath }
    }
}
