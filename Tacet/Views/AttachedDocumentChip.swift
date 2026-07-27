import SwiftUI

// The attached document chip — shown right above the input field.
// A calm pill: format icon on the left, file name in the middle, remove button on the right.
struct AttachedDocumentChip: View {
    let document: AttachedDocument
    let remove: () -> Void

    var body: some View {
        HStack(spacing: Spacing.s2) {
            Image(systemName: document.format.icon)
                .font(Typography.chip())
                .foregroundStyle(Palette.grey)

            Text(document.name)
                .font(Typography.chip())
                .foregroundStyle(Palette.ink)
                .lineLimit(1)

            Button(action: remove) {
                Image(systemName: "xmark.circle.fill")
                    .font(Typography.user())
                    .foregroundStyle(Palette.grey)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Remove document")
        }
        .padding(.horizontal, Spacing.s3)
        .padding(.vertical, Spacing.s2)
        .overlay(
            RoundedRectangle(cornerRadius: Spacing.chipCorner)
                .stroke(Palette.divider, lineWidth: Spacing.hairline)
        )
        .contentShape(RoundedRectangle(cornerRadius: Spacing.chipCorner))
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityLabel("Attached document: \(document.name)")
    }
}

#Preview {
    AttachedDocumentChip(
        document: AttachedDocument(
            id: UUID(),
            url: URL(fileURLWithPath: "/tmp/summary.pdf"),
            name: "summary.pdf",
            format: .pdf
        ),
        remove: {}
    )
    .padding(Spacing.s5)
    .frame(maxWidth: .infinity, alignment: .leading)
    .background(Palette.background)
}
