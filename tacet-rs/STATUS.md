# STATUS — tacet-rs (product name: **Tacet**, binary: **`tacet`**)

Every line below was REALLY RUN while this file was being written; none of it
was written from memory. **Last run: 27 Jul 2026**, macOS arm64 (Darwin 25.5),
Homebrew cargo/rustc 1.96 — AFTER the NAMING and ADDON arms merged, AFTER the
ENGLISH-TRANSLATION arms merged, and after the integration fixes of the
LANGUAGE round were made. The detailed dump is in the "INTEGRATION ROUND —
the THIRD seam" section at the very bottom.

| Measurement | Result |
| --- | --- |
| `cargo build --workspace` | clean, no warnings, exit code 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean, exit code 0 |
| `cargo test --workspace` | **607 passed, 0 failed, 6 ignored** (23 test targets) |
| `cargo run -p tacet-cli -- eval` | **21/21 (100.0%)**, exit code 0 |
| `cargo run -p tacet-cli -- models list` | 4 packages · 4 usable, `qwen3-4b` selected, exit code 0 |
| `cargo run -p tacet-cli -- addon list` | "no addon installed." + the install command, exit code 0 |
| `cargo run -p tacet-cli -- package list` | 4 embedded · 0 user · 4 total, exit code 0 |
| `cargo run -p tacet-cli -- config list` | `model` and `engine`, both unset; file path printed |
| `cargo run -p tacet-cli -- tools` | **9 tools without addons; 11 with an addon record** (both measured, the second under a throwaway `HOME`) |
| `cargo clippy -p tacet-engine --features metal --all-targets -- -D warnings` | clean — **only after the `std::env::var` regression below was fixed** |
| `cargo test -p tacet-engine --features metal` | 40/40 |
| `cargo clippy -p tacet-cli --features metal --all-targets -- -D warnings` | clean |
| `ls -l target/debug/tacet` | one binary, named **`tacet`**, 16,784,248 bytes |
| `cargo test -p tacet-mcp` | 55 unit (1 ignored) + 2 end-to-end, 0 failed — the network identity measurement included (§5) |
| `grep -rni --exclude-dir=target .` for the two old brand names | **0 matches** |

**THE SUBCOMMAND NAMES MOVED** in the English round and the rows above use the
new ones: `model list` → **`models list`**, and `araclar` → **`tools`**. `config`
is a new subcommand that did not exist when the previous table was written.

**A REGRESSION THE DEFAULT BUILD CANNOT SEE.** The mass identifier rename mapped
the Turkish `var` ("there is") onto `has` and rewrote `std::env::var` — a Rust
standard-library call — into `std::env::has` in `tacet-engine/examples/`. Neither
`cargo build --workspace` nor `cargo clippy --workspace --all-targets` catches it,
because both examples carry `required-features = ["candle"]` and are therefore not
built under default features. It only appears under `--features metal`, which is
exactly why that row is in this table. Fixed in this round; the lesson is that a
green `--workspace` is NOT full coverage while feature-gated targets exist.

NOT RUN (honestly):

- **`cargo check`/`cargo build` on Windows.** There is no rustup on this machine,
  the target std library is absent and was not installed.
- **`cargo tree --target x86_64-pc-windows-msvc`.** It ran in earlier rounds
  (`crossterm ... bracketed-paste,events,windows` resolved) but that line is
  evidence of DEPENDENCY RESOLUTION, not of COMPILATION; it was removed from the
  table because it was misleading.
- **`eval --tool-selection` with a real model.** It wants a weight file and takes
  minutes.

## DIFFERENCES from the previous table — and why

- **192 → 607 tests.** The tests of the layers added in between (skills, memory,
  web, MCP, `write_code`, thinking extraction), plus the `config` subcommand added
  in this round. A bigger number DOES NOT MEAN
  bigger coverage; the "still not wired" list below still holds.
- **`--features candle` → `--features metal`.** `metal` already turns on `candle`
  (see `tacet-cli/Cargo.toml`) and that is the path that really runs on this
  machine. Writing an unmeasured flag into the table would have looked as if it
  had been measured.
- **`grep -rn "ignore\]"` NO LONGER COMES BACK EMPTY.** The previous table said
  "it comes back empty"; today there are 6 `#[ignore]`s and ALL OF THEM are
  tests that GO ON THE NETWORK: `tacet-mcp` (1: a real MCP server), `tacet-web`
  (2: a real SearXNG + a relevance measurement), `tacet-tools/web_search` (3: a
  real network). The rule did not change ("a test that goes on the network shall
  be ignored"); what changed is that the network layers exist. There is not a
  single silenced logic test.

---

## SECOND ROUND — the terminal shell + wiring (this section is new)

The shell now runs with a REAL Qwen2.5-3B (`tacet chat`, `--features metal`).
The real-model evidence (verbatim):

- A plain question, asked in Turkish ("Merhaba, kısaca kendini tanıt" —
  "Hello, introduce yourself briefly") → a coherent Turkish introduction, no
  tool. The prompt is quoted verbatim because what was measured here IS the
  Turkish-language behaviour; translating it would destroy the evidence.
- A tool, also asked in Turkish ("125 çarpı 8 kaç eder?" — "what is 125 times
  8?") → it produced
  `calculate({"expression":"125*8"})`, the chip read `[=] 125*8 = 1000 · Read`,
  and the model passed the result on CORRECTLY ("1000").
- Web: the model sometimes produces a WRONG surface form such as `time={...}` /
  a bare `{...}` instead of `web_search(`; by grammar design this falls to plain
  text and no tool runs (tool CHOICE is not enforced — see README). The
  `web_search` WIRING itself was proved with a script: `[globe] searched · 19
  results` (a real SearXNG, the bypass channel).

WIRED UP (a REAL call on the production path, verified by hand):

- **Skills → `Prompt::guide`**: in the `--show-prompt` output, `<guidance
  name="calc">` sits right in front of the question, within 700.
- **Memory → prompt + tool**: the `remember` tool wrote to disk (`memory.json`
  in the configuration directory; in that run the path was still the hidden
  folder named after the old brand), and IN A SEPARATE PROCESS a matching
  message injected `<memory><memory>- The user is a vegetarian`.
- **The approval gate (interactive)**: in a tainted session `web_search` showed
  the REAL payload and asked `[e/H]`, "h" → `[approval] web_search · not sent ·
  NeedsPermission`.
- **The router's cap of 8**: in that run, on a broad message with a 10-tool
  catalog, 8 tools were selected. (The catalog today is 9 without addons, 11 with
  the web search addon; the numbers are confirmed with `tacet tools`.)
  (In the first round there were 4 tools and the cap never came into play.)
- **Streaming output**: `EngineProvider::generate_streaming` was added;
  CandleEngine streams token by token, FakeEngine is compatible with the default
  (single-chunk) implementation.

GAPS (honestly):
- **Cancelling generation with Ctrl-C IS NOT WIRED**: std offers no signal
  handling; real cancellation wants a libc/ctrlc dependency and clashes with the
  zero-dependency identity. Right now Ctrl-C kills the process (the terminal
  default).
- **The edit_document session watcher (tier 2) is not fed**: because
  `ExecutionOutcome` does not carry a `file_path`, the watcher cannot be filled;
  the tool still works by falling back to tier 3 (the most recently changed
  document).
- **The eval catalog (5 tools) and the CLI catalog (9 today, 11 with an addon)
  HAVE DIVERGED**: eval is deliberately network-free/deterministic; adding the
  real `web_search` would break that invariant. The evidence for the new layers
  is in their own crate unit tests (32 skills, 24 memory, 36 web, 54 mcp). NO new
  case was added to eval — deliberate, but this is where item D is left half
  done.

---

## WIRED — mechanisms that really run on the production path

| Mechanism | Evidence |
| --- | --- |
| **The 4096 bypass channel** | `read_document` → `SharedStore` → `create_document`. Verified end to end in the CLI: a 201-line file was produced, but NOT ONE LINE of the bulk data appeared in the `--show-prompt` output. |
| **Constrained generation (grammar)** | `CallConstraint` implements `Constrainer`; the CLI and eval call `engine.generate(..., Some(constraint), ...)`. 20 of eval's 21 cases run with the constraint ON. |
| **The token mask** | `TokenMask::mask` is now called from the production path (`call.rs`), not only from tests. |
| **The engine vocabulary** | `EngineProvider::vocab`; `FakeEngine` (code point = token) and `CandleEngine` (tokenizer `decode`) both report it. |
| **The four gates** | NAME/SCHEMA/APPROVAL/CANCELLATION — in order inside `run`, all structural (no text matching). |
| **The approval gate setup** | The CLI now really applies the `EXTERNAL_TOOLS` list (`external_tool(...)`). The list is EMPTY this round — see below. |
| **The `retryable` control flow** | The CLI turn loop READS the flag: `is_error && !retryable` → no recovery turn is opened. There used to be 7 writes / 0 reads. |
| **The trace collector (chips)** | The CLI prints them; eval now HOLDS ON to the collector and adds the chip states to the evidence pool (`TraceCollector::world_changed` included). |
| **The single write gate** | `write_document` is a free function; the 0600 stamp cannot be invalidated. The CLI smoke test verified `-rw-------` — **ON UNIX**. On Windows the stamp is deliberately not applied (there `set_permissions` only flips the read-only flag and provides no privacy); the note is in `create_document`, `write_code`, `tacet-memory/store.rs` and the README. The Windows behavior WAS NOT MEASURED. |
| **Model package downloading** | `tacet_web::download` → `tacet-cli::model_download` (`tacet models download <name>`). It stood "tested but not wired" for a round; now it has a production caller. The measurements are below (INTEGRATION ROUND §2). |
| **A single ref wire format** | `source_ref_suffix` — both `ToolOutcome::summarized` and `read_document` go through the same function. |
| **Session constants** | `MAX_TURNS` + `SYSTEM_INSTRUCTIONS` in `tacet-engine`. The production binary no longer depends on the test crate. |
| **tacet-zip inflate** | `read_document` opens a real `.xlsx`; no panic on broken input (bounds checks + a zip-bomb cap). |
| **The router** | The CLI and eval use `select(...)`; the full catalog does not go into the prompt, the cap is 8. |
| **The addon gate (web search)** | `tacet_web::addon::web_search_enabled()` → `production_catalog`. Not a mechanism but a GATE: in an installation without the addon, `web_search`/`web_fetch` ARE NOT in the catalog. Measured end to end: without the addon `tacet tools` shows 9 tools, once the record is written, 11. Test: `the_production_branch_reads_the_addon_gate`. |

---

## TODO / still NOT WIRED — honestly

### 1. The skill store — OUT OF SCOPE (deliberate)
`Prompt::guide` and `GUIDANCE_LIMIT = 700` are written and tested but they have
**zero callers in production**. The `SkillStore` layer that would feed them is
not in this round's scope (README "Out of scope"). Marked in `prompt.rs` with
`TODO(skills round)`. Today the 700 limit passes a unit test, not the production
path.

### 2. `EXTERNAL_TOOLS` is empty — the mechanism is wired, it has no input
The CLI sets up the gate but there is NO real external tool to write into the
list (no tool in the catalog takes data off the device). So on the `tacet chat`
path the approval gate still cannot actually be triggered — but it is no longer
UNREACHABLE, only INPUT-LESS. The difference matters: previously there was no
code, now the only thing to do is write a name into the list. The gate itself is
really measured in eval with `send_out`.

### 3. The constraint does not enforce tool SELECTION
The model can choose to answer plainly; the grammar is binding only AFTER
`tool_name(` has been written. An invented tool name falls to plain text as far
as the grammar is concerned and is rejected at gate 1. This is a deliberate
design (see README), but it is NOT full protection against the regression "the
small model starts explaining instead of calling a tool".

### 4. APIs that still have no production caller
They were not removed because they are legitimate as a contract/UI surface, but
today only tests call them — honestly, they should count as dead:

- `ToolExecutor::retry_safe` — the `retryable` field answers the same question
  and that is the one that is WIRED. The risk is here if the two paths diverge.
- `ToolExecutor::recovery_attempt` / `cancel` — there is no user interface in the
  CLI to cancel with; these are the wiring points of the UI round.
- `FreeConstraint` — since `CallConstraint` arrived, its only user is a test.
- `InMemoryDataStore` — production uses `SharedStore`; this is the core's
  reference implementation and the foundation of every tool test (see "false
  alarms").

### 5. NOT MEASURED with a real model
`--features candle` compiles, clippy is clean, its tests pass — but it has NEVER
been run with a real GGUF + tokenizer pair. The risk below follows from that.

---

## Known risks

**(H) High — `vocab_setup` was not verified.** `CandleEngine::vocab` converts
tokens to surface text with `decode(&[id], false)`. The rationale is sound
(`id_to_token` gives BPE marks — `Ġ`, `▁` — raw, and because the grammar works
character by character it would build the mask wrong from the start), but this
conversion was not measured with a real tokenizer. If it is wrong the symptom is
clear: the constraint rejects valid JSON and the error `"the constraint forbade
every token"` comes back — a noisy failure, not a silent corruption. This is the
first place to look in the first real model run.

**(M) Medium — the constraint rejects a token that crosses the grammar boundary
with `)`.** If a single token like `"})"` both closes the arguments and ends the
call, the mask DOES NOT OPEN it; the model is forced to produce `}` and `)`
separately. Such merged tokens are common in real vocabularies, so this can make
generation harder. The fix requires intra-token transition tracking; it was not
done this round.

**(M) Medium — `MAX_TOOLS = 8` is not binding in production.** There are 4 tools
in the catalog; the cap can only be forced in a synthetic test catalog. Until the
tool count passes 8 this is a dead protection.

**(L) Low — the `eval --threshold` unit trap.** `--threshold` is a fraction
0.0–1.0; someone expecting a percentage and writing `--threshold 90` gets a
silent failing exit. Range validation WAS NOT ADDED (a semantic decision); it is
written out explicitly in the README.

---

## FALSE ALARMS in the auditor reports

Three auditor reports were not applied blindly; every claim was verified in the
code. These did not hold:

1. **"`table_summary` LEAKS Turkish text TO THE MODEL."** Wrong. `table_summary`
   is called only on the fallback branch that runs WHEN `ReadDocumentTool`'s
   TYPED STORE IS NOT WIRED (`read_document.rs:80`) and the text it produces is
   the summary of the STORE RECORD of `ctx.store(...)` — it does not enter
   `to_model`. Because production (CLI and eval) always uses `with_store`, that
   branch never runs. Not changed.
   The real one was the other: `"\n(full content ready, source_ref=...)"` REALLY
   DID enter `to_model`; that was fixed.

2. **"`InMemoryDataStore` is effectively dead."** Misleading. It is the reference
   implementation of the core's `DataStore` contract and the foundation of ALL of
   the `read_document`, `time`, `calc`, `executor`, `router` tests. "Not used in
   production" is true, "dead" is not — if it were removed, the tests of 5 files
   would collapse. Not changed.

3. **"Build report: no fix was needed, the workspace was already green."** True
   but it gave a MISLEADING confidence: a green build was hiding the fact that the
   grammar was not wired into production at all. "Build + test green" and "the
   mechanism works" are separate things — that is this round's real finding.

4. **The `eval --threshold 100` "bug"** — as the auditor themselves admitted, it
   was their own test error, not a defect in the code. Verified, not changed.

---

## Changes made this round (summary)

- `tacet-grammar/src/call.rs` **(new)** — `CallConstraint`, 6 tests. The grammar
  is now wired into production.
- `tacet-engine/src/session.rs` **(new)** — `MAX_TURNS`, `SYSTEM_INSTRUCTIONS`
  moved out of eval.
- `EngineProvider::vocab` added; `FakeEngine` and `CandleEngine` implement it.
- CLI: the constraint was wired, `EXTERNAL_TOOLS` applied, `retryable` is read.
- Eval: the constraint was wired, `TraceCollector` is held on to, the
  `EvalCase::unconstrained` flag was added (which defense layer is measured is
  written in the case).
- `create_document`: `DocumentEngine::write` → the free function
  `write_document`.
- `outcome.rs`: `source_ref_suffix` as the single wire format; the dead
  `permission_required` was removed.
- `Cargo.toml`: the unused `anyhow` and `tokio` workspace dependencies were
  removed (the code already said "we do not use tokio").


---

## INTEGRATION ROUND — the seams (this section is new)

The brand surface arm and the language/package arm worked in the same crates;
this round MERGED the places where the two touched. What was done and why:

### 1. The configuration directory — THREE implementations, ONE function

The path was written as `HOME` + a hidden folder in THREE SEPARATE places
(memory, skills, MCP) and nothing guaranteed the three would stay the same:
changing one would SILENTLY separate the others — the user's skills would end up
in one directory and their memory in another. The single source is
`tacet_core::env`:

| platform | directory |
| --- | --- |
| Unix (XDG set) | `$XDG_CONFIG_HOME/tacet` |
| Unix (no XDG) | `~/.tacet` |
| Windows | `%APPDATA%\Tacet` |

`tacet-mcp::config::default_path` was still reading its own `HOME` — it was
wired up. `tacet-memory` was CALLING `tacet_core` but NOT DECLARING it in its
manifest: the workspace did not compile (`unresolved module tacet_core`,
`store.rs:141`). The dependency was added.

**Divergence is now caught at run time:**
`memory_skills_and_mcp_point_at_the_same_config_directory` in `tacet-cli` —
that is the only crate that sees all three layers at once. (`tacet-mcp` was added
to `tacet-cli` as a DEV dependency for this; the production graph did not
change.)

**DELIBERATELY LEFT SEPARATE:** the `~/models/<name>` path. A model weight is not
a setting but gigabytes of data; putting it in `%APPDATA%` would be wrong. This
is not a duplication but a separate concept (the rationale is written in
`main.rs`).

### 2. Environment variables — ONE NAME, `TACET_*`

The variable names that are read are these, and NO other name is read:

| name | who reads it |
| --- | --- |
| `TACET_HOME` | overrides the configuration directory (memory+skills+MCP) |
| `TACET_MODEL` / `TACET_TOKENIZER` | the `tacet` discovery path **and** the warning text |
| `TACET_TRACE_DUMP` | the CLI chip dump **and** `CandleEngine` |
| `TACET_SEARXNG` | the `tacet-web` client — no longer the ONLY address source, but the FIRST in order: variable > addon record > none |
| `TACET_TZ_OFSET` | the `time` tool |
| `TACET_MCP_CONFIG` | the MCP file path |

`TACET_MODEL` was especially important: its name was written separately in the
DISCOVERY path and the WARNING path. If those two diverge the failure is silent —
the user reads the warning "set TACET_MODEL", sets it, and nothing changes. They
now share a single constant and
`the_model_variable_name_appears_verbatim_in_the_warning_text` measures this.

**THE BACKWARD-COMPATIBILITY BRIDGE WAS REMOVED (this round).** For a while
`env_var` took two names — if the new name was absent it fell back to the old one
and printed a warning to stderr once. The rationale was "let's not break a
working shell profile" and that rationale rested on an UNMEASURED assumption: the
app was never released, there is no shell profile that ever wrote a variable
under the old name, and a configuration directory under the old name was never
created on any machine. Code carried to protect a non-existent user was keeping
the old brand ALIVE in the code base. The function came down to a single
argument; the only remaining contract is "an empty value counts as undefined".

### 3. `tacet package list` — NEW

The skill layer was silent: a matching skill enters the prompt, and the user
could see what was loaded only via `--show-prompt` and only if that skill matched
IN THAT TURN. A broken `.md` dropped into the user directory is DELIBERATELY
skipped in silence (the right trade-off) — but the answer to "I put my file
there, it doesn't work" existed nowhere. The command is where that answer lives;
it prints the configuration directory too, because the user cannot be expected to
guess WHERE to put the file. `--json` for scripts.

### 4. The binary's name is `tacet`

`[[bin]] name = "tacet"`; `target/debug/tacet` is produced. The directory and
crate names are also `tacet-*`; details below in the "DIRECTORY AND CRATE NAMES"
section.

### 5. NETWORK IDENTITY `tacet` — the only place where the internal and external
names diverge

Saying that the crate name is an internal identity does NOT mean it is nowhere
visible: the MCP client states a name when introducing itself to a remote server,
and for a while that name was the old brand. Two spots in
`crates/tacet-mcp/src/client.rs`:

| Where | today |
| --- | --- |
| HTTP `User-Agent` | `tacet/1.0` |
| `initialize` -> `clientInfo.name` | `tacet` |

These two values land in the log file of the THIRD PARTY the user connected —
that is, somewhere we cannot take them back from. This was the residue of the
brand migration that would be noticed latest and could be fixed latest: the build
was green, the tests passed, nothing was shouting.

For the same reason **looking at the code and saying "I changed it" was not
counted as enough**. The fake server in `crates/tacet-mcp/tests/local_server.rs`
now records the incoming `User-Agent` header and the `clientInfo.name` in the
`initialize` body; the test reads both OFF THE WIRE and compares them, on both
plain JSON and SSE transports.

Measured (together with a negative control): when the constant is turned into a
wrong value the test goes red —

```
assertion `left == right` failed: the User-Agent header must carry the product name
  left: Some("<wrong-name>/1.0")
 right: Some("tacet/1.0")
```

then it was reverted and `cargo test -p tacet-mcp` went green again (54 unit + 2
end-to-end). So the claim is not a comment but a breakable measurement.

### Fixed along the way (it was not this round's job but it was blocking green)

Five clippy findings going red under `-D warnings`: `unnecessary_to_owned` and
`non_snake_case` (a test name) in `tacet-engine`, `doc_lazy_continuation` (3
lines) and `nonminimal_bool` in `tacet-tools` `executor.rs`,
`needless_borrows_for_generic_args` in the `gemma_probe` example. All of them
behavior-free; none of them silenced with `#[allow]`.

---

## INTEGRATION ROUND — the SECOND seam (language package ⨯ cross-platform)

Two arms had run in the same working area this round: **Arm A** the model package
catalog/discovery and `tacet-web/download.rs`; **Arm B** cross-platform (the
Windows crossterm feature, time zones, the Linux shield, eval skip
classification). There was NO CONFLICT at the file level — the two had written to
separate files. What was left was whether the seams really joined up.

### 1. The only SEAM FAILURE found: a missing dependency line

Arm A had written and tested `tacet-web/src/download.rs` (the approval gate,
resuming with Range, atomic swap, SHA-256) but COULD NOT WRITE the `tacet model
download` subcommand: the `tacet-cli` manifest had no `tacet-web` dependency and
that file belonged to Arm B. So the module WAS NEVER CALLED FROM PRODUCTION —
this repository's recurring failure ("a mechanism is built, it is not wired into
production, and because the build is green nobody notices") was exactly this, and
neither arm could have solved it alone.

Done: `tacet-web.workspace = true` in `tacet-cli/Cargo.toml`, the
`ModelJob::Download` branch, the `model_download` function, the terminal approval
gate (`[e/H]`) and the progress line.

**NO new EXTERNAL dependency WAS ADDED.** `tacet-web` (and `ureq`) was ALREADY in
this binary's graph via `tacet-tools` (`web_search`); the line made the indirect
one direct. Not a single new package entered `Cargo.lock` from this change — what
entered was only a `tacet-web` line in the `tacet-cli` dependency list (confirmed
with `git diff Cargo.lock`).

**The network monopoly was not broken.** The rule was "a network call is FOUND
only in `tacet-web` and `tacet-mcp`", not "nobody may call them". `tacet-cli` does
not open a socket and does not pull `ureq`; it only supplies the terminal end.

### 2. `tacet models download` — it WAS REALLY RUN (verbatim measurements)

With a fabricated root (`HOME`) + a fabricated `packages.json` (`TACET_HOME`),
the real binary (`target/debug/tacet`) was run:

| Measurement | Result |
| --- | --- |
| The file was already in place | `✓ model.gguf (13 B, already present — no network call, sha256 not in the catalog)`, exit 0 |
| The hand-written SHA-256, against the system tool | **byte-for-byte identical** to `shasum -a 256` (for both files) |
| A WRONG sha in the catalog | `SHA-256 did not match (expected 000…, found 3f00…)`, exit **1** |
| The CORRECT sha in the catalog | `sha256 verified`, exit 0 |
| The approval gate, answer `h` | `The download was not approved; nothing was downloaded.`, exit 1 |
| An `http://` address | `the address must start with https://: …`, exit 1 (approval was NEVER asked) |
| `--no-approval`, an unresolvable host | a transport error came back → **the network layer really is reached** |
| A name not in the catalog | `'no-such-thing' is not in the download catalog` + what is in the catalog, exit 1 |
| No catalog at all | the example shape was printed, exit 1 |

That SHA-256 matches `shasum -a 256` byte for byte matters: the digest was
written BY HAND (to avoid adding a dependency) and until then it had only been
measured against the FIPS test vectors. This is a cross-check against an
independent second source.

**STILL NOT MEASURED:** downloading from a real server, resuming with Range and
the 200/206 distinction. The `--no-approval` run above proves the transport layer
IS REACHED, not that a body came down.

### 3. The failure the measurement surfaced: search wording was leaking into downloading

The first output of the `--no-approval` run was this:

```
model.gguf: The search server could not be reached.
```

The user was downloading a MODEL and saw a sentence talking about SEARCH. The
cause: `DownloadError::Network(e)` was writing the underlying `WebError`'s
`Display` VERBATIM and those strings were written for search. The type was right,
the build was green, the sentence was wrong — a silent failure.

The fix was made at the caller's boundary (`download::network_text`): the variant
is still carried from `WebError` (so the classification stays in one place) but
the SENTENCE is rebuilt in the download context; 404 and 403 are separate
sentences, because what the user has to do differs. Generalizing `WebError`'s
strings would have been a smaller patch but in the wrong place: on the search
side, what the user will fix is the SearXNG address and it is right for the
sentence to say so. Also, the transport layer's own explanation (DNS/TLS/refused)
CAME BACK in downloading; in search it is deliberately swallowed (the chip text
must be short).

After the measurement:

```
model.gguf: The download source could not be reached: io: failed to lookup address
information: nodename nor servname provided, or not known
```

The `a_network_error_is_told_in_the_language_of_downloading` test blocks a
regression.

### 4. THE USER IS NO LONGER LIED TO — but it is not overstated either

Arm A had deliberately left `model list` and the "model not found" report silent,
so as NOT TO SUGGEST a subcommand that did not exist. Now that the command
exists, the suggestion arrived too, but CONDITIONALLY: because the embedded
catalog is deliberately EMPTY, `tacet models download` can do nothing without the
user's own `packages.json`. So the suggestion is printed only while the catalog is
FULL; with an empty catalog "where to write the file" is shown as before, and
with a broken catalog it is said that it could not be read (that branch used to
be silent).

The verbatim measurement (a fabricated empty root + a full catalog):

```
(model package not found: 'trial')
  searched: …/home2/models
  no packages at all.
  downloadable (packages.json): trial
  to download: tacet models download trial
```

### 5. Brand residue — the last six places invisible to the user

The test temp directory labels (`tacet-read-document-`, `tacet-edit-`,
`tacet-writecode-`, `/tmp/tacet-calc`, `/tmp/tacet-memory-tool`,
`/tmp/tacet-web-test`) became `tacet-*`.

> THIS HEADING WAS ONCE WRITTEN TOO EARLY. When "the last six places" was said,
> the list was incomplete; an independent audit then found THREE more places and
> two of them WERE NOT TESTS: `run_code.rs`'s shield measurement was opening a
> `tacet-code-measure-<pid>` directory on the production path, the `find_file.rs`
> label had been left behind, and the worst — `SYSTEM_INSTRUCTIONS` in
> `session.rs` was still telling the model THE OLD BRAND, meaning that whenever
> the user said "what's your name" the assistant gave the old name. All three
> were fixed. The lesson goes on the record: the sentence DECLARING the cleanup
> is not the measurement itself. The prompt's name is now protected by the
> `system_instructions_state_the_product_name` test — none of the previous 578
> tests measured the prompt's NAME, only the CALL FORMAT.
>
> IT WAS WRITTEN TOO EARLY A SECOND TIME AS WELL: while this section said "what
> remains is deliberate", the old brand string still occurred in the source
> (crate names, historical comments, old environment variable names). The
> "DIRECTORY AND CRATE NAMES" section below removed that remainder too and tied
> the measure to a single sentence: a `grep -rni` searching for the old brand
> string returns ZERO matches in the repository.

### 6. The state of the two arms AFTER merging

Arm A's green had been taken with a full `--workspace`, Arm B's with
`--exclude tacet-cli` (at the time `main.rs` did not compile). This round the full
command list was run WITHOUT `--exclude` and it is green: build, clippy
(`-D warnings`), 578 tests, eval 21/21, `model list`, `package list`, `cargo tree`
against the Windows target. That is, it has NOW BEEN SEEN that Arm B's crossterm
change compiles `tacet-cli` on macOS (it was marked "not yet seen" in the previous
report).

### 7. NOT FIXED this round, deliberately

- **The `CodeDiagnosis` screen-printing branch WAS NOT MEASURED.**
  `session_catalog` prints the diagnosis to stderr (verified by reading the code)
  but on this machine the shield IS DISCOVERED, so the branch never runs. The way
  to force it went through breaking the interpreter/shield paths; dirtying the
  machine for the sake of a measurement is not a right trade-off. On Linux/Windows
  this is the first place to look.
- **XDG's `~/.local/share` default** has still not been added (Arm A's rationale
  holds: do not create an unmeasured third directory).
- **The TOFU record is not written automatically.** The computed digest IS PRINTED
  to the user and given in a form ready to paste into `packages.json`; writing the
  file ourselves would turn a digest the user has not reviewed into a "verified"
  one.

### 8. The numbers had gone stale: the catalog is 11 tools, not 10

> **THIS NUMBER WENT STALE IN THE FOLLOWING ROUND TOO.** Since the addon gate
> arrived, 11 is only the LARGEST case; in the default (addon-free) installation
> the catalog is 9 tools. The current dump is below in the "THIRD seam" section.
> The text below stands as the record of that round — and the irony is worth
> recording: the section that says "a number goes stale because it sits in a
> comment" had its own number go stale.

This round it was COUNTED with `tacet tools`: the production catalog has 11 tools
(calculate, time, read_document, create_document, edit_document, find_file,
run_code, write_code, web_search, remember, web_fetch). The `catalog.rs` comment,
the README (two places) and STATUS (two places) still said "10 tools", and the
comment's real claim rested on that number: "the router budget is 8, the last TWO
tools are never shown to the model". The correct answer is THREE. When
`write_code` was added the number shifted, the rationale did not — nobody noticed
because a comment does not compile.

The texts were fixed, but the real fix is a test:
`the_catalog_is_larger_than_the_router_budget` measures the catalog size AND THE
NUMBER OF TOOLS THAT DROP. The claim is not an equality, because
`run_code`/`write_code` are not added if the shield measurement does not pass: on
a machine with the shield 11 tools / 3 drop, without it 9 / 1 drops. If it goes
stale again, it is not the build but the TEST that goes red.

### 9. The README's "network monopoly" item WAS A LIE

The item said: "No crate makes a network call. `hf-hub` is deliberately closed:
the model file is supplied from outside, it is not downloaded." Neither sentence
was true any more — the first once web search and MCP arrived, the second this
round once `tacet models download` was wired. Instead of removing the rule it was
written in its REAL form: a network call is FOUND only in `tacet-web` and
`tacet-mcp`; calling is allowed, OPENING is not. `hf-hub` is still closed and its
rationale was written down: downloading goes through the user's own catalog, there
is no embedded mirror, https is mandatory, and it passes through the approval
gate.

## DIRECTORY AND CRATE NAMES — the old brand is entirely gone from the code base

This round did one job: it deleted the old brand name from EVERY layer except the
product name. Earlier rounds wrote "crate names are internal order, invisible to
the user, deliberately not changed". That rationale was a cost-benefit calculation
and the decision changed: as long as two names live in one repository, the
question "which one is current" gets asked again in every new file.

| what | old | new |
| --- | --- | --- |
| the workspace directory | the old brand + `-rs` | `tacet-rs` |
| eleven crate directories + `[package] name` | the old brand + `-<layer>` | `tacet-<layer>` |
| the use name in the source | the old name with an underscore | `tacet_<layer>::` |
| the `[workspace.dependencies]` keys and their `path`s | old | `tacet-*` |
| `Cargo.lock` | NOT by hand, regenerated with `cargo build --workspace` | |

`[[bin]] name = "tacet"` was ALREADY correct and was NOT touched; the binary's
name never changed.

**THE OLD ENVIRONMENT VARIABLE BRIDGE WAS REMOVED.** The detailed rationale is
above in the "### 2. Environment variables" section. The summary:
`env_var(new, old)` came down to a single argument (`env_var(name)`), and the
`OLD_HOME_VAR` / `OLD_PATH_VAR` / `OLD_ADDRESS_VAR` constants and the warning
mechanism (`WARNED` + `warn_once`) were deleted. `MODEL_VAR` / `TOKENIZER_VAR`
came down from a two-element tuple to a plain `&str`. The single preserved
contract: **an empty value counts as undefined**, and its rationale now stands in
the function's own doc comment.

**TWO TESTS WERE REMOVED BECAUSE THEY HAD WEAKENED, NOT HIDDEN.** The claims "no
old brand shall remain in the prompt" in `session.rs` and "the old brand name
shall not leak" in `create_document.rs` stopped measuring ANYTHING once the string
they were looking for no longer existed in the code base — a claim that always
passes is a green lie. The POSITIVE claim standing next to each of them is
preserved and that is the one that really protects: the prompt MUST START with
`"You are Tacet:"`, and the stem of an untitled document is EXACTLY
`tacet-document`. `the_leaf_of_the_platform_path_carries_the_brand_name` in
`env.rs` was turned positive for the same reason: it now measures the LEAF of the
configuration directory.

Also, the build artifacts under `target/` carrying the old name (45,652 files,
2.2 GB) were deleted. Being in gitignore did not make them "absent": `ls
target/debug` still showed `.rlib`s under the old name.

### What was run in the NAMING round (historical — the current table is at the top of the file)

Run: 27 Jul 2026, macOS arm64 (Darwin 25.5), Homebrew cargo/rustc 1.96, in the
new `tacet-rs` directory.

| Measurement | Result |
| --- | --- |
| `cargo build --workspace` | clean, no warnings, exit code 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean, exit code 0 |
| `cargo test --workspace` | **578 passed, 0 failed, 6 ignored** (23 test targets) |
| `cargo run -p tacet-cli -- eval` | **21/21 (100.0%)**, exit code 0 |
| `cargo clippy -p tacet-engine --features metal --all-targets -- -D warnings` | clean — the `measure` and `gemma_probe` EXAMPLES were compiled too |
| `cargo test -p tacet-engine --features metal` | 40/40 |
| `ls target/debug/tacet` | the binary exists, named `tacet`, 16,280,232 bytes |
| `grep -rn --include=*.rs --include=*.toml --include=*.md .` for the old brand string | **0 matches** (exit code 1) |
| the same search, case-insensitive, over ALL files (`--exclude-dir=target`) | **0 matches** |

The examples were also FORCED: they were `touch`ed and recompiled, because
`required-features = ["candle"]` means they are NEVER compiled on a default run
and both of their bodies changed this round. A "clean" coming from the cache would
not have counted as evidence here.

**NOT RUN (honestly):** cross-compilation. There is no rustup on this machine and
no std library for the Windows and Linux targets. The `cargo tree --target
x86_64-pc-windows-msvc` line from earlier rounds was also evidence of DEPENDENCY
RESOLUTION, not of compilation; it was not repeated this round. `eval
--tool-selection` with a real model was not run either (it takes minutes and wants
a weight file); the change does not touch the model path, but that is a rationale,
not a measurement.

---

## INTEGRATION ROUND — the THIRD seam (naming ⨯ addons)

Two arms worked in the same tree: **naming** (the old brand name was removed from
the code base, crate/directory names became `tacet-*`, the old environment
variable bridge was deleted) and **addons** (the `tacet addon` commands, the
`addons.json` registry, the catalog gate for the web search tools). Both were
green on their own. This section records the state AFTER MERGING.

**The build caught none of the seams** — and it should have been expected: ALL
FOUR of the failures found were at the level of comments, documents and relative
paths, that is, in a place the compiler never looks. A variant of this
repository's recurring failure: the mechanism is wired correctly, THE TEXT
DESCRIBING IT stays wrong, and a green build covers it up.

### 1. Broken cross-references — the Swift directory is NOW `Tacet/`

The Swift side had been renamed in another arm from `<old-name>/` → `Tacet/`, but
three references inside `tacet-rs` pointed at the old path. It was verified (`ls`)
that all three targets EXIST at the new path, so the fix is a measurement, not a
guess:

| file | old | new |
| --- | --- | --- |
| `README.md` | `../<old-name>/` | `../Tacet/` |
| `crates/tacet-skills/src/skill.rs` | `<old-name>/Beceriler/*.md` | `Tacet/Beceriler/*.md` |
| `crates/tacet-mcp/README.md` | `<old-name>/Servis/MCPIstemcisi.swift` | `Tacet/Servis/MCPIstemcisi.swift` |

(The old brand name is written with a placeholder in this table DELIBERATELY: had
it been written verbatim, this very file would have falsified the "0 matches"
measurement below. The same placeholder convention was used in the previous round
too.)

Two more failures came out of the same file:

- **`tacet-mcp/README.md` was still describing the old decision:** "the crate name
  `tacet-mcp` is an INTERNAL IDENTITY left over from the old brand and was
  deliberately not changed". That sentence is WRONG after the rename — the crate
  really did change. Arm A had cleaned the old brand STRING out of the file, but
  seeing that a sentence carrying no such string carries the same claim is not
  grep's job. The paragraph was turned into the truth; the distinction between the
  internal name and the NETWORK IDENTITY (the part that still holds) was
  preserved.
- **The relative path was wrong:** `../../mcp-connection-spec.md` → the file is not
  under `tacet-rs/` but at the REPOSITORY ROOT; the correct one is
  `../../../mcp-connection-spec.md`. Both were verified by resolution (`MISSING` /
  `PRESENT`).

### 2. The addon gate FALSIFIED the "11 tools" claim

The previous round (§8) had FIXED the catalog size at 11 and wrote it down as a
measurement — and it was true. When the addon gate arrived, the default
installation's catalog dropped to 9 tools, but three texts still said 11:
`README.md` (two places: the eval section and the command list) and the
`catalog.rs` ordering comment. What is more, the README's "The layers WIRED UP in
the second round" list counted web search unconditionally as "ON THE PRODUCTION
PATH"; it is now OFF by default.

Catalog size depends on two conditions. The two cases MEASURED on this machine:

| case | `tacet tools` | dropped on a message with no hint |
| --- | --- | --- |
| without an addon (the default installation) | **9 tools** | 1 |
| `addons.json` written + open | **11 tools** | 3 |

The record was measured by writing it by hand
(`{"ad":"web-arama","durum":"acik","ayarlar":{"adres":"http://localhost:8888"}}`)
— this is the shape the `a_hand_written_file_is_readable` test already supports;
nothing went on the network and the user's real configuration directory was NOT
TOUCHED (`TACET_HOME` was pointed at the scratchpad; at the end of the round
`~/.tacet` still does not exist).

**The shield-less machine cases (7 / 9) WERE NOT MEASURED** — on this machine the
code shield discovery passes, and the way to disable the shield is not an
environment switch but a machine capability. It was written conditionally in the
code and in the comment; it was not called "measured".

Again the real protection is not text but tests:
`the_catalog_is_larger_than_the_router_budget` measures BOTH states of the gate,
and `the_production_branch_reads_the_addon_gate` measures that the production
branch really asks the gate.

### 3. The eval ⨯ gate connection — DELIBERATELY LEFT AS IT IS

Inside `production_catalog` the gate is read like this:

```rust
let web_enabled = fixed_epoch.is_some() || tacet_web::addon::web_search_enabled();
```

That is, IN MEASUREMENT MODE the gate is never asked. At first glance this looks
like two unrelated concepts (a deterministic clock ⨯ the addon registry) riding on
a single flag, and it was reviewed during integration. IT WAS NOT CHANGED, because
the alternative is worse: making eval ask the gate too would run the same eval set
with a DIFFERENT catalog on a machine with and without the addon, and the tool
selection scores would become incomparable. `fixed_epoch` already means "machine
state is not read"; the addon registry is exactly machine state. Because the
production path passes `None`, the gate is ALWAYS asked in production and a test
measures that.

The remaining flaw, honestly: this is a LOOSENING of the rule "eval sees the same
list as the shell". The one-line alternative fix on the eval arm (adding
`web_search`/`web_fetch` to `DISCOVERY_BOUND`) is still on the table; the choice
belongs to eval's owner.

### What was run in the INTEGRATION round (current)

Run: 27 Jul 2026, macOS arm64 (Darwin 25.5), Homebrew cargo/rustc 1.96, in the
`tacet-rs` directory, AFTER the fixes above were made.

| Command | Result |
| --- | --- |
| `cargo build --workspace` | clean, no warnings |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo test --workspace` | **607 passed, 0 failed, 6 ignored** (23 test targets) |
| `cargo run -p tacet-cli -- eval` | **21/21 (100.0%)**, exit code 0 |
| `cargo run -p tacet-cli -- models list` | 4 packages · 4 usable, `qwen3-4b` selected, exit code 0 |
| `cargo run -p tacet-cli -- addon list` | "no addon installed." + the install command, exit code 0 |
| `cargo clippy -p tacet-engine --features metal --all-targets -- -D warnings` | clean (after the `std::env::var` fix — see the top table) |
| `cargo test -p tacet-engine --features metal` | 40/40 |
| `tacet tools` (without an addon / with the record written) | **9 tools / 11 tools** |
| `grep -rni --exclude-dir=target .` for the two old brand names | **0 matches** (exit code 1) |
| `find ... -iname` for the same two names (file/directory names) | 0 results |

Clippy DID NOT COME FROM THE CACHE: every `.rs` and `Cargo.toml` file was `touch`ed
and all eleven crates were re-checked. In a tree where two arms merged, a "clean in
0.08 seconds" is not evidence.

**NOT RUN (honestly):**

- **Cross-compilation.** There is no rustup on this machine; there is no
  Windows/Linux target std.
- **`eval --tool-selection` with a real model.** It takes minutes and wants a
  weight file.
- **The addon's local SearXNG install path.** `docker compose up -d` downloads
  hundreds of MB onto the user's machine and leaves a persistent container behind;
  the approval is the user's. The discovery part (finding docker/compose) was
  measured in the previous round.
- **Catalog size on a shield-less machine** (§2 above).

---

## LANGUAGE ROUND — the English pass and its FOURTH seam

The tree is now English: identifiers, file and directory names, comments, test
names, CLI output, tool schemas and skill triggers. What follows is what the
integration pass found AFTER the translation arms reported done — recorded
because every one of these was invisible to the check the arms actually ran.

### What the translation arms broke, and how it was caught

| Failure | How it hid | Where |
| --- | --- | --- |
| `std::env::var` rewritten to `std::env::has` | the two examples carry `required-features = ["candle"]`, so the default `--workspace --all-targets` never builds them | `tacet-engine/examples/{measure,gemma_probe}.rs` |
| A test made VACUOUS: the fixture emitted `Baslik N` while the assertion checked `Title 5` | it still passes — an assertion that can no longer fail is green forever | `web_search::the_text_going_to_the_model_shows_at_most_five_results` |
| Ignored network tests still asserting the PRE-rename wire format `kaynak_ref` | `#[ignore]`, so CI never runs them; they would fail the moment anyone did | `web_search` smoke tests (3) |
| Turkish leaking into the MODEL-facing surface: `"sayi"` as the type name in `short_signature()` | it is prompt text, not code — no compiler sees it | `tacet-core/src/schema.rs` |
| A model-facing repair hint naming a field that does not exist (`dil:"python"` — the schema field is `language`) | same: prompt text | `tacet-tools/src/write_code.rs` |
| User-facing Turkish left in shipped strings: `"Sifira bolme yapilamaz."`, `"Hafizaya su an erisilemiyor."`, `"Dosyalar araniyor…"`, `"Hesaplaniyor"`, `"{} giris gezildi"`, `"{total} web sonucu"` | never asserted by any test | `calc`, `memory`, `find_file`, `web_search` |
| Half-translated doc comments — sentences that stop mid-thought, orphan Turkish clauses, a dropped line | comments do not compile | `web_search.rs` above all (the file was ~40% untranslated) |
| Stale doc references to the pre-rename `DURUM.md` | prose | `examples/measure.rs`, `candle_engine.rs` |
| README documenting `tacet model list` when the binary only accepts `models` | the docs are not executed | `README.md` |

The common thread: **`cargo build` + `cargo test` is not a translation check.**
Everything above is either outside the default build graph, inside a string, or
inside a comment. The scan that actually found them was a dictionary diff (every
word in the tree checked against `/usr/share/dict/words` plus a technical
allowlist), not a grep for Turkish characters — most of the remaining Turkish was
ASCII-folded (`parca`, `dolu`, `birinci giris`, `veri`) and no accent grep can see it.

### Turkish that is DELIBERATELY still in the tree

Roughly 100 matches remain and every one is load-bearing. Removing them would
delete evidence or break behaviour, so they are listed here to stop the next pass
from "finishing the job":

1. **Legacy on-disk keys** (`serde(alias)`): `tacet-memory/note.rs`,
   `tacet-memory/store.rs`, `tacet-mcp/config.rs`, `tacet-web/addon.rs`. Read-only
   compatibility — nothing writes them. Deleting them silently drops an existing
   user's notes, MCP connections and web-search addon.
2. **Turkish case/accent-folding logic and its fixtures**: `skills/matching.rs`
   (`I`↔`ı`, `İ`↔`i`), `memory/note.rs::normalized_text`, `tools/router.rs::simplify`,
   `tools/find_file.rs` folding test, `web/relevance.rs::simplify`. The Turkish data
   IS the thing under test.
3. **Records of measured failures**, kept verbatim: the SearXNG ferry fixtures in
   `tools/web_search.rs` and `web/relevance.rs`, the model's invented timetable
   answer, the `ara`/`aralik` and `dok`/`dokuz` prefix-collision notes,
   `executor.rs`'s `yapacagim` note. A paraphrase is not evidence.
4. **`STOP_WORDS`** in `web/relevance.rs` — multilingual on purpose; the queries
   are still user text. Trimming it is a product decision, not a translation.

`web_search.rs` now carries an explicit DO-NOT-TRANSLATE note on its fixture
block, because that is the file a future pass is most likely to get wrong.

### Behaviour that CHANGED with the language switch (not a bug — a cost)

`router.rs`'s `message_triggers` and `time.rs`'s relative-day/month tables are now
English only. A user writing Turkish matches no trigger, the routing score falls
to 0 and the tool order reverts to catalog order; `time` returns an explicit error
rather than silently falling back to today. Making these lists multilingual is
follow-up work and is noted in the code.

### NOT MEASURED in this round

- **A real model.** Everything above ran on the `fake` engine plus unit tests.
  Whether English tool descriptions change a 4B/8B model's SELECTION probability
  is UNMEASURED; eval's 21/21 proves the schema and wiring line up, not that the
  model chooses as well as before.
- **The 4 skill `.md` trigger lists against a real model**, same reason.
