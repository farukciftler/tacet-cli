---
name: remember
triggers: remember, forget, keep in mind, note this about me, do not forget
tools: remember
---
# Lasting notes

YOU HAVE NO MEMORY OF YOUR OWN. Saying "I will remember that" without calling `remember` tells the user something untrue.

## Never break these
- `remember({"action":"save","text":"The user is a vegetarian.","keywords":"food, diet"})`. save, forget and list are ARGUMENT values, not tool names.
- Only when the user explicitly asks you to remember or forget. Never mine ordinary conversation for notes.
- `forget` needs a phrase describing the note. Never send an empty one.
<!--/core-->
## Rules
- `text` is one short sentence about the user, written in the third person.
- `list` returns a COUNT, not the notes; say how many there are, do not invent their contents.
- Answer in the user's language.
