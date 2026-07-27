//
//  HtmlEngine.swift
//  Tacet
//
//  The single-file web page engine (code-spec §4). The model only produces markdown content;
//  the HTML skeleton and the style are embedded here — the model never sees the template, and
//  the 4096 window stays available for content. The page is self-sufficient: NO external
//  font/CSS/JS request (the page carries the network promise too). The markdown mapping:
//  the first "# " → the hero section + the paragraph right below it as the tagline,
//  "## " → a section, a table → <table>, a "- " list → a card grid, a plain line → <p>.
//  `read` strips the tags and returns plain text close to markdown — so the
//  read_document → edit_document chain works for free. No network.
//
//  NOTE: the CSS class names (`kart`, `kartlar`, `tablo-kap`) and the custom properties
//  (`--zemin`, `--murekkep`, `--gri`, `--cizgi`, `--dolgu`) are deliberately LEFT UNCHANGED in
//  this pass: they are not in the shared glossary, and `Servis/OtoTest.swift` asserts on the
//  literal string `class="kartlar"`. Renaming them requires changing BOTH sides at once.
//

import Foundation

/// Escapes the HTML special characters (&, <, >). Order matters: & first.
fileprivate func htmlEscape(_ s: String) -> String {
    var r = s
    r = r.replacingOccurrences(of: "&", with: "&amp;")
    r = r.replacingOccurrences(of: "<", with: "&lt;")
    r = r.replacingOccurrences(of: ">", with: "&gt;")
    return r
}

struct HtmlEngine: DocumentEngine {
    var format: DocumentFormat { .html }

    // MARK: - Writing

    func writeRaw(fileName: String, title: String?, body: String?, table: Table?, folder: URL) throws -> URL {
        // Collect the body: the markdown text + the markdown of the structured table if there is one.
        var chunks: [String] = []
        if let body, !body.isEmpty { chunks.append(body) }
        if let table {
            let mt = table.markdown
            if !mt.isEmpty { chunks.append(mt) }
        }
        let markdown = chunks.joined(separator: "\n\n")

        let page = Self.setupPage(title: title, markdown: markdown, fallbackTitle: fileName)
        let url = targetURL(fileName: fileName, folder: folder)
        try Data(page.utf8).write(to: url)
        return url
    }

    // MARK: - Markdown → HTML

    /// Builds the whole page: hero + sections + embedded style. One file, no requests.
    private static func setupPage(title: String?, markdown: String, fallbackTitle: String) -> String {
        let lines = markdown
            .components(separatedBy: "\n")
            .map { $0.trimmingCharacters(in: .whitespaces) }

        // Hero: the first "# " heading; if there is none, the title parameter.
        var heroTitle = (title?.trimmingCharacters(in: .whitespaces)).flatMap { $0.isEmpty ? nil : $0 } ?? ""
        var heroTagline: String?
        var heroFound = false

        var sections = ""       // the completed <section> blocks
        var activeTitle = ""    // the active section's <h2>
        var activeBody = ""     // the active section's blocks

        func closeSection() {
            if !activeTitle.isEmpty || !activeBody.isEmpty {
                sections += "<section>\n\(activeTitle)\(activeBody)</section>\n"
            }
            activeTitle = ""
            activeBody = ""
        }

        var i = 0
        while i < lines.count {
            let s = lines[i]
            if s.isEmpty { i += 1; continue }

            // The first "# " → the hero title; the plain paragraph right below it becomes the tagline.
            if s.hasPrefix("# "), !heroFound {
                heroFound = true
                heroTitle = String(s.dropFirst(2))
                var j = i + 1
                while j < lines.count, lines[j].isEmpty { j += 1 }
                if j < lines.count {
                    let candidate = lines[j]
                    if !candidate.hasPrefix("#"), !candidate.hasPrefix("- "), !candidate.hasPrefix("|") {
                        heroTagline = candidate
                        i = j + 1
                        continue
                    }
                }
                i += 1
                continue
            }

            // "## " (and any extra "# ") → a new section heading.
            if s.hasPrefix("## ") || s.hasPrefix("# ") {
                closeSection()
                let text = s.hasPrefix("## ") ? String(s.dropFirst(3)) : String(s.dropFirst(2))
                activeTitle = "<h2>\(inline(text))</h2>\n"
                i += 1
                continue
            }

            // "### " → a subheading inside the section.
            if s.hasPrefix("### ") {
                activeBody += "<h3>\(inline(String(s.dropFirst(4))))</h3>\n"
                i += 1
                continue
            }

            // A "- " list → a card grid (consecutive items become one grid).
            if s.hasPrefix("- ") {
                var items: [String] = []
                while i < lines.count, lines[i].hasPrefix("- ") {
                    items.append(String(lines[i].dropFirst(2)))
                    i += 1
                }
                activeBody += cardGrid(items)
                continue
            }

            // A "|" block → a table; if it cannot be parsed, the lines fall through as paragraphs.
            if s.hasPrefix("|") {
                var block: [String] = []
                while i < lines.count, lines[i].hasPrefix("|") {
                    block.append(lines[i])
                    i += 1
                }
                if let t = Table.fromMarkdown(block.joined(separator: "\n")) {
                    activeBody += tableHTML(t)
                } else {
                    for b in block { activeBody += "<p>\(inline(b))</p>\n" }
                }
                continue
            }

            // A plain paragraph.
            activeBody += "<p>\(inline(s))</p>\n"
            i += 1
        }
        closeSection()

        // The hero block (if there is a title).
        var hero = ""
        if !heroTitle.isEmpty {
            hero = "<header class=\"hero\">\n<h1>\(inline(heroTitle))</h1>\n"
            if let tag = heroTagline {
                hero += "<p class=\"tagline\">\(inline(tag))</p>\n"
            }
            hero += "</header>\n"
        }

        let pageTitle = heroTitle.isEmpty ? fallbackTitle : heroTitle
        return """
        <!doctype html>
        <html>
        <head>
        <meta charset="utf-8">
        <meta name="viewport" content="width=device-width, initial-scale=1">
        <title>\(htmlEscape(pageTitle))</title>
        <style>
        \(style)
        </style>
        </head>
        <body>
        \(hero)<main>
        \(sections)</main>
        </body>
        </html>
        """
    }

    /// Inline formatting: escaping + **bold** → <strong>.
    private static func inline(_ raw: String) -> String {
        var m = htmlEscape(raw)
        while let r = m.range(of: #"\*\*([^*]+)\*\*"#, options: .regularExpression) {
            let inner = String(m[r]).dropFirst(2).dropLast(2)
            m.replaceSubrange(r, with: "<strong>\(inner)</strong>")
        }
        return m
    }

    /// A markdown table → a responsive <table> (inside a horizontal-overflow container).
    private static func tableHTML(_ t: Table) -> String {
        var h = "<div class=\"tablo-kap\"><table>\n<thead><tr>"
        for b in t.headers { h += "<th>\(htmlEscape(b))</th>" }
        h += "</tr></thead>\n<tbody>\n"
        for s in t.rows {
            h += "<tr>"
            for i in 0..<t.headers.count {
                let cell = i < s.cells.count ? s.cells[i] : ""
                h += "<td>\(htmlEscape(cell))</td>"
            }
            h += "</tr>\n"
        }
        h += "</tbody></table></div>\n"
        return h
    }

    /// "- " items → a card grid.
    private static func cardGrid(_ items: [String]) -> String {
        var h = "<div class=\"kartlar\">\n"
        for item in items {
            h += "<div class=\"kart\"><p>\(inline(item))</p></div>\n"
        }
        h += "</div>\n"
        return h
    }

    /// The embedded style — the Tacet design language (the same colors as Theme.swift):
    /// serif headings, ink/gray tones, light/dark aware, responsive.
    /// NO external URL (SelfTest looks for that).
    private static let style = """
    :root {
      --zemin: #FFFFFF; --murekkep: #1C1C1A; --gri: #8A8A84;
      --cizgi: #E9E9E4; --dolgu: #F4F4F1;
      --serif: 'New York', Georgia, 'Times New Roman', serif;
      --sans: -apple-system, 'Helvetica Neue', Arial, sans-serif;
    }
    @media (prefers-color-scheme: dark) {
      :root {
        --zemin: #141413; --murekkep: #ECECEA; --gri: #9A9A93;
        --cizgi: #2A2A28; --dolgu: #222220;
      }
    }
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body {
      background: var(--zemin); color: var(--murekkep);
      font-family: var(--sans); line-height: 1.6;
      -webkit-text-size-adjust: 100%;
      padding-bottom: 4rem;
    }
    .hero, main { max-width: 42rem; margin: 0 auto; padding: 0 1.25rem; }
    .hero { padding: 4.5rem 1.25rem 3rem; text-align: center; }
    .hero h1 {
      font-family: var(--serif); font-weight: 500;
      font-size: clamp(2rem, 6vw, 3rem); line-height: 1.15;
    }
    .hero .tagline { margin-top: 1rem; color: var(--gri); font-size: 1.1rem; }
    section { padding: 2rem 0; border-top: 1px solid var(--cizgi); }
    main > section:first-child { border-top: 0; }
    h2 { font-family: var(--serif); font-weight: 500; font-size: 1.5rem; margin-bottom: 1rem; }
    h3 { font-family: var(--serif); font-weight: 500; font-size: 1.15rem; margin: 1rem 0 0.5rem; }
    p { margin: 0.75rem 0; }
    .kartlar {
      display: grid; gap: 0.75rem; margin: 1rem 0;
      grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr));
    }
    .kart {
      background: var(--dolgu); border: 1px solid var(--cizgi);
      border-radius: 0.75rem; padding: 1rem;
    }
    .kart p { margin: 0; }
    .tablo-kap { overflow-x: auto; margin: 1rem 0; }
    table { border-collapse: collapse; width: 100%; font-size: 0.95rem; }
    th, td { text-align: left; padding: 0.5rem 0.75rem; border-bottom: 1px solid var(--cizgi); }
    th {
      color: var(--gri); font-weight: 600; font-size: 0.8rem;
      text-transform: uppercase; letter-spacing: 0.05em;
    }
    """

    // MARK: - Reading (HTML → plain text)

    func read(url: URL) throws -> DocumentBody {
        let raw: String
        if let utf8 = try? String(contentsOf: url, encoding: .utf8) {
            raw = utf8
        } else if let latin = try? String(contentsOf: url, encoding: .isoLatin1) {
            raw = latin
        } else {
            throw DocumentEngineError.noContent
        }
        return DocumentBody(text: Self.extractToText(raw))
    }

    /// Strips the tags and turns the structure back into markdown prefixes (h1 → "# ",
    /// card → "- ", <table> → a markdown table). That is how the "add a section to the site"
    /// flow works through the read_document → edit_document chain.
    static func extractToText(_ raw: String) -> String {
        var m = raw
        // The head section (style + title) and any scripts are dropped entirely.
        m = replace(m, "<head[\\s\\S]*?</head>", "")
        m = replace(m, "<style[\\s\\S]*?</style>", "")
        m = replace(m, "<script[\\s\\S]*?</script>", "")
        // Tables are converted to markdown (including the separator row).
        m = convertTableBack(m)
        // Structural tags → markdown prefixes. The card must be handled BEFORE the generic <p>.
        m = replace(m, "<div class=\"kart\">\\s*<p[^>]*>", "\n- ")
        m = replace(m, "<h1[^>]*>", "\n# ")
        m = replace(m, "<h2[^>]*>", "\n## ")
        m = replace(m, "<h3[^>]*>", "\n### ")
        m = replace(m, "</h[1-6]>", "\n")
        m = replace(m, "<p(\\s[^>]*)?>", "\n")
        m = replace(m, "</p>", "\n")
        m = replace(m, "<br\\s*/?>", "\n")
        m = replace(m, "<li(\\s[^>]*)?>", "\n- ")
        // Every remaining tag is deleted.
        m = replace(m, "<[^>]+>", "")
        // The escapes are undone (& last).
        m = m.replacingOccurrences(of: "&lt;", with: "<")
        m = m.replacingOccurrences(of: "&gt;", with: ">")
        m = m.replacingOccurrences(of: "&quot;", with: "\"")
        m = m.replacingOccurrences(of: "&#39;", with: "'")
        m = m.replacingOccurrences(of: "&apos;", with: "'")
        m = m.replacingOccurrences(of: "&amp;", with: "&")
        // Tidy up the whitespace: reduce 3+ blank lines to 1 blank line.
        m = replace(m, "[ \\t]+\\n", "\n")
        m = replace(m, "\\n{3,}", "\n\n")
        return m.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// Converts <table> blocks into markdown tables.
    private static func convertTableBack(_ raw: String) -> String {
        guard let tablePattern = try? NSRegularExpression(
            pattern: "<table[\\s\\S]*?</table>", options: [.caseInsensitive]
        ) else { return raw }
        var m = raw
        while let match = tablePattern.firstMatch(in: m, range: NSRange(m.startIndex..., in: m)),
              let range = Range(match.range, in: m) {
            let block = String(m[range])
            m.replaceSubrange(range, with: tableMarkdown(block))
        }
        return m
    }

    /// A single <table> block → markdown lines (the first row is the header + separator).
    private static func tableMarkdown(_ block: String) -> String {
        guard let rowPattern = try? NSRegularExpression(
                  pattern: "<tr[\\s\\S]*?</tr>", options: [.caseInsensitive]),
              let cellPattern = try? NSRegularExpression(
                  pattern: "<t[hd][^>]*>([\\s\\S]*?)</t[hd]>", options: [.caseInsensitive])
        else { return "" }

        var rows: [[String]] = []
        for match in rowPattern.matches(in: block, range: NSRange(block.startIndex..., in: block)) {
            guard let range = Range(match.range, in: block) else { continue }
            let row = String(block[range])
            var cells: [String] = []
            for h in cellPattern.matches(in: row, range: NSRange(row.startIndex..., in: row)) {
                guard let cellRange = Range(h.range(at: 1), in: row) else { continue }
                let inner = replace(String(row[cellRange]), "<[^>]+>", "")
                cells.append(inner.trimmingCharacters(in: .whitespacesAndNewlines))
            }
            if !cells.isEmpty { rows.append(cells) }
        }
        guard let headers = rows.first else { return "" }

        var md = "\n| " + headers.joined(separator: " | ") + " |\n"
        md += "| " + headers.map { _ in "---" }.joined(separator: " | ") + " |\n"
        for s in rows.dropFirst() {
            md += "| " + s.joined(separator: " | ") + " |\n"
        }
        return md
    }

    /// A regex replacement helper (if the pattern is invalid the text is returned untouched).
    private static func replace(_ text: String, _ pattern: String, _ with: String) -> String {
        guard let r = try? NSRegularExpression(pattern: pattern, options: [.caseInsensitive]) else {
            return text
        }
        return r.stringByReplacingMatches(
            in: text, range: NSRange(text.startIndex..., in: text), withTemplate: with
        )
    }
}
