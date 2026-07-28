# Tacet

*tacet* — in musical notation: *this instrument is silent in this passage*. Here, the silent instrument is the network.

A personal assistant whose core runs entirely on the device. Model inference, chat history, and calendar/reminder/contact/note data never leave the phone. Two surfaces can leave it — web search and MCP connections — and both are off until you turn them on, with every exit visible on screen.

This repository is private and holds two implementations of the same product.

| | Path | Status |
|---|---|---|
| iOS app | [`Tacet/`](Tacet/) | SwiftUI + SwiftData on Apple's Foundation Models. The product. |
| Terminal | [`tacet-rs/`](tacet-rs/) | The same logic layer in Rust. Published separately — see below. |

## The CLI is public

The Rust workspace is mirrored to a public repository and released from there:

**https://github.com/farukciftler/tacet-cli**

```bash
cargo install --git https://github.com/farukciftler/tacet-cli tacet-cli --features metal
```

A crates.io release is prepared but not yet published: publishing needs a
verified email address on the crates.io account.

Publishing it costs nothing commercially — the CLI is the logic layer, not the product — and it buys two things the private repo cannot: the privacy claim becomes auditable by anyone who wants to read it, and CI compiles and tests Linux and Windows on every push, which no machine here can do.

`tacet update` checks that repository's releases. It runs only when typed: a program whose promise is that it stays off the network cannot quietly go online to ask about itself.

**The public repo is a copy, not a submodule.** `tacet-rs/` here is the working tree; changes are pushed onward. Two things are deliberately left behind: `STATUS.md` (an internal measurement log) and anything that names how the code was written.

## Specifications

The design is written down before it is built, and the documents are kept true to the code afterwards.

- [`tacet-spec.md`](tacet-spec.md) — product and design specification
- [`memory-spec.md`](memory-spec.md) · [`timeline-spec.md`](timeline-spec.md) · [`code-spec.md`](code-spec.md)
- [`web-search-spec.md`](web-search-spec.md) · [`mcp-connection-spec.md`](mcp-connection-spec.md) — the two surfaces that touch the network

## Build

```bash
# iOS
xcodebuild -project Tacet.xcodeproj -scheme Tacet -sdk iphonesimulator \
  -destination 'generic/platform=iOS Simulator' -configuration Debug \
  build CODE_SIGNING_ALLOWED=NO

# Terminal
cd tacet-rs
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p tacet-cli -- eval
```

SourceKit reports "Cannot find type X in scope" across this project even when it compiles; the indexer cannot resolve the whole target. `xcodebuild` is the only authority.

## App Store

Screenshots and store copy live in [`marketing/appstore/`](marketing/appstore/). The screenshot set is generated, not hand-assembled — `screenshots/compose.py` builds the frames from real simulator captures, and `--demo-seed` (a DEBUG-only launch argument) fills the store with records the app itself would write. The rule for that seed: only states the app can genuinely produce, because a screenshot of anything else is a claim the product cannot keep.
