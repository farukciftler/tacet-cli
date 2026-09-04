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

**1. Dependencies are architectural decisions, not conveniences.** The full list is `serde`, `serde_json`, `thiserror`, `clap`, `crossterm`, `ureq`, plus `candle` behind an off-by-default feature. Zip, deflate, CRC32, OOXML and the MCP client are written by hand because being able to read the whole dependency graph is part of what this tool offers. If you need a crate, say so in the PR and explain what it buys — the answer is not automatically no, but it is a conversation, and the reason goes in a comment at the top of the manifest.

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
| `eval --tool-selection --model <name>` | the MODEL's choice, on real weights | ~20 minutes | no |

Reach for `--routing` first when a tool "stops being called". The router shows the
model at most nine tools out of the catalog, and **a tool that is not in those
nine cannot be called however well the model reasons** — so a routing defect
looks exactly like a model regression, arrives twenty minutes later, and gets
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
a Mac or `--features candle` elsewhere) and about twenty minutes. **It does not
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

**A claimed model improvement arrives as a PR that updates the baseline and
pastes the `--compare` verdict**, not as two percentages in a description. Before
checking a model report in, replace `identity.model_path` with a bare file name:
it is the absolute path to the GGUF on your machine, and this repository is
public. `cargo test -p tacet-eval --test baselines` refuses a baseline that
carries one, and refuses one whose case names no longer match the suite — a
baseline nobody can pair against still prints a verdict, which is worse than
having none.

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

## Reporting something security-relevant

If you find something that lets code escape the sandbox, read files outside it, or send data somewhere it should not go, please open a **private** security advisory on GitHub rather than a public issue, and give it a few days before writing about it publicly.

## Licence

MIT. By contributing you agree your work ships under it.
