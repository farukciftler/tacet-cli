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

1. Copy the session into the right `<agent>/<version>/` directory.
2. Anonymise it: every path under `/Users/dev/` or `/home/dev/`, no user
   name, no e-mail address, no token, no prompt text you would not publish.
   The corpus test fails on the runner's own `$HOME` and user name, on an
   e-mail address and on the common credential prefixes, but it cannot know
   what else is private — read the file before committing it.
3. Add a `Case` for it in `tests/sessions_corpus.rs` with the entries the
   reader must produce. A file under this directory that no case names fails
   the test, so a fixture cannot sit here unchecked.
