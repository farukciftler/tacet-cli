# The session corpus (RG-28)

One tree per agent, one directory per version under it, read by all three
session readers in `tests/sessions_corpus.rs`. The version table in
`src/sessions/version.rs` points here: a read range is widened only after the
corpus holds that version and parses it with no unparsed line.

```
claude/<version>/projects/<enc>/<id>.jsonl          a $CLAUDE_CONFIG_DIR
codex/<cli_version>/sessions/YYYY/MM/DD/rollout-*   a $CODEX_HOME
antigravity/<version>/<root>/brain|conversations     copied to ~/.gemini/<root>
```

## What is in it today, and where it came from

Six Claude Code transcripts are **real captures** (next section). Every other
line is **synthetic**: written for the corpus in the measured *shape*
(the field names, the line types, the order quirks the 9 Oct 2026 measurement
counted), never copied out of a real session. The Claude Code versions are the
two ends of the measured read range and the one version a write was measured
on, plus one version outside the range that must read with a warning — and
those four are ONE file, copied with the version string and session id
edited (`VERSION_EDITED_COPIES` in the test names them, and fails if that
stops being true). They show the gate and the reader accept each version's
string; they are not a capture of each version's format. The test can require
a fixture for every end of a range, not a measurement; the measurement is the
`measured` text in `version.rs`. Antigravity files record no version, so an
Antigravity directory's version is its name alone. The
Codex directories are named after the `cli_version` the fixtures carry; Codex
was not installed where they were written, so they follow its source
(openai/codex @ a06545b) and are not a measurement of those releases.

## What is real

Six transcripts written by **Claude Code 2.1.285** (`claude -p`, entrypoint
`claude-desktop`) on 11 Oct 2026, on the measuring Mac (Apple M4 Pro, macOS
26.5.2), in throwaway directories made for this project: every prompt in
them was the run's own, and no transcript was taken from the user's
`~/.claude`. Claude Code still wrote the account's context into them as
attachment lines — its skill and tool listings, the output of the account's
own hooks, the prompt snapshot, the e-mail address and organisation — and
that is masked like everything else (below). All are under
`claude/2.1.285/projects/`, one project directory per working directory the
run used.

| session | run | what it holds |
|---|---|---|
| `8efd26aa-bf4f-4660-9712-f11cb03b9613` | `scripts/session-roundtrip.sh`, session a | E2's assistant-role entry Tacet wrote (model `tacet-local`), then E4's note chained to a parent that is not in the file: the read keeps the note and what followed it, and warns |
| `b35773e3-c3e3-4b5a-94f3-c2d2a449361d` | the same run, session b | E3's note chained to the real leaf, and the answer crediting Tacet |
| `59905b9b-3af4-4285-b3ee-d573d9b01228` | the same run, E5 | `--resume b --fork-session`: b's entries, uuids unchanged, under a new id |
| `c46e3daf-1074-4fbc-956c-3a2638be85a1` | RG-21's hook run, runs 1 and 2 | a start, then a resume that ran Tacet's SessionStart hook (its context is a `hook_additional_context` attachment line) |
| `a75f24d5-c79c-42c4-8417-e7ca4710192b` | RG-21, run 3 | `--resume --fork-session` of the session above: no SessionStart hook ran |
| `2522b459-b16f-4938-b7ca-b01e86dcfc95` | RG-21, run 4 | a start under a record Tacet had already written |

**How.** Each through `corpus_capture --into`, one salt per run (shared by a
run's sessions, so a fork and its origin mask the same words the same way),
drawn from the OS and kept nowhere. **Reviewed** before the commit, against
the source, by listing every string value and key the capture kept verbatim
and every word that survived inside a masked string. Kept verbatim: line
types, uuids, timestamps, the version, the model names (`claude-opus-5-5`,
`tacet-local`), `entrypoint`, `permissionMode`, `stop_reason`, token counts,
costs and durations, and the tool-schema keys of the attachment lines.
Anthropic's own `msg_…` and `req_…` ids were kept verbatim at first (21 of
each) and are now pseudonyms: not secret, but each names a call on a real
account, in a public repository. `Scrub::vendor_id` turns `req_`/`msg_`/
`toolu_`/`srvtoolu_` ids into `<prefix>anon…` of the same length,
deterministically per salt, so entries of one response still share an id;
these six files were brought up to it with `corpus_capture
--vendor-ids-only` (ids only, nothing else moved), and
`the_real_captures_carry_no_raw_vendor_id` fails on a raw one anywhere in
the corpus. Words left in masked
text: markup (`system-reminder`, `command-name`), the words the anonymiser
always keeps (`Users`/`users` and `dev`, for the home rule; `blocked`), Tacet's
own note prefix, and two pseudo-words that happen to spell `two` and `flag`
(a different salt left neither; the source words differ). The e-mail
address, the organisation uuid, the home directory, the user and machine
names and the scratch paths were masked; `grep` for each found none, and
`the_corpus_is_anonymised` passes.

**What they showed that the synthetic files could not.** A real `claude -p`
session is ~280 KB of attachment lines (skill listings, the prompt snapshot,
hook output) by its first answer, so a listing reads only its tail; with no
`ai-title`, the listing named each by its `last-prompt` line and the read by
its first prompt. The two now agree on `last-prompt`
(`claude_sessions.rs`, `an_untitled_session_is_named_the_same_…`). A fork's
`last-prompt` lines carry its origin's first prompt, not the question asked
in the fork. Opus 5.5's thinking blocks are written with empty text, and the
reader leaves them out.

**Still not real.** The 2.1.263 and 2.1.293 ends of the read range and the
2.2.0 case are version-edited copies (`VERSION_EDITED_COPIES`); no captured
Claude Code session has a tool call, a plan, a compaction or a sidechain
(these runs asked for none). **Antigravity:** none — `agy` 1.1.10 stopped at
Google sign-in before any conversation existed (RG-25, 11 Oct 2026), and the
IDE was not driven. **Codex:** none, and none can be made here: Codex is not
installed on the measuring Mac.

## Adding a real session

Capturing one from a person's own `~/.claude`, `~/.codex` or `~/.gemini`
means reading their sessions, which only that person can agree to; a
throwaway run made for the purpose, as above, needs no one's. The steps:

1. Capture each file of the session through the anonymiser, and let it file
   the result:

   ```
   cargo run -p tacet-tools --example corpus_capture -- --into crates/tacet-tools/tests/fixtures/sessions/corpus <input>
   ```

   `--into` derives `<agent>/<version>/…` from what the captured file
   records (`sessions::corpus::place`): Claude Code's `version` and its
   *masked* working directory, Codex's `cli_version` and the rollout's date,
   Antigravity's root and conversation id from the input path — the id held
   to the reader's id shape, and the file under it to one the Antigravity
   reader opens (`task.md`, `implementation_plan.md`, `walkthrough.md`, their
   `.metadata.json`, `.system_generated/logs/transcript*.jsonl`). Antigravity
   records no version, so its files need `--version <v>` as well (the
   product's About box). A version that is not dotted numbers, or that the
   file contradicts, is refused, and so is a file that records two (a
   session resumed after the agent updated itself vouches for neither). A
   Claude Code transcript's name and a subagent's session directory are
   built from the `sessionId` the file records, not from the input path:
   a transcript must be named `<sessionId>.jsonl`, a subagent's
   `agent-<id>.jsonl` must sit in `<sessionId>/subagents/` (its lines record
   the parent's id) and, when its lines record an `agentId`, be named
   `agent-<agentId>.jsonl`; a file whose lines record two sessions or two
   agents is refused.
   It is filed the same way, inside its session's directory. Typing the output path yourself
   (`corpus_capture <input> <output>`) still works, and is how a file with
   no layout of its own (Claude Code's `tasks/<list>/<n>.json`) is captured;
   it is also how the two silent mistakes are made — a transcript left under
   its original project directory, whose name is the unmasked home path,
   still reads by id. `tests/a_capture_is_filed_where_its_reader_looks.rs`
   holds every file here to where `place` would file it.

   It keeps what the readers depend on (line types, ids and parent links,
   timestamps, versions, tool names, task ids and statuses, the markup tags
   the readers split on), and only in that shape: a kept key holding prose,
   an address or a path is masked like any other text, and inside a tool's
   data (a call's input or arguments, a result's content, JSON carried in a
   string, and the payloads no reader opens that hold a tool's answer:
   Codex's `invocation`, `result`, `stdout`, `stderr`, `aggregated_output`
   and `formatted_output`, Claude Code's `progress` `data`) only the statuses
   the readers know, task ids of up to six digits and content-block types
   survive. Object keys are masked too — inside a tool's data every key but
   the few the readers look up there, outside it every key that is not one
   identifier-shaped token — and so is every number in a tool's data but such
   a task id, to one of the same digit count. Outside a tool's data, numbers
   and one-token keys are kept: tool output under some other envelope would
   keep them too, which is another reason for step 2. Any other
   tag keeps its brackets and loses its name. Every other string is masked
   word by word; home directories become `dev`, the capturing user's name
   becomes `dev` in any case, and e-mail addresses are removed. An identifier-shaped value under a kept key outside a tool's data
   (`"source":"AcmeCorp"`) is kept, which is one reason for step 2. The first run prints a salt: pass it with
   `--salt` to the other files of the same session so a path is masked the
   same way in all of them, and commit it nowhere. For a Claude Code
   transcript it prints the `projects/<dir>/` the masked working directory
   belongs under. `.db` and `.jsonl.zst` are refused.
   `tests/a_captured_session_keeps_its_shape.rs` holds the anonymiser to
   this: every fixture here, captured, reads the same with every reader.
2. Read the output before committing it. The corpus test fails on the
   runner's own `$HOME` and user name, on an e-mail address and on the common
   credential prefixes, but it cannot know what else is private.
3. Add a `Case` for it in `tests/sessions_corpus.rs` with the entries the
   reader must produce. A file under this directory that no case names fails
   the test, so a fixture cannot sit here unchecked.
