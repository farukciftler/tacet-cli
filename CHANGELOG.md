# Changelog

**WHY THIS FILE IS SHAPED LIKE THIS.** The version numbers in
`[workspace.dependencies]` are FLOORS, not preferences: eighteen of them carry a
comment recording the fix that a lower resolution would quietly undo. **Cargo
strips comments when it packages**, so none of that reasoning has ever reached a
crates.io consumer — the only thing that survives is the number, and a number
without its reason is indistinguishable from an arbitrary bump.

So this file is that reasoning, moved somewhere it gets published. It is not a
list of everything that changed; it is the list of versions **you cannot go
below**, and why. Entries older than the first release are reconstructed from
those manifest comments rather than written at the time — that is why they read
as reasons rather than as release notes.

The format is loosely [Keep a Changelog](https://keepachangelog.com/); the
project is pre-1.0 and versions are per-crate, so there is no single line of
releases to follow.

---

## Published 2026-09-06 — the whole workspace

All eleven crates were published together, which had not happened before. Four
of them (`tacet-web`, `tacet-mcp`, `tacet-skills`, `tacet-zip`) had **changed
source under an unchanged published version** — verified by downloading the
tarballs and diffing, not by reading the log. That state is strictly worse than a
version gap: the next publish of that number is rejected outright, and until then
no consumer can tell they are missing the fixes.

| crate | was | now |
|---|---|---|
| `tacet-kernel` | 0.1.4 | **0.1.5** |
| `tacet-zip` | 0.1.0 | **0.1.1** |
| `tacet-grammar` | 0.1.2 | **0.2.0** |
| `tacet-web` | 0.1.5 | **0.1.6** |
| `tacet-mcp` | 0.2.0 | **0.2.1** |
| `tacet-engine` | 0.1.9 | **0.1.11** |
| `tacet-skills` | 0.1.0 | **0.1.1** |
| `tacet-memory` | 0.1.1 | **0.1.2** |
| `tacet-tools` | 0.1.13 | **0.1.14** |
| `tacet-eval` | 0.1.9 | **0.1.10** |
| `tacet-cli` | 0.1.25 | **0.1.27** |

Until that day `cargo install tacet-cli` gave you 0.1.25: no `bench` subcommand,
and pinned to `tacet-engine ^0.1.9` — one release below the floor for constrained
generation on Metal, so a Mac user following the README installed the crash.

Every crate now ships the MIT text as a file rather than only the field, and
`docs.rs/tacet-engine` builds with the `candle` feature, without which
`CandleEngine`, `Architecture`, `ModelSetting` and `TokenizerSource` were all
404 on the page an evaluator opens first.

## `tacet-kernel`

**0.1.5** — the constraint contract (`Constrainer`, `ConstraintSession`,
`ConstraintError`) moved here from `tacet-engine`. Three signatures over
`&mut [f32]` and `u32`, naming no model, tokenizer, device or file. Below this
version there is no `constraint` module at all, and taking the guarantee meant
taking a crate full of GGUF loading and prompt budgeting to get it.

**0.1.2 IS THE FLOOR** — below it there is no `hash` module, and the MCP client's
PKCE challenge, the download verifier and the receipt chain would each need their
own SHA-256 again: three chances for one of them to be quietly wrong.

## `tacet-grammar`

**0.2.0** — the `tacet-engine` dependency is **gone**. That is the whole point
of the crate and it was not true before: anyone who wanted the grammar took the
engine with it. `cargo run --example no_engine` demonstrates it against a pretend
runtime, and the claim is checked from outside the checkout — `cargo add
tacet-grammar tacet-kernel` in a scratch crate, with `cargo tree | grep -c
tacet-engine` at zero.

**0.1.1 IS THE FLOOR** — below it a tool that takes NO arguments cannot be called
at all: the prompt advertises `disk_usage()` and the grammar refuses it, so the
model writes something that parses as no call and the tool never runs. Remote
(MCP) catalogs are full of such tools.

## `tacet-engine`

Every number below that is not 0.1.0 is a floor, and each was put there by a
fix that a `^0.1.0` install would have resolved away. `cargo install` picks the
newest match, but a machine with an older copy already in its registry does not
have to move — so a version carrying a gate has to be the MINIMUM, or the install
looks identical while missing the thing it was published for.

**0.1.10 IS THE FLOOR FOR ANY CONSTRAINED GENERATION ON METAL.** candle's
`sample_argmax` is `logits.argmax(D::Minus1)?.to_scalar::<u32>()`, and on the
Metal backend that reduction hands back the extremum's BITS rather than its INDEX
once the distribution is mostly `-inf` — the exact shape a grammar mask produces.
A turn died with `constraint rejected the token: 4286578688`, which is
`0xFF800000`, the bit pattern of `-inf`. No vocabulary has four billion entries;
it was never a token id, and the message named the wrong layer.

**0.1.9 closes a hole straight through the headline claim.** The README says a
malformed tool call is unrepresentable. That held for every ORDINARY token and
never covered the one that is not: end-of-turn was never masked, so a model could
simply STOP in the middle of a JSON string. Measured on qwen3-4b, a `write_code`
call ended on `... print(result]})` with the string, the array and the object all
still open; from the outside it read as the model refusing to call the tool.

**0.1.8 IS THE FLOOR FOR THE TURN LOOP** — below it there is no
`FINAL_PASS_INSTRUCTION`, so the last pass of a turn is offered tools it cannot
usefully call and a model that keeps calling ends the turn with nothing said. It
also carries the quant label fix: below it `dominant_quant` counts TENSORS, and
Gemma3's six F32 norms per layer outvote its quantized body, so a q4 file is
reported as `F32` and a comparison matrix reads as if the model had been run
unquantized.

**0.1.5 IS THE FLOOR FOR ANY SKILL TO SURVIVE A TOOL CALL** — below it the
`<guidance>` block was written ONLY inside the question turn, and from the second
turn of the tool loop the question is deliberately empty, so the guidance
vanished at exactly the turn where the model is recovering from a tool result.
`Plain` kept it, which made it worse than a plain bug: eval runs on FakeEngine,
FakeEngine is `Plain`, so the one template no model ever sees was the one the
measurement covered.

**0.1.3 IS THE FLOOR for anything that reports a measurement** — below it an
engine can only say its own name (`candle`), so four different weight files
produce four indistinguishable reports and a comparison matrix can silently
measure one model in every cell.

## `tacet-tools`

**0.1.13 IS THE FLOOR FOR WINDOWS**, and the defect was invisible on the two
platforms this is developed on. The path handed to the model after a write was
built with `strip_prefix` against a working directory canonicalized at a
different moment; on Windows canonicalization adds the `\\?\` verbatim prefix,
so one side carried it and the other did not, the strip silently failed, and the
model was handed a full absolute path spelled with backslashes — which it then
has to escape to write back inside a JSON argument.

**0.1.12 IS THE FLOOR FOR THE ROUTER**, and the defect it fixes had been silently
setting the ceiling on every tool-selection number this project has ever
recorded. A tool's score was the sum of hint lengths matched over its NAME AND
DESCRIPTION GLUED TOGETHER, so it grew with the SIZE OF ITS PROSE: `run_code`,
whose thousand-character description correctly says it cannot open a FILE, cannot
see a FOLDER and must not LIST from MEMORY, outscored `find_file` three to one on
"Find the file about the budget." — `find_file` sat at rank 7 on its own sentence.

**0.1.11** — below it `create_document` has no way to NAME its destination, so a
file asked for in one directory is written to another and reported as created.

**0.1.9 fixes a sentence the model reads on every turn**: `create_document`'s
description said "do note ask or narrate" where it meant "do NOT ask". It also
puts a ceiling under the tool descriptions, the largest block in the prompt,
which until then had none — the worst nine-tool selection measures 2542 tokens,
62% of the 4096-token floor window, re-sent on every pass of the tool loop.

**0.1.8** carries the trailing-comma repair for small models AND its tests: the
repair landed untested, and a function that rewrites the model's own words before
parsing them is the last place to leave untested.

**0.1.7 closes two failures that were not luck** — each failed in every run of
every variant measured. Below it a bare `{"kind":"date"}` is dropped as ambiguous
(two tools accept it, only one by a closed set), and the memory tool lets the
model answer "I'll remember that" without calling anything, which tells the user
something untrue.

**0.1.6 IS THE FLOOR FOR ANY NON-ENGLISH USER** — below it the router's trigger
table is English-only, so a message in Turkish touches nothing, scores zero and
the budget fills with the head of the catalog. It also matched triggers by bare
substring, so "teşekkürler" contains "url" and a thank-you pulled the web tools
forward.

**0.1.5 IS THE FLOOR for anyone using MCP** — below it the router cannot see a
remote catalog at all, so the model answers "I have no access to servers" while
the connection is working perfectly.

**0.1.4** — below it `shell` is offered on Windows, where the timeout cannot kill
the process group and a runaway command would outlive the turn. **0.1.3** brought
the addon gates and the five tools behind them.

## `tacet-web`

**THE NETWORK MONOPOLY.** A network call exists only in this crate and
`tacet-mcp`. No other crate opens a socket, and so the rule can be audited by eye
rather than by a script, the HTTP dependency stands only in these two manifests.

**0.1.5 IS THE FLOOR FOR `tacet update`** — below it the release asset's published
SHA-256 is never read, so every update is trust-on-first-use over TLS alone,
while the API response already carried the digest. It also refuses a dotless host
in the addon allow-list, where `merhaba` used to be accepted as a hostname.

**0.1.3 IS THE FLOOR** — below it the `http` addon's host allow-list cannot read
its own stored form, so a multi-host install refuses every call it was configured
to permit. **0.1.2 carried the SSRF gate**, and `^0.1.0` would happily have
resolved to 0.1.1, which does not have it.

## `tacet-mcp`

**0.2.0 speaks the 2026-07-28 revision**: stateless requests, the `_meta`
envelope, MRTR, poll-based tasks, issuer-bound OAuth. The 0.1 line speaks only
the frozen revision, and a `^0.1` resolve would silently put a client on it.

## `tacet-eval`

**0.1.8 IS THE FLOOR FOR ANY NUMBER THAT WILL BE BELIEVED**, and it carries
four corrections to the INSTRUMENT rather than to the thing measured:

* `routing` — the router's own choice, no weights, milliseconds. It sets the
  ceiling on the selection number and had no measurement of its own.
* The generation budget. The selection set handed the model
  `SamplingSetting::default()` (1024 tokens) while the shell hands it
  `generation_cap` (~14000 on qwen3-4b), so the two cases that ask for a real
  script were failing on a limit production does not impose.
* **The language gate passed everything.** It matched two-letter markers as
  substrings, so "have" satisfied the Turkish "ve" and "için" satisfied the
  English "in": every Turkish case had been reporting a perfect answer rate
  against a check no input could fail.
* The prompt. The set built no `<guidance>` block, so it measured a prompt the
  app has never sent.

**0.1.7 IS THE FLOOR FOR ANY MEASUREMENT THAT WILL BE COMPARED** — below it the
logic runner repeats the user's question AFTER the tool result on every pass, the
shape the shell removed because it caused the loop, so the set measures a prompt
the product does not build.
