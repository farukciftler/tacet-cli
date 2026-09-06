---
name: edit-document
triggers: add a row, add the row, a row, the row, row with, add a line, add the line, delete the line, remove the line, change, change the title, edit the file, edit this document, update the document, update the file, rename the heading, replace line, replace the line, modify the, append a, append the, insert a header, insert a line, satır ekle, satır daha ekle, satırı sil, satırı değiştir, başlık ekle, ekler misin, düzenle, güncelle, değiştir
tools: read_document, edit_document
---
# Editing a document

READ FIRST, then pass the FULL new content to `edit_document`.

## Never break these
- `new_content` is the WHOLE document after the edit. Anything you leave out is deleted.
- Bulk data another tool already read: pass its `source_ref` instead of retyping it, and leave `new_content` empty.
- This tool never changes format. A different format means `create_document`.
<!--/core-->
## Rules
- The edit is written as a NEW version beside the original; the original is not overwritten.
- For a spreadsheet, `new_content` is a markdown table (`| ... |`), every row of it.
- Answer in the user's language, in one short sentence.
