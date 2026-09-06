# Contributing

Thanks for looking. This is a small project with a clear shape, so contributing is mostly a matter of knowing four things — they take five minutes to read and they are why the code looks the way it does.

## The quickest way to help

**Run it on your machine and tell us what broke.** macOS is the only platform anyone has actually *used* this on. Linux and Windows compile and pass tests in CI, but the interactive shell, the sandbox and the timezone lookup have never been exercised by a human there. If you are on Linux or Windows: install it, have a conversation, try `tacet models list`, and open an issue with what looked wrong. That is not a small contribution — it is the one we cannot do ourselves.

Other good places to start:

- A tool that is missing. Tools are self-contained: one file, one schema, tests next to it. `crates/tacet-tools/src/calc.rs` is the shortest one to read first.
- A skill — a Markdown file with trigger phrases and a short piece of guidance. No Rust needed.
- Docs that are wrong. If a command in the README does not do what it says, that is a bug and a fix is welcome as a one-line PR.
- Anything marked **NOT MEASURED** in a comment. Those are honest admissions that something was never tested on real hardware. Measuring one and reporting the result is genuinely useful, even if nothing needs changing.

## Four things that will save you a rewrite

**1. Dependencies are architectural decisions, not conveniences.** The direct list is `serde`, `serde_json`, `thiserror`, `clap`, `crossterm` and `ureq`; behind the off-by-default `candle` feature, `candle-core`, `candle-transformers` and `tokenizers`. Nine, not seven — and `tokenizers` pulls `onig_sys`, so that feature is a second C build on top of `ring`'s. Zip, deflate, CRC32, OOXML and the MCP client are written by hand because being able to read the whole dependency graph is part of what this tool offers. If you need a crate, say so in the PR and explain what it buys — the answer is not automatically no, but it is a conversation, and the reason goes in a comment at the top of the manifest.

**2. Only two crates may touch the network.** `tacet-web` and `tacet-mcp`. The HTTP dependency appears in exactly those two manifests, which means anyone can audit the privacy claim with `grep`. If your feature needs the network, it calls one of those two — it does not open a socket itself.

**3. Comments explain *why*, not *what*.** The code says what it does. A comment earns its place by recording a decision, a measurement, or a mistake that was already made once. Several comments in this codebase say things like *"this claim was false until it was measured"* — those are the most valuable lines in the file, and deleting one loses knowledge that cost somebody a debugging session.

**4. A test measures the guarantee, not the code.** `assert!(result.is_ok())` after a refactor proves nothing. Feed the hostile input and show the refusal; break the invariant and show the failure. When a security fix landed here, each one shipped with a test that reproduces the exploit. If you cannot write a test that would fail without your change, that is worth mentioning in the PR — sometimes it means the change is fine, sometimes it means the change is not doing what it looks like.

## Before you open a PR

**On Windows you need a C toolchain, and the reason is not the one it looks
like.** A clean Windows stops at ``error: linker `link.exe` not found``, which
reads as "install MSVC". Installing Rust's GNU toolchain instead gets past the
linker — it carries its own — and then stops again:

```
error: failed to run custom build command for `ring v0.17.14`
  failed to find tool "gcc.exe": program not found
```

`ureq -> rustls -> ring`, and `ring` compiles C. So the prerequisite is a C
compiler, however you get one. Three routes, all measured on a fresh Windows
Server 2019 on 4 Sep 2026:

| route | download | note |
|---|---|---|
| **prebuilt binary** | ~15 MB | most people want this — see Releases; nothing is built |
| **GNU + MinGW-w64** | ~378 MB | builds the workspace in 1 m 35 s, `eval` reads 78/78 |
| MSVC Build Tools | ~2 GB | what CI's `windows-latest` already has |

The GNU route, end to end:

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu
# unpack a MinGW-w64 build (winlibs, MSYS2, …) and put its bin\ on PATH
cargo +stable-x86_64-pc-windows-gnu build --workspace
```

On Windows 11 or Server 2022+ the toolchain is one line, because `winget` is
built in — `winget install BrechtSanders.WinLibs.POSIX.UCRT.LLVM` or an MSYS2
package. **Server 2019 has neither `winget` nor `choco`** (winget wants 1809+
desktop or Server 2022), so there the download is manual; that is the machine
these numbers came from.

None of this is visible to CI, which is why it is written here: GitHub's runner
ships MSVC, so the build simply works there and the prerequisite never appears.

```bash
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo fmt --all
cargo run -p tacet-cli -- eval              # behavioural cases, deterministic, no model needed
cargo run -p tacet-cli -- eval --routing    # the router's own choice, no model needed
```

CI runs all of it on macOS, Linux and Windows. Neither of these loads weights or opens a socket.

### The three measurements, and which one to reach for

| Command | Measures | Cost | Gates CI |
|---|---|---|---|
| `eval` | Tacet's LOGIC, on a mock engine | milliseconds | yes |
| `eval --routing` | which tools the ROUTER puts in the prompt | milliseconds | yes |
| `eval --tool-selection --model <name>` | the MODEL's choice, on real weights | ~44 min on Metal (~6 min on a 3090) | no |

Reach for `--routing` first when a tool "stops being called". The router shows the
model at most nine tools out of the catalog, and **a tool that is not in those
nine cannot be called however well the model reasons** — so a routing defect
looks exactly like a model regression, arrives three quarters of an hour later, and gets
fixed in the wrong place. `--routing` answers it in milliseconds and prints the
rank the expected tool landed at. `tacet why "<message>"` explains one message
in the same terms.

### Claiming an improvement

Two runs of the same suite differ by a case or two for no reason. Do not read
"+3 points" off two table headers — one case is worth 1.28 points on the 78-case
behavioural suite and 0.54 points on the 184-case pooled selection suite, and the
analysis module puts the threshold at **six paired cases moving one way and none
the other**. That threshold is a property of the sign test (2 × 0.5⁶ = 0.031),
not of the suite size, so it does not move when cases are added; what moves is
how many POINTS six cases are worth — 7.7 on the behavioural suite, 3.3 on the
pooled selection suite. Produce both reports with `--json` and let the comparison
say it:

```bash
cargo run -p tacet-cli -- eval --compare before.json after.json
```

It pairs the cases by name, runs a sign test over the ones that moved, prints a
bootstrap interval, and states a verdict. It works on any of the three reports.

### The model measurement is run by a human, on purpose

`eval --tool-selection` needs real weights (a 2.5 GB GGUF, `--features metal` on
a Mac or `--features candle` elsewhere) and about three quarters of an hour: the
measured figure on Metal is 44 minutes for both languages, because
`selection_suite()` runs both by default. `--turkish` runs one of them. This said
"about twenty minutes" in two places while the README measured 44 for the same
command. **It does not
gate PRs and it is not automated in CI**, and that is a decision rather than a
gap. On a GitHub-hosted runner the same suite would take hours of 2-vCPU CPU
inference, would evict the build cache every PR depends on, and — worst — would
produce a number from a build nobody ships on hardware that changes underneath
you, so `--compare` would report a runner swap as a regression. A self-hosted
macOS runner is the only shape that gives a readable number, and GitHub advises
against those on public repositories because a fork's workflow can execute code
on the machine. The reasoning is written out in full at the top of
`.github/workflows/nightly.yml`.

So the model half works like this:

```bash
# On your own machine, with the weights already on disk:
cargo run -p tacet-cli --features metal -- eval --tool-selection --json > after.json
cargo run -p tacet-cli -- eval --compare crates/tacet-eval/baselines/<baseline>.json after.json
```

**Which weights, exactly.** `qwen3-4b` in the built-in catalog is
**Qwen3-4B-Instruct-2507** Q4_K_M, 2 497 281 120 bytes, pinned by sha256. It is
not `Qwen/Qwen3-4B` — that is the older hybrid model, it answers to the same
short name in conversation, and for a while it was what the catalog actually
downloaded while every number in the README came from the 2507 file. Measured on
one GPU, one suite, one build: the hybrid model spends a median of 247 generated
tokens per turn against 20 for the instruct model, so the two are not
interchangeable and a report made on the wrong one is not comparable to
anything here. `the_default_package_is_the_weights_the_baseline_was_measured_on`
now fails the build if the catalog and the baseline drift apart again.

**`--compare` refuses two reports made on different weights.** It reads
`identity.model_fingerprint` and stops rather than warns: a sign test over paired
cases answers whether *this change* helped, which needs the model held still, and
a verdict printed under a warning is a wrong verdict people scroll past.
Comparing two models is a fair thing to want — it is just a different question,
and this command does not answer it.

**A claimed model improvement arrives as a PR that updates the baseline and
pastes the `--compare` verdict**, not as two percentages in a description. Before
checking a model report in, replace `identity.model_path` with a bare file name:
it is the absolute path to the GGUF on your machine, and this repository is
public. `cargo test -p tacet-eval --test baselines` refuses a baseline that
carries one, and refuses one whose case names no longer match the suite — a
baseline nobody can pair against still prints a verdict, which is worse than
having none.

### Measuring against someone else's benchmark

`crates/tacet-eval/examples/bfcl_irrelevance.rs` runs BFCL's `irrelevance`
category through this stack with **BFCL's own function definitions**. It exists
because every other number here is this project grading itself, and the
"irrelevance gate" is BFCL's relevance/irrelevance detection under another name.

```bash
curl -sLO https://raw.githubusercontent.com/ShishirPatil/gorilla/main/berkeley-function-call-leaderboard/bfcl_eval/data/BFCL_v4_irrelevance.json
BFCL_JSON=crates/tacet-eval/baselines/bfcl-irrelevance-<model>-<device>.json \
  cargo run --release -p tacet-eval --features metal --example bfcl_irrelevance \
  -- ~/models/qwen3-4b/model.gguf BFCL_v4_irrelevance.json
```

The data is **not vendored**: it is someone else's benchmark and it moves. Three
translations stand between their format and this one — their `dict`/`float` type
names, names carrying characters this call format cannot express, and one turn
rather than a conversation — and each is commented where it happens. **Report all
three counts.** A harness that quietly rewrote 92 names and dropped 3 cases and
said neither would be reporting a number for an easier set than the one it claims.

An artifact in `baselines/` that declares `benchmark` and `source` is understood
to be an external report and is exempt from the name-pairing check below — no
suite here will ever match case names that belong to someone else. It is NOT
exempt from the local-path check.

`crates/tacet-eval/baselines/fake-engine.json` is the one baseline that needs no
weights. It is a real `eval --json` report, byte-reproducible, and the nightly
job pairs against it so the comparator itself is exercised. **Add a case and you
must regenerate it in the same change:**

```bash
cargo run -p tacet-cli -- eval --json > crates/tacet-eval/baselines/fake-engine.json
```

## Writing a tool

Tools are the easiest thing to add and the most useful. The shape:

- One file in `crates/tacet-tools/src/`, registered in `catalog.rs`.
- A schema. The schema is not documentation — a pushdown automaton is built from it, and the model literally cannot generate arguments that violate it. Get the schema right and you get correctness for free.
- Bulk data does not go to the model. If your tool can return a lot (a file, a diff, a table), put it in the `DataStore` and hand the model a short summary plus a reference. The context window is 4096 tokens and it fills faster than you expect.
- If your tool reads the user's own data, it marks the session — so that a later call which sends data outward passes the approval gate.

## What gets a PR merged quickly

A small change, a test that would fail without it, and a sentence about why. That is the whole bar. You do not need to match the comment style of the file you are editing on the first try; that is what review is for.

Bug reports are equally welcome without a fix attached. A clear reproduction is worth more than a guess at the cause.

## Publishing to crates.io

**The order is not a preference, it is the dependency graph.** Cargo refuses to
publish a crate whose path dependencies are not already on the registry at the
version the manifest declares, so a publish out of order fails halfway and leaves
the workspace half-released — some crates on crates.io pointing at versions that
do not exist yet.

```
tacet-kernel     tacet-zip                    # nothing in-tree
tacet-grammar    tacet-web       tacet-mcp    # kernel
tacet-engine                                  # kernel, grammar
tacet-skills                                  # kernel, engine (DEV)
tacet-memory                                  # kernel, skills
tacet-tools                                   # kernel, zip, memory, skills, web, mcp, engine
tacet-eval                                    # + grammar
tacet-cli                                     # everything
```

**DEV-DEPENDENCIES CONSTRAIN THE ORDER TOO, and that is not obvious.** `cargo
publish` runs a verification build of the packaged tarball, and that build
resolves `[dev-dependencies]` like any other. `tacet-skills` has ONE in-tree
dependency in `[dependencies]` — the kernel — and a dev-dependency on
`tacet-engine`, which is six crates further down. Ordered by `[dependencies]`
alone it fails with `failed to select a version for the requirement
tacet-engine = "^0.1.11"` and takes `tacet-memory`, `tacet-tools`, `tacet-eval`
and `tacet-cli` with it, because each one waits on the last.

Derive the order from ALL the dependency tables, never from what the crates feel
like:

```bash
for f in crates/*/Cargo.toml; do
  echo "$(basename $(dirname $f)): $(awk '/^\[(dependencies|build-dependencies|dev-dependencies|target\..*dependencies)\]/{p=1;next} /^\[/{p=0} p&&/^tacet-/{gsub(/\..*/,"",$1); printf "%s ", $1}' $f)"
done
```

**And publish one crate per command, checking the exit status.** A loop of the
form `cargo publish -p $c | tail` reports success for every crate no matter what:
the pipeline's status is `tail`'s. That is how a run here reported eleven
successes when three had failed — including one, `tacet-skills`, whose failure
was the dev-dependency above.

Before any of it, two things that have each cost a release here:

1. **A dry run only works one step ahead of the registry, and it is worth knowing
   that before you plan around it.** Measured 6 Sep 2026 with kernel 0.1.5 not yet
   published: `cargo publish --dry-run` succeeded for `tacet-kernel` and
   `tacet-zip` and failed for the other nine — every one of them with
   `failed to select a version for the requirement tacet-kernel = "^0.1.5"`, never
   with a packaging error. `cargo package --no-verify` fails identically, because
   resolution happens before packaging. So there is no "check everything first"
   pass: dry-run what you are about to publish, publish it, then dry-run the next.
   The one thing you CAN check up front is the tarball contents of the roots
   (`cargo package --list -p <crate>`), which is where a missing `LICENSE` or a
   `path` dependency with no `version` would show.
2. **A crate whose SOURCE changed must have a new version, even if nothing about
   it looks like a release.** Four crates in this repository once had changed
   source under an unchanged published number, which is strictly worse than a
   version gap: the next publish of that number is rejected, and until then no
   consumer can tell they are missing the fixes. `every_declared_floor_is_the_version_the_member_actually_has`
   in `crates/tacet-cli/tests/manifest_floors.rs` catches the half of this that a
   test can catch — a floor left behind after a bump; the other half is noticing
   you changed something. `diff -r` against the published tarball settles it:

   ```bash
   curl -sL -A tacet "https://crates.io/api/v1/crates/tacet-mcp/0.2.0/download" | tar xz
   diff -rq tacet-mcp-0.2.0/src crates/tacet-mcp/src
   ```

**After the chain, check the claim from outside the checkout.** Everything above
proves the crates uploaded; it does not prove the reuse story works for someone
who has never cloned this repository — which is the only place it can be false.

```bash
cargo new /tmp/reuse && cd /tmp/reuse
cargo add tacet-grammar tacet-kernel serde_json
cp <repo>/crates/tacet-grammar/examples/no_engine.rs src/main.rs
cargo run                          # prints the allowed set and the closed token
cargo tree | grep -c tacet-engine  # must be 0
```

**The version numbers in `[workspace.dependencies]` are FLOORS.** Roughly twenty
comments there each record a fix that a lower resolution would quietly undo —
"below 0.1.10 constrained generation on Metal dies with `constraint rejected the
token: 4286578688`" and so on. Cargo strips comments when it packages, so none of
that reasoning reaches a crates.io consumer; the floor is what survives, and it
has to be right.

## Reporting something security-relevant

The policy now lives in [SECURITY.md](SECURITY.md), which is where GitHub's
Security tab looks and where someone searching for it would think to open. Short
version, unchanged: if you find something that lets code escape the sandbox, read
files outside it, or send data somewhere it should not go, open a **private**
security advisory rather than a public issue, and give it a few days before
writing about it publicly.

## The supply chain

`cargo deny check` runs on every push (`deny.toml`, and the `supply chain` job in
`ci.yml`) over the full graph with all features on. Run it locally with
`cargo install cargo-deny && cargo deny check`.

Two of its four checks are hard failures and two are not, deliberately.
`unknown-git`, `unknown-registry` and `wildcards` can only be caused from inside
this repository, so they gate. An advisory filed against a crate four levels down
is news about the world rather than a defect in your pull request, so the CI job
carries `continue-on-error` and the value is the annotation.

**An entry in `deny.toml`'s `ignore` list needs a sentence and a date.** There
are two, both from its first run, both with the reason written next to them.
Silencing something without saying why is how this file stops meaning anything —
and then it is worse than not having it, because it looks like an audit.

Releases carry a Sigstore build-provenance attestation over every asset and an
SPDX SBOM. `SHA256SUMS`, which was there before, proves only that the release job
agreed with itself.

## Licence

MIT. By contributing you agree your work ships under it.
