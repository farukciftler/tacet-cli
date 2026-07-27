//
//  TacetTool.swift
//  Tacet
//
//  The tool catalog contract (spec §7.3). Every tool is defined through the
//  FoundationModels `Tool` protocol, with its arguments typed via @Generable/@Guide.
//  The model never produces free text that we then parse. No tool makes a network call.
//
//  Every tool carries a `ToolReporter` and updates its chip while running and when done.
//  The chip text is produced inside the tool — the model cannot hallucinate chip text (spec §7.4).
//

import Foundation
import FoundationModels

/// A Tacet tool: FoundationModels Tool + chip reporting.
protocol TacetTool: Tool {
    /// The executor the chips are dropped into. Weak reference — no retain cycle.
    var reporter: ToolReporter? { get }
}

/// Which ENGLISH code should this tool's own error type send to the model?
///
/// `errorDescription` goes to the user (into the chip, in their language); this code goes
/// to the model. The two channels are separate: the model context is English, and a
/// localized sentence MUST NOT LEAK into it. Conforming is optional — an error that does
/// not conform returns `unknown_error` and behaves exactly as before (backward-compatible addition).
protocol ToolErrorCode {
    var errorCode: String { get }
}

extension TacetTool {
    /// The progress-reporting variant of `runWithChip`: it changes the chip text while the
    /// work is still running. For long-running tools (web search while it pulls pages).
    ///
    /// Consistent with the timeline principle: every step shown is an event that ACTUALLY
    /// happened in the code — which site was visited is not made up, it is the address being
    /// downloaded at that moment. No dramatization: not a fancy verb like "researching",
    /// but the domain name itself.
    func runWithChip(
        icon: String,
        runningText: String,
        rawInput: String? = nil,
        withProgress operation: (@Sendable (String) async -> Void) async throws -> ToolOutcome
    ) async -> String {
        let id = reporter?.start(icon: icon, text: runningText)
        let reporter = self.reporter
        let advance: @Sendable (String) async -> Void = { text in
            guard let id else { return }
            await reporter?.update(id, state: .running, text: text,
                                   rawInput: nil, rawOutput: nil, filePath: nil)
        }
        do {
            let s = try await operation(advance)
            if let id {
                reporter?.update(id, state: s.state, text: s.chipText,
                                 rawInput: rawInput, rawOutput: s.rawOutput,
                                 filePath: s.filePath)
            }
            return s.toModel
        } catch {
            let cause = Self.shortError(error)
            if let id {
                reporter?.update(id, state: .failed(cause), text: nil,
                                 rawInput: rawInput, rawOutput: cause, filePath: nil)
            }
            return Self.errorToModel(error)
        }
    }

    /// Wraps a tool's work in the chip lifecycle: start → work → final state.
    /// - `icon`: SF Symbol. `runningText`: the text next to the spinner.
    /// - `job`: does the real work; returns the final chip text to show on screen, the state
    ///   and an optional raw output; also returns the String that goes back to the model.
    func runWithChip(
        icon: String,
        runningText: String,
        rawInput: String? = nil,
        job operation: () async throws -> ToolOutcome
    ) async -> String {
        let id = reporter?.start(icon: icon, text: runningText)
        do {
            let s = try await operation()
            if let id {
                reporter?.update(id, state: s.state, text: s.chipText,
                                 rawInput: rawInput, rawOutput: s.rawOutput,
                                 filePath: s.filePath)
            }
            return s.toModel
        } catch {
            let cause = Self.shortError(error)
            if let id {
                reporter?.update(id, state: .failed(cause), text: nil,
                                 rawInput: rawInput, rawOutput: cause, filePath: nil)
            }
            // The error text is returned as a String so the model's flow is not cut off.
            // The text that goes to the model is ENGLISH AND FROM A FIXED VOCABULARY: even
            // if the model echoes it verbatim into its answer, neither a localized sentence
            // leaks nor does a raw error code (`EKErrorDomain error 1.`) become visible.
            return Self.errorToModel(error)
        }
    }

    /// The error text that goes to the model. The `tool_failed:` prefix is KEPT (backward
    /// compatibility), with the fixed English code for the cause appended after it.
    ///
    /// MEASURED FAILURE: every error used to return the same flat "tool_failed". The model
    /// could not tell "disk is full" apart from "the calendar did not save", could not tell
    /// the user why, and retried the SAME call verbatim. The chip already showed the
    /// localized cause; what was missing was the model's channel.
    static func errorToModel(_ error: Error) -> String {
        "tool_failed: \(errorCode(error)); no result was produced"
    }

    /// The fixed-English-code counterpart of `shortError`'s domain mapping. The two split
    /// the SAME cases in the SAME order — an error whose chip reads "No space left on the
    /// device." goes to the model as `disk_full`; the two never contradict each other.
    static func errorCode(_ error: Error) -> String {
        // If the tool declares its own code, that one wins.
        if let coded = error as? ToolErrorCode { return coded.errorCode }
        let ns = error as NSError
        switch (ns.domain, ns.code) {
        case (NSCocoaErrorDomain, NSFileWriteOutOfSpaceError):
            return "disk_full"
        case (NSCocoaErrorDomain, NSFileNoSuchFileError),
             (NSCocoaErrorDomain, NSFileReadNoSuchFileError):
            return "file_not_found"
        case (NSCocoaErrorDomain, NSFileWriteNoPermissionError),
             (NSCocoaErrorDomain, NSFileReadNoPermissionError):
            return "file_permission_denied"
        case (NSCocoaErrorDomain, _), (NSPOSIXErrorDomain, _), (NSOSStatusErrorDomain, _):
            return "file_operation_failed"
        case ("EKErrorDomain", _):
            return "calendar_rejected_the_change"
        case ("CNErrorDomain", _):
            return "contacts_unavailable"
        case (NSURLErrorDomain, _):
            return "network_unavailable"
        default:
            return "unknown_error"
        }
    }

    /// The error text written into the chip — the user reads it, so it must be localized and
    /// understandable. A raw `NSError.localizedDescription` ("EKErrorDomain error 1.") never
    /// reaches the screen; recognized domains are turned into human sentences.
    static func shortError(_ error: Error) -> String {
        // Our own tool errors are already sentences coming from the String Catalog.
        if let localized = error as? LocalizedError,
           let text = localized.errorDescription, !text.isEmpty {
            return text
        }
        let ns = error as NSError
        switch (ns.domain, ns.code) {
        case (NSCocoaErrorDomain, NSFileWriteOutOfSpaceError):
            return String(localized: "There’s no space left on the device.")
        case (NSCocoaErrorDomain, NSFileNoSuchFileError),
             (NSCocoaErrorDomain, NSFileReadNoSuchFileError):
            return String(localized: "The file couldn’t be found.")
        case (NSCocoaErrorDomain, _), (NSPOSIXErrorDomain, _), (NSOSStatusErrorDomain, _):
            return String(localized: "The file operation couldn’t be completed.")
        case ("EKErrorDomain", _):
            return String(localized: "Calendar didn’t accept this operation.")
        case ("CNErrorDomain", _):
            return String(localized: "Contacts couldn’t be reached right now.")
        default:
            return String(localized: "This step couldn’t be completed.")
        }
    }
}

/// The outcome of a tool run: what gets written into the chip + what goes back to the model.
struct ToolOutcome {
    /// The final text shown in the chip (produced by the tool; ~5 words + an optional · detail).
    var chipText: String
    /// The chip's final state — reading (`.readOk`), writing with a checkmark (`.written`).
    var state: ToolState
    /// The text returned to the model — short/summary; raw bulk data is never dumped into the context (spec §7.2).
    var toModel: String
    /// Raw output for the chip's detail view (the second layer of transparency).
    var rawOutput: String?
    /// The path if a file was produced — tapping the chip opens the preview.
    var filePath: String?

    init(chipText: String, state: ToolState, toModel: String, rawOutput: String? = nil, filePath: String? = nil) {
        self.chipText = chipText
        self.state = state
        self.toModel = toModel
        self.rawOutput = rawOutput
        self.filePath = filePath
    }
}
