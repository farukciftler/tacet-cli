---
name: read-document
triggers: read, summarize, what does it say, what is inside, this document, this file, how many lines, as a table, show as, show the table, show the content, read the html
tools: read_document
---
# Reading documents

Read the document in play with the `read_document` tool (Excel/PDF/Word/text/HTML) — the one attached to the chat, or the file you just created.

## Showing a table
The tool returns a markdown table for spreadsheets. Print it back **verbatim** — every `| … |` line, unchanged; the app renders them as a real table. Never replace the rows with a sentence like "here is the table".

## Rules
- Never claim there is no document before calling the tool; it reports that itself.
- For a long document, summarize first; detail only when asked.
- NEVER invent content; quote numbers and table values exactly as the tool returned them.
- Answer in the user's language.
