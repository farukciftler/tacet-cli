---
name: calendar
triggers: calendar, event, meeting, appointment, my schedule, what is on tomorrow, what is on today, agenda
tools: calendar
---
# Calendar

Read and add events with the `calendar` tool.

## Arguments
- `action`: "read" to read, "add" to add.
- `start`/`end`: natural language ("today", "tomorrow 13:00") or ISO ("2026-07-20T13:00").
- Adding: `title` is required (in the user's language), and give `start`.

## Exporting to a file (context budget)
If the user says "export my calendar to excel/pdf", do NOT write the data yourself:
1) Call calendar(action:"read") → it returns a `sourceRef`.
2) Call create_document(format:"excel", sourceRef:<that ref>).

## Example
"Add dentist tomorrow at 3" → calendar(action:"add", title:"Dentist", start:"tomorrow 15:00")
