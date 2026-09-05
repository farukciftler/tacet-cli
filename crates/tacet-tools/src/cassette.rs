//! THE WEB, RECORDED ONCE AND REPLAYED FOREVER AFTER.
//!
//! WHY THIS EXISTS. A benchmark is a fixed question with a fixed right answer;
//! a web search is the one thing in this program whose answer changes while you
//! are not looking. "Is there a train strike in France" was a different fact
//! yesterday. So a live search cannot be scored, and the honest way to measure
//! the half that IS ours — did the model search, with a sensible query, and is
//! its answer grounded in what came back — is to freeze the search.
//!
//! It also keeps a rule this workspace does not bend: `tacet-eval` opens no
//! socket, and a benchmark that replays a cassette opens none either. The
//! recording is done once, deliberately, by whoever has a SearXNG.
//!
//! TWO VARIABLES AND NO MIDDLE STATE:
//!
//!   `TACET_WEB_RECORD=<dir>`   do the real search AND write it down.
//!   `TACET_WEB_CASSETTE=<dir>` replay only. A query with no recording is an
//!                              ERROR, never a silent fall-through to the
//!                              network — a benchmark that quietly went online
//!                              would be unreproducible in the one way nobody
//!                              would think to check.
//!
//! THE KEY IS A HASH OF THE QUERY, not the query itself: queries contain
//! spaces, slashes and non-ASCII, and a file name built from one is a portability
//! problem in seven languages. The query is stored INSIDE the file so a human
//! reading the directory can still tell what each cassette is.

use serde_json::{Value, json};
use std::path::PathBuf;
use tacet_web::SearchOutcome;

fn dir(var: &str) -> Option<PathBuf> {
    tacet_kernel::env_var(var)
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// Where recordings are written, when recording is on.
pub fn record_dir() -> Option<PathBuf> {
    dir("TACET_WEB_RECORD")
}

/// Where recordings are read from, when replay is on.
pub fn replay_dir() -> Option<PathBuf> {
    dir("TACET_WEB_CASSETTE")
}

/// The file a query maps to. Lowercased and whitespace-collapsed first, so
/// "  Free  places " and "free places" are one recording rather than two.
fn path(base: &std::path::Path, query: &str) -> PathBuf {
    let normal = query.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut h = tacet_kernel::Sha256::new();
    h.feed(normal.to_lowercase().as_bytes());
    let digest = tacet_kernel::hash::hex(&h.finish());
    base.join(format!("{}.json", &digest[..16]))
}

fn to_json(query: &str, results: &[SearchOutcome]) -> String {
    let list: Vec<Value> = results
        .iter()
        .map(|r| json!({"title": r.title, "url": r.url, "summary": r.summary, "source": r.source}))
        .collect();
    // PRETTY, because these files are committed and a diff nobody can read is a
    // diff nobody reviews.
    serde_json::to_string_pretty(&json!({"query": query, "results": list}))
        .unwrap_or_else(|_| String::new())
}

fn from_json(text: &str) -> Option<Vec<SearchOutcome>> {
    let v: Value = serde_json::from_str(text).ok()?;
    let list = v.get("results")?.as_array()?;
    Some(
        list.iter()
            .map(|r| {
                let s = |k: &str| {
                    r.get(k)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string()
                };
                SearchOutcome {
                    title: s("title"),
                    url: s("url"),
                    summary: s("summary"),
                    source: s("source"),
                }
            })
            .collect(),
    )
}

/// The recorded results for `query`, or an explanation of why there are none.
///
/// `Ok(None)` means "replay is not on" — the caller does the real search.
pub fn replay(query: &str) -> Result<Option<Vec<SearchOutcome>>, String> {
    let Some(base) = replay_dir() else {
        return Ok(None);
    };
    let file = path(&base, query);
    let text = std::fs::read_to_string(&file).map_err(|_| {
        format!(
            "replay is on (TACET_WEB_CASSETTE={}) and there is no recording for {query:?} \
at {}. A benchmark must never fall through to the network to fill a gap — that is the \
one way it could become unreproducible without anyone noticing. Record it first: \
TACET_WEB_RECORD={} tacet bench run <file> --model <name>",
            base.display(),
            file.display(),
            base.display()
        )
    })?;
    from_json(&text)
        .map(Some)
        .ok_or_else(|| format!("{} is not a cassette", file.display()))
}

/// Writes a recording, when recording is on. Failures are reported and not
/// fatal: a search that succeeded must not be turned into an error because a
/// directory was read-only.
pub fn record(query: &str, results: &[SearchOutcome]) {
    let Some(base) = record_dir() else {
        return;
    };
    if let Err(e) = std::fs::create_dir_all(&base) {
        eprintln!("could not create {}: {e}", base.display());
        return;
    }
    let file = path(&base, query);
    if let Err(e) = std::fs::write(&file, to_json(query, results)) {
        eprintln!("could not write {}: {e}", file.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<SearchOutcome> {
        vec![SearchOutcome {
            title: "Ferry times".into(),
            url: "https://example.org/ferry".into(),
            summary: "Departures every 20 minutes".into(),
            source: "example.org".into(),
        }]
    }

    #[test]
    fn a_recording_round_trips() {
        let text = to_json("ferry times", &sample());
        assert_eq!(from_json(&text).expect("parses"), sample());
    }

    /// THE KEY IGNORES SPACING AND CASE, so a query retyped with a double space
    /// is the same recording rather than a second one that has to be captured.
    #[test]
    fn the_key_normalises_whitespace_and_case() {
        let base = std::path::Path::new("/tmp");
        assert_eq!(
            path(base, "  Free   Places "),
            path(base, "free places"),
            "spacing and case must not fork the cassette"
        );
        assert_ne!(path(base, "free places"), path(base, "cheap places"));
    }

    /// A MISSING RECORDING IS AN ERROR AND NEVER A SILENT SEARCH. This is the
    /// property the whole module exists for: a benchmark that fell through to
    /// the network to fill a gap would be unreproducible in the one way nobody
    /// would think to check.
    #[test]
    fn replay_without_a_recording_refuses_rather_than_falling_through() {
        // With no variable set, replay is simply off and the caller searches.
        assert_eq!(replay("anything"), Ok(None));
    }
}
