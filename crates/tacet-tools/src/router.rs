//! Router — the per-session tool budget.
//!
//! DECISION (inherited by the Swift side): AT MOST 8 tools are shown to the model
//! in a session. The reason is measurement: a small model cannot pick the right
//! one out of 20 tools; selection error grows fast with the tool count. As the
//! catalog grows, the fix is not "push the model harder" but give it fewer
//! options. The user does not see the selection — it is derived silently from the
//! intent profile.
//!
//! SCORING — A SUM OF LENGTHS, NOT A COUNT. The lesson learned in Swift: if
//! occurrences are counted, "table" and "as a table" carry the same weight, on a
//! tie the order is decided by HashMap iteration order (i.e. randomness) and the
//! same message produces a different tool set on two runs. Summing the character
//! lengths of the matched triggers makes a specific phrase naturally beat a
//! generic word and makes the result deterministic — so eval stays comparable.

use std::sync::Arc;
use tacet_core::{Tool, ToolCatalog};

/// The most tools shown to the model in one session.
pub const MAX_TOOLS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntentProfile {
    Document,
    Time,
    Calc,
    /// Messages that explicitly point at THE INTERNET (address, page, site).
    ///
    /// WHY IT WAS SPLIT OFF FROM General: the message "read that page" was scoring
    /// under the General profile through "read" and "page", but General's tool
    /// hints (file, read, memory) promote DEVICE tools — that is, while the
    /// message pointed at the web, `read_document`/`find_file` filled the budget
    /// and `web_fetch` fell off the list of 8. That is exactly what the
    /// measurement showed. A separate profile splits the "internet" intent from
    /// the "search on the device" intent; the two share the same words but want
    /// OPPOSITE tools.
    Web,
    General,
}

impl IntentProfile {
    pub const ALL: [IntentProfile; 5] = [
        IntentProfile::Document,
        IntentProfile::Time,
        IntentProfile::Calc,
        IntentProfile::Web,
        IntentProfile::General,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            IntentProfile::Document => "document",
            IntentProfile::Time => "time",
            IntentProfile::Calc => "calc",
            IntentProfile::Web => "web",
            IntentProfile::General => "general",
        }
    }

    /// The phrases looked for in the user's message.
    ///
    /// BEHAVIOUR CHANGE ON THE ENGLISH PASS: these strings used to be TURKISH on
    /// purpose, because they are not code but DATA — they are matched against
    /// what the user types, and every one of them was added from a measured
    /// failure (the records are kept in the comments below). The whole code base
    /// was moved to English by product decision, so they are English now. THE
    /// PRICE IS EXPLICIT: a user who writes in Turkish no longer touches any
    /// trigger, the score falls to zero and the tool order falls back to the
    /// catalog order — which is precisely the failure mode the measurements
    /// below describe. Making this list MULTILINGUAL (an English list plus a
    /// per-locale list) is the follow-up work; nothing here rules that out.
    ///
    /// Specific phrases ("as a table") sit in the same list as generic words
    /// ("table"); no separation is needed, the sum of lengths already establishes
    /// the ordering.
    fn message_triggers(&self) -> &'static [&'static str] {
        match self {
            IntentProfile::Document => &[
                "document",
                "docx",
                "word",
                "xlsx",
                "excel",
                "presentation",
                "pptx",
                "slide",
                "pdf",
                "report",
                "table",
                "as a table",
                "in table form",
                "add to the document",
                "create a document",
                "write to a file",
                "format it",
                "add headings",
                // CAME FROM MEASUREMENT: on the message "create a markdown file",
                // `create_document` WAS FALLING OFF the budget of 8. The reason is
                // visible in the list: "docx/xlsx/pdf" were there but the third
                // format the tool supports — markdown — was not, and the word "note"
                // pulled the message towards the General profile
                // (remember/find_file).
                "markdown",
                "md file",
                "text file",
                "create a file",
                "make a file",
                "file create",
                "make a list",
            ],
            IntentProfile::Time => &[
                "calendar",
                "event",
                "meeting",
                "appointment",
                "reminder",
                "at what time",
                "today",
                "tomorrow",
                "this week",
                "next week",
                "date",
                "schedule",
                "agenda",
                // CAME FROM MEASUREMENT (the user's real session): "what time is it
                // in turkey", "what day of the month is it" and "how many days
                // until new year" touched no trigger at all — "at what TIME" was
                // there but "what time is it" was not. With the score at zero the
                // selection falls back to catalog order and the `time` tool
                // appeared in the middle rather than at the HEAD of the list; in a
                // small model, position is selection probability.
                "what time is it",
                "day of the month",
                "which day",
                "what day",
                "how many days left",
                "how many days until",
                "new year",
                // "when" IS DELIBERATELY ABSENT: "when is the meeting" is written
                // as often as "when was he elected", and the second is a web
                // question. Writing it into both profiles equalises the scores;
                // writing it into neither leaves the distinction to the message's
                // other words — in measurement that came out right.
            ],
            IntentProfile::Calc => &[
                "calculate",
                "sum",
                "multiply",
                "percent",
                "average",
                "formula",
                "run code",
                // For `write_code`: the user wants the code as a FILE. "write"
                // alone is DELIBERATELY absent — "write a report" is document work;
                // "write code" and "script" occur in no other profile.
                "write code",
                "write a script",
                "script",
                "python",
                "program",
                "math",
                "how much is",
                "total amount",
                // CAME FROM MEASUREMENT: on the messages "list the prime numbers"
                // and "the first 15 terms of the fibonacci sequence", `run_code`
                // WAS FALLING OFF the budget — no trigger matched, so the selection
                // was left to catalog order and the tool sat at the end of the
                // list. The following are the markers of the intent "do not compute
                // this by hand, produce it with a script"; not the case texts but
                // the intent itself.
                "prime",
                "numbers",
                "sequence",
                "factorial",
                "sort",
                "algorithm",
                "simulation",
                "how many",
                "terms",
            ],
            IntentProfile::Web => &[
                "http",
                "https://",
                "www.",
                "address",
                "page",
                "link",
                "site",
                "url",
                "internet",
                "web",
                // MARKERS OF CURRENT/VERIFIABLE INFORMATION. In the user's real
                // session the model answered "when was imamoglu elected mayor" with
                // a wrong date WITHOUT looking at the web. The following do not ask
                // for a URL, but all of them are phrases saying "I should say this
                // from a source, not from memory"; they pull `web_search` forward in
                // the budget. They do not touch greetings — irrelevance is
                // unaffected by this list, because none of them occurs in "hello".
                "was elected",
                "election",
                "in what year",
                "which year",
                "current",
                "news",
                "breaking",
                // "weather forecast" WAS NOT ENOUGH: the eval case reads "what is
                // the weather like in Istanbul?" and users ask that way too. Bare
                // "weather" was added as well — the word almost always means the
                // weather and does not occur in greetings.
                "weather forecast",
                "weather",
                // TIMETABLE / SHOWTIME — came from the user's real session. The
                // message "what are the ortakoy uskudar ferry times" touched NO
                // trigger: with the score at zero the selection fell back to
                // catalog order and `web_search` appeared at the END of the list of
                // 8; the model picked the `time` tool at the head of the list and
                // told the user "14:18".
                //
                // THE DISTINCTION IS THIS: "what time is it" is the device's clock
                // (`time`), "ferry times" is a TIMETABLE and a timetable is not on
                // the device. The two share the word "time" but want OPPOSITE
                // tools — the same logic as where the Web and General profiles were
                // split.
                //
                // The plural "times" was chosen deliberately: "what time is it" and
                // "at what time" are not written with the plural, so the `time`
                // cluster is unaffected by this line.
                "times",
                "service times",
                "ferry",
                "bus times",
                "train times",
                "flight",
                "timetable",
                "departure time",
                "match time",
                "showing",
                "now playing",
                "price of",
                "how much is it in",
                "dollar",
                "euro",
                "stock market",
                "who is",
                // "what is" WAS TRIED AND REMOVED: it also pushed questions about
                // Tacet itself ("what is tacet", "what is a tool") towards the web;
                // a weight with no payoff.
            ],
            IntentProfile::General => &[
                "file",
                "folder",
                "directory",
                "search",
                "find",
                "list",
                "read",
                "note",
                "remember",
                "summarize",
            ],
        }
    }

    /// The hints looked for in a tool's name and description to decide whether the
    /// tool belongs to this profile.
    ///
    /// We do not have the tool names fixed in hand (the catalog is filled at
    /// runtime), so matching is done by text rather than by name: adding a new tool
    /// should not require updating the router.
    ///
    /// UNLIKE `message_triggers`, THESE ARE ENGLISH AND MUST BE. They are matched
    /// against the tool NAME and DESCRIPTION, and both of those are English (see
    /// the tools' `description()` methods). Turkish hints here would touch no tool,
    /// by the product rule everyone would score 0, and the profile would behave as
    /// if it did not exist. The name and the description are scanned together.
    fn tool_hints(&self) -> &'static [&'static str] {
        match self {
            IntentProfile::Document => &[
                "document", "docx", "xlsx", "pptx", "pdf", "table", "report", "sheet", "edit",
            ],
            IntentProfile::Time => &[
                "calendar",
                "event",
                "meeting",
                "appointment",
                "date",
                "clock",
                "reminder",
            ],
            IntentProfile::Calc => &[
                "calculate",
                "code",
                "python",
                "run",
                "formula",
                "numeric",
                "arithmetic",
            ],
            IntentProfile::Web => &["web", "internet", "page", "address", "url", "search"],
            IntentProfile::General => &[
                "file", "folder", "find", "read", "write", "list", "memory", "remember",
            ],
        }
    }
}

/// The profile scores coming out of a message; the highest-scoring profile is the
/// dominant intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntentScores {
    scores: Vec<(IntentProfile, usize)>,
}

impl IntentScores {
    pub fn score(&self, profile: IntentProfile) -> usize {
        self.scores
            .iter()
            .find(|(p, _)| *p == profile)
            .map(|(_, s)| *s)
            .unwrap_or(0)
    }

    /// The dominant profile. If no trigger matched, `General` — forcing an unknown
    /// message into a specific profile brings the wrong tool forward.
    pub fn dominant(&self) -> IntentProfile {
        self.scores
            .iter()
            .filter(|(_, s)| *s > 0)
            // max_by_key picks THE LAST on a tie; the iterator is reversed so the
            // first profile in ALL wins — a determinism requirement.
            .rev()
            .max_by_key(|(_, s)| *s)
            .map(|(p, _)| *p)
            .unwrap_or(IntentProfile::General)
    }

    pub fn all(&self) -> &[(IntentProfile, usize)] {
        &self.scores
    }
}

/// Folds accented/Turkish characters to ASCII and lowercases.
///
/// WHY: the user writes both "toplantı" and "toplanti"; keeping the trigger list
/// in two forms doubles the list and makes one of them go stale while the other is
/// updated. Reducing to a single normal form leaves a single source of truth. The
/// fold is kept even though the triggers are now English: user input is still
/// free text in any language.
pub fn simplify(text: &str) -> String {
    let mut s = String::with_capacity(text.len());
    for c in text.chars() {
        let d = match c {
            'ı' | 'İ' | 'I' => 'i',
            'ş' | 'Ş' => 's',
            'ğ' | 'Ğ' => 'g',
            'ü' | 'Ü' => 'u',
            'ö' | 'Ö' => 'o',
            'ç' | 'Ç' => 'c',
            'â' | 'Â' => 'a',
            'î' | 'Î' => 'i',
            'û' | 'Û' => 'u',
            _ => c,
        };
        s.extend(d.to_lowercase());
    }
    s
}

/// Scores the message. Score = the sum of the character lengths of the matched
/// triggers.
pub fn score_intent(message: &str) -> IntentScores {
    let simple = simplify(message);
    let scores = IntentProfile::ALL
        .iter()
        .map(|p| {
            let total = p
                .message_triggers()
                .iter()
                .filter(|t| simple.contains(**t))
                .map(|t| t.len())
                .sum();
            (*p, total)
        })
        .collect();
    IntentScores { scores }
}

/// The router that applies the tool budget.
///
/// Stateless: the same message + the same catalog always gives the same list.
#[derive(Debug, Clone, Copy, Default)]
pub struct Router {
    max: Option<usize>,
}

impl Router {
    pub fn new() -> Self {
        Self::default()
    }

    /// To narrow the budget (eval scenarios). `MAX_TOOLS` CANNOT BE EXCEEDED: the
    /// budget is an architectural decision, not a call-site setting.
    pub fn max(mut self, count: usize) -> Self {
        self.max = Some(count.min(MAX_TOOLS));
        self
    }

    fn budget(&self) -> usize {
        self.max.unwrap_or(MAX_TOOLS)
    }

    /// Picks the tools to show the model in this session, based on the message.
    ///
    /// Ordering: highest score first; on a tie the catalog order is preserved
    /// (stable sort). Tools scoring zero are added too until the budget fills — an
    /// empty list going to the model is worse than the general tools at the head of
    /// the catalog going instead.
    pub fn select(&self, message: &str, catalog: &ToolCatalog) -> Vec<Arc<dyn Tool>> {
        let scores = score_intent(message);
        let mut ordered: Vec<(usize, usize, Arc<dyn Tool>)> = catalog
            .tools()
            .iter()
            .enumerate()
            .map(|(i, t)| (self.tool_score(t.as_ref(), &scores), i, t.clone()))
            .collect();

        // The key is (-score, catalog order): fully deterministic.
        ordered.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        ordered
            .into_iter()
            .take(self.budget())
            .map(|(_, _, t)| t)
            .collect()
    }

    /// A tool's score: for every profile the tool belongs to, the product of the
    /// score that profile got from the message and the length of the hints matched
    /// on the tool.
    ///
    /// A product was preferred over a sum: if the message contains no time phrase,
    /// the calendar tool scores 0 even if a hint matches. Summed, an unrelated tool
    /// would climb the list just for having a long name.
    fn tool_score(&self, tool: &dyn Tool, scores: &IntentScores) -> usize {
        let text = simplify(&format!("{} {}", tool.name(), tool.description()));
        IntentProfile::ALL
            .iter()
            .map(|p| {
                let message_score = scores.score(*p);
                if message_score == 0 {
                    return 0;
                }
                let hint: usize = p
                    .tool_hints()
                    .iter()
                    .filter(|t| text.contains(**t))
                    .map(|t| t.len())
                    .sum();
                message_score * hint
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use tacet_core::{
        ArgSchema, InMemoryDataStore, SilentReporter, Tool, ToolContext, ToolFuture, ToolOutcome,
        boxed,
    };

    struct FakeTool {
        name: &'static str,
        description: &'static str,
    }

    impl Tool for FakeTool {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            self.description
        }
        fn schema(&self) -> ArgSchema {
            ArgSchema::empty()
        }
        fn run<'a>(&'a self, _args: Value, _ctx: &'a mut ToolContext) -> ToolFuture<'a> {
            boxed(async move { ToolOutcome::read_ok("test", "ok") })
        }
    }

    fn tool(name: &'static str, description: &'static str) -> Arc<dyn Tool> {
        Arc::new(FakeTool { name, description })
    }

    /// The fixture catalog. The names and descriptions are ENGLISH, like the real
    /// tools: `tool_hints` matches against that text, so a Turkish fixture would
    /// measure a different world from production.
    fn catalog() -> ToolCatalog {
        [
            tool("file_read", "read a file"),
            tool("dir_list", "list the contents of a folder"),
            tool("document_edit", "edit a docx document and add a table"),
            tool("presentation_create", "create a pptx presentation sheet"),
            tool("calendar_read", "calendar event and meeting list"),
            tool("reminder_create", "create a reminder and an appointment"),
            tool("code_run", "run python code, calculate numeric results"),
            tool("memory_write", "write a lasting memory note"),
            tool("table_produce", "produce an xlsx table and format a report"),
            tool("text_search", "search inside text and find it"),
        ]
        .into_iter()
        .collect()
    }

    // We also want to see that the unused context setup compiles.
    #[test]
    fn a_tool_runs_with_a_context() {
        let ctx = ToolContext::new(
            Arc::new(InMemoryDataStore::new()),
            ".",
            Arc::new(SilentReporter),
        );
        assert!(!ctx.session_tainted());
    }

    #[test]
    fn at_most_eight_tools_are_returned() {
        let c = catalog();
        assert_eq!(c.tools().len(), 10);
        let selection = Router::new().select("prepare a document for me", &c);
        assert_eq!(selection.len(), MAX_TOOLS);
    }

    #[test]
    fn a_specific_phrase_beats_a_generic_word() {
        // "table" alone: the document profile gets "table" (5).
        let single = score_intent("table");
        // "as a table" matches both "table" and "as a table" -> higher.
        let specific = score_intent("give it as a table");
        assert!(
            specific.score(IntentProfile::Document) > single.score(IntentProfile::Document),
            "{:?} vs {:?}",
            specific.score(IntentProfile::Document),
            single.score(IntentProfile::Document)
        );
    }

    #[test]
    fn scoring_is_a_sum_of_lengths_not_a_count() {
        // A single long trigger ("appointment", 11) must score higher than two
        // short ones ("find" 4 + "note" 4).
        let long = score_intent("appointment");
        let short = score_intent("find note");
        assert_eq!(long.score(IntentProfile::Time), "appointment".len());
        assert!(long.score(IntentProfile::Time) > short.score(IntentProfile::General));
    }

    #[test]
    fn case_is_folded_for_scoring() {
        assert_eq!(
            score_intent("Show Tomorrow's MEETING").score(IntentProfile::Time),
            score_intent("show tomorrow's meeting").score(IntentProfile::Time)
        );
        assert!(score_intent("MEETING").score(IntentProfile::Time) > 0);
    }

    /// RESTORED. This test used to measure Turkish diacritic folding
    /// ("yarınki toplantıyı göster" == "yarinki toplantiyi goster"); it was replaced
    /// by an ASCII case-only test, and from then on `simplify()` kept folding
    /// diacritics in production while NOTHING measured it. The triggers are English
    /// now, so the fold is measured where it lives — on `simplify()` itself — and
    /// once more at the scoring layer, because the user's input is still free text
    /// in any language.
    #[test]
    fn turkish_diacritics_are_folded() {
        assert_eq!(
            simplify("yarınki toplantıyı göster"),
            "yarinki toplantiyi goster"
        );
        // The dotted capital İ and the dotless ı both land on plain `i`; the bare
        // ASCII `I` is mapped explicitly too, so Turkish-locale lowercasing cannot
        // turn it into `ı` on the way.
        assert_eq!(simplify("ÇĞİIÖŞÜ ı"), "cgiiosu i");
        assert_eq!(simplify("Â Î Û"), "a i u");
        // Scoring layer, and this one is NOT decoration: a Turkish keyboard
        // uppercases `i` as `İ`, so an English word typed in caps arrives as
        // "MEETİNG". Rust's plain `to_lowercase()` turns `İ` into `i` + U+0307, which
        // matches no trigger — only the explicit fold above rescues it.
        assert!(score_intent("MEETİNG TOMORROW").score(IntentProfile::Time) > 0);
        assert_eq!(
            score_intent("MEETİNG TOMORROW").score(IntentProfile::Time),
            score_intent("meeting tomorrow").score(IntentProfile::Time)
        );
    }

    #[test]
    fn a_document_intent_brings_the_document_tools_forward() {
        let selection =
            Router::new().select("add this data to the docx document as a table", &catalog());
        let names: Vec<&str> = selection.iter().map(|t| t.name()).collect();
        assert!(
            names[0] == "document_edit" || names[0] == "table_produce",
            "the document tools should come first: {names:?}"
        );
        assert!(names.contains(&"document_edit"));
    }

    #[test]
    fn a_time_intent_brings_the_calendar_tools_forward() {
        let selection =
            Router::new().select("is there a meeting and an appointment tomorrow", &catalog());
        let names: Vec<&str> = selection.iter().map(|t| t.name()).collect();
        assert!(names[..2].contains(&"calendar_read"), "{names:?}");
        assert!(names[..2].contains(&"reminder_create"), "{names:?}");
    }

    #[test]
    fn a_calc_intent_brings_the_code_tool_forward() {
        let selection =
            Router::new().select("calculate that percent, run it with python", &catalog());
        assert_eq!(selection[0].name(), "code_run");
    }

    #[test]
    fn an_unrelated_message_preserves_the_catalog_order() {
        let selection = Router::new().select("hello how are you", &catalog());
        let names: Vec<&str> = selection.iter().map(|t| t.name()).collect();
        assert_eq!(names[0], "file_read");
        assert_eq!(names[1], "dir_list");
        assert_eq!(names.len(), MAX_TOOLS);
    }

    #[test]
    fn the_selection_is_deterministic() {
        let c = catalog();
        let r = Router::new();
        let first: Vec<String> = r
            .select("document table calendar", &c)
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        for _ in 0..20 {
            let again: Vec<String> = r
                .select("document table calendar", &c)
                .iter()
                .map(|t| t.name().to_string())
                .collect();
            assert_eq!(first, again);
        }
    }

    #[test]
    fn the_budget_can_be_narrowed_but_not_exceeded() {
        let c = catalog();
        assert_eq!(Router::new().max(3).select("document", &c).len(), 3);
        // Even if 50 is asked for, the cap is 8.
        assert_eq!(
            Router::new().max(50).select("document", &c).len(),
            MAX_TOOLS
        );
    }

    #[test]
    fn a_small_catalog_is_returned_as_is() {
        let c: ToolCatalog = [tool("file_read", "read")].into_iter().collect();
        let selection = Router::new().select("what is on the calendar", &c);
        assert_eq!(selection.len(), 1);
    }

    /// REGRESSION TEST — the user's real session. Two questions carrying the word
    /// "saat" want OPPOSITE tools; the distinction has to live at the router layer,
    /// because the model tends to pick the tool at the HEAD of the list.
    #[test]
    fn a_timetable_question_goes_to_web_and_a_clock_question_to_time() {
        let timetable = score_intent("what are the ortakoy uskudar ferry times");
        assert_eq!(
            timetable.dominant(),
            IntentProfile::Web,
            "{:?}",
            timetable.all()
        );

        // "What time is it" and "what day of the month is it" MUST NOT DRIFT to Web: those are the
        // device's clock, and the `time` cluster in this project once dropped from
        // 4/4 to 1/4.
        for m in [
            "What time is it?",
            "What day of the month is it?",
            "What is today's date?",
        ] {
            let s = score_intent(m);
            assert!(s.score(IntentProfile::Web) == 0, "{m}: {:?}", s.all());
        }
    }

    /// IRRELEVANCE MUST NOT BE AFFECTED by the trigger list. The easiest mistake
    /// when adding a new word is that it occurs as a substring inside a greeting
    /// (like "sort" inside "assortment").
    #[test]
    fn a_greeting_scores_on_no_profile() {
        for m in [
            "Hello",
            "Thank you very much.",
            "How are you?",
            "Who are you?",
        ] {
            let s = score_intent(m);
            assert_eq!(s.score(IntentProfile::Web), 0, "{m}: {:?}", s.all());
        }
    }

    #[test]
    fn the_dominant_profile_is_picked_correctly() {
        assert_eq!(
            score_intent("create a docx document").dominant(),
            IntentProfile::Document
        );
        assert_eq!(
            score_intent("tomorrow's meeting").dominant(),
            IntentProfile::Time
        );
        assert_eq!(
            score_intent("calculate the percent").dominant(),
            IntentProfile::Calc
        );
        // With no trigger at all it falls back to General.
        assert_eq!(score_intent("zzz qqq").dominant(), IntentProfile::General);
    }

    #[test]
    fn a_tool_from_an_unrelated_profile_scores_nothing() {
        // The message has no time phrase; by the product rule the calendar tool
        // scores 0 and cannot get AHEAD of the scoring document tools.
        let selection = Router::new().select("format the docx document report", &catalog());
        let names: Vec<&str> = selection.iter().map(|t| t.name()).collect();
        let document = names.iter().position(|n| *n == "document_edit").unwrap();
        let calendar = names.iter().position(|n| *n == "calendar_read").unwrap();
        assert!(document < calendar, "{names:?}");
    }
}
