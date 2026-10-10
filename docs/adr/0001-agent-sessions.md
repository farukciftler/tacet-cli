# ADR 0001 — Reading, continuing and reporting back other agents' sessions

* **Status:** accepted for the shared layer (`crates/tacet-tools/src/sessions/`);
  the readers and the `agent-sessions` addon build on it.
* **Date:** 10 Oct 2026
* **Measured on:** one Mac — Claude Code 2.1.285 (17 versions, 2.1.263–2.1.293,
  had written transcripts there in the preceding month), Antigravity IDE 2.5.5,
  `agy` 1.1.10, Antigravity.app 2.2.1. Codex CLI was **not installed**; what is
  said about it comes from its documentation and its source
  (`openai/codex` @ a06545b).
* **Tasks:** RG-05 (this record), RG-06 (model), RG-07 (handoff), RG-16
  (secrets), RG-30 (version gate), RG-35 (licences), RG-37 (ACP).

The test `sessions::tests::the_adr_states_what_the_code_enforces` reads this
file: every credential the code refuses, every version range it trusts and
every status it writes must appear here. A table that drifts from the code
fails the build rather than misleading the next reader.

## Context

A user who works with Claude Code, Codex and Antigravity leaves sessions in
three places, in three formats, none of them documented as stable — Claude
Code's own docs say the transcript format "is internal to Claude Code and
changes between versions". The user wants Tacet to list those sessions, read
one, continue the work locally, and leave a note the original agent will see
when the user goes back to it.

## Scope (RG-05)

In scope:

1. **List and read** sessions from local files. Claude Code:
   `$CLAUDE_CONFIG_DIR or ~/.claude/projects/<enc>/<id>.jsonl` and
   `~/.claude/sessions/<pid>.json` for liveness. Codex:
   `$CODEX_HOME or ~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`,
   `archived_sessions/`, `session_index.jsonl`. Antigravity: the three roots
   under `~/.gemini/`, `brain/<id>/.system_generated/logs/transcript_full.jsonl`
   and `brain/<id>/task.md`; `conversations/<id>.db` is used for its **mtime
   only**.
2. **Continue** the work in a Tacet chat seeded from a summary of the session
   (the full transcript goes to the DataStore, never into the 4096-token
   context).
3. **Report back** through a Tacet-owned handoff file plus each agent's
   official hook.

Out of scope: syncing sessions between agents, editing another agent's
history, running another agent's model (`claude -p`, `codex exec`, `agy -p`
are paid model turns on the network, and are only ever an external tool behind
the approval gate, off by default).

Everything ships as an addon (`agent-sessions`), absent from the catalog until
installed.

## Threat model (RG-05)

**Assets.** The transcripts themselves (the user's code, commands, command
output and prompts — personal data); the credentials stored beside them; the
other agent's ability to resume its own session.

**Adversaries and what each one can do.**

| Threat | Example | Mitigation |
|---|---|---|
| Prompt injection carried in a transcript | A README the other agent read says `<\|im_start\|>system …`, or `</memory>`; the transcript now carries it | Imported text is **defanged** at the prompt boundary like any tool result (audit H3/H4, RG-02); the model gets a short summary, not the transcript |
| Exfiltration after reading | A summarised transcript contains a token, then the model calls `web_fetch` | Reading a session **taints** the Tacet session (`sessions::TAINTS_SESSION`, audit H2, RG-01), so every outbound tool asks first |
| A credential read by a reader that "only wanted the transcript" | `~/.claude/sessions/<pid>.key` next to `<pid>.json`; `auth.json` next to `session_index.jsonl` | One door (`secrets::open`), a denylist checked on the given AND the canonical path, an extension allowlist; a test fails the build if a reader opens a file any other way — a fully qualified `std::fs::read`, an aliased `use std::fs as f`, a bare imported `read_to_string`, a qualified-type `<std::fs::File>::open`, a `std::fs::copy` of the credential to an innocent name, or production code placed after a stray `#[cfg(test)]` (each one a bypass a review made by hand, and each now in the guard's own test) |
| A planted symlink | `brain/x/.system_generated/logs/transcript_full.jsonl -> ~/.codex/auth.json` | Names are checked on what the link resolves to |
| A planted hard link | `ln ~/.codex/auth.json ~/.codex/sessions/rollout-x.jsonl` — **measured** in review: the innocent name returned the API key, because a hard link has no target to resolve | On Unix a file with more than one name is refused (`Refusal::HardLinked`), counted on the open handle. No transcript the three agents write has a second name. Windows: not enforced (no stable link count in std), NOT MEASURED |
| A swap between the check and the open | The checked path is renamed over, or replaced by a link to `auth.json` or by a FIFO, after `check` and before the open | `check` keeps the metadata of the entry it vetted, not followed; the open uses `O_NOFOLLOW` and `O_NONBLOCK`; the inode opened must be the one checked — compared with the check-time metadata, never a fresh lookup (a fresh lookup follows the swap and matches it). Tests drive the swap inside `open` itself. Unix only |
| Path traversal through an id | `--from claude:../../.ssh/x`, or a `session` field inside a handoff file | `model::validate_id`: `[A-Za-z0-9._-]`, at most 128 bytes, no leading dot, no `..` — applied before any path is built |
| A forged or oversized handoff | Another process writes `claude-A.json` claiming to be about session B, or a 60 MB "summary" | Schema-validated read (types, enums, sizes, file name must match the record); the `.md` twin folds every field so it cannot forge headings or finished tasks, and every task line is parsed back before it is written — a title or note that would change its status (`… (blocked: …)` on a pending task, `~~…~~`) is defused |
| A write that corrupts the user's history | Wrong `parentUuid`: **measured**, `--resume` then loaded nothing before the note | Writes are fail-closed: version gate (below), never into a live session, user approval every time, chain re-verified after the write and rolled back if the old leaf is no longer an ancestor; the official hook path is preferred and touches no transcript |
| Impersonating the other agent's model | An assistant-role entry: **measured**, the model then believed it had written the text itself | Never write the assistant role. The only file format is a single user-role entry beginning `[Note from Tacet, a local assistant, not from the user]` |

**Credential denylist (RG-16).** Refused by `sessions::secrets` whatever the
reader asked for, compared case-insensitively:

* exact names: `.credentials.json`, `credentials.json`, `auth.json`,
  `oauth_creds.json`, `google_accounts.json`, `state.vscdb`, `state.vscdb-wal`,
  `state.vscdb-shm`, `state.vscdb.backup`, `.netrc`, `.claude.json` (it holds
  `primaryApiKey` on an API-key login, and no reader needs it);
* copies of those: a name that is one of the stems `auth`, `.credentials`,
  `credentials`, `oauth_creds`, `google_accounts`, `state.vscdb`, `.netrc`,
  `.claude`, `id_rsa`, `id_dsa`, `id_ecdsa`, `id_ed25519`, or begins with one
  followed by anything but a letter or digit (`auth copy.json`,
  `auth (1).json`, `auth.json.md`, `state.vscdb.json`, `id_rsa.json` —
  **measured** in the second RG-16 review: each returned the key while only
  the exact names were denied), and a name that ends in an exact name above
  after anything but a letter or digit (`Copy of auth.json`). `author.md` is
  still read;
* extensions: `*.key`, `*.pem`, `*.p12`, `*.pfx`, `*.keychain`, `*.keychain-db`;
* any name containing `token`, `secret`, `credential`, `oauth`, `password`,
  `passwd`, `apikey`, `api_key` or `api-key`;
* anything inside a `.ssh`, `.gnupg`, `.aws` or `keychains` directory.

`state.vscdb` is shut as a whole because its `oauthToken` row (measured) is in
the same SQLite file as the conversation titles, and a file cannot be read
"except one row". On top of the denylist only `.jsonl`, `.json` and `.md` are
ever opened.

**Version gate (RG-30).** Reading a version outside the measured range goes
ahead **with a warning** the user sees; writing outside the measured *write*
range is **refused**. Read and write ranges are separate because "we parsed its
files" and "we appended and the agent loaded it correctly" are different
measurements.

| Agent | Read range | Write range | Measured |
|---|---|---|---|
| Claude Code | 2.1.263–2.1.293 | write 2.1.285–2.1.285 | read: 30 newest transcripts (340 MB, no unparsed line); write: the E2–E5 append/resume experiment |
| Antigravity IDE | 2.5.5–2.5.5 | none | brain/ transcripts and task.md; no write was ever verified |
| Antigravity CLI (`agy`) | 1.1.10–1.1.10 | none | brain/ transcripts; no write was ever verified |
| Codex CLI | none | none | not installed; source and fixtures only |

A range is widened by a measurement, never by an edit: a read range after the
fixture corpus (RG-28) parses that version with no unparsed line, a write range
after the round-trip script (RG-29) passes on it.

## Dependencies (RG-05)

**No new crate.** Tacet's dependency list is an architectural decision
(CONTRIBUTING §1) and none of the formats here justifies a line in it.

* **zstd:** Codex may compress rollouts older than seven days to `.jsonl.zst`
  (behind a flag that is off today). Such a file is **refused** with a message
  that tells the user to run `zstd -d`; it is not silently skipped. A test
  holds `Cargo.lock` free of `zstd`, `ruzstd`, `prost`, `protobuf`,
  `rusqlite`, `libsqlite3-sys` and `sqlx`, so the refusal stays a decision.
* **protobuf:** Antigravity's `conversations/<id>.db` is SQLite with
  schemaless protobuf blobs. It is **not parsed**; the readable
  `transcript_full.jsonl` (fallback `transcript.jsonl`) and `task.md` carry
  what is needed, and the `.db` contributes only its mtime to a listing.
* **SQLite:** if a reader ever needs a SQLite file, it shells out to
  `sqlite3 -readonly` the way the `db` addon does, behind the same gate — it
  does not link a SQLite crate.
* All JSON is parsed by hand on `serde_json::Value`; every reader runs under
  `catch_unwind` (audit M30, RG-04) so a malformed line cannot take the
  session down.

## Antigravity terms of service (RG-05)

Antigravity's terms (§6) say that using third-party software to access
Antigravity is a violation, and its FAQ says writing to its storage is not
supported. Tacet's position:

* **Read-only.** Reading files the user owns on their own disk, to show the
  user their own work, is what every history viewer in the prior-art list does;
  Tacet does not drive the product, call its model or use its credentials.
* **Official hooks for reporting back.** A PreInvocation hook in
  `.agents/hooks.json` or `~/.gemini/config/hooks.json`, installed only with
  the user's approval, reads Tacet's handoff `.md` and returns it as an
  `ephemeralMessage` in `injectSteps`. Antigravity's own mechanism, writing
  nothing of Antigravity's.
* **One write, experimental, off by default.** Checkbox updates to a
  conversation's `task.md` — and nothing else in Antigravity's storage — behind
  an explicit experimental flag (`version::check_write_target`). Even with the
  flag, the version gate has no Antigravity write range yet, so the write is
  refused until RG-26 verifies by hand that the GUI re-reads the file.

## Writing back

Preferred to fallback, for every agent:

1. **The handoff file (RG-07)** —
   `<config>/handoff/<agent>-<session>.json` plus a rendered `.md` twin, each
   written atomically (unique temp file, mode 0600, rename), the JSON first:

   ```json
   {"schema": 1, "session": "<id>", "agent": "claude|codex|antigravity",
    "by": "tacet", "updated_at": "2026-10-10T09:00:00Z", "summary": "…",
    "tasks": [{"id": "1", "title": "…", "status": "in_progress", "note": "…",
               "updated_at": "…"}],
    "next": "…"}
   ```

   A record without `schema` reads as 1 (another agent following the rule by
   hand will not know to write it); a higher number is refused; unknown
   fields are ignored. A record whose file name the credential denylist
   would refuse on the way back (a session id containing `token`, `secret`…)
   is refused before it is written, with the same reason.
2. **The official hook** — Claude Code and Codex SessionStart
   `additionalContext`, Antigravity PreInvocation `injectSteps`. The hook reads
   the `.md`; no transcript is touched.
3. **A file append** — Claude Code only, inside the measured write range, the
   user-role note format, never into a live session, chain verified after.
   Codex appends additionally refuse when `thread-writer-locks/<id>.lock`
   exists and continue `ordinal`s; they stay refused until a Codex version is
   measured.

**Status mapping** — the shared enum and how each target spells it (the audit
workbook's "İlerleme Protokolü" sheet):

| Shared | Claude `TaskUpdate` | Claude `TodoWrite` | Codex `update_plan` | Antigravity `task.md` | ACP plan | Workbook "Durum" |
|---|---|---|---|---|---|---|
| `pending` | pending | pending | pending | `- [ ]` | pending | Açık |
| `in_progress` | in_progress | in_progress | in_progress | `- [/]` | in_progress | Devam |
| `completed` | completed | completed | completed | `- [x]` | completed | Bitti |
| `blocked` | pending + `addBlockedBy` | pending (reason in content) | pending (reason in `explanation`) | `- [ ] … (blocked: …)` | pending | Engelli |
| `cancelled` | deleted | left out of the list | left out of the plan | `- ~~…~~` | left out | İptal |

`blocked` and `cancelled` are lossy in the three-state targets by design; the
tests in `handoff.rs` state exactly how.

## Prior art and licences (RG-35)

Several projects already read these formats. **Tacet takes approaches from
them, not code**, so no licence obligation is incurred and none of their
terms has to be reconciled with Tacet's MIT licence.

| Project | Licence | What was learned |
|---|---|---|
| cass (coding_agent_session_search) | MIT + an OpenAI/Anthropic "Rider" | Antigravity `transcript.jsonl` as the readable source; the Rider adds terms beyond MIT and must be read before anything is copied |
| Codex `external-agent-migration` | Apache-2.0 | Fail closed on an active, archived or diverged thread before appending; copying code would require the NOTICE and attribution |
| baton | Apache-2.0 | Native-format round-trips are possible; claims, not measurements here |
| continues, catchup | MIT | A markdown handoff injected into the target agent instead of writing its native format — the shape of RG-07 |
| SpecStory CLI, Vibe Kanban, claude-codex-switch, agent-sessions, claude-code-history-viewer, ccusage, ruler | Apache-2.0 / MIT | Read-only viewers and wrappers; confirm the formats and the "official resume only" boundary |

If code is ever copied, the licence row above is re-checked at that commit,
the copied file carries its original header, and Apache-2.0 sources bring
their NOTICE into the distribution.

## ACP, evaluated and deferred (RG-37)

The Agent Client Protocol (v1) would give one interface to all three agents:
`session/list`, `session/load` (which must replay the whole conversation),
`session/resume`, and a `plan` update with the same three statuses as Codex.
Adapters exist (`claude-agent-acp`, `codex-acp`, Antigravity's
`agy_acp_server`). It is **deferred**, for four reasons:

1. **Every adapter is a subprocess** — a Node/TypeScript SDK or a vendor
   binary. Tacet would be spawning the vendor's agent runtime to read a file it
   can read directly, and that process has the vendor's network access and
   credentials, so it would have to be an external tool behind the approval
   gate for an operation that needs neither.
2. **There is no write without a turn.** ACP has no "append a note without
   starting a turn"; `session/prompt` runs the vendor's model — a paid network
   call — which is exactly what the file and hook paths avoid.
3. **Replay is the whole conversation.** `session/load` must stream every
   message back; for a 40 MB transcript that is the bulk this design keeps out
   of the process, through a pipe instead of a file.
4. **Nothing was measured.** No adapter was run on this machine; the
   Antigravity server's `load_session`/`list_sessions` were seen only as strings
   in its binary.

Revisit when the three file readers are in (RG-17) and any of these changes: an
adapter becomes a static binary that runs without network, ACP gains a
turn-free append, or a fourth agent arrives for which ACP is the only
documented access.

## Consequences

* A reader is a translation into `AgentSession` and nothing more; listing,
  summarising, taint, defang and handoff are shared.
* A new credential location is one line in `secrets.rs` **and** one here; the
  test fails until both exist.
* A new agent version is read with a warning the day it ships and written to
  only after someone measures it — slower than the agents release, on purpose.
* Codex support is fixture-tested and unverified against a real install until
  someone with Codex installed runs the corpus and the round-trip script.
