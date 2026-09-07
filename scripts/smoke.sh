#!/usr/bin/env bash
# WHAT A RELEASE LOOKS LIKE TO SOMEBODY TYPING INTO IT.
#
# WHY THIS EXISTS, IN THE WORDS THAT PROVOKED IT: "bunları sen test edemez misin
# publish almadan" — can you not test these before publishing. Twice in one day a
# release went out and the person who installed it was the one who found what it
# did: `<unused6088>` twelve times and an engine error, then a wall of
# `to call to call to call`. Both were visible in one message against a model
# sitting on the same disk.
#
# `cargo test` cannot see any of this. It runs against `FakeEngine`, whose
# answers are scripted, so every failure below is invisible to it BY
# CONSTRUCTION. `eval --tool-selection` does run a model, takes fifty minutes,
# and scores WHICH TOOL was called — not whether the sentence the user reads is
# a sentence.
#
# So this is the cheap middle: a handful of real messages through a real model,
# and the answers checked for the four things that have actually gone wrong.
#
#   1. an EMPTY answer            — the turn produced nothing to read
#   2. an ENGINE ERROR            — it died
#   3. a MARKER FRAGMENT          — `<unused6088>`, `<tool.`, `<|im_`: vocabulary
#                                   leaking into prose
#   4. DEGENERATE REPETITION      — the same short phrase over and over
#
# None of these is a judgement about the model's intelligence. They are all
# "this is not text a person can read", which is the harness's job.
#
# USAGE
#   scripts/smoke.sh                       # the model the catalog pins
#   scripts/smoke.sh ~/models/gemma3-4b    # any folder with model.gguf
#
# The exit code is the number of messages that failed, so it gates a release.

set -u

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO" || exit 1
export PATH="$HOME/.cargo/bin:$PATH"

MODEL_DIR="${1:-}"
BIN="$REPO/target/release/tacet"
[ -x "$BIN" ] || { echo "build first: cargo build --release -p tacet-cli --features metal" >&2; exit 127; }

if [ -n "$MODEL_DIR" ]; then
  export TACET_MODEL="$MODEL_DIR/model.gguf"
  [ -f "$MODEL_DIR/tokenizer.json" ] && export TACET_TOKENIZER="$MODEL_DIR/tokenizer.json"
  [ -f "$TACET_MODEL" ] || { echo "no model.gguf in $MODEL_DIR" >&2; exit 127; }
fi

# HOW THIS CHECK IS ITSELF CHECKED.
#
# A guard nobody has watched go red is not a guard. This one cannot be trusted
# on a green run alone: four detectors below all agree on "fine" when they are
# broken, silent, or reading the wrong stream, and the only proof they work is
# to hand them a known-bad answer and watch them catch it.
#
# `TACET_SMOKE_SCRIPT` does exactly that. It swaps the model for `FakeEngine`,
# which returns the given text for every message and NEEDS NO WEIGHTS — so
# `scripts/smoke-selftest.sh` can drive all four failure modes in about a
# second, on any machine, with no GPU. It is a testing seam, never a release
# path: with it set the run says nothing about a model.
#
# `+x`, NOT `:-`: the seam is on when the variable is SET, empty or not. The
# empty answer is one of the four failures being tested, so an emptiness test
# here would send exactly that case to the real model — which is how this was
# first written, and it silently turned a detector self-test into an
# unannounced model run.
ENGINE_ARGS=()
if [ -n "${TACET_SMOKE_SCRIPT+x}" ]; then
  ENGINE_ARGS=(--engine fake --script "$TACET_SMOKE_SCRIPT")
fi

# A CLEAN WORKING DIRECTORY, because the shell puts a listing of it in every
# prompt and this repository's own tree would make the check depend on itself.
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
printf 'Monday, Lentils\nTuesday, Rice\n' > "$WORK/meals.md"

# Two languages and the three shapes a turn can take: a tool that is on the
# device, a tool that is not, and a question needing no tool at all.
MESSAGES=(
  "what is 26 times 18"
  "yirmi altı çarpı on sekiz"
  "what day is it today"
  "ortaköy üsküdar vapur saatleri"
  "hello, how are you"
  "read meals.md and tell me what is on Tuesday"
)

# ONE MESSAGE AT A TIME, AND NEVER FOREVER.
#
# Both rules were bought. Three of these were once started at once against one
# GPU, each loading the same 2.5 GB of weights, and the machine spent twenty-one
# minutes of wall clock on seventeen seconds of CPU — every process in `U`,
# uninterruptible sleep, waiting on memory the others held. So: serial, and with
# a ceiling, because a check that can hang cannot gate anything. `timeout(1)` is
# not on macOS; perl's alarm is everywhere perl is.
#
# THE TWO STREAMS STAY APART. `--json` promises "one line of JSON per turn on
# stdout, and NOTHING ELSE"; it keeps that promise, and the MCP load notice —
# `(mcp: 29 remote tools added to the catalog)` — goes to stderr where it
# belongs. An early draft of this script merged them with `2>&1`, which fed
# that line to `jq`, blew up the parse, and turned a healthy answer into a
# reported failure. Diagnostics that lie in the FAILING direction are the ones
# that get the check switched off.
PER_MESSAGE_LIMIT="${TACET_SMOKE_TIMEOUT:-180}"
run_message() {
  perl -e 'alarm shift; exec @ARGV or exit 127' "$PER_MESSAGE_LIMIT" \
    "$BIN" chat --message "$1" --dir "$WORK" --json \
    ${ENGINE_ARGS[@]+"${ENGINE_ARGS[@]}"} \
    >"$WORK/out.json" 2>"$WORK/out.err"
}

fails=0
for m in "${MESSAGES[@]}"; do
  run_message "$m"
  rc=$?
  # THE ANSWER COMES FROM `--json`, NOT FROM THE SCREEN. The help text for that
  # flag says why in the CLI's own words: a caller that wants a machine answer
  # should not have to guess at the "Tacet: " prefix. Scraping the banner is how
  # a passing check would quietly start reading nothing at all.
  answer="$(jq -rs 'map(select(type=="object" and has("answer"))) | last | .answer // ""' \
    <"$WORK/out.json" 2>/dev/null)"
  problem=""

  [ "$rc" = 142 ] && problem="no answer in ${PER_MESSAGE_LIMIT}s"
  case "$(cat "$WORK/out.err")" in
    *"engine error"*) problem="engine error" ;;
  esac
  [ -z "$problem" ] && [ -z "$answer" ] && problem="no answer"
  if [ -z "$problem" ]; then
    case "$answer" in
      *"<unused"*|*"<|im_"*|*"<tool"*|*"</tool"*|*"<start_of_turn"*|*"<end_of_turn"*)
        problem="a marker leaked into the answer" ;;
    esac
  fi
  # REPETITION, counted rather than eyeballed: the most frequent 3-word run.
  if [ -z "$problem" ]; then
    worst="$(printf '%s' "$answer" | tr 'A-Z' 'a-z' | tr -cs "a-z0-9" '\n' \
      | awk 'NF{w[NR]=$0} END{for(i=1;i+2<=NR;i++){k=w[i]" "w[i+1]" "w[i+2];c[k]++} m=0; for(k in c) if(c[k]>m) m=c[k]; print m+0}')"
    [ "${worst:-0}" -ge 5 ] && problem="the same three words $worst times — degenerate repetition"
  fi
  # THE SAME COLLAPSE, ONE LEVEL DOWN. The word counter above needs words; the
  # first run of this check let `1033336333333333333` through as the answer to
  # "what day is it today", because a single digit stuck in a loop is not three
  # repeated words. Alphanumerics only, deliberately: a markdown rule (`-----`)
  # and a divider (`=====`) are real prose and must not fire.
  if [ -z "$problem" ]; then
    # A BACKREFERENCE, so this is BRE — `grep -E` has no `\1` and would match
    # nothing at all, silently.
    run="$(printf '%s' "$answer" | grep -o '\([0-9A-Za-z]\)\1\{9,\}' | head -1)"
    [ -n "$run" ] && problem="'${run:0:1}' repeated ${#run} times — a character stuck in a loop"
  fi

  if [ -n "$problem" ]; then
    fails=$((fails + 1))
    printf 'FAIL  %-46s  %s\n' "$m" "$problem"
    printf '      %s\n' "$(printf '%s' "$answer" | head -c 160)"
  else
    printf 'ok    %-46s  %s\n' "$m" "$(printf '%s' "$answer" | head -c 60)"
  fi
done

echo
echo "$fails of ${#MESSAGES[@]} messages produced something a person cannot read"
exit "$fails"
