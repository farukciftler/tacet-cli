---
name: code
triggers: run this code, run the script, execute, simulate, prime numbers, fibonacci, sort this list, python, python script, using python, in python, with python, script, write a script, save the script, asal sayı, beti, kod çalıştır, kod yaz, py adıyla, ekrana yazdır, yazdır, sırala
tools: run_code, write_code
---
# Running and writing code

TWO TOOLS, and the choice is what the user wants back:
- only the ANSWER -> `run_code({"code":"print(2**10)","language":"python"})`
- a FILE -> `write_code`, which saves the program to disk.

## Never break these
- NO NETWORK AND NO FILES in the sandbox: `open(...)`, downloads and installed
  packages fail. Everything the program needs goes in the code itself.
- `run_code`: the LAST line must print. Nothing else comes back.
- `write_code`: `file_name` short, hyphenated, NO extension; `lines` is an
  ARRAY, ONE line per element — no `\n` inside one, indentation is spaces.
- Never write a computed list from memory. If it can be computed, compute it.
<!--/core-->
## Rules
- Self-contained either way: no stdin, no arguments. End a saved file with a line that demonstrates it.
- A failed run comes back with its error; fix it and try once, do not loop.
- Comments in the user's language, code in English. Answer in one short sentence.
