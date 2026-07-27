//
//  IntentCard.swift
//  Tacet
//
//  The snippet views that screenless intents (classes B and C) show inside Shortcuts/Siri
//  (intent-plan §6.1).
//
//  Product principle #2 — "it does not tell, it shows" — applies here too: when a note is
//  added the user sees WHAT was added and which keys it will match on; when a file is given
//  the user sees WHICH file went out. A trace instead of a sentence.
//
//  Palette rule: a single background, no accent colour, no shadow, no gradient, a 1px
//  hairline border. No fixed point sizes — only `Typography` tokens.
//

import SwiftUI

/// `AddNoteIntent`'s snippet: the added note + the derived keys.
struct NoteCard: View {
    let text: String
    let keys: [String]

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.s2) {
            Text(text)
                .font(Typography.tacet())
                .foregroundStyle(Palette.ink)
                .fixedSize(horizontal: false, vertical: true)

            if !keys.isEmpty {
                Text(keys.joined(separator: " · "))
                    .font(Typography.chip())
                    .foregroundStyle(Palette.grey)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(Spacing.s4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Palette.background)
        .overlay {
            RoundedRectangle(cornerRadius: Spacing.chipCorner, style: .continuous)
                .stroke(Palette.divider, lineWidth: Spacing.hairline)
        }
        .accessibilityElement(children: .combine)
    }
}

/// `ProvideDocumentIntent`'s snippet: the identity of the file that goes out to Shortcuts.
struct DocumentCard: View {
    let name: String
    let subtitle: String
    let icon: String

    @ScaledMetric(relativeTo: .subheadline) private var iconColumn = Spacing.iconColumn

    var body: some View {
        HStack(alignment: .top, spacing: Spacing.s3) {
            Image(systemName: icon)
                .font(Typography.icon())
                .foregroundStyle(Palette.grey)
                .frame(width: iconColumn, alignment: .leading)

            VStack(alignment: .leading, spacing: Spacing.s1) {
                Text(name)
                    .font(Typography.user())
                    .foregroundStyle(Palette.ink)
                    .fixedSize(horizontal: false, vertical: true)
                Text(subtitle)
                    .font(Typography.chip())
                    .foregroundStyle(Palette.grey)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(Spacing.s4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Palette.background)
        .overlay {
            RoundedRectangle(cornerRadius: Spacing.chipCorner, style: .continuous)
                .stroke(Palette.divider, lineWidth: Spacing.hairline)
        }
        .accessibilityElement(children: .combine)
    }
}
