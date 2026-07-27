//
//  ConnectionDetail.swift
//  Tacet
//
//  The detail of a single connection — mcp-connection-spec §3.5.
//
//  The tool list (with the unsupported ones tagged), the device-data setting,
//  "Test connection", delete. Deleting is confirmed and reports its outcome. The board
//  does the deleting: here it is only requested, so that a deleted model is never read.
//

import SwiftUI
import SwiftData

struct ConnectionDetail: View {
    let connection: Connection
    let service: any ConnectionProbe
    let deleteRequest: () -> Void

    @Environment(\.modelContext) private var record

    @State private var probing = false
    @State private var probeNote: String?
    @State private var deleteQuestion = false
    @State private var warningText: String?

    /// Is the Keychain record behind `keyRef` really there. Read ONCE, when the view
    /// appears — `body` runs on every state change and the record does not move while
    /// this screen is up, so a query per redraw would buy nothing.
    ///
    /// nil = not looked up yet. The line stays SILENT in that moment instead of guessing:
    /// the wrong guess ("the key is missing") is an accusation the user would act on.
    @State private var keyStored: Bool?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Spacing.s5) {
                header
                deviceDataSection
                toolSection
                probeSection
            }
            .padding(.horizontal, Spacing.s5)
            .padding(.vertical, Spacing.s4)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(Palette.background)
        .task { keyStored = connection.keyRef.map { Keychain.exists(ref: $0) } }
        .navigationTitle("Connections")
        .navigationBarTitleDisplayMode(.inline)
        .safeAreaInset(edge: .bottom) { deleteRow }
        .confirmationDialog("Delete this connection?", isPresented: $deleteQuestion, titleVisibility: .visible) {
            Button("Delete", role: .destructive) { deleteRequest() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("\(connection.name) will be deleted. Its key is removed from the Keychain; traces in past conversations are kept.")
        }
        .issueBanner($warningText)
    }

    // MARK: - Sections

    private var header: some View {
        VStack(alignment: .leading, spacing: Spacing.s2) {
            // The name and the address are ONE element; the key line is deliberately
            // outside it. An explicit `accessibilityLabel` replaces everything the
            // combined children say, so a key line folded in here would be mute — and
            // what it reports can be a fault the user has to act on. Split out, it keeps
            // its own voice, and the nested stack carries the same spacing so the screen
            // does not move.
            VStack(alignment: .leading, spacing: Spacing.s2) {
                Text(connection.name)
                    .font(Typography.brand())
                    .foregroundStyle(Palette.ink)
                Text(connection.rawURL)
                    .font(Typography.chip())
                    .foregroundStyle(Palette.grey)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .accessibilityElement(children: .combine)
            .accessibilityLabel(Text(verbatim: "\(connection.name). \(connection.rawURL)"))
            .accessibilityAddTraits(.isHeader)

            keyLine
        }
    }

    /// What the header says about the access key. Silent when there is no key at all, and
    /// silent until the lookup has answered.
    ///
    /// THE REFERENCE ALONE IS NOT AN ANSWER: the record lives in the Keychain, the
    /// reference in SwiftData, and the two can come apart — a store restored onto a device
    /// whose Keychain was wiped is the plain case. Reporting "stored" off the reference
    /// would be the app vouching for something it never looked at, while every call
    /// quietly comes back 401. `Keychain.exists` looks, without ever seeing the value.
    @ViewBuilder
    private var keyLine: some View {
        if connection.keyRef != nil, let keyStored {
            if keyStored {
                Text("The access key is stored in the Keychain.")
                    .font(Typography.chip())
                    .foregroundStyle(Palette.muted)
            } else {
                Text("The access key is not in this device’s Keychain. The server will refuse every call until you delete this connection and add it again with the key.")
                    .font(Typography.chip())
                    .foregroundStyle(Palette.error)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private var deviceDataSection: some View {
        section("DEVICE DATA") {
            DeviceDataPicker(
                selection: Binding(
                    get: { connection.isDeleted ? .never : connection.deviceData },
                    set: { configure($0) }
                ),
                closed: connection.isDeleted
            )
        }
    }

    private var toolSection: some View {
        section("TOOLS") {
            VStack(alignment: .leading, spacing: Spacing.s3) {
                if connection.toolSummaries.isEmpty {
                    Text("No tools were received from this server.")
                        .font(Typography.chip())
                        .foregroundStyle(Palette.grey)
                } else {
                    ForEach(connection.toolSummaries) { tool in
                        ToolSummaryRow(tool: tool)
                    }
                }
            }
        }
    }

    private var probeSection: some View {
        VStack(alignment: .leading, spacing: Spacing.s3) {
            Button { probe() } label: {
                HStack(spacing: Spacing.s2) {
                    if probing {
                        ProgressView()
                            .controlSize(.small)
                            .tint(Palette.grey)
                    }
                    Text(probing ? "Testing…" : "Test connection")
                    Spacer(minLength: 0)
                }
                .font(Typography.user())
                .foregroundStyle(probing ? Palette.muted : Palette.ink)
                .padding(.vertical, Spacing.s3)
                .padding(.horizontal, Spacing.s4)
                .frame(minHeight: Spacing.touchTarget)
                .overlay(
                    RoundedRectangle(cornerRadius: Spacing.s4, style: .continuous)
                        .stroke(Palette.divider, lineWidth: Spacing.hairline)
                )
                .contentShape(RoundedRectangle(cornerRadius: Spacing.s4, style: .continuous))
            }
            .buttonStyle(.plain)
            .disabled(probing || connection.isDeleted)
            .accessibilityHint(Text("Connects to the server and refreshes its tool list."))

            if let probeNote {
                Text(probeNote)
                    .font(Typography.chip())
                    .foregroundStyle(Palette.grey)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityAddTraits(.isSummaryElement)
            }
        }
    }

    private var deleteRow: some View {
        Button("Delete connection") { deleteQuestion = true }
            .font(Typography.chip())
            .foregroundStyle(Palette.grey)
            .accessibilityHint(Text("You’ll be asked to confirm. The key is removed from the Keychain."))
            .padding(.horizontal, Spacing.s5)
            .padding(.top, Spacing.s3)
            .padding(.bottom, Spacing.s2)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Palette.background)
    }

    private func section<Content: View>(_ title: LocalizedStringKey,
                                        @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: Spacing.s3) {
            Text(title)
                .font(Typography.tag())
                .tracking(1.2)
                .foregroundStyle(Palette.muted)
                .accessibilityAddTraits(.isHeader)
            content()
        }
    }

    // MARK: - Actions

    private func configure(_ new: DeviceDataSetting) {
        guard !connection.isDeleted else { return }
        let old = connection.deviceData
        connection.deviceData = new
        do {
            try record.save()
        } catch {
            // If it could not be written to disk, the setting would look changed on screen
            // and revert when the app closed; we roll it back and say so.
            connection.deviceData = old
            record.rollback()
            warningText = String(localized: "Couldn’t save the setting: \(error.localizedDescription)")
        }
    }

    private func probe() {
        probing = true
        probeNote = nil
        Task { @MainActor in
            let outcome = await service.probe(connection)
            probing = false
            // Work that does not depend on the view's lifetime: do not write if the model
            // was deleted.
            guard !connection.isDeleted else { return }
            switch outcome {
            case .succeeded(let list):
                let working = list.filter { !$0.isUnsupported }.count
                probeNote = String(localized: "Connected. \(working) tools available.")
            case .failed(let cause):
                probeNote = cause
            }
        }
    }
}
