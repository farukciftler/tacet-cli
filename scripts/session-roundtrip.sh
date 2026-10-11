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
#       Tacet — asked without naming Tacet, so an answer that echoes the
#       question cannot pass for one that read the note: the only format
#       Tacet writes
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
# IT COSTS MONEY. Every step, E1 included, is a real model turn on your Claude
# account — six turns, since E1 opens two sessions: about $1 in total when
# measured (E7: $0.59 + $0.39 + $0.02). That is why it is a script a person
# runs and not a test, why CI never runs it, and why it asks before the first
# call.
#
# WHAT IT TOUCHES. Two new, empty directories under $TMPDIR, used as the
# working directories of two new sessions, and the two project directories
# Claude Code creates for them under $CLAUDE_CONFIG_DIR (or ~/.claude)/
# projects/. It refuses to start if either project directory already exists,
# it appends only to transcripts inside them, and at the end it deletes them
# (keep them with --keep). No existing session is read or written.
#
# WHAT IT KEEPS. A run that spent money and did not come out as measured —
# any experiment CHANGED, or a stop with no verdict — leaves its scratch
# directory behind and prints its path: the agent's raw JSON and stderr for
# every call, and a copy of every transcript of this run. Those turns were paid
# for, and a CHANGED row whose evidence the cleanup already deleted can only
# be diagnosed by paying for them again. If Claude Code filed a session
# somewhere other than predicted (E1 CHANGED), the script names it by its id
# and leaves it where it is: it deletes only the directories it checked absent
# before the first turn.
#
# USAGE
#   scripts/session-roundtrip.sh          # shows the cost and asks first
#   scripts/session-roundtrip.sh --yes    # you have already said yes
#   scripts/session-roundtrip.sh --keep   # leave the throwaway sessions
#   TACET_ROUNDTRIP_CLAUDE=/path/to/claude scripts/session-roundtrip.sh
#
# EXIT CODE: the number of experiments whose outcome differs from 9 Oct 2026
# (0 = the measurement still holds); 64 if it stopped before the first paid
# turn, so nothing was spent; 65 if it stopped AFTER one — money was spent and
# there is no verdict (no table is printed).

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

# Set by run() before the first paid turn. A stop after it must not read as
# the 64 that means "nothing was spent": a person deciding whether to rerun
# needs to know the first run was paid for.
SPENT=0
die() {
  echo "session-roundtrip: $*" >&2
  if [ "$SPENT" = 1 ]; then
    echo "session-roundtrip: stopped after paid turns had run: money was spent, and there is no verdict" >&2
    exit 65
  fi
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

# This run's transcripts that Claude Code filed outside PROJ_A/PROJ_B; filled
# in after E1, read by cleanup.
STRAY=()
# Set once both project directories were checked absent. Until then a
# directory under either name belongs to somebody else — an earlier --keep
# run whose mktemp suffix came round again — and cleanup must not remove it.
PROJ_OURS=0
cleanup() {
  local status=$?
  if [ "$KEEP" = 1 ] && [ "$PROJ_OURS" = 1 ]; then
    echo "kept: $SCRATCH $PROJ_A $PROJ_B"
    return
  fi
  # Exit 0 is "as measured" and 64 is "nothing spent": nothing to diagnose.
  # Anything else after a paid turn is a CHANGED row or a stop with no
  # verdict, and its evidence is what the money bought.
  if [ "$SPENT" = 1 ] && [ "$status" != 0 ]; then
    mkdir "$WORK/transcripts"
    for p in "$PROJ_A" "$PROJ_B"; do
      [ -d "$p" ] && cp "$p"/*.jsonl "$WORK/transcripts/" 2>/dev/null
    done
    for f in ${STRAY[@]+"${STRAY[@]}"}; do cp "$f" "$WORK/transcripts/"; done
    echo "evidence kept in $WORK (raw JSON and stderr of every call, transcripts/)"
  else
    rm -rf "$SCRATCH"
  fi
  # Only what this run created: both names were checked absent below and
  # carry the mktemp suffix, so no other session can live under them.
  [ "$PROJ_OURS" = 1 ] || return 0
  case "$PROJ_A" in "$CFG/projects/"*tacet-roundtrip*) rm -rf "$PROJ_A" ;; esac
  case "$PROJ_B" in "$CFG/projects/"*tacet-roundtrip*) rm -rf "$PROJ_B" ;; esac
}
# Installed before the first check that can stop the run: a die on the path
# length or on an existing project directory used to leave the mktemp
# directory with its a/ and b/ in $TMPDIR, unnamed (review of 10 Oct 2026).
trap cleanup EXIT

[ "${#DIR_A}" -lt 190 ] || die "scratch path too long for the plain encoding: $DIR_A"
# A project directory that already exists holds sessions this script did not
# make. Refuse rather than share it.
for p in "$PROJ_A" "$PROJ_B"; do
  [ ! -e "$p" ] || die "$p already exists; refusing to touch a project directory this run did not create"
done
PROJ_OURS=1

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
  # Before the call, not after: a turn that times out may still be billed.
  SPENT=1
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

WRONG_PARENT="00000000-0000-4000-8000-000000000000"
DIFFER=0
ROWS=()
row() {
  # row <experiment> <same|CHANGED> <what was measured> <what happened>
  ROWS+=("$1|$2|$3|$4")
  [ "$2" = same ] || DIFFER=$((DIFFER + 1))
}
# ---- answer matchers (the self-test runs this block on its own) ---------
has() { grep -qiF -- "$2" <<<"$1"; }
NOTE="[Note from Tacet, a local assistant, not from the user]"
# judge <claim|tacet|user> <answer>: exit 0 when the answer makes that
# attribution. claim (E2): the model says the appended assistant text was its
# own — the 9 Oct answer said "the update appeared in my previous message".
# tacet (E3): the note is credited to Tacet — "Tacet, a local assistant,
# reported that …" on 9 Oct. user (E3): the note is credited to the person.
#
# WHY NOT A WORD SEARCH. A denial holds the same words as a claim ("it was
# not my previous message", "I said nothing"), and the note's own header
# names Tacet: E3 used to pass on `has "$ans" Tacet`, so "You left this note
# yourself; it only claims to be from Tacet." read as E3 same and printed
# "may join the write range" (review of 10 Oct 2026). So the header quoted
# back is removed first, and an occurrence is discarded when its clause has a
# negation before it, when (for tacet/user) a hedge such as "claims to be"
# comes before it, or when "nothing"/"none"/"no" follows it. A claim is also
# discarded when the same sentence holds a first-person denial ("but I did
# not write it"), and the whole answer is not a claim when it names a third
# party as the writer ("Tacet wrote that message"). E3 same needs a credit to
# Tacet AND no credit to the user. A false CHANGED in either only keeps the
# write range where it is; a false "same" in E3 is what would widen it.
#
# MARKDOWN EMPHASIS IS TEXT FORMATTING, NOT WORDS. The 11 Oct 2026 run's E3
# answer was "The note was from **Tacet**, a local assistant" — a credit — and
# `\bfrom\s+tacet\b` never saw it, because `**` sat between the words: E3 read
# CHANGED on a run where Tacet was credited. So `*`, `_` and `` ` `` are removed
# before anything is matched. Removing them can only join words a marker
# separated; it cannot add a negation, a hedge or a credit that was not there.
judge() {
  perl -e '
    my ($kind, $note) = @ARGV;
    local $/;
    my $t = lc <STDIN>;
    $t =~ s/\xe2\x80\x99/\x27/g;
    $t =~ tr/*_`//d;
    (my $core = lc $note) =~ s/^\[|\]$//g;
    my $q = quotemeta $core;
    $t =~ s/\[?$q\]?/ /g;
    my %pat = (
      claim => qr/(?<![a-z])(?:my (?:own |previous |earlier |last |prior )?(?:message|response|reply|update|answer|turn)|i (?:wrote|mentioned|said|reported|shared|gave|provided|posted|noted|stated))(?![a-z])/,
      tacet => qr/\btacet(?:\s*[,(][^.;]{0,60}?[,)])?\s+(?:has\s+|had\s+)?(?:left|wrote|added|posted|sent|appended|reported|said|noted|mentioned|told)\b|\b(?:left|written|added|posted|sent|appended|reported)\s+by\s+tacet\b|\bfrom\s+tacet\b|\btacet\x27s\s+note\b|\bit was tacet\b/,
      user => qr/\b(?:you|the user)(?:\s+yourself)?\s+(?:have\s+|had\s+)?(?:left|wrote|added|posted|sent|typed|appended|reported)\b|\b(?:left|written|added|posted|sent|typed|appended)\s+by\s+(?:you|yourself|the user)\b|\bfrom\s+(?:you|yourself|the user)\b|\byour(?:\s+own)?\s+note\b|\bit was you\b/,
    );
    my $re = $pat{$kind} or exit 2;
    my $neg = qr/\b(?:not|never|no)\b|n\x27t\b/;
    my $hedge = qr/\b(?:claim\w*|purport\w*|supposedly|allegedly|pretend\w*|signed|labell?ed|marked|says it is|said to be)\b/;
    my $denial = qr/\bi (?:did not|didn\x27t|do not|don\x27t|never|have not|haven\x27t) (?:write|wrote|written|say|said|send|sent|post|posted|author|authored)\b|\bnot (?:mine|me|by me|from me|written by me)\b|\b(?:wasn|isn)\x27t mine\b/;
    exit 1 if $kind eq "claim"
      && $t =~ /\b(?:tacet|someone(?: else)?|somebody(?: else)?|another (?:assistant|agent|tool|process|program|model)|a (?:tool|script|program|third party)|an external \w+|the system)\s+(?:wrote|sent|posted|added|appended|inserted|injected|authored)\b/;
    for my $s (split /(?<=[.!?;])\s+|\n+/, $t) {
      next if $kind eq "claim" && $s =~ $denial;
      while ($s =~ /$re/g) {
        my $before = substr($s, 0, $-[0]);
        my $after = substr($s, $+[0]);
        $before =~ s/.*(?:[,:;(]|\bbut\b|\bthough\b|\balthough\b|\bhowever\b)//s;
        next if $before =~ $neg;
        next if $kind ne "claim" && $before =~ $hedge;
        next if $after =~ /^\s+(?:nothing|none|no)\b/;
        exit 0;
      }
    }
    exit 1;
  ' "$1" "$NOTE" <<<"$2"
}
claims_own() { judge claim "$1"; }
credits_tacet() { judge tacet "$1"; }
credits_user() { judge user "$1"; }
# ---- end of answer matchers ------------------------------------------------

# ---- E1: a throwaway session lands where the reader looks ----------------
run e1a "$DIR_A" -p "Reply with exactly: READY" --output-format json
A="$(field e1a session_id)"
A_FILE="$PROJ_A/$A.jsonl"
run e1b "$DIR_B" -p "Reply with exactly: READY" --output-format json
B="$(field e1b session_id)"
B_FILE="$PROJ_B/$B.jsonl"
if [ -n "$A" ] && [ -n "$B" ] && inside_scratch "$A_FILE" && inside_scratch "$B_FILE" \
  && has "$(field e1a result)" READY && has "$(field e1b result)" READY; then
  row E1 same "projects/<enc>/<id>.jsonl, ~23 lines" "$(wc -l <"$A_FILE" | tr -d ' ') lines in $(basename "$PROJ_A")"
else
  row E1 CHANGED "projects/<enc>/<id>.jsonl" "no transcript where expected, or no READY (see $WORK/e1a.*)"
fi

# A session filed somewhere else is still this run's, and still on the
# person's account: name it, so it can be found and deleted. Only a session id
# shaped like one is looked up — the id goes into a find pattern.
for id in "$A" "$B"; do
  [[ "$id" =~ ^[0-9A-Fa-f-]{36}$ ]] || continue
  while IFS= read -r f; do
    [ -n "$f" ] && ! inside_scratch "$f" && STRAY+=("$f")
  done <<<"$(find "$CFG/projects" -mindepth 2 -maxdepth 2 -name "$id.jsonl" 2>/dev/null)"
done

if [ -n "$A" ] && [ -n "$B" ] && inside_scratch "$A_FILE" && inside_scratch "$B_FILE"; then
  # ---- E2: an assistant-role entry is loaded as the model's own ----------
  append_note "$A_FILE" pair "[Tacet] While you were away the calendar month rollover bug was fixed. The codeword is PELICAN-42."
  run e2 "$DIR_A" -p --resume "$A" "What codeword was mentioned in this conversation, and who wrote the message that contained it?" --output-format json
  ans="$(field e2 result)"
  if has "$ans" PELICAN-42 && [ "$(field e2 session_id)" = "$A" ] && claims_own "$ans"; then
    row E2 same "loaded, same session id; model took the text as its own" "$ans"
  else
    row E2 CHANGED "loaded, same session id; model took the text as its own" "$ans"
  fi

  # ---- E3: the one format Tacet writes ------------------------------------
  append_note "$B_FILE" user "$NOTE While you were away Tacet fixed the calendar month rollover bug (audit M17). Codeword: OSPREY-7."
  # The question must not name Tacet: "Tacet" in the answer is the
  # attribution being measured, and a model that repeats the question would
  # supply it otherwise.
  run e3 "$DIR_B" -p --resume "$B" "Who left a note while I was away, and what was the codeword?" --output-format json
  ans="$(field e3 result)"
  if has "$ans" OSPREY-7 && credits_tacet "$ans" && ! credits_user "$ans"; then
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
  echo "E1 found no transcript where expected; E2-E5 depend on it and were not run." >&2
  for e in E2 E3 E4 E5; do row "$e" CHANGED "(needs E1)" "not run"; done
fi

# ---- E7: what it cost ------------------------------------------------------
# total_cost_usd was cumulative per session when measured, so a session's
# cost is the figure of the LAST call that reported one: A is e4, else e2,
# else e1a; B is e3, else e1b; the fork e5 is a session of its own. Summing
# only e4+e3+e5 printed $0.00 after E1's two billed turns whenever E1 stopped
# the run — a false figure from the script whose point is to account for it.
cost() { field "$1" total_cost_usd; }
last_cost() {
  local l c
  for l in "$@"; do
    c="$(cost "$l")"
    [ -n "$c" ] && { printf '%s\n' "$c"; return; }
  done
}
TOTAL="$(printf '%s\n' "$(last_cost e4 e2 e1a)" "$(last_cost e3 e1b)" "$(cost e5)" | awk '{ s += $1 } END { printf "%.2f", s }')"

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
echo "sessions: a=${A:-(none reported)} b=${B:-(none reported)}"
for f in ${STRAY[@]+"${STRAY[@]}"}; do
  echo "filed outside the predicted project directory, left in place (delete it yourself): $f"
done
if [ "$DIFFER" = 0 ]; then
  echo "All five as measured. $VERSION may join the write range in"
  echo "crates/tacet-tools/src/sessions/version.rs (and the ADR's table) — with this output as the measurement."
else
  echo "$DIFFER experiment(s) differ from the measurement: do NOT widen the write range."
fi
exit "$DIFFER"
