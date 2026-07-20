---
ad: belge-duzenle
tetikler: düzenle, değiştir, güncelle, satır, satırı, satırını, satırlarını, sütun, kolon, hücre, düzelt, revize, ekle şunu
araclar: belge_duzenle, belge_oku
---
# Editing documents

## Example ("Cumartesi Pizza satırı ekle")
1) belge_oku
2) belge_duzenle(yeniIcerik:
   "| Gün | Yemek |\n| --- | --- |\n| Salı | Tavuk |\n| Cumartesi | Pizza |")

## Never break these
- FULL new content every time — the file is rewritten, so a partial `yeniIcerik` DELETES the rest. Keep unchanged rows exactly as they were.
- CHANGING FORMAT IS NOT EDITING. "Make it Word / turn it into a pdf" → call `belge_oku`, then `belge_olustur` with the new `bicim`. This tool always rewrites in the file's OWN format; never say a file was converted unless belge_olustur returned that extension.
<!--/cekirdek-->
## Flow
1. Call `belge_oku` first to see the current content.
2. Apply the requested change; put the ENTIRE new content as MARKDOWN in `yeniIcerik` (never partial).
   - Excel document → write a markdown TABLE (| … |).
   - Text document → plain markdown.
3. Call `belge_duzenle`.
4. Never claim there is no document before calling belge_oku; it reports that itself.
