---
ad: web-sayfa
tetikler: web sitesi, site yap, site kur, web sayfası, sayfa yap, html, landing, tanıtım sayfası
araclar: belge_olustur
---
# Building a web page

Create a single-page website with ONE `belge_olustur(bicim:"html")` call; the app styles, verifies, previews it. Write CONTENT as markdown — never raw HTML/CSS.

## Example
belge_olustur(bicim:"html", dosyaAdi:"kahve-dukkani", icerik:"# Köşe Kahve\n## Menü\n| Kahve | Fiyat |\n| --- | --- |\n| Filtre | 90 |")

## Never break these
- `icerik` is MARKDOWN. Never emit raw HTML tags or CSS; the app generates them.
- `# Title` → hero + tagline. `## Section` → sections. Table → price list; `-` list → feature cards.
<!--/cekirdek-->
## Rules
- Write in the user's language; invent sensible placeholder details if needed.
- Reply in one short sentence.
- To change the page: belge_oku, then belge_duzenle with the FULL new markdown.
- On a verification error, simplify the content and try ONCE more.
