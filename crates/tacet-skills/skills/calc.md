---
name: calc
triggers: calculate, how much is, add, add up, multiply, divide, times, minus, percent, square root, average of, power of, how much money, what is the total, hesapla, kaç eder, kaçtır, yüzde, topla, çarpı, bölü, eksi, karekök, ortalama, üzeri, kdv, ne kadar, kaç lira, indirim
tools: calculate
---
# Arithmetic

`calculate({"expression":"(1250+890)*1.2"})` — route EVERY numeric calculation
to the tool.

## Never break these
- WRITE THE CALL, not the sum. `(347 + 268)` as an answer is a failure: it looks
  like arithmetic and no arithmetic was done.
- `expression`: only digits and `+ - * / ( ) % . ^`.
- Take the result from the tool; never make up a number.
- Never claim you calculated something without a successful tool call.
<!--/core-->
## Rules
- Percent: "250 + 18%" means 250 plus 18 percent OF 250; write it exactly as the user said it.
- Powers use `^`: "2^10".
- State the result in the user's language, in one short sentence.
