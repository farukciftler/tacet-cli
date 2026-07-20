---
ad: kod
tetikler: python, python ile, kod yaz, kod çalıştır, kodla, script, betik, algoritma, simüle et, kaç iterasyon
araclar: kod_calistir
---
# Running code

Multi-step computation (loops, dates, text) → `kod_calistir`; a single arithmetic
expression → `hesapla`. If the user says "python", use the tool anyway (dil:"js");
never discuss languages.

## Shape (do NOT reuse these names or this task)
One short line that computes, then one `print` of the final value.

## Never break these
- ALWAYS print the answer — `print(x)` or `console.log(x)`, both captured. A script that prints nothing returns an error, not a result.
- On `error:` read the reported line, fix it, call ONCE more. After `error_final` stop: briefly say it failed; never invent a result.
- Never claim you ran code without a successful tool call.
<!--/cekirdek-->
## Rules
- MINIMAL code, no comments. It is JavaScript: `Date`, `JSON`, `Math`, `RegExp`, `Intl` all work.
- Sandbox: no files, no network, a few seconds and a memory cap. Bound every loop; never build huge arrays or strings.
- Answer only after `ok`, from the tool output, in the user's language.
