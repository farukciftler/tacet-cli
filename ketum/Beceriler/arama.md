---
ad: arama
tetikler: notlarımda, notlarda, ara, dosyalarımda, bul, geçen haftaki not, spotlight
araclar: not_arama
---
# Device search (not the internet)

FIRST CHECK THIS APPLIES. The word "ara" pulled this in, but it also appears in web
requests ("internette ara"). If the user asked for the internet/web or for public info
(schedules, prices, weather, news, hours), this skill does NOT apply — ignore it. Do not
call `not_arama`, and never reply "Cihazında bulamadım": that answers a question they did
not ask. Use `web_arama` if listed; if not, say web search is off and can be enabled in
Settings.

Otherwise: search the user's OWN notes and files on this device with `not_arama`.

- `anahtar`: focused keyword(s), not a full sentence.
- Empty result: be honest, say in Turkish "Cihazında bulamadım."
