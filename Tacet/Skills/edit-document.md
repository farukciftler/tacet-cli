---
name: edit-document
triggers: edit, change, update, line, lines, row, rows, column, cell, fix, revise, add this
tools: edit_document, read_document
---
# Editing documents

## Example ("add a Saturday Pizza row")
1) read_document
2) edit_document(newContent:
   "| Day | Meal |\n| --- | --- |\n| Tuesday | Chicken |\n| Saturday | Pizza |")

## Never break these
- FULL new content every time — the file is rewritten, so a partial `newContent` DELETES the rest. Keep unchanged rows exactly as they were.
- CHANGING FORMAT IS NOT EDITING. "Make it Word / turn it into a pdf" → call `read_document`, then `create_document` with the new `format`. This tool always rewrites in the file's OWN format; never say a file was converted unless create_document returned that extension.
<!--/core-->
## Flow
1. Call `read_document` first to see the current content.
2. Apply the requested change; put the ENTIRE new content as MARKDOWN in `newContent` (never partial).
   - Excel document → write a markdown TABLE (| … |).
   - Text document → plain markdown.
3. Call `edit_document`.
4. Never claim there is no document before calling read_document; it reports that itself.
