//
//  ToolTrace.swift
//  Tacet
//
//  Tool chip — the signature of the system (spec §4.4, §7.4).
//  The chip's single source of truth is the tool itself; the text is produced
//  by the tool, it is not dictated by the model. The model cannot hallucinate
//  chip text.
//

import Foundation

/// State of the tool chip — spec §4.4 table.
enum ToolState: Equatable, Codable, Hashable {
    case running            // grey, spinner — "Checking calendar…"
    case readOk             // grey — information was read
    case written            // check mark — something changed in the world
    case permissionRequired // grey, tappable — "Calendar permission needed"
    case failed(String)     // error — short reason
    /// Data will leave the device in a tainted session; waiting on the user's
    /// decision (mcp §3.3). Grey, tappable — a tap opens the approval sheet.
    case awaitingApproval
    /// The user said "Don't send". Grey, NOT struck through, not dramatised;
    /// a refusal is not an error, it is a constraint (mcp §2.4/3).
    case notSent
}

/// The trace a tool call leaves in the stream. Identifiable — the chip is listed.
/// Codable — persisted together with the SwiftData message.
struct ToolTrace: Identifiable, Codable, Hashable {
    var id: UUID = UUID()
    /// SF Symbol name (outline). The colour is always the colour of the text above it.
    var icon: String
    /// Chip text: at most ~5 words + an optional `· detail`. The tool produces it.
    var text: String
    var state: ToolState
    /// Raw input/output for the detail view opened by tapping the chip
    /// (second layer of transparency).
    var rawInput: String?
    var rawOutput: String?
    /// If this chip produced a file, the path of that file — tapping the chip
    /// opens a QuickLook preview.
    var filePath: String?

    init(id: UUID = UUID(),
         icon: String,
         text: String,
         state: ToolState = .running,
         rawInput: String? = nil,
         rawOutput: String? = nil,
         filePath: String? = nil) {
        self.id = id
        self.icon = icon
        self.text = text
        self.state = state
        self.rawInput = rawInput
        self.rawOutput = rawOutput
        self.filePath = filePath
    }

    /// Natural sentence for VoiceOver — spec §7.6 ("Tacet read the calendar, tomorrow").
    var spokenLabel: String {
        switch state {
        case .running:        return "Tacet \(text)"
        case .readOk, .written: return "Tacet: \(text)"
        case .permissionRequired: return "\(text). Tap to grant permission."
        case .failed(let n):  return "Failed: \(n)"
        case .awaitingApproval: return "\(text). Tap to see what was sent."
        case .notSent:        return "\(text). Not sent."
        }
    }
}
