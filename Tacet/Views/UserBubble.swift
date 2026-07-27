import SwiftUI
import UIKit

// The user message bubble — spec §4.2.
// Right-aligned, on a fill background, with a short corner (tail) at the bottom right.
struct UserBubble: View {
    let text: String

    /// The trigger for the copy haptic.
    @State private var copyCounter = 0

    var body: some View {
        HStack {
            // The leading space pushes the bubble to the right.
            Spacer(minLength: Spacing.s4)

            Text(text)
                .font(Typography.user())
                .foregroundStyle(Palette.ink)
                .textSelection(.enabled)
                .padding(.horizontal, Spacing.s3)
                .padding(.vertical, Spacing.s2)
                .background(
                    UnevenRoundedRectangle(
                        topLeadingRadius: 18,
                        bottomLeadingRadius: 18,
                        bottomTrailingRadius: 5,
                        topTrailingRadius: 18
                    )
                    .fill(Palette.fill)
                )
                .contextMenu {
                    Button {
                        UIPasteboard.general.string = text
                        copyCounter += 1
                    } label: {
                        Label("Copy", systemImage: "doc.on.doc")
                    }
                }
                // VoiceOver read it as plain text with no role; give it the "you said"
                // context.
                .accessibilityElement(children: .combine)
                .accessibilityLabel(Text("Your message: \(text)"))
                // The width is 80% of the carrier's (row's) width — spec §4.2.
                // The same pattern as TacetReply's 88%; so that a long message does not
                // spread to full width and break the visual hierarchy.
                .containerRelativeFrame(.horizontal, alignment: .trailing) { width, _ in
                    width * Spacing.userBubbleWidth
                }
        }
        .frame(maxWidth: .infinity, alignment: .trailing)
        .sensoryFeedback(.success, trigger: copyCounter)
    }
}

#Preview {
    VStack(spacing: Spacing.messageGap) {
        UserBubble(text: "Hi, what's the weather like today?")
        UserBubble(text: "Could you create a short reminder for tomorrow? There's a meeting at 9.")
    }
    .padding(Spacing.s5)
    .background(Palette.background)
}
