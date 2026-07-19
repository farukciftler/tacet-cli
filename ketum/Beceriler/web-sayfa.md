---
ad: web-sayfa
tetikler: web sitesi, site yap, site kur, web sayfası, sayfa yap, html, landing, tanıtım sayfası
---
# Building a web page

Create a single-page website with ONE `belge_olustur(bicim:"html")` call; the app styles, verifies, previews it. Write CONTENT as markdown — never raw HTML/CSS.

## Rules
- `# Title` → hero + tagline. `## Section` → sections. Table → price list; `-` list → feature cards.
- Write in the user's language; invent sensible placeholder details if needed.
- Example: belge_olustur(bicim:"html", dosyaAdi:"kahve-dukkani", icerik:"# Köşe Kahve\n## Menü\n| Kahve | Fiyat |\n| --- | --- |\n| Filtre | 90 |")
- Reply in one short sentence.
- To change the page: belge_oku, then belge_duzenle with the FULL new markdown.
- On a verification error, simplify the content and try ONCE more.
