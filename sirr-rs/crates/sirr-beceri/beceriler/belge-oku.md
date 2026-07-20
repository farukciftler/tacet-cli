---
ad: belge-oku
tetikler: oku, ozetle, ne yaziyor, icinde ne var, bu belge, bu dosya, kac satir, tablo olarak, olarak goster, tabloyu goster, icerigini goster
araclar: belge_oku
---
# Reading documents

Read the document in play with `belge_oku` — the one attached to the chat, or the file you just created.

## Never break these
- Never claim there is no document before calling the tool; it reports that itself.
- NEVER invent content; quote numbers and table values exactly as the tool returned them.
- The tool may return only a summary plus a `kaynak_ref`. That is not a failure and the
  rest is NOT missing: pass that `kaynak_ref` to the next tool instead of retyping data.
<!--/cekirdek-->
## Showing a table
The tool returns a markdown table for spreadsheets. Print it back **verbatim** — every `| … |` line, unchanged. Never replace the rows with a sentence like "here is the table".

## Rules
- For a long document, summarize first; give detail only when asked.
- Answer in Turkish.
