//! `tacet config` — persistent personal defaults (`config.json` in the config
//! directory, next to skills and memory).
//!
//! Deliberately tiny: ONLY keys that already exist as command-line flags may
//! live here, nothing else — the file is a way to stop retyping a flag, not a
//! second settings system. Precedence for any setting is
//! flag > environment variable > this file > built-in default: the file is the
//! QUIETEST voice, an explicit ask always wins over it.

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{Map, Value};

use crate::ui::{Color, DIM};

const FILE: &str = "config.json";

/// The known keys. `set` refuses anything outside this list: a settings file
/// that accepts arbitrary keys becomes a graveyard of typos ("modle") that
/// silently do nothing.
const KNOWN: &[(&str, &str)] = &[
    ("model", "default model folder for chat (e.g. qwen3-4b)"),
    (
        "engine",
        "default engine when --engine is not given: auto | candle | fake",
    ),
    (
        "theme",
        "colour theme — see /themes inside the shell (mono, night, sage, graphite, violet)",
    ),
    // The only key that is not a shortcut for a flag, and the reason is that it
    // decides whether this program contacts a server on its own. That is a
    // question the user has to answer, so it is stored rather than assumed.
    // `ask` (the default) means it has not been answered yet.
    (
        "thinking",
        "qwen thinking mode: auto (per-turn heuristic, default) | on | off",
    ),
    (
        "update.check",
        "look for a newer release once a day: on | off | ask (default: ask, once, in the shell)",
    ),
];

/// The known keys with their help lines — the in-shell `/config` menu reads
/// the same table `set` validates against, so the menu can never offer a key
/// the validator would refuse.
pub fn known_keys() -> &'static [(&'static str, &'static str)] {
    KNOWN
}

/// How `update.check` is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdatePolicy {
    /// The user said yes: one check a day, at most.
    On,
    /// The user said no. Nothing is asked again.
    Off,
    /// Not decided yet. The shell offers once, after a few turns.
    Ask,
}

/// Reads the policy. An unreadable or nonsense value is treated as `Ask`, never
/// as `On`: a broken config file must not be the thing that turns network
/// access on.
pub fn update_policy() -> UpdatePolicy {
    match get_str("update.check").as_deref() {
        Some("on") => UpdatePolicy::On,
        Some("off") => UpdatePolicy::Off,
        _ => UpdatePolicy::Ask,
    }
}

/// Writes the answer down so the question is asked exactly once.
pub fn set_update_policy(on: bool) -> Result<(), String> {
    set_value("update.check", if on { "on" } else { "off" })
}

fn file_path() -> Option<PathBuf> {
    tacet_kernel::config_path(FILE)
}

fn load_map() -> Map<String, Value> {
    let Some(p) = file_path() else {
        return Map::new();
    };
    let Ok(text) = fs::read_to_string(&p) else {
        return Map::new();
    };
    // A corrupt file reads as empty instead of crashing every command: the
    // worst outcome of a broken config must be "defaults apply", not "the
    // shell is dead until someone edits JSON by hand".
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

/// A string setting from the file — `None` when unset or unreadable.
pub fn get_str(key: &str) -> Option<String> {
    load_map()
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn save(map: &Map<String, Value>) -> Result<(), String> {
    let p = file_path().ok_or_else(|| "the config directory cannot be resolved".to_string())?;
    save_to(&p, map)
}

/// The write itself, with the path handed in.
///
/// SPLIT OUT SO THE PERMISSIONS CAN BE MEASURED. `file_path()` reads a
/// PROCESS-WIDE environment variable, and a test that sets it fights every
/// other test in this binary; the mode of the directory and the file is exactly
/// the kind of thing that must be measured on the REAL write path rather than
/// on a hand-rolled copy of it.
fn save_to(p: &std::path::Path, map: &Map<String, Value>) -> Result<(), String> {
    if let Some(parent) = p.parent() {
        // 0700, NOT THE UMASK. This is very often the FIRST command that
        // creates the config directory (`tacet config set theme night`), and
        // whatever mode it is born with is the mode `memory.json` (personal
        // notes), `mcp.json` (a plain-text bearer token) and `addons.json` (an
        // address that may carry a credential) live under afterwards. Measured:
        // with umask 022 it was born 0755 and every second local account could
        // walk in.
        tacet_kernel::fs::create_private_dir(parent).map_err(|e| e.to_string())?;
    }
    let text =
        serde_json::to_string_pretty(&Value::Object(map.clone())).map_err(|e| e.to_string())?;
    // config.json itself is not a secret today, but it sits in the same
    // directory and is written by the same gate; a file that starts life 0644
    // is one added setting away from leaking one.
    tacet_kernel::fs::write_private(p, (text + "\n").as_bytes()).map_err(|e| e.to_string())
}

fn validate(key: &str, value: &str) -> Result<(), String> {
    if !KNOWN.iter().any(|(k, _)| *k == key) {
        let names: Vec<&str> = KNOWN.iter().map(|(k, _)| *k).collect();
        return Err(format!(
            "unknown key '{key}' — known keys: {}",
            names.join(", ")
        ));
    }
    if key == "engine" && !matches!(value, "auto" | "candle" | "fake") {
        return Err(format!(
            "'{value}' is not an engine — expected: auto | candle | fake"
        ));
    }
    if key == "thinking" && !matches!(value, "auto" | "on" | "off") {
        return Err(format!(
            "'{value}' is not a thinking mode — expected: auto | on | off"
        ));
    }
    if key == "update.check" && !matches!(value, "on" | "off" | "ask") {
        return Err(format!(
            "'{value}' is not an update policy — expected: on | off | ask"
        ));
    }
    if key == "theme" && !crate::ui::THEMES.iter().any(|t| t.name == value) {
        let names: Vec<&str> = crate::ui::THEMES.iter().map(|t| t.name).collect();
        return Err(format!(
            "'{value}' is not a theme — themes: {}",
            names.join(", ")
        ));
    }
    Ok(())
}

pub fn list(json: bool) -> ExitCode {
    let map = load_map();
    if json {
        println!("{}", Value::Object(map));
        return ExitCode::SUCCESS;
    }
    let color = Color::setup();
    for (key, help) in KNOWN {
        match map.get(*key).and_then(|v| v.as_str()) {
            Some(v) => println!("  {key} = {v}"),
            None => println!("  {key} {}", color.paint(DIM, "(unset)")),
        }
        println!("{}", color.paint(DIM, &format!("      {help}")));
    }
    match file_path() {
        Some(p) => println!("{}", color.paint(DIM, &format!("  file: {}", p.display()))),
        None => println!(
            "{}",
            color.paint(DIM, "  file: (config directory unresolved)")
        ),
    }
    warn_if_home_var_ignored(&color);
    ExitCode::SUCCESS
}

/// Says out loud that `TACET_HOME` was dropped.
///
/// THE DROP ITSELF IS RIGHT (a relative value would put `memory.json` and the
/// plain-text MCP token in whatever folder `tacet` was started from, i.e. very
/// often a git repository). DOING IT QUIETLY IS NOT: this is the one variable
/// the user typed by hand and it outranks everything else, so a silent drop
/// leaves them believing their token lives somewhere it does not. Printed next
/// to the resolved path, where the reader is already looking.
pub fn warn_if_home_var_ignored(color: &Color) {
    if !tacet_kernel::env::home_var_ignored() {
        return;
    }
    eprintln!(
        "{}",
        color.paint(
            crate::ui::YELLOW,
            "TACET_HOME is not an absolute path — it was IGNORED."
        )
    );
    eprintln!(
        "{}",
        color.paint(
            DIM,
            "  a relative value would put memory.json and the MCP token in the folder you started tacet from.",
        )
    );
    eprintln!(
        "{}",
        color.paint(DIM, "  use an absolute path, e.g. TACET_HOME=$HOME/.tacet")
    );
}

pub fn get(key: &str) -> ExitCode {
    match get_str(key) {
        Some(v) => {
            println!("{v}");
            ExitCode::SUCCESS
        }
        None => ExitCode::FAILURE,
    }
}

/// The write path shared by the CLI subcommand and the in-shell `/config`:
/// validate, merge, save. The callers own the messaging.
pub fn set_value(key: &str, value: &str) -> Result<(), String> {
    validate(key, value)?;
    let mut map = load_map();
    map.insert(key.to_string(), Value::String(value.to_string()));
    save(&map).map_err(|e| format!("the config could not be written: {e}"))
}

pub fn set(key: &str, value: &str) -> ExitCode {
    match set_value(key, value) {
        Ok(()) => {
            println!("{key} = {value}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

pub fn unset(key: &str) -> ExitCode {
    let mut map = load_map();
    if map.remove(key).is_none() {
        println!("{key} was not set");
        return ExitCode::SUCCESS;
    }
    match save(&map) {
        Ok(()) => {
            println!("{key} removed");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: the config could not be written: {e}");
            ExitCode::FAILURE
        }
    }
}

pub fn path() -> ExitCode {
    // On stderr, so the path on stdout stays pipeable.
    warn_if_home_var_ignored(&Color::setup());
    match file_path() {
        Some(p) => {
            println!("{}", p.display());
            ExitCode::SUCCESS
        }
        None => {
            eprintln!("error: the config directory cannot be resolved");
            ExitCode::FAILURE
        }
    }
}

/// The values a key may take, in the order Enter walks through them.
///
/// `None` means the key is free text and cannot be cycled — `model` is the only
/// one, and even it becomes a list when packages are on disk, because in
/// practice the answer is always one of the folders that exist.
pub fn choices(key: &str) -> Option<Vec<String>> {
    match key {
        "engine" => Some(vec!["auto".into(), "candle".into(), "fake".into()]),
        "theme" => Some(
            crate::ui::THEMES
                .iter()
                .map(|t| t.name.to_string())
                .collect(),
        ),
        "update.check" => Some(vec!["ask".into(), "on".into(), "off".into()]),
        "model" => {
            let found = crate::model_package::catalog();
            if found.is_empty() {
                None
            } else {
                Some(found.into_iter().map(|p| p.name).collect())
            }
        }
        _ => None,
    }
}

/// The next value after the current one, wrapping. An unset key starts at the
/// first choice rather than the second: pressing Enter once should land on
/// something, not skip past it.
pub fn next_value(key: &str) -> Option<String> {
    let list = choices(key)?;
    let current = get_str(key);
    let index = current
        .as_deref()
        .and_then(|c| list.iter().position(|v| v == c))
        .map(|i| (i + 1) % list.len())
        .unwrap_or(0);
    list.get(index).cloned()
}

/// Every known key with its description — for the menu.
pub fn keys() -> Vec<(&'static str, &'static str)> {
    KNOWN.to_vec()
}

/// The value as the menu should show it.
pub fn shown_value(key: &str) -> String {
    match get_str(key) {
        Some(v) => v,
        None => match key {
            // A default that is real should be named, or the reader thinks the
            // setting does nothing until they touch it.
            "update.check" => "ask (default)".to_string(),
            _ => "unset".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// THE CONFIG DIRECTORY IS WHERE THE SECRETS LIVE, AND ITS MODE IS NOT THE
    /// UMASK'S TO CHOOSE.
    ///
    /// `tacet config set theme night` is very often the FIRST command that
    /// creates `~/.tacet`, and whatever mode it lands with is the mode
    /// `memory.json` (personal notes), `mcp.json` (a plain-text bearer token)
    /// and `addons.json` (an address that may carry a credential) live under
    /// afterwards. Measured against the old `create_dir_all` + `fs::write`
    /// pair under umask 022: 0755 and 0644 — any second local account on the
    /// machine could read all of it. This test is RED on that code.
    ///
    /// A DIRECTORY LEFT OVER FROM AN OLDER INSTALL IS SET UP ON PURPOSE: fixing
    /// only fresh installs would protect nobody who already runs the tool.
    #[cfg(unix)]
    #[test]
    fn the_config_directory_and_its_file_exclude_other_local_accounts() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join("tacet-config-mode-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();

        let file = dir.join("config.json");
        let mut map = Map::new();
        map.insert("theme".into(), Value::String("night".into()));
        save_to(&file, &map).unwrap();

        let mode = |p: &std::path::Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700, "the config directory is walkable");
        assert_eq!(mode(&file), 0o600, "the config file is world-readable");
        assert!(fs::read_to_string(&file).unwrap().contains("night"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_key_is_rejected() {
        assert!(validate("modle", "x").is_err());
    }

    #[test]
    fn engine_value_is_validated() {
        assert!(validate("engine", "candle").is_ok());
        assert!(validate("engine", "gpu").is_err());
    }

    #[test]
    fn theme_value_is_validated_against_the_catalogue() {
        assert!(validate("theme", "night").is_ok());
        assert!(validate("theme", "sage").is_ok());
        assert!(validate("theme", "neon").is_err());
    }
}
