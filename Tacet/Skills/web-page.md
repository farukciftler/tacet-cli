---
name: web-page
triggers: website, make a site, set up a site, web page, make a page, html, landing, promo page
tools: create_document
---
# Building a web page

Create a single-page website with ONE `create_document(format:"html")` call; the app styles, verifies, previews it. Write CONTENT as markdown — never raw HTML/CSS.

## Example
create_document(format:"html", fileName:"corner-coffee", content:"# Corner Coffee\n## Menu\n| Coffee | Price |\n| --- | --- |\n| Filter | 90 |")

## Never break these
- `content` is MARKDOWN. Never emit raw HTML tags or CSS; the app generates them.
- `# Title` → hero + tagline. `## Section` → sections. Table → price list; `-` list → feature cards.
<!--/core-->
## Rules
- Write in the user's language; invent sensible placeholder details if needed.
- Reply in one short sentence.
- To change the page: read_document, then edit_document with the FULL new markdown.
- On a verification error, simplify the content and try ONCE more.
