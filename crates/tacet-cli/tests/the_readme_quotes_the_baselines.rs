//! EVERY NUMBER THE README QUOTES FROM A COMMITTED REPORT IS READ BACK FROM IT.
//!
//! WHY THIS FILE EXISTS (audit D1–D5, 9 Oct 2026). Five paragraphs disagreed
//! with the files they cited, and each disagreement had the same shape: a
//! report in `crates/tacet-eval/baselines/` was replaced or added, and the
//! prose that quoted it was not re-derived.
//!
//! * `qwen3-4b-cuda-3090.json` was replaced by a run one commit later; the page
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

/// D3: the corpus is compared with the suite on THE SAME CARD, both ways of
/// counting a missing tool are shown, and the gap is the difference of the two
/// files' rates.
#[test]
fn the_turkish_corpus_is_compared_on_the_same_card() {
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
fn model_table(page: &str) -> Vec<ModelRow> {
    let start = page
        .find("| model | score | tool selection |")
        .expect("the model table's header");
    page[start..]
        .lines()
        .skip(2) // the header and the `|---|` separator
        .take_while(|l| l.starts_with('|'))
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
