# Tacet — audit implementation summary

Source of authority: `audits/tacet-small-model-architecture-audit.md` §4 (P0 5 + P1 9 + P2 9 = 23 items).
Excel report: `tacet-audit-report.xlsx` (produced with the app's own `ExcelEngine`, via `--eval-merge --audit`).
Verdict data: `verdicts-full.json` (23 records · code, priority, goal, done, evidence, verdict, comment).

## 0. Input coverage

**This summary covers ALL 23 items.** The previous version covered only 11:
the orchestrator had truncated the verdict JSON at 12000 characters
(`apply-audit.js:274`, `.slice(0, 12000)`). The untruncated verdict record was
recovered back out of the audit agent's structured-output call; the
`goal/done/evidence/verdict/comment` fields of every item, P1-7…P2-9 included,
are now in hand. The Excel report was regenerated from this complete data too.

## 1. Verdict distribution (n = 23)

| Verdict | Count |
|---|---|
| held | 6 |
| partial | 16 |
| did not hold | 0 |
| not applied | 0 |
| NOT MEASURED | 1 |

Priority breakdown: **P0** 3 held / 2 partial · **P1** 1 held / 7 partial / 1 NOT MEASURED ·
**P2** 2 held / 7 partial.

The dominance of "partial" collects into a single pattern: **the mechanism hits
its goal, the measurement/lock that is the acceptance criterion stays missing.**
In 11 of the 16 partials, what is missing is exactly the deterministic assertion
or the isolated before/after measurement the audit explicitly asked for.

## 2. P0 items

| Code | Verdict | Summary |
|---|---|---|
| **P0-1** skill core-first truncation | **partial** | The mechanism was proved deterministically (in all 10 skills the core is complete, the limit is not exceeded, 8 self-test assertions ✓). But the behaviour went the WRONG way: code 82→64, webSearch 100→76, webPage 100→86. Once the code skill's core moved to the front, the code tool started taking the place of web search. Without retuning the content of the core (`code.md` in particular) there is no net gain. |
| **P0-2** SourceRef silent success | **held** | The ref is now BINDING: if it cannot be resolved `engine.write` is never called, no file is produced, `state: .failed`. 10/10 self-test ✓. Requiring a separator (`=`/`:`) when stripping the prefix also closed the new failure path the audit's naive suggestion would have opened (truncating a legitimate ID such as `reference-1`). |
| **P0-3** MCP retry double side effect | **held** | The sticky `mayHaveSideEffect` flag was placed in the `MCPToolBridge.call` funnel — not on MCPTool. When a new remote call path is added, forgetting to mark it is structurally impossible. 11/11 self-test ✓. |
| **P0-4** enum discriminator | **held** | The calendar/reminder/time/document-format fields are `@Generable` enums. `action.lowercased` live matching is ZERO. An invalid operation is grammatically UNPRODUCIBLE. Calendar 90→95. |
| **P0-5** eval CI gate | **partial** | Threshold + non-zero exit + N-run works and is locked down by mutation tests (`EVAL GATE: PASSED 87/109 (threshold: 0.75) → PASS`). **There is NO temperature control** — `GenerationOptions(temperature:)` has zero matches in production; the "variance across runs cannot be measured" problem still stands exactly as it was. |

## 3. P1 items

| Code | Verdict | Summary |
|---|---|---|
| **P1-1** prompt core + profile appendix | **partial** | Token cost dropped by half (everyday 1238→635, document →598, search →581, connection →532), the language channel 3→2. But the ≤300 core target was not reached and the actual gain claim ("early summarisation is reduced") was never measured. Together with P0-1 it is the shared suspect for the code/search drift. |
| **P1-2** in-turn profile recovery | **NOT MEASURED** | `secondProfile` + `recoveryNeeded` + the cancellation-safe trigger are live and in the right place. There is neither a deterministic assertion nor an eval case; the gain rests entirely on reading the code. The diagnosis `'make a weekly meal table' → none` shows the gaps persist. |
| **P1-3** skill trigger word boundary | **partial** | The behaviour is right (`cloudy`/`in the December month`/`alphabetical` no longer match), the `wholeTermLimit = 4` threshold is justified. But the locking tests the audit asked for are NOT in SelfTestCases — this can regress silently today. |
| **P1-4** skill↔tool consistency | **partial** | The hand-written `skillProfiles` map was removed; matching is now bound to the real `tool.name` set of the active profile (cleaner than the audit's suggestion). What is missing is the consistency test: if a skill's `tools:` tag is misspelled the skill is silently never injected (fail-closed but INVISIBLE). |
| **P1-5** tolerant table parser | **held** | `Table.blocks` drops no rows; the local lossy parser in `TacetReply` was deleted; `ExcelEngine` throws `unsupported` instead of silent single-column garbage. 5/5 self-test ✓. documentRead 85→99. |
| **P1-6** schema budget + slot relevance order | **partial** | The schema half is live and **proved by mutation** (`nodeBudget = 48`; deleting the guards turns 4 tests red). The relevance half (`enum ToolRelevance`) was tested DEAD CODE — see §6. |
| **P1-7** retry in a cancelled turn | **partial** | `recoverFromError(myTurn:)` + `guard generationNo == myTurn` in both retry branches. The fix is complete; but there is no deterministic lock, and if the guard falls out in a refactor no test catches it — which is exactly what the audit asked for on this item. |
| **P1-8** argument correctness in eval | **partial** | `TestCase.inputMustContain` / `outputMustContain` were added, an `ARGUMENT CORRECTNESS (P1-8)` assertion block exists. But the acceptance criterion was "the hidden error must SURFACE" and it was not isolated: calendar 90→95, mixed in with the P0-4 enum. The measuring instrument exists, the measurement does not. |
| **P1-9** language anchor (NLLanguageRecognizer) | **partial** | 9 candidate languages, `confidenceBase = 0.50`, a ≥8-letter requirement, a three-valued `Result` enum — separating "not measured" from "wrong" prevents a false red. But a 9-language run was never executed and reported before/after; how many deviations it catches is unknown. |

## 4. P2 items

| Code | Verdict | Summary |
|---|---|---|
| **P2-1** narrow escape hatch in the document lock | **partial** | The `openWeb`/`hasContactSignal` computations were moved ABOVE the lock (previously the lock was the first line and these computations never ran at all). The escape hatch is narrow and justified. But the three regression-protection assertions the audit called MANDATORY do not exist; if the hatch is widened later, "show this as a table" can silently drift. |
| **P2-2** skill injection distance | **partial** | An `InjectionState` state machine (`turn - last >= distance`) instead of a permanent Set; a skill skipped because the profile did not match is no longer marked by mistake. The correctness of the distance value and the behavioural gain over a long turn were not measured. |
| **P2-3** dead `language` field | **partial** | `grep "var language" Tacet/Tools/*.swift` → ZERO (the acceptance criterion literally). But the second criterion was "success on code cases must not drop" and code went 82→64. This is probably not the cause of the drop (P0-1/P1-1 are stronger suspects) but because it was not isolated, "no regression" cannot be proved. |
| **P2-4** imperative instruction in tool output | **partial** | Two known violations (MCPTool:331, CreateDocumentTool:130) were converted to factual language. But the acceptance criterion was ZERO matches and **three live violations remain**: the `remote_tool_error` / `remote_tool_empty` lines in `ModelService` and `AnswerFilter:723 "Tell the user plainly…"`. The grep assertion would fail if run today. |
| **P2-5** announce the attached document to the prompt | **partial** | The `[Attached document: …]` line is written ONLY when `read_document` is in the session set (it spends no tokens in a profile without the tool) — both halves were met. documentRead +14 is supporting evidence but shares a category with P1-5/P2-6, so it cannot be attributed. |
| **P2-6** `read_document` → DataStore offload | **partial** | Tables/long text go to the store IN FULL, `data_ref=…` to the model; the preview depends on the offload (10 when a ref exists, the old 30 otherwise) — if the store is not attached no new loss path is opened. Truncation is now a window decision, not data loss. There is no fixture test, and the size of the saving is unknown. |
| **P2-7** deviation matrix + mutation check | **held** | Assertions 460 → 573, FAILED 0; against a target of +12 this is +113. The mutation check was ACTUALLY performed (deleting the schema guards turns 4 red, restoring them turns them green). One caveat: some of the assertions were locking pure functions that are not wired into production (see §6). |
| **P2-8** keep chips after a retry | **held** | In `ToolExecutor.newTurn`, `traces = []` is no longer unconditional; it sits inside the `forgetSideEffect` branch. The first attempt's traces are carried into `ReplyOutcome`. There is no numeric assertion, but the behaviour is in a single `if` block — low risk. |
| **P2-9** MCP name dedup + description cap | **partial** | The description cap is live (`descriptionCap = 160`, cut at a word boundary, at both tool and field level). Dedup (`resolveNames`) was written but was NOT WIRED — see §6. |

## 5. Eval before/after — category table

`clean-raw-shard0.json` (BEFORE) and `AFTER-raw.json` (AFTER) measure **the same 109
cases** (intersection 109/109), so the comparison is matched. Unmeasurable turns: 0 in
both runs.

| Category | BEFORE | AFTER | Δ | n |
|---|---|---|---|---|
| search | 100.0 | 90.0 | **−10.0** | 4 |
| documentRead | 85.0 | 98.6 | **+13.6** | 7 |
| documentWrite | 100.0 | 96.2 | −3.8 | 8 |
| security | 94.0 | 100.0 | **+6.0** | 5 |
| reminder | 95.0 | 99.0 | +4.0 | 5 |
| calc | 85.7 | 82.9 | −2.9 | 7 |
| contact | 100.0 | 100.0 | 0.0 | 4 |
| code | 82.5 | 64.4 | **−18.1** | 8 |
| chat | 95.0 | 95.0 | 0.0 | 8 |
| calendar | 90.0 | 95.0 | +5.0 | 9 |
| webSearch | 100.0 | 76.0 | **−24.0** | 5 |
| webPage | 100.0 | 86.2 | **−13.8** | 4 |
| time | 93.3 | 93.3 | 0.0 | 3 |
| chain | 90.3 | 86.9 | −3.4 | 32 |
| **OVERALL** | **92.1** | **89.1** | **−3.0** | **109** |

Additional runs (no before/after match, tagged with a separate `Run` label in the Excel):
- `AFTER-mcp` — 61 cases, overall **93.0**. Breakdown: mcp 95.2 (n=29), mcp-chain 94.1 (n=28), mcp-gate 100 (n=1), **mcp-disconnected 60.0 (n=3)**.
- `BEFORE-extra-shard2` — 96 different cases, overall 90.7. Case intersection with `AFTER-raw` is ZERO.

## 6. Two "false greens" — closed in this run

Two new instances of the regression pattern the audit itself diagnosed (§5.4 "a
mechanism that is not wired into production") had been named in the P1-6 and P2-9
verdicts: both mechanisms were written, their tests green, **but they were never
used on the production call path.**

In this run both were wired to the real call path:

- **`ToolRelevance` (P1-6).** `ModelService.connectionTools()` now calls
  `selectedMCPTools()` instead of `Array(mcpTools.prefix(cap))`; that in turn orders
  the pool by the user's request for that turn via `ToolRelevance.sort`.
  The translation cap of `setUpTools` was separated from the slot (6) and became the
  pool (24) — had the pool stayed at 6 as well, the ordering could not have gone
  beyond reshuffling the server's first six among themselves.
  `toolSignature(.connection)` now writes the **selected names** rather than a count;
  otherwise the session would never be rebuilt after the first turn and the mechanism
  would stay dead again (the same trap as the Contact ↔ web swap in the everyday set).
- **`resolveNames` (P2-9).** `MCPToolBridge.setUpTools` now resolves names for the
  whole candidate list and passes the `MCPTool(resolvedName:)` parameter;
  `get-user` and `get_user` no longer collapse to the same name and shadow each other.

The build is clean; self-test **573 assertions / 0 failed** (62 asynchronous included) —
the wiring broke no assertion. The verdicts stand in the table **as the audit run gave
them**: this wiring was done after the audit and was not confirmed by a new verdict run.

## 7. Remaining risks

1. **The overall average DROPPED by 3 points and the cause was not isolated.** The loss
   concentrates in three categories (code −18, webSearch −24, webPage −14); the shared
   suspects are P0-1 (skill core) + P1-1 (rewriting the instruction). Because both were
   applied in the same round, the culprit cannot be told apart.

2. **Variance is still unknown (the P0-5 gap).** Because temperature is not pinned, how
   much of the 92.1→89.1 difference is real regression and how much is sampling noise
   cannot be told apart. **This is the root risk that also makes item 1 questionable**;
   the single missing line (`ModelService.swift:1259`) was not a technical obstacle but
   an ownership one. This is the highest-leverage next piece of work.

3. **P2-4 did not close.** Three live imperative instructions remain
   (`remote_tool_error`, `remote_tool_empty`, `AnswerFilter:723`). The acceptance
   criterion was ZERO matches; the grep assertion would fail if run today. A cheap and
   entirely mechanical close.

4. **Behaviours with no test lock — six items.** P1-3, P1-4, P1-7, P2-1, P2-2, P2-6 work
   correctly but have none of the deterministic assertions the audit asked for; all of
   them can regress silently. The two riskiest: P1-7 (if the guard falls out in a
   refactor the invisible production race comes back) and P1-4 (a wrong `tools:` tag
   silently annihilates the skill).

5. **P1-2 is entirely unmeasured.** The recovery mechanism is live and cancellation-safe,
   but there is neither a deterministic assertion nor an eval case.

6. **Three items pile into the same category.** P1-5, P2-5 and P2-6 all affect
   documentRead, and which of them the +14 came from is unknown. Category-level eval does
   not have the resolution to isolate a single item.

7. **mcp-disconnected 60.0 (n=3)** — the weakest point of the MCP eval is the answer given
   when there is no connection. A small sample, but 34+ points behind every other MCP
   subcategory.
