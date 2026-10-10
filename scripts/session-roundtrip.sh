#!/usr/bin/env bash
# THE CLAUDE CODE WRITE-BACK EXPERIMENT, AS A SCRIPT A PERSON RUNS (RG-29).
#
# WHY THIS EXISTS. Tacet's version gate (crates/tacet-tools/src/sessions/
# version.rs) lets a note be appended to a Claude Code transcript only on a
# version where that was MEASURED to work — and the measurement was five
# commands typed by hand on 9 Oct 2026, against Claude Code 2.1.285:
#
#   E1  a throwaway headless session lands in projects/<enc>/<id>.jsonl
#   E2  a user+assistant pair chained to the leaf is loaded by --resume — and
#       the model then believes it wrote the assistant text itself
#   E3  ONE user-role "[Note from Tacet …]" entry is loaded and attributed to
#       Tacet: the only format Tacet writes
#   E4  a note with the WRONG parentUuid makes --resume drop every earlier
#       message without a word: the risk the post-write chain check exists for
#   E5  --resume --fork-session gives a new id and leaves the original file
#       byte-identical
#
# Claude Code's own docs call the transcript format "internal … changes
# between versions", and 17 versions wrote transcripts on one machine in one
# month. So when Claude Code jumps a version, this is re-run, and the write
# range in version.rs (and the ADR's table) moves only if all five come out the
# way they did on 9 Oct.
#
# IT COSTS MONEY. Every step after E1 is a real model turn on your Claude
# account: about $1 in total when measured (E7: $0.59 + $0.39 + $0.02). That is
# why it is a script a person runs and not a test, why CI never runs it, and
# why it asks before the first call.
#
# WHAT IT TOUCHES. Two new, empty directories under $TMPDIR, used as the
# working directories of two new sessions, and the two project directories
# Claude Code creates for them under $CLAUDE_CONFIG_DIR (or ~/.claude)/
# projects/. It refuses to start if either project directory already exists,
# it appends only to transcripts inside them, and at the end it deletes them
# (keep them with --keep). No existing session is read or written.
#
# USAGE
#   scripts/session-roundtrip.sh          # shows the cost and asks first
#   scripts/session-roundtrip.sh --yes    # you have already said yes
#   scripts/session-roundtrip.sh --keep   # leave the throwaway sessions
#   TACET_ROUNDTRIP_CLAUDE=/path/to/claude scripts/session-roundtrip.sh
#
# EXIT CODE: the number of experiments whose outcome differs from 9 Oct 2026
# (0 = the measurement still holds), or 64 if it stopped before spending.

set -u

YES=0
KEEP=0
for arg in "$@"; do
  case "$arg" in
    --yes) YES=1 ;;
    --keep) KEEP=1 ;;
    -h | --help)
      sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "unknown argument: $arg (try --help)" >&2
      exit 64
      ;;
  esac
done

CC="${TACET_ROUNDTRIP_CLAUDE:-claude}"
CFG="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
# The measured run used the default project directory naming; an override
# would file the sessions somewhere this script does not look.
unset CLAUDE_CODE_PROJECT_DIR_NAME
# One model turn measured at well under a minute; five minutes is a hang.
PER_CALL_LIMIT=300

die() {
  echo "session-roundtrip: $*" >&2
  exit 64
}

for tool in jq uuidgen perl; do
  command -v "$tool" >/dev/null 2>&1 || die "needs $tool on PATH"
done
if command -v shasum >/dev/null 2>&1; then
  sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
elif command -v sha256sum >/dev/null 2>&1; then
  sha() { sha256sum "$1" | cut -d' ' -f1; }
else
  die "needs shasum or sha256sum on PATH"
fi
command -v "$CC" >/dev/null 2>&1 || die "Claude Code not found as '$CC' (set TACET_ROUNDTRIP_CLAUDE)"

VERSION="$("$CC" --version 2>/dev/null | head -n 1)"
[ -n "$VERSION" ] || die "'$CC --version' printed nothing"

# Claude Code's project directory for a launch directory: every character
# outside [A-Za-z0-9] becomes '-' (measured: /private/tmp/cc-exp-a was filed
# as -private-tmp-cc-exp-a). Past 200 characters it truncates and hashes,
# which this script does not reproduce — hence the length check below.
enc() { printf '%s' "$1" | LC_ALL=C sed 's/[^A-Za-z0-9]/-/g'; }

# PHYSICAL paths: Claude Code files a session under the resolved directory
# (macOS's /tmp is /private/tmp), so the name is computed from the same.
SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/tacet-roundtrip.XXXXXX")" || die "mktemp failed"
SCRATCH="$(cd "$SCRATCH" && pwd -P)"
mkdir "$SCRATCH/a" "$SCRATCH/b" || die "cannot create the scratch directories"
DIR_A="$SCRATCH/a"
DIR_B="$SCRATCH/b"
PROJ_A="$CFG/projects/$(enc "$DIR_A")"
PROJ_B="$CFG/projects/$(enc "$DIR_B")"
[ "${#DIR_A}" -lt 190 ] || die "scratch path too long for the plain encoding: $DIR_A"
# A project directory that already exists holds sessions this script did not
# make. Refuse rather than share it.
for p in "$PROJ_A" "$PROJ_B"; do
  [ ! -e "$p" ] || die "$p already exists; refusing to touch a project directory this run did not create"
done

cleanup() {
  if [ "$KEEP" = 1 ]; then
    echo "kept: $SCRATCH $PROJ_A $PROJ_B"
    return
  fi
  rm -rf "$SCRATCH"
  # Only what this run created: both names were checked absent above and
  # carry the mktemp suffix, so no other session can live under them.
  case "$PROJ_A" in "$CFG/projects/"*tacet-roundtrip*) rm -rf "$PROJ_A" ;; esac
  case "$PROJ_B" in "$CFG/projects/"*tacet-roundtrip*) rm -rf "$PROJ_B" ;; esac
}
trap cleanup EXIT

confirm_spend() {
  echo "Claude Code: $VERSION"
  echo "This runs 6 headless model turns on your Claude account (about \$1 when"
  echo "measured on 9 Oct 2026) in two new throwaway sessions under:"
  echo "  $DIR_A"
  echo "  $DIR_B"
  [ "$YES" = 1 ] && return 0
  [ -r /dev/tty ] || die "no terminal to ask; pass --yes to agree to the cost"
  printf 'Type yes to spend it: ' >/dev/tty
  local answer
  read -r answer </dev/tty || answer=""
  [ "$answer" = yes ] || die "not confirmed; nothing was spent"
}
confirm_spend

WORK="$SCRATCH/out"
mkdir -p "$WORK"

# run <label> <dir> <claude args…>: one headless turn from <dir>, its JSON in
# $WORK/<label>.json. Prints nothing; the caller reads the fields.
run() {
  local label="$1" dir="$2"
  shift 2
  (cd "$dir" && perl -e 'alarm shift; exec @ARGV or exit 127' "$PER_CALL_LIMIT" \
    "$CC" "$@" >"$WORK/$label.json" 2>"$WORK/$label.err")
}
field() { jq -r ".$2 // empty" "$WORK/$1.json" 2>/dev/null; }

# The append, as the experiment did it (.audit/cc-append-experiment.py): copy
# the session fields from the leaf — the last line carrying a uuid and a
# parentUuid key — add the entry, then a last-prompt line naming the new leaf.
# ONE write, appended. A transcript outside this run's project directories is
# refused before anything is read from it.
inside_scratch() {
  case "$1" in
    "$PROJ_A"/*.jsonl | "$PROJ_B"/*.jsonl) [ -f "$1" ] ;;
    *) return 1 ;;
  esac
}
append_note() {
  local file="$1" mode="$2" text="$3" parent="${4:-}"
  inside_scratch "$file" || die "refusing to append to $file"
  local leaf now u1 u2 content out last
  leaf="$(jq -c 'select(.uuid != null and has("parentUuid"))' "$file" | tail -n 1)"
  [ -n "$leaf" ] || die "no chain entry in $file"
  [ -n "$parent" ] || parent="$(jq -r '.uuid' <<<"$leaf")"
  now="$(date -u +%Y-%m-%dT%H:%M:%S.000Z)"
  u1="$(uuidgen | tr 'A-Z' 'a-z')"
  content="$text"
  [ "$mode" = pair ] && content="[Tacet] progress update follows."
  out="$(jq -c --arg p "$parent" --arg u "$u1" --arg t "$now" --arg c "$content" \
    '{isSidechain, userType, entrypoint, cwd, sessionId, version, gitBranch}
     | with_entries(select(.value != null))
     + {parentUuid: $p, type: "user", uuid: $u, timestamp: $t,
        message: {role: "user", content: $c}}' <<<"$leaf")"
  last="$u1"
  if [ "$mode" = pair ]; then
    u2="$(uuidgen | tr 'A-Z' 'a-z')"
    out="$out
$(jq -c --arg p "$u1" --arg u "$u2" --arg t "$now" --arg c "$text" --arg m "msg_tacet_${u2//-/}" \
      '{isSidechain, userType, entrypoint, cwd, sessionId, version, gitBranch}
       | with_entries(select(.value != null))
       + {parentUuid: $p, type: "assistant", uuid: $u, timestamp: $t,
          message: {id: $m, type: "message", role: "assistant", model: "tacet-local",
                    content: [{type: "text", text: $c}], stop_reason: "end_turn",
                    stop_sequence: null, usage: {input_tokens: 0, output_tokens: 0}}}' <<<"$leaf")"
    last="$u2"
  fi
  out="$out
$(jq -c --arg c "$content" --arg l "$last" \
    '{type: "last-prompt", lastPrompt: $c, leafUuid: $l, sessionId}' <<<"$leaf")"
  printf '%s\n' "$out" >>"$file"
}

NOTE="[Note from Tacet, a local assistant, not from the user]"
WRONG_PARENT="00000000-0000-4000-8000-000000000000"
DIFFER=0
ROWS=()
row() {
  # row <experiment> <same|CHANGED> <what was measured> <what happened>
  ROWS+=("$1|$2|$3|$4")
  [ "$2" = same ] || DIFFER=$((DIFFER + 1))
}
has() { grep -qiF -- "$2" <<<"$1"; }

# ---- E1: a throwaway session lands where the reader looks ----------------
run e1a "$DIR_A" -p "Reply with exactly: READY" --output-format json
A="$(field e1a session_id)"
A_FILE="$PROJ_A/$A.jsonl"
run e1b "$DIR_B" -p "Reply with exactly: READY" --output-format json
B="$(field e1b session_id)"
B_FILE="$PROJ_B/$B.jsonl"
if [ -n "$A" ] && [ -n "$B" ] && inside_scratch "$A_FILE" && inside_scratch "$B_FILE" \
  && has "$(field e1a result)" READY; then
  row E1 same "projects/<enc>/<id>.jsonl, ~23 lines" "$(wc -l <"$A_FILE" | tr -d ' ') lines in $(basename "$PROJ_A")"
else
  row E1 CHANGED "projects/<enc>/<id>.jsonl" "no transcript where expected (see $WORK/e1a.*)"
  echo "E1 failed; the rest depends on it." >&2
fi

if [ -n "$A" ] && [ -n "$B" ] && inside_scratch "$A_FILE" && inside_scratch "$B_FILE"; then
  # ---- E2: an assistant-role entry is loaded as the model's own ----------
  append_note "$A_FILE" pair "[Tacet] While you were away the calendar month rollover bug was fixed. The codeword is PELICAN-42."
  run e2 "$DIR_A" -p --resume "$A" "What codeword was mentioned in this conversation, and who wrote the message that contained it?" --output-format json
  ans="$(field e2 result)"
  if has "$ans" PELICAN-42 && [ "$(field e2 session_id)" = "$A" ]; then
    row E2 same "loaded, same session id; model took the text as its own" "loaded — read the attribution: $ans"
  else
    row E2 CHANGED "loaded, same session id" "$ans"
  fi

  # ---- E3: the one format Tacet writes ------------------------------------
  append_note "$B_FILE" user "$NOTE While you were away Tacet fixed the calendar month rollover bug (audit M17). Codeword: OSPREY-7."
  run e3 "$DIR_B" -p --resume "$B" "What did Tacet report while I was away, and what was the codeword?" --output-format json
  ans="$(field e3 result)"
  if has "$ans" OSPREY-7 && has "$ans" Tacet; then
    row E3 same "loaded and attributed to Tacet" "$ans"
  else
    row E3 CHANGED "loaded and attributed to Tacet" "$ans"
  fi

  # ---- E4: a wrong parent loses the history -------------------------------
  append_note "$A_FILE" user "$NOTE Codeword: HERON-9." "$WRONG_PARENT"
  run e4 "$DIR_A" -p --resume "$A" "List every codeword mentioned in this conversation." --output-format json
  ans="$(field e4 result)"
  if has "$ans" HERON-9 && ! has "$ans" PELICAN-42; then
    row E4 same "only HERON-9: all earlier history silently dropped" "$ans"
  else
    row E4 CHANGED "only HERON-9: all earlier history silently dropped" "$ans"
  fi

  # ---- E5: a fork leaves the original byte-identical ----------------------
  before="$(sha "$B_FILE")"
  run e5 "$DIR_B" -p --resume "$B" --fork-session "Give the codeword from Tacet's note." --output-format json
  after="$(sha "$B_FILE")"
  F="$(field e5 session_id)"
  ans="$(field e5 result)"
  if [ -n "$F" ] && [ "$F" != "$B" ] && [ "$before" = "$after" ] && has "$ans" OSPREY-7; then
    row E5 same "new id, OSPREY-7, original sha256 unchanged" "new id $F, sha256 ${after:0:16} both times"
  else
    row E5 CHANGED "new id, OSPREY-7, original sha256 unchanged" "id '$F', sha256 ${before:0:16} -> ${after:0:16}: $ans"
  fi
else
  for e in E2 E3 E4 E5; do row "$e" CHANGED "(needs E1)" "not run"; done
fi

# ---- E7: what it cost ------------------------------------------------------
# total_cost_usd was cumulative per session when measured, so a session's
# cost is its last call's figure.
cost() { field "$1" total_cost_usd; }
TOTAL="$(printf '%s\n' "$(cost e4)" "$(cost e5)" "$(cost e3)" | awk '{ s += $1 } END { printf "%.2f", s }')"

echo
echo "Claude Code $VERSION — session round-trip, $(date -u +%Y-%m-%dT%H:%MZ)"
echo "measured baseline: Claude Code 2.1.285, 9 Oct 2026"
echo
for r in "${ROWS[@]}"; do
  IFS='|' read -r exp verdict want got <<<"$r"
  printf '%s  %-7s  measured: %s\n           now:      %s\n' "$exp" "$verdict" "$want" "$got"
done
echo
echo "cost reported by Claude Code: \$$TOTAL (measured run: about \$1)"
if [ "$DIFFER" = 0 ]; then
  echo "All five as measured. $VERSION may join the write range in"
  echo "crates/tacet-tools/src/sessions/version.rs (and the ADR's table) — with this output as the measurement."
else
  echo "$DIFFER experiment(s) differ from the measurement: do NOT widen the write range."
fi
exit "$DIFFER"
