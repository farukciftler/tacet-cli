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
#   * RESUMABLE. `--journal` keeps every finished case, keyed by the commit,
#     the uncommitted diff, the model and the features — never by the date. A
#     closed lid at case 180 costs one case, not the 47 minutes behind it, and
#     that stays true across midnight. Running this script again continues;
#     running it twice in a row is a no-op that re-reports.
#   * LOGGED. Everything the run prints goes to `run.log` in that directory,
#     each attempt headed by its start time, the commit, the machine and the
#     model, so a number found later can be traced to what produced it.
#   * LIVE. The eval's own progress line ("[103/184] case · 31m elapsed · ~20m
#     left") is in the log as it happens; `tail -f` is the live view.
#   * IT REFUSES TO MIX. Different code gets a different directory (below), and
#     a journal stamped for a different model or catalog stops the run (see
#     `CaseJournal::open`), so a report can never be half one build and half
#     another.
#
# USAGE
#   scripts/nightly-eval.sh                 # tonight's run, resuming if it can
#   scripts/nightly-eval.sh --model qwen2.5-3b
#   scripts/nightly-eval.sh --print-run-dir # where it would go; touches nothing
#   tail -f ~/.tacet/nightly/latest.log     # the live view
#
# The exit code is the eval's, so cron mail says whether it passed.

set -u -o pipefail

MODEL="qwen3-4b"
FEATURES="metal"
case "$(uname -s)" in
  Linux) FEATURES="cuda" ;;
esac
PRINT_RUN_DIR=0
while [ $# -gt 0 ]; do
  case "$1" in
    --model) MODEL="$2"; shift 2 ;;
    --features) FEATURES="$2"; shift 2 ;;
    --print-run-dir) PRINT_RUN_DIR=1; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO" || exit 1

# THE DIRECTORY NAME IS THE RESUME KEY, and it names exactly what was built:
# the commit, the uncommitted changes if there are any, the model and the
# features. Two runs of the same code share a journal and continue each other;
# different code starts a new one, because the cases are not comparable across
# a code change.
#
# THIS DIRECTORY IS WHAT KEEPS TWO BUILDS APART, not the journal. The journal's
# own stamp (`CaseJournal::open`) refuses a different model, engine or catalog;
# it is not relied on here to tell one commit from another, so the key has to.
#
# NO DATE IN IT. It used to be `<yyyymmdd>-<commit>`, so a run interrupted at
# 02:00 and re-run after midnight — the common case for a job that starts at
# 03:07 and takes most of an hour — opened a fresh directory and started from
# case 1, while the header above promised it would continue. The log carries
# the start time of every attempt instead.
#
# `-dirty` CARRIES A HASH OF THE DIFF. A bare `-dirty` flag made every
# uncommitted state of one commit share a journal, so a run could be half one
# edit and half the next. The hash covers tracked changes against HEAD and the
# content of untracked, non-ignored files (a new module is an untracked file),
# and it is computed without writing anything into the repository.
COMMIT="$(git rev-parse --short HEAD)"
working_copy_hash() {
  {
    git diff HEAD --binary
    git ls-files --others --exclude-standard -z | while IFS= read -r -d '' f; do
      printf '%s\0' "$f"
      git hash-object -- "$f"
    done
  } | git hash-object --stdin | cut -c1-12
}
DIRTY=""
if ! git diff HEAD --quiet || [ -n "$(git ls-files --others --exclude-standard)" ]; then
  DIRTY="-dirty-$(working_copy_hash)"
fi
OUT="$HOME/.tacet/nightly"
RUN="$OUT/$COMMIT$DIRTY-${MODEL//\//_}-$FEATURES"

# `--print-run-dir` answers "where would tonight's run go" and stops before
# touching anything — no directory, no build, no model. It exists so the resume
# key can be tested by running the script instead of by reading it:
# `the_journal_directory_survives_midnight_and_tells_edits_apart`.
if [ "$PRINT_RUN_DIR" = 1 ]; then
  echo "$RUN"
  exit 0
fi
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
