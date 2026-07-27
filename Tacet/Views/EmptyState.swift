import SwiftUI

// The first-launch screen. A calm sentence in the middle, a short description
// and example prompts that are written into the input field when tapped.
struct EmptyState: View {
    // When a chip is selected it writes the matching text into the input field.
    let exampleSelected: (String) -> Void

    /// The "what can it do" catalogue — opened from a quiet link, never forced.
    @State private var capabilitiesOpen = false
    @ScaledMetric(relativeTo: .body) private var touchTarget = Spacing.touchTarget

    /// The job picked in the welcome screen passes through here. The welcome sheet
    /// cannot reach the input field; we reuse the channel the empty state ALREADY uses
    /// (exampleSelected) — no second path is opened.
    private var bridge: WelcomeBridge { .shared }

    // Example prompts — localised into the user's language (String Catalog).
    // The text written into the input field on tap is in that language too → the model
    // answers in that language. The examples do not only say "you can ask a question";
    // the product's invisible capabilities (documents, reminders, note search, time) are
    // discovered from here as well.
    private let examples = [
        String(localized: "What's on tomorrow?"),
        String(localized: "Remind me to make a call at 6:00 PM"),
        String(localized: "Turn this week’s notes into a document"),
        String(localized: "Find last week's meeting note"),
        String(localized: "How many days until New Year?"),
    ]

    var body: some View {
        // There are more examples now and they may not fit on screen at large point
        // sizes; the scrolling safety net prevents clipping, and when it fits, the
        // scrolling is not felt.
        ScrollView {
            content
                .padding(.horizontal, Spacing.s5)
                .padding(.vertical, Spacing.s5)
                .frame(maxWidth: .infinity)
                .containerRelativeFrame(.vertical, alignment: .center)
        }
        .scrollBounceBehavior(.basedOnSize)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .sheet(isPresented: $capabilitiesOpen) {
            Capabilities(exampleSelected: { text in
                capabilitiesOpen = false
                exampleSelected(text)
            })
        }
        // The welcome sheet may have closed before this view was set up (a new chat is
        // being opened) or it may close while the view is already on screen; both paths
        // are wired to the same single-shot consumer.
        .task { takePendingJob() }
        .onChange(of: bridge.pendingPrompt) { _, _ in takePendingJob() }
    }

    private var content: some View {
        VStack(spacing: Spacing.s4) {
            TacetMark(size: 44)
                .padding(.bottom, Spacing.s1)

            Text("What you ask stays here.")
                .font(Typography.tacet())
                .foregroundStyle(Palette.ink)
                .multilineTextAlignment(.center)

            // The product's main promise: on the first screen, without a tap, in plain words.
            Text("Tacet's core runs on this device. Web search and connections engage only if you turn them on, and only in plain sight.")
                .font(Typography.chip())
                .foregroundStyle(Palette.grey)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)

            // Example prompt chips, wrapping to the next line on a narrow screen.
            VStack(spacing: Spacing.s2) {
                ForEach(examples, id: \.self) { text in
                    SampleChip(text: text) { exampleSelected(text) }
                }
            }
            .padding(.top, Spacing.s2)

            // Not all of the capabilities are listed here: instead of explaining, one
            // quiet door is opened and whoever wants to goes through it.
            Button { capabilitiesOpen = true } label: {
                Text("What can Tacet do?")
                    .font(Typography.chip())
                    .foregroundStyle(Palette.muted)
                    .frame(minWidth: touchTarget, minHeight: touchTarget)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .padding(.top, Spacing.s2)
        }
    }

    /// Carries the job coming from the welcome screen into the input field. Single shot —
    /// the bridge empties once it is read.
    private func takePendingJob() {
        guard let text = WelcomeBridge.shared.consume() else { return }
        exampleSelected(text)
    }
}

// A single-line pill chip. 1px line border, grey text.
// The welcome screen and Capabilities use the same chip: the chip style stays in one place.
struct SampleChip: View {
    let text: String
    let tap: () -> Void

    @ScaledMetric(relativeTo: .body) private var touchTarget = Spacing.touchTarget

    var body: some View {
        Button(action: tap) {
            Text(text)
                .font(Typography.chip())
                .foregroundStyle(Palette.grey)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.horizontal, Spacing.s3)
                .padding(.vertical, Spacing.s2)
                // The chip is the welcome screen's ONLY action: the touch target never
                // drops below 44pt.
                .frame(minHeight: touchTarget)
                .overlay(
                    RoundedRectangle(cornerRadius: Spacing.chipCorner, style: .continuous)
                        .stroke(Palette.divider, lineWidth: Spacing.hairline)
                )
        }
        .buttonStyle(.plain)
        .accessibilityHint(Text("Puts this example in the input field"))
    }
}

#Preview {
    EmptyState(exampleSelected: { _ in })
        .background(Palette.background)
}
