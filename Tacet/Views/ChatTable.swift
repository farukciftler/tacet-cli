//
//  ChatTable.swift
//  Tacet
//
//  Table display inside the chat (like Gemini/Claude). A markdown table in a Tacet
//  reply is rendered as a plain table between the paragraphs; underneath it, the
//  "Download Excel" button produces the same table as .xlsx and previews/shares it.
//

import SwiftUI

struct ChatTable: View {
    let table: Table
    /// The Excel download request — the parent view produces the file and previews it.
    var download: (Table) -> Void = { _ in }
    /// Turned off in the document preview: that file is already being looked at, and
    /// "Download Excel" would offer to produce the same file again. In the chat it
    /// defaults to on.
    var showDownload: Bool = true

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.s2) {
            Grid(alignment: .leading, horizontalSpacing: Spacing.hairline, verticalSpacing: Spacing.hairline) {
                // The header row.
                GridRow {
                    ForEach(Array(table.headers.enumerated()), id: \.offset) { i, h in
                        cell(h, column: h, rowNumber: nil, columnNumber: i, isHeader: true)
                    }
                }
                // The data rows.
                ForEach(Array(table.rows.enumerated()), id: \.offset) { r, row in
                    GridRow {
                        ForEach(Array(table.headers.enumerated()), id: \.offset) { i, h in
                            cell(i < row.cells.count ? row.cells[i] : "",
                                 column: h, rowNumber: r + 1, columnNumber: i, isHeader: false)
                        }
                    }
                }
            }
            .background(Palette.divider)   // hairline grid between cells
            .clipShape(RoundedRectangle(cornerRadius: 10))
            .overlay(
                RoundedRectangle(cornerRadius: 10)
                    .stroke(Palette.divider, lineWidth: Spacing.hairline)
            )
            // The table's boundary and size must not be lost in a screen reader.
            .accessibilityElement(children: .contain)
            .accessibilityLabel(Text("Table, \(table.headers.count) columns, \(table.rows.count) rows"))

            // The Excel download button.
            if showDownload {
                Button {
                    download(table)
                } label: {
                    HStack(spacing: Spacing.s1) {
                        Image(systemName: "arrow.down.circle")
                        Text("Download Excel")
                    }
                    .font(Typography.chip())
                    .foregroundStyle(Palette.grey)
                    .padding(.horizontal, Spacing.s3)
                    .padding(.vertical, Spacing.s2)
                    .overlay(Capsule().stroke(Palette.divider, lineWidth: Spacing.hairline))
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Download table as Excel")
            }
        }
    }

    /// Visually a cell is only its value; in a screen reader it is read together with the
    /// column header and its position context — otherwise the table structure is lost
    /// entirely.
    private func cell(_ text: String,
                      column: String,
                      rowNumber: Int?,
                      columnNumber: Int,
                      isHeader: Bool) -> some View {
        Text(text)
            .font(isHeader ? Typography.chip().weight(.medium) : Typography.chip())
            .foregroundStyle(isHeader ? Palette.ink : Palette.grey)
            .padding(.horizontal, Spacing.s3)
            .padding(.vertical, Spacing.s2)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(isHeader ? Palette.fill : Palette.background)
            .accessibilityLabel(cellLabel(text, column: column, rowNumber: rowNumber,
                                          columnNumber: columnNumber, isHeader: isHeader))
    }

    private func cellLabel(_ text: String,
                           column: String,
                           rowNumber: Int?,
                           columnNumber: Int,
                           isHeader: Bool) -> Text {
        let columnName = column.isEmpty ? String(localized: "column \(columnNumber + 1)") : column
        guard let rowNumber else {
            return Text("Column header: \(columnName)")
        }
        let value = text.isEmpty ? String(localized: "empty") : text
        return Text("row \(rowNumber), \(columnName): \(value)")
    }
}
