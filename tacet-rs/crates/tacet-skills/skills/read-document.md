---
name: read-document
triggers: read, summarize, what does it say, what is inside, this document, this file, how many rows, as a table, show the table, show its content
tools: read_document
---
# Reading documents

Read the document in play with `read_document` — the one attached to the chat, or the file you just created.

## Never break these
- Never claim there is no document before calling the tool; it reports that itself.
- NEVER invent content; quote numbers and table values exactly as the tool returned them.
- The tool may return only a summary plus a `source_ref`. That is not a failure and the
  rest is NOT missing: pass that `source_ref` to the next tool instead of retyping data.
<!--/core-->
## Showing a table
The tool returns a markdown table for spreadsheets. Print it back **verbatim** — every `| … |` line, unchanged. Never replace the rows with a sentence like "here is the table".

## Rules
- For a long document, summarize first; give detail only when asked.
- Answer in the user's language.
