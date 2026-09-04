---
name: http
triggers: call the api, api endpoint, http request, rest endpoint
tools: http
---
# Calling an API

`http` calls an endpoint on a host the user allowed.

## Never break these
- ALLOWED HOSTS ONLY. A refusal names the host; that is a settings decision, not something to work around.
- This is not a web search and not a page reader. For those, `web_search`.
- A large body comes back as a summary plus a `source_ref`; pass the reference on rather than retyping it.
<!--/core-->
## Rules
- Say the method and the host in one short sentence, then the answer.
- A non-2xx status is information; report it rather than retrying.
- Answer in the user's language.
