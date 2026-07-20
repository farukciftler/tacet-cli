---
ad: belge-olustur
tetikler: excel, xlsx, pdf, word, docx, dosya olustur, dosya yap, rapor, dokum, dok, markdown, elektronik tablo, tablo yap
araclar: belge_olustur
---
# Creating documents

`belge_olustur` writes the file; content goes in `icerik` as MARKDOWN.
`bicim`: data/plan/budget -> "excel" (`icerik` MUST be a markdown table);
prose/report -> "pdf"; note -> "markdown".

## Table shape — EXAMPLE ONLY
| Gun | Ogle | Aksam |
| --- | --- | --- |
| Pazartesi | Mercimek | Tavuk |

- THE TABLE ABOVE IS A FORMAT EXAMPLE, never content. Never copy its columns or rows.
- Bulk data already read by another tool: do NOT retype it into `icerik`; pass the
  `kaynak_ref` that tool returned.
- Numeric cells are plain numbers ("1500"); never compute totals yourself.
<!--/cekirdek-->
## Rules
- `dosya_adi`: short, hyphenated, no extension; derive it from the user's actual subject.
- Write document content in Turkish unless the user asks otherwise.

## Your reply
Keep it minimal, in Turkish: "Hazir." Do NOT repeat the file name; the chip already shows it.
