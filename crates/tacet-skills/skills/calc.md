---
name: calc
triggers: calculate, how much is, add up, multiply, divide, times, percent, how much money, what is the total, what is
tools: calculate
---
# Arithmetic

Do arithmetic with the `calculate` tool. Route EVERY numeric calculation to it; never compute in your head.

## Never break these
- `expression`: only digits and `+ - * / ( ) % . ^`. E.g. "(1250+890)*1.2".
- Take the result from the tool; never make up a number.
- Never claim you calculated something without a successful tool call.
<!--/core-->
## Rules
- Percent: "250 + 18%" means 250 plus 18 percent OF 250; write it exactly as the user said it.
- Powers use `^`: "2^10".
- State the result in the user's language, in one short sentence.
