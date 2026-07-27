//
//  Connection.swift
//  Tacet
//
//  An MCP server the user added by hand (mcp-connection-spec §5.1).
//
//  THE TOKEN DOES NOT LIVE HERE. The bearer key is kept in the Keychain
//  (`kSecAttrAccessibleWhenUnlockedThisDeviceOnly`); the model only holds the
//  `keyRef` that finds that record. The SwiftData store is not encrypted and can
//  reach iCloud — a secret is not written there.
//
//  Raw tool descriptions do not fit in the 4096 window (100–500 tokens/tool);
//  at the moment of adding, the on-device model summarizes them and they are
//  cached in `toolSummaries` (spec §5.3). What enters the session is that summary.
//

import Foundation
import SwiftData

/// Can device data be sent to this connection (spec §3.1).
///
/// Spec v0.1 defined only two modes and deliberately left "always allow" out.
/// The third mode was added by a user decision; its counterpart was written into
/// spec §3.1 and into the decision record. The option that closes the gate is
/// tied to a warning modal that explains what is disabled while it is being
/// selected — it is NOT silently reachable.
enum DeviceDataSetting: String, Codable, CaseIterable {
    /// Default: in a tainted session the call is never made at all.
    case never
    /// In a tainted session every call lands on the approval sheet.
    case askEveryTime
    /// The approval gate is skipped — sent without asking, even in a tainted
    /// session.
    ///
    /// Spec §3.1 deliberately left this out of v1; it was brought back by a user
    /// decision and tied to a warning modal at selection time. When it is
    /// selected, two of the three defences in §5.8 fall away: the approval gate
    /// and showing the real content. What remains is only the profile split —
    /// personal-data tools still never enter the same profile as MCP.
    case always

    /// Is this the mode that skips the approval gate — handled separately on the
    /// call path and in the interface.
    var skipsGate: Bool { self == .always }
}

/// A single tool imported from the server: its name + its compressed description.
struct ToolSummary: Codable, Hashable, Identifiable {
    var id: String { name }
    /// Remote tool name (the name inside the MCP `tools/list`).
    var name: String
    /// Description reduced to 1–2 lines — this is the text that enters the session.
    var summary: String
    /// True if it was skipped because the schema could not be flattened; the
    /// detail view lists it as "unsupported", it is not swallowed silently
    /// (spec §5.2).
    var isUnsupported: Bool = false

    init(name: String, summary: String, isUnsupported: Bool = false) {
        self.name = name
        self.summary = summary
        self.isUnsupported = isUnsupported
    }
}

@Model
final class Connection {
    var id: UUID = UUID()
    /// Free text, e.g. "home server". The chip text starts with this name.
    var name: String = ""
    /// Streamable HTTP endpoint. Plain `http://` is accepted only on the local
    /// network (validation lives in the service layer).
    var rawURL: String = ""
    /// Raw `DeviceDataSetting` value. If an unknown value is read back, the most
    /// restrictive option (`never`) is applied.
    private var rawDeviceData: String = DeviceDataSetting.never.rawValue
    /// Keychain account key — NOT THE TOKEN ITSELF, only a reference. nil if no
    /// key was entered. When the connection is deleted this record is removed
    /// from the Keychain.
    var keyRef: String?
    /// Encoded [ToolSummary]. Plain Data — the `Message.tracesData` pattern,
    /// portable.
    private var toolSummariesData: Data?
    var createdAt: Date = Date()
    /// When a tool was last called; shown in the list. nil if never used.
    var lastUsed: Date?

    init(name: String = "",
         rawURL: String = "",
         deviceData: DeviceDataSetting = .never,
         keyRef: String? = nil,
         toolSummaries: [ToolSummary] = []) {
        self.id = UUID()
        self.name = name
        self.rawURL = rawURL
        self.rawDeviceData = deviceData.rawValue
        self.keyRef = keyRef
        self.toolSummariesData = try? JSONEncoder().encode(toolSummaries)
        self.createdAt = Date()
    }

    var deviceData: DeviceDataSetting {
        get { DeviceDataSetting(rawValue: rawDeviceData) ?? .never }
        set { rawDeviceData = newValue.rawValue }
    }

    /// Cached tool specs. Refreshed if the server list changes.
    var toolSummaries: [ToolSummary] {
        get {
            guard let toolSummariesData else { return [] }
            return (try? JSONDecoder().decode([ToolSummary].self, from: toolSummariesData)) ?? []
        }
        set { toolSummariesData = try? JSONEncoder().encode(newValue) }
    }

    /// The tools that can be offered to the model — the unflattenable ones are out.
    var availableTools: [ToolSummary] {
        toolSummaries.filter { !$0.isUnsupported }
    }

    var url: URL? { URL(string: rawURL) }

    /// Is it saveable — name non-empty and the URL a parseable http(s) address.
    var isValid: Bool {
        guard !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              let url, let scheme = url.scheme?.lowercased(),
              scheme == "https" || scheme == "http",
              url.host != nil else { return false }
        return true
    }
}
