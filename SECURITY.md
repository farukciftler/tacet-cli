# Security policy

This file exists so GitHub's **Security** tab has something to show. The policy
below was already written — it lived as the second-to-last section of
[CONTRIBUTING](CONTRIBUTING.md), which is the last place someone looking for it
would think to open.

## Reporting a vulnerability

Open a **private security advisory**:
<https://github.com/farukciftler/tacet-cli/security/advisories/new>

Please do not open a public issue first, and please give it a few days before
writing about it publicly. If you have not heard back in a week, say so in a
public issue *without the details* and it will get attention.

There is no bounty. This is a one-person project; what there is instead is a
credited fix, a test that reproduces what you found, and a note in the file that
says what was wrong — the same treatment every other defect here gets.

## What is in scope

The whole point of this program is that it runs on the user's machine and does
not send their data anywhere. Anything that breaks one of those is in scope:

* **Sandbox escape.** `run_code` and `write_code` execute model-written code
  behind `bwrap` (Linux) or `sandbox-exec` (macOS) with no network. Reading a
  file outside the sandbox, opening a socket from inside it, or outliving the
  timeout is a vulnerability. On Linux one limit is known and stated, not
  hidden: `bwrap` shadows the runtime socket directories (`/run`,
  `$XDG_RUNTIME_DIR`, `/tmp/.X11-unix`), the LXD, Incus, Multipass and MicroK8s
  API sockets, `$SSH_AUTH_SOCK`, every socket `/proc/net/unix` lists as bound
  in the host's network namespace, and sockets up to two levels deep in
  `/tmp`, `$TMPDIR` and a working folder outside `$HOME`. A socket bound in
  another network namespace (a container's) in a place none of those names is
  still reachable; a report naming one that matters is in scope.
* **Path escape.** Every tool that touches the filesystem resolves through
  `sandbox_path`. Reaching a file outside the working directory and the
  registered workspace roots — through a symlink, a `..`, a device path, a
  Windows 8.3 name, whatever — is a vulnerability.
* **Data leaving the device.** Only `tacet-web` and `tacet-mcp` may open a
  socket, and that is enforced by both crates being the only ones that declare
  `ureq`. A path that gets user data onto the network through any other route is
  a vulnerability — with one stated exception: `tacet handoff note codex
  <id> --program <path>`, a command you type, starts the Codex binary you named
  as `codex app-server` after a y/N that says it runs with your Codex login and
  can reach the network. That one is in scope only if a model can reach it or
  it runs without the question. So is a path that bypasses the approval gate
  in front of an outbound tool once the session has touched personal data.
* **Prompt-boundary attacks with a real effect.** A remote MCP server, a fetched
  page or a file's contents causing a tool call the user did not ask for — for
  example a description that carries terminal escapes, or content that
  impersonates a `<tool_response>` fence.
* **Supply chain.** A release asset that does not match its source, or a way to
  make `tacet update` install something the GitHub API did not vouch for.
* **Other agents' sessions.** The `agent-sessions` addon reads Claude Code,
  Codex and Antigravity transcripts, and those folders sit next to the agents'
  credentials. In scope: getting a session reader to return a credential
  (`auth.json`, `.credentials.json`, `state.vscdb`, a `*.key` file, a
  `.claude/settings*` file, or a copy, symlink or hard link of one) or any file
  outside the folders the install named; an imported transcript causing an
  outbound, `shell` or `db_write` call that did not ask first; text in a
  transcript forging a turn in Tacet's prompt. And for `tacet handoff`: a write
  into another agent's files without a y/N, into a session that agent has
  open, on a version outside the measured write range, or one that leaves the
  session's earlier history unloadable; a handoff record or session id that
  writes outside `<config>/handoff/`; and `tacet handoff note codex --program`
  starting anything but the file it named, or being reachable from a model
  tool. The design and the threat table are in
  [docs/adr/0001-agent-sessions.md](docs/adr/0001-agent-sessions.md).

## What is not in scope

* **The model saying something wrong.** It is a 4B model on a laptop; it is wrong
  regularly. That is a quality problem, and the eval suite is where it belongs.
* **A tool refusing something it should allow.** Annoying, not dangerous — an
  ordinary issue.
* **`shell`, `db`, `clipboard` and `http` doing what they say they do.** They are
  addons: off by default, installed deliberately, and each prints what it can
  reach before it is turned on. "The shell addon can run shell commands" is the
  feature.
* **Anything requiring an attacker who already has your user account.** If they
  can write Tacet's config directory — `$TACET_HOME` if set, else
  `$XDG_CONFIG_HOME/tacet`, else `~/.tacet` (`%APPDATA%\Tacet` on Windows) —
  they can write `~/.bashrc`. The same goes for the other agents' folders: an
  attacker who can write `~/.claude` can already change what Claude Code runs.
* **Whether reading your own Antigravity history is within Antigravity's
  terms.** Its terms (§6) forbid third-party access; Tacet reads files you own
  on your disk and writes nothing of Antigravity's by default. That is your
  decision about your agreement, not a vulnerability in Tacet.
* **What another agent does with a note it was given.** A note is written in
  the user role and says it is from Tacet; if the other agent's model then acts
  on it, that is that agent's behaviour.

## Supported versions

The latest release. This project is pre-1.0 and moves quickly; there is no
backporting. `tacet update` moves you to it, and verifies the asset digest the
GitHub API reports before it installs anything.
