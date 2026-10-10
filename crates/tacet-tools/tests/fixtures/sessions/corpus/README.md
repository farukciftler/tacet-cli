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

Every line is **synthetic**: written for the corpus in the measured *shape*
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

## Adding a real session

No real session is in the corpus yet: capturing one means reading somebody's
`~/.claude`, `~/.codex` or `~/.gemini`, which only that person can agree to.
The steps, for whoever does:

1. Capture each file of the session through the anonymiser, and let it file
   the result:

   ```
   cargo run -p tacet-tools --example corpus_capture -- --into crates/tacet-tools/tests/fixtures/sessions/corpus <input>
   ```

   `--into` derives `<agent>/<version>/…` from what the captured file
   records (`sessions::corpus::place`): Claude Code's `version` and its
   *masked* working directory, Codex's `cli_version` and the rollout's date,
   Antigravity's root and conversation id from the input path. Antigravity
   records no version, so its files need `--version <v>` as well (the
   product's About box). A version that is not dotted numbers, or that the
   file contradicts, is refused. Typing the output path yourself
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
