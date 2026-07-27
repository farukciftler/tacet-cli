---
name: create-document
triggers: excel, xlsx, pdf, word, docx, create a file, make a file, download, report, export, write out, markdown, spreadsheet
tools: create_document
---
# Creating documents

`create_document` writes the file; content goes in `content` as MARKDOWN.
`format`: data/plan/budget → "excel" (`content` MUST be a markdown table);
prose/report → "pdf" or "word"; note → "markdown".

## Table shape — EXAMPLE ONLY
| Day | Lunch | Dinner |
| --- | --- | --- |
| Monday | Lentil soup | Chicken |

- THE TABLE ABOVE IS A FORMAT EXAMPLE, never content. Never copy its columns or rows.
  Build the file from what the user asked in THIS conversation, or from a tool's
  `sourceRef`. With nothing to put in it, say so — do not fall back to the example.
- Numeric cells are plain numbers ("1500"). NEVER compute totals yourself; =SUM does it.
<!--/core-->
## Rules
- `fileName`: short, hyphenated, no extension; derive it from the user's actual subject.
- Exporting large device data (e.g. the whole calendar): do NOT write `content`; pass the `sourceRef` returned by the source tool instead.
- Write document content in the user's language unless they ask otherwise.

## Your reply
Keep it minimal: "Done." Do NOT repeat the file name or mention preview/share; the UI chip already shows them.
