# Working in this repository

## The README is part of the build

**Every change that alters what the README says must update the README in the
same commit.** Not afterwards, not in a follow-up — the two go together or the
change is not finished.

This is a rule rather than a preference because the failure mode is specific and
this project has walked into it more than once: a page full of measured numbers
is the most credible thing here and therefore the most damaging thing to leave
stale. A stale README is not a documentation problem, it is a false measurement
with a date on it.

What counts as "alters what the README says":

* a number the page quotes — test counts, eval scores, timings, sizes, versions;
* a command, flag or subcommand name the page tells the reader to type;
* a type or crate name in the architecture diagram;
* a claim about what is measured, on which platform, and what is still not;
* a tool joining or leaving the default catalog, or an addon changing what it
  opens.

Before opening a PR, re-derive the claims you touched instead of trusting them.
Cheap checks that have each caught a real error:

```bash
cargo run -p tacet-cli -- --help          # every subcommand the page names exists
cargo run -p tacet-cli -- eval            # the case count the page quotes
grep -n '^version' crates/tacet-cli/Cargo.toml
grep -v '^\s*#' crates/*/Cargo.toml | grep ureq     # the network-monopoly claim
```

A number that cannot be re-derived should carry the date and machine it was
measured on, the way the platform table does.

## The rest of the shape

The conventions, the four gates, how a model claim is made, and why the model
measurement is run by a human are in [CONTRIBUTING.md](CONTRIBUTING.md). Read it
before the first change; it takes five minutes.

Two habits that matter more here than in most repositories:

**A comment says why, not what.** The code says what. Where the obvious approach
was tried and failed, record the measurement that killed it — several comments in
this tree exist only to stop the next person rebuilding something that does not
work.

**A green test that cannot go red is worse than no test.** After writing a guard,
break the thing it guards and watch it fail. This project has shipped a test that
compared a list against itself, and a baseline guard that checked library
functions rather than the command that runs them.
