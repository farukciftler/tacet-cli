//
//  DocxEngine.swift
//  Tacet
//
//  WordprocessingML (.docx) writing/reading. The document is an OOXML zip package; it is
//  produced/read with ZipStore, with no external package and no network. No network.
//

import Foundation

struct DocxEngine: DocumentEngine {
    var format: DocumentFormat { .docx }

    func writeRaw(fileName: String, title: String?, body: String?, table: Table?, folder: URL) throws -> URL {
        let url = targetURL(fileName: fileName, folder: folder)

        var paragraphs = ""

        // If a title was given, make the first paragraph the title (bold).
        if let t = title, !t.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            paragraphs += titleParagraph(t)
        }

        // The body may be markdown: split it into lines and process them.
        if let b = body {
            for line in b.components(separatedBy: "\n") {
                let trimmed = line.trimmingCharacters(in: .whitespaces)
                if trimmed.isEmpty { continue }        // skip empty lines
                if trimmed.hasPrefix("# ") {
                    let text = String(trimmed.dropFirst(2))
                    paragraphs += titleParagraph(text)
                } else {
                    paragraphs += normalParagraph(trimmed)
                }
            }
        }

        // If a table was given (rare): turn each row into a "h1 | h2 | ..." paragraph.
        if let t = table {
            if !t.headers.isEmpty {
                paragraphs += normalParagraph(t.headers.joined(separator: " | "))
            }
            for r in t.rows {
                paragraphs += normalParagraph(r.cells.joined(separator: " | "))
            }
        }

        // Make sure there is at least one paragraph (for a valid document).
        if paragraphs.isEmpty {
            paragraphs += normalParagraph("")
        }

        let documentXml = """
        <?xml version="1.0" encoding="UTF-8" standalone="yes"?>
        <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
        \(paragraphs)<w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
        </w:body></w:document>
        """

        let contentTypes = """
        <?xml version="1.0" encoding="UTF-8" standalone="yes"?>
        <Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
        <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
        <Default Extension="xml" ContentType="application/xml"/>
        <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
        </Types>
        """

        let rels = """
        <?xml version="1.0" encoding="UTF-8" standalone="yes"?>
        <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
        <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
        </Relationships>
        """

        let entries = [
            ZipEntry(name: "[Content_Types].xml", data: Data(contentTypes.utf8)),
            ZipEntry(name: "_rels/.rels", data: Data(rels.utf8)),
            ZipEntry(name: "word/document.xml", data: Data(documentXml.utf8))
        ]

        let data = ZipStore.package(entries)
        try data.write(to: url)
        return url
    }

    func read(url: URL) throws -> DocumentBody {
        let zip = try Data(contentsOf: url)
        let parts = try ZipStore.open(zip)
        guard let docData = parts["word/document.xml"] else {
            throw DocumentEngineError.noContent
        }
        let splitter = DocxSplitter()
        let text = splitter.parse(docData)
        return DocumentBody(text: text)
    }

    // MARK: - Paragraph producers

    private func normalParagraph(_ text: String) -> String {
        "<w:p><w:r><w:t xml:space=\"preserve\">\(OoxmlEscape.escape(text))</w:t></w:r></w:p>\n"
    }

    private func titleParagraph(_ text: String) -> String {
        "<w:p><w:r><w:rPr><w:b/><w:sz w:val=\"28\"/></w:rPr><w:t xml:space=\"preserve\">\(OoxmlEscape.escape(text))</w:t></w:r></w:p>\n"
    }
}

/// Collects the <w:t> texts inside word/document.xml and splits them at <w:p> boundaries.
fileprivate final class DocxSplitter: NSObject, XMLParserDelegate {
    private var paragraphs: [String] = []
    private var current = ""
    private var insideWT = false

    func parse(_ data: Data) -> String {
        let parser = XMLParser(data: data)
        parser.shouldProcessNamespaces = false
        parser.delegate = self
        parser.parse()
        return paragraphs.joined(separator: "\n")
    }

    func parser(_ parser: XMLParser, didStartElement elementName: String,
                namespaceURI: String?, qualifiedName qName: String?,
                attributes attributeDict: [String: String]) {
        if elementName == "w:p" {
            current = ""
        } else if elementName == "w:t" {
            insideWT = true
        }
    }

    func parser(_ parser: XMLParser, foundCharacters string: String) {
        if insideWT { current += string }
    }

    func parser(_ parser: XMLParser, didEndElement elementName: String,
                namespaceURI: String?, qualifiedName qName: String?) {
        if elementName == "w:t" {
            insideWT = false
        } else if elementName == "w:p" {
            paragraphs.append(current)
            current = ""
        }
    }
}
