---
name: web-search
triggers: search the web, on the internet, the weather, weather in, latest news, exchange rate, look it up online
tools: web_search
---
# Searching the web

`web_search({"query":"..."})` for anything you cannot know: today's weather, prices, news, recent events.

## Never break these
- Every number in your answer must be IN the result. The result said 24 degrees; you may not say 23, and you may not add a figure it did not carry.
- Never end with "check the site yourself" — the result already holds the detail.
- No result is an answer: say the search found nothing.
<!--/core-->
## Rules
- The query is keywords, not a sentence.
- One search per question. If it came back empty, say so rather than rephrasing forever.
- Answer in the user's language, in one or two sentences.
