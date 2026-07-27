//! `SearchOutcome` and the parsing of SearXNG's JSON.
//!
//! NO NETWORK in this file: its input is a `&str`. Deliberate, because every
//! dirty corner of parsing (missing field, null, empty `results`, a response
//! that came back as HTML) can then be tested without going to the network.
//! The network layer only does the "fetch bytes" job (`client.rs`); the
//! interpretation happens here.

use crate::error::{WebError, WebResult};
use serde_json::Value;

/// A single search result.
///
/// `source` is the domain (`www.mgm.gov.tr`), not the full URL. WHY A SEPARATE
/// FIELD: the full URL has no business in the text that goes to the model — it
/// costs tokens, and the model tries to REPRODUCE a long address it saw in its
/// own answer and hallucinates links that do not exist. The domain shows the
/// source honestly without giving material to invent from. The full address
/// lives in `url` and shows up in the chip detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOutcome {
    pub title: String,
    pub url: String,
    pub summary: String,
    pub source: String,
}

/// Converts a SearXNG response body into `Vec<SearchOutcome>`.
///
/// TOLERANT FIELD READING, STRICT BODY CHECKING: if a single result is missing
/// its `content`, that result goes through with an empty summary (field
/// coverage varies between SearXNG engines, and one engine's gap must not drop
/// the whole search). But if the body is not JSON at all, or `results` is not
/// an array, it returns an ERROR — "passing softly" there would hide the fact
/// that the server is misconfigured.
pub fn parse(body: &str) -> WebResult<Vec<SearchOutcome>> {
    let root: Value = serde_json::from_str(body)
        .map_err(|e| WebError::InvalidJson(format!("body is not JSON: {e}")))?;

    let array = root
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| WebError::InvalidJson("no 'results' array in the response".into()))?;

    let mut outcomes = Vec::with_capacity(array.len());

    // If there is an infobox it goes FIRST: for queries like "how many lira is
    // a dollar" the direct answer is in there, while the top organic result is
    // usually a news page that contains that answer.
    if let Some(box_) = root
        .get("infoboxes")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
    {
        let content = text_field(box_, "content");
        if !content.is_empty() {
            let url = box_
                .get("urls")
                .and_then(Value::as_array)
                .and_then(|u| u.first())
                .map(|u| text_field(u, "url"))
                .unwrap_or_default();
            outcomes.push(SearchOutcome {
                title: text_field(box_, "infobox"),
                source: domain(&url),
                url,
                summary: content,
            });
        }
    }

    for item in array {
        let url = text_field(item, "url");
        // A result without a URL is unusable: neither citable nor fetchable.
        if url.is_empty() {
            continue;
        }
        outcomes.push(SearchOutcome {
            title: text_field(item, "title"),
            source: domain(&url),
            url,
            summary: text_field(item, "content"),
        });
    }

    if outcomes.is_empty() {
        return Err(WebError::EmptyResult);
    }
    Ok(outcomes)
}

/// Turns a JSON field into text; missing or `null` gives an empty string.
fn text_field(value: &Value, name: &str) -> String {
    value
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Extracts the domain from a URL. NOT a full URL parser, and it must not be:
/// all we need is the authority part between the scheme and the first `/`; the
/// user info and the port are dropped too, so an address like
/// `user@host:8443` does not add noise to the source line.
pub fn domain(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or(authority);
    host.split(':').next().unwrap_or(host).to_string()
}

/// Truncates the text at a word boundary.
///
/// THE WORD BOUNDARY MATTERS: a raw character cut ("...Türkiye'de enflasyon o")
/// is both unreadable and hands the model half a fact; trying to complete a
/// half sentence is exactly when the model MAKES THINGS UP. If no whitespace is
/// found near the limit (one long word, a URL) it cuts hard — walking back
/// forever would destroy the whole summary.
pub fn truncate_at_word(text: &str, at_most: usize) -> String {
    if text.chars().count() <= at_most {
        return text.to_string();
    }
    let truncated: String = text.chars().take(at_most).collect();
    let cut = match truncated.rfind(char::is_whitespace) {
        // Falling back to a very early space would throw away more than half
        // of the summary.
        Some(i) if i >= truncated.len() / 2 => i,
        _ => truncated.len(),
    };
    format!("{}…", truncated[..cut].trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shortened copy of a real server response — the shape is verbatim.
    const SAMPLE: &str = r#"{
        "query": "rust async",
        "number_of_results": 0,
        "results": [
            {"title": "Async Rust", "url": "https://doc.rust-lang.org/book/ch17-00.html",
             "content": "Learn how to use Rust's async and await syntax.",
             "engine": "duckduckgo", "score": 4.0},
            {"title": "Async book", "url": "https://rust-lang.github.io/async-book/",
             "content": "Learn how to write concurrent code.", "engine": "google", "score": 2.0}
        ]
    }"#;

    #[test]
    fn the_sample_response_parses_and_the_domain_is_extracted() {
        let s = parse(SAMPLE).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].title, "Async Rust");
        assert_eq!(s[0].source, "doc.rust-lang.org");
        assert_eq!(s[1].source, "rust-lang.github.io");
        assert!(s[0].summary.starts_with("Learn how"));
    }

    #[test]
    fn missing_fields_do_not_drop_a_result_but_a_url_less_one_is_thrown_away() {
        let body = r#"{"results":[
            {"url":"https://a.example/x"},
            {"title":"no url","content":"text"}
        ]}"#;
        let s = parse(body).unwrap();
        assert_eq!(s.len(), 1, "a result without a URL must be dropped");
        assert_eq!(s[0].source, "a.example");
        assert_eq!(s[0].title, "");
        assert_eq!(s[0].summary, "");
    }

    #[test]
    fn the_infobox_goes_first() {
        let body = r#"{"results":[{"url":"https://news.example/a","title":"news"}],
            "infoboxes":[{"infobox":"USD","content":"1 USD = 41,2 TRY",
                          "urls":[{"url":"https://tcmb.gov.tr/kur"}]}]}"#;
        let s = parse(body).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].summary, "1 USD = 41,2 TRY");
        assert_eq!(s[0].source, "tcmb.gov.tr");
    }

    #[test]
    fn an_infobox_with_empty_content_is_ignored() {
        let body = r#"{"results":[{"url":"https://a.example/x"}],
                        "infoboxes":[{"infobox":"empty","content":""}]}"#;
        assert_eq!(parse(body).unwrap().len(), 1);
    }

    #[test]
    fn empty_results_gives_an_empty_result_error() {
        assert_eq!(parse(r#"{"results":[]}"#), Err(WebError::EmptyResult));
    }

    /// When `formats: json` is off, SearXNG returns 200 + HTML.
    /// If this passes SILENTLY the user says "search job broken" and never finds
    /// out why.
    #[test]
    fn html_instead_of_json_is_invalid_json() {
        let e = parse("<!DOCTYPE html><html><body>search</body></html>").unwrap_err();
        assert!(matches!(e, WebError::InvalidJson(_)));
        assert!(e.to_string().contains("formats: json"));
    }

    #[test]
    fn a_missing_results_field_is_invalid_json() {
        let e = parse(r#"{"query":"x"}"#).unwrap_err();
        assert!(matches!(e, WebError::InvalidJson(_)));
    }

    #[test]
    fn a_non_array_results_field_is_invalid_json() {
        assert!(matches!(
            parse(r#"{"results":"none"}"#),
            Err(WebError::InvalidJson(_))
        ));
    }

    #[test]
    fn domain_drops_the_port_the_user_info_and_the_path() {
        assert_eq!(
            domain("https://user@www.mgm.gov.tr:8443/tahmin?il=34"),
            "www.mgm.gov.tr"
        );
        assert_eq!(domain("http://localhost:8080/a"), "localhost");
        assert_eq!(domain("broken-address"), "broken-address");
        assert_eq!(domain(""), "");
    }

    #[test]
    fn truncation_respects_the_word_boundary() {
        let t = "one two three four five six seven eight";
        let c = truncate_at_word(t, 20);
        assert!(c.ends_with('…'));
        assert!(!c.contains("fou…"), "must not cut mid-word: {c}");
        assert_eq!(truncate_at_word("short", 20), "short");
    }

    #[test]
    fn a_long_text_without_spaces_is_cut_hard() {
        // An implementation that walks back looking for a space would throw
        // away the entire summary here.
        let c = truncate_at_word(&"a".repeat(100), 10);
        assert_eq!(c.chars().count(), 11, "10 characters + ellipsis");
    }

    #[test]
    fn truncation_does_not_panic_on_a_multi_byte_character() {
        // Because it cuts with `chars().take()`, the UTF-8 boundary is always
        // valid.
        let c = truncate_at_word("çığır açan ölçüm şğüöçİ ile", 12);
        assert!(c.ends_with('…'));
    }
}
