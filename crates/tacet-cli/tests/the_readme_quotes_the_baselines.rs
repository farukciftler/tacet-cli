//! EVERY NUMBER THE README QUOTES FROM A COMMITTED REPORT IS READ BACK FROM IT.
//!
//! WHY THIS FILE EXISTS (audit D1–D5, 9 Oct 2026). Five paragraphs disagreed
//! with the files they cited, and each disagreement had the same shape: a
//! report in `crates/tacet-eval/baselines/` was replaced or added, and the
//! prose that quoted it was not re-derived.
//!
//! * `qwen3-4b-cuda-3090.json` was replaced by a run seven commits later; the page
//!   kept the old run's 7m 34s and 6.3× while the file said 426,988 ms — 7m 07s,
//!   6.7× (D1). The comparator block under it was the old pair's too.
//! * "`baselines/` holds two files and neither is a 3090 run" — it held seven,
//!   two of them 3090 runs (D2).
//! * The Turkish corpus, run on the 3090, was set beside the METAL suite and
//!   called "the same model"; the same card's suite was in the next file (D3).
//! * The seven-language table cited `qwen3-4b-both.json` for an `en` column of
//!   133/160; the file says 136/160 (D5).
//!
//! THE PAGE WAS NEVER WRONG WHEN IT WAS WRITTEN — the files moved under it.
//! That is a failure only a mechanism catches, and a comment asking the next
//! person to remember is not one. So each test below derives the sentence the
//! page must contain from the JSON and looks for it, and a page or a report
//! changed alone fails the build with the line it expected.
//!
//! WHAT IT DOES NOT PIN: numbers no committed file holds. The model table's
//! rows were hand-recorded on a rented card with no report kept; for those the
//! test can check the arithmetic (each score from its own counts) and the
//! shape (one table, four rows), and the README says the rest is unmeasured.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use tacet_eval::bench::BenchScore;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives at <repo>/crates/tacet-cli")
        .to_path_buf()
}

fn baselines() -> PathBuf {
    repo_root().join("crates/tacet-eval/baselines")
}

fn readme() -> String {
    std::fs::read_to_string(repo_root().join("README.md")).expect("README.md is readable")
}

/// The README with every run of whitespace folded to one space, so a sentence
/// the page wraps across two lines is still one string to look for.
fn folded(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn report(name: &str) -> Value {
    let path = baselines().join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} is readable: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name} is JSON: {e}"))
}

fn count(v: &Value, key: &str) -> usize {
    v[key]
        .as_u64()
        .unwrap_or_else(|| panic!("`{key}` is a count in this report")) as usize
}

/// One axis of a selection report: (passed, total).
fn axis(v: &Value, name: &str) -> (usize, usize) {
    (
        count(v, &format!("{name}_passed")),
        count(v, &format!("{name}_total")),
    )
}

/// "85.0%" — one decimal, the way every table on the page prints a rate.
fn pct((p, t): (usize, usize)) -> String {
    format!("{:.1}%", 100.0 * p as f64 / t as f64)
}

/// "7m 07s" from milliseconds, rounded to the second: 426,988 ms is 7m 07s,
/// and truncating would print 7m 06s for a run that took 426.988 s.
fn minutes_seconds(ms: u64) -> String {
    let s = (ms as f64 / 1000.0).round() as u64;
    format!("{}m {:02}s", s / 60, s % 60)
}

fn wall_ms(v: &Value) -> u64 {
    v["wall_ms"]
        .as_u64()
        .expect("the report records its wall clock")
}

fn require(page: &str, want: &str, why: &str) {
    assert!(
        folded(page).contains(&folded(want)),
        "README.md must say {want:?} — {why}"
    );
}

/// (Turkish passed, Turkish cases, English passed, English cases, failed) of
/// a selection report — a case is Turkish when its name starts `tr-`.
fn language_split(v: &Value) -> (usize, usize, usize, usize, usize) {
    let cases = v["cases"].as_array().expect("the report lists its cases");
    let (mut tp, mut tn, mut ep, mut en) = (0, 0, 0, 0);
    for c in cases {
        let passed = c["passed"].as_bool().expect("each case says if it passed");
        if c["name"].as_str().unwrap_or("").starts_with("tr-") {
            tn += 1;
            tp += passed as usize;
        } else {
            en += 1;
            ep += passed as usize;
        }
    }
    (tp, tn, ep, en, tn + en - tp - ep)
}

/// D1–D5 / RG-36, review of 10 Oct 2026. The language split and the failure
/// diagnosis under the Metal table said "counted off the baseline itself" and
/// were not: the page had "61/69 against 96/115" and "Twenty-seven of the 184
/// cases failed", the committed file holds 99/115 English and 24 failures, and
/// the diagnosis's own strings (`constraint rejected the token: 32`,
/// `find_filepattern`) are in no file in the tree. They are the baseline as
/// first committed in 3c2c3e4, which the corrected scoring replaced; the page
/// now says so, and this reads both files back.
#[test]
fn the_language_split_and_the_failure_diagnosis_are_their_files() {
    let page = readme();
    let (tp, tn, ep, en, failed) = language_split(&report("qwen3-4b-both.json"));
    require(
        &page,
        &format!("{tp}/{tn} against {ep}/{en}"),
        "the Turkish/English split of the committed Metal baseline",
    );
    require(
        &page,
        &format!("(the table's run: {failed})"),
        "the committed baseline's failure count, beside the diagnosed run's",
    );
    const DIAGNOSED: &str = "3c2c3e4";
    let paragraph = page
        .split("\n\n")
        .map(folded)
        .find(|p| p.contains("This diagnosis is of the run before the one in the table"))
        .expect("the diagnosis names the run it was counted from");
    assert!(paragraph.contains(DIAGNOSED), "{paragraph}");
    if !has_history(&repo_root()) {
        eprintln!("no git history here: the diagnosed run's counts were NOT checked");
        return;
    }
    let out = Command::new("git")
        .args([
            "show",
            &format!("{DIAGNOSED}:crates/tacet-eval/baselines/qwen3-4b-both.json"),
        ])
        .current_dir(repo_root())
        .output()
        .expect("git runs");
    assert!(out.status.success(), "{DIAGNOSED} holds no Metal baseline");
    let raw = String::from_utf8_lossy(&out.stdout);
    let old: Value = serde_json::from_str(&raw).expect("the old baseline is JSON");
    let (.., old_failed) = language_split(&old);
    const WORDS: [(usize, &str); 3] = [(24, "Twenty-four"), (27, "Twenty-seven"), (30, "Thirty")];
    let word = WORDS
        .iter()
        .find(|(n, _)| *n == old_failed)
        .map(|(_, w)| *w)
        .unwrap_or_else(|| panic!("{old_failed} failures; give the test its word"));
    assert!(
        paragraph.contains(&format!("{word} of its 184 cases failed")),
        "{DIAGNOSED}'s baseline has {old_failed} failures: {paragraph}"
    );
    for quoted in ["constraint rejected the token: 32", "find_filepattern"] {
        require(&page, quoted, "the diagnosis quotes the run it counted");
        assert!(
            raw.contains(quoted),
            "the page quotes {quoted:?} from {DIAGNOSED}'s baseline, which does not hold it"
        );
    }
}

/// The Metal block under "What it scores" is `qwen3-4b-both.json`, axis by axis.
#[test]
fn the_metal_table_is_the_metal_baseline() {
    let page = readme();
    let both = report("qwen3-4b-both.json");
    assert_eq!(
        both["identity"]["device"], "metal",
        "the file the block cites"
    );

    let tool = axis(&both, "tool");
    let irr = axis(&both, "irrelevance");
    let step = axis(&both, "step");
    let answer = axis(&both, "answer");
    require(
        &page,
        &format!(
            "| tool selection | **{}/{}** · {} |",
            tool.0,
            tool.1,
            pct(tool)
        ),
        "the Metal block's tool row",
    );
    assert_eq!(irr.0, irr.1, "the page prints the gate as 100%");
    require(
        &page,
        &format!("| irrelevance gate | **{}/{}** · 100% |", irr.0, irr.1),
        "the Metal block's gate row",
    );
    require(
        &page,
        &format!("| step chain | {}/{} · {} |", step.0, step.1, pct(step)),
        "the Metal block's step row",
    );
    require(
        &page,
        &format!(
            "| answer quality | {}/{} · {} |",
            answer.0,
            answer.1,
            pct(answer)
        ),
        "the Metal block's answer row",
    );
    let minutes = wall_ms(&both) as f64 / 60_000.0;
    require(
        &page,
        &format!("in both languages, **{minutes:.1} min**"),
        "the Metal wall clock is `wall_ms` in the file",
    );
}

/// D1: the 3090 sentence is `qwen3-4b-cuda-3090.json`'s wall clock and counts,
/// and its speed-up is the ratio of the two committed wall clocks.
#[test]
fn the_3090_wall_time_is_the_committed_reports() {
    let page = readme();
    let both = report("qwen3-4b-both.json");
    let cuda = report("qwen3-4b-cuda-3090.json");
    assert_eq!(cuda["identity"]["device"], "cuda");
    assert_eq!(
        both["identity"]["model_fingerprint"], cuda["identity"]["model_fingerprint"],
        "the page calls these the same weights"
    );

    let metal_ms = wall_ms(&both);
    let cuda_ms = wall_ms(&cuda);
    let speedup = metal_ms as f64 / cuda_ms as f64;
    require(
        &page,
        &format!(
            "**{} against Metal's {:.1} min — {:.1}×**",
            minutes_seconds(cuda_ms),
            metal_ms as f64 / 60_000.0,
            speedup
        ),
        "D1: the 3090 wall clock and speed-up come from the two committed files",
    );
    let (tool, irr, step, answer) = (
        axis(&cuda, "tool"),
        axis(&cuda, "irrelevance"),
        axis(&cuda, "step"),
        axis(&cuda, "answer"),
    );
    require(
        &page,
        &format!(
            "tool selection {}/{}, irrelevance {}/{}, step chain {}/{}, answer {}/{}.",
            tool.0, tool.1, irr.0, irr.1, step.0, step.1, answer.0, answer.1
        ),
        "D1: the 3090 run's own counts, quoted beside its wall clock",
    );
    // The model table's note quotes the same file to say its top row is NOT it.
    require(
        &page,
        &format!(
            "({}/{}, {}/{}, {}/{}, {})",
            tool.0,
            tool.1,
            step.0,
            step.1,
            answer.0,
            answer.1,
            minutes_seconds(cuda_ms)
        ),
        "the model table's note names the committed 3090 run's numbers",
    );
}

/// A duration the page writes, in seconds, with the words in front of it.
struct Duration<'a> {
    text: String,
    seconds: f64,
    before: &'a [&'a str],
}

/// Every duration in a folded paragraph: "47.7 min", "6 min", "7 minutes",
/// "7m 07s" and "7m07s".
///
/// The first version of this scan matched only "N.N min". The second review
/// added "On the RTX 3090 the run takes 6 min." to the page and the test stayed
/// green — an integer, a spelled-out unit or a minutes-and-seconds figure was
/// invisible to it, and those are the forms a hand-copied time takes.
fn durations<'a>(words: &'a [&'a str]) -> Vec<Duration<'a>> {
    let bare = |w: &str| {
        w.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.')
            .trim_end_matches('.')
            .to_string()
    };
    let m_s = |m: &str, s: &str| -> Option<f64> {
        let m: u64 = m.parse().ok()?;
        (s.len() == 2).then_some(())?;
        let s: u64 = s.parse().ok()?;
        Some((m * 60 + s) as f64)
    };
    let mut found = Vec::new();
    for i in 0..words.len() {
        let w = bare(words[i]);
        let next = words.get(i + 1).map(|n| bare(n)).unwrap_or_default();
        // "7m07s" as one word.
        if let Some((m, s)) = w.strip_suffix('s').and_then(|x| x.split_once('m')) {
            if let Some(seconds) = m_s(m, s) {
                found.push(Duration {
                    text: w.clone(),
                    seconds,
                    before: &words[..i],
                });
                continue;
            }
        }
        // "7m 07s" as two.
        if let (Some(m), Some(s)) = (w.strip_suffix('m'), next.strip_suffix('s')) {
            if let Some(seconds) = m_s(m, s) {
                found.push(Duration {
                    text: format!("{w} {next}"),
                    seconds,
                    before: &words[..i],
                });
                continue;
            }
        }
        // "47.7 min", "6 min", "7 minutes".
        let figure = w.trim_start_matches(|c: char| !c.is_ascii_digit());
        if next.starts_with("min") && !figure.is_empty() {
            if let Ok(minutes) = figure.parse::<f64>() {
                found.push(Duration {
                    text: figure.to_string(),
                    seconds: minutes * 60.0,
                    before: &words[..i],
                });
            }
        }
    }
    found
}

/// The first 3090 run's wall clock in seconds, from the report as it was
/// committed at `sha` — or None where there is no git history to read it from.
fn wall_s_at(sha: &str, file: &str) -> Option<f64> {
    if !has_history(&repo_root()) {
        return None;
    }
    let out = Command::new("git")
        .args(["show", &format!("{sha}:crates/tacet-eval/baselines/{file}")])
        .current_dir(repo_root())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    Some(wall_ms(&v) as f64 / 1000.0)
}

/// D1 (review): a 3090 wall time anywhere on the page is either a committed
/// file's, or says that no report was kept.
///
/// The first fix corrected the sentence that cites the 3090 report and left the
/// paragraph under it saying "6.4 minutes with these weights" — the hand-taken
/// model-table figure, unqualified, one paragraph from the committed 7m 07s.
/// Pinning the one sentence left every other 3090 sentence free to drift, so
/// this scans every paragraph that names the card. Allowed: the Metal report's
/// minutes (the speed-up sentence sets the two side by side), the committed
/// 3090 report's minutes-and-seconds, a figure the page says it once "quoted
/// as" (checked against the replaced report in git when there is history), and
/// the CUDA build time, which is a compile and not a run. Anything else must
/// sit in a paragraph that says "no report kept".
#[test]
fn every_3090_wall_time_is_a_committed_file_or_says_it_is_not() {
    let page = readme();
    let cuda_ms = wall_ms(&report("qwen3-4b-cuda-3090.json"));
    let cuda = minutes_seconds(cuda_ms);
    let metal = format!(
        "{:.1}",
        wall_ms(&report("qwen3-4b-both.json")) as f64 / 60_000.0
    );
    let replaced = wall_s_at("511f8db", "qwen3-4b-cuda-3090.json");
    require(
        &page,
        &format!("{cuda} with these weights"),
        "D1: the GPU paragraph quotes the committed 3090 run for these weights",
    );

    let mut scanned = 0;
    for paragraph in page.split("\n\n").map(folded) {
        if !paragraph.contains("3090") {
            continue;
        }
        // D1 (second review): "The 3090 wall times remain hand-recorded" sat
        // one paragraph above the committed 7m 07s. Only the model table's are.
        assert!(
            !paragraph.contains("wall times remain hand-recorded")
                || paragraph.contains("The model table's 3090 wall times remain hand-recorded"),
            "D1: the page says the 3090 wall times are hand-recorded without \
             saying which — the committed run's are not: {paragraph}"
        );
        let words: Vec<&str> = paragraph.split(' ').collect();
        for d in durations(&words) {
            scanned += 1;
            let tail = d.before.iter().rev().take(4).copied().collect::<Vec<_>>();
            if tail.first() == Some(&"took") && tail.iter().any(|w| w.contains("build")) {
                continue; // `cargo build --features cuda` took 2m 47s — a compile
            }
            if tail.len() >= 2 && tail[1] == "quoted" && tail[0] == "as" {
                if let Some(wall) = replaced {
                    assert!(
                        (d.seconds - wall).abs() < 1.0,
                        "D1: the page says it once quoted {} for the replaced run, \
                         and that run's report in 511f8db says {wall:.3} s: {paragraph}",
                        d.text
                    );
                } else {
                    eprintln!(
                        "no git history: the replaced run's {} was NOT checked",
                        d.text
                    );
                }
                continue;
            }
            assert!(
                d.text == metal || d.text == cuda || paragraph.contains("no report kept"),
                "D1: README quotes a 3090-paragraph wall time of {} that no committed \
                 report holds, and the paragraph does not say \"no report kept\": \
                 {paragraph}",
                d.text
            );
        }
    }
    assert!(
        scanned >= 4,
        "only {scanned} durations were found in 3090 paragraphs — the scan stopped matching"
    );
}

/// D2: the page's count of the directory is the directory, and the 3090 run's
/// whole-case count is the one the "after" paragraph sets against 155.
#[test]
fn the_baselines_directory_is_the_one_the_page_describes() {
    let page = readme();
    let files = std::fs::read_dir(baselines())
        .expect("the baselines directory")
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .count();
    const WORDS: [&str; 13] = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
        "eleven", "twelve",
    ];
    let word = WORDS
        .get(files)
        .unwrap_or_else(|| panic!("{files} reports; give the page a numeral for it"));
    require(
        &page,
        &format!("it holds {word}, re-counted"),
        "D2: the number of reports the page says `baselines/` holds",
    );

    let cuda = report("qwen3-4b-cuda-3090.json");
    let cases = cuda["cases"].as_array().expect("cases");
    let whole = cases.iter().filter(|c| c["passed"] == true).count();
    require(
        &page,
        &format!("passes {whole} of its {} cases whole", cases.len()),
        "D2: why the committed 3090 run is not the \"after\" of the +4.9 verdict",
    );
    assert_ne!(
        whole, 155,
        "the committed 3090 run passes 155 cases whole — it may BE the missing \
         \"after\" report, and the paragraph saying it is not must be rewritten"
    );
}

/// The suite's tool axis with every case that names a tool the box lacked left
/// out — what `bench run --skip-missing` did to the corpus beside it.
fn tool_axis_skipping_missing(v: &Value) -> ((usize, usize), usize) {
    let catalog: Vec<&str> = v["catalog"]
        .as_array()
        .expect("catalog")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let mut passed = 0;
    let mut total = 0;
    let mut skipped = 0;
    for case in v["cases"].as_array().expect("cases") {
        // The report's own split: an irrelevance case is never on the tool axis.
        if case["category"] == "Irrelevance" {
            continue;
        }
        let missing = case["steps"]
            .as_array()
            .expect("steps")
            .iter()
            .filter_map(|s| s["expected"].as_str())
            .any(|tool| !catalog.contains(&tool));
        if missing {
            skipped += 1;
            continue;
        }
        total += 1;
        passed += (case["passed"] == true) as usize;
    }
    ((passed, total), skipped)
}

/// D3: the corpus is compared with the suite on the same KIND of card, both
/// ways of counting a missing tool are shown, and the gap is the difference of
/// the two files' rates. "Same card" in the header read as one physical card,
/// which the paragraph under the table says neither file can show.
#[test]
fn the_turkish_corpus_is_compared_on_the_same_kind_of_card() {
    let page = readme();
    let corpus = report("tr-corpus-qwen3-4b-cuda.json");
    let suite = report("qwen3-4b-cuda-3090.json");
    assert!(
        corpus["device"]
            .as_str()
            .is_some_and(|d| d.contains("3090")),
        "the corpus report is a 3090 run"
    );
    assert_eq!(
        suite["identity"]["device"], "cuda",
        "and so is the suite beside it"
    );

    let totals = &corpus["totals"];
    let c = |name: &str| axis(totals, name);
    let s = |name: &str| axis(&suite, name);
    let ((skip_p, skip_t), skipped) = tool_axis_skipping_missing(&suite);
    assert!(
        skipped > 0,
        "the 3090 box lacked tools; the scan found none"
    );

    let header = page
        .lines()
        .find(|l| l.starts_with("| axis | Turkish corpus |"))
        .expect("the corpus table has its header row");
    assert!(
        header.contains("same kind of card (RTX 3090)") && !header.contains("same card"),
        "D3: the header must not claim one physical card: {header}"
    );

    let rows = [
        format!(
            "| tool selection | **{}/{}** · {} | {}/{} · {} — **{}/{} · {}** skipping what the box lacked |",
            c("tool").0,
            c("tool").1,
            pct(c("tool")),
            s("tool").0,
            s("tool").1,
            pct(s("tool")),
            skip_p,
            skip_t,
            pct((skip_p, skip_t))
        ),
        format!(
            "| irrelevance | **{}/{}** · {} | {}/{} · {} |",
            c("irrelevance").0,
            c("irrelevance").1,
            pct(c("irrelevance")),
            s("irrelevance").0,
            s("irrelevance").1,
            if s("irrelevance").0 == s("irrelevance").1 {
                "100%".to_string()
            } else {
                pct(s("irrelevance"))
            }
        ),
        format!(
            "| step chain | {}/{} · {} | {}/{} · {} |",
            c("step").0,
            c("step").1,
            pct(c("step")),
            s("step").0,
            s("step").1,
            pct(s("step"))
        ),
        format!(
            "| answer quality | {}/{} · {} | {}/{} · {} |",
            c("answer").0,
            c("answer").1,
            pct(c("answer")),
            s("answer").0,
            s("answer").1,
            pct(s("answer"))
        ),
    ];
    for row in &rows {
        require(
            &page,
            row,
            "D3: the corpus table row, from the two 3090 files",
        );
    }
    require(
        &page,
        &format!("the suite counts its {skipped} such cases"),
        "D3: how many suite cases name a tool the box lacked",
    );

    let rate = |(p, t): (usize, usize)| 100.0 * p as f64 / t as f64;
    let like_for_like = rate((skip_p, skip_t)) - rate(c("tool"));
    let as_scored = rate(s("tool")) - rate(c("tool"));
    require(
        &page,
        &format!("**{like_for_like:.1} points below the curated suite on tool selection"),
        "D3: the gap with missing tools skipped on both sides",
    );
    require(
        &page,
        &format!("({as_scored:.1} if the suite's"),
        "D3: the gap with the suite's missing-tool failures left in",
    );
}

/// "6 Sep 2026" from a report's "2026-09-06".
fn day_month_year(iso: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mut parts = iso.split('-');
    let y = parts.next().expect("year");
    let m: usize = parts.next().and_then(|m| m.parse().ok()).expect("month");
    let d: u32 = parts.next().and_then(|d| d.parse().ok()).expect("day");
    format!("{d} {} {y}", MONTHS[m - 1])
}

/// D2, D3 (review): the two 3090 runs are dated by what the files record, and
/// are one KIND of card, not provably one box.
///
/// Both sentences the first fix wrote claimed more than the files hold. "Made
/// later the same day" as the +4.9 verdict: the verdict entered the page on
/// 5 Sep 2026 and the runs were committed on the 6th — and no committed file
/// records the verdict's date at all, so the page may date the runs only by
/// the corpus report's own `date`. "The same box": the corpus report says its
/// box had no web tools, while the suite's catalog lists them. Neither file
/// names a machine, so the page says what differs instead.
#[test]
fn the_two_3090_runs_are_dated_by_their_files_and_not_called_one_box() {
    let page = readme();
    let corpus = report("tr-corpus-qwen3-4b-cuda.json");
    let suite = report("qwen3-4b-cuda-3090.json");
    let date = day_month_year(corpus["date"].as_str().expect("the corpus report is dated"));
    let paragraph = |opening: &str| {
        page.split("\n\n")
            .map(folded)
            .find(|p| p.starts_with(opening))
            .unwrap_or_else(|| panic!("the page has a paragraph opening {opening:?}"))
    };

    let after =
        paragraph("**And the \"after\" report behind this verdict is not in the repository.**");
    assert!(
        !after.contains("same day"),
        "D2: the page dates the 3090 runs against the verdict, whose date no committed file holds: {after}"
    );
    assert!(
        after.contains(&date),
        "D2: the 3090 runs are dated {date} by the corpus report, and the paragraph does not say so: {after}"
    );

    let beside = paragraph("The right-hand column is");
    assert!(
        !beside.contains("same box"),
        "D3: neither report names a machine, so the page cannot call them one box: {beside}"
    );
    assert!(
        beside.contains(&date),
        "D3: the suite is set beside the corpus without the corpus report's date {date}: {beside}"
    );
    let note = corpus["note"]
        .as_str()
        .expect("the corpus report has a note");
    let web: Vec<&str> = suite["catalog"]
        .as_array()
        .expect("catalog")
        .iter()
        .filter_map(Value::as_str)
        .filter(|t| t.starts_with("web_"))
        .collect();
    assert!(
        note.contains("no web tools") && !web.is_empty(),
        "the reports no longer differ in web tools (corpus note: {note:?}); the \
         sentence that says they do must be rewritten, and this test with it"
    );
    for tool in web {
        assert!(
            beside.contains(&format!("`{tool}`")),
            "D3: the suite's catalog has {tool} and the corpus box had no web \
             tools — the page must say the two runs differ there: {beside}"
        );
    }
}

/// D1 (review): the distance between the two 3090 runs is the history's, not a
/// phrase. The page said the replacement came "one commit later"; it came seven
/// commits later, and two of the six between them change what is scored — the
/// difference the paragraph goes on to stress. The commits are named on the
/// page, so the count is read back from git. A tarball without history cannot
/// count them and says so instead of passing quietly; the names are checked
/// either way.
#[test]
fn the_replaced_3090_run_is_as_many_commits_back_as_the_page_says() {
    const FIRST: &str = "511f8db";
    const REPLACEMENT: &str = "4af5484";
    let page = readme();
    let paragraph = page
        .split("\n\n")
        .map(folded)
        .find(|p| p.starts_with("That file is the second 3090 run of the day."))
        .expect("the page has the paragraph on the replaced 3090 run");
    assert!(
        !paragraph.contains("one commit later"),
        "D1: the replacement was not one commit later: {paragraph}"
    );
    for sha in [FIRST, REPLACEMENT] {
        assert!(
            paragraph.contains(sha),
            "D1: the paragraph must name {sha}: {paragraph}"
        );
    }

    if !has_history(&repo_root()) {
        eprintln!("no git history here: the commit count on the page was NOT checked");
        return;
    }
    // With the history there, a count git cannot give is a failure: the two
    // commits the page names are not in it.
    let out = Command::new("git")
        .args(["rev-list", "--count", &format!("{FIRST}..{REPLACEMENT}")])
        .current_dir(repo_root())
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "D1: git cannot count {FIRST}..{REPLACEMENT} — the page names commits this \
         history does not hold: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let n: usize = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .expect("git rev-list --count prints a number");
    const WORDS: [&str; 11] = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
    ];
    let word = WORDS
        .get(n)
        .unwrap_or_else(|| panic!("{n} commits; give the page a numeral for it"));
    assert!(
        paragraph.contains(&format!("{word} commits after the first run's {FIRST}")),
        "D1: git counts {n} commits from {FIRST} to {REPLACEMENT}, and the page \
         does not say \"{word} commits after the first run's {FIRST}\": {paragraph}"
    );
    assert!(
        n >= 1 && paragraph.contains(&format!("two of the {} between them", WORDS[n - 1])),
        "D1: the page counts the commits BETWEEN the runs wrongly: {paragraph}"
    );
}

/// The comparator block under the 3090 paragraph is what `eval --compare`
/// prints for the two committed files TODAY — the old block was the replaced
/// run's, and nothing re-ran it when the file changed.
#[test]
fn the_comparator_block_is_what_compare_prints_for_the_committed_files() {
    let scratch = std::env::temp_dir().join(format!("tacet-readme-compare-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).expect("scratch");
    // A config directory of its own: `eval --compare` needs no model and no
    // socket, and must not read the developer's ~/.tacet either.
    let out = Command::new(env!("CARGO_BIN_EXE_tacet"))
        .args(["eval", "--compare"])
        .arg(baselines().join("qwen3-4b-both.json"))
        .arg(baselines().join("qwen3-4b-cuda-3090.json"))
        .env("TACET_HOME", &scratch)
        .env("NO_COLOR", "1")
        .output()
        .expect("tacet runs");
    let _ = std::fs::remove_dir_all(&scratch);
    let printed =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "eval --compare failed:\n{printed}");

    let line = |prefix: &str| -> String {
        printed
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with(prefix))
            .unwrap_or_else(|| panic!("--compare no longer prints a {prefix:?} line:\n{printed}"))
            .to_string()
    };
    let page = readme();
    for wanted in [
        line("paired on"),
        line("delta"),
        line("sign test"),
        line("verdict:"),
    ] {
        require(
            &page,
            &wanted,
            "the comparator block, re-run on the committed files",
        );
    }
    // Anchored on the tools sentence: the device sentence before it also says
    // "so" ("…KV cache, so the two runs…").
    let excluded: String = printed
        .split("did not have the same tools, so ")
        .nth(1)
        .and_then(|rest| rest.split(" case(s)").next())
        .unwrap_or_else(|| panic!("--compare no longer counts excluded cases:\n{printed}"))
        .to_string();
    require(
        &page,
        &format!("so {excluded} case(s) … are EXCLUDED"),
        "the comparator block's excluded count",
    );
    require(
        &page,
        &format!("and {excluded} cases would have read as regressions"),
        "the sentence above the block quotes the same count",
    );
}

/// One row of the model table: (name cell, score, tool, irrelevance, step, answer).
type ModelRow = (
    String,
    f64,
    (usize, usize),
    (usize, usize),
    (usize, usize),
    (usize, usize),
);

/// "**133/160** · 83.1%" or "162/190" → (133, 160).
fn first_fraction(cell: &str) -> (usize, usize) {
    let digits: String = cell
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit() || *c == '/')
        .collect();
    let (p, t) = digits
        .split_once('/')
        .unwrap_or_else(|| panic!("no fraction in {cell:?}"));
    (
        p.parse().expect("numerator"),
        t.parse().expect("denominator"),
    )
}

/// The table under "### The model table", up to the first line that is not a
/// row. A blank line inside it ends it here exactly as it ends it on GitHub,
/// which is the point (D4).
fn model_table_lines(page: &str) -> Vec<&str> {
    let start = page
        .find("| model | score | tool selection |")
        .expect("the model table's header");
    page[start..]
        .lines()
        .skip(2) // the header and the `|---|` separator
        .take_while(|l| l.starts_with('|'))
        .collect()
}

fn model_table(page: &str) -> Vec<ModelRow> {
    model_table_lines(page)
        .into_iter()
        .map(|l| {
            let cells: Vec<&str> = l.trim_matches('|').split('|').map(str::trim).collect();
            assert_eq!(cells.len(), 7, "a model row has seven cells: {l}");
            let score: f64 = cells[1]
                .trim_matches('*')
                .parse()
                .unwrap_or_else(|_| panic!("a score in {l}"));
            (
                cells[0].to_string(),
                score,
                first_fraction(cells[2]),
                first_fraction(cells[3]),
                first_fraction(cells[4]),
                first_fraction(cells[5]),
            )
        })
        .collect()
}

/// D4 + the arithmetic of the hand-recorded rows. The rows cannot be re-run
/// here (rented card, no report kept), so what is pinned is that the table is
/// ONE table of four models and that each score is its own counts under the
/// published weights — a row edited in one cell only goes red.
#[test]
fn the_model_table_is_one_table_whose_scores_are_their_counts() {
    let rows = model_table(&readme());
    assert_eq!(
        rows.len(),
        4,
        "the page says four models; a blank line inside the table splits it and \
         GitHub renders the rest as text (D4). Rows read: {:?}",
        rows.iter().map(|r| &r.0).collect::<Vec<_>>()
    );
    for (name, score, tool, irr, step, answer) in &rows {
        let derived = BenchScore::from_counts(*tool, *irr, *step, *answer).out_of_100();
        assert_eq!(
            derived, *score,
            "{name}: the row's counts score {derived}, the row says {score}"
        );
    }
}

/// What the clone at `dir` can say about its own past.
#[derive(Debug, PartialEq)]
enum History {
    /// A repository with its whole history.
    Full,
    /// No repository at all — a source tarball, a vendored copy.
    Absent,
    /// A repository cut off at some depth, as `actions/checkout` makes by
    /// default.
    Shallow,
}

fn history_at(dir: &Path) -> History {
    match Command::new("git")
        .args(["rev-parse", "--is-shallow-repository"])
        .current_dir(dir)
        .output()
    {
        Ok(out) if out.status.success() => {
            if String::from_utf8_lossy(&out.stdout).trim() == "true" {
                History::Shallow
            } else {
                History::Full
            }
        }
        _ => History::Absent,
    }
}

/// Whether the history-reading checks can run here. A tarball skips them and
/// says so; A SHALLOW CLONE FAILS. It is not "no history": its one commit
/// holds every row, so "the commit that first held them" comes out as HEAD,
/// and the date test went red on CI's depth-1 checkout on all three
/// platforms while the commit-count test skipped silently (review of
/// 10 Oct 2026). CI's test job fetches the whole history now; a shallow clone
/// anywhere else is told to do the same.
fn has_history(dir: &Path) -> bool {
    match history_at(dir) {
        History::Full => true,
        History::Absent => false,
        History::Shallow => panic!(
            "this clone is shallow, so the README's dates and commit counts cannot be \
             read from it: run `git fetch --unshallow` (CI: `fetch-depth: 0`)"
        ),
    }
}

/// The commit that first put `line` into README.md in the repository at
/// `dir`, as (full sha, "YYYY-MM-DD" in the author's zone) — or None when no
/// commit holds it. Call `has_history` first: in a shallow clone this
/// answers HEAD for every line, and with no repository it answers None.
fn first_commit_holding_at(dir: &Path, line: &str) -> Option<(String, String)> {
    let out = Command::new("git")
        .args([
            "log",
            "--reverse",
            "--format=%H %ad",
            "--date=short",
            "-S",
            line,
            "--",
            "README.md",
        ])
        .current_dir(dir)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let first = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()?
        .to_string();
    let (sha, date) = first.split_once(' ')?;
    Some((sha.to_string(), date.to_string()))
}

/// The review's two holes, on a two-commit repository built here: a depth-1
/// clone of it is told apart from a full one (where it had dated every row by
/// HEAD), and a line no commit holds is `None` in a history that is there —
/// which `the_model_table_is_dated_by_the_commit_that_first_held_its_rows`
/// now fails on instead of printing "no git history here".
#[test]
fn a_shallow_clone_is_refused_and_an_uncommitted_row_is_not_a_missing_history() {
    let base = std::env::temp_dir().join(format!("tacet-readme-history-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let full = base.join("full");
    std::fs::create_dir_all(&full).unwrap();
    let git = |dir: &Path, args: &[&str]| {
        let out = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&full, &["init", "-q"]);
    std::fs::write(full.join("README.md"), "| row one |\n").unwrap();
    git(&full, &["add", "README.md"]);
    git(
        &full,
        &["commit", "-q", "-m", "one", "--date=2026-09-05T12:00:00"],
    );
    std::fs::write(full.join("README.md"), "| row one |\n| row two |\n").unwrap();
    git(&full, &["commit", "-q", "-am", "two"]);
    let url = format!("file://{}", full.display());
    git(&base, &["clone", "-q", "--depth", "1", &url, "shallow"]);

    assert_eq!(history_at(&full), History::Full);
    assert_eq!(history_at(&base.join("shallow")), History::Shallow);
    assert!(
        std::panic::catch_unwind(|| has_history(&base.join("shallow"))).is_err(),
        "a shallow clone was measured"
    );
    let first = first_commit_holding_at(&full, "| row one |").expect("committed");
    let head = first_commit_holding_at(&full, "| row two |").expect("committed");
    assert_ne!(
        first.0, head.0,
        "row one is dated by the commit that added it"
    );
    assert_eq!(first_commit_holding_at(&full, "| never committed |"), None);
    let _ = std::fs::remove_dir_all(&base);
}

/// D4 (review): the model table's rows are dated by the commit that first held
/// them, not by memory.
///
/// The first fix dated the four rows "5–6 Sep 2026". All four first appear in
/// ae0893a, committed 11:46 on 5 Sep, and the rental's own commits end at 01:35
/// that morning — so the 6th is a day after the rows existed, a false
/// measurement date on the one table that has no report to correct it against.
/// No file records when a hand-taken row was measured; the history records the
/// latest it can have been, so that is what the page may say: "on or before"
/// the day the last of the four rows was committed, naming that commit. The
/// CHANGELOG entry that describes the fix says the same.
#[test]
fn the_model_table_is_dated_by_the_commit_that_first_held_its_rows() {
    let page = readme();
    let lines = model_table_lines(&page);
    assert_eq!(lines.len(), 4, "the model table has four rows");
    if !has_history(&repo_root()) {
        eprintln!("no git history here: the model table's date was NOT checked");
        return;
    }
    let mut firsts = Vec::new();
    for line in &lines {
        // NOT A SKIP (review of 10 Oct 2026). With the history there, a row
        // no commit holds is an edit to a hand-recorded measurement — which
        // is exactly when the pre-commit gate runs, and it used to print "no
        // git history here" and pass. Nobody can re-run these rows (rented
        // card, no report kept); a changed one is a new claim with no date.
        let first = first_commit_holding_at(&repo_root(), line).unwrap_or_else(|| {
            panic!(
                "D4: no commit holds the model-table row {line:?} — a hand-recorded row \
                 was edited; restore it, or replace the table with a committed report"
            )
        });
        firsts.push(first);
    }
    // ISO dates sort as strings; the latest is the earliest day all four existed.
    let (sha, date) = firsts
        .iter()
        .max_by(|a, b| a.1.cmp(&b.1))
        .expect("four rows")
        .clone();
    let day = day_month_year(&date);
    let note = page
        .split("\n\n")
        .map(folded)
        .find(|p| p.starts_with("**These four rows are hand-recorded"))
        .expect("the note under the model table");
    assert!(
        note.contains(&format!("on or before {day}")),
        "D4: the last of the model table's rows was first committed on {day} \
         ({sha}), so the page may date them \"on or before {day}\" and no later: {note}"
    );
    let cited = note
        .split("first committed in ")
        .nth(1)
        .and_then(|rest| rest.split(|c: char| !c.is_ascii_hexdigit()).next())
        .unwrap_or("");
    assert!(
        cited.len() >= 7 && sha.starts_with(cited),
        "D4: the note must name the commit that first held the rows, {sha}; it names \
         {cited:?}: {note}"
    );
    let changelog =
        folded(&std::fs::read_to_string(repo_root().join("CHANGELOG.md")).expect("CHANGELOG.md"));
    assert!(
        changelog.contains(&format!("hand-recorded on or before {day}")),
        "D4: the CHANGELOG entry for the fix dates the rows differently from the history, \
         which says {day} ({sha})"
    );
}

/// D5: the seven-language table's `en` column is the file the paragraph under
/// it names, and the paragraph still names it.
#[test]
fn the_seven_language_en_column_is_the_file_it_cites() {
    let page = readme();
    let section = page
        .split_once("### Seven languages")
        .expect("the seven-language section")
        .1;
    require(
        section,
        "(`crates/tacet-eval/baselines/qwen3-4b-both.json`)",
        "the `en` column's citation",
    );
    let both = report("qwen3-4b-both.json");
    let en_cell = |label: &str| -> String {
        let row = section
            .lines()
            .find(|l| l.starts_with(&format!("| {label} |")))
            .unwrap_or_else(|| panic!("the seven-language table has a {label} row"));
        row.trim_matches('|')
            .split('|')
            .map(str::trim)
            .nth(1)
            .expect("an en cell")
            .trim_matches('*')
            .to_string()
    };
    let (tool, irr, step, answer) = (
        axis(&both, "tool"),
        axis(&both, "irrelevance"),
        axis(&both, "step"),
        axis(&both, "answer"),
    );
    assert_eq!(
        en_cell("tool"),
        format!("{}/{}", tool.0, tool.1),
        "D5: tool"
    );
    assert_eq!(
        en_cell("irrelevance"),
        format!("{}/{}", irr.0, irr.1),
        "D5: irrelevance"
    );
    let score = BenchScore::from_counts(tool, irr, step, answer).out_of_100();
    assert_eq!(en_cell("score"), format!("{score:.1}"), "D5: score");
}

/// The BFCL table is the three BFCL reports, and so are the translation counts
/// in the bullet list under it.
#[test]
fn the_bfcl_table_is_the_bfcl_reports() {
    let page = readme();
    let sets = [
        ("irrelevance", "bfcl-irrelevance-qwen3-4b-metal.json"),
        (
            "live_irrelevance",
            "bfcl-live-irrelevance-qwen3-4b-metal.json",
        ),
        ("live_relevance", "bfcl-live-relevance-qwen3-4b-metal.json"),
    ];
    let mut renamed = 0;
    let mut untranslatable = 0;
    for (category, file) in sets {
        let r = report(file);
        let scored = count(&r, "cases_scored");
        let passed = count(&r, "passed");
        let wall = r["wall_s"].as_u64().expect("wall_s");
        let expected = if r["must_call"] == true {
            "call **something**"
        } else {
            "call nothing"
        };
        require(
            &page,
            &format!(
                "| `{category}` | {scored} | {expected} | **{passed}** · {} | {} |",
                pct((passed, scored)),
                minutes_seconds(wall * 1000)
            ),
            "the BFCL row, from its report",
        );
        renamed += count(&r, "tool_names_rewritten");
        untranslatable += count(&r, "schemas_untranslatable");
    }
    require(
        &page,
        &format!("**{renamed} function names had to be rewritten**"),
        "the sum of `tool_names_rewritten` over the three reports",
    );
    require(
        &page,
        &format!("**{untranslatable} schemas could not be expressed at all**"),
        "the sum of `schemas_untranslatable` over the three reports",
    );
    let live = report("bfcl-live-irrelevance-qwen3-4b-metal.json");
    let curated = report("bfcl-irrelevance-qwen3-4b-metal.json");
    let unscored = |r: &Value| count(r, "entries_in_file") - count(r, "cases_scored");
    require(
        &page,
        &format!(
            "({} of {} there, {} of {} in the curated set)",
            unscored(&live),
            count(&live, "entries_in_file"),
            unscored(&curated),
            count(&curated, "entries_in_file")
        ),
        "the unscored BFCL cases, from the reports",
    );
}

/// The exchange-rate sentence the page must carry, derived from the model
/// table's top row and the published weights (audit D9).
///
/// The row's denominators ARE the suite's proportions (160 tool, 24
/// irrelevance, 190 step, 47 answer — `bench.rs` pins that they are the live
/// suite's), so the cost of one gate failure, the earnings of one tool case
/// with its step, how many of those buy the failure back, and the before and
/// after scores all come from numbers the page itself prints.
fn exchange_rate_sentence(top: &ModelRow) -> String {
    use tacet_eval::bench::WEIGHTS;
    let weight = |axis: &str| {
        WEIGHTS
            .iter()
            .find(|(name, _)| *name == axis)
            .unwrap_or_else(|| panic!("a weight for {axis}"))
            .1
    };
    let (_, _, tool, irr, step, answer) = *top;
    let cost = 100.0 * weight("irrelevance") / irr.1 as f64;
    let earn = 100.0 * (weight("tool") / tool.1 as f64 + weight("step") / step.1 as f64);
    let clean = BenchScore::from_counts(tool, irr, step, answer);
    let row = |k: usize| {
        BenchScore::from_counts(
            (tool.0 + k, tool.1),
            (irr.0 - 1, irr.1),
            (step.0 + k, step.1),
            answer,
        )
    };
    let k = (1..=tool.1 - tool.0)
        .find(|&k| row(k).composite > clean.composite)
        .expect("some number of tool cases buys back one gate failure");
    let word = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
    ]
    .get(k)
    .copied()
    .map(str::to_string)
    .unwrap_or_else(|| k.to_string());
    format!(
        "one irrelevance failure costs {cost:.2} points and a tool case landed with its step \
         earns {earn:.2}, so it takes {word} of them to buy one back ({} -> {} on the top row \
         of the model table)",
        clean.out_of_100(),
        row(k).out_of_100()
    )
}

/// What the page must not say any more: that the 0.40 weight stops a tool gain
/// from buying a loss in restraint. D9 measured the opposite, and the 135M
/// row beside the claim showed it — composite up, irrelevance down.
fn claims_restraint_cannot_be_bought(page: &str) -> bool {
    let flat = folded(page);
    flat.contains("is not allowed to quietly buy a loss in restraint")
        || flat.contains("cannot be bought back")
}

/// The 135M paragraph as the completeness review read it (README:1262-1264).
const AUDITED_135M: &str = "\
reach a tool now do. This is why the composite weights irrelevance at 0.40: a
gain in tool accuracy is not allowed to quietly buy a loss in restraint, and it
is why 51 of 119 tools found still only moves the composite from 46.4 to 59.9.";

#[test]
fn the_audited_135m_claim_is_refused() {
    assert!(claims_restraint_cannot_be_bought(AUDITED_135M));
}

/// D9, ON THE PAGE. `bench.rs` pins the exchange rate in the code; this pins
/// the sentence quoting it, so a weight, a denominator or the top row changed
/// alone goes red with the sentence it now expects. And the old claim — that
/// the weight forbids the trade — may not come back anywhere on the page.
#[test]
fn the_exchange_rate_on_the_page_is_the_weights() {
    let page = readme();
    let rows = model_table(&page);
    let top = rows.first().expect("the model table has a top row");
    require(
        &page,
        &exchange_rate_sentence(top),
        "the D9 exchange rate, from the top row's counts and `WEIGHTS`",
    );
    require(
        &page,
        "Two tests pin that exchange rate: `bench.rs` holds the arithmetic in both \
         directions, and `the_readme_quotes_the_baselines` derives this sentence",
        "the page names the tests that pin it — bench.rs and this one",
    );
    assert!(
        !claims_restraint_cannot_be_bought(&page),
        "README.md says the irrelevance weight stops a tool gain from buying a loss in \
         restraint; D9 measured six tool cases buying one back"
    );
}
