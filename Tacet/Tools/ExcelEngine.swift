//
//  ExcelEngine.swift
//  Tacet
//
//  .xlsx (OOXML SpreadsheetML) writing/reading. Pure Swift; no external package, no network.
//  Packaging goes through ZipStore (a STORE zip). Cells are inlineStr; sharedStrings is not used.
//  While writing, a =SUM formula is added below fully numeric columns (spec §7.3.2).
//

import Foundation

struct ExcelEngine: DocumentEngine {
    var format: DocumentFormat { .xlsx }

    // MARK: - Writing

    func writeRaw(fileName: String, title: String?, body: String?, table: Table?, folder: URL) throws -> URL {
        let source = try reduceToTable(title: title, body: body, table: table)
        let headers = source.headers
        let rows = source.rows

        let sheet = sheetXml(headers: headers, rows: rows)

        let entries: [ZipEntry] = [
            ZipEntry(name: "[Content_Types].xml", data: Data(contentTypesXml.utf8)),
            ZipEntry(name: "_rels/.rels", data: Data(relsXml.utf8)),
            ZipEntry(name: Self.signaturePath, data: Data(appXml.utf8)),
            ZipEntry(name: "xl/workbook.xml", data: Data(workbookXml.utf8)),
            ZipEntry(name: "xl/_rels/workbook.xml.rels", data: Data(workbookRelsXml.utf8)),
            ZipEntry(name: "xl/styles.xml", data: Data(stylesXml.utf8)),
            ZipEntry(name: "xl/worksheets/sheet1.xml", data: Data(sheet.utf8)),
        ]

        let url = targetURL(fileName: fileName, folder: folder)
        try ZipStore.package(entries).write(to: url)
        return url
    }

    // MARK: - Reducing the source to a table

    /// The only input xlsx takes is a table. If there is no structured table, one is SALVAGED
    /// from the body; if it cannot be salvaged an EXPLICIT ERROR is thrown.
    ///
    /// There used to be a silent fallback branch here: the body was dumped line by line into a
    /// SINGLE COLUMN. When the model wrote a markdown table ("| Name | Age |") the result was a
    /// garbage xlsx with each line crammed into one cell — and on top of that the tool reported
    /// success, "created" (audit P1-5). Now:
    /// - if the body is a parsable table → it is converted into a real table,
    /// - if the body has pipes but no table comes out → an error (the model fixes it once),
    /// - if the body is a pipe-less plain list → a single column is LEGITIMATE, it is written,
    /// - if there is neither a table nor a body → an error.
    private func reduceToTable(title: String?, body: String?,
                               table: Table?) throws -> (headers: [String], rows: [[String]]) {
        if let t = table, !t.headers.isEmpty {
            if !t.rows.isEmpty { return (t.headers, t.rows.map { $0.cells }) }
            // There are headers but no rows: try to salvage rows from the body; if that fails,
            // a header-row-only sheet is written (the old behavior is preserved).
            if let b = body, let parsed = Table.fromMarkdown(b), !parsed.rows.isEmpty {
                return (parsed.headers, parsed.rows.map { $0.cells })
            }
            return (t.headers, [])
        }

        let text = (body ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { throw Self.noContentError }

        // 1) Is the body a markdown table? (the separator row is optional — see Table)
        if let parsed = Table.fromMarkdown(text), !parsed.rows.isEmpty {
            return (parsed.headers, parsed.rows.map { $0.cells })
        }

        // 2) There are pipes but no table came out → do not silently dump into one column,
        //    MAKE IT FIX THIS.
        if text.contains("|") {
            throw DocumentEngineError.unsupported(
                code: "unparsable_table (a header row and data rows are needed, "
                    + "one row per line, cells separated by |)",
                chip: String(localized: "The table could not be read.")
            )
        }

        // 3) A pipe-less plain list: a single column is a legitimate result.
        let slices = text.components(separatedBy: "\n")
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }
        guard !slices.isEmpty else { throw Self.noContentError }
        return ([title ?? "Text"], slices.map { [$0] })
    }

    /// The "there is nothing to turn into a table" error. It sits in one place: both branches
    /// report the same fact, and letting them drift apart would be meaningless.
    private static var noContentError: DocumentEngineError {
        .unsupported(
            code: "empty_table_input (column headers and data rows are needed)",
            chip: String(localized: "There is no content for the table.")
        )
    }

    // MARK: - Reading

    func read(url: URL) throws -> DocumentBody {
        let zip = try Data(contentsOf: url)
        let entries = try ZipStore.open(zip)
        // The sheet path is not fixed: Numbers/LibreOffice may write "sheet.xml" or
        // "worksheets/sheet2.xml". Resolve it from the relationships, fall back to the constant.
        let sheetPath = firstSheetPath(entries) ?? "xl/worksheets/sheet1.xml"
        guard let sheetData = entries[sheetPath] else {
            throw DocumentEngineError.noContent
        }

        // Index the sharedStrings table if there is one (for t="s").
        var shared: [String] = []
        if let ssData = entries["xl/sharedStrings.xml"] {
            let ssParser = SharedStringsParser()
            let p = XMLParser(data: ssData)
            p.delegate = ssParser
            p.parse()
            shared = ssParser.values
        }

        let sheetParser = SheetParser(shared: shared)
        let parser = XMLParser(data: sheetData)
        parser.delegate = sheetParser
        parser.parse()

        var rawRows = sheetParser.rows
        var withFormula = sheetParser.rowHasFormula
        func dropLastRow() {
            rawRows.removeLast()
            if !withFormula.isEmpty { withFormula.removeLast() }
        }
        // Fully empty trailing rows (common in real files) must not enter the table.
        while let last = rawRows.last, last.allSatisfy({ $0.isEmpty }) { dropLastRow() }
        // The summary row we wrote ourselves IS NOT DATA: it carries a formula + a cached
        // value. Had it been read back and rewritten, the new SUM would have summed the old
        // total too (203.5 → 407). The write path regenerates that row anyway.
        //
        // THE SIGNATURE IS MANDATORY: the description "the last row that contains a formula and
        // whose first cell is Total" is the ORDINARY last row of a budget table. Without
        // checking the signature the user's own total row was silently deleted — you cannot say
        // "this row is mine" without asking who wrote the file.
        if writtenByUs(entries), let last = rawRows.last, withFormula.last == true,
           last.first?.trimmingCharacters(in: .whitespaces) == "Total" {
            dropLastRow()
            while let s = rawRows.last, s.allSatisfy({ $0.isEmpty }) { dropLastRow() }
        }
        guard let first = rawRows.first else { throw DocumentEngineError.noContent }

        // Equalize the column count: the extra cells of a data row wider than the header row
        // used to be dropped in Table.markdown.
        let width = max(first.count, rawRows.map(\.count).max() ?? 0)
        func equalize(_ s: [String]) -> [String] {
            s.count >= width ? Array(s.prefix(width)) : s + Array(repeating: "", count: width - s.count)
        }
        let headers = equalize(first)
        let bodyRows = rawRows.dropFirst().map { Row(cells: equalize($0)) }
        let table = Table(headers: headers, rows: Array(bodyRows))
        return DocumentBody(text: table.summary, table: table)
    }

    /// Did this app write the package? Only the `docProps/app.xml` signature is looked at;
    /// content heuristics (a row label, the presence of a formula) CARRY NO authorship information.
    private func writtenByUs(_ entries: [String: Data]) -> Bool {
        guard let data = entries[Self.signaturePath],
              let text = String(data: data, encoding: .utf8) else { return false }
        return text.contains(Self.signatureStamp)
    }

    /// Resolves the in-package path of the first sheet through `xl/workbook.xml` +
    /// `xl/_rels/workbook.xml.rels`. nil if it cannot be resolved (the caller falls back to the
    /// fixed path).
    private func firstSheetPath(_ entries: [String: Data]) -> String? {
        guard let wbData = entries["xl/workbook.xml"],
              let relsData = entries["xl/_rels/workbook.xml.rels"] else { return nil }

        let workbook = WorkbookParser()
        let p1 = XMLParser(data: wbData)
        p1.delegate = workbook
        p1.parse()

        let relationship = RelationshipParser()
        let p2 = XMLParser(data: relsData)
        p2.delegate = relationship
        p2.parse()

        guard let rid = workbook.firstSheetRId, let target = relationship.targets[rid] else { return nil }
        let path = resolvePackagePath(base: "xl", target: target)
        return entries[path] != nil ? path : nil
    }

    // MARK: - Worksheet generation

    private func sheetXml(headers: [String], rows: [[String]]) -> String {
        let columnCount = headers.count
        // Detect the numeric columns: in the data rows every non-empty cell of that column is a
        // number. The total accumulates here too — it is needed for the SUM formula's cached value.
        var numericColumn = [Bool](repeating: false, count: columnCount)
        var columnTotal = [Double](repeating: 0, count: columnCount)
        for k in 0..<columnCount {
            var atLeastOne = false
            var allNumbers = true
            var total = 0.0
            for s in rows {
                guard k < s.count else { continue }
                let h = s[k].trimmingCharacters(in: .whitespaces)
                if h.isEmpty { continue }
                atLeastOne = true
                guard isNumeric(h), let d = Double(h) else { allNumbers = false; break }
                total += d
            }
            numericColumn[k] = atLeastOne && allNumbers
            columnTotal[k] = total
        }
        let hasNumeric = numericColumn.contains(true)

        var body = ""

        // Row 1: the headers (all inlineStr).
        body += "<row r=\"1\">"
        for (k, b) in headers.enumerated() {
            body += inlineCell(ref: "\(columnLetter(k))1", text: b)
        }
        body += "</row>"

        // The data rows.
        for (idx, s) in rows.enumerated() {
            let r = idx + 2
            body += "<row r=\"\(r)\">"
            for k in 0..<columnCount {
                let ref = "\(columnLetter(k))\(r)"
                let value = k < s.count ? s[k] : ""
                let trimmed = value.trimmingCharacters(in: .whitespaces)
                if !trimmed.isEmpty, isNumeric(trimmed) {
                    // A leading "+" trips some readers; the number renders the same anyway.
                    let number = trimmed.hasPrefix("+") ? String(trimmed.dropFirst()) : trimmed
                    body += "<c r=\"\(ref)\"><v>\(number)</v></c>"
                } else {
                    body += inlineCell(ref: ref, text: value)
                }
            }
            body += "</row>"
        }

        // The summary (Total) row: only if there is at least one numeric column AND at least
        // two columns.
        //
        // With a single column there IS NO label column: the loop only sees k == 0, "Total" is
        // written there and no `<f>` is produced at all. The read path took the formula-less row
        // for DATA and pulled it into the table, and on the second write that column was no
        // longer all-numeric so the total disappeared too — the round trip corrupted the table.
        if hasNumeric, !rows.isEmpty, columnCount >= 2 {
            let firstData = 2
            let lastData = rows.count + 1
            let totalRow = lastData + 1
            body += "<row r=\"\(totalRow)\">"
            for k in 0..<columnCount {
                let ref = "\(columnLetter(k))\(totalRow)"
                if k == 0 {
                    body += inlineCell(ref: ref, text: "Total")
                } else if numericColumn[k] {
                    let letter = columnLetter(k)
                    // The cached <v> is mandatory: QuickLook does not evaluate formulas, the
                    // cell would look empty and the user would think the file is broken.
                    let cache = numberText(columnTotal[k]).map { "<v>\($0)</v>" } ?? ""
                    body += "<c r=\"\(ref)\"><f>SUM(\(letter)\(firstData):\(letter)\(lastData))</f>\(cache)</c>"
                } else {
                    body += inlineCell(ref: ref, text: "")
                }
            }
            body += "</row>"
        }

        return """
        <?xml version="1.0" encoding="UTF-8" standalone="yes"?>
        <worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>\(body)</sheetData></worksheet>
        """
    }

    private func inlineCell(ref: String, text: String) -> String {
        "<c r=\"\(ref)\" t=\"inlineStr\"><is><t xml:space=\"preserve\">\(OoxmlEscape.escape(text))</t></is></c>"
    }

    // MARK: - Fixed parts

    private let contentTypesXml = """
    <?xml version="1.0" encoding="UTF-8" standalone="yes"?>
    <Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
    <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
    <Default Extension="xml" ContentType="application/xml"/>
    <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
    <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
    <Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
    <Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>
    </Types>
    """

    private let relsXml = """
    <?xml version="1.0" encoding="UTF-8" standalone="yes"?>
    <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
    <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
    <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/>
    </Relationships>
    """

    /// The signature part that says WE wrote the file. It is the standard OOXML
    /// "extended properties" part — ordinary for Excel/Numbers/QuickLook, and for us the only
    /// proof of "the summary row belongs to us".
    static let signaturePath = "docProps/app.xml"
    static let signatureStamp = "<Application>Tacet</Application>"

    private let appXml = """
    <?xml version="1.0" encoding="UTF-8" standalone="yes"?>
    <Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"\
     xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes">\
    \(ExcelEngine.signatureStamp)</Properties>
    """

    private let workbookXml = """
    <?xml version="1.0" encoding="UTF-8" standalone="yes"?>
    <workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
    <sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>
    """

    private let workbookRelsXml = """
    <?xml version="1.0" encoding="UTF-8" standalone="yes"?>
    <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
    <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
    <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
    </Relationships>
    """

    private let stylesXml = """
    <?xml version="1.0" encoding="UTF-8" standalone="yes"?>
    <styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
    <fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts>
    <fills count="1"><fill><patternFill patternType="none"/></fill></fills>
    <borders count="1"><border/></borders>
    <cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
    <cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs>
    </styleSheet>
    """
}

// MARK: - Helpers

/// Converts a (0-based) column index into a letter: 0→A, 25→Z, 26→AA.
fileprivate func columnLetter(_ index: Int) -> String {
    var n = index
    var s = ""
    repeat {
        let r = n % 26
        s = String(UnicodeScalar(UInt8(65 + r))) + s
        n = n / 26 - 1
    } while n >= 0
    return s
}

/// Converts a column letter into an index — the inverse of `columnLetter`: A→0, Z→25, AA→26.
/// The input may be a full cell reference ("C2", "$AB$14"); it stops as soon as it sees a digit.
fileprivate func columnIndex(_ ref: String) -> Int? {
    var n = 0
    var hasLetter = false
    for u in ref.unicodeScalars {
        switch u {
        case "$":
            continue
        case "A"..."Z":
            n = n * 26 + Int(u.value - 64)
            hasLetter = true
        case "a"..."z":
            n = n * 26 + Int(u.value - 96)
            hasLetter = true
        case "0"..."9":
            return hasLetter ? n - 1 : nil
        default:
            return nil
        }
        guard n <= 16_384 else { return nil }   // Excel's column cap (XFD)
    }
    return hasLetter ? n - 1 : nil
}

/// Can it be written into the cell as `<v>`? `Double(...)` is too permissive:
/// it accepts "nan"/"inf"/"0x1p2" (Excel considers the file unrepairable) and it turns
/// identifiers like "007" into a number, losing the leading zeros.
/// Accepted: `^[+-]?\d+(\.\d+)?$`, with no leading zero and no more than 15 digits.
fileprivate func isNumeric(_ text: String) -> Bool {
    var s = text[...]
    if s.hasPrefix("+") || s.hasPrefix("-") { s = s.dropFirst() }
    let parts = s.split(separator: ".", omittingEmptySubsequences: false)
    guard parts.count == 1 || parts.count == 2 else { return false }

    let whole = parts[0]
    guard !whole.isEmpty, whole.allSatisfy({ $0.isASCII && $0.isNumber }) else { return false }
    // Identifier/phone fields like "007", "0532…" must stay text (except "0").
    if whole.count > 1, whole.first == "0" { return false }
    // Numbers beyond Excel's 15-digit precision are silently rounded.
    guard whole.count <= 15 else { return false }

    if parts.count == 2 {
        let fraction = parts[1]
        guard !fraction.isEmpty, fraction.allSatisfy({ $0.isASCII && $0.isNumber }) else { return false }
    }
    return true
}

/// Converts a Double into the plain decimal notation Excel accepts (no exponent, no NaN).
/// nil if it is not finite — in that case no cached value is written at all.
fileprivate func numberText(_ v: Double) -> String? {
    guard v.isFinite else { return nil }
    if v == v.rounded(), abs(v) < 1e15 { return String(Int64(v)) }
    let raw = String(format: "%.10f", v)
    var s = raw[...]
    while s.hasSuffix("0") { s = s.dropLast() }
    if s.hasSuffix(".") { s = s.dropLast() }
    return String(s)
}

/// Drops the namespace prefix: some producers write `<x:row>`/`<x:c>`, and because XMLParser's
/// namespace processing is off, the element name arrives with its prefix.
fileprivate func localName(_ element: String) -> String {
    guard let i = element.lastIndex(of: ":") else { return element }
    return String(element[element.index(after: i)...])
}

/// Turns an OPC relative path into an absolute in-package path ("worksheets/s.xml" → "xl/worksheets/s.xml").
fileprivate func resolvePackagePath(base: String, target: String) -> String {
    if target.hasPrefix("/") { return String(target.dropFirst()) }
    var parts = base.split(separator: "/").map(String.init)
    for p in target.split(separator: "/") {
        if p == "." { continue }
        if p == ".." { if !parts.isEmpty { parts.removeLast() }; continue }
        parts.append(String(p))
    }
    return parts.joined(separator: "/")
}

// MARK: - Parsers

/// Collects the row/cell values in the sheet XML.
///
/// Cells are placed at the column from their `r` reference, NOT in arrival order: Excel does not
/// write an empty cell at all (if A2 is empty the row starts with `<c r="B2">`), and lining them
/// up in order shifted every value one column to the left — silent data corruption in the
/// read_document → edit_document chain.
fileprivate final class SheetParser: NSObject, XMLParserDelegate {
    let shared: [String]
    var rows: [[String]] = []
    /// In the same order as `rows`: did the row contain a computed (`<f>`) cell?
    var rowHasFormula: [Bool] = []

    /// The upper bound while filling in the empty rows in between: on a sparse sheet (r="1" and
    /// r="100000") we must not generate millions of empty rows.
    private let maxEmptyRows = 1024

    private var activeRow: [String] = []
    private var activeCell = ""
    private var cellKind: String?
    private var cellColumn: Int?        // the 0-based column resolved from `r`
    private var nextColumn = 0          // arrival order when there is no `r`
    private var rowNo: Int?             // <row r="N">
    private var expectedRow: Int?       // the expected number of the next row
    private var formulaInRow = false
    private var collecting = false

    init(shared: [String]) {
        self.shared = shared
    }

    func parser(_ parser: XMLParser, didStartElement elementName: String,
                namespaceURI: String?, qualifiedName qName: String?,
                attributes attributeDict: [String: String]) {
        switch localName(elementName) {
        case "row":
            activeRow = []
            nextColumn = 0
            formulaInRow = false
            rowNo = attributeDict["r"].flatMap { Int($0) }
        case "f":
            formulaInRow = true
        case "c":
            cellKind = attributeDict["t"]
            cellColumn = attributeDict["r"].flatMap { columnIndex($0) }
            activeCell = ""
        case "t", "v":
            collecting = true
        default:
            break
        }
    }

    func parser(_ parser: XMLParser, foundCharacters string: String) {
        if collecting { activeCell += string }
    }

    func parser(_ parser: XMLParser, didEndElement elementName: String,
                namespaceURI: String?, qualifiedName qName: String?) {
        switch localName(elementName) {
        case "t", "v":
            collecting = false
        case "c":
            let value: String
            if cellKind == "s", let i = Int(activeCell.trimmingCharacters(in: .whitespaces)),
               i >= 0, i < shared.count {
                value = shared[i]
            } else {
                value = activeCell
            }
            place(value, column: cellColumn ?? nextColumn)
        case "row":
            finishRow()
        default:
            break
        }
    }

    /// Puts the cell into its own column; fills the missing columns in between with blanks.
    private func place(_ value: String, column: Int) {
        guard column >= 0 else { return }
        while activeRow.count < column { activeRow.append("") }
        if column < activeRow.count {
            activeRow[column] = value        // if the same column was written twice, the last one wins
        } else {
            activeRow.append(value)
        }
        nextColumn = column + 1
    }

    private func finishRow() {
        if let n = rowNo, let exp = expectedRow, n > exp, n - exp <= maxEmptyRows {
            // Fully empty rows are never written to the file; put the gap back.
            for _ in exp..<n { rows.append([]); rowHasFormula.append(false) }
        }
        rows.append(activeRow)
        rowHasFormula.append(formulaInRow)
        // A gap before the first row is not invented: the header row must not shift.
        expectedRow = (rowNo ?? expectedRow ?? 1) + 1
        rowNo = nil
        activeRow = []
    }
}

/// The relationship id of the first `<sheet>` in `xl/workbook.xml`.
fileprivate final class WorkbookParser: NSObject, XMLParserDelegate {
    var firstSheetRId: String?

    func parser(_ parser: XMLParser, didStartElement elementName: String,
                namespaceURI: String?, qualifiedName qName: String?,
                attributes attributeDict: [String: String]) {
        guard firstSheetRId == nil, localName(elementName) == "sheet" else { return }
        // The prefix does not have to be "r": look for the attribute whose local name is "id".
        firstSheetRId = attributeDict.first { localName($0.key) == "id" }?.value
    }
}

/// The Id → Target mapping in a `.rels` part.
fileprivate final class RelationshipParser: NSObject, XMLParserDelegate {
    var targets: [String: String] = [:]

    func parser(_ parser: XMLParser, didStartElement elementName: String,
                namespaceURI: String?, qualifiedName qName: String?,
                attributes attributeDict: [String: String]) {
        guard localName(elementName) == "Relationship" else { return }
        if let id = attributeDict["Id"], let target = attributeDict["Target"] {
            targets[id] = target
        }
    }
}

/// Collects the <si> values in sharedStrings.xml (for t="s" cells).
fileprivate final class SharedStringsParser: NSObject, XMLParserDelegate {
    var values: [String] = []
    private var active = ""
    private var insideSI = false
    private var collecting = false

    func parser(_ parser: XMLParser, didStartElement elementName: String,
                namespaceURI: String?, qualifiedName qName: String?,
                attributes attributeDict: [String: String]) {
        switch localName(elementName) {
        case "si":
            insideSI = true
            active = ""
        case "t":
            collecting = true
        default:
            break
        }
    }

    func parser(_ parser: XMLParser, foundCharacters string: String) {
        if insideSI, collecting { active += string }
    }

    func parser(_ parser: XMLParser, didEndElement elementName: String,
                namespaceURI: String?, qualifiedName qName: String?) {
        switch localName(elementName) {
        case "t":
            collecting = false
        case "si":
            values.append(active)
            insideSI = false
        default:
            break
        }
    }
}
