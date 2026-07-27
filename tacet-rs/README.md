# tacet-rs

Tacet's architecture ported to Rust. The Swift source (`../Tacet/`) is a
reference, not a one-to-one translation — here things are written idiomatic Rust.

**THE NAME.** The product is **Tacet**, the binary **`tacet`**, the crates
**`tacet-*`**.

The LOGIC layer of a fully on-device assistant: routing, prompt assembly,
constrained generation, tool execution and the 4096 token bypass channel. No UI,
no network.

## Invariants

1. **Naming is English**, the same vocabulary as the Swift side (`Tool`,
   `ToolOutcome`, `DataStore`, `Router`). Symbols are ASCII: `executor`,
   `source_ref`. Types `PascalCase`, functions/fields `snake_case`.
2. **Zero-dependency identity.** OOXML zip/deflate/crc32 is written by hand;
   there is no off-the-shelf zip crate. General dependencies: `serde`,
   `serde_json`, `thiserror`, `clap` (CLI only). If a new one is to be added,
   its rationale is written in the comment at the top of the file.
   `candle-core`, `candle-transformers`, `tokenizers` are ONLY under the
   `candle` feature and their rationales are written in
   `tacet-engine/Cargo.toml`.
3. **Network monopoly.** A network call is found ONLY in the `tacet-web` and
   `tacet-mcp` crates; no other crate opens a socket, and the HTTP dependency
   (`ureq`) also stands only in these two manifests, so the rule can be audited
   by eye. Calling is allowed, OPENING is not: `tacet-tools` (`web_search`) and
   `tacet-cli` (`tacet models download`) call these two but do not set up a
   socket themselves.

   > This item used to say "no crate makes a network call, the model file is
   > supplied from outside, it is not downloaded". Since web search, MCP and
   > model package downloading arrived, neither sentence was true. `hf-hub` is
   > STILL closed and that is deliberate: downloading goes through the user's own
   > `packages.json`, there is NO embedded catalog/mirror (see
   > `model_package::embedded_catalog`), https is mandatory, and the download
   > passes THROUGH THE APPROVAL GATE.
4. **The 4096 token bypass channel.** Bulk device data DOES NOT PASS through the
   model: the tool puts the data into `DataStore` and returns a short summary +
   `source_ref` to the model. Whatever needs the data in the next step is again a
   tool, and it takes it from the store by reference.
5. **Comments are in English and explain the WHY**, not what the code does.

## Layers

```
tacet-cli ──────────────► the developer shell; drives the turn loop
   │
   ├── tacet-eval ──────► cases, scoring, report
   │
   ├── tacet-grammar ───► ArgSchema → the constrained generation grammar
   │        │             CallConstraint: the REAL implementation of Constrainer
   │        ▼
   ├── tacet-engine ────► contracts: Prompt, EngineProvider, Constrainer,
   │                      TokenCounter, session constants (MAX_TURNS)
   │                      FakeEngine (default) / CandleEngine (--features candle)
   │
   ├── tacet-tools ─────► concrete Tool implementations + ToolExecutor + Router
   │        └── tacet-zip ──► hand-written zip/deflate/crc32 → OOXML generation
   │
   └── tacet-core ──────► THE CONTRACT: Tool, ArgSchema, ToolOutcome, ToolError,
                          ToolState, ToolContext, DataStore, ToolCatalog
```

The arrows are the direction of dependency: `tacet-core` depends on nothing,
everyone depends on it. The contract therefore does not bend under the pressure
of the implementations.

**The grammar → engine direction is deliberate.** `tacet-engine` DOES NOT KNOW
`tacet-grammar`; the `Constrainer` contract stands in the engine, its
implementation (`CallConstraint`) is in the grammar. Had it been set up the other
way round, the engine would have had to know the grammar's internal
representation (PDA, stack, token mask), and a build running unconstrained would
have compiled the grammar code for nothing.

### Crates

| Crate | Its job |
| --- | --- |
| `tacet-core` | The types all layers agree on. No work is done here. **Single-owner:** the others only read. |
| `tacet-zip` | Pure-Rust zip/deflate/crc32; OOXML (xlsx) generation and reading. No off-the-shelf crate. DOES NOT PANIC on broken input. |
| `tacet-grammar` | `ArgSchema` → grammar (PDA + token mask). `CallConstraint` wires this into the generation loop. |
| `tacet-engine` | Prompt assembly, context budget, the engine contract, session constants. `FakeEngine` + `CandleEngine`. |
| `tacet-tools` | The concrete tools (`calculate`, `time`, `read_document`, `create_document`), `ToolExecutor` (four gates), `Router`, `SharedStore`. |
| `tacet-eval` | Case-based evaluation and reporting. |
| `tacet-cli` | The developer shell (`clap`). |

## The core's red lines

**Error text has two channels.** The text going to the user is a human sentence
(`ToolError::short_error`); the text going to the model is FIXED:

```
tool_failed: the action could not be completed; no result was produced
```

Even if the model reflects this into its answer verbatim, nothing leaks: not the
raw error code, not the file path. The single passage point is
`ToolOutcome::failed`.

**The chip text is produced by the tool, not the model.** Every step visible on
screen is an event that really happened in the code; the model cannot hallucinate
a visible step.

**The bypass channel has exactly one wire format.** The reference suffix going to
the model is produced only with `tacet_core::source_ref_suffix`:
`\n(full content ready, source_ref=document#1)`. When two separate call sites
write their own `format!`, the model learns two formats.

**The single gate for writing a document is the free function `write_document`.**
The trait only defines `write_raw`; folder preparation, unique naming and the
0600 permission stamp stand OUTSIDE the trait, meaning a new engine cannot
invalidate them.

> **0600 ON UNIX ONLY.** On Windows the stamp is DELIBERATELY not applied: the
> `set_permissions` there only flips the "read-only" flag, it does not restrict
> access — applying it would produce the illusion of "protected" without
> providing any privacy. The real equivalent is narrowing the ACL, and that wants
> a `windows-sys` dependency. On Windows the protection rests on the file's
> LOCATION (the configuration directory under the user profile) and THIS WAS NOT
> MEASURED ON THIS MACHINE. The same note is also written inside
> `create_document`, `write_code` and `tacet-memory/store.rs`.

## The four gates (ToolExecutor)

A tool call passes through these in order; none of them works by text matching,
all of them are structural:

1. **NAME** — if it is not in the catalog it does not run. The model cannot
   execute a signature it invented.
2. **SCHEMA** — the tool does not see it before `ArgSchema::validate` passes. The
   grammar already enforces this, but the grammar can be disabled; the gate being
   two-layered is deliberate.
3. **APPROVAL** — in a tainted session a call that would take data to the outside
   world does not pass without a user decision.
4. **CANCELLATION** — if the user cancelled the turn, the tool never starts.

## Constrained generation: what is enforced, what is not

`CallConstraint` masks the logits at every generation step (sampling runs AFTER
it, so no sampling strategy can punch through the constraint).

- **ENFORCED:** once the model has written `tool_name(`, the arguments MUST
  conform to the grammar of that tool's schema. Invalid JSON, a field not in the
  schema, a value outside the enum set, a number out of range, a missing required
  field — none of them can be generated. (The `tacet-grammar/src/call.rs` tests
  prove this.)
- **NOT ENFORCED:** the model CHOOSING to call a tool. Because a plain answer is a
  legitimate output, free text must stay open at the start; otherwise even "hello"
  would get a tool call. An invented tool name counts as "plain text" to the
  grammar and is rejected at gate 1 (not in the catalog) — that is the second line
  of defense.

## Commands

```sh
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p tacet-cli -- eval             # 21 cases, exit code depends on the threshold
cargo run -p tacet-cli -- eval --json      # for CI
cargo run -p tacet-cli -- tools --schema   # the schema as it appears in the prompt, verbatim
cargo run -p tacet-cli -- grammar --tool create_document --try-input '{"format":"excel"}'
cargo run -p tacet-cli -- chat --message "125 times 8" --script 'calculate({"expression":"125*8"})'
cargo run -p tacet-cli -- package list     # SKILL packages (embedded + user)
cargo run -p tacet-cli -- package list --json
cargo run -p tacet-cli -- models list      # MODEL packages (weight files)
cargo run -p tacet-cli -- models download qwen3-4b  # from the source in packages.json
```

`package` and `models` are SEPARATE commands because they are separate things:
`package` is about kilobytes of skill text (the configuration directory), `models`
about gigabytes of weights (the model roots). `models download` reads from the
user's own `~/.tacet/packages.json` — there is no embedded mirror — and before
downloading it shows what it will download and asks `[y/N]` (`--no-approval` for
scripts).

`eval --threshold` is a FRACTION (0.0–1.0), not a percentage. Asking for
`--threshold 90` means asking for 9000% and is rejected; when running with a real
engine pass something like `--threshold 0.8`.

### The tool selection measurement

The `eval` above measures Tacet's LOGIC and runs with `FakeEngine`. There is a
separate set that measures the model's own CHOICE; it is only meaningful with a
real model:

```sh
cargo build --release -p tacet-cli --features metal
./target/release/tacet eval --tool-selection              # ~30 cases, takes minutes
./target/release/tacet eval --tool-selection --json       # for diffing between runs
./target/release/tacet eval --tool-selection --only time  # a single group, diagnostic
./target/release/tacet eval --tool-selection --model qwen3-8b
```

The output gives THREE numbers and they are DELIBERATELY separate:

| number | what it measures |
| --- | --- |
| `TOOL HIT RATE` | was the correct tool called |
| `IRRELEVANCE` | was no tool called where none should have been — **it MUST NOT drop** |
| `PER STEP` | step-by-step accuracy in multi-turn cases |

There is NO single "success rate", because the two numbers are each other's
price: every change that makes tools get called more aggressively raises the hit
rate and in the same move starts calling tools for a greeting. That is why the
exit code is tied to IRRELEVANCE — the tool hit rate varies with model capacity,
while irrelevance is a limit that must not degrade.

Because the set requires a real model it does not run in CI; instead the ROUTER
layer of the same cases
(`the_expected_tool_does_not_drop_out_of_the_routers_budget`) runs in seconds
under `cargo test`. The budget is 8: if the expected tool drops out of that
budget, the model NEVER sees it in the prompt and the measurement lies by saying
"the model chose wrong".

Catalog size is NOT A SINGLE NUMBER, it depends on two conditions (measured, see
STATUS.md): 11 if the web search addon is installed, 9 if not; two more tools drop
if the code shield measurement does not pass. The eval arm is PROTECTED from this
fluctuation: in measurement mode (when `fixed_epoch` is given) the addon gate is
never consulted, otherwise the same eval set would run with two different catalogs
on two machines and the scores could not be compared.

## Chatting with a real model (the terminal shell)

The default build does NOT pull the candle tree at all. For real inference build
with the `candle` (CPU) or `metal` (Apple GPU) feature. If the model files are
placed under `~/models/<name>/`, or `TACET_MODEL`/`TACET_TOKENIZER` are set,
`--engine auto` (the default) picks the REAL model on its own; otherwise it falls
back to FakeEngine with a meaningful message.

CAREFUL — the DEFAULT for `<name>` is `qwen3-4b` (`DEFAULT_MODEL`, main.rs);
another weight needs `--model <name>`. This line said `qwen2.5-3b` for a while and
THAT WAS WRONG: the model of a user who filled that folder was not found on its
own. `tacet models list` prints what is found in which roots.

```sh
# Install with one command — `tacet` goes on PATH (~/.cargo/bin):
cargo install --path crates/tacet-cli --features metal   # Apple GPU
# or CPU:
cargo install --path crates/tacet-cli --features candle

# The model is a LOCAL FILE; the engine never downloads on its own:
#   ~/models/qwen3-4b/<weight>.gguf + tokenizer.json   (the default name)
# or explicitly:
export TACET_MODEL=/path/model.gguf
export TACET_TOKENIZER=/path/tokenizer.json
# To pull it from your own source instead of filling the folder by hand:
#   tacet models list             # what exists, which is selected, where the roots are
#   tacet models download <name>  # from the address in ~/.tacet/packages.json, with approval

tacet                        # NO SUBCOMMAND: the interactive shell directly
tacet chat                   # the same thing, the explicit form (for scripts)
tacet chat --message "..."   # a single message (diagnostic; the approval gate is SilentDeny)
tacet tools                  # the catalog (9 tools without addons, 11 with web search)
tacet package list           # the installed SKILL packages
tacet models list             # the installed MODEL packages (+ which one is selected)
tacet models download <name>  # from your own packages.json, through the approval gate

tacet addon list             # the installed addons (+ is the gate open)
tacet addon install web-search  # flagless: IT ASKS (local SearXNG / enter an address)
tacet addon try web-search   # send a real query to the server, see whether it works
tacet addon close|open       # turn search off/on WITHOUT LOSING the address
tacet addon remove web-search
```

In the interactive shell: a spinning indicator + elapsed time, live tool chips,
and a LOCKED input during generation — keys pressed while the model writes are not
spilled onto the screen, `ctrl-c` stops the answer (it does not close the
program). If there is no tty (piped input, CI) none of this is done and the shell
falls back to plain text.

Slash commands: `/help`, `/tools`, `/grammar <tool>`, `/eval`, `/memory`,
`/history`, `/model`, `/clear`, `/quit`. Both files are LOCAL. The device defaults
to the CPU; for Metal the `metal` feature must be enabled — it DOES NOT silently
fall back to the CPU.

## The layers WIRED UP in the second round

Everything that was "out of scope" in the first round is now ON THE PRODUCTION
PATH:

- **Web search** (`web_search`/`web_fetch`) — the network is ONLY in `tacet-web`;
  in a tainted session it passes through the approval gate (`EXTERNAL_TOOLS`).
  OFF BY DEFAULT: these two tools appear in the catalog only if an addon has been
  installed with `tacet addon install web-search` AND is open; the gate is read
  inside `production_catalog`, it is not left to the caller. There is NO search
  address embedded in the code — the address order is `TACET_SEARXNG` > the addon
  record > none (an explicit error).
- **MCP** — if there is an `mcp.json` in the configuration directory the remote
  tools are in the catalog; all of them are external tools.
- **Memory** — `memory.json` in the configuration directory (0600 on Unix, see the
  platform note above); a note matching the message is injected into the prompt
  with the `<memory>` fence + the `remember` tool is in the catalog.
- **Skills** — the SINGLE skill matching the message is added to the prompt with
  the `<guidance>` fence, with a 700-character limit, selected on every turn with
  `SkillStore::matching`.
- **New tools** — `edit_document`, `find_file`, `run_code`.
- **Router** — the 8-tool cap IS NOW BINDING: in the 11-tool catalog with addons
  the last THREE, and in the 9-tool one without addons the last ONE tool drops out
  on a message with no hint (`the_catalog_is_larger_than_the_router_budget`
  measures both cases).

For the detailed "what is WIRED, what is missing" dump, see `STATUS.md`.
