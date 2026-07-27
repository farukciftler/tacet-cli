---
name: time
triggers: what time, what day is, which day, what is the date, todays date, how many days left, which month, current time
tools: time
---
# Date and time

Call `time` for anything about the current date or time. You do not know today's date; your training data is old and guessing it is a factual error.

## Never break these
- Never state a date or time you did not get from the tool in THIS turn.
- One call is enough; do not call it repeatedly in the same turn.
<!--/core-->
## Rules
- Answer in the user's language, in one short sentence.
- Relative questions ("how many days left") still start with a `time` call to anchor today.
