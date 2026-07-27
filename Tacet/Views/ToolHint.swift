//
//  ToolHint.swift
//  Tacet
//
//  The one-off hint (coach mark) for the tool trace. The product's strongest
//  difference is the tool chip, but it is UNDISCOVERABLE: for a user who does not
//  know the chip can be tapped, the transparency claim stays only a claim. This is
//  the one thing worth explaining — there is no bubble on any other surface.
//
//  No bubble/arrow/shadow: a left-aligned box in the same hairline language as the
//  chips.
//
//  Bound to where it is produced: it is drawn inside the `ChatView.LiveBlock` body,
//  right after the chip ForEach and before `TimelineRibbon`, only while
//  `!executor.traces.isEmpty`. A SECOND CALL MUST NOT BE ADDED — the flag is one-off.
//

import SwiftUI

/// It carries its own flag: if it has been seen it draws nothing. That way the caller
/// does not need to set up a flag/state — a one-line `ToolHint()` is enough.
struct ToolHint: View {
    @AppStorage(WelcomeSetting.chipHintKey) private var seen = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @ScaledMetric(relativeTo: .body) private var touchTarget = Spacing.touchTarget

    var body: some View {
        if !seen {
            VStack(alignment: .leading, spacing: Spacing.s2) {
                // It does not say "chip": within a turn, read chips can fold into the
                // timeline ribbon; the hint has to cover both states.
                Text("The line above is a tool trace: Tacet leaves one here for every tool it touches. Tap it to see exactly what it sent and received, raw.")
                    .font(Typography.chip())
                    .foregroundStyle(Palette.grey)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)

                Button { seen = true } label: {
                    Text("Got it")
                        .font(Typography.chip())
                        .foregroundStyle(Palette.ink)
                        .frame(minWidth: touchTarget, minHeight: touchTarget)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .frame(maxWidth: .infinity, alignment: .trailing)
            }
            .padding(.horizontal, Spacing.s3)
            .padding(.vertical, Spacing.s3)
            .overlay(
                RoundedRectangle(cornerRadius: Spacing.chipCorner, style: .continuous)
                    .stroke(Palette.divider, lineWidth: Spacing.hairline)
            )
            // The box is navigated as a whole; the text is a static element, the button
            // stays separate. Focus is NOT STOLEN: the user may be listening to the
            // streaming reply.
            .accessibilityElement(children: .contain)
            .transition(reduceMotion ? .opacity : .opacity.combined(with: .offset(y: 2)))
        }
    }
}

#Preview {
    ToolHint()
        .padding(Spacing.s5)
        .background(Palette.background)
}
