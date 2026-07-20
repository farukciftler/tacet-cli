---
ad: hesap
tetikler: hesapla, kac eder, topla, carp, bol, bolersek, yuzde, kac para, toplam ne kadar, kacta kac
araclar: hesapla
---
# Arithmetic

Do arithmetic with the `hesapla` tool. Route EVERY numeric calculation to it; never compute in your head.

## Never break these
- `ifade`: only digits and `+ - * / ( ) % . ^`. E.g. "(1250+890)*1.2".
- Take the result from the tool; never make up a number.
- Never claim you calculated something without a successful tool call.
<!--/cekirdek-->
## Rules
- Percent: "250 + 18%" means 250 plus 18 percent OF 250; write it exactly as the user said it.
- Powers use `^`: "2^10".
- State the result in Turkish, in one short sentence.
