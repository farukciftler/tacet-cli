//
//  FileCard.swift
//  Tacet
//
//  The presentation of a produced file — timeline-spec §9.2.
//
//  The card is NOT a bubble, it belongs to the chip family: hairline frame,
//  Spacing.chipCorner corner, Palette.background. It sits under the reply body,
//  aligned with the bubble. The single source of truth is again ToolTrace; the card
//  asks for no new model field, it is fed only from trace.filePath.
//
//  The file is already on the device — the card uses no wording that implies
//  downloading.
//

import SwiftUI

struct FileCard: View {
    /// The file's path on disk (`ToolTrace.filePath`).
    let path: String

    /// Setup from a trace — the path TacetReply will use.
    /// Traces without a file path cannot be drawn as a card; the caller filters them out.
    init?(trace: ToolTrace) {
        guard let path = trace.filePath, !path.isEmpty else { return nil }
        self.path = path
    }

    /// Setup directly from a path (for previews and tests).
    init(path: String) {
        self.path = path
    }

    @State private var previewOpen = false

    private var url: URL { URL(fileURLWithPath: path) }
    private var name: String { url.lastPathComponent }
    private var ext: String { url.pathExtension }

    /// Is the file still on disk. If it was deleted the card stays, the actions drop (§9.2).
    private var isOnDevice: Bool { FileManager.default.fileExists(atPath: path) }

    /// "Spreadsheet · XLSX". When UTType cannot resolve it, the label is already the
    /// extension itself; in that case it is not written twice.
    private var kindLine: String {
        let label = FileIcon.kindLabel(extension: ext)
        let upper = ext.uppercased()
        if label.isEmpty { return upper }
        if label == upper { return upper }
        return "\(label) · \(upper)"
    }

    var body: some View {
        HStack(spacing: Spacing.s3) {
            FileIcon.icon(extension: ext)
                .resizable()
                .scaledToFit()
                .frame(width: 24, height: 24)
                .foregroundStyle(isOnDevice ? Palette.ink : Palette.muted)
                .accessibilityHidden(true)

            VStack(alignment: .leading, spacing: 2) {
                Text(name)
                    .font(Typography.user())
                    .foregroundStyle(isOnDevice ? Palette.ink : Palette.grey)
                    .lineLimit(1)
                    .truncationMode(.middle)

                if isOnDevice {
                    Text(kindLine)
                        .font(Typography.chip())
                        .foregroundStyle(Palette.muted)
                        .lineLimit(1)
                } else {
                    // No silent disappearance: the state is said in words.
                    Text("no longer on this device")
                        .font(Typography.chip())
                        .foregroundStyle(Palette.muted)
                        .lineLimit(1)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityElement(children: .combine)
            .accessibilityLabel(narration)

            if isOnDevice {
                actions
            }
        }
        .padding(.horizontal, Spacing.s3)
        .padding(.vertical, Spacing.s3)
        .background(Palette.background)
        .overlay(
            RoundedRectangle(cornerRadius: Spacing.chipCorner, style: .continuous)
                .stroke(Palette.divider, lineWidth: Spacing.hairline)
        )
        .clipShape(RoundedRectangle(cornerRadius: Spacing.chipCorner, style: .continuous))
        .frame(maxWidth: .infinity, alignment: .leading)
        .sheet(isPresented: $previewOpen) {
            DocumentPreviewSheet(url: url)
        }
    }

    // The two actions on the right: open (the existing QuickLook sheet) and share.
    private var actions: some View {
        HStack(spacing: Spacing.s2) {
            Button {
                previewOpen = true
            } label: {
                Text("Open")
                    .font(Typography.chip())
                    .foregroundStyle(Palette.ink)
                    .padding(.horizontal, Spacing.s3)
                    .padding(.vertical, Spacing.s1 + 2)
                    .overlay(
                        RoundedRectangle(cornerRadius: Spacing.chipCorner, style: .continuous)
                            .stroke(Palette.divider, lineWidth: Spacing.hairline)
                    )
                    .contentShape(RoundedRectangle(cornerRadius: Spacing.chipCorner,
                                                   style: .continuous))
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Open")
            .accessibilityHint("Previews the file")

            ShareLink(item: url) {
                Image(systemName: "square.and.arrow.up")
                    .font(Typography.icon())
                    .foregroundStyle(Palette.grey)
                    .frame(width: 28, height: 28)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Share")
        }
    }

    // VoiceOver: a single sentence. No embellishment about where the file is.
    private var narration: Text {
        isOnDevice
            ? Text("File: \(name), \(kindLine)")
            : Text("File: \(name), no longer on this device")
    }
}

#Preview {
    VStack(alignment: .leading, spacing: Spacing.chipReplyGap) {
        FileCard(path: "/tmp/Star discovery questions.xlsx")
        FileCard(path: "/tmp/summary.pdf")
        FileCard(path: "/tmp/deleted-a-very-long-file-name-example.docx")
    }
    .padding(Spacing.s5)
    .background(Palette.background)
}
