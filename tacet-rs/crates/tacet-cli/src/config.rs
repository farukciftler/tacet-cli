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
    ("engine", "default engine when --engine is not given: auto | candle | fake"),
    ("theme", "colour theme — see /themes inside the shell (mono, night, sage, graphite, violet)"),
];

fn file_path() -> Option<PathBuf> {
    tacet_core::config_path(FILE)
}

fn load_map() -> Map<String, Value> {
    let Some(p) = file_path() else { return Map::new() };
    let Ok(text) = fs::read_to_string(&p) else { return Map::new() };
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
    load_map().get(key).and_then(|v| v.as_str()).map(str::to_string)
}

fn save(map: &Map<String, Value>) -> Result<(), String> {
    let p = file_path().ok_or_else(|| "the config directory cannot be resolved".to_string())?;
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text =
        serde_json::to_string_pretty(&Value::Object(map.clone())).map_err(|e| e.to_string())?;
    fs::write(&p, text + "\n").map_err(|e| e.to_string())
}

fn validate(key: &str, value: &str) -> Result<(), String> {
    if !KNOWN.iter().any(|(k, _)| *k == key) {
        let names: Vec<&str> = KNOWN.iter().map(|(k, _)| *k).collect();
        return Err(format!("unknown key '{key}' — known keys: {}", names.join(", ")));
    }
    if key == "engine" && !matches!(value, "auto" | "candle" | "fake") {
        return Err(format!("'{value}' is not an engine — expected: auto | candle | fake"));
    }
    if key == "theme" && !crate::ui::THEMES.iter().any(|t| t.name == value) {
        let names: Vec<&str> = crate::ui::THEMES.iter().map(|t| t.name).collect();
        return Err(format!("'{value}' is not a theme — themes: {}", names.join(", ")));
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
        None => println!("{}", color.paint(DIM, "  file: (config directory unresolved)")),
    }
    ExitCode::SUCCESS
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

#[cfg(test)]
mod tests {
    use super::*;

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
