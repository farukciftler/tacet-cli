//
//  BelgeOnizleme.swift
//  ketum
//
//  QuickLook ile bir dosyayı önizleyen sarmalayıcı ve paylaşım sayfası.
//  Belge motorları bir URL üretir; burada onu gösterir, paylaşır, kaydederiz.
//

import SwiftUI
import QuickLook
import UIKit

// QLPreviewController'ı SwiftUI'ye taşıyan sarmalayıcı.
// Verilen tek URL'i önizler.
struct BelgeOnizleme: UIViewControllerRepresentable {
    let url: URL

    func makeCoordinator() -> Koordinator {
        Koordinator(url: url)
    }

    func makeUIViewController(context: Context) -> QLPreviewController {
        let denetleyici = QLPreviewController()
        denetleyici.dataSource = context.coordinator
        return denetleyici
    }

    func updateUIViewController(_ denetleyici: QLPreviewController, context: Context) {
        // URL değişirse veri kaynağını tazele ve yeniden yükle.
        context.coordinator.url = url
        denetleyici.reloadData()
    }

    // Tek öğeli veri kaynağı.
    final class Koordinator: NSObject, QLPreviewControllerDataSource {
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

// Önizlemeyi başlık çubuğu, paylaşım ve kapatma ile saran kolaylık sayfası.
struct BelgeOnizlemeSheet: View {
    let url: URL
    @Environment(\.dismiss) private var kapat

    var body: some View {
        NavigationStack {
            BelgeOnizleme(url: url)
                .ignoresSafeArea(edges: .bottom)
                .navigationTitle(url.lastPathComponent)
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) {
                        Button("Kapat") {
                            kapat()
                        }
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.gri)
                    }
                    ToolbarItem(placement: .topBarTrailing) {
                        // Paylaş / Dosyalar'a kaydet.
                        ShareLink(item: url) {
                            Image(systemName: "square.and.arrow.up")
                                .foregroundStyle(Renk.murekkep)
                        }
                        .accessibilityLabel("Paylaş")
                    }
                }
        }
    }
}
