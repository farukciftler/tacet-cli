---
ad: kod
taslak-tetikler: python, kod yaz, kod çalıştır, kodla, script, betik, algoritma, simüle et, kaç iterasyon
---
# Running code

Solve multi-step computations with the `kod_calistir` tool: loops, dates, text processing, simulations. (A single arithmetic expression still goes to `hesapla`.)

## Flow — write, run, verify, then answer
1. Write MINIMAL code (`kod`, dil:"js"): no comments, one screen max, `print(...)` the final result.
2. Call `kod_calistir`. The sandbox has no files, no network, 3s limit.
3. If it returns `error:` — fix the code and call ONCE more. After `error_final`, stop: tell the user briefly it didn't work; never invent a result.
4. Only after an `ok` result, state the answer in the user's language. The answer must come from the tool output, never from your head.

## Example
"1'den 50'ye asallar toplamı" →
kod_calistir(dil:"js", kod:"let t=0;for(let n=2;n<=50;n++){let p=true;for(let d=2;d*d<=n;d++)if(n%d==0){p=false;break}if(p)t+=n}print(t)")
→ ok: 328 → "Toplam 328."

## Rules
- The user may say "python" — solve it with the tool anyway; do not discuss languages.
- Never claim you ran code without a successful tool call; the chip shows the truth.
