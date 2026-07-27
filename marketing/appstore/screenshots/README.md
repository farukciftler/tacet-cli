# App Store screenshots

**Ready to upload:** `paper/`, `night/`, `ink/` — three palette variations of the
same story, 1320×2868 (the 6.9" slot, which App Store Connect downscales for the
smaller sizes). Pick one folder and upload it; do not mix folders in one listing.

| Folder | Background | Device capture | Reads as |
|---|---|---|---|
| `paper/` | warm paper `#F7F3EA` | light mode | calm, editorial, matches the app's default |
| `night/` | night `#0B1220` | dark mode | quiet, focused |
| `ink/` | deep navy `#131C2E` | light mode | highest contrast — the phone glows against the field |

Every colour is a token from the app itself (`Design/Theme.swift`). A marketing
colour that is not in the product is a promise the product does not keep.

## How these were made

`raw/` holds the untouched simulator captures; `compose.py` turns them into the
frames. To regenerate:

```sh
xcrun simctl boot "iPhone 17 Pro Max"
xcrun simctl launch <udid> zortproductions.tacet --demo-seed -AppleLanguages "(en)"
xcrun simctl io <udid> screenshot raw/<name>.png
python3 compose.py
```

`--demo-seed` is a DEBUG-only launch argument (`Services/DemoSeed.swift`). It
writes real records through the real models, so the frames are the actual
interface rendering actual data. It exists because the simulator has no Apple
Intelligence: without seeding, every capture would show the "model unavailable"
state instead of the product.

**The seed may only contain states the app can genuinely produce.** A chip no
tool emits or a flow that does not exist would make the screenshot a claim the
product cannot keep.

## Language

The interface inside these frames is English, captured with
`-AppleLanguages "(en)"`. An English caption over a Turkish frame is not a
styling detail — it is a false statement about what the buyer will see. If a
`tr-TR` listing is added, recapture with the simulator in Turkish and write the
captions in Turkish; do not translate the caption alone.

A previous set (`captioned-en/`, `captioned-tr/`) was deleted rather than kept:
it showed the pre-rename brand in the top bar and in the input placeholder, so
uploading it would have shipped a dead name.
