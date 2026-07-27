//! HTML → plain text. DELIBERATELY SIMPLE.
//!
//! This is NOT an HTML parser and must not try to be. The single goal: reduce a
//! page to rough text the model can read. A real DOM parser (html5ever +
//! tendril + ...) is both a large dependency tree and gives us an accuracy we
//! do not need — the text the model sees is going to be truncated and
//! summarized anyway.
//!
//! WHAT IS DONE: script/style/noscript bodies are DROPPED (otherwise minified
//! JavaScript pours into the model — meaningless and a token killer), tags are
//! stripped, common entities are resolved, whitespace is normalized.
//!
//! WHAT IS NOT DONE: attribute interpretation, table structure, script
//! execution. On a badly structured page the output is bad; that is the
//! accepted cost.

/// Elements like `<script>`/`<style>`/`<noscript>` whose body is NOT text.
/// These are not merely stripped, they are dropped whole.
const DROPPED_BLOCKS: [&str; 4] = ["script", "style", "noscript", "svg"];

/// Tags that mean a line break in the text stream — a space is put in their
/// place so words do not stick together once stripped.
///
/// `td`/`th` WERE ADDED LATER and the reason is a measured failure: in the
/// previous version table CELLS did not count as separators, so
/// `<td>08:05</td><td>08:15</td>` poured into the text as `08:0508:15`. Two
/// adjacent departure times turned into one meaningless number — and ferry
/// timetables, stock tables and weather forecasts are exactly the kind of
/// content that gets SEARCHED for on the web. The absence of this line was a
/// silent data loss of the "we fetched the page but the data inside is
/// unreadable" class. `table` was added too, so two adjacent tables do not
/// stick together.
const BLOCK_TAGS: [&str; 15] = [
    "p", "br", "div", "li", "tr", "td", "th", "table", "h1", "h2", "h3", "h4", "h5", "h6",
    "section",
];

/// Converts an HTML body into plain text.
pub fn to_text(html: &str) -> String {
    let bodiless = drop_blocks(html);
    let stripped = strip_tags(&bodiless);
    normalize_whitespace(&simplify_punctuation(&resolve_entities(&stripped)))
}

/// Reduces typographic quotes and dashes to their ASCII equivalents.
///
/// WHY — measured, on the user's ferry question. The source page writes the
/// times with the Turkish apostrophe, and that mark is TYPOGRAPHIC: `08.55’te`,
/// `10.30’tadır`. They were going through to the model as-is, and Qwen3-4B was
/// CORRUPTING the digits while reading the time — the exact observed outputs:
/// `10.30’tadır` -> "130.0", `08.55’te` -> "08.555".
///
/// The cause is tokenization: `’` is a far rarer token than ASCII `'`, and next
/// to a digit it blurs the boundary of the number. Reducing to ASCII takes
/// nothing away from the MEANING of the text — an apostrophe is an apostrophe —
/// but gives the model a familiar boundary.
///
/// IT HAPPENS HERE, NOT in the `relevance` layer: `relevance` only simplifies
/// FOR SCORING and its output never goes to the model; the thing being
/// corrupted was the text that GOES to the model.
fn simplify_punctuation(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\u{2018}' | '\u{2019}' | '\u{201B}' | '\u{2032}' | '\u{00B4}' => '\'',
            '\u{201C}' | '\u{201D}' | '\u{201F}' | '\u{2033}' => '"',
            // Long dashes go down to a hyphen: in a range like `07:20 – 18:05`
            // the `–` is another rare token sitting between digits.
            '\u{2010}'..='\u{2015}' => '-',
            other => other,
        })
        .collect()
}

/// Deletes blocks like `<script>...</script>` together with their content.
///
/// If the closing tag never arrives (broken HTML) everything to the end of the
/// block is dropped: folding the content of an unterminated `<script>` into the
/// text is exactly what we want to avoid.
fn drop_blocks(html: &str) -> String {
    let mut output = String::with_capacity(html.len());
    let mut rest = html;
    'outer: while let Some(i) = rest.find('<') {
        output.push_str(&rest[..i]);
        let body = &rest[i..];
        for name in DROPPED_BLOCKS {
            if is_opening(body, name) {
                rest = after_block(body, name);
                continue 'outer;
            }
        }
        output.push('<');
        rest = &rest[i + 1..];
    }
    output.push_str(rest);
    output
}

/// Is this a `<name` OPENING tag — a closing one (`</name`) does not count.
///
/// The distinction is vital: a version that also counts the closing tag as
/// "block starts" will, on seeing `</script>`, try to skip the block again, the
/// cursor never advances, and the function goes into an INFINITE LOOP. (That is
/// exactly what happened in the first version.)
fn is_opening(body: &str, name: &str) -> bool {
    !body.starts_with("</") && tag_starts(body, name)
}

/// Moves from the opening tag PAST the block.
///
/// The closing tag's `>` is swallowed too: leaving the body AT THE START of the
/// closing tag makes the calling loop find the same closing tag again and
/// progress stops. If there is no closing tag at all (broken HTML) everything
/// to the end of the page is dropped — folding the content of an unterminated
/// `<script>` into the text is exactly what we avoid.
fn after_block<'a>(body: &'a str, name: &str) -> &'a str {
    // `to_ascii_lowercase` only touches ASCII bytes, the length does not
    // change; that is why the index coming out of it is valid on `body`.
    let lower = body.to_ascii_lowercase();
    let Some(j) = lower.find(&format!("</{name}")) else {
        return "";
    };
    match body[j..].find('>') {
        Some(k) => &body[j + k + 1..],
        None => "",
    }
}

/// Does `body` start with `<name` or `</name` (case-insensitive).
///
/// It checks the name boundary: `<scriptish>` is NOT a `script` tag, and a
/// naive `starts_with` would swallow it too.
fn tag_starts(body: &str, name: &str) -> bool {
    let b = body.strip_prefix('<').unwrap_or(body);
    let b = b.strip_prefix('/').unwrap_or(b);
    let Some(head) = b.get(..name.len()) else {
        return false;
    };
    if !head.eq_ignore_ascii_case(name) {
        return false;
    }
    b[name.len()..]
        .chars()
        .next()
        .is_none_or(|c| !c.is_ascii_alphanumeric() && c != '-')
}

/// Deletes tags; leaves a space in place of block tags.
fn strip_tags(html: &str) -> String {
    let mut output = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        output.push_str(&rest[..i]);
        let body = &rest[i..];
        if BLOCK_TAGS.iter().any(|name| tag_starts(body, name)) {
            output.push(' ');
        }
        // If there is no closing `>`, the remainder counts wholly as a tag and
        // is dropped; otherwise junk starting with `<` would leak into the text.
        rest = match body.find('>') {
            Some(j) => &body[j + 1..],
            None => "",
        };
    }
    output.push_str(rest);
    output
}

/// Resolves the common HTML entities.
///
/// The full entity table (2000+ entries) WAS NOT ADDED: these are the handful
/// seen on real pages, and the rest arrive as numeric escapes anyway and are
/// resolved generically below.
fn resolve_entities(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('&') {
        output.push_str(&rest[..i]);
        let body = &rest[i..];
        // If the semicolon is far away this is not an entity but a plain `&`.
        //
        // It is searched with `char_indices`, NOT with `body[..12]`: slicing
        // through the middle of a multi-byte character is a panic, and this
        // input comes to us from the outside world — not one line open to a
        // panic may be left there.
        let end = body
            .char_indices()
            .take(12)
            .find(|(_, c)| *c == ';')
            .map(|(j, _)| j);
        match end.map(|j| (&body[1..j], j)) {
            Some((name, j)) => {
                output.push_str(&resolution(name).unwrap_or_else(|| body[..=j].to_string()));
                rest = &body[j + 1..];
            }
            None => {
                output.push('&');
                rest = &body[1..];
            }
        }
    }
    output.push_str(rest);
    output
}

fn resolution(name: &str) -> Option<String> {
    let s = match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" | "#39" => "'",
        // A hard space becomes a normal space; otherwise the whitespace
        // normalization below does not see it and the text stays bloated.
        "nbsp" | "#160" => " ",
        "hellip" => "…",
        "mdash" => "—",
        "ndash" => "–",
        // MEASURED — the Sehir Hatlari timetable page. The page writes accented
        // letters with NAMED entities: `Kadık&ouml;y`, `&Uuml;sk&uuml;dar`,
        // `&ccedil;`. Because the table was not resolved, those names went to
        // the model CORRUPTED — the model could not read which column was
        // Uskudar and which was Ortakoy, and paired departures with arrivals at
        // random. On top of that `relevance_score` was missing the word
        // overlap: the word "uskudar" does not occur in the text
        // "&Uuml;sk&uuml;dar", so the CORRECT timetable window was losing
        // points.
        "szlig" => "ß",
        "deg" => "°",
        "euro" => "€",
        "pound" => "£",
        "cent" => "¢",
        "yen" => "¥",
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "middot" => "·",
        "bull" => "•",
        "laquo" => "«",
        "raquo" => "»",
        "lsquo" => "‘",
        "rsquo" => "’",
        "ldquo" => "“",
        "rdquo" => "”",
        "times" => "×",
        "divide" => "÷",
        "aelig" => "æ",
        "oelig" => "œ",
        _ => return accented_resolution(name).or_else(|| numeric_resolution(name)),
    };
    Some(s.to_string())
}

/// Named entities carrying an accent suffix: `ouml`, `Uuml`, `ccedil`,
/// `eacute`...
///
/// WHY A RULE AND NOT A TABLE: Latin-1's named entities are regular — the first
/// character is the BASE LETTER, the rest is the ACCENT NAME. Instead of
/// writing about 60 entries one by one, writing eight accent groups is both
/// shorter and makes a gap visible. Uppercase is not listed separately: every
/// lowercase accented letter has an uppercase form in Latin-1 and
/// `to_uppercase` gives it correctly (`&yuml;` -> ÿ -> Ÿ included).
fn accented_resolution(name: &str) -> Option<String> {
    /// (accent name, base letters, their equivalents) — the order matches
    /// position by position.
    const ACCENTS: [(&str, &str, &str); 8] = [
        ("grave", "aeiou", "àèìòù"),
        ("acute", "aeiouy", "áéíóúý"),
        ("circ", "aeiou", "âêîôû"),
        ("tilde", "ano", "ãñõ"),
        ("uml", "aeiouy", "äëïöüÿ"),
        ("ring", "a", "å"),
        ("cedil", "c", "ç"),
        ("slash", "o", "ø"),
    ];
    let letter = name.chars().next()?;
    if !letter.is_ascii_alphabetic() {
        return None;
    }
    // The first character is ASCII, so the byte slice is safe.
    let accent = &name[1..];
    let lower = letter.to_ascii_lowercase();
    let (_, bases, equivalents) = ACCENTS.iter().find(|(a, _, _)| *a == accent)?;
    let position = bases.chars().position(|c| c == lower)?;
    let resolved = equivalents.chars().nth(position)?;
    Some(if letter.is_ascii_uppercase() {
        resolved.to_uppercase().to_string()
    } else {
        resolved.to_string()
    })
}

/// Numeric escapes of the form `&#8217;` / `&#x2019;`.
fn numeric_resolution(name: &str) -> Option<String> {
    let body = name.strip_prefix('#')?;
    let code = match body.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => body.parse::<u32>().ok()?,
    };
    char::from_u32(code).map(String::from)
}

/// Collapses consecutive whitespace into one space; does not preserve line
/// structure.
///
/// LINE STRUCTURE IS DROPPED DELIBERATELY: in stripped HTML the line breaks
/// come from the source's indentation, not from meaning. Preserving them would
/// teach the model not the page's formatting but the HTML author's tab habits.
fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_stripped_and_whitespace_is_normalized() {
        let h = "<html>  <body><h1>Title</h1>\n\n<p>One   two</p></body></html>";
        assert_eq!(to_text(h), "Title One two");
    }

    #[test]
    fn script_and_style_bodies_are_dropped_whole() {
        let h =
            "<p>before</p><script>var a = 1 < 2;</script><style>p{color:red}</style><p>after</p>";
        let t = to_text(h);
        assert_eq!(t, "before after");
        assert!(!t.contains("var a"), "JS must not leak into the text");
        assert!(!t.contains("color"), "CSS must not leak into the text");
    }

    #[test]
    fn an_unclosed_script_is_dropped_to_the_end() {
        let t = to_text("<p>start</p><script>hidden content and the rest");
        assert_eq!(t, "start");
    }

    #[test]
    fn a_similarly_named_tag_is_not_dropped_by_mistake() {
        // `<scriptish>` is NOT a script block — naive matching blows up here.
        let t = to_text("<scriptish>visible</scriptish>");
        assert_eq!(t, "visible");
    }

    #[test]
    fn entities_are_resolved() {
        let t = to_text("<p>a&amp;b &lt;c&gt; &quot;d&quot; e&#39;f&nbsp;g &hellip;</p>");
        assert_eq!(t, "a&b <c> \"d\" e'f g …");
    }

    #[test]
    fn numeric_entities_are_resolved() {
        // They are resolved and THEN reduced to ASCII (see
        // `simplify_punctuation`): if either stage failed, this line would stay
        // as `&#8217;`.
        assert_eq!(to_text("<p>&#8217;&#x2019;</p>"), "''");
        assert_eq!(
            to_text("<p>&#8364;5</p>"),
            "€5",
            "a currency sign must NOT go down to ASCII"
        );
    }

    /// REGRESSION TEST — clock corruption. The source page says `10.30’tadır`
    /// and the model was reading it as "130.0".
    #[test]
    fn typographic_punctuation_goes_down_to_ascii() {
        assert_eq!(
            to_text("<p>08.55’te, 10.30’tadır</p>"),
            "08.55'te, 10.30'tadır"
        );
        assert_eq!(
            to_text("<p>&ldquo;a&rdquo; &mdash; 07:20 &ndash; 18:05</p>"),
            "\"a\" - 07:20 - 18:05"
        );
        // Letters and digits are UNTOUCHED: the simplification is punctuation only.
        assert_eq!(to_text("<p>Üsküdar 07:45</p>"), "Üsküdar 07:45");
    }

    /// REGRESSION TEST — a fragment taken from the real Sehir Hatlari page.
    /// Until it is resolved the model cannot read the column headers and
    /// `relevance_score` cannot see the word "uskudar".
    #[test]
    fn named_accented_entities_are_resolved() {
        assert_eq!(
            to_text("<p>Kadık&ouml;y &Uuml;sk&uuml;dar Ortak&ouml;y G&uuml;nleri</p>"),
            "Kadıköy Üsküdar Ortaköy Günleri"
        );
        // The accent rule covers the rest of Latin-1 too.
        assert_eq!(
            to_text("<p>&eacute;&Agrave;&ntilde;&ccedil;&oslash;&aring;</p>"),
            "éÀñçøå"
        );
        // On weather pages the degree sign arrives as a named entity; unresolved,
        // `has_unit` does not count it as a value with a unit and the summary
        // loses points.
        assert_eq!(to_text("<p>28&deg;C &euro;5 a&rsquo;b</p>"), "28°C €5 a'b");
    }

    #[test]
    fn an_ampersand_that_is_not_an_entity_is_preserved() {
        assert_eq!(
            to_text("<p>a & b, c &unknown; d</p>"),
            "a & b, c &unknown; d"
        );
    }

    #[test]
    fn block_tags_do_not_glue_words_together() {
        // Without the inserted space this would come out as "onetwothree" and
        // the model would see a single word.
        assert_eq!(
            to_text("<li>one</li><li>two</li><div>three</div>"),
            "one two three"
        );
    }

    /// REGRESSION TEST — table cells. In the previous version `td` did not
    /// count as a block, so a ferry timetable poured into the text as
    /// `08:0508:1508:55`: the page had been fetched but the data inside had
    /// become unreadable. Most of what gets searched for on the web
    /// (timetables, exchange rates, forecasts) lives in a table.
    #[test]
    fn table_cells_do_not_stick_together() {
        let h = "<table><tr><td>08:05</td><td>08:15</td></tr><tr><td>08:55</td><td>09:05</td></tr></table>";
        assert_eq!(to_text(h), "08:05 08:15 08:55 09:05");
        // A header cell separates too: "OrtaköyÜsküdar" must not be one word.
        assert_eq!(
            to_text("<tr><th>Ortaköy</th><th>Üsküdar</th></tr>"),
            "Ortaköy Üsküdar"
        );
    }

    #[test]
    fn an_unclosed_tag_does_not_leak_into_the_text() {
        assert_eq!(to_text("visible <p class=\"x"), "visible");
    }

    /// THE CONTRACT for broken input: no panic, no `<...>` content leaking.
    ///
    /// Stray `>` marks staying in the text is ACCEPTED behaviour. Cleaning
    /// those up too would need an arbitrary rule like "every `>` is noise",
    /// whereas in plain text `>` is a legitimate character (quotation,
    /// comparison). This function is not an HTML parser but a rough stripper —
    /// giving bad output on broken input is a deliberate cost.
    #[test]
    fn broken_input_does_not_panic_and_tag_content_does_not_leak() {
        assert_eq!(to_text(""), "");
        assert_eq!(to_text("plain text"), "plain text");
        assert_eq!(to_text("&"), "&");
        assert_eq!(to_text("<<<>>>"), ">>");
        // The real guarantee: the INSIDE of a tag never comes out.
        assert_eq!(to_text("<a href=\"hidden\">visible</a>"), "visible");
        assert!(!to_text("<p onclick=\"code()\">x</p>").contains("onclick"));
    }

    #[test]
    fn multi_byte_content_is_not_corrupted() {
        assert_eq!(to_text("<p>çığır açan ölçüm</p>"), "çığır açan ölçüm");
    }

    /// REGRESSION TEST: in the first version `drop_blocks` counted the closing
    /// tag as "block starts" again, the cursor did not advance and the function
    /// went into an INFINITE LOOP (the test run hung at 100% CPU without
    /// erroring). This test catches that loop: if it comes back, this is where
    /// it hangs.
    #[test]
    fn consecutive_dropped_blocks_do_not_loop_forever() {
        let h = "<script>a</script><style>b</style><script>c</script><p>end</p>";
        assert_eq!(to_text(h), "end");
    }

    /// A multi-byte character after `&`: the naive `body[..12]` slice would
    /// split the UTF-8 boundary here and PANIC. The input comes from the
    /// outside world.
    #[test]
    fn a_multi_byte_character_after_an_ampersand_does_not_panic() {
        assert_eq!(to_text("<p>a &çığırğüö b</p>"), "a &çığırğüö b");
        assert_eq!(to_text("&ç"), "&ç");
    }
}
