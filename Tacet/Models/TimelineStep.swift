//
//  TimelineStep.swift
//  Tacet
//
//  The deterministic event sequence of a reply turn (timeline-spec §5.1).
//  NOT a SwiftData @Model — a Codable struct like `ToolTrace`; it is persisted
//  as JSON embedded in the message.
//
//  Single-source-of-truth rule: on a tool step `text` is EMPTY, the row text is
//  read from the `ToolTrace.text` bound through `toolTraceID`. Keeping the same
//  text in two places would break the principle that the chip is produced by the
//  tool.
//

import Foundation

/// The kind of the step. Its sources are all deterministic points that already
/// exist (timeline-spec §5.2 table); there is no invented state verb.
enum TimelineKind: String, Codable, CaseIterable, Hashable {
    /// The router picked an intent profile.
    case routing
    /// A skill / memory was attached to the prompt.
    case enrichment
    /// A tool ran — the text is read from `ToolTrace`.
    case tool
    /// The first chunk arrived from the reply stream.
    case writing
    /// Cancellation or scene interruption; the turn was left halfway.
    case interruption
}

/// A single row of the timeline ribbon. Steps are sequential: opening one closes
/// the previous one.
struct TimelineStep: Identifiable, Codable, Hashable {
    var id: UUID = UUID()
    var kind: TimelineKind
    /// Row text ("routed · calendar profile").
    /// EMPTY when `kind == .tool` — the text is read from `ToolTrace`.
    var text: String
    /// The matching `ToolTrace.id` when `kind == .tool`.
    var toolTraceID: UUID?
    var start: Date
    /// `nil` = still running or left halfway.
    var end: Date?

    init(id: UUID = UUID(),
         kind: TimelineKind,
         text: String = "",
         toolTraceID: UUID? = nil,
         start: Date = Date(),
         end: Date? = nil) {
        self.id = id
        self.kind = kind
        self.text = kind == .tool ? "" : text
        self.toolTraceID = toolTraceID
        self.start = start
        self.end = end
    }

    /// Did the step close.
    var isDone: Bool { end != nil }

    /// Duration in seconds; `nil` while still running. Clamped to zero so that
    /// it cannot go negative if the clock is moved back (timeline-spec §6 test
    /// criterion).
    var duration: TimeInterval? {
        guard let end else { return nil }
        return max(0, end.timeIntervalSince(start))
    }
}
