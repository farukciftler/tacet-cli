# Tacet — Product and Design Specification

**Version:** 0.3 (draft) · **Date:** 25 July 2026 · **Platform:** iOS 26.0+ — **iPhone only**, macOS later — some tools require an iOS 27 API, marked in the table
**Design direction:** C — "Transparent tools", merged with B's serif voice
**Related documents:** [mcp-connection-spec.md](mcp-connection-spec.md) (network promise, tainted session, approval gate) · [web-search-spec.md](web-search-spec.md) · [memory-spec.md](memory-spec.md) · [timeline-spec.md](timeline-spec.md) · [code-spec.md](code-spec.md)

---

## 1. Product summary

Tacet is a personal assistant whose core runs entirely on the device. It uses the on-device model of Apple's Foundation Models framework; model inference, chat history, and calendar/reminder/contact/note data never leave the device. The assistant's job is to answer questions about the user's own life and to handle small tasks: check the calendar, set a reminder, search notes/contacts, produce documents, chat with the data on the device.

Two optional surfaces of the product do leave the device, and this is not hidden: **web search** (a search server the user hosts themselves) and **connections** (MCP servers the user adds themselves). Both are off by default, the user turns them on by hand, and every exit leaves a trace visible on screen.

The promise is one sentence:

> **The core runs on the device; web and connections come into play only if you turn them on, and only with you seeing it.**

The name is the promise itself: *tacet* — in musical notation, "this instrument is silent in this passage". Every product decision aligns with that sentence: the side that stays silent is the cloud. Tacet does not choose to be silent; the gate is not at the model's mercy, it is in the code (§7.5).

**Target user:** Someone who cares about privacy and wants to work in natural language with the data on their phone (calendar, reminders, notes, contacts). The core functions need no internet connection.

**Out of scope (v1):** Cloud sync, third-party account linking, a ready-made connector catalog, iPad and landscape layout, scheduled background generation ("Watch"). Rationales in §8.2.

---

## 2. Product principles

1. **The core stays on the device; exits are visible and optional.** Model inference and the personal-data tools touch the network under no circumstances. The only two surfaces that leave the device (web search, connections) are off by default, the user turns them on by entering their own server, the outgoing content stands raw in the chip, and in a session where personal data has been touched it passes through the approval gate. This is not a settings promise, it is an architectural fact (§7.5).
2. **It doesn't say, it shows.** The privacy promise is proven by the interface, not by text: every time the assistant touches a tool it leaves a visible trace on screen (the tool chip). At every step the user sees "what did it look at, what did it do, what did it send".
3. **A quiet interface.** One background, **no accent color**, no ornament. Personality comes from typography, not decor: Tacet speaks in serif.
4. **Small model, honest assistant.** The model is a router, not a source of knowledge. When it does not know, it does not make things up; it calls a tool or says it does not know. Arithmetic and data work is always solved in code, not in the model.

---

## 3. Design language

### 3.1 Color

It is built on a single background and there is **no accent color**. The palette is ink/grey tones; the only colored token is `error` and it appears only on a failed tool chip. State is carried by words and marks, not by color.

| Token | Light mode | Dark mode | Use |
|---|---|---|---|
| `background` | `#FFFFFF` | `#141413` | Screen background |
| `ink` | `#1C1C1A` | `#ECECEA` | Primary text, send button fill |
| `grey` | `#5F5F5A` | `#9A9A93` | Secondary text, tool chip text |
| `muted` | `#76766F` | `#7E7E78` | Placeholder, section labels |
| `divider` | `#E9E9E4` | `#2A2A28` | Hairline borders, separators, chip frame |
| `fill` | `#F4F4F1` | `#222220` | User bubble background |
| `error` | `#B4483C` | `#D46A5E` | Only a failed tool chip (rare) |

Rules: no shadows, no gradients, blur only on system-sourced surfaces (keyboard, sheet). Borders are 1 px (hairline), never thicker. There is **no green "on your device" dot and there never will be** — privacy is not claimed with a badge, it is shown by behavior (principle #2).

### 3.2 Typography

There are two voices, and who is speaking is clear before you read a word:

| Role | Typeface (iOS) | Size / line | Use |
|---|---|---|---|
| User text | SF Pro (system, `.default`) | 15 / 1.5 | The message inside the bubble |
| Tacet reply | New York (system, `design: .serif`) | 17 / 1.6 | All of the assistant's answers |
| Brand | New York, medium | 19–20 | The top bar "Tacet" + the brand mark |
| Label | SF Pro | 10, letter-spacing 14%, uppercase | Section labels, date separators |
| Chip / meta | SF Pro | 11 | Tool chips, timestamps |

Dynamic Type is fully supported; sizes are defined relative to `body`/`callout`, and fixed px is only a design reference. `.system(size:)` is not used in code — `Typography` tokens or `@ScaledMetric`. Weights: regular (400) and medium (500). Semibold/bold is not used.

### 3.3 Spacing, corners, icons

- Spacing scale: 4 / 8 / 12 / 16 / 22 pt. Screen horizontal margin 22 pt.
- Vertical spacing between messages 14 pt; between a tool chip and its related reply 10 pt (the chip should read like the reply's "top line").
- Corners: user bubble `18/18/5/18` (tail at bottom right), tool chip a full pill (20), input field 24, send button a circle.
- Touch target at least 44 pt (`Spacing.touchTarget`).
- Icons: SF Symbols, outline only, a 1.5–2 pt stroke feel, 12–16 pt size. The color is always the color of the text above it. Emoji are never used in the interface.

---

## 4. Components

### 4.1 Top bar

On the left the brand mark + serif "Tacet". There is **no** colored dot, badge or status light. If the model is unavailable (unsupported device, Apple Intelligence off, model downloading) this is said in the interface's own words. Tapping the brand opens a short explanation sheet: where the data is processed, what stays on the device, which surfaces (web search, connections) go out if they are on. New chat on the right, history on the left.

### 4.2 User bubble

`fill` background, `ink` text, right-aligned, max width 80%. No shadow, no border.

### 4.3 Tacet reply

No bubble. Left-aligned serif text, max width 88%. The assistant does not stand like a part of the interface but like a voice writing on the page. While the reply streams, the text appears word by word; the only loading animation is a single, calm blinking dot (no three bouncing dots).

### 4.4 The tool chip — the system's signature

When the assistant calls a tool, a chip lands in the stream right above the related reply. The chip is pill-shaped: a 1 px `divider` frame, icon + 11 pt text, left-aligned.

States:

| State | Appearance | Example text |
|---|---|---|
| Running | `grey` icon + text, a 12 pt spinner in place of the icon | "Checking the calendar…" |
| Done (read) | `grey`, tool-specific icon | "Calendar read · tomorrow" |
| Done (write) | `grey`, checkmark icon | "Reminder set · 13:00" |
| Awaiting approval | `grey`, hand icon, tappable | "my server · awaiting approval" |
| Not sent | `grey`, short reason | "Not sent · device data off" |
| Failed | `error` exclamation + short reason | "Could not reach the calendar" |
| Needs permission | `grey`, tappable | "Calendar permission needed — allow" |

Rules: chip text is at most ~5 words + an optional `· detail`. The read/write distinction is made with the icon and the verb form, not with color (no accent color, §3.1). Tapping a chip opens a small detail view showing the tool's raw input/output — the second layer of the transparency principle. **On every call that leaves the device, exactly the outgoing content stands in that raw input**; even if approval is skipped, transparency is not. Multiple tool calls in the same turn are listed as separate chips, one under the other.

### 4.5 Input field

A pill with a 1 px `divider` frame, placeholder "Ask Tacet" (`muted`). On the right a circular send button filled with `ink`. The button activates when there is text; when empty the button is not dimmed — it keeps the same appearance but is inactive (the principle of not using dimmed/disabled buttons).

On the left there is a single secondary icon: the **microphone** (v1, §7.7). There are no attachment/camera icons; documents enter the chat via sharing.

### 4.6 Empty state and welcome (first launch)

Empty state: a single serif sentence in the middle of the screen, a short explanation below it, and below that example prompt chips (tapping one writes it into the input field). No logo, no illustration, no animation. The examples do not merely say "you can ask questions"; they hint at the product's invisible capabilities (reminders, document generation, search).

**The welcome flow (onboarding, v1)** is a direct application of principle #2: there is **no** slideshow, no intro video, no "our features" list. On first launch the user does not read, they **do** something:

- The flow consists of one single real task — the user picks a ready prompt (or writes their own), Tacet actually runs it, permission is requested right then and there with the context obvious if permission is needed, the tool chip appears on screen, and tapping the chip shows what was read.
- The only thing explained is what the chip is, and even that is said in one sentence while the chip is on screen.
- Permissions are not collected up front; each permission is requested the first time it is genuinely needed (§7.3, `PermissionGate`).
- The flow can be skipped at every step, and skipping drops the user straight into the empty state — it is not a one-way door.
- Web search and connections are **not turned on and not suggested** during the welcome. They stay off by default; the user goes to Settings themselves (principle #1).

---

## 5. Motion

There is almost no animation, and everything that exists is functional: message send 200 ms soft settle, chip appearance 150 ms fade + 2 pt upward shift, streaming caret 900 ms breathing. With `Reduce Motion` on, all transitions become instant. No component moves on its own.

---

## 6. Copy and tone

Tacet's voice: calm, short, definite. It speaks Turkish; if the user writes in another language it answers in that language.

- Answers state the result first, then add a single sentence of context if needed. No greetings, no filler ("Of course!", "Great question").
- When it does not know it says so plainly: "I couldn't find this on your device." Making things up is forbidden; if the model cannot suggest a tool for a world-knowledge question it states its limit.
- Action confirmations are past tense and short: "Set.", "Deleted." Exclamation marks are not used in system copy.
- Interface copy (buttons, labels) starts with a verb and is in sentence case: "Allow", "See detail".
- Error copy does not apologize; it says what happened and what to do: "Could not reach the calendar. You can grant permission in Settings."
- **Privacy sentences are never written in the absolute mood.** Unqualified sentences like "I don't go online" / "data never leaves the device" become lies when web search or connections are on. The correct mood: "My core runs on the device; web search is off." This rule applies to permission strings and store copy too (§7.9).

---

## 7. Technical architecture

### 7.1 The model layer

- Only `SystemLanguageModel` (on-device, ~3B). Private Cloud Compute is **not used** — this is the product's marketable constraint, and it still holds after web search was added: what leaves is not model inference but a query/argument going to a surface the user turned on.
- An availability check runs at app launch; if the model is absent the interface drops into an LLM-less mode and the state is explained in words. The model is treated like a feature flag, not like a guarantee.
- `LanguageModelSession` carries a single chat session. The instructions are kept short (~150 tokens): identity, language, the "say you don't know, call the tool" rule, output length expectation.

### 7.2 Context budget (4096 tokens)

The context window is actively managed like memory in a low-resource system:

- Measurement with `tokenCount(for:)` before every turn; ~80% of `contextSize` is taken as the threshold.
- **Deployment-target note:** `contextSize` and `tokenCount(for:)` are iOS 26.4+ APIs; because the deployment target was pulled back to 26.0, these calls are guarded with `#available`. On the branch where the guard fails, the budget check is skipped, the turn continues, and the `.exceededContextWindowSize` catch path remains the only safety net. If measurement fails a trace is left; no error is shown to the user.
- When the threshold is exceeded: the last 4–6 turns are preserved, older history is summarized into a single paragraph, and a new session is opened with that summary + the preserved turns. If `.exceededContextWindowSize` is caught, the same recovery path runs.
- Because the summary can carry personal data, **the tainted-session flag travels with the summary** (§7.5); only a real chat reset clears the flag.
- Tool outputs are never dumped raw into the context: long results (e.g. 30 calendar records) are filtered and summarized in the tool layer; if needed only a reference ID is returned and the next turn fetches it again by ID.

### 7.3 Tool catalog (the Tool protocol)

All tools are defined with the `Tool` protocol and their arguments type-safely with `@Generable`/`@Guide`. The model does not produce free text to be parsed. The catalog is the on-device counterpart of the most-used tool families in assistants like Claude. **Only two tools touch the network** (`web_search` and the MCP tools); everything else is on-device and has no access to the network layer (§7.5).

**Personal-data tools** — on-device; their first successful call "taints" the session (§7.5):

| Tool | Source | Action type | Example chip text |
|---|---|---|---|
| `CalendarTool` | EventKit | read + write | "Calendar read · tomorrow" / "Event added" |
| `ReminderTool` | EventKit (Reminders) | read + write | "Reminder set · 13:00" |
| `ContactTool` | Contacts | read | "Searched contacts" |
| `SearchNotesTool` | Core Spotlight — `SpotlightSearchTool` on iOS 27, a `CSSearchQuery` wrapper on iOS 26 | read (local RAG) | "Searched notes · 3 results" |

**Document tools** — generation + perception; the output is a QuickLook preview + share sheet + saving to Files. The engines are pure-Swift OOXML/PDF writers, no network:

| Tool | Engine | Action type | Example chip text |
|---|---|---|---|
| `CreateDocumentTool` | Excel / Docx / Pdf / Html / Text engines | write | "Document created · july.xlsx" |
| `ReadDocumentTool` | PDFKit + an OOXML parser | read | "Document read · contract.pdf" |
| `EditDocumentTool` | the same engines | write | "Document edited" |

**Helper tools:**

| Tool | Source | Action type | Example chip text |
|---|---|---|---|
| `CalcTool` | Pure Swift | computation | "Calculated" |
| `RunCodeTool` | An on-device interpreter (code-spec) | computation | "Code ran" |
| `TimeTool` | Foundation | read | (no chip shown — insignificant) |

**Tools that leave the device** — off by default, the user turns them on by entering their own server:

| Tool | Source | Where it goes | Example chip text |
|---|---|---|---|
| `WebSearchTool` | The user's own SearXNG instance ([web-search-spec.md](web-search-spec.md)) | The address the user entered | "Searched · «keyword»" |
| MCP tools | The MCP server the user added ([mcp-connection-spec.md](mcp-connection-spec.md)) | The address the user entered | "my server · docker_list" |

General notes: RAG is done without an embedding pipeline, over the Spotlight index. Arithmetic is always routed to `CalcTool`. Every tool's `description` field is written with the clarity of one job + when to call it — that is the only lever in the model's decision to reach for a tool. Permissions are requested through `PermissionGate`, at the moment of the first real need.

#### 7.3.1 Tool budget and profiles

In a 4096-token window each tool's spec takes up room too; all tools in the catalog cannot be given to a single session. The rule: at most 6–8 tools in a session. Tools are grouped into profiles and the session switches profile according to the mode of the conversation:

- **Everyday profile (default):** Calendar, Reminder, Search, Calculate, Time, Code + **the Contacts ↔ web search swap**.
- **Document profile:** Create/Read/Edit Document + Calendar, Reminder, Search, Calculate, Time. While a document is attached the profile is **locked** here.
- **Search profile:** `web_search` + Time. Nothing else.
- **Connection profile:** the selected connection's tools + Calculate + Time.

**A profile is also a security boundary.** In the search and connection profiles the personal-data tools are **deliberately absent**: so the model cannot say "search the address in my notes" in a single step. A mixed job that needs personal data is spread over two turns; because the second turn has tainted the session, it falls into the approval gate. The Contacts ↔ web search swap in the everyday profile is the small-scale form of the same rationale: the two never sit in the same set, and which one enters is decided by the guiding signal in the question.

`CalcTool` is absent from the search profile too: in the measured cases, the model fabricating a live value it could not find and having the tool do the arithmetic led to it presenting the result as "it came from the tool". Arithmetic being one turn late is better, in every circumstance, than a fabricated exchange rate being presented instantly.

The profile choice is not asked of the user; it is made silently with a deterministic intent classification and appears in the timeline row. If the wrong profile is chosen (if no tool ran at all) the app layer retries the turn once with a second profile.

#### 7.3.2 The file generation pattern (two data flows)

1. **A small table/document from the chat:** The model produces a `@Generable Table` (headers + type-safe rows) → the generation tool writes the file. Because of the context window the practical limit is ~50–100 rows; this flow is for jobs like a budget draft, a list, a comparison.
2. **A large file from device data:** "Dump last month's meetings into Excel" → `CalendarTool` fetches the data, the app layer hands the structured data straight to the document tool **without passing it through the model**; the model only chains the two tools and confirms the result. Bulk data never enters the context window — the file size is bounded by the device, not the window.

Numeric summaries (total, average) are embedded in the cell as a real Excel formula (`=SUM(...)`); the spreadsheet does the calculation, not the model.

#### 7.3.3 Deliberately left out

| Claude tool | Tacet decision | Rationale |
|---|---|---|
| Weather | None | WeatherKit opens a separate network surface; the user's own search server already covers this. |
| Maps / directions | None (v1–v2) | MapKit largely requires the network and goes out to a third party; it is not the user's own server. |
| Cloud image generation | None — instead on-device Image Playground (v2) | Not the user's own server. |
| A third-party connector catalog | None | A connection is a server the user added by hand; a ready-made list creates the impression of "a server Tacet knows". |
| Shortcuts triggering | **Made it into v1** | See §7.8 — the decision changed. |

When a capability requiring the network is rejected, the rationale is not "network" but **"it is not the user's own server"**. The distinction is deliberate: the claim that Tacet is an anti-network product is no longer true; what Tacet refuses is taking the user's data to an endpoint the user did not choose.

If the user asks for one of these capabilities, Tacet states its limit in one sentence: "I have no weather service; if your search server is on I can search for it."

### 7.4 Tool chip ↔ tool mapping

When a tool call starts, a "running" chip lands in the UI; when the tool returns, the chip moves to its final state. The mapping is fed not from the session transcript but from the app's own tool execution layer (`ToolExecutor`) — the single source of truth is the tool itself. The chip's text is produced by the tool, not written by the model; the model cannot hallucinate the chip text.

### 7.5 Network architecture and privacy guarantees

The four mechanisms the promise rests on. All of them are in code, none of them at the model's mercy:

**1. Network monopoly — two files.** `URLSession` occurs only in `Services/MCPClient.swift` and `Services/WebSearchClient.swift`. The tool layer, the model layer, the view layer and the personal-data tools never touch a network API. This rule is an architectural boundary: network code is not added to any other file.

**2. Off by default, no embedded endpoint.** Neither web search nor connections come on out of the box. The search address is a server the user hosts themselves; while the stored address is empty the tool never enters the session and the network code never runs. There is **no** default search address, API key or Tacet-owned server inside the app. The address rule requires `https://`; plain `http://` is accepted only for local network addresses. (There is an environment-variable convenience in development builds and it is wrapped in `#if DEBUG` — that branch is not compiled in the App Store build.)

**3. Profile separation.** Personal-data tools **do not enter the same session** as tools that leave the device (§7.3.1). The model cannot call a tool it does not have; this rests on the absence of the capability, not on the text of a prohibition.

**4. Tainted session + a deterministic approval gate.** The **first genuinely successful** call of the Calendar, Reminder, Contacts, Search and Document tools marks the session "tainted". A permission refusal or an error does not taint it — data that could not be reached cannot taint the session. In a tainted session every call that leaves the device stops at the approval gate in `ToolExecutor`, and the user decides while seeing **exactly the arguments that would be sent**. Gate properties:

- The decision is three-valued (accept / refuse / cancel); "the user refused" and "the turn was cancelled" are separate paths.
- There is a per-connection "device data" setting: *never* (in a tainted session the call is never made, not even asked about), *ask* (default), *always allow* (the gate is skipped).
- **Even if the gate is skipped, transparency is not:** the outgoing content stands in the chip's raw input in every case, and if it was sent without approval being asked, that is stated additionally.
- Remote tools with a destructive side effect are asked about **every single time**, even if the session is clean and even if "always allow" is selected. "You may send data from my device without asking" and "you may do work on my server without asking" are two separate decisions.
- The taint travels with the context summary and is not cleared by `newTurn()`; only a real chat reset clears it.

**Persistence:** Chat history, memory notes and settings are stored on the device with SwiftData and are included in the iCloud backup (the user can turn this off). No analytics; crash reports only through Apple's system mechanism, opt-in. The app has no server of its own — there is no data Tacet could collect, because there is nowhere for the data to go.

### 7.6 Accessibility

Dynamic Type (serif included), chips are spoken in VoiceOver as natural sentences like "Tacet read the calendar, tomorrow", contrasts at WCAG AA, Reduce Motion supported. Touch target min 44 pt.

### 7.7 Voice input (v1)

The microphone button in the input field turns speech into text and writes the text into the input field; the user sees it and can correct it before sending. Spoken replies (TTS) are not in v1.

**Absolute rule — recognition must be on-device.** `SpeechTranscriber` on iOS 26, `SFSpeechRecognizer` with `requiresOnDeviceRecognition = true` on the fallback path. **Under no circumstances** do we fall back to server-based recognition: if we did, the audio would go to an Apple server and the product's core promise would break without the user turning anything on.

The consequences and the accepted cost:

- If the on-device language model has not been downloaded, or the selected language is not supported on-device, the microphone **does not work and does not silently fall back to the server**. The button becomes inactive and the reason is stated in one sentence: "On-device recognition isn't ready for this language."
- Accuracy may stay somewhat below server recognition. Accepted; the user sees the text before sending and can correct it.
- The audio recording is not written to disk and not added to the chat; the buffer is discarded when recognition ends. The only thing left in the stream is text.
- The `NSMicrophoneUsageDescription` and `NSSpeechRecognitionUsageDescription` permission strings follow the mood rule in §6 and state the on-device constraint explicitly.

### 7.8 App Intents / Siri / Shortcuts (v1) — the decision changed

The previous decision treated Shortcuts as a "v2 candidate, contested"; the rationale was "a network call can be made from inside a Shortcut, which punches through the cannot-leak promise by the user's own hand". **That rationale is no longer valid, and it was standing in the wrong place to begin with.**

Why: the user's own Shortcut can take the user's own data wherever they want — that is the definition of Shortcuts and it is like this everywhere in the system (Notes, Calendar, Health included). Blocking a user's decision about their own data is not privacy, it is paternalism. Tacet's responsibility is not to constrain the user's decision but to **keep its own exits visible and approved** (principle #1). So App Intents make it into v1, and they are done well.

The surface's principles:

1. **The actions that were opened (as implemented).** Five intents were opened: `AskTacetIntent` (ask — it **does NOT return an answer**, it opens the app), `GenerateDocumentIntent` (create a document), `AddNoteIntent` (add a note to memory — not search), `ProvideDocumentIntent` (hand a produced file to Shortcuts), `OpenChatIntent` (open a past chat). There is **no** reminder / calendar / note-search intent: those require a permission gate and a tool chip on screen and cannot be run in the background. `AppShortcutsProvider` defines four shortcuts; the phrases are written in the source language (tr) and the remaining eight languages come from `AppShortcuts.xcstrings`.
2. **What stayed closed.** No capability that leaves the device is opened as an intent: web search and MCP tool calls cannot be triggered from Shortcuts. The rationale is not capacity but the gate: the approval gate (§7.5) rests on the user seeing the screen, and in the Shortcuts context there is no screen to see. Rather than weaken the gate, we do not open the surface.
3. **The trace rule — every Shortcut call THAT RUNS A TOOL is written into the stream.** Every call that executes a tool produces a tool chip and a timeline record, exactly as if it had been done inside the app. `AddNoteIntent` never touches `ToolExecutor` (deliberate: writing one line into memory is not a tool turn), so it produces no chip. When the user opens the app later they see "what happened in the background" in the same interface. There is no path that runs invisibly.
4. **Intents that return personal data.** Because an intent's return value can appear on the lock screen / in the Shortcuts preview, intents that return personal data bring the app to the foreground via `.needsToContinueInForegroundError` or return only a summary — raw calendar/contact content is not spilled onto a locked screen.
5. **Chain responsibility is written down explicitly (implemented).** There is a Settings > SHORTCUTS section: a `Give files to Shortcuts` switch (default OFF), a `Last given` record and a one-sentence note — where a file given to a Shortcut goes afterwards is the decision of the user's Shortcut, not Tacet's. When the gate is closed the export record is deleted too.

### 7.9 Release (App Store)

Because the product has two optional surfaces that go online, the store surface **has to be consistent** with that frame too. The "data not collected" claim is not enough on its own; the claim and the behavior must match.

- **Privacy label (nutrition label):** Tacet itself collects no data, but the app uses the network when the user turns it on. Network usage and the search query / tool arguments going to the server address the user entered are declared in the label. "Data Not Collected" can only be defended together with that declaration, with the user-configured endpoint explained openly.
- **A privacy policy URL is mandatory.** The policy states the core/optional surface distinction and that data goes to the user's own server (that Tacet has no server).
- **`PrivacyInfo.xcprivacy` was added.** The manifest declares `NSPrivacyTracking = false`, an empty `NSPrivacyCollectedDataTypes` and three required-reason categories: UserDefaults (CA92.1), FileTimestamp (C617.1, 3B52.1), SystemBootTime (35F9.1 — duration measurement with `DispatchTime`).
- **The store description** follows the mood rule in §6: the absolute "does not go online" sentence is not used; the frame used is "the core runs on the device, web and connections come into play only if you turn them on".
- **Permission strings were fixed.** In the `…NSCalendarsFullAccessUsageDescription`, `…NSCalendarsWriteOnlyAccessUsageDescription`, `…NSContactsUsageDescription`, `…NSRemindersFullAccessUsageDescription`, `…NSMicrophoneUsageDescription` and `…NSSpeechRecognitionUsageDescription` strings the absolute sentence **"Data never leaves the device."** IS GONE; each string promises only within the scope of that permission. All six are translated into 9 languages via `InfoPlist.xcstrings` — the system permission dialog appears in the user's language.

---

## 8. v1 scope, out-of-scope decisions and open questions

### 8.1 Scope

**v1:** A single chat stream + chat list; four profiles (everyday, document, search, connection); the welcome flow (§4.6); voice input (§7.7); App Intents / Siri / Shortcuts (§7.8); web search and connections (off by default); the memory board; the skill board; the timeline line; dark mode; localization into 9 languages (tr, en, de, es, fr, ja, ko, pt-BR, zh-Hans). iPhone only, portrait only.

**v1.1:** A richer chip detail view, showing MCP approval wait time separately, `HealthTool` / `HistoryTool`, more formats in the document preview.

**v2 candidates:** macOS, iPad and adaptive layout, `ImageTool` (Image Playground), `PhotoTool`, `BarcodeTool`, widget / lock screen, spoken replies (on-device TTS), user-defined shortcut prompts.

### 8.2 Out-of-scope decisions and their rationales

This section is the answer to "why isn't there X?"; it stands here so that if a decision is argued later, it is argued with its rationale.

| Decision | Status | Rationale |
|---|---|---|
| **Watch** (a scheduled agent / background briefing) | **Removed entirely** from v1 | On iOS, guaranteed "overnight-while-charging" generation requires `BGTaskScheduler` + an entitlement; without the entitlement the run time is up to the system and the "a summary every morning at 8" promise cannot be kept. A scheduling promise that cannot be kept is a direct violation of principle #4 ("honest assistant"): while the rest of the product says what it cannot do, a feature silently not running is unacceptable. The decision was to remove the feature rather than weaken it — a Watch that half works is worse than no Watch at all. Re-evaluation condition: if a suitable entitlement is obtained, or if triggering can be tied to a user action (e.g. "generate a briefing for last night" at app launch). |
| **iPad / landscape layout** | Out of scope (v2 candidate) | v1's design language was calibrated for a single-column reading surface used one-handed (serif reply width 88%, 22 pt margin, a single stream). An iPad layout means a second information architecture (split view, a persistent chat list) and cannot be done right in v1. Being complete on iPhone was preferred over shipping a half-adapted iPad version. |
| **Private Cloud Compute** | None | The product's marketable constraint (§7.1). |
| **Cloud sync / accounts** | None | Tacet has no server; adding accounts changes that. |

### 8.3 Open questions

1. Does the serif reply keep its readability in very long texts (5+ paragraphs)? — To be tested with large Dynamic Type sizes.
2. In the welcome flow, which single task is the task that "explains the product"? (A reminder gives the fastest satisfaction, but a calendar read shows the chip better.) To be measured in user testing.
3. How do we keep the `SearchNotesTool` experience from falling flat for users with an empty Spotlight index?
4. How often does profile routing (§7.3.1) pick the wrong profile; and how much does the retry cost in latency?
5. It has not been measured in how many of the 9 languages on-device speech recognition is genuinely ready. For unsupported languages, is it right for the microphone to not appear at all, or to appear inactive?
6. A tool chip is produced for a job run from a Shortcut, but if the user never opens the app the trace is never read. Is a marker needed at app launch for accumulated background traces?
