//
//  TimelineLine.swift
//  Tacet
//
//  The folded timeline row + the collapsible timeline (timeline-spec §3.2, §3.3).
//
//  The folding rule (§2.2) lives here as a PURE FUNCTION: side-effect and error
//  chips stay OUTSIDE the fold, only read chips go inside. Transparency is not a
//  collapsible decoration.
//
//  The detail of a tool step is the EXISTING `ToolChipDetail` sheet — no second
//  detail surface is written (§2.4).
//

import SwiftUI

// MARK: - Folding rule (pure functions, covered by SelfTest)

enum TimelineFolding {

    /// The traces that stay OUTSIDE the fold and are always visible.
    /// Rule: a side effect (`written`) and an error (`failed`) are never hidden.
    /// In addition, states that are waiting for user action or are still running are
    /// kept outside too — folding those would create a dead end (the spec text counts
    /// only written/failed; this is a reasoned extension).
    static func outsideFold(_ traces: [ToolTrace]) -> [ToolTrace] {
        traces.filter { !isFoldable($0.state) }
    }

    /// The traces that go INSIDE the fold — reads only.
    static func insideFold(_ traces: [ToolTrace]) -> [ToolTrace] {
        traces.filter { isFoldable($0.state) }
    }

    /// A single state decision: only `readOk` folds.
    static func isFoldable(_ state: ToolState) -> Bool {
        switch state {
        case .readOk:
            return true
        case .running, .written, .permissionRequired, .failed, .awaitingApproval, .notSent:
            return false
        }
    }

    /// Should the folding row be drawn? If there are no steps, or there is a single step
    /// and it is a write (a reply with no tools), the timeline STAYS SILENT — it has
    /// nothing to say (§3.2).
    static func showsRow(_ steps: [TimelineStep]) -> Bool {
        guard !steps.isEmpty else { return false }
        if steps.count == 1, steps[0].kind == .writing { return false }
        return true
    }

    /// The text of the folding row: "trail · 4 steps · 6 s", or if there are failures
    /// "trail · 4 steps · 1 not completed" (§3.4).
    static func summaryText(steps: [TimelineStep], traces: [ToolTrace]) -> String {
        let count = steps.count
        let notCompleted = traces.filter {
            if case .failed = $0.state { return true }
            return false
        }.count

        let stepPart = String(localized: "\(count) steps")
        if notCompleted > 0 {
            let errorPart = String(localized: "\(notCompleted) not completed")
            return "\(TimelineFolding.title) · \(stepPart) · \(errorPart)"
        }
        return "\(TimelineFolding.title) · \(stepPart) · \(TimelineDuration.text(totalDuration(steps)))"
    }

    /// The total duration of the closed steps. A running step is not added to the sum —
    /// it is not guessed (§7: no lying progress).
    static func totalDuration(_ steps: [TimelineStep]) -> TimeInterval {
        steps.reduce(0) { $0 + ($1.duration ?? 0) }
    }

    static var title: String { String(localized: "trail") }
}

// MARK: - Row text (single source of truth: ToolTrace)

enum TimelineText {
    /// The visible text of a step. On a tool step the text is read from the trace, not
    /// from the step.
    static func row(_ step: TimelineStep, traces: [ToolTrace]) -> String {
        if step.kind == .tool {
            return trace(step, traces: traces)?.text ?? step.text
        }
        return step.text
    }

    /// The trace bound to the step.
    static func trace(_ step: TimelineStep, traces: [ToolTrace]) -> ToolTrace? {
        guard let id = step.toolTraceID else { return nil }
        return traces.first { $0.id == id }
    }
}

// MARK: - Duration format

enum TimelineDuration {
    /// "0.2 s" / "6 s" — the decimal separator follows the device language.
    static func text(_ seconds: TimeInterval) -> String {
        let f = NumberFormatter()
        f.locale = .current
        f.numberStyle = .decimal
        f.minimumFractionDigits = 0
        f.maximumFractionDigits = seconds < 10 ? 1 : 0
        let number = f.string(from: NSNumber(value: max(0, seconds))) ?? "0"
        return String(localized: "\(number) s")
    }
}

// MARK: - View

/// The collapsible timeline. NOT a `DisclosureGroup` — it is built by hand so that the
/// opening animation respects `reduceMotion` and the rows stay in the hairline language.
/// The default is ALWAYS CLOSED; the open/closed state is NOT WRITTEN to disk and is not
/// stored with the message (§3.3) — it lives only as long as the view does.
struct TimelineLine: View {
    let steps: [TimelineStep]
    let traces: [ToolTrace]

    @State private var isOpen: Bool
    @State private var detailTrace: ToolTrace?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// `openAtStart` (a tap on the live ribbon) is an INITIAL value, not a continuous
    /// binding: when it was assigned in `onAppear`, the fold the user had opened by hand
    /// was reset every time the LazyVStack row left the screen and came back. Now it is
    /// only here.
    init(steps: [TimelineStep], traces: [ToolTrace] = [], openAtStart: Bool = false) {
        self.steps = steps
        self.traces = traces
        _isOpen = State(initialValue: openAtStart)
    }

    var body: some View {
        if TimelineFolding.showsRow(steps) {
            VStack(alignment: .leading, spacing: Spacing.s2) {
                foldRow
                if isOpen {
                    timeline
                        .transition(reduceMotion
                                    ? .opacity
                                    : .opacity.combined(with: .move(edge: .top)))
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .sheet(item: $detailTrace) { trace in
                ToolChipDetail(trace: trace)
            }
        }
    }

    // The folded row: one step back from the chips — NO hairline frame, muted.
    private var foldRow: some View {
        Button {
            if reduceMotion {
                isOpen.toggle()
            } else {
                withAnimation(.easeInOut(duration: 0.18)) { isOpen.toggle() }
            }
        } label: {
            HStack(spacing: Spacing.s1) {
                Text(TimelineFolding.summaryText(steps: steps, traces: traces))
                    .font(Typography.chip())
                    .foregroundStyle(Palette.muted)
                Image(systemName: isOpen ? "chevron.down" : "chevron.right")
                    .font(Typography.iconSmall())
                    .foregroundStyle(Palette.muted)
                    .accessibilityHidden(true)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text(TimelineFolding.summaryText(steps: steps, traces: traces)))
        .accessibilityHint(isOpen ? Text("Tap to close") : Text("Tap to open"))
    }

    // The open list: a vertical hairline connects the steps; no icons, no colour.
    private var timeline: some View {
        VStack(alignment: .leading, spacing: Spacing.s2) {
            ForEach(steps) { step in
                row(step)
            }
        }
        .padding(.leading, Spacing.s3)
        .overlay(alignment: .leading) {
            Rectangle()
                .fill(Palette.divider)
                .frame(width: Spacing.hairline)
                .accessibilityHidden(true)
        }
        .accessibilityElement(children: .contain)
    }

    @ViewBuilder
    private func row(_ step: TimelineStep) -> some View {
        let trace = TimelineText.trace(step, traces: traces)
        let text = TimelineText.row(step, traces: traces)
        let duration = step.duration.map { TimelineDuration.text($0) } ?? "—"

        if let trace {
            // A tool step: the detail is the EXISTING ToolChipDetail sheet (§2.4).
            Button { detailTrace = trace } label: {
                rowBody(text: text, duration: duration, chevron: true)
            }
            .buttonStyle(.plain)
            .accessibilityLabel(Text("\(text), \(duration)"))
            .accessibilityHint(Text("Tap for details"))
        } else {
            // A pipeline step: it has no detail, the duration is already in the row.
            rowBody(text: text, duration: duration, chevron: false)
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(Text("\(text), \(duration)"))
        }
    }

    private func rowBody(text: String, duration: String, chevron: Bool) -> some View {
        HStack(spacing: Spacing.s2) {
            Text(text)
                .font(Typography.chip())
                .foregroundStyle(Palette.grey)
                .lineLimit(1)
                .truncationMode(.middle)
            Spacer(minLength: Spacing.s2)
            Text(duration)
                .font(Typography.chip())
                .foregroundStyle(Palette.muted)
                .monospacedDigit()
            if chevron {
                Image(systemName: "chevron.right")
                    .font(Typography.iconSmall())
                    .foregroundStyle(Palette.muted)
                    .accessibilityHidden(true)
            }
        }
        .contentShape(Rectangle())
    }
}

#Preview("Timeline line") {
    let traceID = UUID()
    let steps: [TimelineStep] = [
        TimelineStep(kind: .routing, text: "routed · calendar profile",
                     start: Date(), end: Date().addingTimeInterval(0.2)),
        TimelineStep(kind: .enrichment, text: "skill attached · read-document",
                     start: Date(), end: Date().addingTimeInterval(0.05)),
        TimelineStep(kind: .tool, toolTraceID: traceID,
                     start: Date(), end: Date().addingTimeInterval(1.1)),
        TimelineStep(kind: .writing, text: "written",
                     start: Date(), end: Date().addingTimeInterval(4.4))
    ]
    let traces = [ToolTrace(id: traceID, icon: "calendar", text: "calendar read · 3 events",
                            state: .readOk, rawInput: "today", rawOutput: "09:00 meeting")]

    return VStack(alignment: .leading, spacing: Spacing.chipReplyGap) {
        TimelineLine(steps: steps, traces: traces)
        TacetReply(text: "You have three meetings today.")
    }
    .padding(Spacing.s5)
    .frame(maxWidth: .infinity, alignment: .leading)
    .background(Palette.background)
}
