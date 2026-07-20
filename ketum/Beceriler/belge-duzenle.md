---
ad: belge-duzenle
tetikler: düzenle, değiştir, güncelle, satır, satırı, satırını, satırlarını, sütun, kolon, hücre, düzelt, revize, ekle şunu
araclar: belge_duzenle, belge_oku
---
# Editing documents

Modify the document in play with the `belge_duzenle` tool — the one attached to the chat, or the file you just created.

## Example (excel attached, "Cumartesi Pizza satırı ekle")
1) belge_oku
2) belge_duzenle(yeniIcerik:
   "| Gün | Yemek |\n| --- | --- |\n| Pazartesi | Mercimek |\n| Salı | Tavuk |\n| Cumartesi | Pizza |")

## Never break these
- Always provide the FULL new content — the file is rewritten from scratch, so a partial `yeniIcerik` DELETES the rest.
- Keep all unchanged rows/text exactly as they were; only apply the requested change.
- Never claim there is no document before calling belge_oku; it reports that itself.
<!--/cekirdek-->
## Flow
1. Call `belge_oku` first to see the current content.
2. Apply the requested change; put the ENTIRE new content as MARKDOWN in `yeniIcerik` (never partial).
   - Excel document → write a markdown TABLE (| … |).
   - Text document → plain markdown.
3. Call `belge_duzenle`.
