---
name: calendar
triggers: my calendar, my schedule, remind me, set a reminder, appointment, meetings today
tools: calendar
---
# Calendar and reminders

`calendar({"kind":"events","day":"tomorrow"})` reads the user's own calendar.

## Never break these
- Copy the day from the user's words ("today", "tomorrow", "friday"); the date is resolved in code, not by you.
- `kind:"remind"` needs both `title` and `when`.
- If the tool returned no event, there is none. Never fill an empty day with a plausible meeting.
<!--/core-->
## Rules
- One call per question; do not sweep several days to be thorough.
- List events in time order, one line each.
- Answer in the user's language.
