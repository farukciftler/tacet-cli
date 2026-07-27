# Tacet — App Store Connect delivery package

This folder carries everything to be uploaded to App Store Connect. The order below
is the order of the screens you open: **copy first, then screenshots, submission last.**

> **Read first — THE COPY IS READY, THE SCREENSHOTS ARE NOT.** In this round the brand
> became **Tacet** (the previous name is no longer mentioned anywhere). All the COPY in
> this folder was updated for the new name and the character counts were recounted.
> The PNGs under `screenshots/` were **not touched**: they were all captured before the
> brand change and most of them still show the **OLD BRAND** (a frame-by-frame
> measurement is in §5.4).
>
> If the package is uploaded as it stands, the store page will show **Tacet in the copy
> and the OLD BRAND in the screenshots**. Do not submit before the screenshots are
> recaptured. The two remaining placeholders (support + privacy URL) are in §6.

---

## 1. What is in the package

```
marketing/appstore/
├── README.md              ← this file (upload instructions)
├── preview.html           ← see the whole set at a glance in a browser
├── store-copy.md          ← the store copy (TR + EN), character counts verified
├── screenshot-script.md   ← the capture script for the frames (archive; does not go to the store)
└── screenshots/           ← ALL CAPTURED WITH THE OLD BRAND (§5.4)
    ├── raw/               6 frames — no captions
    ├── captioned-tr/      7 frames — Turkish captions
    ├── captioned-en/      7 frames — NOT PUBLISHABLE, see READ-DO-NOT-UPLOAD.txt
    └── pending/           1 frame — additionally carries an absolute privacy sentence
```

All the PNGs are **1320 × 2868, RGB, no alpha channel** — the exact size of the App
Store's 6.9" iPhone (iPhone 16 Pro Max class) slot. Apple accepts this single size and
scales down to smaller devices itself; you do not need to prepare a second size.

### Frame list

Captions are given in English. The `tr-TR` caption strings are not reproduced here: the
whole set has to be recaptured (§5.4), so they are not ready-to-ship assets — they are in
the git history and must be re-authored during the recapture round.

| # | File | What it shows | Mode | Caption (en) |
|---|---|---|---|---|
| 1 | `01-excel` | Prompt → tool trace → Excel file card | light | Say it; let it produce the file. |
| 2 | `02-tool-trace` | The "reminder set" chip, with the approval mark | dark | It does not say what it did, it shows it. |
| 3 | `03-raw-exchange` | The tool detail page: raw input/output | light | Tap; see what went out and what came back. |
| 4 | `04-document` | PDF read → summary | dark | Attach the document, get its summary. |
| 5 | `05-capabilities` | The "what it can do" catalogue | light | The full list of what it can do. |
| 6 | `06-approval-gate` | The submission approval sheet | dark | If it touched your personal data, it asks first. |
| 8 | `08-welcome` | Welcome — "pick a job" | dark | Instead of explaining, it does a job. |

There is no number 7: it is in the `pending/` folder, with its reason written there.

Number 8 is also absent from `raw/` — the uncropped welcome frame was left out of the
set because its framing carried a privacy sentence that had not been corrected that day.
**That sentence has been fixed in the code** (§6.2); on a recapture, number 8 can enter
the `raw/` set in full as well.

---

## 2. Upload order (App Store Connect)

**My Apps → Tacet → iOS App → 1.0 Prepare for Submission**

### 2.1 Screenshots — App Previews and Screenshots

> **DO NOT UPLOAD YET.** The existing frames show the old brand (§5.4). The order below
> applies to the **recaptured** set; because the file names will stay the same, the order
> stays the same too.

Set the localisation picker (top left of the page) to **Turkish**, then drag the contents
of `screenshots/captioned-tr/` in **this order**:

| Order | File | Why it is here |
|---|---|---|
| 1 | `01-excel.png` | The first frame appears alone in the list — this is the only frame showing a concrete output |
| 2 | `06-approval-gate.png` | The product's difference: everything that leaves is visible first |
| 3 | `02-tool-trace.png` | 1-3 are visible without scrolling; together the three tell "ask → have it done → see it" |
| 4 | `05-capabilities.png` | Open up the scope here |
| 5 | `03-raw-exchange.png` | The proof of the transparency claim |
| 6 | `08-welcome.png` | The first-launch experience |
| 7 | `04-document.png` | **Conditional** — read the defect in §5 and decide |

The first three frames are critical: in App Store search results and at the top of the
product page, only these three are visible without scrolling.

**Ordering rule:** after uploading you can reorder by dragging; the sequence number comes
from the board's own layout, not from the file name.

### 2.2 Screenshots — the English localisation

**Not possible right now.** The *captions* of the frames in `captioned-en/` are English but
their *interface* is Turkish. Showing a Turkish interface in the English store is
misleading. What needs to be done is in §6.3.

Interim solution: do not open the English localisation at all. For unlocalised languages
App Store Connect shows the primary language's (Turkish) screenshots — which is better
than showing the wrong screenshots.

### 2.3 Copy

All of it is in `store-copy.md`. The character counts were **recounted after the brand
change** (the old name 4 → `Tacet` 5 characters); none exceeds its limit.

| App Store Connect field | Limit | Source | Count |
|---|---|---|---|
| Name | 30 | §1 | TR 25 · EN 27 |
| Subtitle | 30 | §2 | TR 25 · EN 28 |
| Promotional Text | 170 | §3 | TR 164 · EN 168 |
| Description | 4000 | §4 | TR 2 949 · EN 3 106 |
| Keywords | 100 | §5 | TR 92 · EN 99 |
| What's New in This Version | 4000 | §6 | TR 452 · EN 518 |
| Support URL | — | §7 | **PLACEHOLDER** |
| Marketing URL | — | §7 | optional |
| Privacy Policy URL | — | §7 | **PLACEHOLDER** |
| App Review → Notes | 4000 | §9 | 3 768 |

**Warning about the Name field — THE RULE CHANGED.** The name is now **Tacet**: initial
capital, the rest lower case. The previous version of this package said "the brand is
lower case everywhere"; that rule **belonged to the old name and is no longer valid**. The
lower-case form `tacet` is only the command name of the desktop Rust shell; it does not
go into the store.

Do not write `TACET`, `tacet` or `TaCet`. Turkish suffixes take an apostrophe: Tacet'i,
Tacet'e, Tacet'in, Tacet'te, Tacet'ten.

**The Promotional Text privilege:** this field can be changed without submitting a new
version. Changing the Description requires a new version review.

---

## 3. The privacy label — App Privacy

**App Store Connect → App Privacy → Get Started**

Source: `store-copy.md` §8. The short answer:

| Question | Answer |
|---|---|
| Do you or your third-party partners collect data from this app? | **No** |
| (result) | "Data Not Collected" |

This answer rests on Apple's own definition: collection is data reaching **the developer
or their third party**. Tacet has no server. Network traffic goes only to the address the
user entered themselves; the receiving end is the user.

**But while giving this answer, do not neglect this:** the privacy policy must explain the
web search and MCP surfaces explicitly (the policy content is itemised in §7). Ticking
"Data Not Collected" and then never mentioning the network surfaces in the policy is a
claim-behaviour inconsistency, and it is the most likely reason for a rejection.

If the reviewer objects, a prepared fallback statement is waiting in `store-copy.md` §8 —
**do not volunteer it**, only if asked.

---

## 4. Other submission fields

| Field | Recommendation | Rationale |
|---|---|---|
| **Primary Category** | Productivity | The product does work; Utilities is a weaker signal |
| **Secondary Category** | Utilities | |
| **Age Rating** | 4+ | No user-generated sharing, no messaging, no ads, no browser, no purchases |
| **Price** | (your decision) | |
| **Availability** | All regions | |
| **Content Rights** | Does **not** contain third-party content | |
| **Sign-in required** | **No** | |
| **Demo account** | Not needed | |
| **Export Compliance** | Only standard HTTPS/TLS is used | The app does not do its own encryption; Keychain and URLSession are system services |

### Age rating detail
While filling in the questionnaire, everything is **None / No**. The only question that
gives pause is "Unrestricted Web Access": the answer is **No** — Tacet has no embedded
browser, the user cannot browse arbitrary web pages; only text results come back from the
search server they set up themselves.

---

## 5. Known defects — decide before submitting

### 5.1 `04-document.png` — a conditional frame

The summary text in the frame has missing diacritics in three words (the correct spellings
carry the accented characters the frame is missing).

The cause: the demo PDF was written in ASCII; Tacet summarised the document faithfully. So
this is not an application bug but a demo-content bug. **But a user seeing this in the
Turkish store will not know the difference — they will read it as a typo.**

A second defect: the prompt says "summarise this document **in three bullets**", while the
reply is a flat paragraph. The instruction was visibly not carried out.

**Decision:** fixing it requires no code change — if the frame is recaptured with a demo
PDF whose spelling is correct, the problem ends. Until it is recaptured, put this frame in
position **7** (never let it into the first three) or drop it entirely. Six frames is a
sufficient set.

### 5.2 `07-connections.png` — pulled

The framing contains the sentence **"Unless you connect, ‹OLD BRAND› does not go online."**
(the frame was captured with the old brand; the name is not repeated here, it is visible in
the framing). It is absolute and wrong: web search is an independent surface and, when it
is on, it goes online even if no MCP is connected. Detail and the suggested code fix:
`screenshots/pending/READ-DO-NOT-UPLOAD.txt`.

### 5.3 `08-welcome.png` — cropped (the reason for the crop is NO LONGER VALID)

The top third of the welcome screen was left out of frame, because the sentence
"‹OLD BRAND› runs entirely on this device… it does not go to the internet." was sitting there.

**That sentence was fixed in the code** (see §6.2): `Views/Welcome.swift` now says
"Tacet's core runs on this device. Web search and connections engage only if you turn them
on, and only in plain sight." So the crop is no longer needed — but the frame will be
recaptured anyway because of §5.4, and the crop goes away at that point.

### 5.4 EVERY FRAME SHOWS THE OLD BRAND (BLOCKER)

The screenshots were captured BEFORE the brand change. The binary files under
`screenshots/` were not touched in this round (regenerating them is separate work), which
is why **if the package is uploaded as-is the store screenshots will read the OLD BRAND** —
Tacet in the copy, the old brand in the images. A reviewer will count that as an
inconsistency; even if they do not, the user will see it.

Measured frame by frame (by looking at the images, not by guessing):

| Frame | Is the old brand visible | Where |
|---|---|---|
| `01-excel` | **yes** | old brand in the top bar · "ask ‹old brand›" in the input field |
| `02-tool-trace` | **yes** | old brand in the top bar · "ask ‹old brand›" in the input field |
| `03-raw-exchange` | no | the tool detail page; no brand mark in the framing |
| `04-document` | **yes** | old brand in the top bar · "ask ‹old brand›" in the input field |
| `05-capabilities` | no | the "what it can do" page; the brand does not appear in the visible framing |
| `06-approval-gate` | **yes** | old brand in the top bar · in the approval sheet, "If you do not send it ‹old brand› skips this step…" |
| `08-welcome` | **yes** | the old brand appears three times in the body copy ("…will ask at exactly that moment", "…every tool it touches…", "…what can it do?") |
| `07-connections` (pending) | **yes** | "Unless you connect, ‹old brand› does not go online." |

This holds for all three sets:

- `captioned-tr/` — the table above was derived by reading this set.
- `captioned-en/` — **the same phone body**; verified by pixel comparison (identical to the
  TR frames apart from the caption band), so it carries the same brand residue. This set
  was already unpublishable because of the interface language (§2.2).
- `raw/` — looked at individually; `01`, `02`, `04`, `06` show the old brand, `03` and `05`
  do not.

**Moreover the frames are old not only in terms of the brand but also in terms of the
COPY:** the welcome text in the `08-welcome` frame has changed completely in the code. So
"let us just retouch the name in the top bar" is not a solution — the frames must be
recaptured.

**To do:** recapture the raw frames in the simulator with a new build, then re-run the
caption generator script. Even though `03` and `05` are technically brand-clean, it is
safer to recapture them in the same run: a set coming from a single run is internally
consistent in typography and status bar.

---

## 6. What you have to do by hand — in priority order

### 6.0 EVERY SCREENSHOT MUST BE RECAPTURED (BLOCKER — new)
The brand changed; every frame was captured with the old brand. Detail and the
frame-by-frame measurement are in §5.4. This work also covers §6.3 (the English set): both
sets come out of the same recapture round, they are not two separate jobs.

### 6.1 The privacy policy and support page must go live (BLOCKER)
Both are mandatory and both are currently placeholders. **You cannot submit without a
working URL.** The 10 items the privacy policy must contain are written in `store-copy.md`
§7 — items 4 and 5 in particular (what goes out when web search and MCP are on) must not be
skipped; the defensibility of the "Data Not Collected" statement in §3 rests on those two
items.

The support page needs at minimum: what it is, a contact route, the Apple Intelligence
requirement, and an FAQ (why the microphone is inactive, why the model does not answer, how
to turn on web search).

### 6.2 Absolute privacy sentences in the code — LARGELY FIXED, one remains

The previous version of this item was a nine-line "to fix" list. That list is **stale**:
almost all the sentences it counted have been fixed on the iOS side. Re-scanned (run while
this package was being updated, not a guess):

```sh
grep -rn --include="*.swift" -i \
  "stays on this device\|never goes online\|does not go online\|\
entirely on this device\|does not leave the device" Tacet/
```

Result:

| Old list | State today |
|---|---|
| `Welcome.swift` "‹old brand› runs entirely on this device… it does not go to the internet." | **fixed** — now "Tacet's core runs on this device. Web search and connections engage only if you turn them on, and only in plain sight." |
| `EmptyState.swift` the same sentence | **fixed** — the same new sentence |
| `TopBar.swift` the same sentence | **gone** — the scan finds no match |
| `Settings.swift` "Unless you connect, ‹old brand› does not go online." | **fixed** — "…until you do, web search stays off." (surface-specific, not absolute) |
| `ConnectionBoard.swift` the same sentence | **fixed** — the rationale is written at the head of the file too |
| `Capabilities.swift`, `PermissionSection.swift`, `ToolChip.swift` | **gone** — the scan finds no match |
| `Settings.swift:727` "Everything stays on this device." | **STILL STANDS** |

**The one remaining item and why it still matters:** the `footnote` inside `Settings.swift`
is the note at the **very bottom** of the Settings screen. The same screen contains the WEB
SEARCH and CONNECTIONS sections; writing "Everything stays on this device." underneath
those two sections is precisely the absolute claim rejected at the head of this package.

(Note: the previous version of this item said the same sentence was correct in
`Services/DocumentContext.swift` for the **documents folder**. The re-scan finds no such
sentence there any more, so that parenthetical no longer applies — the only live occurrence
is the Settings footnote.)

Recommendation: either drop the footnote or write out its scope — for example "Your chats,
your memory and your documents stay on this device." The decision belongs to the `Tacet/`
side; this package only raises the flag.

There is no longer a code obstacle to frame 07 coming back and to 08 being captured
uncropped — the obstacle is §5.4 (the frames were captured with the old brand).

### 6.3 The English screenshot set must be recaptured
Set the simulator language to English, play the same script with English prompts,
recapture the raw frames, then re-run the generator script. The layout and typography are
ready. Until this is done, do **not** open the English localisation in App Store Connect.

**Must be done together with §6.0.** Because the brand change invalidated the TR set as
well, the two sets are now two halves of the same job: TR + EN come out of a single
recapture round.

### 6.4 Clear the simulator status bar override (cosmetic)
```
xcrun simctl status_bar F74085BF-8F5B-4588-8578-92DDE02EDE3F clear
```

---

## 7. Final check before submitting

- [ ] The privacy policy URL opens and contains the web search + MCP items
- [ ] The support URL opens
- [ ] The Name field says **Tacet** — initial capital (the old "lower case" rule is INVALID)
- [ ] The screenshots were recaptured: the OLD BRAND appears in no uploaded frame (§5.4)
- [ ] The TR screenshots were uploaded, the first three frames in the right order
- [ ] The EN localisation is either closed or recaptured with an English interface
- [ ] The App Privacy questionnaire is complete ("Data Not Collected")
- [ ] The App Review Notes were pasted in — **including the Apple Intelligence enabling steps**
- [ ] The age rating questionnaire came out 4+
- [ ] Export Compliance was answered
- [ ] The build was uploaded and attached to the version

**Do not skip the App Review Notes.** If the reviewer opens the app on a device with Apple
Intelligence off, the chat produces no reply. The notes explain this and the steps to turn
it on; without them the risk of a "does not work" rejection is high.
