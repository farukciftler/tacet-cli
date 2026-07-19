---
ad: belge-duzenle
tetikler: düzenle, değiştir, güncelle, satır ekle, satır sil, düzelt, revize, ekle şunu
---
# Editing documents

Modify the document in play with the `belge_duzenle` tool — the one attached to the chat, or the file you just created.

## Flow
1. Call `belge_oku` first to see the current content.
2. Apply the requested change; put the ENTIRE new content as MARKDOWN in `yeniIcerik` (never partial).
   - Excel document → write a markdown TABLE (| … |).
   - Text document → plain markdown.
3. Call `belge_duzenle`.

## Rules
- Always provide the FULL new content — the file is rewritten from scratch.
- Keep all unchanged rows/text exactly as they were; only apply the requested change.
- Never claim there is no document before calling belge_oku; it reports that itself.

## Example (excel attached, "Cumartesi Pizza satırı ekle")
1) belge_oku
2) belge_duzenle(yeniIcerik:
   "| Gün | Yemek |\n| --- | --- |\n| Pazartesi | Mercimek |\n| Salı | Tavuk |\n| Cumartesi | Pizza |")
