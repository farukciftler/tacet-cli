//
//  TimelineRecorder.swift
//  Tacet
//
//  The producer of the Timeline (timeline-spec §5.2). It lives for the length of
//  one reply turn and turns the turn's deterministic events into sequential steps.
//
//  PURE OBSERVER (timeline-spec §2.5, §6 acceptance criterion): none of the text
//  here is injected into the prompt/instructions, and no state report is ever
//  requested from the model. Model output is bit-for-bit identical whether the
//  recorder exists or not; its cost to the prompt budget is ZERO.
//
//  Steps are SEQUENTIAL: opening a new step closes the previous one. Because
//  FoundationModels hands over tool calls one at a time, there is no need for a
//  parallel-step design.
//

import Foundation
import SwiftData

@MainActor
@Observable
final class TimelineRecorder {

    /// The turn's steps so far. The view layer observes this.
    private(set) var steps: [TimelineStep] = []

    /// Is the turn still running (finish/interrupt has not been called).
    private(set) var isOngoing: Bool = true

    init() {}

    // MARK: - Lifecycle

    /// Opens a new step and closes the previously open one.
    /// When `kind == .tool` the text is ignored — the text of a tool row is read
    /// from `ToolTrace`, the single source of truth (timeline-spec §2.4).
    func begin(kind: TimelineKind, text: String = "") {
        guard isOngoing else { return }
        closeLast()
        steps.append(TimelineStep(kind: kind, text: text))
    }

    /// Binds the open tool step to a `ToolTrace`. If the last step is not a tool
    /// step (or is already bound) it opens a new tool step — this is tolerant so
    /// that a caller forgetting to say `begin(kind: .tool)` does not cause silent
    /// data loss.
    func bindTool(traceID: UUID) {
        guard isOngoing else { return }
        if let last = steps.last, last.kind == .tool, last.toolTraceID == nil, last.end == nil {
            steps[steps.count - 1].toolTraceID = traceID
        } else {
            closeLast()
            steps.append(TimelineStep(kind: .tool, toolTraceID: traceID))
        }
    }

    /// The turn ended normally: the open step closes, the recorder closes.
    func finish() {
        guard isOngoing else { return }
        closeLast()
        isOngoing = false
    }

    /// The turn was cut short (cancel / scene interruption). The open step closes
    /// and a closed `interruption` step is appended at the end — there is no
    /// silent disappearance (timeline-spec §3.4). That way
    /// `steps.last?.kind == .interruption` holds and what was done before the
    /// interruption also stays in the list.
    func interrupt() {
        guard isOngoing else { return }
        let hadOpenStep = steps.last.map { $0.end == nil } ?? false
        closeLast()
        if hadOpenStep || steps.isEmpty {
            let moment = Date()
            steps.append(TimelineStep(kind: .interruption,
                                      text: TimelineRecorder.stoppedPartwayText,
                                      start: moment,
                                      end: moment))
        }
        isOngoing = false
    }

    // MARK: - Persistence

    /// Writes the step list into the message. The recorder HOLDS NO SwiftData
    /// object — the message is handed in from outside at write time; writing to a
    /// deleted or context-less object would be a fatal error, so it is checked
    /// first.
    func write(_ message: Message) {
        guard !message.isDeleted, message.modelContext != nil else { return }
        guard !steps.isEmpty else { return }
        message.steps = steps
    }

    /// Clears for a new turn.
    func reset() {
        steps = []
        isOngoing = true
    }

    // MARK: - Internal

    /// Closes the last step with now, if it is open. The duration cannot be
    /// negative (`TimelineStep.duration` clamps to zero), but the end is clamped
    /// here too so it is never written behind the start.
    private func closeLast() {
        guard let last = steps.last, last.end == nil else { return }
        steps[steps.count - 1].end = max(Date(), last.start)
    }

    /// Row text of the interruption step.
    static var stoppedPartwayText: String { String(localized: "stopped partway") }
}
