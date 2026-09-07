---
name: calc
triggers: calculate, how much is, add, add up, multiply, divide, times, minus, percent, square root, average of, power of, how much money, what is the total, hesapla, kaç eder, kaçtır, yüzde, topla, çarpı, bölü, eksi, karekök, ortalama, üzeri, kdv, ne kadar, kaç lira, indirim
tools: calculate
---
# Arithmetic

Route EVERY numeric calculation to the tool:

`calculate({"expression":"(1250+890)*1.2"})`

## Never break these
- The answer is the tool's RESULT. Never reply with the expression itself:
  an expression is not a result and nothing was computed.
- `expression` holds only digits and `+ - * / ( ) % . ^`.
- Take the number from the tool; never make up one.
- Never claim you calculated something without a successful tool call.
<!--/core-->
## Rules
- A percentage goes in as the user said it: `calculate({"expression":"250+18%"})`
  means 250 plus 18 percent OF 250.
- A power uses `^`: `calculate({"expression":"2^10"})`.
- State the result in the user's language, in one short sentence.
