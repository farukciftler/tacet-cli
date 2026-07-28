//! The addon registry — which addon is INSTALLED, which one is OPEN, and what
//! its settings are.
//!
//! CLOSED BY DEFAULT. If the file does not exist the registry is EMPTY, and an
//! empty registry means "no addons at all". The "data does not leave the
//! device" default lives exactly here: the `web_search` tool DOES NOT APPEAR in
//! the catalog unless there is an OPEN record in this file (see
//! `tacet-tools/src/catalog.rs`), so the model cannot even call it.
//!
//! WHY IN THIS CRATE: two separate sides read the registry — the shell that
//! runs the commands (`tacet-cli`) and the tool layer that builds the catalog
//! gate (`tacet-tools`). The shell depends on the tool layer, not the other way
//! round; so the registry has to live in a crate BELOW both. `tacet-kernel` was a
//! candidate too, but the rule there is "no work happens in this crate" (no
//! file reads, no directory creation), while the registry writes to disk. What
//! is left is `tacet-web`: the first addon kind was the very thing that opens
//! this crate's network surface, and address validation
//! (`client::address_is_valid`) is defined here.
//!
//! MANY ADDONS, ONE DEFINITION TABLE. The registry file has always held a LIST,
//! but every path that used it asked one question — "is it `web-search`?" — so a
//! second addon was impossible to add without editing the install flow, the
//! gate and the listing in three places. What an addon IS now lives in one
//! place, `DEFINITIONS`: its name, what it is for, which tools it puts in the
//! catalog, whether it opens a network surface, and what the install has to ask.
//! The shell walks that table; it does not know any addon by name.
//!
//! ONE GATE FUNCTION. Whether an addon may act is asked through `is_open(name)`
//! and nowhere else. A gate copied into each tool is a gate that DIVERGES: one
//! copy learns that an unreadable registry means closed and the other does not.
//! `web_search_is_open()` survives only as a named shorthand for
//! `is_open(WEB_SEARCH)`.
//!
//! NO NETWORK. This file OPENS NO SOCKET; it only reads and writes a local JSON
//! file. The side that goes online is `client.rs`, and only the ADDRESS is
//! passed to it from here.
//!
//! ON FAILURE IT FALLS CLOSED. A corrupt registry file means "false" for
//! `web_search_is_open()`: interpreting an unreadable configuration as "it was
//! probably open" would carry the risk of sending the user's query to a server
//! they do not know about. The corruption itself does not stay silent — `read()`
//! returns `Err` and `tacet addon list` prints it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The registry's name inside the config directory.
pub const REGISTRY_FILE: &str = "addons.json";

/// LEGACY REGISTRY NAME. Files written before the English rename are called
/// `eklentiler.json`. It is read as a fallback ONLY when the new name is
/// absent; nothing ever writes to it. Removing this line would leave an
/// existing user with web search silently switched off — the failure would look
/// exactly like "the addon was never installed".
pub const LEGACY_REGISTRY_FILE: &str = "eklentiler.json";

/// The web search addon's name AND kind. Today they are the same string, but
/// the fields are separate: a user may later want two records of the same kind
/// (`work-searx`, `home-searx`) — the name diverges then, the kind stays.
pub const WEB_SEARCH: &str = "web-search";

/// Runs commands from a list the user approved at install time.
pub const SHELL: &str = "shell";

/// Opens named directories to the file tools.
pub const WORKSPACE: &str = "workspace";

/// Plain HTTP requests to an approved host list.
pub const HTTP: &str = "http";

/// A database connection. Its setting CARRIES A PASSWORD — see `Setting::secret`.
pub const DB: &str = "db";

/// The system clipboard. Asks nothing at install time.
pub const CLIPBOARD: &str = "clipboard";

/// LEGACY ADDON NAME. Registries written before the rename hold `web-arama`;
/// `parse` maps it onto `WEB_SEARCH` so an existing installation keeps working.
pub const LEGACY_WEB_SEARCH: &str = "web-arama";

/// The key of the base address inside `settings`.
pub const ADDRESS_KEY: &str = "address";

/// The allowed command list (`shell`).
pub const COMMANDS_KEY: &str = "commands";

/// The opened directories (`workspace`).
pub const DIRECTORIES_KEY: &str = "directories";

/// The allowed host list (`http`).
pub const HOSTS_KEY: &str = "hosts";

/// LEGACY SETTING KEY (`adres`), accepted on read for the same reason as
/// `LEGACY_WEB_SEARCH`.
pub const LEGACY_ADDRESS_KEY: &str = "adres";

/// A setting that holds MANY values keeps them in one string, separated by
/// NEWLINES.
///
/// NOT A COMMA — and that is a measured constraint, not taste: a comma is a
/// legal character in a directory name on every platform this runs on, so a
/// comma-separated list cannot represent `/home/me/notes,drafts` at all. A
/// newline cannot occur in a value here because every value arrives one line at
/// a time (the prompt reads lines) and `Shape::check` refuses control
/// characters outright.
pub const VALUE_SEPARATOR: char = '\n';

/// THE INSTALL TIME IS NOT KEPT. The registry file is compared byte for byte in
/// tests; a timestamp inside it would make the same input produce two different
/// files, and the comparison would either become fragile or have to strip the
/// stamp out. The install time also feeds no decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Addon {
    pub name: String,
    pub kind: String,
    /// `false` = installed but CLOSED. In the registry file `"state": "closed"`.
    pub open: bool,
    /// Kind-specific settings. `BTreeMap`, because the output must be
    /// DETERMINISTIC: with a `HashMap` the same registry produces a different
    /// key order on every write and the user's file looks like it changed for
    /// no reason.
    pub settings: BTreeMap<String, String>,
}

impl Addon {
    pub fn new(name: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: kind.into(),
            open: true,
            settings: BTreeMap::new(),
        }
    }

    pub fn with_setting(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.settings.insert(key.into(), value.into());
        self
    }

    pub fn setting(&self, key: &str) -> Option<&str> {
        self.settings.get(key).map(String::as_str)
    }

    /// A many-valued setting, split back into its values (see
    /// `VALUE_SEPARATOR`). A missing setting is an EMPTY LIST, not an error: an
    /// allow-list nobody filled in allows nothing, which is the safe reading.
    pub fn values(&self, key: &str) -> Vec<&str> {
        self.setting(key)
            .map(|v| {
                v.split(VALUE_SEPARATOR)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn state_text(&self) -> &'static str {
        if self.open { "open" } else { "closed" }
    }
}

/// Joins many values into one setting string. The inverse of `Addon::values`.
pub fn join_values<S: AsRef<str>>(values: &[S]) -> String {
    values
        .iter()
        .map(|v| v.as_ref().trim())
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>()
        .join(&VALUE_SEPARATOR.to_string())
}

/// The whole registry. Kept UNIQUE by name and SORTED by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Record {
    addons: Vec<Addon>,
}

impl Record {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn all(&self) -> &[Addon] {
        &self.addons
    }

    pub fn is_empty(&self) -> bool {
        self.addons.is_empty()
    }

    pub fn find(&self, name: &str) -> Option<&Addon> {
        self.addons.iter().find(|a| a.name == name)
    }

    /// Installed AND open. The only question the catalog gate asks.
    pub fn is_open(&self, name: &str) -> bool {
        self.find(name).is_some_and(|a| a.open)
    }

    /// Adds, or replaces the record with the SAME NAME. A second `web-search`
    /// record would leave the question of which one is in force unanswered.
    pub fn add(&mut self, addon: Addon) {
        match self.addons.iter_mut().find(|a| a.name == addon.name) {
            Some(existing) => *existing = addon,
            None => self.addons.push(addon),
        }
        self.addons.sort_by(|a, b| a.name.cmp(&b.name));
    }

    /// Deletes a record. `false` = there was no such record.
    pub fn delete(&mut self, name: &str) -> bool {
        let before = self.addons.len();
        self.addons.retain(|a| a.name != name);
        before != self.addons.len()
    }

    /// Opens/closes a record. `None` = there is no such record.
    pub fn set_state(&mut self, name: &str, open: bool) -> Option<&Addon> {
        let a = self.addons.iter_mut().find(|a| a.name == name)?;
        a.open = open;
        Some(a)
    }

    /// The text written to disk. The format is built with `serde_json`, NOT BY
    /// HAND: writing the escaping rules by hand (`&` in an address, non-ASCII
    /// characters) is precisely a job that has already broken once in this
    /// repository.
    pub fn json(&self) -> String {
        let array: Vec<serde_json::Value> = self
            .addons
            .iter()
            .map(|a| {
                let settings: serde_json::Map<String, serde_json::Value> = a
                    .settings
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                    .collect();
                serde_json::json!({
                    "name": a.name,
                    "kind": a.kind,
                    "state": a.state_text(),
                    "settings": settings,
                })
            })
            .collect();
        let root = serde_json::json!({ "addons": array });
        // A trailing newline: so the file does not eat the shell prompt when
        // it is `cat`ed.
        format!(
            "{}\n",
            serde_json::to_string_pretty(&root).unwrap_or_else(|_| "{}".into())
        )
    }

    /// SEPARATE AND PUBLIC: so it can be tested without touching the
    /// filesystem (the same pattern as `model_package::parse_remote_catalog`).
    ///
    /// AN UNKNOWN STATE STRING IS AN ERROR. Silently treating the record of a
    /// user who wrote "clsoed" instead of "closed" as "open" would leave the
    /// addon on while the user believed they had switched it off.
    ///
    /// LEGACY KEYS ARE ACCEPTED: registries written before the English rename
    /// use `eklentiler`/`ad`/`tur`/`durum`/`ayarlar` and the values
    /// `acik`/`kapali`, plus the addon name `web-arama` and the setting key
    /// `adres`. They are mapped onto the new names on read; nothing writes them
    /// back out. Dropping this mapping switches web search off silently for
    /// every existing user.
    pub fn parse(raw: &str) -> Result<Record, String> {
        let root: serde_json::Value =
            serde_json::from_str(raw).map_err(|e| format!("could not read JSON: {e}"))?;
        let array = root
            .get("addons")
            .or_else(|| root.get("eklentiler"))
            .and_then(|a| a.as_array())
            .ok_or_else(|| "no `addons` array".to_string())?;
        let mut record = Record::empty();
        for a in array {
            let raw_name = a
                .get("name")
                .or_else(|| a.get("ad"))
                .and_then(|n| n.as_str())
                .ok_or_else(|| "the addon has no `name` field".to_string())?;
            let name = if raw_name == LEGACY_WEB_SEARCH {
                WEB_SEARCH
            } else {
                raw_name
            };
            let raw_kind = a
                .get("kind")
                .or_else(|| a.get("tur"))
                .and_then(|k| k.as_str())
                .unwrap_or(raw_name);
            let kind = if raw_kind == LEGACY_WEB_SEARCH {
                WEB_SEARCH
            } else {
                raw_kind
            };
            let open = match a
                .get("state")
                .or_else(|| a.get("durum"))
                .and_then(|s| s.as_str())
            {
                None | Some("open") | Some("acik") => true,
                Some("closed") | Some("kapali") => false,
                Some(s) => return Err(format!("'{name}': unknown state '{s}'")),
            };
            let mut settings = BTreeMap::new();
            if let Some(object) = a
                .get("settings")
                .or_else(|| a.get("ayarlar"))
                .and_then(|s| s.as_object())
            {
                for (key, value) in object {
                    let v = value
                        .as_str()
                        .ok_or_else(|| format!("'{name}': setting `{key}` must be text"))?;
                    let key = if key == LEGACY_ADDRESS_KEY {
                        ADDRESS_KEY.to_string()
                    } else {
                        key.clone()
                    };
                    settings.insert(key, v.to_string());
                }
            }
            record.add(Addon {
                name: name.to_string(),
                kind: kind.to_string(),
                open,
                settings,
            });
        }
        Ok(record)
    }
}

// ---------------------------------------------------------------------------
// What an addon IS — the definition table
// ---------------------------------------------------------------------------

/// The SHAPE a setting's value must have — the SCHEMA gate of the four
/// (name → schema → approval → cancellation).
///
/// A CLOSED ENUM, NOT A REGEX PER CALL SITE. What can be made impossible by the
/// shape is not filtered out of free text later: `CommandName` cannot express
/// `rm -rf /; curl evil` because a command name has no room for a space, a
/// semicolon or a slash — there is no metacharacter left to escape. The rule
/// this repo already paid for is that a text filter is a list of the tricks
/// somebody thought of, while a shape is the set of things that can exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// A base URL. THE RULE IS NOT REWRITTEN HERE: it is
    /// `client::address_is_valid` (https everywhere, plain http only on a local
    /// network), the same gate the web client uses.
    Address,
    /// A BARE command name — `git`, `ls`, `rg`. No path, no arguments, no
    /// metacharacters.
    CommandName,
    /// An ABSOLUTE directory path with no `..` in it. Whether it EXISTS is not
    /// asked here (this crate does no filesystem work); the shell asks that at
    /// install time and `tacet_tools::sandbox_path` is the gate at run time.
    Directory,
    /// A host name: no scheme, no port, no path, no credential.
    Host,
}

impl Shape {
    /// `Ok(())` = the value may be stored. The message is written for the user
    /// who typed it, so it says what was wrong AND what is accepted.
    pub fn check(self, value: &str) -> Result<(), String> {
        let v = value.trim();
        if v.is_empty() {
            return Err("empty value".to_string());
        }
        // BEFORE ANYTHING ELSE, for every shape: a control character in a
        // setting is either a mistake or an attempt to smuggle a second value
        // past `VALUE_SEPARATOR` (a newline inside one entry would split it into
        // two allow-list entries on the next read).
        if v.chars().any(|c| c.is_control()) {
            return Err("a control character is not allowed in a setting".to_string());
        }
        match self {
            Shape::Address => crate::address_is_valid(v).map_err(|e| e.to_string()),
            Shape::CommandName => check_command_name(v),
            Shape::Directory => check_directory(v),
            Shape::Host => check_host(v),
        }
    }
}

/// A command name is LETTERS, DIGITS, `-`, `_`, `.` and nothing else.
///
/// WHY SO NARROW. The list is what the user allows the model to run, so every
/// character admitted here is a character that reaches a process launcher. A
/// space would let one entry become "a command plus its arguments"; a `/` would
/// let the entry point outside `PATH` (`/tmp/x`); `;`, `|`, `&`, `$`, a
/// backtick, a quote or a newline are what turn one command into two on any
/// path that ever hands the string to a shell. None of them are needed to name
/// a program.
fn check_command_name(v: &str) -> Result<(), String> {
    if v == "." || v == ".." || v.starts_with('-') {
        // A leading `-` would be read as an option by whatever runs it.
        return Err(format!("'{v}' is not a command name"));
    }
    let bad: String = v
        .chars()
        .filter(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.'))
        .collect();
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "'{v}': a BARE command name is expected (letters, digits, - _ .) — \
             '{bad}' is not allowed; arguments and paths are not written here"
        ))
    }
}

/// An absolute path, with no `..` component.
///
/// `..` IS REFUSED AT THE SOURCE. `/home/me/work/..` names the parent, so an
/// entry like that quietly widens the opened area to everything beside it. The
/// run-time gate (`tacet_tools::sandbox_path`) resolves symlinks and is the
/// authority; this check exists so a widening entry is never STORED in the
/// first place — a registry a human reads should say what it means.
fn check_directory(v: &str) -> Result<(), String> {
    let path = Path::new(v);
    if !path.is_absolute() {
        return Err(format!(
            "'{v}': an absolute path is expected (starting from the root, not '~' or a relative name)"
        ));
    }
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(format!(
            "'{v}': '..' is not allowed — write the directory you mean, not a way up out of it"
        ));
    }
    Ok(())
}

/// A host name and NOTHING ELSE.
///
/// A scheme, a port, a path or a `user@` in an allow-list entry is how an entry
/// stops meaning what it looks like: `https://example.com/ok` as an "allowed
/// host" either never matches or matches by accident. It is refused so the list
/// stays a list of hosts.
fn check_host(v: &str) -> Result<(), String> {
    if v.contains("://") {
        return Err(format!("'{v}': write the HOST only, without a scheme"));
    }
    for (character, what) in [
        ('/', "a path"),
        ('@', "a credential"),
        (':', "a port"),
        ('*', "a wildcard"),
        ('?', "a query"),
        (' ', "a space"),
    ] {
        if v.contains(character) {
            return Err(format!("'{v}': {what} is not written in a host entry"));
        }
    }
    if v.starts_with('.') || v.ends_with('.') || v.contains("..") {
        return Err(format!("'{v}': the dots in the host name are misplaced"));
    }
    let bad: String = v
        .chars()
        .filter(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '.'))
        .collect();
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "'{v}': a host name holds letters, digits, '-' and '.' — '{bad}' is not allowed"
        ))
    }
}

/// One question the install asks.
#[derive(Debug, Clone, Copy)]
pub struct Setting {
    pub key: &'static str,
    /// What the user is asked, in one line.
    pub prompt: &'static str,
    /// The sentence under the prompt — an example, and what the value costs.
    pub help: &'static str,
    pub shape: Shape,
    /// MANY values, one per line (see `VALUE_SEPARATOR`).
    pub many: bool,
    /// The value is a SECRET: it is not echoed back by `addon list`, not by
    /// `addon list --json`, and not by `addon try --json`. The registry file is
    /// already 0600; what leaks a password is the terminal output people paste
    /// into a bug report.
    pub secret: bool,
    /// Without it the addon cannot be installed.
    pub required: bool,
}

/// What an addon IS. The single place the shell reads to install, list and gate
/// one.
#[derive(Debug, Clone, Copy)]
pub struct Definition {
    pub name: &'static str,
    /// One line, told to the user in `addon list`.
    pub summary: &'static str,
    /// The tools this addon puts in the catalog while it is OPEN. MAY BE EMPTY:
    /// `workspace` adds no tool, it widens the reach of the file tools that are
    /// already there.
    ///
    /// THIS FIELD DOES NOT ENFORCE THE GATE, it explains it. The catalog asks
    /// `is_open(<name const>)` and adds the tool itself; a name typed wrong here
    /// costs a wrong line in `addon list` and nothing more. Were the catalog to
    /// look tools up BY NAME through this list instead, a typo would make an
    /// unknown tool "belong to no addon" and it would appear ungated — a
    /// fail-OPEN, which is why the lookup is not built that way.
    ///
    /// THE NAMES ARE THE TOOL LAYER'S, NOT OURS. They are written here to match
    /// what the tool modules call themselves (`tacet_tools::shell` answers
    /// "shell", `http_call` answers "http"); when a tool is renamed this row has
    /// to move with it, and the only cost of missing that is a wrong line in
    /// `addon list`.
    pub tools: &'static [&'static str],

    /// WHAT CHANGES FOR THE MODEL when this addon is open, in one sentence.
    ///
    /// A SENTENCE AND NOT A COMPUTED LIST, because the answer is not always "a
    /// tool appeared": `workspace` adds nothing to the catalog and instead
    /// widens where the existing file tools may look. A line assembled from
    /// `tools` would have to say "the  tools are in the catalog" for that one —
    /// a true-looking sentence about nothing.
    pub effect: &'static str,
    /// Does opening it open a NETWORK surface. Said out loud in `addon list`,
    /// because that is the one property this project's default is about.
    pub network: bool,
    /// Shown BEFORE the approval question at install time — what the user is
    /// agreeing to.
    pub warning: &'static str,
    pub settings: &'static [Setting],
}

impl Definition {
    pub fn setting(&self, key: &str) -> Option<&'static Setting> {
        self.settings.iter().find(|s| s.key == key)
    }
}

/// EVERY addon this build knows how to install.
///
/// Adding a sixth means adding a row here and nothing else on this side; the
/// tool itself and the catalog line are the tool layer's job.
pub const DEFINITIONS: &[Definition] = &[
    Definition {
        name: CLIPBOARD,
        summary: "reads and writes the system clipboard",
        tools: &["clipboard"],
        effect: "the `clipboard` tool is in the catalog.",
        network: false,
        warning: "the clipboard often holds what was last copied from a password manager.",
        settings: &[],
    },
    // NO SETTING, AND `network: false` — BOTH MATCH THE TOOL THAT ACTUALLY
    // SHIPS. This entry once asked for a connection string, called it a secret,
    // and warned "data leaves this machine". The tool behind it (`db.rs`) is
    // SQLite-only by an argued decision: it opens no socket, and it reads `.db`
    // files inside the working directory. So the screen was mis-pricing the
    // risk in both directions — it made the user approve a network they were
    // never given, and it parked a real `postgres://user:pass@…` password on
    // disk for a reader that does not exist (`CONNECTION_KEY` was read nowhere
    // outside this file). A consent screen that describes a different product
    // than the one installed is the same failure as a half-built gate.
    Definition {
        name: DB,
        summary: "reads SQLite database files in the working directory",
        tools: &["db"],
        effect: "the `db` tool is in the catalog.",
        network: false,
        warning: "queries are READ-ONLY and reach only `.db` files the working directory \
                  already lets Tacet see. Needs the `sqlite3` command on this machine.",
        settings: &[],
    },
    Definition {
        name: HTTP,
        summary: "makes HTTP requests to hosts you approved",
        tools: &["http"],
        effect: "the `http` tool is in the catalog.",
        network: true,
        warning: "a request leaves this machine. Only the hosts listed here can be reached.",
        settings: &[Setting {
            key: HOSTS_KEY,
            prompt: "allowed hosts",
            // "SUBDOMAINS ARE COVERED TOO" USED TO STAND HERE AND IT WAS FALSE.
            // The gate is whole-string equality (`http::HttpClient::check`), on
            // purpose: a suffix match hands `evil-api.example.com` the same pass
            // as `api.example.com`. The install screen has to describe the gate
            // that exists, or the user configures `example.com` and cannot work
            // out why every call is refused.
            help: "one exact host per line, no scheme: api.example.com \
                   (a subdomain is a separate entry)",
            shape: Shape::Host,
            many: true,
            secret: false,
            required: true,
        }],
    },
    Definition {
        name: SHELL,
        summary: "runs commands from a list you approve",
        tools: &["shell"],
        effect: "the `shell` tool is in the catalog.",
        network: false,
        warning: "these commands run on your machine with your account's rights.",
        settings: &[Setting {
            key: COMMANDS_KEY,
            prompt: "allowed commands",
            help: "one BARE command name per line: git, ls, rg — no arguments, no paths",
            shape: Shape::CommandName,
            many: true,
            secret: false,
            required: true,
        }],
    },
    Definition {
        name: WEB_SEARCH,
        summary: "searches the web through your own SearXNG server",
        tools: &["web_search", "web_fetch"],
        effect: "the `web_search` and `web_fetch` tools are in the catalog.",
        network: true,
        warning: "the query leaves this machine and goes to the server named below.",
        settings: &[Setting {
            key: ADDRESS_KEY,
            prompt: "SearXNG address",
            help: "https://… or http://localhost:8888",
            shape: Shape::Address,
            many: false,
            secret: false,
            required: true,
        }],
    },
    Definition {
        name: WORKSPACE,
        // NO TOOL OF ITS OWN. `tacet_tools::workspace` widens the roots the
        // EXISTING file tools may reach (`read_document`, `find_file`, …); the
        // catalog gains nothing when it opens, the file tools gain reach.
        summary: "opens named directories to the file tools",
        tools: &[],
        effect: "the file tools can reach the directories named here, on top of the working directory.",
        network: false,
        warning: "everything under these directories becomes readable by the model.",
        settings: &[Setting {
            key: DIRECTORIES_KEY,
            prompt: "directories to open",
            help: "one absolute path per line: /Users/you/notes",
            shape: Shape::Directory,
            many: true,
            secret: false,
            required: true,
        }],
    },
];

/// The definition of an installable addon. `None` = this build does not know
/// that name.
pub fn definition(name: &str) -> Option<&'static Definition> {
    DEFINITIONS.iter().find(|d| d.name == name)
}

/// Which addon a tool belongs to — FOR EXPLAINING, not for gating (see
/// `Definition::tools`).
pub fn provides(tool: &str) -> Option<&'static Definition> {
    DEFINITIONS.iter().find(|d| d.tools.contains(&tool))
}

/// Every installable name, in the order they are listed.
pub fn installable_names() -> Vec<&'static str> {
    DEFINITIONS.iter().map(|d| d.name).collect()
}

/// Must this setting's value be kept off the screen.
///
/// TWO REASONS, and the second is the one that matters: the definition may say
/// so, OR the key NAME may look like a credential. The second covers a registry
/// a user (or a future addon) wrote by hand — printing an unknown `token` field
/// in full because no definition claimed it is exactly how a secret escapes.
pub fn setting_is_secret(addon_name: &str, key: &str) -> bool {
    if let Some(spec) = definition(addon_name).and_then(|d| d.setting(key))
        && spec.secret
    {
        return true;
    }
    let k = key.to_ascii_lowercase();
    ["password", "secret", "token", "connection", "apikey"]
        .iter()
        .any(|needle| k.contains(needle))
        // `key` on its own, and `_key`/`key_` compounds — but not `keyword`.
        || k == "key"
        || k.ends_with("_key")
        || k.starts_with("key_")
}

/// What is printed INSTEAD of a secret.
///
/// The LENGTH IS NOT LEAKED either (a fixed number of dots): the length of a
/// connection string narrows a guess, and it buys the reader nothing.
pub fn masked() -> &'static str {
    "•••••••• (hidden)"
}

/// The value as it may be SHOWN.
pub fn shown_value(addon_name: &str, key: &str, value: &str) -> String {
    if setting_is_secret(addon_name, key) {
        masked().to_string()
    } else {
        value.to_string()
    }
}

// ---------------------------------------------------------------------------
// The allow-list matcher (`http`)
// ---------------------------------------------------------------------------

/// Is `host` covered by the stored allow-list.
///
/// AN ENTRY MATCHES ITSELF AND ITS SUBDOMAINS, AT THE DOT: `example.com` covers
/// `example.com` and `api.example.com`, and covers NEITHER `notexample.com` NOR
/// `example.com.evil.net`. Written with `ends_with` alone it would cover the
/// first, written with `contains` it would cover both — and this repository has
/// already shipped that exact bug once, in the SSRF gate, where a prefix
/// comparison made `10.evil.com` look like the private `10.` range.
///
/// THIS IS A NARROWING, NOT THE SSRF GATE. Passing here does not make an
/// address safe to fetch: `client::target_is_public` still has to say yes, and
/// it is the one that resolves the name and judges the ADDRESS. An allow-list
/// entry can point at loopback; that gate is what stops it.
pub fn host_is_allowed(allow_list: &str, host: &str) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return false;
    }
    allow_list
        .split(VALUE_SEPARATOR)
        .map(|e| e.trim().trim_end_matches('.').to_ascii_lowercase())
        .filter(|e| !e.is_empty())
        .any(|entry| {
            host == entry || {
                // The dot is part of the SUFFIX, so the boundary is a real
                // label boundary and not a coincidence of letters.
                let suffix = format!(".{entry}");
                host.ends_with(&suffix)
            }
        })
}

/// The host of a URL, lower-cased, brackets stripped from an IPv6 literal.
///
/// A DUPLICATE, AND SAID SO. `client::host_and_port` does the same parse and is
/// private; this arm may not widen it. The rules are copied EXACTLY — the host
/// is what follows the LAST `@` (so `https://allowed.example@evil.test/` is
/// `evil.test`, which is what a connection would do, and reading it the other
/// way round would let an allow-list be walked straight past). If that function
/// is ever made public, this one is to be deleted rather than kept in step by
/// hand.
pub fn url_host(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    if authority.is_empty() {
        return None;
    }
    if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, _) = bracketed.split_once(']')?;
        return Some(host.to_ascii_lowercase());
    }
    let host = match authority.rsplit_once(':') {
        Some((host, _)) if !host.is_empty() => host,
        _ => authority,
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// The full path of the registry file. `None` = the config directory could not
/// be resolved.
///
/// LEGACY FALLBACK: if `addons.json` does not exist but `eklentiler.json` does,
/// the old path is returned. Otherwise a user who installed the addon before
/// the rename would find web search silently off, with a "the addon was never
/// installed" symptom that points nowhere.
pub fn registry_path() -> Option<PathBuf> {
    let current = tacet_kernel::env::config_path(REGISTRY_FILE)?;
    if current.exists() {
        return Some(current);
    }
    let legacy = tacet_kernel::env::config_path(LEGACY_REGISTRY_FILE)?;
    if legacy.exists() {
        return Some(legacy);
    }
    Some(current)
}

/// Reads the registry. If the file DOES NOT EXIST an empty registry is returned
/// (not an error: an installation with no addons is the normal state). If the
/// file EXISTS but is corrupt, `Err` — it is not swallowed silently.
pub fn read() -> Result<Record, String> {
    match registry_path() {
        Some(p) => read_from_path(&p),
        None => Ok(Record::empty()),
    }
}

pub fn read_from_path(path: &Path) -> Result<Record, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Record::empty()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if raw.trim().is_empty() {
        return Ok(Record::empty());
    }
    Record::parse(&raw).map_err(|e| format!("{}: {e}", path.display()))
}

/// Writes the registry to disk; creates the config directory if it is missing.
pub fn write(record: &Record) -> Result<PathBuf, String> {
    let path = registry_path().ok_or_else(|| {
        "could not resolve the config directory (TACET_HOME can be set)".to_string()
    })?;
    write_to_path(&path, record)?;
    Ok(path)
}

/// THE SAME 0700/0600 RULE AS THE REST OF THE CONFIG DIRECTORY.
///
/// This file records the SearXNG address, and an address is allowed to carry a
/// port and a path that say where a private service lives. The old pair
/// (`fs::create_dir_all` followed by `fs::write`) left it at 0755/0644
/// (measured), so every other local account could read it, while `memory.json`
/// next to it was 0600 — one directory, two standards, and the weaker one is
/// the one an attacker uses. The rule lives in `tacet_kernel::fs` so that there
/// is ONE copy of it, not a third.
pub fn write_to_path(path: &Path, record: &Record) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        tacet_kernel::create_private_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    tacet_kernel::write_private(path, record.json().as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// THE CATALOG GATE — THE ONE FUNCTION. Whether an addon's tools appear in the
/// catalog depends on this returning `true` (see `tacet_tools::catalog`).
///
/// EVERY tool asks it the same way: `addon::is_open(addon::SHELL)`. No tool
/// writes its own version of this check — a gate that is copied is a gate that
/// eventually disagrees with itself about what an unreadable registry means.
///
/// A read error = CLOSED (see the top of the file). Also CLOSED for a name this
/// build has never heard of: an unknown name in the registry cannot open
/// anything, because nothing is registered under it.
pub fn is_open(name: &str) -> bool {
    read().map(|r| r.is_open(name)).unwrap_or(false)
}

/// Is it INSTALLED at all — open or closed. The two states get different
/// sentences in the shell ("not installed" vs "closed"), so they need different
/// questions.
pub fn is_installed(name: &str) -> bool {
    read().map(|r| r.find(name).is_some()).unwrap_or(false)
}

/// `is_open(WEB_SEARCH)`, under the name the catalog already calls.
pub fn web_search_is_open() -> bool {
    is_open(WEB_SEARCH)
}

/// The base address the search will go to.
///
/// ORDER: environment variable > registry. The variable comes FIRST because it
/// is the user's EXPLICIT request for that particular run (developer shell, a
/// one-off experiment); the registry is the persistent preference. If neither
/// exists it returns `None` and the client raises a "server not configured"
/// error — THERE IS NO ADDRESS BAKED INTO THE CODE.
pub fn web_address() -> Option<String> {
    if let Some(v) = tacet_kernel::env_var(crate::client::ADDRESS_VARIABLE) {
        let v = v.to_string_lossy().trim().to_string();
        if !v.is_empty() {
            return Some(v);
        }
    }
    read()
        .ok()?
        .find(WEB_SEARCH)?
        .setting(ADDRESS_KEY)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tacet-addon-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d.join(REGISTRY_FILE)
    }

    #[test]
    fn an_empty_registry_counts_no_addon_as_open() {
        let r = Record::empty();
        assert!(!r.is_open(WEB_SEARCH));
        assert!(r.is_empty());
    }

    /// The registry names where a private search service lives; it must not be
    /// readable by the other local accounts. The leftover file is deliberately
    /// born 0644 first, so this measures the NARROWING and not just the umask —
    /// putting `fs::write` back turns it red.
    #[cfg(unix)]
    #[test]
    fn the_registry_is_not_readable_by_other_accounts() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("tacet-addon-perm-{}", std::process::id()));
        // An ALREADY OPEN directory from an earlier install: the fix has to
        // narrow what is there, not only what it creates.
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = dir.join("addons.json");
        std::fs::write(&path, "{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let mut r = Record::empty();
        r.add(Addon::new(WEB_SEARCH, WEB_SEARCH).with_setting(ADDRESS_KEY, "https://example.test"));
        write_to_path(&path, &r).unwrap();

        let file = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        let folder = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(file, 0o600, "addons.json is readable by others: {file:o}");
        assert_eq!(folder, 0o700, "the config directory is open: {folder:o}");
        // Narrowing must not have cost us the contents.
        assert_eq!(read_from_path(&path).unwrap(), r);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_written_registry_reads_back_identically() {
        let path = temp_path("round-trip");
        let mut r = Record::empty();
        r.add(
            Addon::new(WEB_SEARCH, WEB_SEARCH)
                .with_setting(ADDRESS_KEY, "https://example.test/searxng"),
        );
        write_to_path(&path, &r).unwrap();
        let back = read_from_path(&path).unwrap();
        assert_eq!(back, r);
        assert!(back.is_open(WEB_SEARCH));
        assert_eq!(
            back.find(WEB_SEARCH).unwrap().setting(ADDRESS_KEY),
            Some("https://example.test/searxng")
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_missing_file_is_an_empty_registry_not_an_error() {
        let path = std::env::temp_dir().join("tacet-addon-no-such-file.json");
        std::fs::remove_file(&path).ok();
        assert_eq!(read_from_path(&path).unwrap(), Record::empty());
    }

    #[test]
    fn a_corrupt_file_is_not_swallowed_silently() {
        let path = temp_path("corrupt");
        std::fs::write(&path, "{ this is not json").unwrap();
        assert!(read_from_path(&path).is_err());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_closed_record_does_not_count_as_open() {
        let r = Record::parse(
            r#"{"addons":[{"name":"web-search","kind":"web-search","state":"closed",
                "settings":{"address":"https://example.test"}}]}"#,
        )
        .unwrap();
        assert!(r.find(WEB_SEARCH).is_some(), "the record must be INSTALLED");
        assert!(
            !r.is_open(WEB_SEARCH),
            "a closed record must not count as open"
        );
    }

    /// DISK FORMAT record: this is what the registry looked like before the
    /// English rename. Without the legacy mapping an existing user's web search
    /// would silently switch off.
    #[test]
    fn a_registry_written_with_the_old_turkish_keys_still_loads() {
        let r = Record::parse(
            r#"{"eklentiler":[{"name":"web-search","kind":"web-search","state":"open",
                "ayarlar":{"address":"https://example.test/searxng"}}]}"#,
        )
        .unwrap();
        assert!(
            r.is_open(WEB_SEARCH),
            "the old record must map onto the new name"
        );
        assert_eq!(
            r.find(WEB_SEARCH).unwrap().setting(ADDRESS_KEY),
            Some("https://example.test/searxng")
        );
        // The closed state is carried across too.
        let closed =
            Record::parse(r#"{"eklentiler":[{"name":"web-search","state":"closed"}]}"#).unwrap();
        assert!(!closed.is_open(WEB_SEARCH));
    }

    #[test]
    fn an_unknown_state_is_an_error() {
        let e = Record::parse(r#"{"addons":[{"name":"a","state":"opne"}]}"#).unwrap_err();
        assert!(e.contains("unknown state"), "{e}");
    }

    #[test]
    fn adding_the_same_name_twice_replaces_it() {
        let mut r = Record::empty();
        r.add(Addon::new(WEB_SEARCH, WEB_SEARCH).with_setting(ADDRESS_KEY, "https://one.test"));
        r.add(Addon::new(WEB_SEARCH, WEB_SEARCH).with_setting(ADDRESS_KEY, "https://two.test"));
        assert_eq!(r.all().len(), 1);
        assert_eq!(
            r.find(WEB_SEARCH).unwrap().setting(ADDRESS_KEY),
            Some("https://two.test")
        );
    }

    #[test]
    fn delete_and_set_state() {
        let mut r = Record::empty();
        r.add(Addon::new(WEB_SEARCH, WEB_SEARCH));
        assert!(r.set_state(WEB_SEARCH, false).is_some());
        assert!(!r.is_open(WEB_SEARCH));
        assert!(r.set_state("missing", true).is_none());
        assert!(r.delete(WEB_SEARCH));
        assert!(!r.delete(WEB_SEARCH), "the second delete must return false");
        assert!(r.find(WEB_SEARCH).is_none());
    }

    /// The output must be DETERMINISTIC: the same registry written twice gives
    /// the same bytes.
    #[test]
    fn the_json_output_is_deterministic() {
        let mut r = Record::empty();
        r.add(
            Addon::new("b-addon", "web-search")
                .with_setting("z", "1")
                .with_setting("a", "2"),
        );
        r.add(Addon::new("a-addon", "web-search"));
        let one = r.json();
        let two = r.json();
        assert_eq!(one, two);
        // Sorted by name: "a-addon" must come first.
        assert!(
            one.find("a-addon").unwrap() < one.find("b-addon").unwrap(),
            "{one}"
        );
        // The setting keys are sorted too.
        assert!(
            one.find("\"a\"").unwrap() < one.find("\"z\"").unwrap(),
            "{one}"
        );
    }

    // -----------------------------------------------------------------
    // The definition table
    // -----------------------------------------------------------------

    /// THE FIVE NAMES THIS TURN INSTALLS, plus the one that already existed.
    /// The table is the only place the shell learns them from, so a missing row
    /// is an addon that cannot be installed at all.
    #[test]
    fn every_named_addon_has_a_definition() {
        for name in [WEB_SEARCH, SHELL, WORKSPACE, HTTP, DB, CLIPBOARD] {
            let d = definition(name).unwrap_or_else(|| panic!("no definition: {name}"));
            assert!(!d.summary.is_empty(), "{name} has no summary");
            assert!(!d.warning.is_empty(), "{name} says nothing before approval");
            // EVERY addon must be able to say what changes when it opens —
            // including the one that adds no tool. A `tools` list is not enough
            // for that, which is why `effect` exists.
            assert!(!d.effect.is_empty(), "{name} does not say what it changes");
        }
        // `workspace` adds NO tool: it widens the reach of the file tools that
        // are already in the catalog. An empty list here is correct, and this
        // line is what stops someone "fixing" it with an invented tool name.
        assert!(definition(WORKSPACE).unwrap().tools.is_empty());
        assert_eq!(definition(SHELL).unwrap().tools, ["shell"]);
        assert_eq!(definition(HTTP).unwrap().tools, ["http"]);
        assert!(definition("no-such-addon").is_none());
        assert_eq!(installable_names().len(), DEFINITIONS.len());
    }

    /// Names and tool names must be UNIQUE across the table. Two rows claiming
    /// one name would make `definition()` return whichever came first and the
    /// other would be dead but visible.
    #[test]
    fn the_table_has_no_duplicates() {
        let mut names: Vec<&str> = DEFINITIONS.iter().map(|d| d.name).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "a duplicated addon name");

        let mut tools: Vec<&str> = DEFINITIONS.iter().flat_map(|d| d.tools.iter().copied()).collect();
        let count = tools.len();
        tools.sort_unstable();
        tools.dedup();
        assert_eq!(tools.len(), count, "a tool claimed by two addons");

        for d in DEFINITIONS {
            for t in d.tools {
                assert_eq!(provides(t).map(|p| p.name), Some(d.name));
            }
        }
        assert!(provides("calculate").is_none(), "a core tool has an owner");
    }

    /// Each install flow asks WHAT IT NEEDS AND NO MORE — this is the shape of
    /// the turn's requirement, so it is measured rather than described.
    #[test]
    fn each_addon_asks_for_what_it_needs() {
        let asks = |name: &str| -> Vec<&str> {
            definition(name)
                .unwrap()
                .settings
                .iter()
                .map(|s| s.key)
                .collect()
        };
        assert_eq!(asks(WEB_SEARCH), vec![ADDRESS_KEY]);
        assert_eq!(asks(SHELL), vec![COMMANDS_KEY]);
        assert_eq!(asks(WORKSPACE), vec![DIRECTORIES_KEY]);
        assert_eq!(asks(HTTP), vec![HOSTS_KEY]);
        assert!(asks(CLIPBOARD).is_empty(), "clipboard must ask nothing");
        assert!(asks(DB).is_empty(), "db must ask nothing");

        // The three list-shaped ones take MANY values, the two single ones do not.
        for (name, key) in [(SHELL, COMMANDS_KEY), (WORKSPACE, DIRECTORIES_KEY), (HTTP, HOSTS_KEY)] {
            assert!(definition(name).unwrap().setting(key).unwrap().many, "{name}");
        }
        assert!(!definition(WEB_SEARCH).unwrap().setting(ADDRESS_KEY).unwrap().many);
    }

    /// A CREDENTIAL IS NEVER ECHOED. The registry file is 0600 already; what
    /// leaks a password is the terminal output a user pastes into a bug report.
    ///
    /// NO SHIPPED ADDON ASKS FOR A SECRET TODAY — `db` used to and no longer
    /// does, because the tool behind it never had a connection to make. The
    /// masking stays measured anyway: it is what a hand-written registry entry,
    /// or the next addon that does need a key, falls back on.
    #[test]
    fn a_credential_is_never_printed() {
        let secret = "postgres://me:hunter2@db.internal/app";
        let shown = shown_value("home-made", "password", secret);
        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(!shown.contains("db.internal"), "{shown}");
        // The length must not leak either.
        assert!(!shown.contains(&secret.len().to_string()), "{shown}");

        // A hand-written registry with no definition behind it is still not
        // printed in full when the key NAMES a credential.
        for key in ["password", "api_key", "key", "session_token", "MY_SECRET"] {
            assert!(setting_is_secret("home-made", key), "{key} was printed");
        }
        // …but an ordinary field is shown as it is; masking everything would
        // make `addon list` useless.
        assert!(!setting_is_secret("home-made", "keyword"));
        assert_eq!(
            shown_value(WEB_SEARCH, ADDRESS_KEY, "https://a.test"),
            "https://a.test"
        );
    }

    /// THE SCHEMA GATE. What the shape makes impossible is not filtered out of
    /// free text later; these are the strings that must not be storable.
    #[test]
    fn the_shapes_refuse_what_they_are_there_to_refuse() {
        // A command name has no room for a second command.
        for bad in [
            "rm -rf /",
            "git; curl evil.test",
            "git|sh",
            "/usr/bin/git",
            "../../bin/sh",
            "git`whoami`",
            "git $(id)",
            "git\nsh",
            "--version",
            "",
        ] {
            assert!(
                Shape::CommandName.check(bad).is_err(),
                "a command name was accepted: {bad:?}"
            );
        }
        for good in ["git", "ls", "rg", "python3", "docker-compose", "a_b"] {
            assert!(Shape::CommandName.check(good).is_ok(), "{good}");
        }

        // A directory entry must not climb out of itself.
        for bad in ["notes", "~/notes", "/home/me/..", "/a/../../etc", ""] {
            assert!(Shape::Directory.check(bad).is_err(), "{bad:?}");
        }
        assert!(Shape::Directory.check("/Users/me/notes").is_ok());
        // A comma is a legal character in a directory name — the separator is a
        // newline precisely so this can be stored.
        assert!(Shape::Directory.check("/Users/me/notes,drafts").is_ok());

        // A host entry is a host and nothing else.
        for bad in [
            "https://api.example.com",
            "api.example.com/v1",
            "user@api.example.com",
            "api.example.com:8443",
            "*.example.com",
            ".example.com",
            "example..com",
            "",
        ] {
            assert!(Shape::Host.check(bad).is_err(), "{bad:?}");
        }
        assert!(Shape::Host.check("api.example.com").is_ok());

        // The address shape IS the web client's rule, not a second copy of it:
        // plain http to a remote host is refused there, so it is refused here.
        assert!(Shape::Address.check("https://searx.example").is_ok());
        assert!(Shape::Address.check("http://localhost:8888").is_ok());
        assert!(Shape::Address.check("http://searx.example").is_err());
    }

    /// A newline inside ONE value would split it into two allow-list entries on
    /// the next read — one entry the user typed, one they never saw.
    #[test]
    fn a_setting_value_cannot_smuggle_a_second_entry() {
        assert!(Shape::CommandName.check("git\nsh").is_err());
        assert!(Shape::Host.check("a.test\nevil.test").is_err());
        assert!(Shape::Directory.check("/a\n/etc").is_err());
    }

    /// Many values go into one setting and come back out unchanged.
    #[test]
    fn a_many_valued_setting_round_trips() {
        let a = Addon::new(SHELL, SHELL).with_setting(COMMANDS_KEY, join_values(&["git", "ls"]));
        assert_eq!(a.values(COMMANDS_KEY), vec!["git", "ls"]);
        // A setting that was never written is an EMPTY list, not an error: an
        // allow-list nobody filled in allows nothing.
        assert!(a.values(HOSTS_KEY).is_empty());
        assert_eq!(join_values::<&str>(&[]), "");
        assert_eq!(join_values(&["  git  ", "", "ls"]), "git\nls");

        // Through the file, too — the separator has to survive JSON.
        let mut r = Record::empty();
        r.add(a.clone());
        let back = Record::parse(&r.json()).unwrap();
        assert_eq!(back.find(SHELL).unwrap().values(COMMANDS_KEY), vec!["git", "ls"]);
    }

    /// THE ALLOW-LIST MATCHES AT THE DOT.
    ///
    /// The two directions that matter are the two that have already been
    /// shipped wrong in this repository's SSRF gate: a name that merely ENDS
    /// with the entry, and a name that merely BEGINS with it.
    #[test]
    fn the_host_allow_list_matches_at_a_label_boundary() {
        let list = join_values(&["example.com", "api.internal"]);
        assert!(host_is_allowed(&list, "example.com"));
        assert!(host_is_allowed(&list, "api.example.com"));
        assert!(host_is_allowed(&list, "a.b.example.com"));
        assert!(host_is_allowed(&list, "EXAMPLE.COM"), "case must not matter");
        assert!(host_is_allowed(&list, "example.com."), "a trailing root dot");

        assert!(!host_is_allowed(&list, "notexample.com"), "a suffix by letters");
        assert!(!host_is_allowed(&list, "example.com.evil.net"), "a prefix by letters");
        assert!(!host_is_allowed(&list, "evil.net"));
        assert!(!host_is_allowed(&list, ""));
        assert!(!host_is_allowed("", "example.com"), "an empty list allows nothing");
    }

    /// The host of a URL is what comes after the LAST `@`.
    ///
    /// `https://api.example.com@evil.test/` connects to `evil.test`. Read the
    /// other way round, an allow-list holding `example.com` would wave it
    /// through — the address the check looks at has to be the address the
    /// connection uses.
    #[test]
    fn the_host_of_a_url_is_the_one_that_would_be_connected_to() {
        assert_eq!(url_host("https://api.example.com/v1?x=1").as_deref(), Some("api.example.com"));
        assert_eq!(url_host("https://API.Example.COM").as_deref(), Some("api.example.com"));
        assert_eq!(url_host("https://example.com:8443/a").as_deref(), Some("example.com"));
        assert_eq!(url_host("https://[::1]:8443/a").as_deref(), Some("::1"));
        assert_eq!(url_host("https://user:pass@example.com/a").as_deref(), Some("example.com"));
        // THE TRAP.
        assert_eq!(url_host("https://api.example.com@evil.test/a").as_deref(), Some("evil.test"));
        let list = join_values(&["example.com"]);
        assert!(!host_is_allowed(&list, &url_host("https://api.example.com@evil.test/").unwrap()));
        // Not a fetchable scheme, no host at all.
        assert_eq!(url_host("file:///etc/passwd"), None);
        assert_eq!(url_host("example.com"), None);
        assert_eq!(url_host("https://"), None);
    }

    /// THE GATE IS ONE FUNCTION and it falls closed. `is_open` cannot be
    /// measured against the machine's real registry without moving a
    /// process-wide variable, so what is measured here is the shape the free
    /// function delegates to, plus the fact that an unknown name opens nothing.
    #[test]
    fn the_gate_falls_closed() {
        let r = Record::parse(
            r#"{"addons":[{"name":"shell","kind":"shell","state":"open"},
                          {"name":"http","kind":"http","state":"closed"}]}"#,
        )
        .unwrap();
        assert!(r.is_open(SHELL));
        assert!(!r.is_open(HTTP), "a closed record must not count as open");
        assert!(!r.is_open(WORKSPACE), "an absent record must not count as open");
        assert!(!r.is_open("no-such-addon"));
        // An unreadable registry is not an open one — the free function maps
        // `Err` onto false, and that is the whole reason it exists.
        assert!(Record::parse("{ broken").is_err());
    }

    /// The written file must be in the shape the parser can read (the format
    /// contract lives in one place; it is also tested with a hand-written
    /// sample).
    #[test]
    fn a_hand_written_file_is_readable() {
        let r = Record::parse(
            r#"{
  "addons": [
    { "name": "web-search", "kind": "web-search", "state": "open",
      "settings": { "address": "http://localhost:8888" } }
  ]
}"#,
        )
        .unwrap();
        assert!(r.is_open(WEB_SEARCH));
        assert_eq!(
            r.find(WEB_SEARCH).unwrap().setting(ADDRESS_KEY),
            Some("http://localhost:8888")
        );
    }
}
