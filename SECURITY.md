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
  timeout is a vulnerability.
* **Path escape.** Every tool that touches the filesystem resolves through
  `sandbox_path`. Reaching a file outside the working directory and the
  registered workspace roots — through a symlink, a `..`, a device path, a
  Windows 8.3 name, whatever — is a vulnerability.
* **Data leaving the device.** Only `tacet-web` and `tacet-mcp` may open a
  socket, and that is enforced by both crates being the only ones that declare
  `ureq`. A path that gets user data onto the network through any other route is
  a vulnerability, and so is one that bypasses the approval gate in front of an
  outbound tool once the session has touched personal data.
* **Prompt-boundary attacks with a real effect.** A remote MCP server, a fetched
  page or a file's contents causing a tool call the user did not ask for — for
  example a description that carries terminal escapes, or content that
  impersonates a `<tool_response>` fence.
* **Supply chain.** A release asset that does not match its source, or a way to
  make `tacet update` install something the GitHub API did not vouch for.

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
  can write `~/.config/tacet`, they can write `~/.bashrc`.

## Supported versions

The latest release. This project is pre-1.0 and moves quickly; there is no
backporting. `tacet update` moves you to it, and verifies the asset digest the
GitHub API reports before it installs anything.
