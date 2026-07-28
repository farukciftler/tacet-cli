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
warn() { printf '%s\n' "$*" >&2; }
fail() { printf 'tacet install: %s\n' "$*" >&2; exit 1; }

MODEL="qwen3-4b"
# The 4B model at Q4 needs ~2.3 GB for the weights and ~0.6 GB of KV cache for a
# short conversation. 4 GB is that plus room for the rest of the machine to keep
# working. MEASURED THE HARD WAY: on a VPS sitting at 52% memory and 80% swap,
# the first chat was killed by the kernel with a bare "Killed" — no message, no
# clue, and the user was left staring at a one-word failure.
NEED_MB=4096

say ""
say "Tacet."
say "the quiet assistant · installing on this machine"
say ""

command -v curl >/dev/null 2>&1 || fail "curl is required (install curl and re-run)"

# --- 0 · Is there room? -------------------------------------------------------
# Reported BEFORE the 2 GB download rather than after: someone whose machine
# cannot run the model should learn it before spending the bandwidth, not when
# the kernel kills the first chat.
available_mb() {
    if [ -r /proc/meminfo ]; then
        # MemAvailable is the kernel's own estimate of what can be handed out
        # without swapping — a better question than "how much is free".
        awk '/^MemAvailable:/ { printf "%d", $2 / 1024; exit }' /proc/meminfo
    elif command -v vm_stat >/dev/null 2>&1; then
        vm_stat | awk '
            /page size of/ { for (i = 1; i <= NF; i++) if ($i ~ /^[0-9]+$/) page = $i }
            /Pages free/ || /Pages inactive/ { gsub(/\./, "", $NF); pages += $NF }
            END { if (page == 0) page = 4096; printf "%d", pages * page / 1048576 }'
    else
        echo ""
    fi
}

ROOM=$(available_mb 2>/dev/null || echo "")
TIGHT=0
if [ -n "$ROOM" ] && [ "$ROOM" -lt "$NEED_MB" ] 2>/dev/null; then
    TIGHT=1
    warn "· WARNING: about ${ROOM} MB of memory is available and the model wants ~${NEED_MB} MB."
    warn "  The install will finish, but the first chat would very likely be killed by the"
    warn "  kernel. Tacet will be installed and the chat NOT started; free some memory (or"
    warn "  use a smaller model) and run:  tacet chat"
    warn ""
fi

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

# --- 4 · Can the user actually type `tacet`? ---------------------------------
# THIS SCRIPT USED TO LIE HERE. It sets PATH for its OWN process, calls the
# binary by its full path, and then told the reader to run `tacet chat` — which
# in their shell answered "command not found", because a child process cannot
# change its parent's environment. rustup writes the PATH line into the shell
# profile, but that only takes effect in a NEW shell; the one they are sitting in
# is untouched. So the question is asked properly: does `tacet` resolve the way
# THEY will type it?
if command -v tacet >/dev/null 2>&1; then
    ON_PATH=1
else
    ON_PATH=0
fi

say ""
say "done."

if [ "$ON_PATH" -eq 0 ]; then
    say ""
    say "one more step — `tacet` is installed but not yet on this shell's PATH:"
    say ""
    say "  . \"\$HOME/.cargo/env\"        # this shell, right now"
    say ""
    say "New terminals pick it up on their own (rustup added it to your profile)."
    say "Installed at: $TACET"
fi

# --- 5 · First chat ----------------------------------------------------------
if [ "$TIGHT" -eq 1 ]; then
    say ""
    say "not starting the chat: this machine is short on memory (see the warning above)."
    say "when there is room:  tacet chat"
elif [ -r /dev/tty ] && [ -w /dev/tty ]; then
    say ""
    say "starting tacet — type away, /quit leaves."
    say ""
    # NOT `exec`. Replacing this process means that when the kernel kills the
    # model for memory, the reader gets the shell's bare "Killed" and nothing
    # else. Running it as a child lets that death be explained.
    set +e
    "$TACET" chat < /dev/tty
    STATUS=$?
    set -e
    # 137 = 128 + SIGKILL(9): on Linux that is almost always the OOM killer.
    if [ "$STATUS" -eq 137 ]; then
        say ""
        warn "tacet was KILLED by the operating system — this is a memory problem, not a crash."
        warn "The model needs ~${NEED_MB} MB and this machine could not spare it."
        warn "Options: close what else is running, add swap, or use a smaller model:"
        warn "  tacet models download qwen2.5-3b && tacet chat --model qwen2.5-3b"
        exit "$STATUS"
    elif [ "$STATUS" -ne 0 ]; then
        exit "$STATUS"
    fi
else
    say ""
    say "start talking with:"
    say "  tacet chat"
fi

say ""
say "everything else: see the Tacet website"
