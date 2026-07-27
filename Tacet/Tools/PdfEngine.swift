//
//  PdfEngine.swift
//  Tacet
//
//  The PDF engine: draws title + body onto A4 pages (UIGraphicsPDFRenderer) and extracts
//  the text with PDFKit when reading. No network; drawing happens on the main actor.
//

import Foundation
import UIKit
import PDFKit
import CoreText

/// The write/read engine for the PDF format.
struct PdfEngine: DocumentEngine {
    var format: DocumentFormat { .pdf }

    // A4 dimensions (points) and the margin.
    private let pageWidth: CGFloat = 595
    private let pageHeight: CGFloat = 842
    private let edge: CGFloat = 48
    /// The minimum vertical space a line needs to fit; below this the page is turned
    /// (otherwise the splitting loop makes no progress).
    private let minLineArea: CGFloat = 24

    func writeRaw(fileName: String, title: String?, body: String?, table: Table?, folder: URL) throws -> URL {
        let url = targetURL(fileName: fileName, folder: folder)

        let page = CGRect(x: 0, y: 0, width: pageWidth, height: pageHeight)
        let contentWidth = pageWidth - edge * 2
        let lowerLimit = pageHeight - edge

        // Collect the body text: markdown/plain text + the table's markdown if there is one.
        var chunks: [String] = []
        if let body, !body.isEmpty { chunks.append(body) }
        if let table {
            let mt = table.markdown
            if !mt.isEmpty { chunks.append(mt) }
        }
        let fullBody = chunks.joined(separator: "\n\n")
        let lines = fullBody.isEmpty ? [] : fullBody.components(separatedBy: "\n")

        let renderer = UIGraphicsPDFRenderer(bounds: page)
        let data = renderer.pdfData { ctx in
            ctx.beginPage()
            var y = edge

            // The top title (if any) — 20pt bold Helvetica.
            if let title, !title.isEmpty {
                let bFont = UIFont(name: "Helvetica-Bold", size: 20) ?? UIFont.boldSystemFont(ofSize: 20)
                let bAttributes: [NSAttributedString.Key: Any] = [.font: bFont, .foregroundColor: UIColor.black]
                let drawn = draw(
                    text: title, attributes: bAttributes, width: contentWidth,
                    y: &y, lowerLimit: lowerLimit, ctx: ctx, page: page
                )
                y += drawn + 16
            }

            // The body lines — with a simple markdown interpretation.
            for raw in lines {
                let (text, attributes, extraSpace) = formatLine(raw, width: contentWidth)
                if text.isEmpty {
                    // An empty line: paragraph spacing.
                    y += 8
                    if y > lowerLimit { ctx.beginPage(); y = edge }
                    continue
                }
                let drawn = draw(
                    text: text, attributes: attributes, width: contentWidth,
                    y: &y, lowerLimit: lowerLimit, ctx: ctx, page: page
                )
                y += drawn + extraSpace
            }
        }

        try data.write(to: url)
        return url
    }

    /// Draws a block of text with word wrapping; opens a new page when needed.
    /// A block that EXCEEDS the page height is split into chunks and drawn across several
    /// pages — such a block used to be drawn as-is, and the part overflowing past the bottom
    /// of the page never appeared in the PDF at all (silent data loss).
    ///
    /// The return value is the height of the LAST chunk; `y` is set to the top edge of that
    /// chunk, so the caller's `y += drawn` arithmetic is not thrown off.
    private func draw(
        text: String,
        attributes: [NSAttributedString.Key: Any],
        width: CGFloat,
        y: inout CGFloat,
        lowerLimit: CGFloat,
        ctx: UIGraphicsPDFRendererContext,
        page: CGRect
    ) -> CGFloat {
        let full = NSAttributedString(string: text, attributes: attributes)
        guard full.length > 0 else { return 0 }

        let framesetter = CTFramesetterCreateWithAttributedString(full)
        let pageArea = lowerLimit - edge
        var start = 0
        var lastChunkHeight: CGFloat = 0

        while start < full.length {
            let remainingText = full.attributedSubstring(
                from: NSRange(location: start, length: full.length - start)
            )
            let needed = measure(remainingText, width: width)
            var space = lowerLimit - y

            // It does not all fit on this page but it does fit on a clean one: turn the page
            // (do not split the block for nothing). This is the old behavior.
            if needed > space, y > edge, needed <= pageArea {
                ctx.beginPage(); y = edge; space = pageArea
            }
            // If we are at the bottom of the page not even one line fits; turn it.
            if space < minLineArea, y > edge {
                ctx.beginPage(); y = edge; space = pageArea
            }

            if needed <= space {
                remainingText.draw(
                    with: CGRect(x: edge, y: y, width: width, height: needed),
                    options: [.usesLineFragmentOrigin, .usesFontLeading], context: nil
                )
                return needed
            }

            // It does not fit: find the longest chunk that does fit on this page.
            guard let chunkLength = fittingLength(
                framesetter: framesetter, full: full, start: start,
                width: width, height: space
            ) else {
                // Not even a single line fits (an unusual font/metric): accept the overflow,
                // do not lose data.
                remainingText.draw(
                    with: CGRect(x: edge, y: y, width: width, height: needed),
                    options: [.usesLineFragmentOrigin, .usesFontLeading], context: nil
                )
                return needed
            }

            let chunk = full.attributedSubstring(
                from: NSRange(location: start, length: chunkLength)
            )
            let chunkHeight = measure(chunk, width: width)
            chunk.draw(
                with: CGRect(x: edge, y: y, width: width, height: chunkHeight),
                options: [.usesLineFragmentOrigin, .usesFontLeading], context: nil
            )
            start += chunkLength
            lastChunkHeight = chunkHeight
            if start < full.length {
                ctx.beginPage()
                y = edge
            }
        }
        return lastChunkHeight
    }

    /// The wrapped height of a block.
    private func measure(_ attr: NSAttributedString, width: CGFloat) -> CGFloat {
        let limit = CGSize(width: width, height: .greatestFiniteMagnitude)
        let box = attr.boundingRect(
            with: limit, options: [.usesLineFragmentOrigin, .usesFontLeading], context: nil
        )
        return ceil(box.height)
    }

    /// The number of characters from `start` that fit into the given height (it does not cut
    /// in the middle of a line). nil if no line fits at all.
    ///
    /// Because CoreText's visible range and TextKit's measurement do not always agree exactly,
    /// the result is verified; if it overflows it is retried with a narrower height.
    private func fittingLength(
        framesetter: CTFramesetter,
        full: NSAttributedString,
        start: Int,
        width: CGFloat,
        height: CGFloat
    ) -> Int? {
        var attempt = height
        for _ in 0..<3 {
            guard attempt >= minLineArea else { return nil }
            let path = CGPath(
                rect: CGRect(x: 0, y: 0, width: width, height: attempt), transform: nil
            )
            let frame = CTFramesetterCreateFrame(
                framesetter, CFRange(location: start, length: 0), path, nil
            )
            let visible = CTFrameGetVisibleStringRange(frame)
            let length = visible.length
            guard length > 0, start + length <= full.length else { return nil }
            let chunk = full.attributedSubstring(
                from: NSRange(location: start, length: length)
            )
            if measure(chunk, width: width) <= height { return length }
            attempt *= 0.85
        }
        return nil
    }

    /// Simple markdown: "# " is a heading (bold, large), "- " is a bullet "• ", the rest is normal.
    private func formatLine(_ raw: String, width: CGFloat) -> (String, [NSAttributedString.Key: Any], CGFloat) {
        let paragraph = NSMutableParagraphStyle()
        paragraph.lineSpacing = 12 * 0.4          // 12pt body, 1.4 line spacing
        paragraph.lineBreakMode = .byWordWrapping

        let bodyFont = UIFont(name: "Helvetica", size: 12) ?? UIFont.systemFont(ofSize: 12)

        if raw.hasPrefix("# ") {
            let text = String(raw.dropFirst(2))
            let font = UIFont(name: "Helvetica-Bold", size: 16) ?? UIFont.boldSystemFont(ofSize: 16)
            let attributes: [NSAttributedString.Key: Any] = [
                .font: font, .foregroundColor: UIColor.black, .paragraphStyle: paragraph
            ]
            return (text, attributes, 8)
        }

        if raw.hasPrefix("- ") {
            let text = "• " + raw.dropFirst(2)
            let attributes: [NSAttributedString.Key: Any] = [
                .font: bodyFont, .foregroundColor: UIColor.black, .paragraphStyle: paragraph
            ]
            return (text, attributes, 2)
        }

        // Table rows and plain text: align with a monospace font when needed.
        let font: UIFont = raw.hasPrefix("|")
            ? (UIFont(name: "Menlo", size: 10) ?? UIFont.monospacedSystemFont(ofSize: 10, weight: .regular))
            : bodyFont
        let attributes: [NSAttributedString.Key: Any] = [
            .font: font, .foregroundColor: UIColor.black, .paragraphStyle: paragraph
        ]
        return (raw, attributes, 2)
    }

    func read(url: URL) throws -> DocumentBody {
        guard let document = PDFDocument(url: url) else {
            throw DocumentEngineError.noContent
        }
        let text = document.string
        if let text, !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return DocumentBody(text: text)
        }
        // A scanned PDF: there is no text layer.
        return DocumentBody(text: "The PDF text couldn’t be extracted (it may be scanned).")
    }
}
