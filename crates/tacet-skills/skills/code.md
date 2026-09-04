---
name: code
triggers: run this code, run the script, execute, simulate, prime numbers, fibonacci, sort this list, python script, write a script, save the script
tools: run_code, write_code
---
# Running and writing code

ONE GUIDE, TWO TOOLS, and the choice is what the user wants back:
- only the ANSWER -> `run_code`; it executes and returns what was printed.
- a FILE -> `write_code`; it saves the program to disk.

## Never break these
- NO NETWORK AND NO FILES in the sandbox: `open(...)`, downloads and installed
  packages all fail. Everything the program needs goes in the code itself.
- `run_code`: the LAST line must print. Nothing else comes back.
- `write_code`: `lines` is an ARRAY, ONE line of code per element. Never put
  `\n` inside an element; indentation is leading spaces.
- Never write out a computed list from memory. If it can be computed, compute it.
<!--/core-->
## Rules
- Self-contained either way: no stdin, no arguments. End a saved file with a line that demonstrates it.
- A failed run comes back with its error; fix it and try once, do not loop.
- `file_name` short and hyphenated; the tool adds the extension.
- Comments in the user's language, code in English. Answer in one short sentence.
