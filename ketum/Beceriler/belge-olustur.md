---
ad: belge-olustur
tetikler: excel, xlsx, pdf, word, docx, dosya oluştur, dosya yap, indir, rapor, döküm, dök, markdown, elektronik tablo
---
# Creating documents

Create an Excel/PDF/Word/Markdown file with the `belge_olustur` tool. Write the content as MARKDOWN in the `icerik` argument.

## Choosing the format
- Numeric data, list, plan, budget, comparison → bicim="excel"; `icerik` must be a MARKDOWN TABLE.
- Report, letter, summary, prose → bicim="pdf" or "word"; plain markdown.
- Simple note → bicim="markdown".

## Markdown table (required for excel)
| Gün | Öğle | Akşam |
| --- | --- | --- |
| Pazartesi | Mercimek | Tavuk |
| Salı | Çorba | Balık |

## Rules
- THE TABLE ABOVE IS A FORMAT EXAMPLE ONLY. Never copy its file name, its columns or
  its rows into a real document. Every document you create must be built from what the
  user asked for in THIS conversation, or from a `kaynakRef` a tool returned. If you have
  no content, say you have nothing to put in the file — do not fall back to the example.
- `dosyaAdi`: short, hyphenated, no extension; derive it from the user's actual subject.
- Write numeric cells as plain numbers ("1500"). NEVER compute totals yourself; the spreadsheet does it with =SUM.
- Exporting large device data (e.g. the whole calendar): do NOT write `icerik`; pass the `kaynakRef` returned by the source tool instead.
- Write document content in Turkish unless the user asks otherwise.

## Your reply
Keep it minimal, in Turkish: "Hazır." Do NOT repeat the file name or mention preview/share; the UI chip already shows them.
