---
name: calc
triggers: calculate, how much is, add up, multiply, divide, divided by, percent, how much money, what is the total
tools: calculate
---
# Arithmetic

Do arithmetic with the `calculate` tool. Route EVERY numeric calculation to it; never compute in your head.

## Rules
- `expression`: only digits and `+ - * / ( ) % .` E.g. "(1250+890)*1.2".
- Percent: "20%" is treated as a 0.2 multiplier.
- Take the result from the tool; never make up a number.
- State the result in the user's language.
