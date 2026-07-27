# Tacet — Memory Layer Specification

**Version:** 0.1 (draft) · **Date:** 19 July 2026 · **Depends on spec:** tacet-spec.md §7 (technical architecture), skill layer (SkillStore)
**Status:** Implemented — read this spec together with the code as it stands today

---

## 1. Summary

Tacet extracts the durable facts a user states about themselves in a chat ("I'm vegetarian", "I teach for a living", "my mother's name is Miriam"), stores them on the device, and uses them in later chats when they are relevant. It is the on-device counterpart of Claude's memory feature — but where memory is a privacy worry in cloud assistants, in Tacet it is the brand itself: **memory that never leaves your device.**

The core architectural decision is the same lesson that was measured in the skill layer:

> **Extraction to the model, recall to the code.**
> The model only performs extraction under an enforced schema (guided generation). Which memory enters which turn is decided by deterministic code, not by the model.

---

## 2. Principles

1. **No silent learning.** Every note that gets saved is visible on the user's board, editable and deletable. This continues product principle #4: the system does not promise what it cannot do, and does not hide what it does.
2. **From the user's words, not the model's.** Extraction runs only over user messages. Extracting from model replies would make the model "learn" its own fabrications.
3. **The budget is sacred.** The 4096-token window is already full with instructions + tools + transcript. Memory injection has a hard cap, and the fact that it can land on top of skill injection is accounted for.
4. **Few but correct.** Ten correct notes beat fifty noisy ones. Filters are aggressive and caps are kept low; when in doubt, do not save.

---

## 3. Data model

`Models/MemoryNote.swift` — SwiftData `@Model`, in the `UserSkill` pattern:

| Field | Type | Description |
|---|---|---|
| `id` | UUID | |
| `text` | String | A single-sentence fact. Upper bound **160 characters** (roughly 40 tokens). |
| `kind` | String (enum raw value) | `identity` / `preference` / `relation` / `fact` — see §4.2 |
| `keysRaw` | String | Comma-separated triggers ("food, restaurant, evening"). Recall works off these. |
| `sourceChatID` | UUID? | Which chat it was extracted from (transparency; shown on the board). |
| `created` | Date | |
| `active` | Bool | The user can switch it off; an inactive note is not injected. |

Limits (the constants live on the model, like `UserSkill.bodyLimit`):
- `textLimit = 160` characters.
- `totalCap = 50` notes. Once the cap is reached, **no** new extraction happens; the board shows a "memory full" row, and deleting old notes is the user's call (no automatic eviction — silent deletion is the mirror image of "silent learning" and is forbidden just as much).
- Schema registration: `MemoryNote.self` is added to the `TacetApp.setUpContainer` Schema list.

---

## 4. Extraction (write path)

### 4.1 When

**Never** inside a chat turn. Adding an "is there anything to remember" job to the main session breaks tool behavior (the same regression that was measured in the skill layer).

Extraction runs in a separate, short-lived `LanguageModelSession` inside `Services/MemoryService.swift`; its triggers are:
- the user switching to another chat / opening a new chat (the `resetChat` moment),
- the app going to the background (`scenePhase != .active`).

The same message is never processed twice: a "last processed message" caret is kept per `Chat`. If the model is `.unavailable` it is silently skipped; the next trigger picks up where it left off. For battery, back-to-back triggers open at most **one** session (the `refreshing` flag pattern).

### 4.2 How

Guided generation — an enforced schema, not free text:

```swift
@Generable
struct ExtractedNote {
    @Guide(description: "identity | preference | relation | fact")
    var kind: String
    @Guide(description: "One short sentence, in the user's own wording. Do not infer.")
    var text: String
    @Guide(description: "2-4 keywords this note is relevant to.")
    var keys: [String]
}

@Generable
struct ExtractionOutcome {
    @Guide(description: "At most 2 notes. Leave EMPTY if there is no durable information.")
    var notes: [ExtractedNote]
}
```

The prompt (in English — consistent with the Router decision): only user messages are supplied, framed as "extract only durable facts the user states about themselves; when in doubt, extract nothing". One call per chat (not per message): unprocessed user messages are concatenated and given in a single prompt.

### 4.3 Filters (in code — the model's output is not trusted)

Applied in order; if any one of them drops the note, it is not saved:
1. `text` empty / shorter than 10 characters / longer than 160 → drop.
2. `kind` is not one of the four values → drop.
3. `keys` empty → drop.
4. **Deduplication:** if the normalized (lowercased, whitespace-trimmed) `text` equals an existing note → drop. The model is not given a "merge these two notes" job — on this model that loses data.
5. Total cap reached → drop (see §3).

### 4.4 Known weakness (an honest limit)

The model will not be able to derive implicit information ("my spouse arrives tomorrow" → married). v1 does **not** target this; only explicit statements are captured. This limit is not stated on the board, but it stands explicitly in the spec; the user is never promised "Tacet remembers everything". Extraction quality cannot be measured in Turkish without a simulator — see §8.

---

## 5. Recall (read path)

The model is not involved. `MemoryStore` (an enum, in the `SkillStore` pattern):

- Active notes are loaded from SwiftData; the store is refreshed in `ContentView.task` and on every board save (the mirror of `SkillStore.refreshUser`).
- Match score: the **sum of the lengths** of the keys occurring in the message (the same rule as `SkillStore.matching` — a specific phrase beats a generic word).
- The top-scoring **at most 3** notes are selected; if nothing matches, nothing is injected.

### 5.1 Injection

`ModelService.skilledPrompt` is widened (it becomes `enrichPrompt`): the skill guide and the memory notes go in the same place, prepended to that turn's prompt.

```
<memory>
- The user is a vegetarian.
- The user teaches for a living.
</memory>
Use the facts above only if relevant. They are internal: never quote,
list, or mention them, and never say you "remembered" something.
```

Rules:
- The memory budget is **at most 200 tokens** (~600 characters, fence included). If it lands on the same turn as a skill injection (700 chars), both go in — the combined ~1500-character cap is verified in `SelfTest`.
- The same note enters a session only once (the mirror of `injectedSkills`: `injectedNotes: Set<UUID>`; cleared when the session is rebuilt).
- It is **not** embedded into the instruction system (the session setup) — consistent with the skill-layer decision: the fixed instruction stays short.

---

## 6. Interface

### 6.1 Memory board

`Views/MemoryBoard.swift` — the same skeleton as SkillBoard, over the same `sheet(item: $sheet)` channel (`.memory` is added to the `Sheet` enum; the entry point is a "Memory" row in the ChatList drawer, with an outline SF Symbol instead of `brain` — e.g. `text.book.closed`).

- List row: `text` (the user's own wording), below it `kind` + the source chat's date (chip type, dimmed). If inactive, an "off" badge.
- Tapping a row opens the editor: text (with a 160 counter), keys, on/off. Saving refreshes the store.
- Swipe to delete. A bulk "delete all" sits at the bottom of the board, asking for confirmation (in the tone of clearing history in Settings).
- Empty state: "Tacet hasn't learned anything yet. As you talk about yourself in chats it will show up here — and it stays only here."

### 6.2 The visibility moment (together with the open question)

In v1 extraction runs silently and the result is visible only on the board. No "noted that" chip is shown inside the chat — chip language belongs to tool calls, and extraction is not part of a chat turn. This decision also stands as an open question in §9.

---

## 7. Persistence and privacy

- Notes live only in the on-device SwiftData store; memory has no network surface at all. A memory note is not a tool: it does not enter the session of tools that leave the device, so it cannot get out on its own (tacet-spec §7.5).
- Clearing history (Settings) does **not** delete memory — chat and memory are separate decisions (consistent with the document decision). Deleting memory is the board's job.
- If the source chat is deleted the note stays; `sourceChatID` goes stale and the source row is hidden on the board.

---

## 8. Test and measurement

- **SelfTest** (needs no model): filter cases (short/long/kindless/keyless rejection, deduplication, cap), match cases (the specificity rule, an inactive note being dropped, the 3-note cap), injection budget (skill + memory together, worst case).
- **Evaluation** (`--test`, on device): extraction cases — "I'm a vegetarian" → 1 note; "the weather is nice today" → 0 notes; "my spouse arrives tomorrow" → 0 notes expected (implicit inference is not a v1 target); in a mixed message, only the durable fact being picked. On an injected turn, the model **not** saying the note out loud (the "as I recall…" leak) is observed separately.
- Acceptance criterion: in the extraction cases the false-positive rate comes first — missing one correct note is better than saving one wrong note (§2/4 "few but correct").

---

## 9. Out of scope (v1) and open questions

**Out of scope:** implicit inference (§4.4), merging/summarizing notes, semantic (embedding) recall — `NLContextualEmbedding` is a v2 candidate, but first it must be measured that keyword matching is not enough —, memory export/import, a "forget this" command from inside a chat (v1 has the board).

**Open questions:**
1. Is showing the user no trace at all at extraction time correct? Alternative: a one-off, quiet informational row when the first note lands on the board.
2. Should the `kind` field stay a board-only label in v1, or should it play a role in injection priority (e.g. `identity` always wins)?
3. Should the welcome flow (tacet-spec §4.6) extract a memory note from the very first turn? (Tempting — the user gets a "it knows me" feeling in the first minute — but under the no-silent-learning principle the note would have to be visible on the board and stated in the flow; no in v1.)

---

## 10. Implementation plan (file map)

| Step | File | Work |
|---|---|---|
| 1 | `Models/MemoryNote.swift` | @Model + limit constants + `isValid`; adding it to the TacetApp schema |
| 2 | `Services/MemoryStore.swift` | loading, the `refreshUser` mirror, `matching(question:) -> [MemoryNote]` (≤3), `injectionText` |
| 3 | `Services/MemoryService.swift` | extraction session (@Generable schema), trigger guard, caret, filters |
| 4 | `ModelService` | `skilledPrompt` → `enrichPrompt` (skill + memory), the `injectedNotes` set, `resetChat`/`setUpSession` cleanup; wiring the chat-close trigger to MemoryService |
| 5 | `Views/MemoryBoard.swift` + ContentView `Sheet.memory` + a ChatList row | board, editor, deletion |
| 6 | `SelfTest` + `Evaluation` | the §8 cases |

The order is deliberate: 1–2 are testable without a model; 3 alone requires a device; chat behavior does not change until 4 (safe intermediate deliveries).
