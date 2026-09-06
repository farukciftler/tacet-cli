---
name: web-fetch
triggers: http, https://, this address, at this url, the page at, the article at, the website, fetch the content, open the link
tools: web_fetch
---
# Fetching one page

`web_fetch({"url":"..."})` when the message CARRIES the address. The page is
already named; there is nothing to search for.

## Never break these
- The `url` is the one in the message, copied exactly — never a shortened,
  guessed or reconstructed address.
- Every fact in your answer must be ON that page. If the page did not say it,
  neither do you.
- `web_search` takes keywords and finds pages. It cannot be handed a URL; a URL
  belongs here.
<!--/core-->
## Rules
- One fetch per address. A page that came back empty is an answer, not a reason
  to try a different URL.
- Summarize; do not paste the page back.
- Answer in the user's language, in one or two sentences.
