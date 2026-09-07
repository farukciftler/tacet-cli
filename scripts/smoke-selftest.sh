#!/usr/bin/env bash
# DOES THE SMOKE CHECK ACTUALLY GO RED?
#
# `scripts/smoke.sh` is the thing standing between a release and the four
# failures a user found by installing one. That makes it worth exactly as much
# as its ability to fail — and a check that has only ever been seen green has
# not demonstrated that ability. This repository has shipped a test comparing a
# list against itself and a baseline guard aimed at the wrong layer; both were
# green every day of their lives.
#
# So each case below hands `smoke.sh` an answer that is known to be bad and
# asserts it says so, plus one known-good answer to prove the detectors are not
# simply stuck on "fail". `FakeEngine` supplies the answers, so this runs in
# about a second and needs neither weights nor a GPU — which matters, because
# the machine that can run a model is the one place this is hardest to run.
#
#   scripts/smoke-selftest.sh
#
# Exit code is the number of detectors that did not behave.

set -u

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SMOKE="$REPO/scripts/smoke.sh"
[ -x "$SMOKE" ] || { echo "not executable: $SMOKE" >&2; exit 127; }
[ -x "$REPO/target/release/tacet" ] || {
  echo "build first: cargo build --release -p tacet-cli" >&2; exit 127; }

# Six messages go through smoke.sh and FakeEngine answers all of them the same
# way, so a detector that fires fires six times. "clean" is the only row that
# should come back zero.
TOTAL=6

# name | the scripted answer | expected failures
run_case() {
  local name="$1" script="$2" want="$3" got out
  out="$(TACET_SMOKE_SCRIPT="$script" "$SMOKE" 2>&1)"
  got=$?
  if [ "$got" = "$want" ]; then
    printf 'ok    %-24s  %d of %d flagged\n' "$name" "$got" "$TOTAL"
    return 0
  fi
  printf 'FAIL  %-24s  flagged %d, expected %d\n' "$name" "$got" "$want"
  printf '%s\n' "$out" | sed 's/^/      /'
  return 1
}

bad=0

# A GOOD ANSWER MUST PASS. Without this row every other row could be satisfied
# by a script that always fails, which would be worse than no check at all: it
# would block every release for a reason nobody could act on.
run_case clean "Twenty-six times eighteen is 468." 0 || bad=$((bad + 1))

# THE FOUR THINGS THAT ACTUALLY REACHED A USER'S SCREEN.
# `<unused6088>` twelve times over, from a vocabulary the sampler could name
# and the tokenizer could not.
run_case marker-unused "The answer is <unused6088> okay" "$TOTAL" || bad=$((bad + 1))
# `<tool.` — a call marker that decayed into prose mid-sentence.
run_case marker-tool "I will use <tool. to find out" "$TOTAL" || bad=$((bad + 1))
# `to call to call to call` — the degenerate loop. Five repeats is the
# threshold, so this row also pins the threshold itself.
run_case repetition \
  "do not call to call to call to call to call to call to call" "$TOTAL" || bad=$((bad + 1))
# Nothing at all: the turn ran and produced no text to read.
run_case empty "" "$TOTAL" || bad=$((bad + 1))
# `1033336333333333333` — what gemma3-4b answered when asked what day it is.
# It slipped past the word counter on the first real run, which is the only
# reason this detector exists.
run_case digit-loop "Today is 1033336333333333333." "$TOTAL" || bad=$((bad + 1))

# AND THE SHAPE THAT MUST NOT FIRE. Prose legitimately repeats short runs —
# a list, a refrain, a restated question. Four repeats stays quiet; the row
# above proves five does not. Without this the threshold would be free to
# drift down to zero and nobody would notice until releases stopped.
run_case under-threshold \
  "one two three one two three one two three one two three" 0 || bad=$((bad + 1))

# A LONG RUN THAT IS NOT A LOOP. Markdown rules and dividers are ordinary
# prose; the character detector is restricted to alphanumerics for exactly this
# reason, and only this row keeps that restriction from being quietly dropped.
run_case rule-not-a-loop \
  "The total is 468.
--------------------
Nothing else to add." 0 || bad=$((bad + 1))

echo
if [ "$bad" = 0 ]; then
  echo "the smoke check goes red on every failure it claims and stays green otherwise"
else
  echo "$bad detector(s) did not behave — smoke.sh cannot gate a release"
fi
exit "$bad"
