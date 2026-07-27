//
//  TimelineRibbon.swift
//  Tacet
//
//  The live timeline ribbon (timeline-spec §3.1). While the reply is being produced,
//  a SINGLE left-aligned row at the height of the reply bubble: spinner + the text of
//  the current step; above it a muted "step n" counter.
//
//  There is no drama: the row text is not an invented state verb, it is a
//  deterministic event that really happens in the code (§2.1). The ribbon knows
//  nothing about the model.
//

import SwiftUI

struct TimelineRibbon: View {
    /// The turn's steps so far (`TimelineRecorder.steps`).
    let steps: [TimelineStep]
    /// The live traces, used to read the text of tool steps.
    var traces: [ToolTrace] = []

    /// The timeline opened on tap — it does not hold up the running work, it opens in
    /// place.
    @State private var lineOpen = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var lastStep: TimelineStep? { steps.last }

    /// The text of the live row. On a tool step it is the trace text; if the trace has
    /// not arrived yet the step stays empty and the row shows only the spinner.
    private var liveText: String {
        guard let lastStep else { return "" }
        return TimelineText.row(lastStep, traces: traces)
    }

    var body: some View {
        if steps.isEmpty {
            EmptyView()
        } else {
            VStack(alignment: .leading, spacing: Spacing.s1) {
                if steps.count > 1 {
                    Text(String(localized: "step \(steps.count)"))
                        .font(Typography.chip())
                        .foregroundStyle(Palette.muted)
                        .accessibilityHidden(true)
                }

                Button {
                    if reduceMotion {
                        lineOpen.toggle()
                    } else {
                        withAnimation(.easeInOut(duration: 0.18)) { lineOpen.toggle() }
                    }
                } label: {
                    HStack(spacing: Spacing.s2) {
                        ProgressView()
                            .controlSize(.small)
                            .frame(width: 13, height: 13)
                            .accessibilityHidden(true)
                        Text(liveText)
                            .font(Typography.chip())
                            .foregroundStyle(Palette.grey)
                            .lineLimit(1)
                            .truncationMode(.tail)
                            // A row change happens as a calm transition, not a slide.
                            .contentTransition(.opacity)
                        Spacer(minLength: 0)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)

                if lineOpen {
                    // The past steps of the running work — visible without waiting.
                    TimelineLine(steps: steps, traces: traces, openAtStart: true)
                        .transition(reduceMotion
                                    ? .opacity
                                    : .opacity.combined(with: .move(edge: .top)))
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: liveText)
            // A step change is announced to VoiceOver.
            .accessibilityElement(children: .contain)
            .accessibilityLabel(Text(liveText))
            .accessibilityValue(Text(String(localized: "step \(steps.count)")))
            .accessibilityHint(Text("Tap to see the steps"))
            .accessibilityAddTraits(.updatesFrequently)
        }
    }
}

#Preview("Live ribbon") {
    let traceID = UUID()
    let steps: [TimelineStep] = [
        TimelineStep(kind: .routing, text: "routed · calendar profile",
                     start: Date(), end: Date().addingTimeInterval(0.2)),
        TimelineStep(kind: .tool, toolTraceID: traceID)
    ]
    let traces = [ToolTrace(id: traceID, icon: "calendar",
                            text: "reading calendar · today", state: .running)]

    return VStack(alignment: .leading, spacing: Spacing.chipReplyGap) {
        TimelineRibbon(steps: steps, traces: traces)
    }
    .padding(Spacing.s5)
    .frame(maxWidth: .infinity, alignment: .leading)
    .background(Palette.background)
}
