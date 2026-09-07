---
name: create-document
triggers: excel, xlsx, pdf, word, docx, create a file, make a file, save a file, write a file, new document, file named, save a new note, write a report, create a report, prepare a report, as a report, export, dump, markdown, spreadsheet, make a table, oluştur, belge oluştur, dosya oluştur, md adıyla, txt adıyla, xlsx adıyla, tablo yap, rapor hazırla, excel dosyası
tools: create_document
---
# Creating documents

`create_document({"format":"markdown","file_name":"july-notes","content":"..."})`.
`file_name` is short, hyphenated, NO extension; `content` is MARKDOWN.
`format`: data/plan/budget -> "excel" (`content` MUST be a markdown table);
prose/report -> "markdown"; note -> "text". Only those three. Asked for PDF,
Word or docx: pick the closest and say which one you produced.

## Table shape, EXAMPLE ONLY
| Day | Lunch |
| --- | --- |
| Monday | Lentils |

- The table above is a FORMAT example. Never copy its rows.
- Bulk data another tool read: pass the `source_ref` it returned, do not retype.
- Numeric cells are plain numbers ("1500"); never compute totals yourself.
<!--/core-->
## Rules
- Write document content in the user's language unless the user asks otherwise.

## Your reply
Keep it minimal: "Done." Do NOT repeat the file name; the chip already shows it.
