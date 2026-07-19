---
ad: web-sayfa
taslak-tetikler: web sitesi, site yap, site kur, web sayfası, sayfa yap, html, landing, tanıtım sayfası
---
# Building a web page

Create a single-page website with `belge_olustur(bicim:"html")`. You write CONTENT as markdown; the app turns it into a styled, self-contained page and verifies it loads. Never write raw HTML/CSS yourself.

## Content structure (markdown → page)
- `# Title` → hero section (site name + one-line tagline under it).
- `## Section` → page sections: about, menu/products, contact…
- Markdown table → price list / feature table. `-` list → feature cards.
- Write the content in the user's language. Invent sensible placeholder details (opening hours, sample prices) if the user gave none — a page with gaps looks broken.

## Example
"kahve dükkanım için site yap" →
belge_olustur(bicim:"html", dosyaAdi:"kahve-dukkani", icerik:"# Köşe Kahve\nTaze kavrulmuş, her sabah.\n\n## Menü\n| Kahve | Fiyat |\n| --- | --- |\n| Filtre | 90 |\n\n## İletişim\nMah. Cad. No 3 — 09.00-19.00")

## Rules
- One tool call per page; the app previews it automatically. Reply in one short sentence ("Hazır.") — do not repeat the file name.
- To change the page afterwards: belge_oku, then belge_duzenle with the FULL new markdown.
- If the tool reports a verification error, simplify the content (plain sections, no exotic markdown) and try ONCE more.
