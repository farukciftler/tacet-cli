import SwiftUI

// The top bar: history + mark + serif "Tacet" on the left, new chat on the right.
// Brand identity: state is told only with the mark and with words, not with colour (a
// green dot). There is no device indicator; the privacy disclosure opens on tapping the
// brand.
struct TopBar: View {
    let state: ModelService.State
    /// The full sentence of the state — ModelService.blockMessage. It is passed in from
    /// outside so that the top bar and the message that lands in the chat come from ONE
    /// source; if TopBar embedded its own string ("in a moment" ↔ "a few minutes") the
    /// two texts would contradict each other.
    var description: String? = nil
    var openHistory: (() -> Void)? = nil
    var newChat: (() -> Void)? = nil
    @State private var sheetOpen = false

    // The touch target and the brand mark sit next to text — they grow with Dynamic Type.
    @ScaledMetric(relativeTo: .body) private var touchTarget: CGFloat = Spacing.touchTarget
    @ScaledMetric(relativeTo: .title3) private var brandSize: CGFloat = 20

    var body: some View {
        VStack(spacing: 0) {
            // The spacing dropped from s3 to s1: the icons now sit inside a 44pt touch
            // box, and the box's own inner padding takes the place of the old spacing.
            HStack(spacing: Spacing.s1) {
                if let openHistory {
                    Button(action: openHistory) {
                        Image(systemName: "list.bullet")
                            .font(Typography.iconLarge())
                            .foregroundStyle(Palette.grey)
                            .frame(minWidth: touchTarget, minHeight: touchTarget)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(Text("Chat history"))
                }

                Button {
                    sheetOpen = true
                } label: {
                    HStack(spacing: Spacing.s2) {
                        TacetMark(size: brandSize)
                        Text(verbatim: "Tacet")
                            .font(Typography.brand())
                            .foregroundStyle(Palette.ink)
                        // The only hint that it is tappable: a quiet mark, not a badge.
                        Image(systemName: "chevron.down")
                            .font(Typography.tag())
                            .foregroundStyle(Palette.muted)
                    }
                    .frame(minHeight: touchTarget)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(Text("About Tacet"))
                .accessibilityHint(Text("Opens the explanation of on-device operation"))

                Spacer()

                if let newChat {
                    Button(action: newChat) {
                        Image(systemName: "square.and.pencil")
                            .font(Typography.iconLarge())
                            .foregroundStyle(Palette.grey)
                            .frame(minWidth: touchTarget, minHeight: touchTarget)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(Text("New chat"))
                }
            }
            // Because the 44pt box pulls the icon ~11pt inwards, the margin was reduced
            // from s5 to s3: the icon's VISUAL alignment stays the same as the content
            // below it (s5).
            .padding(.horizontal, Spacing.s3)
            .padding(.vertical, Spacing.s1)

            stateRow

            Rectangle()
                .fill(Palette.divider)
                .frame(height: Spacing.hairline)
        }
        .background(Palette.background)
        .sheet(isPresented: $sheetOpen) {
            PrivacyDisclosure()
        }
    }

    // If the model is not ready the user must learn it BEFORE typing.
    // Brand: no coloured dot / badge — the state is told only in words and with a quiet
    // mark. In the ready state nothing is shown.
    @ViewBuilder
    private var stateRow: some View {
        switch state {
        case .ready:
            EmptyView()
        case .preparing:
            stateText("ellipsis", Text(verbatim: description ?? L10n.modelPreparing))
        case .unavailable:
            stateText("exclamationmark.circle",
                      Text(verbatim: description ?? L10n.deviceNotEligible))
        }
    }

    private func stateText(_ glyph: String, _ text: Text) -> some View {
        HStack(spacing: Spacing.s2) {
            Image(systemName: glyph)
                .font(Typography.chip())
                .foregroundStyle(Palette.muted)
                .accessibilityHidden(true)
            text
                .font(Typography.chip())
                .foregroundStyle(Palette.grey)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, Spacing.s5)
        .padding(.bottom, Spacing.s3)
        .accessibilityElement(children: .combine)
    }
}

// The plain explanation of on-device operation (on tapping the brand).
private struct PrivacyDisclosure: View {
    @Environment(\.dismiss) private var close

    @ScaledMetric(relativeTo: .body) private var touchTarget: CGFloat = Spacing.touchTarget
    @ScaledMetric(relativeTo: .title3) private var brandSize: CGFloat = 20

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.s4) {
            HStack {
                HStack(spacing: Spacing.s2) {
                    TacetMark(size: brandSize)
                    Text(verbatim: "Tacet").font(Typography.brand()).foregroundStyle(Palette.ink)
                }
                Spacer()
                // At chip point size "Close" stayed about 14pt tall; the visual size is
                // the same, the touch area was opened up to 44.
                Button { close() } label: {
                    Text("Close")
                        .frame(minWidth: touchTarget, minHeight: touchTarget,
                               alignment: .trailing)
                        .contentShape(Rectangle())
                }
                .font(Typography.chip())
                .foregroundStyle(Palette.grey)
                .buttonStyle(.plain)
            }

            // The EXACT same sentence as in the welcome screen and the empty state: one
            // key, one translation. Keeping twin keys that differ by two punctuation
            // marks was splitting the translation in two.
            Text("Tacet's core runs on this device. Web search and connections engage only if you turn them on, and only in plain sight.")
                .font(Typography.tacet())
                .foregroundStyle(Palette.grey)
                .fixedSize(horizontal: false, vertical: true)

            Spacer()
        }
        .padding(Spacing.s5)
        .background(Palette.background)
        .presentationDetents([.medium])
    }
}

#Preview {
    VStack(spacing: 0) {
        TopBar(state: .ready, openHistory: {}, newChat: {})
        TopBar(state: .preparing, description: L10n.modelPreparing, openHistory: {}, newChat: {})
        TopBar(state: .unavailable(""), description: L10n.appleIntelligenceClosed,
               openHistory: {}, newChat: {})
        Spacer()
    }
}
