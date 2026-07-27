//
//  Message.swift
//  Tacet
//
//  Chat history — kept on the device with SwiftData (spec §4.7, §7.5).
//  Persistent model records; live stream state lives in the view model.
//

import Foundation
import SwiftData

/// Who is speaking. Stored as a raw string (simple, for SwiftData enum support).
enum Role: String, Codable {
    /// The user. The raw value stays `"sen"` on purpose: it is PERSISTED DATA,
    /// not prose. Renaming it would push every stored user message through the
    /// `Role(rawValue:) ?? .tacet` fallback below and redraw it as the assistant.
    case user = "sen"
    // The raw value changed once at the brand switch: local records written
    // under the old name still resolve to the assistant role through the
    // `Role(rawValue:) ?? .tacet` fallback below — there is NO separate
    // migration code, because the app was never released.
    case tacet  // assistant
}

/// Persistent chat message. Tool traces are stored embedded in the message
/// (JSON) — chips are persistent alongside the reply and their single source of
/// truth is the tool layer.
@Model
final class Message {
    var id: UUID = UUID()
    private var rawRole: String = Role.tacet.rawValue
    var content: String = ""
    var createdAt: Date = Date()
    /// The chat this belongs to (spec §4.7). The inverse relationship is
    /// declared in Chat.messages.
    var chat: Chat?
    /// Is this message an error report? Error texts are marked so that they are
    /// visually separated from a real reply and "try again" can be offered.
    /// Carries a default value — lightweight-migration compatible.
    var isError: Bool = false
    /// Can the same prompt be sent again? For errors whose side effect already
    /// completed (written to the calendar, then failed) trying again creates a
    /// SECOND event; on a guardrail/language refusal a retry gives the same
    /// result anyway. Both are false.
    /// Carries a default value — lightweight-migration compatible.
    var isRetryable: Bool = true
    /// Encoded [ToolTrace]. Plain Data instead of a SwiftData transformable —
    /// portable.
    private var tracesData: Data?
    /// Encoded [TimelineStep] — exactly the tracesData pattern.
    /// Default nil: old messages return an empty list, no Timeline row is drawn,
    /// and there is NO backfill (timeline-spec §5.1).
    private var stepsData: Data?

    /// Lazy cache of the decoded JSON. `@Transient` — it does NOT add a NEW
    /// STORED FIELD to the schema, no migration needed.
    ///
    /// Why a reference box: if the cache were written into a FIELD of the model,
    /// that write (made from a getter during body evaluation) would both signal
    /// "changed" to SwiftData and carry the risk of a "state modified during
    /// view update" loop. The INSIDE of the box changes; the model's field stays
    /// put.
    @Transient private var cache = ResolutionCache()

    var role: Role {
        get { Role(rawValue: rawRole) ?? .tacet }
        set { rawRole = newValue.rawValue }
    }

    /// The tool chips that land right above this reply (spec §4.4).
    ///
    /// Does not run JSONDecoder on every access: the result is stored together
    /// with the raw Data it was decoded from. If the context is refreshed from
    /// outside and `tracesData` changes, the source comparison no longer holds
    /// and the cache drops by itself.
    var traces: [ToolTrace] {
        get {
            let data = tracesData
            if let cached = cache.traces, cache.tracesSource == data { return cached }
            let resolved = data.flatMap {
                try? JSONDecoder().decode([ToolTrace].self, from: $0)
            } ?? []
            cache.traces = resolved
            cache.tracesSource = data
            return resolved
        }
        set {
            let data = try? JSONEncoder().encode(newValue)
            tracesData = data
            cache.traces = newValue
            cache.tracesSource = data
        }
    }

    /// The timeline of this reply — the deterministic event sequence of the turn
    /// (timeline-spec §5.1). Caching follows the same pattern as `traces`.
    var steps: [TimelineStep] {
        get {
            let data = stepsData
            if let cached = cache.steps, cache.stepsSource == data { return cached }
            let resolved = data.flatMap {
                try? JSONDecoder().decode([TimelineStep].self, from: $0)
            } ?? []
            cache.steps = resolved
            cache.stepsSource = data
            return resolved
        }
        set {
            let data = try? JSONEncoder().encode(newValue)
            stepsData = data
            cache.steps = newValue
            cache.stepsSource = data
        }
    }

    init(role: Role, content: String, traces: [ToolTrace] = [],
         steps: [TimelineStep] = [],
         isError: Bool = false, isRetryable: Bool = true,
         createdAt: Date = Date()) {
        self.id = UUID()
        self.rawRole = role.rawValue
        self.content = content
        self.isError = isError
        self.isRetryable = isRetryable
        self.createdAt = createdAt
        self.tracesData = (try? JSONEncoder().encode(traces))
        // Left nil when empty: "no timeline" and "empty timeline" are the same
        // thing, and this follows the same path as old messages.
        self.stepsData = steps.isEmpty ? nil : (try? JSONEncoder().encode(steps))
    }
}

/// Carrier of the `Message.traces` / `Message.steps` decoding (see `Message.cache`).
/// A REFERENCE, not a value: because the message's field never changes, neither
/// SwiftData nor Observation is triggered. The source Data is stored too — that
/// is the only cheap way to verify the decoding is still valid.
final class ResolutionCache {
    var tracesSource: Data?
    var traces: [ToolTrace]?
    var stepsSource: Data?
    var steps: [TimelineStep]?
}
