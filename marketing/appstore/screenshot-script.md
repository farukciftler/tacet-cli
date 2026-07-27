# Tacet — App Store screenshot script

> **This document will be run again.** The brand became **Tacet** and every existing
> frame was captured with the old brand (`README.md` §5.4). The plan below is not an
> archive, it is **the plan for the recapture**. Every sentence described below as
> "will appear in frame" must be read from the new build: the app copy has changed too,
> so the quotations in the script may not match verbatim.

This document is the capture plan the agent in the next phase will **act out literally**
in the simulator. No invented interface is drawn, nothing the app cannot do is shown, and
no real personal data enters the frame. Every frame comes from the real app.

Output directory: `marketing/appstore/` from the repository root
Intermediate files: `screenshots/` under the session's scratchpad directory

> **Caption language note.** The caption strings below are given in English. Because the
> whole set must be recaptured (§0.0), the previous `tr-TR` caption strings are not
> ready-to-ship assets; they are in the git history and must be re-authored by the Turkish
> copywriter during the recapture round, alongside the TR frames.

---

## 0. READ FIRST — three obstacles

### 0.0 The brand (NEW — the real reason for this round)

The product's name is **Tacet**. The OLD BRAND must appear nowhere in frame: the top-bar
brand mark, the input field placeholder ("ask …"), the welcome body copy and the sentence
on the approval sheet all carry this name. Before capturing, **take one frame and verify
by eye that the correct build is installed** — if an old build is left hanging in the
simulator the whole set goes in the bin again (which is exactly what happened last round).

Spelling: **Tacet**, initial capital. In Turkish copy, suffixes take an apostrophe:
Tacet'i, Tacet'e, Tacet'in, Tacet'te, Tacet'ten.

### 0.1 The absolute privacy sentence in the app — LARGELY RESOLVED

Last round this item was a BLOCKER: the welcome, the empty state and the brand sheet
carried absolute sentences such as "…runs entirely on this device… it does not go to the
internet.", and the frames were therefore captured **cropped**.

**Those sentences have been fixed in the code.** The welcome and empty state are now in
the conditional: "Tacet's core runs on this device. Web search and connections engage
only if you turn them on, and only in plain sight." There are no matches left in the brand
sheet or in the permission/tool-chip copy either (the scan is in `README.md` §6.2).

Consequence: **the obligation to crop is gone.** The welcome frame (#8) can be captured
with the full framing, and the connections frame (#7) can go back into the set.

The **one** remaining item: the screen-bottom note in `Tacet/Views/Settings.swift`,
"Everything stays on this device." It sits at the very bottom of the Settings screen,
after the WEB SEARCH and CONNECTIONS sections. **The bottom of the Settings screen must
not be legible in any frame** — or, until the sentence is fixed, Settings must be captured
without scrolling.

NOT a problem (conditional, honest phrasing — may stay in frame): in the web search section
of Settings, "No search server. Connect your own SearXNG server and Tacet can search the
web. Until you do, web search stays off.", and its equivalent on the Connections board.

### 0.2 Model dependency

Tacet's reply comes from on-device FoundationModels; **the same prompt does not give the
same sentence on every run**. Most frames require a check of "did the model do what was
expected". There is a **backup plan** below for every frame. The rule: if the expected
result did not appear, **try again**; never edit or composite by hand.

---

## 1. Preparation (once, before capturing)

**Device:** `iPhone 17 Pro Max` — UDID `F74085BF-8F5B-4588-8578-92DDE02EDE3F` (6.9", 1320×2868).
This is the App Store's mandatory size set; Apple derives the 6.5"/6.1" sets from it.

```
xcrun simctl boot F74085BF-8F5B-4588-8578-92DDE02EDE3F        # may already be booted
xcrun simctl status_bar F74085BF-8F5B-4588-8578-92DDE02EDE3F override \
  --time "9:41" --batteryState charged --batteryLevel 100 \
  --cellularBars 4 --wifiBars 3 --dataNetwork wifi
xcrun simctl ui F74085BF-8F5B-4588-8578-92DDE02EDE3F appearance light   # or dark
xcrun simctl io F74085BF-8F5B-4588-8578-92DDE02EDE3F screenshot <path>.png
```

**Permissions** (granted up front so no system alert enters the frame):

```
xcrun simctl privacy F74085BF-8F5B-4588-8578-92DDE02EDE3F grant calendar  zortproductions.tacet
xcrun simctl privacy F74085BF-8F5B-4588-8578-92DDE02EDE3F grant reminders zortproductions.tacet
```

**Demo calendar data** (by hand from the simulator's Calendar app; NO real person's name):

| Title | Time |
|---|---|
| Team meeting | tomorrow 10:00–11:00 |
| Dentist | tomorrow 15:30 |
| Parcel delivery | tomorrow 18:00 |

**Demo document** (for frame #4) — generate it in the scratchpad and drag-drop it into the
simulator's Files: `quarter-summary.pdf`, 2 pages, content: a generic quarterly summary
(region/revenue/quantity rows, a made-up neutral company name, no person's name).

**General framing rules (they apply to every frame):**
- The keyboard must be closed (one tap on empty space) — let the frame breathe. Exception: #7 (voice input).
- The reply stream must have finished; no blinking cursor in frame.
- NO accent colour, green dot, badge, gradient or shadow — there are none in the interface either; none will be added afterwards.
- The same chip must not be the lead character in two different frames.

---

## 2. Rationale for the ordering

The first 2–3 frames are visible in the App Store gallery preview. The order was built on
this logic:

1. **#1 the product's "it does things" side** — the only visual block recognisable even in a
   small preview is a table; it gives the impression of "a tool that produces work", not
   "an assistant", in the first second.
2. **#2 and #3 the product's real difference** — tool trace transparency. First what the
   trace is (#2, with the product's own hint box, i.e. the interface's own words rather than
   a claim), then the raw layer beneath the trace (#3). Put back to back, "it does not tell,
   it shows" lands in a single move.
3. **#4–#5 the scope widens** — document reading, memory. The "not just chat" message.
4. **#6 honest privacy** — the exit gate. Its position at the end is deliberate: first you
   see what the product does, then the promise "on the way out the permission is yours"
   arrives with proof.
5. **#7–#8 the input surface and the first minute** — the last question of a user who has
   already decided ("how do I use it, what happens on first launch").

---

## 3. The frames

### Frame 1 — A table and an Excel from natural language

1. **Purpose:** proves that Tacet produces a real spreadsheet from a single sentence and draws the result as a table in the chat.
2. **Screen:** chat.
3. **Acting it out:**
   1. Open the app, **new chat** from the top bar (pencil icon, top right).
   2. Type into the input field and send:
      `Produce an excel for a weekly class timetable: Monday to Friday, two columns for morning and afternoon.`
      (The word "excel" is MANDATORY so that it enters the document profile — `IntentPicker` looks for that word.)
   3. Wait for the reply to finish (tool chip "Creating XLSX…" → "XLSX created · <name>.xlsx").
   4. Close the keyboard, scroll so that the chip + table + file card are visible together.
   5. Take the screenshot.
4. **In frame:** the top bar (serif "Tacet"), the user bubble (the prompt), the tool chip
   `XLSX created · …xlsx`, the in-chat table (hairline grid), the **"Download Excel"** chip
   below the table and the `FileCard` (file name + "Spreadsheet · XLSX").
   **Not in frame:** the keyboard, empty-state copy, a blinking cursor.
5. **Caption (en):** **"Say it. The table appears."**
6. **Mode:** light.
7. **Backup plan:** If the model does not draw the table as markdown or does not produce a
   file, (a) reshape the prompt to `Make a weekly class timetable table and save it as xlsx.`,
   (b) if that still fails, ask for something smaller: `Produce a 3-row shopping list excel:
   item, quantity, price.` If the table is not drawn on the third attempt either, capture with
   just the chip + file card and change the caption to **"Say it. The file gets made."**

---

### Frame 2 — The tool trace: Tacet leaves a mark of what it touched

1. **Purpose:** shows that when Tacet actually does something it leaves a trace on screen,
   and that what that trace is gets explained in the product's own words. **The product's
   strongest difference.**
2. **Screen:** chat + the one-time tool trace hint box (`ToolHint`).
3. **Acting it out:**
   1. The hint is one-time only; if "Got it" was pressed before, reset first:
      **Settings > GETTING STARTED > "Show the welcome again"** (`WelcomeSetting.reset()`
      re-opens the tool trace hint too). Dismiss the welcome that appears.
   2. Open a new chat.
   3. Type and send: `Add an event called "Team retro" tomorrow at 14.00.`
   4. When the reply finishes the thread holds: `Adding event…` → **`Event added`**
      (with the approval mark, in ink — a write trace) and immediately below it the hint box:
      *"The line above is a tool trace: Tacet leaves every tool it touches here…"*
   5. **DO NOT press "Got it".** Close the keyboard, take the screenshot.
4. **In frame:** the user bubble, the `Event added` chip, the tool trace hint box (fully
   legible), Tacet's short serif confirmation sentence.
   **Not in frame:** the calendar permission system alert (permission was granted up front),
   real calendar content.
5. **Caption (en):** **"It doesn't claim. It shows."**
6. **Mode:** dark. (A hairline-framed chip reads best on a dark ground.)
7. **Backup plan:** If the model does not call the calendar tool, or a "Time not understood"
   chip drops, change the time format: `Create a Team retro event tomorrow at 14:00.`
   If the calendar does not work at all, switch to the **reminder**: `Remind me to collect
   the parcel tomorrow at 18.00.` → the chip `Reminder set · 18.00`. The caption stays the
   same (the frame still describes a write trace).

---

### Frame 3 — The raw layer beneath the trace

1. **Purpose:** proves that transparency is a product feature, not talk: tapping the chip
   shows the raw content that **went to** and **came back from** the tool.
2. **Screen:** chat + the `ToolChipDetail` sheet.
3. **Acting it out:**
   1. Stay in frame 2's chat (say "Got it" to the hint box so the framing is clean).
   2. Do a fresh read turn so the detail is populated: send `What's on tomorrow?`
   3. When the reply finishes, tap the `Calendar read · 3 events` chip → the detail sheet
      opens (the chip text in the title, **Input** and **Output** monospace blocks below).
   4. Take the screenshot once the sheet is fully open.
4. **In frame:** the sheet title (the chip text), the "INPUT" and "OUTPUT" labels, the
   monospace blocks, the chat blurred behind.
   **Not in frame:** real personal calendar data — the output must contain only the three
   demo events you prepared. If there is anything unexpected in the output do not capture;
   clean the data and try again.
5. **Caption (en):** **"Tap to see the raw exchange."**
6. **Mode:** light.
7. **Backup plan:** If the chip is one that produced a file, tapping opens a preview rather
   than the detail — which is why a **read chip** was chosen. If the detail comes back empty
   (`—`) try another read chip (`Reminders read · N pending`). If none is populated, swap this
   frame with Frame 6 and put the reminder write chip in #3's place.

---

### Frame 4 — It reads and summarises an attached document

1. **Purpose:** shows that a document attached to the chat is genuinely read and summarised.
2. **Screen:** chat (attached document chip + read chip + serif summary).
3. **Acting it out:**
   1. Drag `quarter-summary.pdf` into the simulator (Files > On My iPhone).
   2. Open a new chat in Tacet, press the **paperclip** button to the left of the input
      field, pick the file.
   3. The attached document chip appears above the input field. Type and send:
      `Summarise this document in three bullets.`
   4. When the reply finishes, close the keyboard and take the screenshot.
4. **In frame:** the attached document chip (file name), the tool chip
   `PDF read · quarter-summary.pdf`, Tacet's serif summary (3 bullets).
   **Not in frame:** the Files app, the file picker, a document belonging to a real company.
5. **Caption (en):** **"Attach a document, get the gist."**
6. **Mode:** dark.
7. **Backup plan:** If the PDF is not read, produce the same content as `.docx` and attach
   that (the DocxEngine path is more stable). If the summary comes back as a flat paragraph
   rather than three bullets, accept it — if that is how the product works, that is how it is
   shown; forcing the prompt with "in three bullets" is dropped on the second attempt.

---

### Frame 5 — Memory: what it learned stays where you can see it, in your hands

1. **Purpose:** shows that Tacet learns from the conversation and that what it learned is
   visible/editable/deletable.
2. **Screen:** the Memory board (top left list icon > **Memory**).
3. **Acting it out:**
   1. Open a new chat and send two messages (wait for the reply in between):
      - `I start work at 08.00 in the mornings, put meetings in the afternoon.`
      - `I always want documents as xlsx.`
   2. Memory extraction is triggered on a chat change / on going to the background
      (`MemoryService.trigger`). **Go to Home, wait 5 s, return to the app** — or open a
      new chat.
   3. Top left list icon > **Memory**. The notes should be listed.
   4. Take the screenshot.
4. **In frame:** the "Memory" title, 2–3 learned note rows (with their on/off switch), the
   "Delete all" row. **Not in frame:** real personal information; only notes derived from
   the two demo sentences above. If there is anything unexpected in the notes, clear it with
   "Delete all" and start over.
5. **Caption (en):** **"What it learns stays here, in your hands."**
6. **Mode:** light.
7. **Backup plan:** If extraction produces no notes, write the sentences more in the register
   of a "lasting preference" (`I always …`, `I prefer …`) and repeat the trigger. If no note
   appears in two rounds, replace this frame with the **Capabilities catalogue** (the "What
   can Tacet do?" sheet in the empty state), caption **"Everything it can do, listed."**

---

### Frame 6 — On the way out, the gate is yours

1. **Purpose:** shows the product's honest privacy frame with proof: the core runs on the
   device, and everything that leaves is **visible and approved**.
2. **Screen:** `ApprovalSheet` ("Send this?") — exactly the content that will leave the device.
3. **Acting it out (the gate only opens while the session is "dirty"):**
   1. From the web search section of Settings, add your own SearXNG server and **turn search
      on**. (The address will not enter the frame; this step exists only to arm the gate.)
   2. Open a new chat. **First** have personal data read: `What's on tomorrow?` → the session
      becomes dirty.
   3. In the same chat, ask for something that will go to the web:
      `Search this on the web: agenda template for a business meeting.`
   4. The chip `search server · awaiting approval` drops into the thread and the approval
      sheet opens. **DO NOT DECIDE** — take the screenshot while the sheet is open.
4. **In frame:** the "Send this?" title, the `search server · web_search` row, the
   **"GOING TO YOUR SERVER:"** label and the monospace content block, the "If you do not
   send it, Tacet skips this step…" sentence, the **Do not send / Send** buttons.
   **Not in frame:** the server address/domain, the API key, a personal query.
5. **Caption (en):** **"Nothing leaves before you see it."**
6. **Mode:** dark.
7. **Backup plan (likely — prepare this in advance):** If there is no server / it is
   unreachable, or the gate does not open, capture the **Connections board**: top left >
   Settings > Connections.
   In frame: the "Connections" title, **"No servers connected."**, "Connect your own MCP
   server and Tacet can use its tools. **Until you do, this surface stays closed.**"
   (This sentence was in the absolute register last round and made the frame unpublishable;
   it has been fixed in the code — it may now stay in frame.)
   Caption: **"Off by default. You open it."**
   This backup is honest and on-brand; it is not lower quality — if the capture is risky,
   prefer it directly.

---

### Frame 7 — Speak, read it before you send

1. **Purpose:** proves that voice input exists and that the recognised text is shown to the
   user **before it is sent**.
2. **Screen:** chat, the input field full, not yet sent.
3. **Acting it out:**
   1. Open a new chat (leave 1–2 turns of older messages above so the screen does not look
      empty — you can use frame 2's chat, but with the hint box closed).
   2. Press the **microphone** button in the input field and dictate (read into the Mac's
      microphone): *"Prepare an agenda for the team meeting tomorrow at 9 in the morning."*
   3. When the text lands in the input field, **do not send**. Take the screenshot while
      dictation is active (the microphone visibly live).
4. **In frame:** the recognised text inside the input field, the active microphone button,
   the send button; above, a calm fragment of a reply from the previous turn.
   **Not in frame:** system dictation bubble / keyboard emoji row noise (the keyboard may
   stay open, but if it swamps the lower half, capture the frame with the keyboard closed and
   the text in place).
5. **Caption (en):** **"Speak. Read it before you send."**
6. **Mode:** light.
7. **Backup plan (a risky frame):** If the on-device recognition model is missing in the
   simulator, the microphone goes inactive and a warning appears ("On-device recognition is
   not ready for this language"). **Do not capture that warning.** In that case drop this
   frame entirely and put the **reminder** frame in its place: `Remind me to collect the
   parcel tomorrow at 18.00.` → the chip `Reminder set · 18.00`, caption
   **"Say it; the reminder is set."**
   (If the reminder was already used in frame 2's backup, use `How many days until New Year?`
   → the `Days counted` chip, caption **"It does the date math."** instead.)

---

### Frame 8 — In the first minute, work rather than narration

1. **Purpose:** shows that onboarding is not a slideshow/tour but a single real task.
2. **Screen:** Welcome (`Welcome`, in its model-ready state).
3. **Acting it out:**
   1. Open the welcome via **Settings > GETTING STARTED > "Show the welcome again"**.
   2. **CROPPING IS NO LONGER NEEDED (see §0.1).** Last round the top third of the page was
      left out of frame because of the absolute privacy sentence; that sentence has been fixed
      in the code. Capture the page **in full** — the sentence at the top,
      *"Tacet's core runs on this device. Web search and connections engage only if you turn
      them on, and only in plain sight."*, must now STAY in frame: it is the only place that
      states the product's privacy frame honestly on a single screen.
   3. The block that must appear: the two body sentences at the top, the **"PICK A TASK"**
      label, the three task chips (`What's on tomorrow?` / `Remind me to call at 18.00` /
      `Make a document out of this week's notes`), the sentence below them
      *"The task you pick lands in the input field… Tacet leaves every tool it touches on
      screen"*, and **"I'll write my own"**.
   4. Take the screenshot. **Verify by eye:** not a single word of the OLD BRAND may be in
      the frame (§0.0).
4. **In frame:** the block above + the Tacet brand mark (if it fits at the top).
   **Not in frame:** the old brand; the "Everything stays on this device." note at the bottom
   of the Settings screen (this frame does not show Settings, but mind the backup plan).
5. **Caption (en):** **"No tour. One real task."**
6. **Mode:** dark.
7. **Backup plan:** If the page does not fit on one screen, crop from the **bottom**, not the
   top; keep the two body sentences and the three task chips in frame, "I'll write my own"
   may drop.

---

## 4. Cover / caption layout

One template, the same across eight frames. The goal: no "design" should be noticeable
between frames, only the screens should differ.

**Canvas:** 1320 × 2868 px (the 6.9" App Store size).

| Element | Value |
|---|---|
| Background | light frame: `#FFFFFF` · dark frame: `#141413` (the app's `ground` token) |
| Caption position | **ABOVE the image** — 150 px from the top edge, left aligned, 110 px left/right margin |
| Caption typography | New York (serif), medium, 76 px, line height 1.25, at most **2 lines** |
| Caption colour | `#1C1C1A` (light) · `#ECECEA` (dark) |
| Screenshot | scale to 1100 px wide, centre horizontally, top edge y = 560 px |
| Screen corner | 62 px rounding |
| Screen frame | 1 px hairline: `#E9E9E4` (light) · `#2A2A28` (dark) — no other frame |
| Device mockup | **NONE** (no phone frame, hand or environment image is drawn) |
| Shadow / gradient / accent colour | **NONE** |
| Bottom space | not filled; the space is part of the design |

Additional rules:
- Captions use a full stop, never an exclamation mark. The brand name is **Tacet**
  everywhere — initial capital. (The old "lower case everywhere" rule was for the old name;
  it is INVALID.)
- If the critical part of the content is at the bottom of the screen (e.g. the input field),
  crop the screenshot from the top; do not change the scale, and never use two different
  scales across frames.
- Two separate sets are produced, TR and EN: `tr/01-…png`, `en/01-…png`. The file name
  carries the order: `01-excel.png`, `02-tool-trace.png`, `03-raw-exchange.png`,
  `04-document.png`, `05-memory.png`, `06-approval-gate.png`, `07-voice.png`,
  `08-welcome.png`.
  (Note: the set currently on disk contains `05-capabilities` and `07-connections` instead —
  those are the backup-plan outcomes of frames 5 and 6, not the planned frames.)
- For the EN set, switch the simulator language to English and play **the same script with
  English prompts** again (the interface strings come through in English from the String
  Catalog). Writing an English caption on top of a TR frame is **forbidden** — the interface
  inside the frame stays Turkish, which makes it a lie.

---

## 5. Difficulty warnings — summary table

| Frame | Risk | What to do |
|---|---|---|
| 1 | The model may not produce the table/xlsx | Try the prompt with the three variants in §3.1; failing that, the chip + file card framing and a caption change |
| 2 | The hint is one-time; the calendar tool may not trigger | Reset the welcome from Settings; if the calendar does not work, switch to the reminder |
| 3 | The detail may come back empty (`—`); a file chip opens a preview instead of the detail | Pick a read chip; if empty, try another read chip |
| 4 | PDF parsing may fail | Produce the same content as .docx |
| 5 | Memory extraction may produce no notes | Write in the preference register + repeat the trigger; failing that, switch to the Capabilities catalogue |
| 6 | Requires a SearXNG/MCP server; the gate only opens on a dirty session | Do not break the order (calendar first, then search); if it is risky, go straight to the Connections board backup |
| 7 | On-device dictation may not work in the simulator | DO NOT capture the warning screen; replace with the reminder or day-counting frame |
| 8 | The absolute privacy sentence can enter the frame | Scroll it out; if that fails, drop the frame (7 frames is acceptable) |
| all | If FoundationModels is unreachable in the simulator, an "Apple Intelligence is off" row appears in the top bar | Apple Intelligence must be on in macOS; if the row is there no frame is captured, this is fixed first |

**General retry policy:** at most 3 attempts per frame. If the third also fails, go to the
backup plan; dropping the frame from the list is preferable to forcing it. **Under no
circumstances** is the output edited by hand, an interface imitation drawn, or text falsified
on top of an image.

**Publication rule:** none of the produced files is uploaded to the App Store or sent
anywhere. Output is written only under `marketing/appstore/`.
