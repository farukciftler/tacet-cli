# tacet-mcp

Tacet's MCP (Model Context Protocol) client. Together with `tacet-web`, **one of
the two crates allowed to make a network call**. Hand-written JSON-RPC 2.0 +
Streamable HTTP + SSE; no official MCP SDK was pulled in.

**THE NAME.** The product is **Tacet**, the binary is **`tacet`**, the crate is
**`tacet-mcp`** — all the same name. For a while this paragraph said "the crate
name is an INTERNAL IDENTITY left over from the old brand and was deliberately
not changed"; that exception was removed, the internal and external names are
now the same. Not confusing the internal name with the NETWORK IDENTITY (rather
than the external NAME) still matters: see the "Network identity" heading below.

Architectural decisions: `../../../mcp-connection-spec.md` (Swift original:
`Tacet/Services/MCPClient.swift`).

## The promise

> Tacet does not go online by itself. If you connect a server, you see what is
> sent there every single time.

This crate carries the **first** half of the promise: with no connection,
nothing happens. The **second** half — "nothing goes out unseen" — is not here,
it is the deterministic approval gate in `tacet-tools::executor`. The split is
deliberate: if the network layer held its own gate, whoever changes the network
layer could change the gate too.

## Configuration — `mcp.json` in the config directory

**Empty by default.** If the file does not exist there are no connections, and
that is not an error; network traffic is zero. Tacet does NOT go looking for MCP
servers running on this machine BY ITSELF — the user writes them down by hand.

```json
{
  "connections": [
    {
      "name": "home server",
      "url": "https://example.com/mcp",
      "key": "bearer-token",
      "enabled": true
    },
    {
      "name": "local",
      "url": "http://127.0.0.1:8080/mcp",
      "key_env": "HOME_MCP_TOKEN"
    }
  ]
}
```

| Field | Required | Note |
| --- | --- | --- |
| `name` | yes | Appears at the start of the chip text ("home server · ..."), and is the prefix of the tool name |
| `url` | yes | The Streamable HTTP endpoint |
| `key` | no | Bearer token, plain text in the file |
| `key_env` | no | Read the token from this environment variable; overrides `key` |
| `enabled` | no | Defaults to `true` |

The file's location is the operating system's config directory: on Unix
`$XDG_CONFIG_HOME/tacet/mcp.json` (or `~/.tacet/mcp.json`), on Windows
`%APPDATA%\Tacet\mcp.json`. The single source of truth for the path is
`tacet_kernel::env` — the memory and skill layers point at the same directory.
`TACET_MCP_CONFIG` redirects the file somewhere else directly; an empty value
counts as "undefined" and the default path is used.

**RENAMED FIELDS.** Before the English rename this file used
`baglantilar`/`ad`/`anahtar`/`anahtar_ortam`/`etkin`, and the variable was
`TACET_MCP_YAPILANDIRMA`. The FILE's old keys are still read (each field carries
a `serde(alias)`) so an existing `mcp.json` keeps working; the ENVIRONMENT
VARIABLE has no fallback — export the new name.

**Key storage.** On iOS the token lives in the Keychain (spec §5.8); there is no
equivalent vault on the desktop. Keep the file at `chmod 600` or use `key_env` —
do not write the token into a file that falls into a git repository.

**The address rule (§3.1).** `https://` everywhere; plain `http://` ONLY on
local network addresses (`localhost`, `127.0.0.0/8`, `10/8`, `172.16/12`,
`192.168/16`, `*.local`). The rule lives in `client::validate_url`, i.e. in the
network layer itself: with an address that is not accepted, not even a client
OBJECT comes into existence.

## Network identity — what lands in the far side's log

When a connection is made to an MCP server this is the only trace the user
leaves in our name, and it is **the product name, not the crate name**:

| Where | Value |
| --- | --- |
| HTTP `User-Agent` | `tacet/1.0` |
| `initialize` -> `clientInfo.name` | `tacet` |

A third party's log file is a place we cannot take anything back from: a wrong
name, once written, cannot be corrected. That is why the assertion is measured
OVER THE WIRE rather than by looking at the code — `tests/local_server.rs` reads
both the header and the `clientInfo` on the fake server and compares them, with
both transport formats. Reverting the constant to the old name turns the test
red (measured: `cargo test -p tacet-mcp`).

## Wiring (skip it and the mechanism stays dead)

On the Swift side `MCPTool` was written, compiled and **never instantiated**. So
as not to fall into the same mistake, the wiring is in one place and marked
`#[must_use]`:

```rust
use tacet_tools::mcp;

let loading = mcp::load_default();                  // config directory + mcp.json; silently empty if missing
let mcp_names = mcp::feed_catalog(&mut catalog, &loading);
let executor = mcp::bind_executor(ToolExecutor::new(catalog), &mcp_names);
```

Skip the third line and the tools APPEAR in the catalog but do not enter the
approval gate's `external_tools` list: in a tainted session data goes out
without being asked about. The `an_unbound_mcp_tool_never_triggers_the_gate`
test in `crates/tacet-tools/src/mcp.rs` exists precisely to make that loss
visible.

`loading.connection_errors`, `loading.skipped` and `loading.notes` must be shown
to the user — none of them may be swallowed silently.

## The tool bridge: the schema narrowing policy

MCP servers write full JSON Schema; our `ArgSchema` is deliberately closed and
small (the condition for it to be compilable into a grammar). When the incoming
schema is wider:

| Incoming | Decision | Why |
| --- | --- | --- |
| `["string","null"]` | narrow to `Text` | In JSON null = "field absent"; `validate` behaves the same way, no loss |
| single-branch `anyOf`/`oneOf` | descend into that branch | Not a choice, an unnecessary wrapper |
| `pattern`, `format`, `minLength`, `multipleOf`, `uniqueItems` | drop, **record it** | Widening is safe: the server validates for itself and the rejection comes back to the model as a normal tool error |
| multi-branch `oneOf`/`anyOf`, `allOf`, `not` | **skip the tool** | No equivalent in the closed subset |
| `$ref` / `$defs` | **skip the tool** | Requires resolution |
| `additionalProperties` with a schema | **skip the tool** | Would let the model invent fields |
| deeper than 3 levels | **skip the tool** | §5.2 schema depth filter |
| typeless field, array without an element, mixed `enum` | **skip the tool** | The grammar cannot know what to produce |

**An untranslatable tool is not accepted silently.** A wrongly narrowed schema
means the grammar FORCES the model into a shape the server will reject, and
nobody can see it — a silent breakage is the worst breakage.

Long tool descriptions are truncated at `DESCRIPTION_LIMIT` (160 characters):
first the first sentence, failing that a cut at a word boundary plus "…". Swift
had the on-device model summarize them; here it is **deterministic** — calling a
model to summarize makes the tool catalog depend on model quality, and the same
server would produce a different definition on every launch (eval would become
incomparable).

## Output and the 4096 bypass channel (§5.5)

MCP output never enters the context raw:

- ≤ 800 characters: as is.
- Long: all of it into the `DataStore`, and to the model the **last 30 lines** +
  a `source_ref`. The tail was chosen because in command/log output the error
  lives at the END.
- `isError: true` is the server's own tool error, not a transport failure: it is
  told to the model as `tool_error: ...` and is NOT TRANSLATED into
  `ERROR_MODEL_TEXT`.

Transport errors go through two channels: a plain sentence to the user, the
fixed `tool_failed: ...` to the model. The raw `ureq`/server text does not leak
to the model.

## Testing

```sh
cargo test -p tacet-mcp                       # unit + end to end (local socket)
cargo test -p tacet-tools mcp::               # bridge + approval gate
TACET_MCP_TEST_URL=https://... \
  cargo test -p tacet-mcp -- --ignored        # goes to a REAL server
```

`tests/local_server.rs` brings up its own MCP server on `127.0.0.1` and runs the
`initialize -> tools/list -> tools/call` flow over a real socket **with both
transport formats** (plain JSON and SSE). The only test that goes online is
`#[ignore]`.

## Outside the v1 scope

The `resources`/`prompts` capabilities; the stdio transport (in the macOS turn);
an "always allow" mode (spec §3.6 — the gate cannot be switched off in this
turn); approval remembered per resource; servers with an OAuth flow.
