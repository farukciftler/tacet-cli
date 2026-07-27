# Tacet — Timeline (Live Step Stream) Specification

**Version:** 0.1 (draft) · **Date:** 19 July 2026 · **Platform:** iOS 26.0+ — iPhone only
**Related documents:** [tacet-spec.md](tacet-spec.md) §4.4 (the chip system — Timeline sits on top of it, not beside it)
**Status:** Implemented — read this spec together with the code as it stands today

---

## 1. Summary and naming

**Timeline** is the layer that shows, step by step, which stage the assistant is at while a reply is being produced, collapses into a single line when the reply is done, and can be expanded for detail on demand. It is the Tacet-language translation of Claude Code's "Read 3 files ›" / collapsible step list.

The name comes from a ship's log: the route is recorded, whoever wants to can open it and look, nobody is forced to read it. It belongs to the same family as `Skill` — one word, plain, explaining its function through its metaphor.

The core architectural decision:

> **Timeline is not a new source of truth.** Most of the steps are already-existing tool traces (`ToolTrace`); Timeline lines them up on a time axis, adds a few deterministic pipeline events, and collapses them. The model knows nothing about Timeline — zero cost to the prompt budget.

---

## 2. Principles

1. **Real signal only.** FoundationModels gives no chain of thought; Tacet does **not fabricate** decorative verbs like "thinking", "analyzing" either (Claude Code's "Untangling" theatrics run against the brand — no dramatization). Every step is a deterministic event that actually happens in the code: profile selected, tool ran, writing began.
2. **Side effects do not collapse.** When the reply is done, read steps collapse into a single line; but steps that change the world (`written` — event added, document produced) and errors (`failed`) **stay outside the collapse, always visible**. Transparency is not a toggleable ornament; Tacet does not hide what it does. Steps that produce a file appear outside the collapse not as a chip but as a **file card** (see §9).
3. **It does not force reading.** The default view cannot be busier than today's. One line while live, one line when done; it opens for whoever wants detail. The step list does not compete with the chat bubble.
4. **The tool layer is the single source of truth.** A tool step's text/state is read from `ToolTrace`, not copied. The chip detail (raw input/output) is the same sheet inside Timeline too — no second detail surface is written.
5. **The model is unaware.** Timeline injects no text into any prompt/instruction, and no "status report" is requested from the model (an extra job for a small model = the regression measured in the skill layer).

---

## 3. User flows

### 3.1 Live view (while the reply is being produced)

The user sends a message. In place of the reply bubble a **timeline ribbon** appears: a single line, the current step, a spinner at the front:

```
◌ reading calendar · today
```

The line changes as the step changes (previous steps accumulate, but even while live only the **last 1 line + a dimmed "step n" counter above** is visible; the full list opens on tap). When writing begins the ribbon is pulled above the reply and the text starts to stream:

```
◌ writing
You have three meetings today...
```

Tapping the live ribbon opens the timeline right then — the user can see the past steps of the ongoing work without waiting.

### 3.2 Collapsed view (once the reply is done)

The ribbon collapses into a single line above the reply:

```
timeline · 4 steps · 6 s ›
event added ✓               ← a side-effect chip does not collapse (§2.2)
I added the dentist at 10:00 tomorrow...
```

- The collapsed line is at `Typography.chip()` size, `Palette.muted`, no hairline frame — it sits one notch behind the chips visually.
- `written` and `failed` chips keep appearing where they are today, below the collapse line. The only thing that changes: **read chips** (`read_ok`) are no longer in the default view, they are inside the collapse.
- If the step count is 1 and that one is writing (a reply with no tools), the collapse line is **not shown at all** — Timeline stays quiet where it has nothing to say.

### 3.3 Expanding for detail

Tapping the collapse line opens the timeline in place (not a sheet — it does not tear you out of context):

```
timeline · 4 steps · 6 s ⌄
│ routed · calendar profile               0.2 s
│ skill added · read-document             —
│ calendar read · 3 events                1.1 s   ›
│ written                                 4.4 s
```

- Each row: step text + duration. Tool steps have a `›` at the end; tapping it opens the **existing** `ToolChipDetail` sheet (raw input/output). Pipeline steps (routing, writing) have no detail; the duration is already in the row.
- A vertical hairline (`Palette.divider`) connects the steps; no icons, no colors — state is carried by words and marks.
- Tapping again collapses it. The open/closed state is not remembered per message; the default is always closed (a user going back through history reads the reply, not the kitchen).

### 3.4 Errors and interruption

- If a tool `failed`, the collapse line does not merely count it, it says it: `timeline · 4 steps · 1 not completed ›`. The failed chip is already visible outside (§2.2).
- If generation is cut short (the user stopped it / the app went to the background) the last step closes as `left unfinished` and appears in the collapse line. Nothing disappears silently.

---

## 4. Interface

Design language unchanged: ink/grey, no accent color, hairline, no dramatization.

| Component | Location | Note |
|---|---|---|
| `TimelineRibbon` | `Views/` | The live single line: spinner + the current step's text. Aligned with the reply bubble, left-aligned. |
| `TimelineLine` | `Views/` | The collapsible timeline (3.2–3.3). Not a `DisclosureGroup` but hand-built — the expand animation respects `reduceMotion`. |
| `ToolChipDetail` | existing | Tool step detail; reused as is, not copied. |

The wording is the same as the chip language: lowercase, not the aorist but real past/present ("read", "writing"); no exclamation marks and no personification. New strings go into `L10n` and `Localizable.xcstrings`.

Accessibility: the live ribbon announces step changes via `accessibilityLabel` (`.updatesFrequently`); the collapse line says "timeline, 4 steps, tap to open"; the timeline rows can be navigated one by one.

---

## 5. Technical architecture

### 5.1 Data model

`Models/TimelineStep.swift` — a Codable struct (not a SwiftData @Model; the `ToolTrace` pattern):

| Field | Type | Description |
|---|---|---|
| `id` | UUID | |
| `kind` | enum raw String | `routing` / `enrichment` / `tool` / `writing` / `interruption` |
| `text` | String | The row text ("routed · calendar profile"). **Empty** for a tool step — the text is read from `ToolTrace`. |
| `toolTraceID` | UUID? | If `kind == .tool`, the corresponding `ToolTrace.id` — the link to the single source of truth |
| `start` / `end` | Date / Date? | Duration comes from here; `end == nil` = ongoing / left unfinished |

Persistence: a `stepsData: Data?` field on `Message` (exactly the `tracesData` pattern, with a default so it is lightweight-migration compatible). On old messages the field is empty → no Timeline line is drawn, chips look as they do today. Backfilling is **not** done.

### 5.2 Producer: TimelineRecorder

`Services/TimelineRecorder.swift` — an `@MainActor` class, alive for the duration of a reply turn:

- `begin(kind:text:)` → opens a new step, closes the previous one (steps are sequential; parallel tool calls arrive one at a time in FoundationModels).
- `bindTool(traceID:)` → binds the tool step to the `ToolTrace`. `ToolExecutor.start/update` already manages the chip lifecycle; the only line added for Timeline is opening a step at the `start` moment. **Tools are not touched** — the `TacetTool` protocol and `runWithChip` do not change.
- `finish()` / `cut()` → closes the last step, writes the step list into `Message.steps`.

Event sources (all of them already-existing deterministic points):

| Step | From where |
|---|---|
| `routing` | the outcome of `ModelService.intentProfile` selection (the profile name) |
| `enrichment` | if `skilledPrompt` attached a skill (the skill name) |
| `tool` | `ToolExecutor.start` |
| `writing` | when the first chunk arrives from the `respond` stream |
| `interruption` | cancellation / scenePhase interruption (the existing left-unfinished path) |

### 5.3 Layer interaction

- `ModelService` only reports events to `TimelineRecorder`; the view layer observes the recorder as `@Observable`. There is no change at all on the model side (instruction, prompt, tool specs are the same).
- When MCP and web search arrive, no extra work is needed: because their chips also go through `ToolExecutor.start`, they land in Timeline on their own. The approval chip (a tainted session) is a step too — an "awaiting approval" row, with the waiting time shown honestly.
- Calls coming from App Intents / Shortcuts also produce a Timeline (the tacet-spec §7.8 trace rule): even if nobody is watching the screen at that moment, the steps are recorded and read in the same collapse line when the user opens the app.

### 5.4 Performance

- Step events are ~3–8 per turn; the cost to the main stream is negligible. A live ribbon update is a single line of text changing with `withAnimation`; it does not race the token stream (writing is a single step, there is no per-chunk update).
- `stepsData` is a few hundred bytes per message; the SwiftData overhead is insignificant.

---

## 6. Test and measurement

- **SelfTest** (needs no model):
  - Recorder: sequential steps opening and closing, durations never being negative, the last step with `end == nil` turning into `interruption` after `cut()`.
  - Encoding: a `TimelineStep` list being written to `Message` and read back; an old message with `stepsData == nil` returning an empty list.
  - Collapse rule: no line being produced on a writing-only turn; `written`/`failed` traces staying in the list outside the collapse (the view helper is tested as a pure function).
- **Evaluation** (`--test`, on device): the step sequence forming in the expected order on a turn with tools (routing → tool → writing); the `interruption` step on a cancelled turn.
- Acceptance criterion: with Timeline **absent and present** the model output behaves bit-identically (tool choice, text) — Timeline is a pure observer; the `Evaluation` run does not measure a Timeline off/on difference because there should be no difference to measure.

---

## 7. Scope

**v1 (this spec):** Live ribbon, collapsible timeline, step persistence, tool trace binding, interruption step, accessibility.

**v1.1 candidates:** showing MCP approval wait time separately, intermediate state within a step for long searches ("3/5 servers replied" style — only if there is real signal).

**Deliberately out:** fake/filler state verbs ("thinking…"), estimated time / progress bar (unknowable — there will be no lying progress bar), feeding the step list back to the model, retroactive timeline generation for past messages.

---

## 8. Open questions

1. Should the `enrichment` step also show memory (Tacet notes) injection? In favor of transparency; but the memory spec tells the model "never mention the notes" — a "2 notes from memory" row in the interface tells the user the very thing the model does not mention. There are two consistent options: both visible (transparency) or both silent (the memory board is the only surface). In v1 **skill is visible, memory is not**; to be reconsidered when the memory layer is implemented.
2. How far should the routing step showing the profile name open up the inner kitchen? If "calendar profile" says nothing to the user, the row is noise; the alternative is to fold the routing step into the duration and hide it as a row.
3. While live, is "the last 1 line" enough, or should the last 2–3 steps stream by, fading out? (Claude Code leaves the last steps dimmed.) To be looked at in the prototype — the rule: it cannot be taller than the chat bubble.

---

## 9. File card (presenting produced files)

### 9.1 Why

Today a produced file is a `written` chip with an "eye" mark; the file name, its type and what can be done with it cannot be read off the chip. The file card becomes the **visible surface outside the collapse** for a step that produces a file (the chip keeps standing as a step in the timeline — the single source of truth is still `ToolTrace`).

### 9.2 Appearance

A card below the reply body, aligned with the bubble:

```
┌───────────────────────────────────────────────┐
│ ⌸  Stars discovery questions                  │
│    Spreadsheet · XLSX              [Open] [⇧] │
└───────────────────────────────────────────────┘
```

- Hairline frame (`Palette.divider`), `Spacing.chipCorner` corner; background `Palette.background` — the card is not a bubble, it belongs to the chip family.
- On the left the **file type icon** (9.3), in the middle the file name (`Typography.user()`, single line, middle truncation) + on the line below the type label (`Typography.chip()`, `Palette.muted`): "Spreadsheet · XLSX".
- On the right two actions: **"Open"** (the existing `DocumentPreviewSheet` / QuickLook) and share (`ShareLink`, `square.and.arrow.up`). The reference design's "Download and open" is meaningless here — the file is already on the device; the card uses no wording that implies otherwise.
- **There is no colored brand icon.** Third-party colors like Excel green or PDF red run against the palette; every icon is single-color (`Palette.ink`/`Palette.grey`), in the hairline-stroke style, from the `TacetMark` family.
- If the file has been deleted from disk, the card stays but the actions drop; the bottom label says "the file is no longer on the device". Nothing disappears silently.

### 9.3 File type icon set — 20 types

**A dedicated icon is needed for each of the 20 most common file types.** The icons live in `Design/FileIcon.swift` + the asset catalog as template (single color) vectors; the extension → icon mapping is in code:

| Group | Extensions (20) |
|---|---|
| Document | `pdf` · `docx` · `md` · `txt` · `rtf` |
| Table/data | `xlsx` · `csv` · `json` |
| Presentation | `pptx` |
| Image | `png` · `jpg` · `heic` · `gif` · `svg` |
| Audio | `mp3` · `m4a` · `wav` |
| Video | `mp4` · `mov` |
| Archive | `zip` |

- The mapping is case-insensitive; synonyms such as `jpeg` → `jpg`, `markdown` → `md` are folded into the table in code.
- **Every extension not on the list** falls back to the generic "document" icon (a fallback is mandatory — a card cannot be drawn without an icon).
- The type label comes not from the extension but from the `UTType` localization ("Spreadsheet", "PNG image"); if `UTType` cannot resolve it, the extension is written alone in uppercase.
- In v1 the types Tacet produces itself (`xlsx`, `docx`, `pdf`, `md`, `csv`, `txt`) actually appear; the remaining icons are drawn now for attached/future file flows — the set is designed in one go so the style stays consistent.

### 9.4 Technical placement

- `Views/FileCard.swift` — the card component; it feeds off `ToolTrace.filePath`, no new model field is needed.
- `TacetReply` separates file-producing traces out of the chip list and draws them as cards; the other `written`/`failed` chips stay exactly as they are.
- `Design/FileIcon.swift` — `icon(extension:) -> Image` + `typeLabel(extension:) -> String`; pure functions, verified in SelfTest with the 20 types + fallback + synonym cases.

---

## Appendix — Decision record

```
Decision: Live step display is added under the name "Timeline", as a
       pure-observer layer; tool chips remain the single source of truth.
Context: The user asked for something like Claude Code's collapsible step
        stream ("let it show what it's doing stage by stage, and let us go
        into detail when we want to").
Options: A (enrich the chips, no separate layer) · B (Timeline: a time axis
        above the chips, this spec) · C (Claude Code one-to-one: state verbs
        + everything collapses)
Chosen: B — A cannot carry the pipeline steps (routing, writing) and does not
        give the live single-line experience; C clashes with the brand in two
        places: fabricated state verbs (no dramatization) and side effects
        being collapsible (Tacet does not hide what it does).
Deliberately deferred: the visibility of memory injection,
        multi-line live streaming.
Re-evaluation trigger: if users are observed never opening the collapse line
        (if the line is noise), the collapse line is shown only on
        multi-step turns; if it is opened often, default-open is considered.
```

---

## 10. Implementation plan (file map)

| Step | File | Work |
|---|---|---|
| 1 | `Models/TimelineStep.swift` + `Message.stepsData` | Codable step + persistence (the `tracesData` pattern) |
| 2 | `Services/TimelineRecorder.swift` | lifecycle, tool binding, interruption |
| 3 | `ModelService` + `ToolExecutor` | one-line notifications at the event points (behavior does not change) |
| 4 | `Views/TimelineRibbon.swift` + `Views/TimelineLine.swift` | live line + collapsible line; `TacetReply`/`ChatView` integration (moving the read chips into the collapse) |
| 5 | `Design/FileIcon.swift` + asset catalog | icons for the 20 types, mapping + fallback + `UTType` label |
| 6 | `Views/FileCard.swift` | the card; splitting file traces out into cards in `TacetReply` |
| 7 | `L10n` + `Localizable.xcstrings` | step strings + card strings ("the file is no longer on the device") |
| 8 | `SelfTest` + `Evaluation` | the §6 cases + icon mapping/fallback cases |

The order is deliberate: 1–2 are testable without an interface; 3 alone changes nothing visible; the current chip view stands unchanged until 4 (safe intermediate deliveries).
