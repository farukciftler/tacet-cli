# Changelog

**WHY THIS FILE IS SHAPED LIKE THIS.** The version numbers in
`[workspace.dependencies]` are FLOORS, not preferences: eighteen of them carry a
comment recording the fix that a lower resolution would quietly undo. **Cargo
strips comments when it packages**, so none of that reasoning has ever reached a
crates.io consumer — the only thing that survives is the number, and a number
without its reason is indistinguishable from an arbitrary bump.

So this file is that reasoning, moved somewhere it gets published. It is not a
list of everything that changed; it is the list of versions **you cannot go
below**, and why. Entries older than the first release are reconstructed from
those manifest comments rather than written at the time — that is why they read
as reasons rather than as release notes.

The format is loosely [Keep a Changelog](https://keepachangelog.com/); the
project is pre-1.0 and versions are per-crate, so there is no single line of
releases to follow.

---

## Unreleased — the audit of 9 Oct 2026: completeness, 10 Oct 2026

What the second pass over the review found unfinished, each fixed with a test
that fails without the fix.

**Security:**

* `tacet sessions --purge` deletes the `<config>/handoff/` records — each a
  summary of another agent's transcript — and the transcript backups `tacet
  handoff note` keeps, with the conversations and the receipt log; `--json`
  reports `handoffs` beside `purged` and `receipts` (RG-32).
* The corpus anonymiser (`examples/corpus_capture.rs`) masks what a tool
  returned wherever it sits: a value inside a tool's data survives only when
  a reader takes it from there, a key only when a reader looks it up there,
  a number only when it is a task id of at most six digits (a card number
  under `id` was kept), `blockedBy` is kept as a string and as a number
  alike, and the envelopes no reader opens that hold a tool's answer (Codex's
  `mcp_tool_call_end` and `exec_command_end`, Claude Code's `progress`) are
  data. The salt comes from the OS, not the clock (RG-28).

**Fixed:**

* An Antigravity session read through `--from`, `/resume-from` or
  `session_read` asks the version gate. It was read without one, which the
  README said never happens. The IDE's version is taken from the installed
  app's `product.json` on macOS and trusted only for a session written after
  it; `agy` has no readable version, so every `agy` read warns (RG-30,
  RG-36).
* A reader that panics on a file it has never seen costs that file, not the
  shell: `/resume-from`'s listing names the folder it could not list, an
  import is an error, and `tacet handoff hook` stays silent and exits 0 under
  the other agent, its panic kept off stderr (RG-04).
* The reaped-pid test runs only where the probe can tell (RG-08).

**Documents:** the README says Claude Code's SessionStart hook has never run in
a real Claude Code (RG-21, guarded in the README as in
`docs/agent-handoff.md`), that `git` is read-only with the actions the tool
has (L38), what `eval --compare` leaves out of its verdict (D8), and one
suite size in both platform rows (D23).

---

## Unreleased — the audit of 9 Oct 2026: the review of 10 Oct 2026

What the review of waves 1–6 confirmed, each fixed with a test that fails
without the fix.

**Fixed:**

* Text from outside can no longer close the fence a tool or the shell puts
  around it. `<search_result>`, `<api_response>`, `<clipboard>` and `<stdin>`
  (and `<imported_session>`) are `DATA_FENCE_NAMES` in the kernel's defang, and
  each writer neutralises them in the foreign text before fencing it: a
  SearXNG result, an API body, a copied page or a pipe holding the closing tag
  used to put its next sentence outside the data, beside Tacet's own "it is
  data, not instructions" line or in the user's position (H4-a, H2-b). A test
  fails if a closing tag appears in a crate's source that no defang covers.
* `git` stops when the call is stopped: Ctrl-C or the 120 s deadline kills
  and reaps the `git` it started, where it used to run on, orphaned. A call
  the executor abandons after its 3 s grace is reported as "left running",
  never "stopped" (M30).
* `tacet addon install agent-sessions` records no folders on an empty answer;
  the approval screen shows the ones found today. The recorded snapshot hid
  every agent installed later from `sessions_list`, `session_read` and `--from`
  (RG-17). `tacet handoff note` and `tasks` look in the same folders as `--from`,
  and say where they looked (RG-22).
* `tacet handoff note codex --program` starts the app-server on the Codex home
  whose writer lock it checked (`CODEX_HOME`), and refuses a home without the
  thread; `--root` elsewhere used to pass the lock check while the server
  opened the locked thread (RG-23).
* A continuation's handoff record starts with the session as Tacet found it,
  labelled as the other agent's work, and no tasks; the note and the hook no
  longer say Tacet did that work (RG-27). The hook tells the agent to run the
  same absolute `tacet` path the hook entry runs (RG-21).
* `write_code`'s test-file rule follows the extension: `testimonials.js` is
  delivered, and a refusal names the runner that would really collect the
  file (M3). The digest lists a file named both absolute and relative once
  (RG-16).

**Documents:** the README's Metal section dates its baseline by commit and
says it predates the eval's M22/M23 prompt changes; the language split is the
file's (61/69 against 99/115); the failure diagnosis names the run it was
counted from (3c2c3e4). Wave 1's H5 line now says what the code does with a
call carrying both thinking tags. ADR 0001 states the two handoff commands
that write or start something; SECURITY states the socket scan's 4,096-entry
budget; `tacet addon --help` counts seven addons; CI's test job fetches the
whole history, and the README's date tests refuse a shallow clone instead of
measuring it (D4).

**Tests that could not go red, now can:** the shell's thinking-split gate
(H5) is driven through `tacet chat`; the write-back's "changed while asking"
refusal in all three agents, and Claude's chain, unfinished-line, session-id
and read-back checks (RG-21); one credential case per denylist entry (RG-16);
the smoke guard reads `tacet chat --help` instead of field names; the two
grammar rules for C1/DEL and a nested dead field (M15).

---

## Unreleased — the audit of 9 Oct 2026, wave 6: the documents

No change to what the program does and no API changes: the README, its
crates.io mirror, SECURITY, CONTRIBUTING, this file, one comment in the
workspace manifest, two tests that hold the documents to the tree, and three
tests that measure what the page already said: `db_write`'s question answered
through a pseudo-terminal, both ways, and a stopped tool that will not stop
abandoned after its 3 s grace.

**Fixed:**

* The 3090 wall time is the committed report's, 7m 07s and 6.7×, not the
  replaced run's 7m 34s and 6.3×; the comparator block under it is re-derived
  from the two committed files (23 cases excluded, 161 paired, +2.5 points,
  not distinguishable) (D1).
* `baselines/` holds seven files, two of them 3090 runs, and neither is the
  "after" report of the +4.9 verdict, which is still not committed; the runs
  are dated 6 Sep 2026, the day after the verdict, by the corpus report's own
  date (D2).
* The Turkish corpus is compared with the 184-case suite on the same kind of
  card the same day — not provably the same machine, since the suite's catalog
  has the web tools and the corpus box had none — not with the Metal run: 72.4% against 78.1%, or 89.3% with the twenty cases
  whose tool that box lacked skipped the way the corpus skipped them — a 16.9
  point gap, not "thirteen" (D3).
* The model table is one table again (a blank line split it), and it says
  that its four rows are hand-recorded on or before 5 Sep 2026 (first
  committed in ae0893a) and cannot be
  re-measured here (D4). The seven-language table's `en` column quotes the file
  it cites, 136/160, not 133/160 (D5).
* The journal example writes under `~/.tacet`, not `/tmp` (D10); the version
  example is 0.1.29 (D11); "`cargo install tacet-cli` is this page" now says
  what is unreleased and how to install from a checkout (D12).
* SECURITY names the real config directory — `$TACET_HOME`, else
  `$XDG_CONFIG_HOME/tacet`, else `~/.tacet` — not `~/.config/tacet` (D13). The
  workspace manifest's version comment names `tacet-zip` alone as inheriting
  the workspace version; `tacet-skills` left it on 7 Sep 2026 (D14).
* The headline claims re-checked against the fixes that made them true: what
  runs is the call the grammar validated, a grammar call carrying both thinking
  tags runs as written and only a recovered one is refused, each half citing
  the test that pins it (D19, after H5 and L25); the summary of
  an imported session is on the list of defanged inputs (D20, after H3 and H4);
  the Cancel gate says what an abandoned tool still does, which of its two
  paths — the signal and the key — each test shows, that the key path and the
  3 s abandonment are unit tests only, and that the signal path is run on
  macOS only and not wired on Windows (D21, after M4); the `db_write` question
  says when it became answerable at a terminal, and a pty test now answers it
  — the earlier citation drove only the outbound approval (D22, after H1). The test count is re-measured and
  dated (D23).
* CONTRIBUTING's gate list is the list CI runs, the pressure run and the fmt
  `--check` included, and a test now holds that direction too. The paragraph
  under it names the three things beyond the list that can fail a pull
  request — `cargo deny check`, the Linux `bwrap` preflight, and
  `TACET_SANDBOX_MUST_RUN=1` on the Linux `cargo test` — and how to run the
  last one locally (RG-36, review).
* Every 3090 wall time on the page, in any of the forms a hand-copied time
  takes ("6 min", "7 minutes", "7m 34s"), is a committed report's or sits in a
  paragraph that says no report was kept; the sentence that said "the 3090
  wall times remain hand-recorded" says the model table's (D1, review).

**Added:**

* README *Other agents' sessions*: what the `agent-sessions` addon reads and
  refuses, what reaches the model, that nothing opens a socket or starts the
  other agent, how `tacet handoff` reports back, the 9 Oct 2026 Claude Code
  experiment with its machine and version, what was not measured, and
  Antigravity's terms. SECURITY scopes the readers and `tacet handoff`;
  CONTRIBUTING says how a version range is widened (RG-36).
* `crates/tacet-cli/tests/the_readme_quotes_the_baselines.rs`: the Metal
  table, the BFCL tables, the 3090 runs' wall times and dates, the Turkish
  corpus comparison and the seven-language `en` column are read back from the
  files in `crates/tacet-eval/baselines/`, the
  comparator block is re-run on the two committed reports, and the model
  table's scores are recomputed from their own counts. (This line said "every
  number the README quotes" until the review of 10 Oct 2026 found the language
  split and the failure diagnosis checked by nothing; they are now, below.) It starts the binary
  under test as `eval --compare`, so it is a reviewed row in
  `network_monopoly`'s `RUNTIME_SPAWN_SITES`.
* `crates/tacet-cli/tests/the_docs_match_the_tree.rs`: the version example,
  the journal path, the config directory, the manifest comment, an undated test
  count, a table split by a blank line, CI steps missing from CONTRIBUTING's
  list, a cited test path that selects no test (cargo's filter is a substring,
  so a wrong path runs zero tests green), a quoted `[y/N]` question its cited
  test never waits for, a refusal claim that does not cite the executor, and a
  "last publish" that is not CHANGELOG's newest each fail the build.
* `consent_reaches_the_person` answers `db_write`'s `Apply it? [y/N]` under a
  pty with `y` and with `n` and counts the rows the file holds after;
  `executor::guarded` abandons a cancelled tool that ignores the stop after
  `ABANDON_GRACE`, and not before.

**The audit's IDs, all six waves.** Every ID below is named by a commit on this
branch (re-counted from the log on 10 Oct 2026), and that commit's message
carries its test and the proof that the test fails without the fix; what could
not be run on the machine that wrote it (a Linux box, a Windows box,
real weights, Codex, the Antigravity GUI) is said in that commit and, for the
session work, in `docs/agent-handoff.md`.

* H1–H7 (with H2-a/b/c and H4-a/b/c); M1–M30; L1–L38; T1–T5.
* D1–D23: D6, D7, D15 in wave 3; D8, D9, D16, D17, D18 alongside the code they
  describe; the rest here.
* RG-01 to RG-37: the readers and the shared layer in waves 2 and 4, the
  write-back in wave 5, the documents (RG-36) here; RG-05, RG-06, RG-07, RG-16,
  RG-30, RG-35 and RG-37 are recorded in `docs/adr/0001-agent-sessions.md`.

---

## Unreleased — the audit of 9 Oct 2026, wave 5

Not published, and kept apart from waves 1 to 4 so a reader can tell which
round a line belongs to. `tacet_tools::sessions::writeback` is new public API
in `tacet-tools`; nothing existing changed shape.

**Added:**

* `tacet handoff` (`hook`, `install`, `show`, `update`, `note`, `tasks`): Tacet's
  progress reported back to the agent a session came from (RG-21 to RG-27,
  RG-31). Every write into another agent's files asks y/N first, and with
  nobody at the terminal the answer is no. Codex rollout appends and
  Antigravity `task.md` writes are refused on every version until a write is
  measured; see `docs/agent-handoff.md` for what is measured and what is not.
* `tacet handoff note codex <id> --program <path>` is the one place Tacet
  starts another agent's CLI: the binary at the path you give, as
  `app-server`, after a y/N naming the resolved file. No model turn is
  started. The network-monopoly test now holds that no model tool can reach
  it (RG-23, RG-33).

**Fixed (before it shipped):**

* `tacet handoff install --path` merged the hook into the settings file
  before asking, then wrote that merge after the yes. A change made while the
  question was on screen (Claude Code rewrites its own `settings.json` on
  `/model`) was lost. It now re-reads the file after the yes and writes
  nothing if it changed. The temporary file is created 0600 rather than at
  the umask's 0644, so a settings file holding an API key is never briefly
  world-readable (RG-21).
* The guard that the Codex app-server is reached only from `tacet handoff`
  checked only who names the spawning function. A model tool that ran the
  `handoff` subcommand itself passed it. The guard now also checks who names
  the subcommand's module and job type (RG-23).

## Unreleased — the audit of 9 Oct 2026, wave 4

Not published, and kept apart from waves 1 to 3 so a reader can tell which
round a line belongs to. `tacet_tools::sessions_tool` is new public API in
`tacet-tools`; nothing existing changed shape.

**Added:**

* The `agent-sessions` addon (off by default), with `sessions_list` and
  `session_read`, `tacet chat --from <agent>:<id>` and `/resume-from`
  (RG-17, RG-18, RG-19, RG-20). A continuation stores the full transcript and
  sends a summary plus a reference to it. The folder a transcript names is
  entered only on a yes, and the question cannot carry terminal control
  characters. No program is started and no network is opened.

**Fixed (before it shipped):**

* The router's session triggers no longer fire on a qualifier alone: "leave
  off the VAT", "my last session with the dentist", a parliament's or a
  court's "dünkü/önceki oturumu" and "kaldığım yer" (where I stayed) each put
  `sessions_list` first and now score nothing (RG-19).
* A test now runs `tacet why` over a written `addons.json` and fails if an
  open `agent-sessions` addon does not put both readers in the catalog, or a
  closed one does (RG-17).

---

## Unreleased — the audit of 9 Oct 2026, wave 3

Not published either, and kept apart from waves 1 and 2 for the same reason.
No library API changes in this wave: everything below is the `tacet` binary,
a test corpus and a script.

**Security:**

* A pipe into `--message`, a replayed transcript (`--continue`, `--session`)
  and an injected memory note taint the session from the start, so an
  outbound call after any of them asks first; before, only a tool could taint
  (H2-b). The `--message` help no longer says "NO DATA LEAVES" (D15).
* Receipts (format 2) keep a tool chip's kind, its outcome by one of five
  fixed names and its length — never its words: a failed chip's sentence,
  built from the call's input, stays out (L27, RG-32). `tacet sessions
  --purge` deletes `receipts.jsonl`, its head and its lock.
* While a turn runs, a SIGINT cancels the turn and the running tool instead of
  ending the shell — under `--json` and with stdout piped too (M4).

**Fixed:**

* `receipts.head` records the count and the last hash, and `tacet log` reports
  a cut tail, an emptied log, and a missing head as not intact. A head is
  never rebuilt from a log that already holds format-2 receipts, so deleting
  it no longer hides a cut: such a log reads "head missing" until
  `tacet sessions --purge` (L26). The hash is unkeyed and the help no longer
  calls the log tamper-proof.
* Appends from several sessions hold a lock with a per-acquisition token; a
  stale lock has one taker (L28).
* A `--continue` chain keeps every earlier turn, and "latest" is the file
  written last (L29); the privacy notice says a continued conversation is
  copied into each new file (L27, L29).
* The daily update check runs only at the end of an interactive session with
  stdout and stderr on a terminal — never after `--message`, under `--json`,
  or piped (D7). The README now says what the bare `tacet update` does: it
  downloads and installs after asking; `--check` only looks (D6).
* The updater removes an earlier download's leftovers with the staged binary
  (L3); `probe_features` gives up on a binary after 10 s (L7).
* Codex reader: an ordinal of `u64::MAX` in a paginated rollout no longer
  overflows the gap count (RG-28).
* `scripts/session-roundtrip.sh` (RG-29): E3 reads "same" — the outcome that
  lets a Claude Code version join the write range — only when the answer
  credits Tacet with the note and does not credit the user; an answer that
  merely says "Tacet", or quotes the note's header, no longer passes. E2 tells
  a claim of authorship from a denial anywhere in the sentence. E1 checks both
  sessions answered READY. A run that differs or stops after a paid turn
  keeps its scratch directory (raw output and transcript copies), names any
  session filed outside the predicted project directory, and counts E1's
  cost; a stop before the first turn leaves no scratch behind.

---

## Unreleased — the audit of 9 Oct 2026, wave 2

Nothing below is published yet either. It sits apart from wave 1 so a reader
can tell which round of fixes a line belongs to; when both are published they
go out under one version per crate, with the bumps named in both lists.

**Breaking, for the next release of each crate:**

* `tacet-kernel` (additive, but `tacet-tools` needs it): `Tool` gains
  `external_content()`, `acts_on_host()` and `deadline()`, all defaulted;
  `ToolContext` carries a `CancelToken` (`cancel_token()`, `with_cancel_token`)
  and the outside-text flag.
* `tacet-mcp`: `ConnectionSetting` has a new public `always_ask` field (a struct
  literal breaks).
* `tacet-tools`: `ApprovalRequest` has a new public `reason: ApprovalReason`
  field; `executor::quiet_tool_panics`, `executor::ABANDON_GRACE`.

**Security:**

* An MCP server's answers taint the session (H2-a). Once the session has read
  text from outside the machine — a web page, an `http` or MCP answer, an
  imported transcript — every `shell` and `db_write` call asks first, and the
  question names that reason instead of claiming data will be sent (H2-c,
  RG-01). A per-connection `always_ask` in `mcp.json` asks before every call to
  that server; no setting turns the taint off.
* Every tool call runs on a worker thread under `catch_unwind` with a per-tool
  deadline: a panic is a failed call, not the end of the session, and a tool
  that does not stop is left behind after 3 s with the turn saying so (M30,
  RG-04). Ctrl-C reaches a running tool: `shell` and `run_code` end their
  process group, SIGTERM then SIGKILL, within about a second (M4).
* A consent question (outbound approval, `db_write`, an MCP server's question)
  reaches the person at the terminal, and only them. The turn indicator steps
  aside while a question is open, so the question is no longer erased and `y`⏎
  no longer swallowed. The answer is read raw, with its own echo, so Esc (alone
  or followed by Enter), ctrl-c and ctrl-d decline, and ctrl-c no longer kills
  the shell. With stdin piped, the question is never asked: the answer is no,
  nothing is read from the pipe, and the session is not given `db_write` (H1,
  RG-03, L31).
* Tool-chip failure reasons and every server string that `tacet mcp try` and
  the MCP start-up report print are made into one line, with escapes removed
  (M13, M14). `tacet mcp try` sends the same OAuth token the chat would (L10).

**New, additive (not reachable until the `agent-sessions` addon, RG-17):**

* `tacet-tools`: `sessions::claude` — `ClaudeSource` (a `SessionSource` for
  Claude Code), `ClaudeRead`, `pid_alive`, `project_dir_name`,
  `project_dir_matches`, `LIST_TAIL_BYTES`, `MAX_TRANSCRIPT_BYTES`,
  `CONFIG_DIR_VAR` (RG-08, RG-09, RG-10).
* `tacet-tools`: `sessions::codex` — `CodexSource`, a `SessionSource` for Codex
  CLI rollouts, written from openai/codex's source and fixture-tested only;
  a `.jsonl.zst` rollout is listed and refused with the `zstd -d` advice
  (RG-11, RG-12, RG-13).
* `tacet-tools`: `sessions::antigravity` — `AntigravitySource`, a
  `SessionSource` over the step logs and `task.md` under the three
  `~/.gemini/antigravity*` roots; the conversation databases are dated, never
  opened (RG-14, RG-15). A title is what the user typed inside
  `<USER_REQUEST>`, Antigravity's `<ADDITIONAL_METADATA>` block is kept as a
  system note rather than the user's words, and the workspace is
  `run_command`'s `Cwd` — the shape measured over 50 real logs on 10 Oct 2026.

---

## Unreleased — the audit of 9 Oct 2026, wave 1

Nothing below is published yet. When it is, the crates whose public API changed
need the bumps named here, and the `[workspace.dependencies]` floors move with
them.

**Breaking, for the next release of each crate:**

* `tacet-mcp` → 0.3.0: `auth::begin(&AuthSetting, server_url)`,
  `auth::refresh_request_body(…, resource)`, and a new public `resource` field on
  `auth::Authorization` and `auth::Token` (a struct literal for either breaks).
  `tacet-tools` must take the new version.
* `tacet-memory`: `MemoryStore::save` takes `&mut self`; `MemoryStore::forget`
  returns `Forgotten` instead of `usize`. New: `Forgotten`, `LoadProblem`,
  `MemoryStore::load_problem`, `neutralize_markup`. The workspace floor (0.1.2)
  goes up to the release that has them.
* `tacet-engine`: `Generation` gains public `raw` and `executable` fields — build
  it with `Generation::new`. New: `Generation::split_against`, `MIN_ANSWER`,
  `CONVERSATION_KEPT`, `token::MIN_QUESTION_KEPT`, `TokenCounter::prompt_ceiling`,
  the `thinking_switch` module. `TokenCounter::validate` refuses only above `prompt_ceiling()`.
* `tacet-kernel`: `defang` and `register_markers`; `ArgSchema::json_number_value`;
  `ArgSchema::choice` panics on an empty set.

**Security:**

* The prompt defang lives in `tacet_kernel::defang` and covers every
  chat-template marker (`<think>`, `<tool_call>`, `<eos>`, `<bos>`, `</s>`,
  `<unusedN>`, `<start_of_turn>`, …), this prompt's `<memory>`/`<cwd>` fences
  and the loaded tokenizer's special tokens; it now reaches the directory census
  and the system block. Skill names are normalised to `[a-z0-9-]{1,32}`.
  Ordinary code such as `x <| f` and `<|>` is no longer altered (H3, H4, L37).
* The thinking split cannot edit or launch a call: a `<think>` inside a call's
  arguments stays in the argument, and when the split can only guess where a
  block ends nothing runs unless both readings name the same call (H5). A block
  that is never closed counts as a guess when the search stepped over a
  `</think>` inside a call, so a call declined in the thinking is not run. A
  call with both `<think>` and `</think>` in its values is never edited: a
  grammar call (`name(` or bare JSON) runs exactly as written, because it
  parses and the split can prove where it ends; a recovered call (a marked
  `<tool_call>` or a name glued to its argument) carrying both is refused. (This
  line said every such call was refused until the review of 10 Oct 2026;
  `thinking::tests::a_proven_split_has_one_reading` runs the grammar one.)
* `git` no longer lets a repository's own config run programs (fsmonitor,
  hooks, `diff.external`, textconv, filter drivers including submodules',
  `gpg.program`, promisor `ext::` fetch). Requires git ≥ 2.31; an older git
  answers `git_unsupported` (H6).
* `run_code` (Linux) shadows every socket `/proc/net/unix` lists, the LXD,
  Incus, Multipass and MicroK8s API sockets, and sockets in a working folder
  outside `$HOME`; each scan root has its own budget, so a big working folder no
  longer starves `/tmp` (H7). `write_code` refuses `toxfile`, `dangerfile`,
  `conanfile`, `serverless`, `truffle-config`, the ipython/jupyter config
  family, any `.py` name the user's python can import as an installed
  package, and the names of tools started as `python -m <name>` (pip, pytest,
  mypy, black, ruff, coverage, …), installed or not (M3). `run_code` canonicalises
  `$HOME` in the command it spawns, not only in the library (M2).
* No `ureq` agent reads `ALL_PROXY`/`HTTPS_PROXY`/`HTTP_PROXY` any more, in
  `tacet-web` or `tacet-mcp`; MCP refuses userinfo in an address, follows no
  redirect and sends the RFC 8707 `resource` on login (M12, L1).
* `db` / `db_write` refuse a statement that begins with `-`, which sqlite3 read
  as a command-line option (L18).

**Fixed:**

* `tacet-grammar`: closed-set values and keys are compiled to their JSON
  spelling; numbers are checked by the tool layer's own parser; numbers (400
  fraction / 4 exponent digits) and arrays without `max` (1024 items) now end
  (M15, M16, L25).
* A 4096-token window no longer refuses ordinary turns: `fit` refuses only when
  fewer than `MIN_ANSWER` (256) tokens would be left, or when the question could
  keep fewer than `MIN_QUESTION_KEPT` (8) of its tokens (or the history none)
  beside the fixed part; a typed question is no longer cut from the front; the in-call loop detector treats JSON keys as
  structure again (L21, L22).
* The shell and the eval build each pass's prompt in one function
  (`tacet_eval::turn_prompt::build_turn_prompt`), tools inside the budget and
  `fit` inside it too; the eval records a refused prompt as `refused for the
  window` (M22). The Qwen soft switch is one rule for both (M23). A journal is
  stamped with the commit, the uncommitted edits, the prompt text and every
  case (M24). `--compare` sizes the suite from both the fixed and the broken
  cases (L30).
* `archive`: refuses empty-block deflate floods, overlapping entries and
  mismatched central/local names; verifies every CRC, a recorded 0 included
  (the CRC and the empty-block limit on extract only, since listing decodes
  nothing);
  extraction is all-or-nothing and its rollback removes only the directory the
  run made (M8, L11, L14).
* `calculate` reads `1,000` as a thousand and has one percent rule; `calendar`
  no longer moves a date into the next month on the 29th-31st and no longer
  panics on a multi-byte offset (M9, M17).
* Two sessions share `memory.json` without losing each other's notes; a
  damaged file is set aside and the last good copy loaded; `forget` deletes one
  note or asks (M20).
* Documents: a 1.5 KiB sheet can no longer claim a million cells; the second
  `O_NOFOLLOW` belt uses each architecture's own bit (M6, L15).

---

## Published 2026-09-07 — a marker is not text

Three crates, hours after the last release, because a user hit a crash on it.

| crate | was | now |
|---|---|---|
| `tacet-engine` | 0.1.12 | **0.1.13** |
| `tacet-skills` | 0.1.2 | **0.1.3** |
| `tacet-cli` | 0.1.28 | **0.1.29** |

`tacet-skills` LEAVES THE SHARED WORKSPACE VERSION and takes its own. It has
changed in three consecutive releases and `tacet-zip`, which shared that number
with it, in none — so the shared version was forcing a no-change release of
`tacet-zip` every time. Two crates that do not move together should not share a
number.

**`tacet-engine` 0.1.13 is the floor for a generation that only contains text.**
Reported from a real session on gemma3-4b, two faults in one turn:

    Tacet <unused6088><unused6088><unused6088>… (twelve of them)
    engine error: the sampler returned 262207, which is not a token id
                  (the vocabulary has 262145 entries)

The first is the PADDED OUTPUT LAYER: qwen3 is 151936 wide against 151669 real
tokens, gemma3 262208 against 262145, and every sampling branch could choose a
padding position. Three steps of a 184-case run died this way. The second is
gemma3's 6242 `<unusedNNNN>` placeholders, marked `special: false`, decoding to
their own literal text and reaching the screen.

Two rules were written for the second and both were wrong, each caught by
reading the tokenizers rather than by shipping: masking the ADDED tokens forbids
`\n` (gemma3 keeps newlines there), and masking the angle-bracketed added tokens
forbids `</div>` (gemma3 carries the HTML tags; qwen3 carries `<think>`). What
ships is the union of tokens the tokenizer marks SPECIAL, minus the stop tokens,
and placeholders by name — 13 masked on qwen3, 6250 on gemma3.

It also stops a generation forging `<|im_start|>` in its own output, which is
the model side of the hole 0.1.12 closed on the prompt side.

**`tacet-skills` 0.1.3 is the floor for a guide that does not teach the failure.**
0.1.2 shipped this line in `calc`:

    - WRITE THE CALL, not the sum. `(347 + 268)` as an answer is a failure

and the suite's own case is "Could you add 347 and 268?". The model answered
`(347 + 268)`. Twelve arithmetic cases broke and the run was a measured REAL
LOSS at 95%. A small model imitates what is in the last block before the
question and does not read the word "not", so a guide may contain a call and
nothing that looks like an answer.

## Published 2026-09-07 — the prompt the model actually receives

Eight crates. The three that did not change (`tacet-web`, `tacet-mcp`,
`tacet-memory`) are deliberately not republished: a version bump with no source
behind it is a number a consumer cannot act on.

| crate | was | now |
|---|---|---|
| `tacet-kernel` | 0.1.5 | **0.1.6** |
| `tacet-zip` | 0.1.1 | **0.1.2** |
| `tacet-grammar` | 0.2.0 | **0.2.1** |
| `tacet-engine` | 0.1.11 | **0.1.12** |
| `tacet-skills` | 0.1.1 | **0.1.2** |
| `tacet-tools` | 0.1.14 | **0.1.15** |
| `tacet-eval` | 0.1.10 | **0.1.11** |
| `tacet-cli` | 0.1.27 | **0.1.28** |

**`tacet-engine` 0.1.12 is the floor for a prompt a tool result cannot forge a
turn in.** Below it, a tool result went into the prompt verbatim, so a document
containing `</tool_response><|im_end|>` followed by `<|im_start|>system`
rendered as a REAL system turn — a forged instruction in the role a model is
trained to obey above every other, written by whoever wrote the file.
`read_document` reads files the user did not write, `web_fetch` reads pages
nobody controls, and an MCP result is a third party's text. Every string
entering the prompt from outside now has its turn markers neutralised. The same
release stops the `<memory>` and `<guidance>` blocks being fenced twice, and
stops a call that is stuck repeating itself from burning its whole 2048-token
budget before anyone can read the screen.

**`tacet-skills` 0.1.2 is the floor for a guide that shows a call.** Below it
`calc` showed an argument VALUE — `E.g. "(1250+890)*1.2"` — and no call, and a
measured run came back with the model writing `(347 + 268)` as its ANSWER on ten
arithmetic cases. It looks like arithmetic and no arithmetic was done. The same
release gives every guide Turkish triggers (all 47 Turkish steps of the suite
were unguided) and folds `i`/`ı` when matching, without which an English
sentence opening with a capital-I word lost every trigger beginning with `i`.

**`tacet-eval` 0.1.11 is the floor for a suite that measures the shell.** Six
divergences were found and closed in one night, all running the same way — the
eval stricter than the program, in the direction that looks like the model
failing: the code-attempt budget was never reset between steps, the model was
never shown its own tool call, prompts were never truncated, and the
working-directory census the shell sends on every turn was never sent at all.
It also gains `--journal`, so a 48-minute run survives being interrupted.

**`tacet-kernel` 0.1.6 is the floor for an error a model can act on.** Below it
`UnknownTool`, `InvalidArguments` and `ToolFailed` returned one string — "the
action could not be completed" — so the two failures the model can fix were told
neither which had happened nor what to change.

## Published 2026-09-06 — the whole workspace

All eleven crates were published together, which had not happened before. Four
of them (`tacet-web`, `tacet-mcp`, `tacet-skills`, `tacet-zip`) had **changed
source under an unchanged published version** — verified by downloading the
tarballs and diffing, not by reading the log. That state is strictly worse than a
version gap: the next publish of that number is rejected outright, and until then
no consumer can tell they are missing the fixes.

| crate | was | now |
|---|---|---|
| `tacet-kernel` | 0.1.4 | **0.1.5** |
| `tacet-zip` | 0.1.0 | **0.1.1** |
| `tacet-grammar` | 0.1.2 | **0.2.0** |
| `tacet-web` | 0.1.5 | **0.1.6** |
| `tacet-mcp` | 0.2.0 | **0.2.1** |
| `tacet-engine` | 0.1.9 | **0.1.11** |
| `tacet-skills` | 0.1.0 | **0.1.1** |
| `tacet-memory` | 0.1.1 | **0.1.2** |
| `tacet-tools` | 0.1.13 | **0.1.14** |
| `tacet-eval` | 0.1.9 | **0.1.10** |
| `tacet-cli` | 0.1.25 | **0.1.27** |

Until that day `cargo install tacet-cli` gave you 0.1.25: no `bench` subcommand,
and pinned to `tacet-engine ^0.1.9` — one release below the floor for constrained
generation on Metal, so a Mac user following the README installed the crash.

Every crate now ships the MIT text as a file rather than only the field, and
`docs.rs/tacet-engine` builds with the `candle` feature, without which
`CandleEngine`, `Architecture`, `ModelSetting` and `TokenizerSource` were all
404 on the page an evaluator opens first.

## `tacet-kernel`

**0.1.5** — the constraint contract (`Constrainer`, `ConstraintSession`,
`ConstraintError`) moved here from `tacet-engine`. Three signatures over
`&mut [f32]` and `u32`, naming no model, tokenizer, device or file. Below this
version there is no `constraint` module at all, and taking the guarantee meant
taking a crate full of GGUF loading and prompt budgeting to get it.

**0.1.2 IS THE FLOOR** — below it there is no `hash` module, and the MCP client's
PKCE challenge, the download verifier and the receipt chain would each need their
own SHA-256 again: three chances for one of them to be quietly wrong.

## `tacet-grammar`

**0.2.0** — the `tacet-engine` dependency is **gone**. That is the whole point
of the crate and it was not true before: anyone who wanted the grammar took the
engine with it. `cargo run --example no_engine` demonstrates it against a pretend
runtime, and the claim is checked from outside the checkout — `cargo add
tacet-grammar tacet-kernel` in a scratch crate, with `cargo tree | grep -c
tacet-engine` at zero.

**0.1.1 IS THE FLOOR** — below it a tool that takes NO arguments cannot be called
at all: the prompt advertises `disk_usage()` and the grammar refuses it, so the
model writes something that parses as no call and the tool never runs. Remote
(MCP) catalogs are full of such tools.

## `tacet-engine`

Every number below that is not 0.1.0 is a floor, and each was put there by a
fix that a `^0.1.0` install would have resolved away. `cargo install` picks the
newest match, but a machine with an older copy already in its registry does not
have to move — so a version carrying a gate has to be the MINIMUM, or the install
looks identical while missing the thing it was published for.

**0.1.10 IS THE FLOOR FOR ANY CONSTRAINED GENERATION ON METAL.** candle's
`sample_argmax` is `logits.argmax(D::Minus1)?.to_scalar::<u32>()`, and on the
Metal backend that reduction hands back the extremum's BITS rather than its INDEX
once the distribution is mostly `-inf` — the exact shape a grammar mask produces.
A turn died with `constraint rejected the token: 4286578688`, which is
`0xFF800000`, the bit pattern of `-inf`. No vocabulary has four billion entries;
it was never a token id, and the message named the wrong layer.

**0.1.9 closes a hole straight through the headline claim.** The README says a
malformed tool call is unrepresentable. That held for every ORDINARY token and
never covered the one that is not: end-of-turn was never masked, so a model could
simply STOP in the middle of a JSON string. Measured on qwen3-4b, a `write_code`
call ended on `... print(result]})` with the string, the array and the object all
still open; from the outside it read as the model refusing to call the tool.

**0.1.8 IS THE FLOOR FOR THE TURN LOOP** — below it there is no
`FINAL_PASS_INSTRUCTION`, so the last pass of a turn is offered tools it cannot
usefully call and a model that keeps calling ends the turn with nothing said. It
also carries the quant label fix: below it `dominant_quant` counts TENSORS, and
Gemma3's six F32 norms per layer outvote its quantized body, so a q4 file is
reported as `F32` and a comparison matrix reads as if the model had been run
unquantized.

**0.1.5 IS THE FLOOR FOR ANY SKILL TO SURVIVE A TOOL CALL** — below it the
`<guidance>` block was written ONLY inside the question turn, and from the second
turn of the tool loop the question is deliberately empty, so the guidance
vanished at exactly the turn where the model is recovering from a tool result.
`Plain` kept it, which made it worse than a plain bug: eval runs on FakeEngine,
FakeEngine is `Plain`, so the one template no model ever sees was the one the
measurement covered.

**0.1.3 IS THE FLOOR for anything that reports a measurement** — below it an
engine can only say its own name (`candle`), so four different weight files
produce four indistinguishable reports and a comparison matrix can silently
measure one model in every cell.

## `tacet-tools`

**0.1.13 IS THE FLOOR FOR WINDOWS**, and the defect was invisible on the two
platforms this is developed on. The path handed to the model after a write was
built with `strip_prefix` against a working directory canonicalized at a
different moment; on Windows canonicalization adds the `\\?\` verbatim prefix,
so one side carried it and the other did not, the strip silently failed, and the
model was handed a full absolute path spelled with backslashes — which it then
has to escape to write back inside a JSON argument.

**0.1.12 IS THE FLOOR FOR THE ROUTER**, and the defect it fixes had been silently
setting the ceiling on every tool-selection number this project has ever
recorded. A tool's score was the sum of hint lengths matched over its NAME AND
DESCRIPTION GLUED TOGETHER, so it grew with the SIZE OF ITS PROSE: `run_code`,
whose thousand-character description correctly says it cannot open a FILE, cannot
see a FOLDER and must not LIST from MEMORY, outscored `find_file` three to one on
"Find the file about the budget." — `find_file` sat at rank 7 on its own sentence.

**0.1.11** — below it `create_document` has no way to NAME its destination, so a
file asked for in one directory is written to another and reported as created.

**0.1.9 fixes a sentence the model reads on every turn**: `create_document`'s
description said "do note ask or narrate" where it meant "do NOT ask". It also
puts a ceiling under the tool descriptions, the largest block in the prompt,
which until then had none — the worst nine-tool selection measures 2542 tokens,
62% of the 4096-token floor window, re-sent on every pass of the tool loop.

**0.1.8** carries the trailing-comma repair for small models AND its tests: the
repair landed untested, and a function that rewrites the model's own words before
parsing them is the last place to leave untested.

**0.1.7 closes two failures that were not luck** — each failed in every run of
every variant measured. Below it a bare `{"kind":"date"}` is dropped as ambiguous
(two tools accept it, only one by a closed set), and the memory tool lets the
model answer "I'll remember that" without calling anything, which tells the user
something untrue.

**0.1.6 IS THE FLOOR FOR ANY NON-ENGLISH USER** — below it the router's trigger
table is English-only, so a message in Turkish touches nothing, scores zero and
the budget fills with the head of the catalog. It also matched triggers by bare
substring, so "teşekkürler" contains "url" and a thank-you pulled the web tools
forward.

**0.1.5 IS THE FLOOR for anyone using MCP** — below it the router cannot see a
remote catalog at all, so the model answers "I have no access to servers" while
the connection is working perfectly.

**0.1.4** — below it `shell` is offered on Windows, where the timeout cannot kill
the process group and a runaway command would outlive the turn. **0.1.3** brought
the addon gates and the five tools behind them.

## `tacet-web`

**THE NETWORK MONOPOLY.** A network call exists only in this crate and
`tacet-mcp`. No other crate opens a socket, and so the rule can be audited by eye
rather than by a script, the HTTP dependency stands only in these two manifests.

**0.1.5 IS THE FLOOR FOR `tacet update`** — below it the release asset's published
SHA-256 is never read, so every update is trust-on-first-use over TLS alone,
while the API response already carried the digest. It also refuses a dotless host
in the addon allow-list, where `merhaba` used to be accepted as a hostname.

**0.1.3 IS THE FLOOR** — below it the `http` addon's host allow-list cannot read
its own stored form, so a multi-host install refuses every call it was configured
to permit. **0.1.2 carried the SSRF gate**, and `^0.1.0` would happily have
resolved to 0.1.1, which does not have it.

## `tacet-mcp`

**0.2.0 speaks the 2026-07-28 revision**: stateless requests, the `_meta`
envelope, MRTR, poll-based tasks, issuer-bound OAuth. The 0.1 line speaks only
the frozen revision, and a `^0.1` resolve would silently put a client on it.

## `tacet-eval`

**0.1.8 IS THE FLOOR FOR ANY NUMBER THAT WILL BE BELIEVED**, and it carries
four corrections to the INSTRUMENT rather than to the thing measured:

* `routing` — the router's own choice, no weights, milliseconds. It sets the
  ceiling on the selection number and had no measurement of its own.
* The generation budget. The selection set handed the model
  `SamplingSetting::default()` (1024 tokens) while the shell hands it
  `generation_cap` (~14000 on qwen3-4b), so the two cases that ask for a real
  script were failing on a limit production does not impose.
* **The language gate passed everything.** It matched two-letter markers as
  substrings, so "have" satisfied the Turkish "ve" and "için" satisfied the
  English "in": every Turkish case had been reporting a perfect answer rate
  against a check no input could fail.
* The prompt. The set built no `<guidance>` block, so it measured a prompt the
  app has never sent.

**0.1.7 IS THE FLOOR FOR ANY MEASUREMENT THAT WILL BE COMPARED** — below it the
logic runner repeats the user's question AFTER the tool result on every pass, the
shape the shell removed because it caused the loop, so the set measures a prompt
the product does not build.
