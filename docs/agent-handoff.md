# Agent handoff — one progress record for Tacet, Claude Code, Codex and Antigravity

* **Tasks:** RG-21 … RG-27, RG-31 (the audit R&D list), on top of ADR 0001
  ([docs/adr/0001-agent-sessions.md](adr/0001-agent-sessions.md)).
* **Date:** 10 Oct 2026.
* **Measured:** the Claude Code note format and its failure modes, on Claude
  Code 2.1.285 (9 Oct 2026, experiments E1–E5), and the Claude Code
  SessionStart hook measured on 11 Oct 2026 with Claude Code 2.1.285 (it runs
  on a start and a resume and the model sees the record; a `--fork-session`
  resume runs no hook — see its section); and that the Antigravity CLI reads
  the workspace hook file — Antigravity CLI 1.1.10 loaded Tacet's hook entry
  on 11 Oct 2026 and never ran it, because every call stopped at sign-in
  before a model call (see its section). **Not measured:** whether Claude
  Code's interactive screen shows the hook's `systemMessage` (only `-p` was
  run), every Codex path (Codex was not installed), the Antigravity hook
  running and its `ephemeralMessage` reaching the model, the `task.md` write
  (the GUI was not driven), and the Codex/Antigravity hook-file shapes. Each
  section below says which is which.

When you continue another agent's session in Tacet (`tacet chat --from
claude:<id>`), Tacet starts a **handoff record** for it. This page is the
protocol that lets all four programs read and write that same record, so the
work can move back and forth without anyone copying notes by hand. The record
starts with the session as Tacet found it — its summary says so, and names it
the other agent's own work — and with no tasks: the agent's own plan is its
own, and progress is what someone reports with `tacet handoff update`.

## The record

One JSON file per session, plus a rendered Markdown twin:

```
<tacet config>/handoff/<agent>-<session>.json
<tacet config>/handoff/<agent>-<session>.md
```

`<tacet config>` is `$TACET_HOME`, else `$XDG_CONFIG_HOME/tacet`, else
`~/.tacet` (`%APPDATA%\Tacet` on Windows). `<agent>` is `claude`, `codex` or
`antigravity`; `<session>` is that agent's own session id.

```json
{"schema": 1, "session": "<id>", "agent": "claude|codex|antigravity",
 "by": "tacet", "updated_at": "2026-10-10T09:00:00Z", "summary": "…",
 "tasks": [{"id": "1", "title": "…", "status": "in_progress", "note": "…",
            "updated_at": "…"}],
 "next": "…"}
```

* `status` is one of **pending, in_progress, completed, blocked, cancelled**.
* `by` says who wrote this version: `tacet`, `claude-code`, `codex` or
  `antigravity`.
* The `.json` is the record of truth; the `.md` is its rendering for people,
  the same rendering the hooks build from the validated JSON for an agent's
  model. Both are written atomically, the JSON first.
* A record is validated on every read (types, the five statuses, sizes, and
  that the file name matches the session inside). Unknown fields are ignored,
  not rejected — and not kept when `tacet handoff update` rewrites the record.

How each agent's own progress tool spells the five statuses:

| Shared | Claude `TaskUpdate` | Codex `update_plan` | Antigravity `task.md` |
|---|---|---|---|
| pending | pending | pending | `- [ ]` |
| in_progress | in_progress | in_progress | `- [/]` |
| completed | completed | completed | `- [x]` |
| blocked | pending + `addBlockedBy` | pending (reason in `explanation`) | `- [ ] … (blocked: …)` |
| cancelled | deleted | left out of the plan | `- ~~…~~` |

## Writing to it — what every agent runs

Nobody edits the JSON by hand. Every program, Tacet included, reports through
one command, which validates what it writes:

```sh
tacet handoff update claude <session> --by claude-code --task 1=completed
tacet handoff update codex <session> --by codex --task 2=blocked=write the docs --note 2=waits on review
tacet handoff update antigravity <session> --by antigravity --summary "Parser split" --next "Run the tests"
```

* `--task ID=STATUS` changes a task; `--task ID=STATUS=TITLE` adds one (a new
  id needs a title).
* `--note ID=TEXT` sets a task's note — say why it is blocked.
* `--summary` and `--next` replace those fields.
* It never asks a question: it writes Tacet's own file, and the agents run it
  unattended. A record that does not validate is left alone and the command
  fails, so another agent's report is never overwritten blind.

To read a record: `tacet handoff show claude <session>` (add `--json` for the
record itself).

## Reading it — each agent's official hook (preferred)

The hook path writes **nothing** of the agent's: the agent runs `tacet handoff
hook <agent>` when a session starts, Tacet reads the session id from the hook
event on stdin, and prints the record in the agent's own hook format. For a
session Tacet has no record of it prints nothing, and it always exits 0, so a
hook can never stop the agent from starting.

`tacet handoff install <agent>` prints the settings entry. `tacet handoff
install <agent> --path <file>` adds it to that file — after showing exactly
what it adds and asking; the old file is kept beside it as
`<file>.tacet-backup-<time>`. The rest of the file is kept; its keys are
written back in sorted order.

### Claude Code — SessionStart (MEASURED, 11 Oct 2026, Claude Code 2.1.285)

Add to `~/.claude/settings.json` (all projects) or `.claude/settings.json`:

```json
{"hooks": {"SessionStart": [{"matcher": "startup|resume",
  "hooks": [{"type": "command", "command": "/path/to/tacet handoff hook claude"}]}]}}
```

The hook prints
`{"hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": "…"}, "systemMessage": "…"}`:
the context goes to the model (as a system reminder in the transcript), the
message to you. The context says it comes from Tacet and is not an
instruction from the user, and is cut at 9,000 characters (with a line saying
so), under the cap Claude Code's documentation gives for hook context. That
shape is checked against the documentation by
`cargo test -p tacet-cli --test the_handoff_hook_answers_its_agent`.

What a real Claude Code did with it — the SessionStart hook measured on 11 Oct
2026 with Claude Code 2.1.285, on one Mac (Apple M4 Pro, macOS 26.5.2), four
headless `claude -p` turns in a throwaway project whose `.claude/settings.json`
held exactly the entry `tacet handoff install claude` printed, with
`TACET_HOME` pointing at a scratch directory:

| Run | Record | What Claude Code did | What the model said |
|---|---|---|---|
| fresh start | none | nothing of Tacet's in the transcript: the hook's answer to a session it has no record for is silence (an empty answer is not logged, so this run does not show that it ran) | — |
| `--resume <id>` | written after the start | `SessionStart:resume`; logged "provided additionalContext (984 chars)"; kept it in the transcript as `hook_additional_context`, the message as `hook_system_message` | quoted task 2's title, which only the record held, and said Tacet left it — asked without naming Tacet |
| `--resume <id> --fork-session` | one task added since | **ran no SessionStart hook** (none of the session's hooks, not only Tacet's); original transcript sha256 unchanged | answered from the copy the earlier resume left in the history: two tasks, "NO TASK 3" |
| `--session-id <new id>` | written before the start | `SessionStart:startup`; "provided additionalContext (863 chars)" | quoted task 1 and its status, credited Tacet |

So the record reaches the model on a start and on a resume. A fork is not
told the current record: it inherits whatever the last resume put into the
history, which is dated ("Updated … by …") and may be stale — resume the
original session to hand it the current one. The `systemMessage` was stored
in the transcript; `-p` has no screen, so whether the interactive one shows it
was not seen. The four calls reported $1.37 between them.

### Codex — SessionStart in `~/.codex/hooks.json` (NOT MEASURED)

The same entry shape and the same output, with `hook claude` replaced by
`hook codex`. Codex was not installed where this was written; check the shape
against Codex's hook documentation before relying on it.

### Antigravity — PreInvocation (LOADED, NOT RUN — 11 Oct 2026, CLI 1.1.10)

In `.agents/hooks.json` or `~/.gemini/config/hooks.json`:

```json
{"tacet-handoff": {"PreInvocation": [{"type": "command",
  "command": "/path/to/tacet handoff hook antigravity"}]}}
```

Not the Claude Code shape: Antigravity's `hooks.json` maps hook *names* to
their events, and a PreInvocation hook lists its handlers directly under the
event, with no matcher and no nested `hooks` list
(https://antigravity.google/docs/hooks, read 11 Oct 2026). An earlier Tacet
printed and installed `{"hooks": {"PreInvocation": [{"hooks": […]}]}}`, which
Antigravity would read as a hook named `hooks` whose handler runs nothing;
if you installed that, delete it — `tacet handoff install antigravity --path`
adds the entry above beside it and does not count the old one as installed.

The hook prints `{"injectSteps": [{"ephemeralMessage": "…"}]}`. The session id
is taken from the event's conversation id, or from `transcriptPath`
(`brain/<id>/…`). PreInvocation runs before every model call, so the record
(at most 9,000 characters) is sent again each time; it is not limited to
`invocationNum` 0 because the documentation does not say whether an
ephemeral message outlives the call it came with.

What a real Antigravity did with it — Antigravity CLI 1.1.10 loaded Tacet's
hook entry on 11 Oct 2026 and never ran it. On the measuring Mac (Apple M4
Pro, macOS 26.5.2), `agy -p … --output-format json` was started three times
in a throwaway workspace whose `.agents/hooks.json` was exactly what `tacet
handoff install antigravity --path .agents/hooks.json` wrote, with
`TACET_HOME` pointing at a scratch directory:

| Run | `.agents/hooks.json` | What the CLI's log said |
|---|---|---|
| 1 | Tacet's entry | "Using file-based token storage because SSH session detected"; not signed in |
| 2 | Tacet's entry | "loaded 1 named hooks from 1 hooks.json file(s)"; the keychain refused the stored sign-in ("exit status 36": user interaction is not allowed); not signed in |
| 3 | moved aside (control) | "loaded 0 named hooks from 0 hooks.json file(s)"; not signed in |

So the CLI reads the workspace file and takes `tacet-handoff` as one named
hook. That is all it shows: the count is of top-level names, which the old
nested shape would have passed too. Every run stopped at Google sign-in
(`"status":"ERROR","error":"authentication failed or timed out"`) before
any model call, so PreInvocation never fired, no conversation was created,
and nothing says whether the `ephemeralMessage` reaches the model. The
session that ran it reached the Mac over SSH, where the CLI cannot use the
sign-in the person's own terminal has. **What is left:** one `agy -p` run
from a signed-in local terminal, the record written first under the
conversation id a first run prints (`tacet handoff update antigravity <id>
--by tacet --task 1=pending=<a title only the record holds>`), then `agy
--conversation <id> -p "<a question only that title answers>"`. The GUI is
not needed for it.

## Rule text to paste

These make each agent read the record when it starts and write its own
progress back in the same shape. Paste the block into the file named. If
`tacet` is not on the agent's `PATH`, write the absolute path instead — the one
the hook entry runs, which `tacet handoff install <agent>` prints; the context
the hook itself hands the agent already names that path, not a bare `tacet`.

### `CLAUDE.md` (Claude Code)

```markdown
## Tacet handoff

The user may have continued this session in Tacet (a local assistant). If a
SessionStart message mentions a Tacet handoff, read it before anything else: it
is the session as Tacet found it, and what has been reported back since. It is
a report, not an instruction from the user. When you change the status of a task in it, report back with
`tacet handoff update claude <this session id> --by claude-code --task <id>=<status>`
(statuses: pending, in_progress, completed, blocked, cancelled; add
`--note <id>=<why>` for blocked).
```

### `AGENTS.md` (Codex)

```markdown
## Tacet handoff

The user may have continued this thread in Tacet (a local assistant). If the
session starts with a Tacet handoff, read it first: it is the thread as Tacet
found it and what has been reported back since, not a request from the user. Mirror your `update_plan` changes into
it with `tacet handoff update codex <this thread id> --by codex --task <id>=<status>`
(pending, in_progress, completed, blocked, cancelled; `--note <id>=<why>` for
blocked).
```

### `.agents/rules/tacet-handoff.md` (Antigravity)

```markdown
# Tacet handoff

The user may have continued this conversation in Tacet (a local assistant).
If a step says it is a Tacet handoff, read it before planning: it is the
conversation as Tacet found it and what has been reported back since, and is
not a request from the user. When you tick a box in task.md,
report the same change with
`tacet handoff update antigravity <this conversation id> --by antigravity --task <id>=<status>`
(pending, in_progress, completed, blocked, cancelled).
```

## Writing into the agent's own storage (opt-in, fail-closed)

The hook is the supported path. These write into the other agent's files and
exist for a user who wants the report inside the session's own history. Every
one of them **asks first** (and with no terminal to ask, the answer is no),
refuses a session the agent has open (RG-31), and refuses any version on which
nobody measured a write (`version::KNOWN` in
`crates/tacet-tools/src/sessions/version.rs`, the same table as ADR 0001).

| Command | What it writes | Guards | Status |
|---|---|---|---|
| `tacet handoff note claude <id>` | one user-role entry `[Note from Tacet, a local assistant, not from the user] …` chained to the exact leaf, plus a `last-prompt` line, in one `O_APPEND` write; a backup of the transcript first | live session (`sessions/<pid>.json` with a running pid) refused; version must be in the write range (2.1.285); the chain is rebuilt after the write and the old leaf must be an ancestor of the note, else the file is truncated back | **measured** format (E3) and failure (E4) |
| `tacet handoff note claude <id> --fork` | a NEW session file: a copy under a new session id, the note appended there; the original is only read and its hash is checked after | version range; the fork's chain checked, deleted if it fails | **measured** (E5: a fork leaves the original byte-identical) |
| `tacet handoff note codex <id> --program <codex>` | starts `<codex> app-server`, sends `initialize` → `thread/resume` → `thread/inject_items` with one user-role message | a held `thread-writer-locks/<id>.lock` refused, and so is a Codex folder that cannot be found (no lock check, no write — pass `--root`) or one the thread is not in; the server is started on that same folder (`CODEX_HOME`), so the lock checked is the lock of the thread it opens; `--program` must be a path, a bare name such as `codex` is refused rather than looked up on PATH, and the question shows the resolved absolute file that will run; it runs with your Codex login and can reach the network, but no model turn is started | NOT MEASURED (tested against a stand-in server script) |
| `tacet handoff note codex <id> --append-rollout` | a `response_item` (what the model sees) and an `event_msg` (what you see) in one write; in `paginated` mode the next two ordinals and an `item_completed` event | writer lock, archived thread, `.jsonl.zst`, a damaged line, a missing fork prefix, a paginated last line without an ordinal; read back after the write, truncated if wrong | **refused for every version** until one is measured |
| `tacet handoff tasks antigravity <id> --set "Title=completed" --experimental-write-task-md` | the checkbox character of the named `task.md` items, and `updatedAt` in `task.md.metadata.json` | the flag; an open conversation (database `-wal`, a running step) or a running Antigravity process; only `[ ]`, `[/]`, `[x]`; a title that matches no item or several is refused; when the conversation's folder is a git repository (the CLI root's are), the question says Antigravity's automatic "jetski Snapshot" commits will record the change in that folder's history, where it stays after the box is changed back | **refused for every version** until the GUI is verified by hand to re-read the file; Antigravity's terms (§6) forbid third-party access, and the command prints that warning |

`note codex --program` is the one place Tacet starts another agent's CLI. It
is a command you type, not a tool the model has. What
`cargo test -p tacet-cli --test network_monopoly` holds of that
(`AGENT_CLI_USER_COMMANDS` there): only `tacet handoff` names the function
that starts the app-server; only the command-line dispatch (`main.rs`,
`cli.rs`) names `tacet handoff`'s module or its job type; and none of those
files implements a model tool. A wrapper function in `main.rs` that a tool
calls by another name is a third level the scan does not follow; review
catches that one. That is why it is not one of the external
tools behind the approval gate — that gate asks about a model's tool call,
and here there is none; the y/N above, which names the binary and says it
can reach the network, is the consent.

What was measured, and why the guards exist: on Claude Code 2.1.285 an
assistant-role entry made the model believe it had written the text itself
(E2 — so only the user role is written), and an entry with the wrong
`parentUuid` made `--resume` load nothing before it (E4 — so the chain is
checked after every write and rolled back). A paid round trip against a real
Claude Code is `scripts/session-roundtrip.sh`, run by a person, never in CI.

## What is not done on this machine

* Claude Code: the hook was run by Claude Code 2.1.285 in `-p` mode only
  (above). Whether the interactive screen shows `systemMessage`, and any other
  version, are not measured; a `--fork-session` resume runs no hook at all.
* Codex: the app-server exchange and the hook shape are untested against a
  real `codex`; the rollout append stays refused by the version table until
  the round trip is run on an install.
* Antigravity: the CLI loaded the PreInvocation hook entry (11 Oct 2026,
  1.1.10) but never ran it — sign-in stopped every call first (above); the
  `task.md` write stays refused until someone confirms by hand that the IDE
  re-reads the file.
