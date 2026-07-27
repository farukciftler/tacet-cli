import Foundation

// The plain-text engine for .md and .txt.
struct TextEngine: DocumentEngine {
    var format: DocumentFormat

    func writeRaw(fileName: String, title: String?, body: String?, table: Table?, folder: URL) throws -> URL {
        let isMarkdown = format == .md
        var chunks: [String] = []

        // Title: "# " for markdown, a plain line for plain text.
        if let title, !title.isEmpty {
            chunks.append(isMarkdown ? "# \(title)" : title)
        }

        // Table: markdown for md, the summary or the rows for txt.
        if let table {
            if isMarkdown {
                chunks.append(table.markdown)
            } else {
                let summary = table.summary
                if summary.isEmpty {
                    chunks.append(txtTable(table))
                } else {
                    chunks.append(summary)
                }
            }
        }

        // The body text.
        if let body, !body.isEmpty {
            chunks.append(body)
        }

        var content = chunks.joined(separator: "\n\n")
        // Do not produce an empty file: leave at least some mark.
        if content.isEmpty {
            content = title?.isEmpty == false ? title! : " "
        }

        let url = targetURL(fileName: fileName, folder: folder)
        try Data(content.utf8).write(to: url)
        return url
    }

    func read(url: URL) throws -> DocumentBody {
        let content: String
        if let utf8 = try? String(contentsOf: url, encoding: .utf8) {
            content = utf8
        } else {
            content = (try? String(contentsOf: url, encoding: .isoLatin1)) ?? ""
        }
        return DocumentBody(text: content)
    }

    // When there is no summary, dump the rows into plain text separated by TABs.
    private func txtTable(_ table: Table) -> String {
        var lines: [String] = []
        if !table.headers.isEmpty {
            lines.append(table.headers.joined(separator: "\t"))
        }
        for row in table.rows {
            lines.append(row.cells.joined(separator: "\t"))
        }
        return lines.joined(separator: "\n")
    }
}
