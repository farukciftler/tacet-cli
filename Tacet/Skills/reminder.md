---
name: reminder
triggers: remind, remind me, don't forget, reminder, set an alarm
tools: reminder
---
# Reminders

Create a reminder with the `reminder` tool.

## Arguments
- `title`: required; a short action phrase in the user's language.
- `time`: natural language ("today 18:00") or ISO date.

## Calendar or reminder?
- Appointment/meeting at a specific time slot → use the calendar tool.
- A to-do or "remind me to…" → use reminder.

## Example
"remind me to call at 6" → reminder(title:"Call", time:"today 18:00")
