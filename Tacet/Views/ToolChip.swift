import SwiftUI

// The tool chip — the system's signature (spec §4.4).
// Shows Tacet's touch on the world in a single line, as a calm chip.
struct ToolChip: View {
    let trace: ToolTrace
    /// The executor of the live turn — only for the decision of the "awaiting approval"
    /// chip. It is nil on past messages; there is no pending decision then either, and
    /// the chip opens the normal detail sheet.
    var executor: ToolExecutor? = nil

    // Is the detail sheet (raw input/output) open.
    @State private var detailOpen = false
    // Is the file preview (QuickLook) open.
    @State private var previewOpen = false
    // The "what should I do" sheet opened on a chip that needs permission.
    @State private var permissionOpen = false
    // Is the sharing approval sheet open (mcp §3.3).
    @State private var approvalOpen = false

    /// Is this chip the chip of the request currently awaiting a user decision. If the
    /// executor has no pending request (or it belongs to another chip) the approval sheet
    /// does not open — an old chip that has already been decided does not ask a second time.
    private var pendingApproval: ToolExecutor.ApprovalRequest? {
        guard trace.state == .awaitingApproval,
              let request = executor?.pendingApproval,
              request.traceID == trace.id else { return nil }
        return request
    }

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    // If this chip produced a previewable file, its URL.
    private var fileURL: URL? {
        guard let path = trace.filePath, FileManager.default.fileExists(atPath: path) else { return nil }
        return URL(fileURLWithPath: path)
    }

    var body: some View {
        Button {
            // If permission is needed the chip must not be a dead end: instead of the raw
            // input/output, the sheet offering the direct "go to iOS Settings" path opens.
            if trace.state == .permissionRequired {
                permissionOpen = true
            } else if pendingApproval != nil {
                // A chip awaiting a decision: a tap opens the approval sheet directly.
                approvalOpen = true
            } else if fileURL != nil {
                previewOpen = true
            } else {
                detailOpen = true
            }
        } label: {
            chipBody
        }
        .buttonStyle(.plain)
        .accessibilityLabel(trace.spokenLabel)
        .accessibilityHint(hint)
        .sheet(isPresented: $permissionOpen) {
            PermissionRedirect(title: trace.text)
        }
        .sheet(isPresented: $detailOpen) {
            ToolChipDetail(trace: trace)
        }
        .sheet(isPresented: $previewOpen) {
            if let url = fileURL {
                DocumentPreviewSheet(url: url)
            }
        }
        .sheet(isPresented: $approvalOpen) {
            if let request = pendingApproval {
                ApprovalSheet(source: request.source,
                              toolName: request.toolName,
                              content: request.content) { accept in
                    executor?.decideApproval(accept)
                }
            }
        }
        // If the request is resolved FROM OUTSIDE (the user hit "stop" → `stop()` →
        // `resolvePendingApproval()`) the sheet's content empties out, but because
        // `approvalOpen` was still true an EMPTY sheet stayed hanging on screen. The
        // dismissal signal has to come from the request itself.
        .onChange(of: pendingApproval == nil) { _, emptied in
            if emptied { approvalOpen = false }
        }
    }

    // The hint that says in advance what a tap will do.
    private var hint: Text {
        if trace.state == .permissionRequired {
            return Text("Tap to grant permission")
        }
        if pendingApproval != nil {
            return Text("Tap to see what was sent")
        }
        return fileURL != nil ? Text("Tap to preview the file") : Text("Tap for details")
    }

    // The chip itself: pill frame, icon + text, left aligned.
    private var chipBody: some View {
        HStack(spacing: Spacing.s2) {
            leadingElement
            Text(trace.text)
                // A write action is distinguished by WEIGHT, not colour (brand: colour
                // does not tell state).
                .font(trace.state == .written ? Typography.chip().weight(.medium) : Typography.chip())
                .foregroundStyle(color)
            // A small mark on the chip of a previewable file.
            if fileURL != nil {
                Image(systemName: "eye")
                    .font(Typography.tag())
                    .foregroundStyle(color)
            }
        }
        .padding(.horizontal, Spacing.s3)
        .padding(.vertical, Spacing.s2)
        .overlay(
            RoundedRectangle(cornerRadius: Spacing.chipCorner)
                .stroke(Palette.divider, lineWidth: Spacing.hairline)
        )
        .contentShape(RoundedRectangle(cornerRadius: Spacing.chipCorner))
    }

    // The leading element depending on the state: spinner, glyph or a checkmark.
    @ViewBuilder
    private var leadingElement: some View {
        switch trace.state {
        case .running:
            ProgressView()
                .controlSize(.small)
                .frame(width: 13, height: 13)
        case .readOk, .permissionRequired:
            glyph(trace.icon)
        case .written:
            glyph("checkmark")
        case .failed:
            glyph("exclamationmark.triangle")
        case .awaitingApproval:
            // The wait is honestly visible: not a spinner, but a hand waiting on the user.
            glyph("hand.raised")
        case .notSent:
            // A refusal is not an error but a constraint — no warning mark, no drama.
            glyph("nosign")
        }
    }

    // Icon: outline SF Symbol, the same colour as the text above it, the chip point-size
    // token (a fixed point size did not scale under Dynamic Type — the Theme.swift rule).
    private func glyph(_ name: String) -> some View {
        Image(systemName: name)
            .font(Typography.chip())
            .foregroundStyle(color)
    }

    // The colour of the text (and the icon). Brand: colour does not tell state — a write
    // is distinguished by ink + a checkmark, a failure by ink + a warning mark. No
    // green/red is used.
    private var color: Color {
        switch trace.state {
        case .running, .readOk, .permissionRequired:
            return Palette.grey
        // A chip awaiting approval and a chip that was not sent are grey: not struck
        // through, not red. It stands quietly and carries the user's decision.
        case .awaitingApproval, .notSent:
            return Palette.grey
        case .written, .failed:
            return Palette.ink
        }
    }
}

// Permission is needed: the only path is iOS Settings. The sheet is short, has one
// button and is honest; the app cannot grant the permission itself, the user decides.
private struct PermissionRedirect: View {
    let title: String
    @Environment(\.dismiss) private var close

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.s4) {
            HStack {
                Text("Permission needed")
                    .font(Typography.brand())
                    .foregroundStyle(Palette.ink)
                Spacer()
                Button { close() } label: { Text("Close") }
                    .font(Typography.chip())
                    .foregroundStyle(Palette.grey)
                    .buttonStyle(.plain)
            }

            Text(title)
                .font(Typography.user())
                .foregroundStyle(Palette.ink)
                .fixedSize(horizontal: false, vertical: true)

            Text("To continue this step, grant Tacet permission in iOS Settings. What is read does not go out without your approval.")
                .font(Typography.chip())
                .foregroundStyle(Palette.grey)
                .fixedSize(horizontal: false, vertical: true)

            Button {
                if let url = URL(string: UIApplication.openSettingsURLString) {
                    UIApplication.shared.open(url)
                }
            } label: {
                HStack(spacing: Spacing.s2) {
                    Text("Open iOS Settings")
                        .font(Typography.user())
                        .foregroundStyle(Palette.ink)
                    Image(systemName: "arrow.up.forward")
                        .font(Typography.chip())
                        .foregroundStyle(Palette.muted)
                        .accessibilityHidden(true)
                }
                .padding(.horizontal, Spacing.s4)
                .padding(.vertical, Spacing.s3)
                .overlay(
                    RoundedRectangle(cornerRadius: Spacing.chipCorner, style: .continuous)
                        .stroke(Palette.divider, lineWidth: Spacing.hairline)
                )
                .contentShape(RoundedRectangle(cornerRadius: Spacing.chipCorner, style: .continuous))
            }
            .buttonStyle(.plain)
            .accessibilityElement(children: .combine)

            Spacer()
        }
        .padding(Spacing.s5)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Palette.background)
        .presentationDetents([.medium])
    }
}

// Detail: raw input and output, the second layer of transparency.
// Not file-private, because the Timeline also opens the SAME sheet — a second surface
// is not written for the detail of a tool step (timeline-spec §2.4).
struct ToolChipDetail: View {
    let trace: ToolTrace
    @Environment(\.dismiss) private var close

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: Spacing.s4) {
                    section("Input", trace.rawInput)
                    section("Output", trace.rawOutput)
                }
                .padding(.horizontal, Spacing.s5)
                .padding(.vertical, Spacing.s4)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .background(Palette.background)
            .navigationTitle(trace.text)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Close") { close() }
                }
            }
        }
    }

    // A single raw block: title + monospace content.
    @ViewBuilder
    private func section(_ title: String, _ content: String?) -> some View {
        VStack(alignment: .leading, spacing: Spacing.s2) {
            Text(title)
                .font(Typography.tag())
                .foregroundStyle(Palette.muted)
            Text(content ?? "—")
                .font(.system(.footnote, design: .monospaced))
                .foregroundStyle(Palette.ink)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(Spacing.s3)
                .background(Palette.fill)
                .clipShape(RoundedRectangle(cornerRadius: Spacing.chipCorner))
        }
    }
}

#Preview {
    VStack(alignment: .leading, spacing: Spacing.chipReplyGap) {
        ToolChip(trace: ToolTrace(
            id: UUID(), icon: "magnifyingglass", text: "reading calendar",
            state: .running, rawInput: "today", rawOutput: nil))

        ToolChip(trace: ToolTrace(
            id: UUID(), icon: "calendar", text: "3 events read",
            state: .readOk, rawInput: "range: today",
            rawOutput: "09:00 meeting\n13:00 lunch\n18:00 gym"))

        ToolChip(trace: ToolTrace(
            id: UUID(), icon: "calendar.badge.plus", text: "event added",
            state: .written, rawInput: "title: Dentist\ndate: tomorrow 10:00",
            rawOutput: "id: E-4821"))

        ToolChip(trace: ToolTrace(
            id: UUID(), icon: "lock", text: "grant permission for location",
            state: .permissionRequired, rawInput: nil, rawOutput: nil))

        ToolChip(trace: ToolTrace(
            id: UUID(), icon: "exclamationmark.triangle", text: "no network",
            state: .failed("no connection"), rawInput: "request", rawOutput: nil))

        ToolChip(trace: ToolTrace(
            id: UUID(), icon: "hand.raised", text: "home server · awaiting approval",
            state: .awaitingApproval, rawInput: "query: tomorrow's meeting", rawOutput: nil))

        ToolChip(trace: ToolTrace(
            id: UUID(), icon: "hand.raised", text: "home server · not sent",
            state: .notSent, rawInput: "query: tomorrow's meeting", rawOutput: nil))
    }
    .padding(Spacing.s5)
    .frame(maxWidth: .infinity, alignment: .leading)
    .background(Palette.background)
}
