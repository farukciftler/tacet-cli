import Foundation
import SwiftUI
import UIKit

// MARK: - Chip/card split (pure functions, covered by SelfTest)

/// A turn's traces are spread across three places: the file card, the visible chip and
/// the folded timeline. The decision is made in one place so that the live block and a
/// past message apply the same rule. The single source of truth is again `ToolTrace` —
/// both the card and the chip describe the same trace.
enum ReplyTraces {

    /// A trace that produced a file: it is drawn as a card and REMOVED from the chip list
    /// (timeline-spec §9.4). A failed trace does not become a card; there is no file, and
    /// the place of an error is the chip.
    static func isCard(_ trace: ToolTrace) -> Bool {
        guard let path = trace.filePath, !path.isEmpty else { return false }
        if case .failed = trace.state { return false }
        return true
    }

    /// The traces to be drawn as cards.
    static func cards(_ traces: [ToolTrace]) -> [ToolTrace] {
        traces.filter { isCard($0) }
    }

    /// The chips visible once the reply is finished.
    ///
    /// - `hasTimeline == false` (an old message, no step data): the chips are all visible
    ///   AS THEY ARE TODAY — no timeline is generated retroactively (timeline-spec §5.1).
    /// - `hasTimeline == true`: only read chips go into the fold; `written` and `failed`
    ///   stay where they are (§2.2).
    static func chips(_ traces: [ToolTrace], hasTimeline: Bool) -> [ToolTrace] {
        let withoutCards = traces.filter { !isCard($0) }
        return hasTimeline ? TimelineFolding.outsideFold(withoutCards) : withoutCards
    }

    /// The chips visible while production is running. The running step is already written
    /// in the live ribbon; so as not to say the same thing twice, the `running` chip is
    /// not drawn while the ribbon is present.
    static func liveChips(_ traces: [ToolTrace], hasRibbon: Bool) -> [ToolTrace] {
        let withoutCards = traces.filter { !isCard($0) }
        guard hasRibbon else { return withoutCards }
        return withoutCards.filter { trace in
            if trace.state == .running { return false }
            return !TimelineFolding.isFoldable(trace.state)
        }
    }
}

// The assistant reply: bubble-less, left-aligned serif text.
// While the text streams, a single calm blinking dot is shown.
struct TacetReply: View {
    let text: String
    var isStreaming: Bool = false
    /// The "Download Excel" request from an in-chat table.
    var downloadTable: (Table) -> Void = { _ in }
    /// Is this an error notice (not a real reply)?
    var isError: Bool = false
    /// The "Try again" action in the error block. If nil, the button is not shown.
    var retry: (() -> Void)? = nil
    /// The traces of the files this reply produced — they are drawn as cards under the
    /// body (timeline-spec §9.4). The caller separates them with `ReplyTraces.cards`.
    var fileTraces: [ToolTrace] = []

    // With Reduce Motion on, the animation is off and the dot stays steady.
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// The trigger for the copy haptic.
    @State private var copyCounter = 0

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: Spacing.chipReplyGap) {
                replyBody
                // The file card sits UNDER the body, aligned with the bubble (§9.2).
                ForEach(fileTraces) { trace in
                    if let card = FileCard(trace: trace) { card }
                }
            }
            // The width is 88% of the carrier's (row's) width — spec §4.3.
            .containerRelativeFrame(.horizontal, alignment: .leading) { width, _ in
                width * Spacing.tacetReplyWidth
            }
            Spacer(minLength: 0)
        }
        .sensoryFeedback(.success, trigger: copyCounter)
    }

    @ViewBuilder
    private var replyBody: some View {
        if isError {
            errorBlock
        } else {
            content
                .contextMenu {
                    if !text.isEmpty {
                        Button {
                            UIPasteboard.general.string = text
                            copyCounter += 1
                        } label: {
                            Label("Copy", systemImage: "doc.on.doc")
                        }
                    }
                }
        }
    }

    /// An error notice: a small bordered block in the error colour so it is distinct from
    /// a real reply. The accent colour is not used — the palette is ink/grey + Palette.error.
    private var errorBlock: some View {
        VStack(alignment: .leading, spacing: Spacing.s2) {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.s2) {
                Image(systemName: "exclamationmark.triangle")
                    .font(Typography.chip())
                    .foregroundStyle(Palette.error)
                Text(text)
                    .font(Typography.chip())
                    .foregroundStyle(Palette.error)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            if let retry {
                Button(action: retry) {
                    Text("Try again")
                        .font(Typography.chip())
                        .foregroundStyle(Palette.ink)
                        .padding(.horizontal, Spacing.s3)
                        .padding(.vertical, Spacing.s1)
                        .overlay(
                            RoundedRectangle(cornerRadius: Spacing.chipCorner)
                                .stroke(Palette.divider, lineWidth: Spacing.hairline)
                        )
                }
                .buttonStyle(.plain)
            }
        }
        .padding(Spacing.s3)
        .overlay(
            RoundedRectangle(cornerRadius: Spacing.s3)
                .stroke(Palette.error.opacity(0.4), lineWidth: Spacing.hairline)
        )
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(Text("Error: \(text)"))
    }

    // Empty text while streaming → the dot; while streaming → plain text; once finished,
    // render the tables.
    @ViewBuilder
    private var content: some View {
        if text.isEmpty && isStreaming {
            BreathDot(reduceMotion: reduceMotion)
        } else if isStreaming {
            // During streaming a table can be half-formed — show plain text (so it does
            // not flicker).
            textBody(text)
        } else {
            VStack(alignment: .leading, spacing: Spacing.s3) {
                ForEach(Array(ParseCache.blocks(text).enumerated()), id: \.offset) { _, block in
                    switch block {
                    case .text(let t): textBody(t)
                    case .table(let tb): ChatTable(table: tb, download: downloadTable)
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func textBody(_ t: String) -> some View {
        // While the stream is running, every chunk is a NEW text: writing to the cache
        // only piles up intermediate results that will never be read again.
        Text(formatted(t, cached: !isStreaming))
            .font(Typography.tacet())
            .foregroundStyle(Palette.ink)
            .textSelection(.enabled)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// Resolves inline markdown: `**bold**`, `*italic*`, `` `code` ``.
    ///
    /// `Text(String)` DOES NOT resolve markdown — SwiftUI only does that for
    /// `LocalizedStringKey` literals known at compile time. Because model output is a
    /// runtime String, the asterisks showed up raw on screen ("**Dawn:** 05:04").
    ///
    /// `inlineOnlyPreservingWhitespace` is deliberate: it PRESERVES line breaks. Full
    /// markdown parsing would merge the paragraphs and collapse a list into one line.
    /// Block structure (tables) is already handled separately by `blocks(_:)`.
    ///
    /// If parsing fails we fall back to the raw text: losing the reply because of a
    /// half-finished `**` (which happens during streaming) is unacceptable.
    ///
    /// For finished texts the result comes from the cache — the same message is not
    /// re-parsed on every redraw (see `ParseCache`).
    private func formatted(_ raw: String, cached: Bool) -> AttributedString {
        ParseCache.formatted(raw, cached: cached)
    }

    // The block split lives inside `Table.blocks` — that is the single source of truth.
    //
    // The old local parser treated the separator row ("|---|") as MANDATORY, and when
    // `fromMarkdown` returned nil it DROPPED the collected pipe rows without putting them
    // in any block — when the model wrote a valid but separator-less table, the content
    // silently vanished from the screen (audit P1-5). Because `Table.blocks` returns every
    // line it does not recognise as `.text`, that is not possible.
}

// MARK: - Parse cache

/// Markdown resolution and the block split are PURE functions: the same text always gives
/// the same result. Even so, finished messages were being re-parsed on every redraw
/// throughout the stream — in a long chat that meant resolving the markdown of the whole
/// visible history dozens of times per second.
///
/// The key is the text itself, NOT the message id: the same text gives the same result in
/// different messages too, and if the content changes so does the key — meaning a stale
/// result is structurally impossible. `NSCache` empties itself under memory pressure — it
/// does not grow without bound as the chat gets longer.
private enum ParseCache {

    private final class FormatBox {
        let value: AttributedString
        init(_ value: AttributedString) { self.value = value }
    }

    private final class BlockBox {
        let value: [Table.TextBlock]
        init(_ value: [Table.TextBlock]) { self.value = value }
    }

    private static let formatCache: NSCache<NSString, FormatBox> = {
        let cache = NSCache<NSString, FormatBox>()
        cache.countLimit = 200
        return cache
    }()

    private static let blockCache: NSCache<NSString, BlockBox> = {
        let cache = NSCache<NSString, BlockBox>()
        cache.countLimit = 200
        return cache
    }()

    /// Inline markdown resolution. While `cached == false` (during streaming) the result
    /// is not stored; every text there is an intermediate state that will never be seen
    /// again.
    static func formatted(_ raw: String, cached: Bool) -> AttributedString {
        let key = raw as NSString
        if cached, let box = formatCache.object(forKey: key) { return box.value }
        let result = (try? AttributedString(
            markdown: raw,
            options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)
        )) ?? AttributedString(raw)
        if cached { formatCache.setObject(FormatBox(result), forKey: key) }
        return result
    }

    /// The block split. It is only called on finished text (no table is drawn during
    /// streaming), so it is cached unconditionally.
    static func blocks(_ text: String) -> [Table.TextBlock] {
        let key = text as NSString
        if let box = blockCache.object(forKey: key) { return box.value }
        let result = Table.blocks(text)
        blockCache.setObject(BlockBox(result), forKey: key)
        return result
    }
}

// A single dot, blinking slowly like a breath.
/// The wait is the brand mark itself: the ensō spins until the first token
/// arrives. With reduced motion it stands still (the mark alone already reads
/// as "Tacet is here").
private struct BreathDot: View {
    let reduceMotion: Bool

    var body: some View {
        SpinningTacetMark(size: 16, reduceMotion: reduceMotion)
            .accessibilityLabel("Tacet is typing")
    }
}

#Preview("Reply") {
    VStack(alignment: .leading, spacing: Spacing.messageGap) {
        TacetReply(text: "You have three meetings today. The first one starts at ten.")
        TacetReply(text: "", isStreaming: true)
        TacetReply(text: "The model isn't ready on this device.",
                   isError: true, retry: {})
        TacetReply(text: "I've prepared the table.",
                   fileTraces: [ToolTrace(icon: "tablecells", text: "table written",
                                          state: .written,
                                          filePath: "/tmp/Star discovery questions.xlsx")])
    }
    .padding(Spacing.s5)
}
