//! A CLOSED SET THE SCHEMA DECLARES IS A CLOSED SET THE GRAMMAR ENFORCES.
//!
//! This is the project's own thesis turned on its own catalog. `ArgSchema::choice`
//! is not documentation: `compile.rs` embeds the values into the automaton as a
//! literal alternation, so a value outside the set is not rejected after the
//! call is parsed — it cannot be written in the first place.
//!
//! WHY THIS FILE EXISTS. `calendar`'s `kind` was declared `ArgSchema::text()`
//! while its body accepted exactly `events` and `remind` and refused everything
//! else with a message. Nothing was wrong at runtime — the call was refused,
//! correctly, one turn too late. The model could spend a whole generation on
//! `{"kind":"banana"}` and be told no afterwards, which is the shape this
//! codebase exists to make unreachable. A test that walks the catalog is the
//! only thing that stops the next tool doing it: no reviewer re-reads seventeen
//! schemas.
//!
//! This test lives in `tacet-cli` because it is the only crate that can see both
//! halves — `tacet-tools` builds the catalog, `tacet-grammar` compiles it, and
//! neither depends on the other.

use tacet_grammar::Grammar;
use tacet_kernel::{Field, SchemaKind};

/// A JSON literal that is legal for a field, used to get PAST the fields that
/// come before the one under test. It has to be shaped by KIND, not guessed: a
/// quoted `"1"` was refused by `write_code`'s `lines`, which is an array — and
/// that refusal read like a failure of the field under test.
fn a_legal_literal(field: &Field) -> String {
    match &field.schema.kind {
        SchemaKind::Choice { choices } => format!("\"{}\"", choices[0]),
        SchemaKind::Text { .. } => "\"x\"".to_string(),
        SchemaKind::Number {
            is_integer, min, ..
        } => {
            let base = min.unwrap_or(1.0).max(1.0);
            if *is_integer {
                format!("{}", base as i64)
            } else {
                format!("{base}")
            }
        }
        SchemaKind::Bool => "true".to_string(),
        SchemaKind::Array { item, .. } => {
            format!("[{}]", a_legal_literal(&Field::new("i", (**item).clone())))
        }
        SchemaKind::Object { fields } => {
            let inner: Vec<String> = fields
                .iter()
                .filter(|f| f.required)
                .map(|f| format!("\"{}\":{}", f.name, a_legal_literal(f)))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
    }
}

/// Walks the whole production catalog. For every field declared as a choice:
/// every value IN the set must be writable, and a value outside it must be
/// refused by the automaton — not by the tool body afterwards.
#[test]
fn every_declared_choice_is_enforced_by_the_grammar() {
    let store = std::sync::Arc::new(tacet_tools::data_store::SharedStore::new());
    let memory = tacet_tools::memory::SharedMemory::in_memory();
    let (catalog, _, _) = tacet_tools::catalog::production_catalog(&store, &memory, Some(0));

    let mut checked = 0usize;
    for tool in catalog.tools() {
        let schema = tool.schema();
        let fields = schema.fields();
        for (index, field) in fields.iter().enumerate() {
            let Some(choices) = field.schema.choices() else {
                continue;
            };
            checked += 1;

            // The prefix: every required field BEFORE this one, then this
            // field's key and its opening quote.
            let mut prefix = String::from("{");
            for earlier in fields.iter().take(index).filter(|f| f.required) {
                prefix.push_str(&format!(
                    "\"{}\":{},",
                    earlier.name,
                    a_legal_literal(earlier)
                ));
            }
            prefix.push_str(&format!("\"{}\":\"", field.name));

            // (a) EVERY LEGAL VALUE IS WRITABLE. A choice list the grammar
            // cannot actually produce would be worse than free text.
            for value in choices {
                let mut state = Grammar::compile(&schema).state();
                state.advance(&prefix).unwrap_or_else(|e| {
                    panic!("{}: could not reach `{}`: {e:?}", tool.name(), field.name)
                });
                state.advance(value).unwrap_or_else(|e| {
                    panic!(
                        "{}: `{}` declares `{value}` but the grammar refuses it: {e:?}",
                        tool.name(),
                        field.name
                    )
                });
            }

            // (b) AND A VALUE OUTSIDE THE SET IS UNREPRESENTABLE. `zzz` shares
            // no first character with any value in any set in this catalog, so
            // the refusal must land on the first character; the loop does not
            // assume that.
            assert!(
                !choices.iter().any(|c| c.starts_with('z')),
                "{}: `{}` has a value starting with `z`, so `zzz` is no longer \
                 a foil — pick another",
                tool.name(),
                field.name
            );
            let mut state = Grammar::compile(&schema).state();
            state.advance(&prefix).expect("reaching the field");
            let refused = "zzz\""
                .chars()
                .any(|c| state.advance(&c.to_string()).is_err());
            assert!(
                refused,
                "{}: `{}` declares choice{:?} but the grammar accepted `zzz` — \
                 the set is documentation, not a constraint",
                tool.name(),
                field.name,
                choices
            );
        }
    }

    // A silent zero would make this file green forever if `choices()` ever
    // stopped reporting.
    assert!(
        checked >= 8,
        "only {checked} closed sets found in the production catalog — the walk \
         is no longer walking"
    );
}

/// THE ONE THAT PAID FOR THE FILE, pinned by name so a revert is loud.
///
/// `calendar` IS macOS-ONLY — it speaks to Calendar/Reminders through
/// `osascript`, and `production_catalog` registers it behind
/// `#[cfg(target_os = "macos")]`. This test called `.expect("calendar is a
/// built-in")` and went red on Linux and Windows for a reason that is not a
/// defect (CI #110). The repository already had that trap written down, in the
/// `HOST_CONDITIONAL` note in `main.rs`, and it caught me anyway.
///
/// THE FIX IS NOT `#[cfg(target_os = "macos")]` ON THE TEST, and not an early
/// return either: both turn "not here" into "not tested" with nothing saying so.
/// The platform rule is asserted FIRST, in both directions — present on macOS,
/// absent everywhere else — so a registration that goes missing on a Mac fails
/// here, and one that appears where it cannot work fails here too. Only then
/// does the closed-set claim run, on the platform that has the tool.
#[test]
fn calendar_kind_is_a_closed_set() {
    let store = std::sync::Arc::new(tacet_tools::data_store::SharedStore::new());
    let memory = tacet_tools::memory::SharedMemory::in_memory();
    let (catalog, _, _) = tacet_tools::catalog::production_catalog(&store, &memory, Some(0));

    let found = catalog.find("calendar");
    let expected = cfg!(target_os = "macos");
    assert_eq!(
        found.is_some(),
        expected,
        "`calendar` is registered behind `#[cfg(target_os = \"macos\")]`, so it \
         should be {} on this platform and it is {}",
        if expected { "present" } else { "absent" },
        if found.is_some() { "present" } else { "absent" }
    );
    let Some(calendar) = found else {
        // Not a skip: the assertion above is this platform's whole claim about
        // `calendar`, and it just ran.
        return;
    };
    let schema = calendar.schema();
    let kind = schema
        .fields()
        .iter()
        .find(|f| f.name == "kind")
        .expect("calendar takes a kind");
    assert_eq!(
        kind.schema.choices().map(<[String]>::to_vec),
        Some(vec!["events".to_string(), "remind".to_string()]),
        "the body accepts exactly these two and refuses the rest; the schema \
         must say so, or the grammar cannot"
    );

    let mut state = Grammar::compile(&schema).state();
    state.advance(r#"{"kind":""#).expect("opening the field");
    assert!(
        state.advance("b").is_err(),
        "`{{\"kind\":\"banana\"}}` must be unwritable, not merely refused later"
    );
}
