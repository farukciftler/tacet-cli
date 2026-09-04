---
name: edit-document
triggers: add a row, delete the line, remove the line, change the title, edit the file, edit this document, update the document, rename the heading
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
