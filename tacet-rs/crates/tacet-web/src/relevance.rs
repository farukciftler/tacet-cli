//! Relevance measurement: how well a piece of text answers the query.
//!
//! WHY THIS FILE EXISTS — a measured failure. For the query
//! "ortakoy uskudar vapur saatleri" SearXNG returns 37 results and the FIRST
//! result's `content` field is this:
//!
//! ```text
//! ORTAKÖY - ÜSKÜDAR - KADIKÖY · BOĞAZ'dan Geliş · ANADOLUKAVAĞI -
//! RUMELİKAVAĞI - SARIYER · KÜÇÜKSU - BEŞİKTAŞ - KABATAŞ · ÇENGELKÖY ...
//! ```
//!
//! That is the page's NAVIGATION MENU — the list of all Sehir Hatlari lines.
//! The model faithfully copied it and wrote "Stops: Anadolukavagi, Sariyer,
//! Istinye". So the observed "hallucination" did NOT come from the model's
//! memory, it came from the menu text at the top of the ranking; to the user
//! the two are indistinguishable and both are wrong. The BODY of the same page
//! has 76 real departure times and none of them reached the model.
//!
//! Two measurements come out of that, and they are this file's two functions:
//!
//! 1. **Relevance alone is not enough.** The menu text contains every word of
//!    the query ("ortakoy", "uskudar") but not a single FACT. That is why the
//!    scoring adds DATA DENSITY to word overlap: clocks, temperatures, prices,
//!    percentages. If the thing asked for is a concrete value, the text that
//!    CARRIES that value must come first.
//! 2. **Not the whole page, the right window of it.** The first 3000 characters
//!    of the page text are navigation and a cookie warning; the timetable is
//!    further down. Truncating from the front (`truncate_at_word`) picks
//!    exactly the menu and throws away the answer. That is why
//!    `relevant_section` cuts not from a fixed place but from the
//!    BEST-SCORING window.
//!
//! LANGUAGE: it works with Turkish text but is not TIED to Turkish — the only
//! language-specific assumption is the stop word list below, and the scoring
//! works even if that list is passed empty. Numbers and units are
//! language-independent; the real signal is already there.
//!
//! NO NETWORK: its input is a `&str`, all of it testable without going online.

/// Words that carry no meaning, occur in every text and therefore ruin the
/// overlap signal. Left in the query, a phrase like "how much" matches every
/// page and the scoring loses its discriminating power.
///
/// MULTILINGUAL BY CONSTRUCTION: the list holds both the Turkish and the
/// English stop words, because the queries themselves are still user text and
/// can arrive in either language. Dropping the Turkish entries would be a
/// product decision, not a translation.
const STOP_WORDS: [&str; 24] = [
    "ne", "nedir", "kac", "kadar", "nasil", "hangi", "icin", "ile", "ve", "veya", "bir", "bu",
    "su", "the", "what", "how", "much", "many", "is", "are", "of", "in", "for", "and",
];

/// Minimum length for a word to count as meaningful (in its simplified form).
const MIN_WORD: usize = 3;

/// Makes text comparable: lowercase + Turkish accent folding.
///
/// ACCENT FOLDING IS MANDATORY, not a quiet preference: even within the same
/// page a line name occurs as "Üsküdar" in one place, "USKUDAR" in another and
/// "uskudar" in the URL. Without folding, word overlap counts those three
/// spellings as THREE SEPARATE words and the right page loses points purely
/// because of a spelling choice.
///
/// `to_lowercase` alone is NOT ENOUGH: it turns 'I' into 'i', whereas the
/// Turkish equivalent is 'ı'. Since folding sends both to 'i', that trap closes
/// as well.
pub fn simplify(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        output.push(match c {
            'ç' => 'c',
            'ğ' => 'g',
            'ı' => 'i',
            'ö' => 'o',
            'ş' => 's',
            'ü' => 'u',
            'â' => 'a',
            'î' => 'i',
            'û' => 'u',
            other => other,
        });
    }
    output
}

/// Extracts the meaningful keywords from a query.
///
/// Repeats are DROPPED: a word occurring twice does not make it twice as
/// important, it only inflates pages that use that word a lot.
pub fn keywords(query: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    for raw in simplify(query).split(|c: char| !c.is_alphanumeric()) {
        if raw.chars().count() < MIN_WORD || STOP_WORDS.contains(&raw) {
            continue;
        }
        if !words.iter().any(|w| w == raw) {
            words.push(raw.to_string());
        }
    }
    words
}

/// The text's CONCRETE DATA density.
///
/// What is asked for is usually a value: a departure time, a temperature, a
/// price, a rate. The sentence "The sailing times are on our website" overlaps
/// perfectly with the query but carries zero facts; "08:05 08:15 08:55" does
/// the opposite. This function measures that difference and becomes the second
/// axis of the scoring.
///
/// THE WEIGHTS ARE COARSE AND DELIBERATELY SO: the goal is to RANK one text
/// against another, not to produce an absolute "amount of information". The
/// clock gets the highest score because the `08:05` shape is rare in ordinary
/// text; a bare number gets the lowest because years, page numbers and phone
/// numbers are bare numbers too.
pub fn data_density(text: &str) -> usize {
    text.split_whitespace().map(token_score).sum()
}

/// The density of the STRONG data signals only: clocks and values with units.
///
/// The difference from `data_density` is that IT DOES NOT COUNT BARE NUMBERS,
/// and that difference comes from a measured failure. For ranking, a bare
/// number is a weak but real signal; but for the decision "is the answer in the
/// summaries, should I fetch the page" it is POISON. Search summaries are full
/// of dates and comment counts ("7 Eyl 2025 ...", "24 May 2021 ...", "View all
/// 10 comments") — none of them answer the user's question, but all of them are
/// bare numbers. In the observed run exactly that happened: the dates met the
/// threshold, the page was not fetched, and the model again said "you can check
/// the website". A clock or a value with a unit, on the other hand, IS the
/// answer.
pub fn strong_data_density(text: &str) -> usize {
    text.split_whitespace().map(token_score).filter(|s| *s >= 2).sum()
}

/// The number of CLOCK-shaped tokens in the text.
///
/// WHY A SEPARATE MEASURE — a measured failure. A window cut out of a timetable
/// page comes out like this:
///
/// ```text
/// Ortaköy Üsküdar Kadıköy Kalkış Kalkış Varış 08:05 08:15 * 08:55 09:05 09:30
/// ```
///
/// The numbers are right, but because the HTML table was reduced to plain text
/// there is no row/column boundary: which time belongs to which direction IS
/// NOT WRITTEN in the text. The model looks at the gap and invents a pairing,
/// and what the user sees is an impossible pair like "departure 08:55, arrival
/// 07:45". A wrong pairing is WORSE than missing information: the user trusts
/// it and misses the ferry.
///
/// This counter answers the question "is this window a timetable dump" cheaply,
/// and `web_search` uses it to add the DO NOT pair warning for the model.
pub fn clock_count(text: &str) -> usize {
    text.split_whitespace().filter(|t| token_score(t) == 3).count()
}

/// A single token's data score: clock 3, value with a unit 2, bare number 1.
fn token_score(token: &str) -> usize {
    let t = token.trim_matches(|c: char| !c.is_alphanumeric() && !"°%₺$€£:.,".contains(c));
    if t.is_empty() || !t.chars().any(|c| c.is_ascii_digit()) {
        return 0;
    }
    if is_clock(t) {
        return 3;
    }
    if has_unit(t) { 2 } else { 1 }
}

/// `08:05`, `8.30`, `23:59` — the clock shape.
///
/// THE DOT IS ACCEPTED TOO: on Turkish pages the time is frequently written
/// `08.55` (observed example: a trhaber page saying "ilk sefer saat 08.55'te").
/// Looking only for a colon would leave those pages at zero points.
///
/// BUT WITH A DOT SEPARATOR THE HOUR MUST BE TWO DIGITS. The reason: `1.25` is
/// not a time, it is a rate or a price, and finance pages are full of them; a
/// rule counting all of those as "clock" would put a stock page ahead of the
/// timetable on a query asking about times. With a colon separator there is no
/// such confusion, so a single-digit hour (`8:30`) is free there.
fn is_clock(token: &str) -> bool {
    let Some((left, right)) = token.split_once([':', '.']) else { return false };
    let dotted = !token.contains(':');
    // Ignore seconds or a suffix: `08:05:30` is a time too, `08.05.2026` is not.
    let right = right.split([':', '.']).next().unwrap_or(right);
    let (Ok(h), Ok(m)) = (left.parse::<u32>(), right.parse::<u32>()) else { return false };
    // The first two parts of the date `08.05.2026` look like a time; what tells
    // them apart is the four-digit third part.
    if token.split(['.', ':']).nth(2).is_some_and(|p| p.len() == 4) {
        return false;
    }
    let min_digits = if dotted { 2 } else { 1 };
    (min_digits..=2).contains(&left.len()) && right.len() == 2 && h <= 23 && m <= 59
}

/// Does it carry a unit/currency sign attached to a number.
fn has_unit(token: &str) -> bool {
    if token.contains(['°', '%', '₺', '$', '€', '£']) {
        return true;
    }
    let t = simplify(token);
    let letters: String = t.chars().filter(|c| c.is_alphabetic()).collect();
    matches!(
        letters.as_str(),
        "c" | "f" | "tl" | "usd" | "try" | "eur" | "btc" | "km" | "kg" | "kmsa" | "mb" | "gb"
    )
}

/// A text's total relevance score for a query: word overlap + data density.
///
/// Word overlap is WEIGHTED (x3): an off-topic but number-packed table (the ad
/// strip on a stock page) must not come first merely for being dense. First the
/// RIGHT PAGE, then the dense part of that page.
pub fn relevance_score(text: &str, words: &[String]) -> usize {
    let t = simplify(text);
    let overlap = words.iter().filter(|w| t.contains(w.as_str())).count();
    overlap * 3 + data_density(text)
}

/// The chunk size (in characters) of the quote that goes to the model. The
/// window slides at this granularity.
const CHUNK_SIZE: usize = 90;

/// Cuts out the section of the page text that BEST answers the query.
///
/// WHY NOT TRUNCATE FROM THE FRONT: the beginning of a page's text is the
/// navigation menu, the cookie warning and a crowd of headings — measured, on
/// the Sehir Hatlari page the timetable only starts around character 3000.
/// `truncate_at_word(text, 700)` picks exactly the menu and sees none of the 76
/// departure times.
///
/// HOW: the text is split into chunks of `CHUNK_SIZE`, each chunk is scored,
/// then the highest-scoring CONSECUTIVE run of chunks that fits the cap is
/// chosen (a maximum-sum-subarray with a limit). Being consecutive is
/// mandatory: lining up detached best chunks invents an adjacency that does not
/// exist in the text, and the model ties the numbers of two separate lines
/// together.
///
/// For empty or wholly irrelevant text it returns EMPTY — "there is no relevant
/// section" is valuable information for the caller; forcing something out would
/// mean presenting irrelevant text to the model as "a quote from the page".
pub fn relevant_section(text: &str, words: &[String], cap: usize) -> String {
    let chunks = chunk(text, CHUNK_SIZE);
    if chunks.is_empty() || cap == 0 {
        return String::new();
    }
    let scores: Vec<usize> = chunks.iter().map(|c| relevance_score(c, words)).collect();

    let (mut best_score, mut best) = (0usize, 0usize..0usize);
    for start in 0..chunks.len() {
        let (mut length, mut total) = (0usize, 0usize);
        for end in start..chunks.len() {
            let grown = length + chunks[end].chars().count() + 1;
            if grown > cap {
                break;
            }
            length = grown;
            total += scores[end];
            if total > best_score {
                best_score = total;
                best = start..end + 1;
            }
        }
    }
    if best_score == 0 {
        return String::new();
    }

    // TRIM THE ZERO-SCORE EDGES. Because the maximum-sum search does not lower
    // the score, it is happy to pull irrelevant chunks at the front and the
    // back into the window: the navigation menu scores zero, but including it
    // costs nothing either, so it gets included. What it does cost is BUDGET —
    // half of the 700 characters the model sees becomes "Kariyer · Staj
    // Basvurusu". Edge trimming is the cheap way to pick the window that
    // delivers the same score in LESS space.
    let (mut start, mut end) = (best.start, best.end);
    while start < end && scores[start] == 0 {
        start += 1;
    }
    while end > start && scores[end - 1] == 0 {
        end -= 1;
    }
    chunks[start..end].join(" ")
}

/// Splits the text into chunks of at most `size` characters, at word
/// boundaries.
///
/// The word boundary matters here too: a split that cuts a time (`08:05`) in
/// half devalues both halves from a data point of view.
fn chunk(text: &str, size: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && current.chars().count() + word.chars().count() + 1 > size {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplification_folds_the_turkish_accents() {
        assert_eq!(simplify("Üsküdar ÇAĞRI ışık Öğün"), "uskudar cagri isik ogun");
        // The trap of the letter 'I': `to_lowercase` makes it 'i', and folding
        // makes 'ı' into 'i' — the two spellings meet in the same place.
        assert_eq!(simplify("ISIK"), simplify("ışık"));
    }

    #[test]
    fn keywords_drop_the_stop_words_and_the_short_ones() {
        let k = keywords("bitcoin fiyatı ne kadar?");
        assert_eq!(k, vec!["bitcoin", "fiyati"]);
        // No inflation from repeats.
        assert_eq!(keywords("vapur vapur vapur"), vec!["vapur"]);
        assert!(keywords("ne kadar").is_empty());
    }

    #[test]
    fn the_clock_shape_is_recognized_and_a_date_is_not() {
        assert!(is_clock("08:05"));
        assert!(is_clock("8:30"), "with a colon separator a single digit is free");
        assert!(is_clock("08.55"), "on Turkish pages the time is also written with a dot");
        assert!(is_clock("23:59"));
        assert!(is_clock("07:45:10"));
        assert!(!is_clock("24:00"), "invalid time");
        assert!(!is_clock("08.05.2026"), "a date must not count as a time");
        // DELIBERATE TRADE-OFF: `1.25` is a rate/price and finance pages are
        // full of them. The cost of requiring a two-digit hour with a dot
        // separator is missing the few pages that write "8.30"; the gain is
        // that a stock table does not get ahead of the timetable on a query
        // about times.
        assert!(!is_clock("1.25"));
        assert!(!is_clock("abc"));
    }

    #[test]
    fn data_density_separates_facts_from_talk() {
        let talk = "Güncel sefer saatleri yukarıdaki tablolarda yer almaktadır.";
        let fact = "Ortaköy Kalkış 08:05 08:55 10:15 11:50 Üsküdar 08:15 09:05";
        assert_eq!(data_density(talk), 0, "a sentence carrying zero facts must score zero");
        assert!(data_density(fact) >= 18, "{}", data_density(fact));
    }

    /// The measurement the fetch decision rests on MUST NOT BE FOOLED by dates.
    #[test]
    fn strong_density_does_not_count_dates_but_does_count_clocks() {
        let dated = "7 Eyl 2025 ... 24 May 2021 ... View all 10 comments · 2 sefer daha";
        assert_eq!(strong_data_density(dated), 0, "a date or a count is not an answer");
        assert!(data_density(dated) > 0, "there is still a weak signal for ranking");

        assert!(strong_data_density("Ortaköy 08:05 Üsküdar 08:15") >= 6);
        assert!(strong_data_density("sıcaklık 28°C, en düşük 21°C, nem %76") >= 6);
    }

    #[test]
    fn the_clock_count_tells_a_timetable_dump_apart() {
        let timetable = "Kalkış Kalkış Varış 08:05 08:15 08:55 09:05 09:30 10:15 10:25 10:50";
        assert_eq!(clock_count(timetable), 8);
        // A date or a bare number MUST NOT BE MISTAKEN for a timetable,
        // otherwise a pointless warning eats the model's budget on every news
        // page.
        assert_eq!(clock_count("7 Eyl 2025 · 24 May 2021 · 10 yorum · 2026"), 0);
        assert_eq!(clock_count("sıcaklık 28°C, nem %76"), 0);
    }

    #[test]
    fn values_with_units_score_higher_than_bare_numbers() {
        assert_eq!(token_score("32°C"), 2);
        assert_eq!(token_score("$65,287.15"), 2);
        assert_eq!(token_score("%1,20"), 2);
        assert_eq!(token_score("2026"), 1);
        assert_eq!(token_score("vapur"), 0);
    }

    /// THE VERBATIM TEST OF THE OBSERVED FAILURE: the menu text carries every
    /// word of the query but zero facts; the timetable does the opposite. If
    /// the scoring does not pick the timetable, the user gets the "check the
    /// website" answer again.
    #[test]
    fn the_menu_text_cannot_beat_the_timetable() {
        let words = keywords("ortaköy üsküdar vapur saatleri");
        let menu = "ORTAKÖY - ÜSKÜDAR - KADIKÖY · ANADOLUKAVAĞI - RUMELİKAVAĞI - SARIYER \
                    · KÜÇÜKSU - BEŞİKTAŞ - KABATAŞ · ÇENGELKÖY - İSTİNYE";
        let timetable = "Ortaköy Üsküdar Kalkış 08:05 08:15 08:55 09:05 10:15 10:25 11:50 12:00";
        assert!(
            relevance_score(timetable, &words) > relevance_score(menu, &words),
            "menu {} vs timetable {}",
            relevance_score(menu, &words),
            relevance_score(timetable, &words)
        );
    }

    /// The reason `relevant_section` exists: the answer is not at the FRONT of
    /// the page.
    #[test]
    fn the_relevant_section_skips_the_leading_navigation_and_finds_the_timetable() {
        let page = format!(
            "{} Ortaköy Üsküdar Kalkış 08:05 08:15 08:55 09:05 10:15 10:25 11:50 12:00 15:50 16:00",
            "Hakkımızda Vizyon ve Değerler İletişim Kariyer Staj Başvurusu Çerez Politikası "
                .repeat(30)
        );
        let section = relevant_section(&page, &keywords("ortaköy üsküdar vapur saatleri"), 300);
        assert!(section.contains("08:05"), "the timetable should have been picked: {section}");
        assert!(!section.contains("Kariyer"), "the navigation text should not have been picked: {section}");
        assert!(section.chars().count() <= 300, "{}", section.chars().count());
    }

    #[test]
    fn the_relevant_section_stays_within_the_cap_and_preserves_adjacency() {
        let page = "unrelated ".repeat(50) + "08:05 08:15 " + &"unrelated ".repeat(50) + "09:00";
        let s = relevant_section(&page, &keywords("kalkis saatleri"), 200);
        assert!(s.chars().count() <= 200);
        // Detached chunks are not joined: two distant times cannot be in the
        // same quote.
        assert!(!(s.contains("08:05") && s.contains("09:00")), "detached chunks were joined: {s}");
    }

    #[test]
    fn irrelevant_or_empty_text_returns_empty() {
        assert_eq!(relevant_section("", &keywords("vapur"), 200), "");
        assert_eq!(relevant_section("lorem ipsum dolor", &keywords("vapur"), 200), "");
        // A cap of zero is empty too: if the caller has no budget we produce no
        // quote.
        assert_eq!(relevant_section("08:05 vapur", &keywords("vapur"), 0), "");
    }

    #[test]
    fn multi_byte_text_does_not_panic() {
        let t = "çığır açan ölçüm 08:05 şğüöçİ 32°C";
        assert!(!relevant_section(t, &keywords("ölçüm sıcaklık"), 100).is_empty());
        assert!(data_density(t) > 0);
    }

    #[test]
    fn chunking_does_not_split_a_word_in_half() {
        let c = chunk("one two three four five six seven eight nine ten", 12);
        assert!(c.iter().all(|x| x.chars().count() <= 12), "{c:?}");
        assert_eq!(c.join(" "), "one two three four five six seven eight nine ten");
    }
}

#[cfg(test)]
mod network_tests {
    use super::*;

    /// REAL NETWORK — `cargo test -p tacet-web smoke_from_page -- --ignored --nocapture`
    ///
    /// It measures the whole chain: fetch the page → strip the HTML → cut the
    /// relevant section. In the observed failure this page NEVER reached the
    /// model; and when it did, front truncation would pick the navigation menu.
    /// The test requires the cut section to contain REAL departure times.
    #[test]
    #[ignore = "requires the real network"]
    fn smoke_from_page_extracts_the_timetable() {
        let address = "https://sehirhatlari.istanbul/tr/seferler/ic-hatlar/bogaz-hatlari/ortakoy-uskudar-kadikoy-173";
        let text = crate::WebSearchClient::new().page_text(address).expect("the page must arrive");
        let words = keywords("ortaköy üsküdar vapur saatleri");

        println!("--- PAGE LEN : {} characters", text.chars().count());
        println!("--- FRONT TRUNCATION (old behaviour):\n{}", crate::truncate_at_word(&text, 300));
        let section = relevant_section(&text, &words, 700);
        println!("--- RELEVANT SECTION ({} characters):\n{section}", section.chars().count());

        assert!(section.chars().count() <= 700);
        let clocks = section.split_whitespace().filter(|t| token_score(t) == 3).count();
        assert!(clocks >= 5, "at least five times should have come out of the timetable, got {clocks}");
    }
}
