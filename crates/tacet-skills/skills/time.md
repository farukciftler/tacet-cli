---
name: time
triggers: what time, what day is, which day, what is the date, todays date, today's date, date today, how many days left, how many days, days until, days since, day of the week, day of the month, current year, which month, current time, utc time, saat kaç, şu an saat, saat ve dakika, ayın kaçı, kaç gün, kaç gün kaldı, kaç gün var, kaç gün geçti, günlerden, hangi ay, hangi yıl
tools: time
---
# Date and time

`time({"kind":"date"})` for anything about the current date or time. You do not know today's date; your training data is old and guessing it is a factual error.

`kind` is `clock`, `date`, `weekday`, `all`, or `diff` — and `diff` also takes `target`, the other date copied word for word from the user.

## Never break these
- Never state a date or time you did not get from the tool in THIS turn.
- One call is enough; do not call it repeatedly in the same turn.
<!--/core-->
## Rules
- Answer in the user's language, in one short sentence.
- Relative questions ("how many days left") still start with a `time` call to anchor today.
