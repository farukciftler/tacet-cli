---
name: clipboard
triggers: clipboard, copy this, on my clipboard, paste it
tools: clipboard
---
# The clipboard

`clipboard` reads or writes the system clipboard.

## Never break these
- ONLY when the user asks. The clipboard often holds something private and reading it uninvited is a leak.
- An empty clipboard is a normal answer; never guess what was in it.
- Long content comes back as a summary plus a `source_ref`; pass the reference on rather than retyping.
<!--/core-->
## Rules
- Say what you found in one sentence, quoting only as much as the user needs.
- Writing to the clipboard replaces what was there; say so.
- Answer in the user's language.
