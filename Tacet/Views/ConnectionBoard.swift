//
//  ConnectionBoard.swift
//  Tacet
//
//  The connections (MCP) board — mcp-connection-spec §4, §3.5.
//
//  The empty state states its promise WITH ITS SCOPE: until you connect, THIS
//  surface stays closed. Saying "Tacet does not go online" was wrong — web search
//  is an independent surface and goes out when enabled, even with no MCP connected
//  at all. The panel cannot make a global claim. A row shows: name, URL, tool
//  count, last used. Deleting is confirmed and reports its outcome.
//

import SwiftUI
import SwiftData

// MARK: - Service contract

/// "Probe the connection" and the lifecycle — implemented by `ConnectionService`.
/// The view layer never touches the network API, it only calls this (§2.1).
@MainActor
protocol ConnectionProbe: AnyObject {
    /// `initialize` + `tools/list` for a form that has not been saved yet.
    /// A mandatory step before adding (§3.1).
    func probe(name: String, rawURL: String, key: String?) async -> ConnectionProbeOutcome
    /// Re-probes an existing connection; refreshes the tool summaries (§5.3).
    func probe(_ connection: Connection) async -> ConnectionProbeOutcome
    /// Writes the key into the Keychain, makes the connection persistent.
    func add(name: String,
             rawURL: String,
             key: String?,
             deviceData: DeviceDataSetting,
             tools: [ToolSummary]) throws -> Connection
    /// Called BEFORE the connection is deleted: removes the Keychain record.
    func removeKey(_ connection: Connection)
}

/// The probe outcome. On failure the cause is written in plain language, never
/// swallowed silently.
enum ConnectionProbeOutcome {
    /// The tools the server returned — unsupported ones are in the list too.
    case succeeded([ToolSummary])
    /// The plain-language counterpart of a cause such as timeout / auth / TLS.
    case failed(String)
}

// MARK: - Board

struct ConnectionBoard: View {
    let service: any ConnectionProbe

    @Query(sort: \Connection.createdAt, order: .reverse) private var connections: [Connection]
    @Environment(\.modelContext) private var record
    @Environment(\.dismiss) private var close

    @State private var path: [Connection] = []
    @State private var setup = false
    @State private var warningText: String?

    var body: some View {
        NavigationStack(path: $path) {
            list
                .background(Palette.background)
                .navigationTitle("Connections")
                .navigationBarTitleDisplayMode(.inline)
                .navigationDestination(for: Connection.self) { connection in
                    ConnectionDetail(connection: connection,
                                     service: service,
                                     deleteRequest: { sendToDelete(connection) })
                }
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) {
                        Button("Close") { close() }
                            .font(Typography.chip())
                            .foregroundStyle(Palette.grey)
                    }
                }
                .sheet(isPresented: $setup) {
                    NewConnection(service: service)
                }
                .issueBanner($warningText)
        }
    }

    // MARK: - List

    private var list: some View {
        List {
            Section {
                if connections.isEmpty {
                    emptyState
                } else {
                    Text("Tacet only reaches the servers you connect here.")
                        .font(Typography.chip())
                        .foregroundStyle(Palette.grey)
                        .padding(.bottom, Spacing.s1)
                        .connectionRow()
                }
            }

            ForEach(connections) { connection in
                NavigationLink(value: connection) {
                    row(connection)
                }
                .connectionRow()
            }

            Section {
                addServerRow
                    .connectionRow()
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
    }

    private func row(_ connection: Connection) -> some View {
        VStack(alignment: .leading, spacing: Spacing.s1) {
            Text(connection.name)
                .font(Typography.user())
                .foregroundStyle(Palette.ink)
                .lineLimit(1)
            Text(connection.rawURL)
                .font(Typography.chip())
                .foregroundStyle(Palette.grey)
                .lineLimit(1)
                .truncationMode(.middle)
            Text(lowerLine(connection))
                .font(Typography.chip())
                .foregroundStyle(Palette.muted)
                .lineLimit(1)
        }
        .padding(.vertical, Spacing.s3)
        .padding(.horizontal, Spacing.s4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(
            RoundedRectangle(cornerRadius: Spacing.s4, style: .continuous)
                .stroke(Palette.divider, lineWidth: Spacing.hairline)
        )
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(verbatim: "\(connection.name). \(connection.rawURL)"))
        .accessibilityValue(Text(verbatim: lowerLine(connection)))
    }

    /// "4 tools · last used today 09.12" — if it was never used, it says so.
    private func lowerLine(_ connection: Connection) -> String {
        let count = connection.availableTools.count
        let tools = String(localized: "\(count) tools")
        guard let last = connection.lastUsed else {
            return tools + " · " + String(localized: "not used yet")
        }
        return tools + " · " + String(localized: "last used \(LastUsedFormat.dateTime(last))")
    }

    private var addServerRow: some View {
        Button { setup = true } label: {
            HStack(spacing: Spacing.s2) {
                Image(systemName: "plus")
                    .accessibilityHidden(true)
                Text("Add server")
                Spacer(minLength: 0)
            }
            .font(Typography.user())
            .foregroundStyle(Palette.grey)
            .padding(.vertical, Spacing.s3)
            .padding(.horizontal, Spacing.s4)
            .overlay(
                RoundedRectangle(cornerRadius: Spacing.s4, style: .continuous)
                    .stroke(Palette.divider, style: StrokeStyle(lineWidth: Spacing.hairline, dash: [4, 4]))
            )
        }
        .buttonStyle(.plain)
        .accessibilityHint(Text("Opens a form to add your own MCP server."))
    }

    // The text is written verbatim in §4.
    private var emptyState: some View {
        VStack(alignment: .leading, spacing: Spacing.s2) {
            Text("No servers connected.")
                .font(Typography.tacet())
                .foregroundStyle(Palette.ink)
            Text("Connect your own MCP server and Tacet can use its tools. Until you do, this surface stays closed.")
                .font(Typography.chip())
                .foregroundStyle(Palette.grey)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.vertical, Spacing.s4)
        .connectionRow()
    }

    // MARK: - Deleting

    /// A delete request coming from the detail view: pop the stack first, then delete.
    /// A deleted model is not read during the pop animation.
    private func sendToDelete(_ connection: Connection) {
        path.removeAll()
        Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(400))
            delete(connection)
        }
    }

    private func delete(_ connection: Connection) {
        guard !connection.isDeleted else { return }
        // Touching a field of a deleted object is fatal: the name and the key cleanup
        // are handled BEFORE the delete.
        let name = connection.name
        service.removeKey(connection)
        record.delete(connection)
        do {
            try record.save()
        } catch {
            record.rollback()
            warningText = String(localized: "Couldn’t delete \(name): \(error.localizedDescription)")
        }
    }
}

/// The format of the "last used" line under a row.
///
/// `FormatStyle` instead of `DateFormatter`: no expensive object is created per row,
/// and the 12/24-hour preference that a fixed "HH.mm" pattern ignored is honoured.
private enum LastUsedFormat {
    /// "today 02:41" / "yesterday 08:12" / "18 Jul 02:15" (time format per device preference)
    static func dateTime(_ d: Date) -> String {
        let calendar = Calendar.current
        let t = d.formatted(.dateTime.hour().minute())
        if calendar.isDateInToday(d) { return String(localized: "today \(t)") }
        if calendar.isDateInYesterday(d) { return String(localized: "yesterday \(t)") }
        return "\(d.formatted(.dateTime.day().month(.abbreviated))) \(t)"
    }
}

private extension View {
    /// The shared background of every board row: no system separators, no system insets.
    func connectionRow() -> some View {
        listRowBackground(Palette.background)
            .listRowSeparator(.hidden)
            .listRowInsets(EdgeInsets(top: Spacing.s1, leading: Spacing.s5,
                                      bottom: Spacing.s1, trailing: Spacing.s5))
    }
}
