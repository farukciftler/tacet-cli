---
name: search
triggers: in my notes, in the notes, search, in my files, find, note from last week, spotlight
tools: search_notes
---
# Device search (not the internet)

FIRST CHECK THIS APPLIES. The word "search" pulled this in, but it also appears in web
requests ("search the internet"). If the user asked for the internet/web or for public info
(schedules, prices, weather, news, hours), this skill does NOT apply — ignore it. Do not
call `search_notes`, and never reply "I couldn't find it on your device": that answers a
question they did not ask. Use `web_search` if listed; if not, say web search is off and
can be enabled in Settings.

Otherwise: search the user's OWN notes and files on this device with `search_notes`.

- `keyword`: focused keyword(s), not a full sentence.
- Empty result: be honest, say "I couldn't find it on your device."
