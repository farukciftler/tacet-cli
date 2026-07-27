//
//  DocumentPreview.swift
//  Tacet
//
//  The wrapper that previews a file with QuickLook, plus the sharing sheet.
//  Document engines produce a URL; here we show it, share it and save it.
//

import SwiftUI
import QuickLook
import UIKit

// The wrapper that carries QLPreviewController into SwiftUI.
// It previews the single URL it is given.
struct DocumentPreview: UIViewControllerRepresentable {
    let url: URL

    func makeCoordinator() -> Coordinator {
        Coordinator(url: url)
    }

    func makeUIViewController(context: Context) -> QLPreviewController {
        let controller = QLPreviewController()
        controller.dataSource = context.coordinator
        return controller
    }

    func updateUIViewController(_ controller: QLPreviewController, context: Context) {
        // If the URL changes, refresh the data source and reload.
        context.coordinator.url = url
        controller.reloadData()
    }

    // A single-item data source.
    final class Coordinator: NSObject, QLPreviewControllerDataSource {
        var url: URL

        init(url: URL) {
            self.url = url
        }

        func numberOfPreviewItems(in controller: QLPreviewController) -> Int {
            1
        }

        func previewController(_ controller: QLPreviewController, previewItemAt index: Int) -> QLPreviewItem {
            url as NSURL
        }
    }
}

// A convenience sheet wrapping the preview with a title bar, sharing and closing.
struct DocumentPreviewSheet: View {
    let url: URL
    @Environment(\.dismiss) private var close
    /// For xlsx, our own table instead of QuickLook. If nil, we fall back to QuickLook.
    @State private var table: Table?

    var body: some View {
        NavigationStack {
            content
                .navigationTitle(url.lastPathComponent)
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) {
                        Button("Close") {
                            close()
                        }
                        .font(Typography.chip())
                        .foregroundStyle(Palette.grey)
                    }
                    ToolbarItem(placement: .topBarTrailing) {
                        // Share / save to Files.
                        ShareLink(item: url) {
                            Image(systemName: "square.and.arrow.up")
                                .foregroundStyle(Palette.ink)
                        }
                        .accessibilityLabel("Share")
                    }
                }
        }
        .task { loadTable() }
    }

    /// For xlsx, QuickLook draws the table in the top-left corner of the page at its
    /// natural point size: on an iPhone it stays too small to read and there is no zoom
    /// API. The app's own table renderer fills the screen, follows the design language
    /// and honours Dynamic Type. For the other formats (pdf, docx, html) QuickLook
    /// already draws at full width, so it is kept.
    @ViewBuilder private var content: some View {
        if let table {
            ScrollView {
                ChatTable(table: table, showDownload: false)
                    .padding(Spacing.s4)
            }
        } else {
            DocumentPreview(url: url)
                .ignoresSafeArea(edges: .bottom)
        }
    }

    private func loadTable() {
        guard table == nil, url.pathExtension.lowercased() == "xlsx" else { return }
        // If it cannot be read we fall back to QuickLook silently: showing something,
        // however small, is preferable to the preview never opening at all.
        table = (try? ExcelEngine().read(url: url))?.table
    }
}
