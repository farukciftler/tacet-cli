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

**A call that has started cannot be finished invalidly.** Once the model emits `calculate(`, a pushdown automaton masks the logits at every step. Malformed JSON, a field that isn't in the schema, an out-of-range number, a missing required key — none of them can be *generated*. Not validated after the fact: unrepresentable. Sampling runs after masking, so no sampling strategy can escape it.

The sentence used to read "invalid tool calls are impossible", and measuring it showed that was wider than the truth: **the grammar arms after `name(`, so it says nothing about a call that never starts that way.** Running a real model over 115 cases, seven of the twenty-two failures were the right tool with the right arguments written in a shape nobody taught it — ` ```tool read_document"path=report.md" ` and `<tool_call> read_document (path: "x") </tool_call>`. Those are recovered now, by a layer that will only look behind a marker no prose contains; the underlying gap is real and is written down rather than papered over.

**And a valid call has to end.** That is a second property, weaker than the first and until recently not held at all: a valid *prefix* could wander forever. A model wrote a complete, correct `calendar(…)` call and then emitted whitespace for twelve minutes, because whitespace was legal at a structural position and legal again immediately. Unrepresentable-invalid and always-terminating are different claims — the grammar now bounds consecutive whitespace and the length of a field the schema leaves open, and the engine caps a constrained generation at 2048 tokens, measured against a largest-observed legitimate call of 1523.

**The network monopoly is checkable by eye.** Exactly two crates may open a socket, and the HTTP dependency appears in exactly those two manifests. You do not have to trust a privacy claim you cannot audit — `grep -v '^\s*#' crates/*/Cargo.toml | grep ureq` is the audit, and `cargo test -p tacet-cli --test network_monopoly` is the same audit as a failing build: it asserts that exactly those two manifests declare an HTTP client, that no other client was swapped in under a different name, and that nobody reached a socket through `std::net` instead — scanning every `.rs` file under `crates/*/{src,tests,examples,benches}`, not just the library code. (One honest asterisk: if you install the `shell` addon and put `curl` on its allow-list, you have handed a program the network. That is why `shell` sits behind the approval gate — see [Addons](#addons).)

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

One honest note on what "the full list" means: those are the DIRECT dependencies, and they are the ones a reader can audit. Transitively, `ureq` brings `rustls`, which brings `ring` — and `ring` compiles C. Nothing here calls it and it changes no claim about what the program does, but it is what sets the build prerequisite on Windows: not the MSVC linker it looks like, but a C compiler, however you get one. Measured, with the three routes and their sizes, in [CONTRIBUTING](CONTRIBUTING.md).

## Install

```bash
cargo install tacet-cli --features metal   # Apple GPU
cargo install tacet-cli --features candle  # CPU
cargo install tacet-cli                    # no inference; still runs eval, tools and the addon flow
```

⚠️ **`cargo install` does not remember `--features`.** Upgrade without the flag
and you get a binary that cannot run a model. It **refuses to answer** rather
than making something up: it names the missing feature, tells you how to
reinstall, and exits non-zero. It used to fall back to a scripted engine and keep
answering — which meant the failure looked like a working product giving bad
replies, and nobody reported it. `tacet --version` prints the build it actually is:

```bash
tacet --version        # tacet 0.1.26 (metal)  ← the part in brackets is the engine
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

Two models are in the built-in catalog — `qwen3-4b` (the default:
Qwen3-4B-Instruct-2507, Q4_K_M) and `qwen2.5-3b` (smaller, for machines with
less to spare). The name is worth reading closely: `Qwen3-4B` and
`Qwen3-4B-Instruct-2507` are two different models, and the catalog pins the
second — the one every number on this page was measured on. Every download is over
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
tacet eval                             # 78-case behavioural suite
```

## Tools

Out of the box, with nothing installed:

`calculate` · `time` · `calendar` · `read_document` · `create_document` · `edit_document` · `find_file` · `run_code` · `write_code` · `git` · `remember` · `archive` · `checksum`

Documents are real OOXML — an `.xlsx` produced by Tacet contains a working `=SUM()`, not a pre-computed number.

`git` is **read-only**: status, log, diff, show. It reads a diff so the model can write a commit message; it does not commit, push, or change a branch.

`archive` lists or extracts a `.zip` with the workspace's own inflate. It refuses the whole archive — never one entry — when a name would escape the destination, an entry is a symlink, the declared sizes cross the caps, or a name repeats: four gates that run on the central directory, so both actions apply them. The CRC and the declared-vs-actual size are proven on **extract** only, because listing decodes nothing — which is why a listing labels its numbers "declared" rather than reporting them as sizes. Extraction always goes into a **new** directory whose name is rotated until it is free, so there is no argument through which it could overwrite something.

`checksum` is SHA-256 over a file: the digest, or a comparison against a published one, or against a second file. A mismatch comes back as an answer, not an error.

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
| `db` | `db`, and `db_write` only if you list a file | read-only SQLite by default, over the `sqlite3` binary already on your machine |
| `clipboard` | `clipboard` | reads and writes the system clipboard |

Two things worth knowing before you install `shell`:

**There is no shell.** The command runs as `program` + a list of arguments, never through `sh -c`, so `; rm -rf /` arrives as an *argument* — nothing parses it. And the allow-list is not checked after the fact: it **is** the argument's schema, so the constrained decoder cannot generate a program name outside it.

**Allowing a program allows everything that program can do.** `curl` is network access; `git` can push. So `shell` sits behind the same approval question as `web_search` and `http`: once a turn has touched personal data, every call that could carry it off the machine stops and asks you first.

**Writing to a database is a second tool, not a flag.** `db` is read-only and stays read-only: its lock is `sqlite3 -readonly`, measured on your own binary before the tool is built. If you want a database changed, you name the file — `data/app.db`, relative to the project — while installing the `db` addon, and a *separate* `db_write` tool appears. Name nothing and it does not exist, which is the only gate that holds: the SQL is free text, so a model that has the tool can always spell `DROP TABLE`, and the answer is for the tool not to be there.

Every `db_write` call is measured before it happens. The statement runs first against a **copy** of the file, and what you are shown is the difference that copy actually took: objects created, dropped or redefined, row counts moved, journal mode changed. Then it asks. Nothing is written until you say yes, the copy that was measured is left beside the database as `<name>.tacet-backup`, and the question is asked again for the next statement — a "no" is about one statement, not about the session.

Two things it does **not** promise. It is not one statement: `sqlite3` re-parses the string it is given, so `SELECT 1; DROP TABLE t` runs both halves — measured, and the reason the effect is shown rather than the statement filtered. And allowing a file allows everything SQL can do to that file, `DROP` and a `WHERE`-less `UPDATE` included. What holds is the file boundary: `-safe` refuses `ATTACH`, `VACUUM INTO` and `writefile()`, all three measured at startup, so the statement cannot reach a second file.

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
   ├── tacet-engine ► contracts: Prompt, EngineProvider, Constrainer, TokenCounter
   │                  FakeEngine (default) / CandleEngine (--features candle)
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

## What it scores — honestly

Most of this repository's green numbers come from a **mock engine**: they measure
Tacet's own logic, deterministically, in milliseconds. That is the right tool for
CI and the wrong one for the question people actually ask, which is whether a 4B
model on your laptop picks the right tool. So here is that number, produced on
real weights and checked into `crates/tacet-eval/baselines/` with the model's
fingerprint, so the next run can be compared against it rather than against a
memory of how things used to go:

```bash
tacet eval --tool-selection --model qwen3-4b     # both languages, ~40 min
tacet eval --tool-selection --model qwen3-4b --turkish   # Turkish only
```

| | |
|---|---|
| tool selection | **133/160** · 83.1% |
| irrelevance gate | **24/24** · 100% |
| step chain | 162/190 · 85.3% |
| answer quality | 41/47 · 87.2% |

Qwen3-4B-Instruct-2507 Q4_K_M on Metal, 184 cases in both languages, 44 min —
the weights `tacet models download qwen3-4b` fetches, pinned by digest, with the
fingerprint recorded in the baseline.

**Six minutes on a GPU — and the model matters more than the card.** The same
184 cases on a rented RTX 3090: 6.2 minutes with these weights, and 53.7 minutes
with `Qwen/Qwen3-4B`, the *other* model of the same name and size, which spends a
median of 237 generated tokens per turn against 19. Before reaching for a bigger
card, check which file is loaded. And when you do reach for one: running several
copies of the suite side by side wins nothing, because one stream already
saturates the card (measured — two streams give ~28 tok/s each where one gives
~55). Batching several sequences into a single forward pass is the thing that
scales, and `cargo run --release --example batch_decode --features candle,cuda`
measures where it stops: 124 tok/s at batch 1, 504 at batch 32 on that card.

**Turkish scores higher than English** — 61/69 against 96/115 — which is the
opposite of what the effort spent on Turkish defects would suggest, and it is only
visible because the two now run together. The routing eval always measured both;
this one measured one language at a time and never both, so the expensive
measurement was looking at the easier half and calling it the score.

**The irrelevance gate is the one to read first.** Twenty-four messages that must
*not* reach a tool, and none of them did. A local assistant that reaches for a
tool on "thanks, that's all" is worse than no assistant, and that is the failure
this number rules out.

**The failures, because a score without a diagnosis is a number to be proud of
rather than one to act on.** Twenty-seven of the 184 cases failed, and they are
not scattered. Counted off the baseline itself:

| | |
|---|---|
| called the wrong tool — 10 | 5× `edit_document` → `read_document`, 3× `web_fetch` → `web_search`, and one each of `read_document` → `find_file` and `run_code` → `calculate` |
| answered in prose, no call — 8 | including **two that are ours, not the model's**: one turn died on `engine error: constraint rejected the token: 32`, another on `generation was cut off halfway` |
| wrote a call in a syntax nobody taught it — 5 | three of them the same glued shape, which Turkish produced and English never did |
| declined a tool it *had* — 5 | `web_search`, `git`, `calendar`, `remember` |

```
find_filepattern: "bütçe"          ← the tool name glued to its first argument
```

The two largest are worth naming because neither is what you would guess. Five
cases read the document and then said *"no tool is available to edit or update
the content"* — with `edit_document` sitting at rank 1 in that turn's own prompt,
confirmed with `tacet why`. And three handed a URL called `web_search` instead of
`web_fetch`, which is the tool description's own fault: it said *"use only when a
search result summary is not enough"*, which is exactly the wrong instruction for
a message that already carries the address.

**Both are fixed, and this is the first time the instrument has called an effect
real.** The descriptions now say what those tools can do, the router puts a tool
that requires a `url` first when the message carries one, and a call fenced as
` ```json ` is recovered. Measured by re-running the whole suite on one rented
RTX 3090 against a same-weights, same-host run made before the change:

```
paired on 184 cases
  before   146/184  (79.3%)
  after    155/184  (84.2%)
  fixed 12   broke 3
  delta +4.9 points   95% CI [+1.1, +9.2]
  sign test p = 0.0352
  verdict: REAL at 95%.
```

Nine of the twelve are the cases named above. The table further up is the **Metal**
baseline and has not been re-measured since, so it still reads pre-fix — the two
numbers are from different machines and `--compare` refuses to pair across
either a different model or a different catalog, which is why this one is quoted
on its own terms rather than folded into the table.

**A refusal is usually ours.** An earlier run had six of them, and the cause was
the system prompt: it opened with *"an assistant that runs entirely on the
device. Data never leaves the device"* — a true statement about the architecture
that a small model reads as a statement about its own capabilities, and then
declines to use `web_search`, which `tacet why` confirmed was sitting at rank 2
in its own prompt. Five survive in the numbers above, and the same test applies
to each: run `tacet why` on the message before blaming the model.

**What happened when we fixed that one is the most useful thing on this page.**
This was the earlier, English-only suite — 115 cases, not the 184 above, so the
numbers below do not belong to the table. The raw score moved +3, and
`eval --compare` ran a sign test and refused to call it:

```
fixed 8   broke 5
delta +2.6 points   95% CI [-3.5, +8.7]
sign test p = 0.5811
verdict: NOT DISTINGUISHABLE from no change at 95%.
         this instrument needs 230 paired cases to call a 2.6-point effect;
         this suite has 115.
```

Two runs of anything differ by a case or two for no reason. If you take one number
away from this section, take that one — and note that the instrument volunteered
its own resolution limit rather than letting a +3 be reported as progress.

## Benchmarks — measure YOUR tools

The suite above is compiled in and measures this project. A **benchmark** is the
other direction: a file you write, run against the tools your machine actually
has — your MCP servers, your addons, your language — answering "does this
assistant call *my* tools".

```bash
tacet bench check my-tools.json               # no model runs; costs nothing
tacet bench run my-tools.json --model qwen3-4b
```

A file is JSON and the whole format fits on a screen:

```json
{
  "name": "our-github-mcp",
  "language": "en",
  "requires": ["gh_search_issues", "web_fetch"],
  "cases": [
    { "name": "open-issues-by-label", "category": "tool",
      "steps": [{ "message": "which issues are labelled regression?",
                  "expect": "gh_search_issues",
                  "evidence": ["#412"],
                  "forbidden": ["web_search"] }] },
    { "name": "thanks", "category": "irrelevance",
      "steps": [{ "message": "great, thanks!" }] }
  ]
}
```

`benchmarks/example.json` is a worked one, and `benchmarks/en/` holds **314
English cases** across eight groups — arithmetic and time, documents, files and
archives, code and git, web, memory and calendar, 45 irrelevance cases, and 29
multi-step chains. They were drafted by a multi-agent pass and then cut down by
the gate below; what survived is what a fresh install can actually be asked.

Three things about the format are deliberate:

**`requires` is not paperwork.** It is what makes the runner *stop* when the
machine lacks a tool, instead of scoring every case that needs it as a model
failure and publishing that as a result — the same defect `eval --compare` was
taught to refuse when a Linux run paired against a macOS baseline read nineteen
absent-tool failures as a regression.

**`bench check` runs before any model does**, and it asks the question nobody
writes by hand: the router shows the model nine tools, so *would the expected
tool even be among them?* A case whose tool never reaches the prompt measures the
router and reports the model, every time it is run, forever. Checking it is free.
It earned its keep immediately. Over the first 321 drafted questions it found
**22 cases whose expected tool the router never showed** — and all but seven were
the ROUTER's fault, not the question's: seven unmistakable web questions ("is
there a train strike going on in France?", "which stable Rust version is the
newest one right now", "how bad is the air quality in Delhi") scored zero on
every profile, so `web_search` was not among the nine and they would have been
recorded as model failures forever. Those triggers are in the router now, and the
questions stayed. Seven cases were deleted instead, because their signal is one
the router structurally cannot read — an adversarial negation ("I don't want
another file lying around"), or a follow-up whose subject is only in the previous
turn ("add coffee to that list too"), which a stateless router cannot resolve.

`--portable` checks against the default catalog rather than yours. It matters:
the router shows nine tools of however many exist, so a machine with 29 MCP tools
attached answers a different question, and a benchmark that only passes on its
author's laptop is not a benchmark.

**There is no regex, no script and no expected answer text.** `evidence` is a
plain substring. Scoring prose against prose needs a judge, a judge is a second
model, and a second model is a second thing to be wrong.

The score is out of 100 with the four axes printed beside it, and the weights are
in the source rather than in someone's head — **irrelevance 0.40, tool 0.30, step
0.20, answer 0.10**. The safety axis is heaviest on purpose: a model that fires a
tool at "thanks, that's all" must not be able to buy that back with tool
accuracy, and a test asserts it cannot. An axis with no cases is left out and the
rest renormalised, not scored as zero — a benchmark made only of irrelevance
cases is a legitimate benchmark.

### The model table

Four models, the same 184-case suite, one rented RTX 3090 each, Q4_K_M unless
noted. **Score is out of 100** with the weights above — irrelevance 0.40, tool
0.30, step 0.20, answer 0.10.

| model | score | tool selection | irrelevance gate | step chain | answer | wall |
|---|---|---|---|---|---|---|
| **Qwen3-4B-Instruct-2507** | **91.1** | 133/160 · 83.1% | **24/24** | 162/190 | 43/47 | 6.4 min |
| Qwen3-8B | 81.3 | 128/160 · 80.0% | **20/24** | 153/190 | 37/47 | 54.6 min |
| Qwen3-0.6B (Q8_0) | 64.0 | 52/160 · 32.5% | **24/24** | 79/190 | 28/47 | 21.3 min |
| FunctionGemma-270M (F16) | 47.4 | **0/160** | **24/24** | 24/190 | 23/47 | 28.6 min |

**Read the score with its floor in mind.** A model that never calls anything
still passes every irrelevance case, and 0.40 of the weight is exactly that — so
**40 is the floor, not zero**. FunctionGemma's 47.4 is a model that cannot call a
single tool in this format; the number to read beside it is the 0/160.

**The 8B is worse than the 4B, and the axis it loses on is the safety one.**
Twenty of twenty-four on the irrelevance gate means four messages that must not
reach a tool did. Bigger did not mean better here, and it cost eight times the
wall clock.

**gemma-3-12b is absent because it does not terminate.** On the same card and the
same suite it emitted a median of **2048 tokens per turn** — exactly the
constrained ceiling — against Qwen3-8B's 280, with three of five sampled turns
stopping on `Length` rather than on a finished call. At 18 tok/s against the 8B's
47 that is 124 seconds per case, and a full run is six hours. The verbosity is
the finding; the score was not worth the rent.

### What the grammar is worth

`tacet bench gap` runs the same calls twice, with the automaton on and off. Same
prompt, same sampler, both columns capped at 256 tokens so only the mask differs.
The 39 calls of `benchmarks/en/arithmetic-time.json`, on a rented RTX 3090,
5 Sep 2026.

| model | started a call | valid **if** started | correct call |
|---|---|---|---|
| Qwen3-4B | 76.9% → **97.4%** | 86.7% → **100.0%** | 66.7% → **97.4%** |
| Qwen3-0.6B | 20.5% → 23.1% | 100.0% → 100.0% | 7.7% → 10.3% |
| FunctionGemma-270M | 0% → 0% | — | 0% → 0% |

**`valid if started` reaching exactly 100% is the front-page claim, measured.**
Unconstrained, the 4B writes a malformed call roughly one time in seven; with the
automaton it never does — 38 calls of 38 here, and 100% in every earlier run of
this command as well.

**The 4B also STARTS twenty points more calls with the grammar on, and that is
the mask working rather than an oddity.** The automaton arms at `name(` — so a
generation that was drifting towards a signature echo instead of a call gets the
brace forced on it and lands as a real call. Constraint is not only rejecting the
invalid here; it is pulling a near-miss over the line. That is where the 4B's
+30.7 points of correct calls come from.

**The 0.6B says almost nothing, and the honest reading is that it is too small to
be asked.** It starts 8 calls of 39 unconstrained and 9 of 39
constrained. Eight is not enough to measure a rate against, and both columns being 100% valid means this suite never
caught it writing bad syntax at all — not that the guarantee is unnecessary. What
it does show is the limit: +2.6 points of correct calls. *Valid is syntax,
correct is judgement*, and the automaton was only ever the first.

**These numbers replace an earlier table, and the correction is worth more than
the table.** That version had the 0.6B *starting fewer* calls with the grammar on
(46% → 26%), a result printed on this page as "unexplained and left in". It was
an artefact of the measurement. `bench gap` decided a call had started by looking
for `name(` in the output, and an unconstrained Qwen3-0.6B spends about a third
of its turns parroting the tool signature back:

```
(time(kind: "clock", target?: "what time it is"))
calendar(kind: 'date', target?: text).
```

The `?:` is copied straight out of the tool description. That is not a call and
never becomes one — but it contains `time(`, so it counted as a start, and only
ever in the unconstrained column, because the mask forbids that shape. The
measurement was reading *"the grammar stopped the model parroting the schema"* as
*"the grammar stopped the model calling a tool"*. Requiring the brace that
Tacet's call format demands (`name({...})`) separates them, and the gap closes to
+2.6. It cost the 4B's unconstrained column twenty points too: that model echoes
signatures as well, just less often.

Two things ruled it out before the dump was read, and both are worth recording
because they are the cheap checks: the mask intervenes **19 times in 16,402
tokens** and never before token 30, so it cannot be steering the opening; and
forcing both columns through one identical argmax changed the result **not at
all**. When the intervention is that rare and the sampler is not the difference,
the remaining suspect is what the metric counts.

Speed and memory from the same runs: time to first token 392 ms (4B) / 135 ms
(0.6B) / 142 ms (270M); decode 64 / 128 / 75 tok/s; peak resident 933 MiB for the
4B at Q4_K_M, 706 MiB for the 0.6B, and 1622 MiB for the 270M at BF16 — the one
that calls nothing is also the one that costs the most memory.

### Distilling a tool-caller

A 135M model cannot call tools in this format. After three to five minutes of
training on a set Tacet generated from its own benchmarks, it can. **Every column
below is measured on 165 cases the student never saw** — a per-case 75/25 split
of the whole suite, teacher run over the train half only. RTX 3090, 5 Sep 2026.

| SmolLM2-135M-Instruct | base | distilled | + composed set |
|---|---|---|---|
| **composite** | 47.7 | 54.9 | **62.4** |
| irrelevance | **36/38** | 28/38 | 34/38 |
| tool selection | 6/101 | 39/101 | 39/101 |
| started a call | 0.0% | 63.6% | 63.6% |
| valid **if** started | — | 100% | 100% |
| **correct call** | **0.0%** | **36.4%** | 36.4% |
| `search_filter` — tool | 0/5 | 1/5 | **4/5** |
| `search_filter` — answer | 0/5 | 0/5 | 1/5 |
| `message_intent` | 0/4 | 0/4 | 0/4 |
| decode | 132 tok/s | 130–134 tok/s | 131–134 tok/s |
| peak resident | 528 MiB | 528 MiB | 528 MiB |
| training | — | 218 s | 300 s |

**These replace an earlier table that was measured on the student's own training
data.** That version reported 0% → 61.5% correct calls and a `search_filter`
score of 44.0 → 76.4. The set had been generated from every benchmark file and
the result then reported on three of them: 179 of the 1017 pairs came from the
scored cases. Held out properly the gain is real and smaller — 0% → 36.4% — and
the recipe now splits before it generates. See [training/](training/).

**The training data is the teacher's correct answers and nothing else.** With
`TACET_DISTIL_DIR` set, every benchmark step that *passes* writes its rendered
prompt and the call the teacher produced. A step that called the wrong tool
contributes nothing — that prompt is exactly where the student must not copy the
teacher. "Correct" is the benchmark's own pass/fail, not a judge model. Qwen3-4B
over the 498 train cases gave 764 pairs.

**This table is measured against `benchmarks/tasks/` as it stood at 36 cases.**
It now holds 131 — widened precisely because slot filling was starved of them —
and the distillation has not been re-run since. The numbers are what they were
measured on, not what a run today would give.

**THE COST IS THE ROW TO READ, and it is four times what this page used to
claim.** Teaching a 135M model to reach for tools costs it restraint: irrelevance
falls from 36/38 to 28/38 — eight messages that must not reach a tool now do. The
old figure was "4/4 → 3/4", which is the same effect seen through a four-case
sample. This is why the composite weights irrelevance at 0.40: a gain in tool
accuracy is not allowed to quietly buy a loss in restraint.

**The third column is what the set looks like when it is composed rather than
poured.** The pairs are four different behaviours in one pile — calls, answers
built from a tool result already in the prompt, correct abstentions, and argument
extraction — and treating them as one is why the plain column loses restraint.
Capping the answer turns at the number of call turns, tripling the handful of
extraction rows and weighting the 171 abstentions ×2 recovers six of the eight
irrelevance cases and takes `search_filter` from 1/5 to 4/5, for no loss
anywhere: 62.4 against 54.9.

**And the dial goes the other way too.** A further run capped the answers WITHOUT
weighting the abstentions: tool selection rose to 48/101 and the gate fell
further, to 27/38, for a composite of 57.9. Reaching and restraint trade against
each other, the weights are where that trade is made, and the composite is what
decides it — which is the whole reason irrelevance carries 0.40 of it.

**Slot filling is where the honest limit is.** `search_filter` goes from calling
nothing to picking the right tool 4 times out of 5 — and getting the answer right
once. The student learned *which* tool, not *what to put in it*, and no weighting
fixes that: the training half contains **13** argument-extraction rows, because
that is how many of those cases the teacher passes. Tripling thirteen rows is
thirty-nine. More cases have to be written. `message_intent` stays at 0/4
throughout.

The small denominators are the honest caveat: 5 and 4 cases for the task
benchmarks, 11 for the gap. What carries weight here is the irrelevance axis,
which is 38 cases, and the composite over all 165.

The recipe, the one dependency trap, and the constraint that teacher and student
must share a chat template are in [training/](training/).

### Down to 92 KiB

A `choice[...]` field is not a generation problem. `search_filter`'s `audience`,
`price` and `when` are five values each and `message_intent`'s `intent` is one of
four — an argmax over a handful of classes, where the guarantee the automaton
buys on a GPU is free because there is nothing to emit from.

So the same job, as hashed character n-grams into one int8 weight per class.
Trained on generated examples, scored on the 131 human-written cases in
`benchmarks/tasks/` — 95 of them written after the model was trained. Against the
distilled 135M, on the held-out cases it was scored on:

| | SmolLM2-135M | classifier |
|---|---|---|
| size | 528 MiB resident | **92 KiB** |
| work per message | ~200 tokens generated | **4,380 integer ops** |
| `search_filter` tool | 4/5 | **5/5** |
| `search_filter` slots | 1/5 | **15/15** |
| `message_intent` intent | 0/4 | **3/4** |

**Which is what makes an ESP32-S3 a real target rather than a slide.** A decode
step reads every weight once, so `tokens/s <= bandwidth / size`, and that board's
PSRAM sustains ~40 MB/s: a 135M model at Q4 is 68 MB and cannot beat 0.59 tok/s
even if it fitted, which it does not. At 92 KiB the weights are 18% of the
*internal* SRAM and the bandwidth wall never applies — 4,380 ops is 46 µs at
240 MHz, on the middle of three stated cycle assumptions.

**And 48 KiB of it now ships inside the router.** The trigger list reaches these
two tools on 87 of the 105 requests that expect one; the `tool` head, added as a
signal that only ever raises a score and never overrules one, takes that to
**102** — it catches 15 of the 18 requests no substring can reach, including the
ones that name no place at all. Measured against `eval --routing`, which is the
guard for exactly this: 166/166 reach and 166/166 in the top three, unchanged, at
pressure 0 and 20. An earlier version of the same head, trained without the other
tools' work as negatives, called 38% of the other suites' messages an extraction
request and cost fourteen of those top-three positions — which is why it is in
the repository with its false-positive rate measured rather than asserted.

**What it cannot do is the honest half.** `city`, `promised_date` and `amount`
are open text — span copying, not classification — and stay with the host. Nine
cases is a small denominator. And the device figures are arithmetic from a
measured operation count, not silicon: nothing has been run on a board.

The two implementations must compute identical features or the model is being
fed n-grams it was not fitted to, so the trainer and the C are compared
accumulator by accumulator. Comparing their *answers* was not enough — breaking
the letter folding changed every Turkish message's features and flipped no
prediction at all. The tighter check found two real bugs in a minute: Python
lowercases `İ` into two codepoints, and reading every non-ASCII character as two
bytes mistakes an em dash for a Turkish letter. Details in [esp32/](esp32/).

### Seven languages

The same Qwen3-4B against the natively-authored cores in `benchmarks/core/`
(~50 cases each, `--skip-missing` for the two web cases the rented box could not
serve):

| | en | ru | es | fr | zh | de | tr |
|---|---|---|---|---|---|---|---|
| score | 91.1 | **94.4** | 94.1 | 91.8 | 91.0 | 88.5 | **84.5** |
| tool | 133/160 | 31/34 | 28/30 | 29/33 | 28/34 | 26/32 | 23/33 |
| irrelevance | 24/24 | 13/13 | 13/13 | 13/13 | 13/13 | 13/13 | 13/13 |

**The irrelevance gate holds in all seven** — 91 of 91 across the six cores. What
moves is tool selection, and Turkish is now the weakest rather than the
strongest, which is the reverse of what the English/Turkish suite shows. The two
are not the same cases: these were written natively per language, and the
comparison to make is across this row, not against the older suite.

## Platform support — honestly

| Platform | State |
|---|---|
| macOS (arm64) | **Verified.** Full suite runs here: build, clippy `-D warnings`, tests, eval. |
| Linux | **The sandbox is measured, and on a default install it is OFF.** Measured on a stock Ubuntu 24.04 (4 Sep 2026), not only in CI: as an ordinary user `bwrap --unshare-net` fails with `loopback: Failed RTM_NEWADDR: Operation not permitted`, because the distro ships `kernel.apparmor_restrict_unprivileged_userns=1`. So `run_code` and `write_code` are **absent from the catalog** — the honest-refusal path, working as designed, but it is the DEFAULT on the most common Linux rather than an edge case. The shell says so on startup — the line that reports `run_code` as off now names the sysctl and the one-line remedy, rather than leaving "no sandbox" as the whole explanation. As **root** on the same machine the shield verifies and both tools appear, which is why testing with sudo hides the problem. What the shield itself does is measured: with the restriction lifted, `the_network_is_really_cut`, `the_sandbox_cannot_be_escaped` and the timeout kill all pass against a real `bwrap` 0.9.0 — **1069 tests, 0 failures** (33 binaries), under `TACET_SANDBOX_MUST_RUN=1`, the flag that makes the 22 tests that used to skip execute or fail the build. Still unmeasured: nobody has held a conversation with the interactive shell on Linux. |
| Windows | **Measured for the first time, on a real machine.** Windows Server 2019 (Kamatera, 4 Sep 2026), Rust 1.98.1: the workspace builds and **1037 tests pass, 0 failures** (31 binaries). Two things that CI cannot see, because GitHub's runner is not a user's machine: (1) a clean Windows has no MSVC linker, so `cargo build` stops at ``error: linker `link.exe` not found`` until ~2 GB of Visual Studio Build Tools is installed — see CONTRIBUTING; (2) a Windows build prints a `dead_code` warning for `O_NOFOLLOW`, which no job fails on because `clippy -D warnings` runs only on ubuntu. That warning is real: the symlink second belt is Unix-only. The FIRST belt was measured there and holds — `create_new` on an existing symlink gives `os error 80` and leaves the target untouched. Still unmeasured: the interactive shell, and `0600` stamping is deliberately not applied here (`set_permissions` only flips a read-only flag, which would give the appearance of protection without the substance). |

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

The most useful thing anyone can do right now: **run the interactive shell on
Linux or Windows and say what broke.** CI now exercises the Linux sandbox
against a real `bwrap` and it passes, but nobody has held a conversation with
tacet on either platform, and Windows has no runtime measurement at all — see
the platform table above.

## Star History
<a href="https://www.star-history.com/?type=date&repos=farukciftler%2Ftacet-cli">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=farukciftler/tacet-cli&type=date&theme=dark&legend=top-left&sealed_token=MfNE7RG_L6_LbXQ9Ssr9OP8hVvTFPtMdejwZ3kb_UV2-BR3alnRZ2kEpfPvfdn0yWyA9HhkF1HxCb3zW2daO2gnU1CznfjOIu68cC0j8fjGncTK8ydx1WCbcB1-2j1NRKTZ_woaKLZ3-aK60EqY8RpIifIjijA" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=farukciftler/tacet-cli&type=date&legend=top-left&sealed_token=MfNE7RG_L6_LbXQ9Ssr9OP8hVvTFPtMdejwZ3kb_UV2-BR3alnRZ2kEpfPvfdn0yWyA9HhkF1HxCb3zW2daO2gnU1CznfjOIu68cC0j8fjGncTK8ydx1WCbcB1-2j1NRKTZ_woaKLZ3-aK60EqY8RpIifIjijA" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=farukciftler/tacet-cli&type=date&legend=top-left&sealed_token=MfNE7RG_L6_LbXQ9Ssr9OP8hVvTFPtMdejwZ3kb_UV2-BR3alnRZ2kEpfPvfdn0yWyA9HhkF1HxCb3zW2daO2gnU1CznfjOIu68cC0j8fjGncTK8ydx1WCbcB1-2j1NRKTZ_woaKLZ3-aK60EqY8RpIifIjijA" />
 </picture>
</a>

## License

MIT. See [LICENSE](LICENSE).
