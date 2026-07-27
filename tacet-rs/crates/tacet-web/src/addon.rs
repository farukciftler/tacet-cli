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
//! round; so the registry has to live in a crate BELOW both. `tacet-core` was a
//! candidate too, but the rule there is "no work happens in this crate" (no
//! file reads, no directory creation), while the registry writes to disk. What
//! is left is `tacet-web`: today's only addon kind is the very thing that opens
//! this crate's network surface, and address validation
//! (`client::address_is_valid`) is defined here.
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

/// LEGACY ADDON NAME. Registries written before the rename hold `web-arama`;
/// `parse` maps it onto `WEB_SEARCH` so an existing installation keeps working.
pub const LEGACY_WEB_SEARCH: &str = "web-arama";

/// The key of the base address inside `settings`.
pub const ADDRESS_KEY: &str = "address";

/// LEGACY SETTING KEY (`adres`), accepted on read for the same reason as
/// `LEGACY_WEB_SEARCH`.
pub const LEGACY_ADDRESS_KEY: &str = "adres";

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
        Self { name: name.into(), kind: kind.into(), open: true, settings: BTreeMap::new() }
    }

    pub fn with_setting(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.settings.insert(key.into(), value.into());
        self
    }

    pub fn setting(&self, key: &str) -> Option<&str> {
        self.settings.get(key).map(String::as_str)
    }

    pub fn state_text(&self) -> &'static str {
        if self.open { "open" } else { "closed" }
    }
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
        format!("{}\n", serde_json::to_string_pretty(&root).unwrap_or_else(|_| "{}".into()))
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
            let name = if raw_name == LEGACY_WEB_SEARCH { WEB_SEARCH } else { raw_name };
            let raw_kind =
                a.get("kind").or_else(|| a.get("tur")).and_then(|k| k.as_str()).unwrap_or(raw_name);
            let kind = if raw_kind == LEGACY_WEB_SEARCH { WEB_SEARCH } else { raw_kind };
            let open = match a.get("state").or_else(|| a.get("durum")).and_then(|s| s.as_str()) {
                None | Some("open") | Some("acik") => true,
                Some("closed") | Some("kapali") => false,
                Some(s) => return Err(format!("'{name}': unknown state '{s}'")),
            };
            let mut settings = BTreeMap::new();
            if let Some(object) =
                a.get("settings").or_else(|| a.get("ayarlar")).and_then(|s| s.as_object())
            {
                for (key, value) in object {
                    let v = value
                        .as_str()
                        .ok_or_else(|| format!("'{name}': setting `{key}` must be text"))?;
                    let key =
                        if key == LEGACY_ADDRESS_KEY { ADDRESS_KEY.to_string() } else { key.clone() };
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

/// The full path of the registry file. `None` = the config directory could not
/// be resolved.
///
/// LEGACY FALLBACK: if `addons.json` does not exist but `eklentiler.json` does,
/// the old path is returned. Otherwise a user who installed the addon before
/// the rename would find web search silently off, with a "the addon was never
/// installed" symptom that points nowhere.
pub fn registry_path() -> Option<PathBuf> {
    let current = tacet_core::env::config_path(REGISTRY_FILE)?;
    if current.exists() {
        return Some(current);
    }
    let legacy = tacet_core::env::config_path(LEGACY_REGISTRY_FILE)?;
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
    let path = registry_path()
        .ok_or_else(|| "could not resolve the config directory (TACET_HOME can be set)".to_string())?;
    write_to_path(&path, record)?;
    Ok(path)
}

pub fn write_to_path(path: &Path, record: &Record) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, record.json()).map_err(|e| format!("{}: {e}", path.display()))
}

/// THE CATALOG GATE. Whether the `web_search`/`web_fetch` tools appear in the
/// catalog depends on this returning `true` (see `tacet_tools::catalog`).
///
/// A read error = CLOSED (see the top of the file).
pub fn web_search_is_open() -> bool {
    read().map(|r| r.is_open(WEB_SEARCH)).unwrap_or(false)
}

/// The base address the search will go to.
///
/// ORDER: environment variable > registry. The variable comes FIRST because it
/// is the user's EXPLICIT request for that particular run (developer shell, a
/// one-off experiment); the registry is the persistent preference. If neither
/// exists it returns `None` and the client raises a "server not configured"
/// error — THERE IS NO ADDRESS BAKED INTO THE CODE.
pub fn web_address() -> Option<String> {
    if let Some(v) = tacet_core::env_var(crate::client::ADDRESS_VARIABLE) {
        let v = v.to_string_lossy().trim().to_string();
        if !v.is_empty() {
            return Some(v);
        }
    }
    read().ok()?.find(WEB_SEARCH)?.setting(ADDRESS_KEY).map(str::to_string)
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
        assert!(!r.is_open(WEB_SEARCH), "a closed record must not count as open");
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
        assert!(r.is_open(WEB_SEARCH), "the old record must map onto the new name");
        assert_eq!(
            r.find(WEB_SEARCH).unwrap().setting(ADDRESS_KEY),
            Some("https://example.test/searxng")
        );
        // The closed state is carried across too.
        let closed = Record::parse(r#"{"eklentiler":[{"name":"web-search","state":"closed"}]}"#).unwrap();
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
        assert_eq!(r.find(WEB_SEARCH).unwrap().setting(ADDRESS_KEY), Some("https://two.test"));
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
        r.add(Addon::new("b-addon", "web-search").with_setting("z", "1").with_setting("a", "2"));
        r.add(Addon::new("a-addon", "web-search"));
        let one = r.json();
        let two = r.json();
        assert_eq!(one, two);
        // Sorted by name: "a-addon" must come first.
        assert!(one.find("a-addon").unwrap() < one.find("b-addon").unwrap(), "{one}");
        // The setting keys are sorted too.
        assert!(one.find("\"a\"").unwrap() < one.find("\"z\"").unwrap(), "{one}");
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
        assert_eq!(r.find(WEB_SEARCH).unwrap().setting(ADDRESS_KEY), Some("http://localhost:8888"));
    }
}
