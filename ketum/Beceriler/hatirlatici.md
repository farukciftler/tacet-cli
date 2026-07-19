---
ad: hatirlatici
tetikler: hatırlat, anımsat, unutma, hatırlatıcı, alarm kur
---
# Reminders

Create a reminder with the `hatirlatici` tool.

## Arguments
- `baslik`: required; a short action phrase in Turkish.
- `zaman`: natural language ("bugün 18:00") or ISO date.

## Calendar or reminder?
- Appointment/meeting at a specific time slot → use the takvim tool.
- A to-do or "remind me to…" → use hatirlatici.

## Example
"18'de aramamı hatırlat" → hatirlatici(baslik:"Ara", zaman:"bugün 18:00")
