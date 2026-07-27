//
//  ApprovalSheet.swift
//  Tacet
//
//  Sharing approval (mcp §3.3, §4). In a dirty session, before any data leaves the
//  device, the user sees THE EXACT CONTENT THAT WILL BE SENT and decides.
//  Tone: no drama, no scare tactics; it shows what will go and asks.
//

import SwiftUI

struct ApprovalSheet: View {
    /// The connection/server name — "home server".
    let source: String
    let toolName: String
    /// Exactly the arguments that will be sent; not a category summary.
    let content: String
    /// true = send, false = don't send. Closing also produces false.
    let decision: (Bool) -> Void

    @Environment(\.dismiss) private var close

    /// Whatever the dismissal path, the decision is reported exactly once: swiping to
    /// dismiss is also a "Don't send", but if a button was pressed it is not reported a
    /// second time.
    @State private var decided = false

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: Spacing.s4) {
                    Text("\(source) · \(toolName)")
                        .font(Typography.user())
                        .foregroundStyle(Palette.ink)
                        .fixedSize(horizontal: false, vertical: true)

                    VStack(alignment: .leading, spacing: Spacing.s2) {
                        Text("GOING TO YOUR SERVER:")
                            .font(Typography.tag())
                            .tracking(0.6)
                            .foregroundStyle(Palette.muted)

                        Text(content)
                            .font(.system(.footnote, design: .monospaced))
                            .foregroundStyle(Palette.ink)
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(Spacing.s3)
                            .background(Palette.fill)
                            .clipShape(RoundedRectangle(cornerRadius: Spacing.chipCorner,
                                                        style: .continuous))
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel(Text("Content that will go to the server"))
                    .accessibilityValue(Text(content))

                    Text("If you don’t send it, Tacet skips this step and tells you it couldn’t do it.")
                        .font(Typography.chip())
                        .foregroundStyle(Palette.grey)
                        .fixedSize(horizontal: false, vertical: true)

                    buttons
                }
                .padding(.horizontal, Spacing.s5)
                .padding(.vertical, Spacing.s4)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .background(Palette.background)
            .navigationTitle(Text("Send this"))
            .navigationBarTitleDisplayMode(.inline)
        }
        .presentationDetents([.medium, .large])
        .interactiveDismissDisabled(false)
        .onDisappear {
            // Closing = "Don't send".
            notify(false)
        }
    }

    private var buttons: some View {
        HStack(spacing: Spacing.s3) {
            button(title: Text("Don’t send"), emphasised: false) { notify(false) }
            button(title: Text("Send"), emphasised: true) { notify(true) }
        }
        .padding(.top, Spacing.s2)
    }

    // No accent colour: the primary action is distinguished by its fill, the secondary by
    // a hairline frame.
    private func button(title: Text, emphasised: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            title
                .font(Typography.user())
                .foregroundStyle(emphasised ? Palette.background : Palette.ink)
                .frame(maxWidth: .infinity)
                .frame(minHeight: Spacing.touchTarget)
                .background {
                    if emphasised {
                        RoundedRectangle(cornerRadius: Spacing.chipCorner, style: .continuous)
                            .fill(Palette.ink)
                    } else {
                        RoundedRectangle(cornerRadius: Spacing.chipCorner, style: .continuous)
                            .stroke(Palette.divider, lineWidth: Spacing.hairline)
                    }
                }
                .contentShape(RoundedRectangle(cornerRadius: Spacing.chipCorner, style: .continuous))
        }
        .buttonStyle(.plain)
    }

    private func notify(_ accept: Bool) {
        guard !decided else { return }
        decided = true
        decision(accept)
        close()
    }
}

#Preview {
    Color.clear.sheet(isPresented: .constant(true)) {
        ApprovalSheet(
            source: "home server",
            toolName: "open_issue",
            content: "{\n  \"title\": \"Dentist appointment\",\n  \"time\": \"10:00\"\n}",
            decision: { _ in }
        )
    }
}
