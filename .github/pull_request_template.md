<!--
CONTRIBUTING.md has the four gates and they are short. The three questions below
are the ones that actually decide whether this merges quickly.
-->

**What changes, and why**

**The test that would fail without this**

<!-- `assert!(result.is_ok())` after a refactor proves nothing. Break the thing
your test guards and watch it go red — this repository has shipped a test that
compared a list against itself, and a baseline guard that checked library
functions rather than the command that runs them.

If you cannot write one, say so here. Sometimes that means the change is fine;
sometimes it means it is not doing what it looks like. -->

**Does this change anything the README says?**

<!-- A number it quotes, a command it tells the reader to type, a type or crate
name in the architecture diagram, a claim about what is measured and on what, a
tool joining or leaving the default catalog. If yes, the README changes IN THIS
PR — the rule is in CLAUDE.md and the failure mode it exists for has happened
here more than once.

Re-derive what you touched rather than trusting it:
    cargo run -p tacet-cli -- --help
    cargo run -p tacet-cli -- eval
    grep -v '^\s*#' crates/*/Cargo.toml | grep ureq
-->

**Gates**

- [ ] `cargo fmt --all`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `cargo run -p tacet-cli -- eval` (and `--routing` if you touched the router)
