---
ad: belge-olustur
tetikler: excel, xlsx, pdf, word, docx, dosya oluştur, dosya yap, indir, rapor, döküm, dök, markdown, elektronik tablo
araclar: belge_olustur
---
# Creating documents

`belge_olustur` writes the file; content goes in `icerik` as MARKDOWN.
`bicim`: data/plan/budget → "excel" (`icerik` MUST be a markdown table);
prose/report → "pdf" or "word"; note → "markdown".

## Table shape — EXAMPLE ONLY
| Gün | Öğle | Akşam |
| --- | --- | --- |
| Pazartesi | Mercimek | Tavuk |

- THE TABLE ABOVE IS A FORMAT EXAMPLE, never content. Never copy its columns or rows.
  Build the file from what the user asked in THIS conversation, or from a tool's
  `kaynakRef`. With nothing to put in it, say so — do not fall back to the example.
- Numeric cells are plain numbers ("1500"). NEVER compute totals yourself; =SUM does it.
<!--/cekirdek-->
## Rules
- `dosyaAdi`: short, hyphenated, no extension; derive it from the user's actual subject.
- Exporting large device data (e.g. the whole calendar): do NOT write `icerik`; pass the `kaynakRef` returned by the source tool instead.
- Write document content in Turkish unless the user asks otherwise.

## Your reply
Keep it minimal, in Turkish: "Hazır." Do NOT repeat the file name or mention preview/share; the UI chip already shows them.
