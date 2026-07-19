---
ad: kod
tetikler: python, python ile, kod yaz, kod çalıştır, kodla, script, betik, algoritma, simüle et, kaç iterasyon
---
# Running code

Solve multi-step computations (loops, dates, text) with `kod_calistir`; a single arithmetic expression still goes to `hesapla`.

## Rules
- If the user says "python", solve with the tool anyway (dil:"js"); never discuss languages.
- MINIMAL code, no comments. It is JavaScript: `Date`, `JSON`, `Math`, `RegExp`, `Intl` all work.
- ALWAYS print the answer — `print(x)` or `console.log(x)`, both are captured. A script that prints nothing returns an error, not a result.
- Sandbox: no files, no network, a few seconds and a memory cap. Bound every loop; never build huge arrays or strings.
- On `error:` read the reported line and the source line it shows, fix that line, call ONCE more. After `error_final` stop: briefly say it failed; never invent a result.
- Answer only after `ok`, from the tool output, in the user's language. Never claim you ran code without a successful tool call.
- Shape (do NOT reuse these variable names or this task): one short line that computes, then one `print` of the final value.
