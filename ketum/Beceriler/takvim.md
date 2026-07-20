---
ad: takvim
tetikler: takvim, etkinlik, toplantı, randevu, programım, yarın ne var, bugün ne var, ajanda
araclar: takvim
---
# Calendar

Read and add events with the `takvim` tool.

## Arguments
- `eylem`: "oku" to read, "ekle" to add.
- `baslangic`/`bitis`: natural language ("bugün", "yarın 13:00") or ISO ("2026-07-20T13:00").
- Adding: `baslik` is required (Turkish), and give `baslangic`.

## Exporting to a file (context budget)
If the user says "export my calendar to excel/pdf", do NOT write the data yourself:
1) Call takvim(eylem:"oku") → it returns a `kaynakRef`.
2) Call belge_olustur(bicim:"excel", kaynakRef:<that ref>).

## Example
"Yarın 3'te diş hekimi ekle" → takvim(eylem:"ekle", baslik:"Diş hekimi", baslangic:"yarın 15:00")
