# Tacet

**A private AI assistant that lives in your terminal. No cloud, no account, no telemetry.**

*tacet* — in musical notation: *this instrument is silent in this passage*. Here, the silent instrument is the network.

```
$ tacet
Tacet.

> how many days until 14 March?

[time] diff · 229 days
229 days.

> put that in a spreadsheet called countdown

[create_document] countdown.xlsx · written
Done — countdown.xlsx, one row, with a live =DATEDIF() formula.
```

---

## Why this exists

Most "local AI" tools are a thin shell around a model file. The hard part isn't running the model — it's everything around it: deciding when a tool is needed, forcing the model to produce a *valid* call, executing it safely, and keeping bulk data out of a 4096-token context window.

Tacet is that layer, written to be read. Every non-obvious decision has a comment explaining **why**, and several of them record a measurement that proved the obvious approach wrong.

## What makes it different

**Invalid tool calls are impossible, not unlikely.** Once the model emits `calculate(`, a pushdown automaton masks the logits at every step. Malformed JSON, a field that isn't in the schema, an out-of-range number, a missing required key — none of them can be *generated*. Not validated after the fact: unrepresentable. Sampling runs after masking, so no sampling strategy can escape it.

**The network monopoly is checkable by eye.** Exactly two crates may open a socket, and the HTTP dependency appears in exactly those two manifests. You do not have to trust a privacy claim you cannot audit — `grep ureq crates/*/Cargo.toml` is the audit.

**Nothing leaves the device by default.** Web search is an *addon* you install deliberately, pointed at a SearXNG instance you run. Until you do, the search tools are not merely disabled — they are absent from the catalog the model can see.

**Four gates on every tool call**, none of which work by matching text:

| Gate | Rejects |
|---|---|
| Name | a tool that isn't in the catalog — the model cannot invent a callable |
| Schema | arguments that don't validate, even if the grammar was bypassed |
| Approval | outbound data in a session that has touched personal data |
| Cancel | anything, the moment you interrupt the turn |

**Bulk data never enters the model.** A tool that reads a 40 000-row spreadsheet puts the data in a store and hands the model a short summary plus a reference. The next tool that needs it fetches it by reference. The context window is a budget, not a bottleneck.

**Almost no dependencies.** OOXML (`.xlsx`, `.docx`) generation, zip, deflate and CRC32 are written by hand. So is the MCP client — JSON-RPC 2.0 over Streamable HTTP with SSE, in ~430 lines with a single `use std`. The full dependency list is `serde`, `serde_json`, `thiserror`, `clap`, `crossterm`, `ureq`, plus `candle` behind an off-by-default feature. Adding to that list is an architectural decision documented at the top of the file, not a convenience.

## Install

```bash
cargo install --git https://github.com/farukciftler/tacet-cli tacet-cli --features metal   # Apple GPU
cargo install --git https://github.com/farukciftler/tacet-cli tacet-cli --features candle  # CPU
```

Or grab a prebuilt binary from [Releases](../../releases) — macOS (Apple Silicon
and Intel), Linux and Windows.

A crates.io release is prepared; until it is published, `cargo install tacet-cli`
does not resolve.

Check for a newer version at any time — this is the only command that talks to GitHub, and only when you run it:

```bash
tacet update            # tells you what's available
tacet update --install  # downloads and replaces the binary, with your confirmation
```

## Quickstart

Tacet needs a model. It never downloads one behind your back:

```bash
tacet model list                 # what's on disk, which roots were searched
tacet model download qwen3-4b    # from your own packages.json, https only, sha256 verified
```

Point it at weights you already have instead:

```bash
export TACET_MODEL=/path/to/model.gguf
export TACET_TOKENIZER=/path/to/tokenizer.json
```

Then:

```bash
tacet                                  # interactive shell
tacet chat --message "what's 125 * 8"  # one shot
tacet tools --schema                   # the exact schema the model sees
tacet eval                             # 21-case behavioural suite
```

## Tools

`calculate` · `time` · `read_document` · `create_document` · `edit_document` · `find_file` · `run_code` · `write_code` · `remember` · `web_search` · `web_fetch` · `mcp`

Documents are real OOXML — an `.xlsx` produced by Tacet contains a working `=SUM()`, not a pre-computed number.

`run_code` executes behind a sandbox that blocks the network. On macOS that is `sandbox-exec`; on Linux, `bwrap`. **If no sandbox is available, the tool is removed from the catalog rather than run unprotected** — the model is never handed an unguarded interpreter.

## Addons

```bash
tacet addon install web-search
```

Choose a local SearXNG (started with Docker under `~/.tacet/`, after showing you the image and asking) or enter the address of an instance you already run. Until an addon is installed and enabled, its tools do not exist as far as the model is concerned.

## Skills

A skill is a Markdown file with trigger phrases and a short piece of guidance. When a message matches, exactly one skill is fenced into that single turn's prompt — never into the system instruction. That distinction was measured: embedding guidance in the system instruction pushed a small model toward *explaining* the task instead of *calling the tool*.

```markdown
---
name: calc
triggers: [how much, how many, times, percent]
---
Do arithmetic with the `calculate` tool. Never compute it yourself.
```

Drop it in `~/.tacet/skills/`.

## MCP

Connect servers you run yourself in `~/.tacet/mcp.json`. Their tools join the catalog and pass through the same four gates as built-in ones — a remote tool gets no privileges a local tool doesn't have.

## Architecture

```
tacet-cli ──────────► terminal shell; drives the turn loop
   │
   ├── tacet-eval ──► cases, scoring, reports
   │
   ├── tacet-grammar► ArgSchema → constrained-generation grammar (PDA + token mask)
   │        │
   │        ▼
   ├── tacet-engine ► contracts: Prompt, ModelProvider, Constrainer, TokenCounter
   │                  MockEngine (default) / CandleEngine (--features candle)
   │
   ├── tacet-tools ─► concrete tools + ToolExecutor + Router
   │        └── tacet-zip ──► hand-written zip/deflate/crc32 → OOXML
   │
   └── tacet-core ──► the CONTRACT: Tool, ArgSchema, ToolOutcome, DataStore, Catalog
```

Arrows are dependency direction. `tacet-core` depends on nothing, so the contract never bends under pressure from an implementation. `tacet-engine` deliberately does not know `tacet-grammar`: the `Constrainer` contract lives in the engine, its implementation in the grammar, so a run without constraints doesn't compile grammar code it never uses.

## Platform support — honestly

| Platform | State |
|---|---|
| macOS (arm64) | **Verified.** Full suite runs here: build, clippy `-D warnings`, tests, eval. |
| Linux | Compiles in CI. The `bwrap` sandbox path has *not* been exercised against a real `bwrap`. |
| Windows | Compiles in CI. No runtime measurement at all: the timezone path, model roots and file-permission behaviour are unverified. |

Where a guarantee holds on one platform and not another, the code says so at the point where it matters. `0600` permission stamping, for example, is deliberately not applied on Windows — there `set_permissions` only flips a read-only flag, which would produce the *appearance* of protection without the substance.

## Development

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p tacet-cli -- eval
```

The eval suite scores *tool usage*, not answer correctness — a case can pass with wrong arithmetic if the model called the right tool with the right arguments. That is intentional (the arithmetic is the tool's job, and the tool has its own tests), but it means eval is not a substitute for reading the output.

## License

MIT. See [LICENSE](LICENSE).
