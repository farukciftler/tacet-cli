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
use tacet_kernel::{Tool, ToolCatalog};

/// The most tools shown to the model in one session.
// 8 → 9 THE DAY THE 13TH TOOL (calendar) JOINED: with 13 tools and a budget
// of 8, `find_file` fell off its own "find the file" message purely on the
// hint-length tie — the eval invariant caught it. One more slot restores every
// tool's home turf; the count is still small enough for a 4B's selection.
pub const MAX_TOOLS: usize = 9;

/// How many of the budget's slots a remote catalog may claim when none of its
/// tools won a slot on merit.
pub const RESERVED_SLOTS: usize = 3;

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
    /// THE ENGLISH HALF of the trigger data. These strings are not code but
    /// DATA — they are matched against what the user types, and every one of
    /// them was added from a measured failure (the records are kept in the
    /// comments below).
    ///
    /// THE PRICE THIS COMMENT USED TO WARN ABOUT IS NO LONGER PAID, and the
    /// correction matters because the old text ("a user who writes in Turkish no
    /// longer touches any trigger") reads as an open wound and would send the
    /// next reader fixing something that is fixed. When the code base was moved
    /// to English these strings went with it and the Turkish score really did
    /// fall to zero; `locale_triggers` was then added and `score_intent` sums
    /// BOTH lists. Every entry there mirrors a measured English twin here — the
    /// follow-up work that comment promised is done.
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
                // CAME FROM MEASUREMENT (the eval invariant, the day the 13th
                // tool joined the catalog): "summarize the file budget-2026.md"
                // touched NO Document trigger — the profile scored zero and
                // read_document fell off the budget purely by tie order.
                // "file" ALONE was tried and REVERTED in the same change: it
                // boosted the document trio on "find the FILE about X" and
                // pushed find_file itself off the budget. The narrower pair
                // below carries the original failing case without the side
                // effect.
                "summar",
                ".md",
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
                // TURKISH RESTORED (simplified forms — the matcher folds
                // diacritics): the English pass translated these away, but they
                // are DATA matched against what the user actually types, and
                // Turkish users type Turkish. Every entry mirrors a measured
                // English twin above.
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

    /// THE TURKISH HALF OF THE TABLE — the follow-up the header above promised.
    ///
    /// WHY A SECOND LIST AND NOT MORE ENTRIES IN THE FIRST: the English list is
    /// matched against tool names and descriptions elsewhere in this file, and
    /// mixing languages there was already producing questions like "is `ozetle`
    /// an English word we forgot to translate". Two lists, one score: the total
    /// is the sum over both, so moving a string from one to the other cannot
    /// change any measurement — which is how the Turkish entries that used to
    /// sit inline above were moved down here safely.
    ///
    /// WRITTEN WITHOUT DIACRITICS on purpose. The message is put through
    /// `simplify` before matching, which folds ş→s, ı→i, ğ→g and friends; a
    /// trigger spelled "döviz" would then match nothing at all. The folded
    /// spelling is not a typo, it is the only spelling that can match.
    ///
    /// MEASURED, this is not speculation: with these absent, "Dolar kuru şu an
    /// ne durumda?" and "Kahve sevdiğimi unut artık" scored zero on every
    /// profile, so `web_search` and `remember` never entered the nine-tool
    /// budget and the model was asked to answer without ever being shown the
    /// tool it needed. Three of the Turkish suite's five failures were this.
    fn locale_triggers(&self) -> &'static [&'static str] {
        match self {
            IntentProfile::Document => &[
                "belge",
                "rapor",
                "tablo",
                "ozetle",
                "ozet cikar",
                "duzenle",
                "dosyaya yaz",
                "word dosyasi",
                "excel dosyasi",
                "sunum",
                "baslik ekle",
            ],
            IntentProfile::Time => &[
                "takvim",
                "toplanti",
                "etkinlik",
                "hatirlat",
                "randevu",
                "ajanda",
                "yarin",
                "bugun",
                "saat kac",
                "ayin kaci",
                "kac gun",
                "hangi gun",
                "gelecek hafta",
                "yilbasi",
            ],
            IntentProfile::Calc => &[
                "hesapla",
                "kac eder",
                "yuzde",
                "carp",
                "topla",
                "ortalama",
                "kod calistir",
                "betik",
                "program yaz",
            ],
            IntentProfile::Web => &[
                // THE CURRENT-INFORMATION MARKERS, in Turkish.
                //
                // "kuru", NOT "kur" — AND THE REASON IS THE INTERESTING PART.
                // The term-boundary rule that keeps "url" out of
                // "tesekkurler" is length-based: a root under four characters
                // must match as a WHOLE term. But Turkish glues its suffixes on
                // the end, so the exchange rate is written "dolar kuru", never
                // bare "kur" — the rule that protects against a false positive
                // also blocks a legitimate inflection. Four characters and up
                // keep prefix matching, so "kuru" matches "kuru" and "kurunu"
                // while "kurulum" and "kurabiye" stay out by their own
                // spelling. Write Turkish triggers at four characters where the
                // language allows it; this is not a workaround, it is the rule
                // working as designed.
                "doviz",
                "kuru",
                "dolar",
                "euro",
                "altin",
                "haber",
                "guncel",
                "son durum",
                "borsa",
                "hava durumu",
                "kac tl",
                "kac dolar",
                "internette",
                "sitesi",
                "adresi",
                "sefer saatleri",
                "canli",
            ],
            IntentProfile::General => &[
                // MEMORY, the way it is asked: "unutma" (do not forget) and
                // "unut" (forget) point at the SAME tool — one writes a note,
                // the other removes it, and both are `remember`.
                "hatirla",
                "unutma",
                "unut",
                "aklinda tut",
                "not al",
                "kaydet",
                // FILES.
                "dosya",
                "klasor",
                "listele",
                // "ara" and "bul" are three letters: the term-boundary rule is
                // what stops "ara" from catching "aralik" (December) — the
                // measured failure the rule was born from, one crate over.
                "ara",
                "bul",
                "oku",
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
                // "spreadsheet" WAS ADDED HERE AND THEN TAKEN BACK OUT, and the
                // measurement is worth more than the line would have been.
                //
                // The lint below found that "sheet" had been reaching
                // `read_document` by hiding inside "spreadsheet" in its own
                // description — correct in effect, accidental in mechanism, and
                // silently gone once scoring respected term boundaries. Naming
                // it explicitly restored that case's ordering exactly. It also
                // changed the hint's weight from 5 to 11 (scores are sums of
                // lengths), moved TEN more orderings, fixed one case and broke
                // two others. Suite totals across all four variants tried today:
                // 42, 41, 41, 40 out of 50, with a different set failing every
                // time; a sign test over the pooled suite gives p = 0.69.
                //
                // So the instrument cannot tell these apart, and when it cannot,
                // the smaller perturbation wins — every ordering that moves is a
                // chance to break something the suite cannot see. The promotion
                // was never load-bearing either: `read_document` is already a
                // document tool by the word "document" in its own NAME.
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
                .chain(p.locale_triggers().iter())
                // TERM BOUNDARIES, NOT BARE `contains`. Measured: "Çok
                // teşekkürler" folds to "cok tesekkurler", which contains the
                // three-letter Web trigger "url" — so a thank-you pulled
                // `web_fetch` and `web_search` to the front of the budget, on
                // the one metric this project ties its exit code to. The rule
                // is not reinvented here: it is `tacet_skills::matching`, the
                // same function the skill and memory layers match with, so the
                // three cannot drift apart.
                .filter(|t| tacet_skills::matching::contains(&simple, t))
                .map(|t| t.len())
                .sum();
            (*p, total)
        })
        .collect();
    IntentScores { scores }
}

/// WHY a tool did or did not reach the model, for one message.
///
/// WHY IT EXISTS: the three defects fixed in this file were found by dumping a
/// prompt and looking at which tools were in it — sixty seconds of work that
/// told us more than an hour of model runs. A diagnostic that valuable should
/// not be a thing somebody has to know to improvise; it should be a command.
/// The shell renders it as `tacet why "<message>"`.
#[derive(Debug, Clone)]
pub struct Explanation {
    /// Profile, score, and the triggers that fired — in scoring order.
    pub profiles: Vec<(IntentProfile, usize, Vec<&'static str>)>,
    /// The tools that reached the model, in the order it sees them, each with
    /// the two numbers that put it there: its profile score and its word
    /// overlap with the message. A tool at position four with `0 / 0` is there
    /// by catalog order and nothing else — which is worth being able to see.
    pub selected: Vec<(String, usize, usize)>,
    /// The ones that did not, with the reason as far as the router knows it.
    pub dropped: Vec<String>,
}

impl Router {
    /// Explains a selection instead of just making one.
    pub fn explain(&self, message: &str, catalog: &ToolCatalog) -> Explanation {
        let simple = simplify(message);
        let scores = score_intent(message);
        let profiles = IntentProfile::ALL
            .iter()
            .map(|p| {
                let fired: Vec<&'static str> = p
                    .message_triggers()
                    .iter()
                    .chain(p.locale_triggers().iter())
                    .filter(|t| tacet_skills::matching::contains(&simple, t))
                    .copied()
                    .collect();
                (*p, scores.score(*p), fired)
            })
            .collect();
        let message_stems = stems(&simple);
        let selected: Vec<(String, usize, usize)> = self
            .select(message, catalog)
            .iter()
            .map(|t| {
                (
                    t.name().to_string(),
                    self.tool_score(t.as_ref(), &scores),
                    overlap(t.as_ref(), &message_stems),
                )
            })
            .collect();
        let dropped = catalog
            .names()
            .into_iter()
            .map(String::from)
            .filter(|n| !selected.iter().any(|(s, _, _)| s == n))
            .collect();
        Explanation {
            profiles,
            selected,
            dropped,
        }
    }
}

/// The router that applies the tool budget.
///
/// Stateless: the same message + the same catalog always gives the same list.
#[derive(Debug, Clone, Default)]
pub struct Router {
    max: Option<usize>,
    /// See `reserving`.
    reserved: Vec<String>,
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
    /// Names that may not be crowded out of the budget entirely — the remote
    /// (MCP) tools.
    ///
    /// WHY THIS EXISTS: the profiles are a table written against the built-in
    /// catalog, so a remote tool only ever scores through the word overlap
    /// above — and word overlap is a LATIN-ALPHABET trick. Measured: "wie ist
    /// die Festplattenauslastung auf meinem Server" reached the remote tools,
    /// "サーバーのディスク使用状況" did not, and the model answered that it has
    /// no access to servers. It was telling the truth: nothing in its prompt
    /// said otherwise. A few reserved slots mean the model always knows the
    /// connection EXISTS, whatever language the question is in.
    pub fn reserving(mut self, names: Vec<String>) -> Self {
        self.reserved = names;
        self
    }

    pub fn select(&self, message: &str, catalog: &ToolCatalog) -> Vec<Arc<dyn Tool>> {
        let scores = score_intent(message);
        let mut ordered: Vec<(usize, usize, Arc<dyn Tool>)> = catalog
            .tools()
            .iter()
            .enumerate()
            .map(|(i, t)| (self.tool_score(t.as_ref(), &scores), i, t.clone()))
            .collect();

        // The key is (-profile score, -word overlap, catalog order): fully
        // deterministic, and the middle term is what lets a tool the profiles
        // have never heard of reach the model at all (see `overlap`).
        let message_stems = stems(&simplify(message));
        ordered.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| {
                    overlap(b.2.as_ref(), &message_stems)
                        .cmp(&overlap(a.2.as_ref(), &message_stems))
                })
                .then(a.1.cmp(&b.1))
        });
        let budget = self.budget();
        // THE RESERVATION IS FOR SILENCE, NOT FOR COMPETITION. It exists so a
        // question in a language the trigger table has never seen can still
        // reach a connected server. When a profile DID fire, the message has
        // said what it is about and speculative remote tools should not be
        // pushing scoring ones out of the budget — measured on the same
        // exchange-rate question, where five of nine slots had gone to the
        // server.
        let nothing_matched = scores.score(scores.dominant()) == 0;
        let mut chosen: Vec<Arc<dyn Tool>> = ordered
            .iter()
            .take(budget)
            .map(|(_, _, t)| t.clone())
            .collect();

        // THE RESERVATION, applied last and only when it changes something: if
        // no reserved tool made the cut on merit, the weakest of the chosen
        // give way to the best-ranked reserved ones. `RESERVED_SLOTS` is small
        // on purpose — this is "the model must know the connection exists", not
        // "remote tools win".
        if nothing_matched && !self.reserved.is_empty() && budget > RESERVED_SLOTS {
            let already = chosen
                .iter()
                .filter(|t| self.reserved.iter().any(|n| n == t.name()))
                .count();
            if already < RESERVED_SLOTS {
                let wanted = RESERVED_SLOTS - already;
                let extra: Vec<Arc<dyn Tool>> = ordered
                    .iter()
                    .map(|(_, _, t)| t)
                    .filter(|t| self.reserved.iter().any(|n| n == t.name()))
                    .filter(|t| !chosen.iter().any(|c| c.name() == t.name()))
                    .take(wanted)
                    .cloned()
                    .collect();
                chosen.truncate(budget.saturating_sub(extra.len()));
                chosen.extend(extra);
            }
        }
        chosen
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
                // TERM BOUNDARIES ON THIS SIDE TOO. The message side was fixed
                // first ("url" inside "tesekkurler"); the tool side had the
                // same hole and it was found by `tacet why` on a live catalog:
                // the Turkish word "türleri" (types) folds to "turleri", which
                // CONTAINS "url", so a directory-listing tool scored as a WEB
                // tool and took a slot in an exchange-rate question. Same root
                // cause, third instance — and the last place it can hide.
                // TERM BOUNDARIES ON THIS SIDE TOO — and the reasoning is
                // worth keeping, because it was reversed once and reversed
                // back. It closes a real hole: the Turkish "türleri" (types)
                // folds to "turleri", CONTAINS "url", and a directory-listing
                // tool scored as a WEB tool. Reverting it was tried, on the
                // suspicion that it caused three English cases to flip, and
                // MEASURED: the suite scored 26/32 with it and 26/32 without,
                // the same total with one different case failing. It costs
                // nothing measurable and fixes something real, so it stays.
                let hint: usize = p
                    .tool_hints()
                    .iter()
                    .filter(|t| tacet_skills::matching::contains(&text, t))
                    .map(|t| t.len())
                    .sum();
                message_score * hint
            })
            .sum()
    }
}

/// WORDS THE MESSAGE AND THE TOOL SHARE — the tie-breaker that lets a tool the
/// profiles know nothing about be chosen.
///
/// WHY IT HAD TO EXIST: the profiles are a fixed table written against the
/// BUILT-IN catalog, but MCP adds tools at run time, named and described by
/// somebody else, in whatever language they please. Measured: with a server
/// offering 20 remote tools, "serverdeki disk durumu nedir" scored every one of
/// them zero, the budget filled with built-ins in catalog order, and the model —
/// which had never been shown the tool — answered "I cannot access external
/// systems". It was right: nothing had told it otherwise.
///
/// It is a TIE-BREAKER, not a score. A profile that fires still decides first;
/// this only orders the mass of tools that scored zero, where the alternative
/// was catalog order, which is to say alphabetical luck.
fn overlap(tool: &dyn Tool, message_stems: &[String]) -> usize {
    if message_stems.is_empty() {
        return 0;
    }
    let text = simplify(&format!("{} {}", tool.name(), tool.description()));
    let tool_stems = stems(&text);
    let matched: Vec<&String> = message_stems
        .iter()
        .filter(|stem| tool_stems.iter().any(|t| t == *stem))
        .collect();
    // TWO STEMS, NOT ONE. Measured with `tacet why` on "Dolar kuru şu an ne
    // durumda?": the single stem "duru" (from "durumda", an everyday word for
    // "state") matched `disk_durumu`, `servis_durumu` and `ag_durumu`, and
    // three server tools took slots in a question about an exchange rate. One
    // shared everyday word is a coincidence; two are a subject. "serverdeki
    // disk durumu" shares three ("serv", "disk", "duru") and is unaffected.
    if matched.len() < 2 {
        return 0;
    }
    matched.iter().map(|stem| stem.len()).sum()
}

/// The distinctive words of a piece of text, cut to a stem.
///
/// STEMMED, because Turkish glues its grammar onto the end of the word: the
/// message says "diskin", "durumu", "loglari" where the tool says "disk",
/// "durum", "log". Five characters is enough to keep "docker" apart from
/// "dosya" and short enough to survive a suffix.
fn stems(text: &str) -> Vec<String> {
    // "server"/"sunucu" are NOT here on purpose: for a remote catalog the
    // connection's name is the strongest signal in the sentence, and stopping
    // it was measured to hide every remote tool from a message that named the
    // machine it meant.
    const STOP: &[&str] = &[
        "nedir", "nasil", "bana", "icin", "ile", "var", "bir", "the", "and", "for", "what",
        "please", "you", "can", "give", "tell",
    ];
    let mut out: Vec<String> = Vec::new();
    for word in text.split(|c: char| !c.is_alphanumeric()) {
        if word.chars().count() < 3 || STOP.contains(&word) {
            continue;
        }
        // FOUR characters, not five: measured across languages, "servidor" and
        // "serverim" share four ("serv") and diverge at the fifth. Four is the
        // shortest cut that still keeps "docker" apart from "dosya".
        let stem: String = word.chars().take(4).collect();
        if !out.contains(&stem) {
            out.push(stem);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use tacet_kernel::{
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

    /// THE THREE TURKISH FAILURES, as a test that needs no model.
    ///
    /// Measured before this: "Dolar kuru şu an ne durumda?" and "Kahve
    /// sevdiğimi unut artık" scored zero on every profile, so the nine-slot
    /// budget filled with the head of the catalog and the model was never shown
    /// `web_search` or `remember`. Three of the Turkish suite's five failures
    /// were this, and none of them was the model's fault.
    #[test]
    fn a_turkish_message_reaches_the_tool_it_needs() {
        let rate = score_intent("Dolar kuru şu an ne durumda?");
        assert!(
            rate.score(IntentProfile::Web) > 0,
            "an exchange-rate question must point at the web"
        );
        let forget = score_intent("Kahve sevdiğimi unut artık");
        assert!(
            forget.score(IntentProfile::General) > 0,
            "'unut' must point at the memory tool"
        );
        let news = score_intent("Bugün haberlerde ne var?");
        assert!(news.score(IntentProfile::Web) > 0);
    }

    /// AND THE FALSE POSITIVE THAT CAME WITH THE SAME CODE.
    ///
    /// "Çok teşekkürler, harikaydı!" folds to "cok tesekkurler, harikaydi!",
    /// which CONTAINS the three-letter Web trigger "url". With a bare substring
    /// search a thank-you pulled `web_fetch` and `web_search` to the front of
    /// the budget — on an irrelevance case, which is the metric the CLI ties
    /// its exit code to. The term-boundary rule is what stops it.
    #[test]
    fn a_thank_you_does_not_look_like_a_web_request() {
        let thanks = score_intent("Çok teşekkürler, harikaydı!");
        assert_eq!(
            thanks.score(IntentProfile::Web),
            0,
            "'tesekkurler' contains 'url' and must not score as a web request"
        );
        for greeting in ["Selam, nasılsın?", "İyi akşamlar", "Günaydın"] {
            let s = score_intent(greeting);
            assert_eq!(
                s.score(IntentProfile::Web),
                0,
                "{greeting} must not touch the web profile"
            );
        }
        // The rule the limit exists for, in its original shape: "ara" must not
        // catch "aralık".
        let december = score_intent("20 aralık için toplantı koy");
        assert!(
            december.score(IntentProfile::Time) > 0,
            "a December meeting is calendar work"
        );
    }

    /// WHAT `tacet why` FOUND THE DAY IT WAS WRITTEN.
    ///
    /// A remote catalog of twenty server tools was connected. For "Dolar kuru
    /// şu an ne durumda?" five of the nine slots went to the server: three
    /// because the everyday word "durumda" shares a four-character stem with
    /// `disk_durumu` / `servis_durumu` / `ag_durumu`, and two because the
    /// reservation fired even though the message had said plainly what it was
    /// about. One shared everyday word is a coincidence; and the reservation is
    /// for silence, not for competition.
    #[test]
    fn a_remote_catalog_does_not_crowd_a_message_that_said_what_it_wants() {
        let mut catalog = ToolCatalog::new();
        catalog.add(Arc::new(FakeTool {
            name: "web_search",
            description: "Searches the web.",
        }));
        for name in [
            "serverim_disk_durumu",
            "serverim_servis_durumu",
            "serverim_ag_durumu",
            "serverim_dizin_listele",
        ] {
            catalog.add(Arc::new(FakeTool {
                name,
                description: "Sunucudaki durumu gosterir.",
            }));
        }
        let remote: Vec<String> = catalog
            .names()
            .into_iter()
            .filter(|n| n.starts_with("serverim_"))
            .map(String::from)
            .collect();
        let router = Router::new().max(3).reserving(remote);

        // The message named its subject: the web tool leads and the server
        // tools do not take the rest of the budget on a shared "durum".
        let picked: Vec<String> = router
            .select("Dolar kuru şu an ne durumda?", &catalog)
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        assert_eq!(picked.first().map(String::as_str), Some("web_search"));

        // And the case the reservation exists for is untouched: a question in a
        // language the table has never seen still reaches the server.
        let silent: Vec<String> = router
            .select("サーバーのディスク使用状況を教えて", &catalog)
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        assert!(
            silent.iter().any(|n| n.starts_with("serverim_")),
            "nothing scored, so the connection must still be visible: {silent:?}"
        );
    }

    /// THE SAME BUG, ON THE TOOL SIDE.
    ///
    /// Found by `tacet why` against a live remote catalog: a tool described in
    /// Turkish as listing "türleri" (types) folds to "turleri", which contains
    /// the three-letter web hint "url" — so a directory listing scored as a web
    /// tool and took a slot in a question about an exchange rate. The message
    /// side had already been fixed; this is the other half of the same hole.
    #[test]
    fn a_turkish_word_in_a_tool_description_is_not_a_web_hint() {
        struct Remote;
        impl Tool for Remote {
            fn name(&self) -> &str {
                "serverim_dizin_listele"
            }
            fn description(&self) -> &str {
                "Belirtilen klasörün içeriğini listeler (dosya adları, türleri)."
            }
            fn schema(&self) -> ArgSchema {
                ArgSchema::empty()
            }
            fn run<'a>(&'a self, _a: Value, _c: &'a mut ToolContext) -> ToolFuture<'a> {
                boxed(async { ToolOutcome::read_ok("ok", "ok") })
            }
        }
        let router = Router::new();
        let scores = score_intent("Dolar kuru şu an ne durumda?");
        assert!(
            scores.score(IntentProfile::Web) > 0,
            "the message is a web one"
        );
        assert_eq!(
            router.tool_score(&Remote, &scores),
            0,
            "'turleri' contains 'url' and must not make a directory listing a web tool"
        );
    }

    /// LINT 1 — THE RULE MAY NOT BE UNLEARNED.
    ///
    /// The same substring bug was found three times in one day, in three
    /// different places: "url" inside "teşekkürler" (a message), "url" inside
    /// "türleri" (a tool description), and "sum" inside "summarize". Each was
    /// fixed by routing the comparison through `tacet_skills::matching`, which
    /// is the rule the skill and memory layers already share.
    ///
    /// This test reads THIS FILE and refuses to let a scoring path go back to a
    /// bare substring search. It is deliberately literal rather than clever: a
    /// clever check on a source file is a check nobody can read when it fires.
    #[test]
    fn every_scoring_path_matches_on_term_boundaries() {
        // ONLY THE CODE, NOT THE TESTS: the forbidden patterns below appear as
        // string literals in this very test, so linting the whole file would
        // make it fail on itself — which is the sort of joke a test plays once.
        let whole = include_str!("router.rs");
        let source = whole.split("#[cfg(test)]").next().unwrap_or(whole);
        let uses: usize = source.matches("matching::contains(").count();
        assert!(
            uses >= 2,
            "both scoring sides (message triggers and tool hints) must go \
             through the shared rule; found {uses} call sites"
        );
        for forbidden in [
            "|t| simple.contains(",
            "|t| text.contains(",
            ".filter(|t| simple.contains",
            ".filter(|t| text.contains",
        ] {
            assert!(
                !source.contains(forbidden),
                "a scoring path went back to a bare substring search: {forbidden:?}. \
                 Three separate bugs came from exactly this; use \
                 tacet_skills::matching::contains."
            );
        }
    }

    /// LINT 2 — NO HINT MAY PROMOTE A TOOL BY MATCHING INSIDE A WORD.
    ///
    /// The `türleri` / `url` case, generalised: every tool in the production
    /// catalog is scored against every profile hint BOTH WAYS, and any hint
    /// that a bare search would find but a term-boundary search would not is
    /// reported by name. Such a match is silent — the tool simply ranks higher
    /// than it should and nobody sees why — so the list is pinned here rather
    /// than left to be rediscovered.
    ///
    /// A NEW ENTRY IS NOT AUTOMATICALLY A BUG. It becomes one when it changes a
    /// ranking, which is why the assertion is on the LIST rather than on the
    /// count: adding a tool whose description happens to contain "url" inside a
    /// word is fine, and recording it here is the price.
    #[test]
    fn no_tool_is_promoted_by_a_hint_hiding_inside_a_word() {
        let store = std::sync::Arc::new(crate::data_store::SharedStore::new());
        let memory = crate::memory::SharedMemory::in_memory();
        let (catalog, _, _) = crate::catalog::production_catalog(&store, &memory, Some(0));
        let mut collisions: Vec<String> = Vec::new();
        for tool in catalog.tools() {
            let text = simplify(&format!("{} {}", tool.name(), tool.description()));
            for profile in IntentProfile::ALL {
                for hint in profile.tool_hints() {
                    if text.contains(hint) && !tacet_skills::matching::contains(&text, hint) {
                        collisions.push(format!("{}: {hint}", tool.name()));
                    }
                }
            }
        }
        collisions.sort();
        collisions.dedup();
        // THE RECORDED LIST, and each entry is a decision.
        //
        // Both are the General hint "write" hiding inside "rewrite", in the
        // sentence "never rewrite it" that `time` and `calendar` use to tell the
        // model not to reword a date. Those promotions were FALSE — neither tool
        // writes anything — and they vanished when scoring started respecting
        // term boundaries. They are recorded rather than removed because the
        // prose is right; it is the hint that had no business matching it.
        //
        // The third entry is the opposite case and the instructive one: "sheet"
        // hides inside "spreadsheet" in `read_document`'s own description. That
        // promotion was CORRECT in effect — read_document really is the
        // spreadsheet tool — and accidental in mechanism, so it vanished with
        // the boundary rule and took the tool's rank with it.
        //
        // Naming "spreadsheet" as a hint was tried, and MEASURED: it restored
        // one case's ordering exactly, moved ten more, and the suite went from
        // 41/50 to 40/50 with a different set failing. The instrument cannot
        // resolve that (p = 0.69), so the smaller change won and the entry
        // stays here — recorded, harmless, and explained rather than fixed.
        // PLATFORM-PROOF, THE HARD WAY — this test failed on Linux the first
        // time it ran there, and the reason is the one this project documented
        // in its own review the same morning: `calendar` is macOS-only, so a
        // fixed list of expected pairs is a list about ONE operating system.
        // The assertion is therefore two-directional and asks the catalog what
        // it contains rather than assuming: nothing unrecorded may collide, and
        // every recorded pair whose tool is actually here must still collide —
        // so a pair that stops colliding cannot rot in this list either.
        let recorded = [
            ("calendar", "write"),
            ("read_document", "sheet"),
            ("time", "write"),
        ];
        let present: Vec<String> = catalog
            .tools()
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        let expected: Vec<String> = recorded
            .iter()
            .filter(|(tool, _)| present.iter().any(|p| p == tool))
            .map(|(tool, hint)| format!("{tool}: {hint}"))
            .collect();
        assert_eq!(
            collisions, expected,
            "a profile hint matches INSIDE a word of a tool's own text. With the \
             term-boundary rule in place this is silent rather than harmful, but \
             it means the hint and the description disagree about what the tool \
             is — decide which one is wrong, then fix the wording, add the hint \
             properly, or record the pair here with the reason."
        );
    }

    /// AND THE SAME LINT ON THE OTHER PLATFORM'S CATALOG.
    ///
    /// The two-directional form above was not caution, it was a repair: the
    /// fixed list passed on macOS and failed on Linux, where `calendar` is not
    /// in the catalog at all. This case builds the smaller shape by hand so the
    /// platform difference is measured on both machines rather than on
    /// whichever one happens to run the test.
    #[test]
    fn the_collision_lint_holds_on_a_catalog_without_the_mac_only_tools() {
        let store = std::sync::Arc::new(crate::data_store::SharedStore::new());
        let memory = crate::memory::SharedMemory::in_memory();
        let (full, _, _) = crate::catalog::production_catalog(&store, &memory, Some(0));
        let mut lean = ToolCatalog::new();
        for tool in full.tools() {
            if tool.name() != "calendar" {
                lean.add(std::sync::Arc::clone(tool));
            }
        }
        let mut collisions: Vec<String> = Vec::new();
        for tool in lean.tools() {
            let text = simplify(&format!("{} {}", tool.name(), tool.description()));
            for profile in IntentProfile::ALL {
                for hint in profile.tool_hints() {
                    if text.contains(hint) && !tacet_skills::matching::contains(&text, hint) {
                        collisions.push(format!("{}: {hint}", tool.name()));
                    }
                }
            }
        }
        collisions.sort();
        collisions.dedup();
        assert!(
            !collisions.iter().any(|c| c.starts_with("calendar")),
            "a tool that is not in this catalog cannot collide in it: {collisions:?}"
        );
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
