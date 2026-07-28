#!/bin/sh
# Tacet installer — macOS & Linux. ONE paste does everything:
#   1. installs Rust's build tool (rustup) IF cargo is missing,
#   2. builds and installs the `tacet` binary with cargo,
#   3. downloads the model (one-time, ~2 GB, stays on your disk),
#   4. starts your first chat.
# It never touches your shell config beyond what rustup itself does, and apart
# from the two downloads above nothing is sent anywhere: Tacet runs on this
# machine.
set -eu

say()  { printf '%s\n' "$*"; }
fail() { printf 'tacet install: %s\n' "$*" >&2; exit 1; }

MODEL="qwen3-4b"

say ""
say "Tacet."
say "the quiet assistant · installing on this machine"
say ""

command -v curl >/dev/null 2>&1 || fail "curl is required (install curl and re-run)"

# --- 1 · Rust toolchain -------------------------------------------------------
if ! command -v cargo >/dev/null 2>&1; then
    if [ -x "$HOME/.cargo/bin/cargo" ]; then
        PATH="$HOME/.cargo/bin:$PATH"
    else
        say "· rust is not installed — getting rustup (the official installer)"
        curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
        PATH="$HOME/.cargo/bin:$PATH"
    fi
fi
say "· cargo found: $(command -v cargo)"

# --- 2 · Tacet ---------------------------------------------------------------
# On Apple silicon the Metal feature roughly doubles generation speed; on
# Linux the plain candle engine is the right build.
case "$(uname -s)" in
    Darwin) FEATURES="metal" ;;
    *)      FEATURES="candle" ;;
esac
say "· building tacet (features: $FEATURES) — a few minutes on first install"
cargo install tacet-cli --features "$FEATURES"

TACET="$HOME/.cargo/bin/tacet"
[ -x "$TACET" ] || TACET="tacet"

# --- 3 · The model -----------------------------------------------------------
# Skipped when the model folder already exists. `--no-approval` only skips
# tacet's own "download? [Y/n]" prompt — running this installer IS that answer,
# and the script announced the download at the top.
if [ -d "$HOME/models/$MODEL" ] || [ -d "$HOME/.tacet/models/$MODEL" ]; then
    say "· model $MODEL already on disk — skipping the download"
else
    say "· downloading $MODEL (one-time, ~2 GB, stays on your disk)"
    "$TACET" models download "$MODEL" --no-approval
fi

# --- 4 · First chat ----------------------------------------------------------
say ""
say "done."
if [ -r /dev/tty ] && [ -w /dev/tty ]; then
    say "starting tacet — type away, /quit leaves."
    say ""
    exec "$TACET" chat < /dev/tty
else
    say "start talking with:"
    say "  tacet chat"
fi
say ""
say "everything else: see the Tacet website"
