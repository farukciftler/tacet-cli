#!/usr/bin/env bash
# THE MODEL MEASUREMENT, RUN LOCALLY AND OVERNIGHT.
#
# WHY IT IS A SCRIPT IN THE REPOSITORY AND NOT A CI JOB.
# `.github/workflows/nightly.yml` explains at length why the model half cannot
# run on a hosted runner — the compute is one to two orders of magnitude off,
# and a number from hardware that changes under you makes `--compare` report a
# runner swap as a regression. Its conclusion was "a documented LOCAL command
# plus a checked-in baseline". This is that command, written down so it is the
# same command every night rather than whatever was in the shell history.
#
# WHAT IT GUARANTEES:
#   * RESUMABLE. `--journal` keeps every finished case, keyed by the commit. A
#     closed lid at case 180 costs one case, not the 47 minutes behind it.
#     Running this script again continues; running it twice in a row is a no-op
#     that re-reports.
#   * LOGGED. Everything the run prints goes to a dated file, with the commit,
#     the machine and the model fingerprint at the top, so a number found later
#     can be traced to what produced it.
#   * LIVE. The eval's own progress line ("[103/184] case · 31m elapsed · ~20m
#     left") is in the log as it happens; `tail -f` is the live view.
#   * IT REFUSES TO MIX. A journal stamped for a different model or catalog
#     stops the run (see `CaseJournal::open`), so a report can never be half one
#     build and half another.
#
# USAGE
#   scripts/nightly-eval.sh                 # tonight's run, resuming if it can
#   scripts/nightly-eval.sh --model qwen2.5-3b
#   tail -f ~/.tacet/nightly/latest.log     # the live view
#
# The exit code is the eval's, so cron mail says whether it passed.

set -u -o pipefail

MODEL="qwen3-4b"
FEATURES="metal"
case "$(uname -s)" in
  Linux) FEATURES="cuda" ;;
esac
while [ $# -gt 0 ]; do
  case "$1" in
    --model) MODEL="$2"; shift 2 ;;
    --features) FEATURES="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO" || exit 1

# THE COMMIT IS PART OF THE DIRECTORY NAME, and that is the resume key. Two
# runs of the same commit share a journal and continue each other; a new commit
# starts a new one, because the cases are not comparable across a code change
# and the journal's own stamp would refuse them anyway.
COMMIT="$(git rev-parse --short HEAD)"
DIRTY=""
if ! git diff --quiet || ! git diff --cached --quiet; then DIRTY="-dirty"; fi
STAMP="$(date +%Y%m%d)"
OUT="$HOME/.tacet/nightly"
RUN="$OUT/$STAMP-$COMMIT$DIRTY"
mkdir -p "$RUN"

LOG="$RUN/run.log"
ln -sfn "$LOG" "$OUT/latest.log"
ln -sfn "$RUN" "$OUT/latest"

# `cargo` IS NOT ON A NON-INTERACTIVE PATH. This has already cost one whole
# measurement: the rebuild was skipped with `cargo: command not found` on a
# line nobody read, and the "after" run measured the old binary.
export PATH="$HOME/.cargo/bin:$PATH"
if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is not on PATH; the binary would be stale and the number wrong" >&2
  exit 127
fi

{
  echo "=== tacet nightly eval"
  echo "started   $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "commit    $COMMIT$DIRTY"
  echo "machine   $(uname -srm)"
  echo "model     $MODEL"
  echo "features  $FEATURES"
  echo "journal   $RUN/cases"
  echo
} >> "$LOG"

# THE BUILD IS PART OF THE MEASUREMENT. A run against a stale binary is worse
# than no run: it produces a number attributed to a commit it did not measure.
echo "--- building" >> "$LOG"
if ! cargo build --release -p tacet-cli --features "$FEATURES" >> "$LOG" 2>&1; then
  echo "the build failed; nothing was measured" >> "$LOG"
  exit 1
fi

echo "--- running (resumable; $(ls "$RUN/cases" 2>/dev/null | grep -c '\.json$') files already there)" >> "$LOG"
./target/release/tacet eval --tool-selection --model "$MODEL" --json \
  --journal "$RUN/cases" \
  > "$RUN/report.json" 2>> "$LOG"
CODE=$?
echo "--- eval exited $CODE at $(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$LOG"

# A REPORT OF ZERO BYTES IS NOT A REPORT. The eval writes it in one go at the
# end, so an interrupted run leaves an empty file — and the comparison below
# would read it as a run in which nothing passed.
if [ ! -s "$RUN/report.json" ]; then
  echo "no report was written; the run did not finish. Re-run this script to continue." >> "$LOG"
  exit "$CODE"
fi

# THE COMPARISON IS THE POINT. A number with nothing to pair against is a
# number nobody can act on; `--compare` runs a sign test and a paired bootstrap
# and says whether the difference is real.
BASE="crates/tacet-eval/baselines/qwen3-4b-both.json"
if [ -f "$BASE" ]; then
  echo >> "$LOG"
  echo "--- against $BASE" >> "$LOG"
  ./target/release/tacet eval --compare "$BASE" "$RUN/report.json" >> "$LOG" 2>&1
fi

echo >> "$LOG"
echo "report  $RUN/report.json" >> "$LOG"
echo "log     $LOG" >> "$LOG"
exit "$CODE"
