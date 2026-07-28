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

Most "local AI" tools are a thin shell around a model file. The hard part isn't running the model — it's everything around it: deciding when a tool is needed, forcing the model to produce a *valid* call, executing it safely, and keeping bulk data out of a context window that is measured in thousands of tokens, not millions.

Tacet is that layer, written to be read. Every non-obvious decision has a comment explaining **why**, and several of them record a measurement that proved the obvious approach wrong.

## What makes it different

**Invalid tool calls are impossible, not unlikely.** Once the model emits `calculate(`, a pushdown automaton masks the logits at every step. Malformed JSON, a field that isn't in the schema, an out-of-range number, a missing required key — none of them can be *generated*. Not validated after the fact: unrepresentable. Sampling runs after masking, so no sampling strategy can escape it.

**The network monopoly is checkable by eye.** Exactly two crates may open a socket, and the HTTP dependency appears in exactly those two manifests. You do not have to trust a privacy claim you cannot audit — `grep ureq crates/*/Cargo.toml` is the audit. (One honest asterisk: if you install the `shell` addon and put `curl` on its allow-list, you have handed a program the network. That is why `shell` sits behind the approval gate — see [Addons](#addons).)

**Nothing leaves the device by default.** Everything with outside reach is an *addon* you install deliberately: web search against your own SearXNG, HTTP against hosts you name, a shell against programs you list. Until you install one, its tools are not merely disabled — they are **absent from the catalog the model is shown**, so it cannot call them or claim it did.

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
cargo install tacet-cli --features metal   # Apple GPU
cargo install tacet-cli --features candle  # CPU
cargo install tacet-cli                    # no inference; still runs eval, tools and the addon flow
```

⚠️ **`cargo install` does not remember `--features`.** Upgrade without the flag
and you get a binary that cannot run a model — it still starts, still answers,
and the answers are canned. `tacet --version` prints the build it actually is:

```bash
tacet --version        # tacet 0.1.11 (metal)  ← the part in brackets is the engine
```

Or grab a prebuilt binary from [Releases](../../releases) — macOS (Apple Silicon
and Intel), Linux and Windows.

Check for a newer version at any time — this is the only command that talks to GitHub, and only when you run it:

```bash
tacet update            # tells you what's available
tacet update --install  # downloads and replaces the binary, with your confirmation
```

Nothing checks on its own until you say so. A few turns into your first session
the shell asks once whether it may look for a new version daily, and writes the
answer down:

```bash
tacet config set update.check on    # one request a day, at the end of a session
tacet config set update.check off   # never
```

The check is throttled to once every 24 hours, prints a single line when a newer
version exists, stays silent when it fails, and never runs when output is piped.

## Quickstart

Tacet needs a model. It never downloads one behind your back:

```bash
tacet models list                 # what's on disk, which roots were searched
tacet models download qwen3-4b    # ~2 GB, https only, sha256 checked before it lands
```

Two models are in the built-in catalog — `qwen3-4b` (the default) and
`qwen2.5-3b` (smaller, for machines with less to spare). Every download is over
HTTPS to an address printed on screen first, and the file is rejected unless its
sha256 matches the one compiled into the binary. Nothing is fetched until you
ask; add your own entries in `~/.tacet/packages.json`.

**Already have weights?** Point at them and skip the download entirely:

```bash
export TACET_MODEL=/path/to/model.gguf
```

**No separate tokenizer file needed.** Tacet reads the tokenizer *out of the
GGUF* — vocabulary, merges and token types — so a file from Ollama or LM Studio
works as it is. `TACET_TOKENIZER=/path/to/tokenizer.json` still overrides, for a
GGUF that carries no tokenizer inside it.

Then:

```bash
tacet                                  # interactive shell
tacet chat --message "what's 125 * 8"  # one shot
tacet tools --schema                   # the exact schema the model sees
tacet eval                             # 21-case behavioural suite
```

## Tools

Out of the box, with nothing installed:

`calculate` · `time` · `calendar` · `read_document` · `create_document` · `edit_document` · `find_file` · `run_code` · `write_code` · `git` · `remember`

Documents are real OOXML — an `.xlsx` produced by Tacet contains a working `=SUM()`, not a pre-computed number.

`git` is **read-only**: status, log, diff, show. It reads a diff so the model can write a commit message; it does not commit, push, or change a branch.

`run_code` executes behind a sandbox that blocks the network. On macOS that is `sandbox-exec`; on Linux, `bwrap`. **If no sandbox is available, the tool is removed from the catalog rather than run unprotected** — the model is never handed an unguarded interpreter.

## Addons

Everything with reach beyond the working directory is an addon, and every addon is **off until you install it**. Not disabled — absent. A closed addon's tools are not in the catalog the model is shown, so it cannot call them, mention them, or fail at them.

```bash
tacet addon list                 # what exists, what is installed
tacet addon install shell        # asks before it writes anything
tacet addon close shell          # keep the config, take the tool away
```

| Addon | What it opens | Where the line is |
|---|---|---|
| `web-search` | `web_search`, `web_fetch` | your own SearXNG — local under Docker, or an address you already run |
| `shell` | `shell` | only the programs you list, and **no shell interpretation** |
| `workspace` | *(no new tool)* | named directories the file tools may also reach |
| `http` | `http` | only the exact hosts you list, HTTPS only, no redirects |
| `db` | `db` | read-only SQLite, over the `sqlite3` binary already on your machine |
| `clipboard` | `clipboard` | reads and writes the system clipboard |

Two things worth knowing before you install `shell`:

**There is no shell.** The command runs as `program` + a list of arguments, never through `sh -c`, so `; rm -rf /` arrives as an *argument* — nothing parses it. And the allow-list is not checked after the fact: it **is** the argument's schema, so the constrained decoder cannot generate a program name outside it.

**Allowing a program allows everything that program can do.** `curl` is network access; `git` can push. So `shell` sits behind the same approval question as `web_search` and `http`: once a turn has touched personal data, every call that could carry it off the machine stops and asks you first.

Third-party tools do not plug in here — they plug in through [MCP](#mcp).

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
   │        ├── tacet-zip ──► hand-written zip/deflate/crc32 → OOXML
   │        └── tacet-web ──► THE ONLY CRATE WITH A SOCKET (with tacet-mcp):
   │                          search, page fetch, the SSRF gate, the addon registry
   │
   ├── tacet-skills ► trigger-matched guidance, fenced into one turn
   ├── tacet-memory ► notes on disk, injected only when relevant
   │
   └── tacet-kernel► the CONTRACT: Tool, ArgSchema, ToolOutcome, DataStore, Catalog
```

Arrows are dependency direction. `tacet-kernel` depends on nothing, so the contract never bends under pressure from an implementation. `tacet-engine` deliberately does not know `tacet-grammar`: the `Constrainer` contract lives in the engine, its implementation in the grammar, so a run without constraints doesn't compile grammar code it never uses.

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

## Contributing

Small changes welcome; the shape of the project is written down in
[CONTRIBUTING.md](CONTRIBUTING.md) and takes five minutes to read.

The most useful thing anyone can do right now: **run it on Linux or Windows and
say what broke.** CI compiles and tests there, but no human has used the
interactive shell on either — see the platform table above.

## Star History

<a href="https://www.star-history.com/#farukciftler/tacet-cli&Date">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=farukciftler/tacet-cli&type=Date&theme=dark" />
    <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=farukciftler/tacet-cli&type=Date" />
    <img alt="Star history chart for farukciftler/tacet-cli" src="https://api.star-history.com/svg?repos=farukciftler/tacet-cli&type=Date" />
  </picture>
</a>

## License

MIT. See [LICENSE](LICENSE).
