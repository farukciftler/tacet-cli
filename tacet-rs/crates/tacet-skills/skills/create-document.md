---
name: create-document
triggers: excel, xlsx, pdf, word, docx, create a file, make a file, report, export, dump, markdown, spreadsheet, make a table
tools: create_document
---
# Creating documents

`create_document` writes the file; content goes in `content` as MARKDOWN.
`format`: data/plan/budget -> "excel" (`content` MUST be a markdown table);
prose/report -> "markdown"; plain note -> "text".

Those three are the ONLY values this build accepts. A user asking for PDF, Word or
docx still gets a file: pick the closest value above and say which one you produced.

## Table shape — EXAMPLE ONLY
| Day | Lunch | Dinner |
| --- | --- | --- |
| Monday | Lentils | Chicken |

- THE TABLE ABOVE IS A FORMAT EXAMPLE, never content. Never copy its columns or rows.
- Bulk data already read by another tool: do NOT retype it into `content`; pass the
  `source_ref` that tool returned.
- Numeric cells are plain numbers ("1500"); never compute totals yourself.
<!--/core-->
## Rules
- `file_name`: short, hyphenated, no extension; derive it from the user's actual subject.
- Write document content in the user's language unless the user asks otherwise.

## Your reply
Keep it minimal: "Done." Do NOT repeat the file name; the chip already shows it.
