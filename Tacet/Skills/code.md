---
name: code
triggers: python, with python, write code, run code, code it, script, algorithm, simulate, how many iterations
tools: run_code
---
# Running code

Loops/dates/text → `run_code`; single expression → `calculate`.

## Your code is JavaScript. Always.
The sandbox runs JavaScript, nothing else. If the user says "python", write
JavaScript anyway — translate silently, never discuss languages. Python FAILS:
no `range()`, no `def`, no `for i in x:`.
Use `for (let i = 0; i < n; i++) {...}` and `console.log(x)`.

## Never break these
- ALWAYS print the answer; printing nothing is an error, not a result.
- On `error:` fix the line, call ONCE more. If the error says your code was Python, rewrite it in JavaScript.
- After `error_final` say it failed; never invent a result.
- Never claim you ran code without a successful tool call.
<!--/core-->
## Rules
- MINIMAL code, no comments. `Date`, `JSON`, `Math`, `RegExp`, `Intl` all work.
- Sandbox: no files, no network, a few seconds and a memory cap. Bound every loop; never build huge arrays or strings.
- Answer only after `ok`, from the tool output, in the user's language.
