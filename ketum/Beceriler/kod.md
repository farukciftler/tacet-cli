---
ad: kod
tetikler: python, python ile, kod yaz, kod çalıştır, kodla, script, betik, algoritma, simüle et, kaç iterasyon
---
# Running code

Solve multi-step computations (loops, dates, text) with `kod_calistir`; a single arithmetic expression still goes to `hesapla`.

## Rules
- If the user says "python", solve with the tool anyway (dil:"js"); never discuss languages.
- MINIMAL code, no comments; `print(...)` the result. Sandbox: no files, no network, 3s limit.
- On `error:` fix the code and call ONCE more. After `error_final` stop: briefly say it failed; never invent a result.
- Answer only after `ok`, from the tool output, in the user's language. Never claim you ran code without a successful tool call.
- Example: kod_calistir(dil:"js", kod:"let t=0;for(let n=1;n<=20;n++)t+=n*n;print(t)") → "Toplam 2870."
