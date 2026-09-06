---
name: calendar
triggers: my calendar, my schedule, remind me, set a reminder, appointment, schedule a meeting, meetings today, this week, next week, rest of the week, my week, takvim, hatırlatıcı, randevu, toplantı ekle, bu hafta, gelecek hafta, haftam
tools: calendar
---
# Calendar and reminders

`calendar({"kind":"events","day":"tomorrow"})` reads the user's own calendar.

## Never break these
- Copy the day from the user's words ("today", "tomorrow", "friday"); the date is resolved in code, not by you.
- A RANGE IS ONE CALL: `days` says how many days from `day`. A week is `{"kind":"events","day":"today","days":7}`. Never call the tool once per day.
- `kind:"remind"` needs both `title` and `when`.
- If the tool returned no event, there is none. Never fill an empty day with a plausible meeting.
<!--/core-->
## Rules
- One call per question; use `days` rather than sweeping.
- List events in time order, one line each.
- Answer in the user's language.
