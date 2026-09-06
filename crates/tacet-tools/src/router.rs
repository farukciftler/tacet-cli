//! Router — the per-session tool budget.
//!
//! DECISION (inherited by the Swift side): AT MOST `MAX_TOOLS` tools are shown
//! to the model in a session — 8 when this line was written, 9 since, and the
//! constant is the only place the number lives. The reason is measurement: a
//! small model cannot pick the right
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

/// THE SPLITTING RULE, and it is the only one this enum has ever grown by:
/// **two intents that share words but want OPPOSITE tools are two profiles.**
/// `Web` was split off `General` for exactly that reason ("read that page" vs
/// "read that file"), and three more splits have since been forced by the
/// routing measurement:
///
/// * `General` -> `Files` + `Memory`. "Find the file about the budget" and
///   "Remember my address" both scored under General, so the `remember` tool —
///   whose NAME is the General hint `remember` — headed the list on every file
///   question. Measured on the routing set: `find_file` sat at rank 7 on its own
///   sentence.
/// * `Time` -> `Clock` + `Calendar`. "What time is it" and "what is on my
///   calendar tomorrow" both scored under Time, and `calendar` won both,
///   because `calendar` is itself a Time hint while `time` was not. The device
///   clock and the user's diary are not the same question.
/// * `Repo` was added rather than folded into `Files`: "which FILES have I
///   changed in this git repository" fires both, and with one profile
///   `find_file` outscored `git` on a sentence about commits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntentProfile {
    Document,
    /// The DEVICE CLOCK and calendar arithmetic — what time is it, what is
    /// today's date, how many days until X.
    Clock,
    /// The USER'S DIARY — events, meetings, reminders.
    Calendar,
    /// CHANGING a document that already exists.
    ///
    /// WHY IT IS NOT PART OF `Document`: the three document tools share every
    /// word a message can carry (".md", "table", "report"), so under one
    /// profile their order is fixed by hint mass alone and `create_document`
    /// headed every request to EDIT one — the message had no way to say
    /// otherwise. The edit VERBS are what separate them, and a verb needs a
    /// profile to score in.
    DocEdit,
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
    /// The version-controlled working directory.
    Repo,
    /// Files ON THIS DEVICE — finding, listing, reading them.
    Files,
    /// What the user asked to be remembered or forgotten.
    Memory,
    /// A .zip the user received — what is inside it, and unpacking it.
    ///
    /// WHY IT IS NOT PART OF `Files`: an archive question uses every Files word
    /// ("what FILES are in this", "unpack it into a FOLDER") while wanting a
    /// tool `find_file` cannot be. Folded into Files, `archive` would have to
    /// beat `find_file` and `read_document` on their own hint mass to be reached
    /// at all, and the split is the same one `Repo` was given for the same
    /// reason.
    Archive,
    /// Is this file what it claims to be — checksums, digests, "are these two
    /// the same file".
    Integrity,
    /// A sentence that has STRUCTURE HIDDEN IN IT: a request to be turned into a
    /// search filter, a message to be classified.
    ///
    /// ONE PROFILE FOR TWO TOOLS, and it is worth saying why rather than
    /// splitting it. The two families share nothing in their vocabulary — "where
    /// can I take the kids" against "what does this reply mean" — so a shared
    /// profile lifts both on either message. What separates them is the second
    /// half of `tool_score`: the profile score is multiplied by the hints matched
    /// on THE TOOL'S OWN name and description, and `search_filter` matches
    /// "search"/"filter"/"request" where `message_intent` matches
    /// "message"/"intent"/"classif". A second profile would buy the same
    /// separation for twice the table.
    Extract,
    /// A QUESTION ABOUT DATA HELD IN A FILE — rows, counts, a lookup in a
    /// SQLite database.
    ///
    /// WHY IT WAS MISSING FOR SO LONG: `db` is an addon tool, and the routing
    /// eval builds its catalog from `production_catalog`, where no addon gate is
    /// open. So the one instrument that would have caught this could not see the
    /// tool — and nothing else asked. `tacet why "how many rows are in the users
    /// table of app.db"` on a real install put `create_document` first and left
    /// `db` off the list entirely: the message scored `document 5` on "table"
    /// and `calc 8` on "how many", and the database tool matched no profile at
    /// all. A tool the user installed, answering the plainest possible phrasing
    /// of its own job, was unreachable.
    Data,
    /// WHAT THE USER LAST COPIED — and only when they say so.
    ///
    /// The same hole as `Data` and for the same reason. The tool's description
    /// goes out of its way to say it must be used ONLY on an explicit request;
    /// that instruction never reached the model, because the tool never reached
    /// the prompt.
    Clipboard,
}

impl IntentProfile {
    /// APPENDED, NEVER INSERTED. `dominant()` breaks a tie in favour of the
    /// FIRST profile in this array, so a new profile at the end LOSES its ties —
    /// the conservative direction for a table nobody has measured for a decade
    /// of messages yet. It also keeps every existing profile's tie behaviour
    /// exactly where the measurements that produced it left it.
    ///
    /// MEASURED WHEN `Archive` AND `Integrity` WERE ADDED, and it is a one-time
    /// measurement rather than a standing test — there is no "before" left to
    /// run once the tools are in the catalog, because `run_routing` builds its
    /// catalog from `production_catalog`.
    ///
    /// `eval --routing` before the change: REACH 154/154, TOP3 153/154, MEAN
    /// RANK 1.38. With the two tools in the catalog but no new case yet: the
    /// SAME 154 outcomes, and a rank-by-rank diff of the two JSON reports showed
    /// ZERO ranks moved. That was the outcome worth checking, because the
    /// second sort key (`overlap`) orders every tool no profile scored, so two
    /// new tools really can displace an existing one on a sentence about
    /// something else — here they did not, on any of the 154. With the twelve
    /// new cases added: REACH 166/166, TOP3 165/166, MEAN RANK 1.36, every new
    /// case at rank 1, and the same single case (`tr-dosya-ara`) outside the top
    /// three as before. Identical numbers under `--routing-pressure 20`.
    ///
    /// MEASURED AGAIN WHEN `Data` AND `Clipboard` WERE ADDED, in the same
    /// commit that made the five file-extension triggers able to fire at all.
    /// `eval --routing` before and after: REACH 166/166, TOP 3 166/166, MEAN
    /// RANK 1.33 — identical, and identical again under `--routing-pressure 20`.
    /// The rank-by-rank diff is NOT zero this time and the two moves are worth
    /// naming: `tr-belge-markdown` went 2 -> 1 and `tr-belge-oku` went 2 -> 3,
    /// so the mean is unchanged because they cancel. Thirteen selections
    /// reordered, every one of them a `.md` sentence — which is the intended
    /// effect of a rule that had never fired starting to fire, not a side
    /// effect of the two new profiles. Neither `db` nor `clipboard` is in the
    /// eval catalog (no addon gate is open there), which is why their
    /// reachability is measured by `the_database_tool_is_reachable_by_the_
    /// plainest_phrasings` and its clipboard twin instead.
    ///
    /// The STANDING guarantees are `every_expected_tool_reaches_the_model` and
    /// `a_connected_server_does_not_push_the_expected_tool_out_of_the_budget`
    /// over in `tacet_eval::routing`.
    pub const ALL: [IntentProfile; 14] = [
        IntentProfile::Document,
        IntentProfile::DocEdit,
        IntentProfile::Clock,
        IntentProfile::Calendar,
        IntentProfile::Calc,
        IntentProfile::Web,
        IntentProfile::Repo,
        IntentProfile::Files,
        IntentProfile::Memory,
        IntentProfile::Archive,
        IntentProfile::Integrity,
        IntentProfile::Extract,
        // APPENDED, per the rule at the top of this array: both lose their ties.
        IntentProfile::Data,
        IntentProfile::Clipboard,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            IntentProfile::Extract => "extract",
            IntentProfile::Data => "data",
            IntentProfile::Clipboard => "clipboard",
            IntentProfile::Document => "document",
            IntentProfile::DocEdit => "doc-edit",
            IntentProfile::Clock => "clock",
            IntentProfile::Calendar => "calendar",
            IntentProfile::Calc => "calc",
            IntentProfile::Web => "web",
            IntentProfile::Repo => "repo",
            IntentProfile::Files => "files",
            IntentProfile::Memory => "memory",
            IntentProfile::Archive => "archive",
            IntentProfile::Integrity => "integrity",
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
                // "md" AND NOT ".md". `matching::contains` requires a
                // non-alphanumeric character BEFORE a match, and `budget-2026.md`
                // has a `6` in front of the dot — so the dotted form could not
                // fire on any real filename, and all 137 extension occurrences
                // across `benchmarks/` are preceded by an alphanumeric. Without
                // the dot the two-character root is under `WHOLE_TERM_LIMIT`, so
                // it matches `notes.md` as a whole term and stays out of
                // `mdadm`. The same edit, for the same reason, on the four
                // below.
                "md",
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
                // CAME FROM THE ROUTING SET: "Export the product list into a
                // spreadsheet file" and "Save a new note file named ideas.md"
                // touched NOTHING in this profile — the first scored only
                // through the Files words "list"/"file", the second only
                // through ".md", and `create_document` sat at ranks 8 and 9 on
                // two plain requests to create a document. "spreadsheet" is the
                // word users write when they mean xlsx without naming it.
                "spreadsheet",
                "export",
                "file named",
                "save a new",
            ],
            IntentProfile::DocEdit => &[
                "change the title",
                "change the",
                // BARE "change" IS HERE, and the git sentence it also touches
                // was checked rather than assumed: "Which files have I changed
                // in this git repository?" scores 6 here and 23 on Repo, and
                // `git`'s Repo mass wins it by more than three to one. What it
                // buys is "Change Tuesday from Rice to Beans." — an edit
                // request with no document word in it at all, which reached
                // NOTHING and left `edit_document` at rank 5 behind two tools
                // that were there on catalog order.
                "change",
                "append",
                "insert",
                "replace",
                "modify",
                "update the",
                "edit the",
                "add the row",
                "add a row",
                "add this row",
                "delete the line",
                "add a section",
                "new section",
            ],
            IntentProfile::Clock => &[
                // CAME FROM MEASUREMENT (the user's real session): "what time is it
                // in turkey", "what day of the month is it" and "how many days
                // until new year" touched no trigger at all — "at what TIME" was
                // there but "what time is it" was not. With the score at zero the
                // selection falls back to catalog order and the `time` tool
                // appeared in the middle rather than at the HEAD of the list; in a
                // small model, position is selection probability.
                "what time is it",
                "at what time",
                "day of the month",
                "which day",
                "what day",
                "today",
                "tomorrow",
                "date",
                // THE "how many days" FAMILY IS ONE ENTRY, not four. The routing
                // set failed on "until Christmas", "until 15 October", "left
                // until new year" and "have passed since 1 January" — four
                // sentences, one intent, and writing four strings would have
                // measured the four sentences rather than the intent. The
                // longest common phrase is the trigger; the two directions
                // ("until"/"since") are kept because they also occur without
                // the "how many days" opener ("days since the release").
                "how many days",
                "days until",
                "days since",
                "new year",
                // CAME FROM THE ROUTING SET: with only bare "current" in the Web
                // profile, "What is the current year?", "Which month are we in
                // currently?" and "What is the current UTC time?" all scored as
                // INTERNET questions and `time` fell to rank 4 behind
                // web_search/web_fetch/find_file. These three name the DEVICE
                // clock, and each is longer than the Web trigger it has to beat.
                "current year",
                "current month",
                "current time",
                "current date",
                "which month",
                "what year",
                "utc",
                // "when" IS DELIBERATELY ABSENT: "when is the meeting" is written
                // as often as "when was he elected", and the second is a web
                // question. Writing it into both profiles equalises the scores;
                // writing it into neither leaves the distinction to the message's
                // other words — in measurement that came out right.
            ],
            IntentProfile::Calendar => &[
                "calendar",
                "event",
                "meeting",
                "appointment",
                "reminder",
                // "remind me" IS SEPARATE FROM "reminder": the request is
                // written as a verb ("remind me to call the dentist") far more
                // often than as the noun, and the noun's trigger does not match
                // it — "reminder" is not a prefix of "remind".
                "remind me",
                // "tomorrow" IS IN BOTH THIS PROFILE AND `Clock`, deliberately.
                // It is the one word that belongs to both questions — "what is
                // tomorrow's date" is the clock, "tomorrow's meeting" is the
                // diary — and putting it in only one made that one win both.
                // Listed twice it cancels out, and the DECIDING word ("date" vs
                // "meeting") carries the sentence, which is what should happen.
                // "today" is NOT given the same treatment: it has no diary
                // reading strong enough to outweigh the news and weather
                // questions it also appears in.
                "tomorrow",
                "schedule",
                "my schedule",
                "agenda",
                "this week",
                "next week",
                "upcoming",
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
                // "times" MOVED HERE FROM THE WEB PROFILE. In English the
                // multiplication sign is written as a word, and while it lived
                // under Web the sentence "What is 125 times 8?" scored as an
                // internet question — measured on the routing set, `calculate`
                // came fourth behind web_search, web_fetch and find_file. It is
                // the same word the timetable case wanted, and the two are told
                // apart by the company it keeps: "ferry" and "bus times" carry
                // their own triggers over there.
                "times",
                // The file extension IS the request when a script is asked for
                // as a file.
                "py",
                "js",
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
                // FOUND BY DRAFTING BENCHMARK QUESTIONS, before any model ran.
                // Nineteen unmistakable web questions were written and seven of
                // them scored ZERO on every profile — "is there a train strike
                // going on in France?", "which stable Rust version is the newest
                // one right now", "how bad is the air quality in Delhi", "who
                // won the champions league final" — so the nine tools shown were
                // whatever sits early in catalog order and `web_search` was not
                // among them. A tool that is not in the prompt cannot be called,
                // so those would have been scored as model failures forever.
                //
                // EACH ONE IS EITHER A REQUEST TO GO AND LOOK, or a word that
                // only makes sense about the outside world. The tempting
                // additions that are NOT here: "most recent" and "recently",
                // because `git` cases say "what was the most recent commit
                // here about?" and pulling the web tools in front of `git` for
                // that would trade one defect for another.
                // TIME-BOUND PHRASES, and they are phrases rather than words for
                // the usual reason: "lately" and "these days" say "the answer
                // moves", where the bare words they contain say nothing.
                "these days",
                "in the last few days",
                "lately",
                "going for",
                "search for",
                "look up",
                "online",
                "find out",
                "newest",
                "strike",
                "air quality",
                "who won",
                "was elected",
                "election",
                "in what year",
                "which year",
                // BARE "current" WAS REMOVED AND THE PHRASES IT STOOD FOR PUT IN
                // ITS PLACE. Measured on the routing set, the single word cost
                // four cases and bought none that these do not: "What is the
                // current year?", "Which month are we in currently?" and "What
                // is the current UTC time?" are DEVICE CLOCK questions, and
                // "Which git branch am I currently on?" is a question about the
                // working directory — all four scored as internet questions and
                // all four put `web_search` at the head of the list. What the
                // word was there for is a PRICE, a RATE or a HEADLINE moving in
                // the world, and each of those is written out below.
                "current price",
                "current rate",
                "current stock",
                "current news",
                "current exchange",
                "current inflation",
                "currently trading",
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
                // BARE "times" WAS REMOVED, and the reason is the multiplication
                // sign nobody writes as a symbol. The line above used to argue
                // that the plural is safe because "what time is it" is never
                // written plural — true, and it missed the other English
                // sentence that carries it: "What is 125 TIMES 8?". Measured on
                // the routing set, that one word made an arithmetic question
                // score as an internet question and put `web_search`,
                // `web_fetch` and `find_file` ahead of `calculate`. The
                // timetable case it was added for ("ortakoy uskudar ferry
                // times") still fires on "ferry" below, so nothing was lost.
                "service times",
                // "ferry times" IS SPELLED OUT ALONGSIDE "ferry", and the pair
                // is what replaced the bare "times" this profile used to carry.
                // With "times" moved to Calc (see there), the real session's
                // "what are the ortakoy uskudar ferry times" scored 5 on each
                // side and the TIE went to Calc — a timetable question about to
                // be answered by the calculator. The longer phrase settles it
                // where it belongs without touching "125 times 8".
                "ferry times",
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
                // "How much is the dollar today?" was scoring 11 on Calc
                // through "how much is" and 6 on Web through "dollar", so the
                // calculator headed a question about an exchange rate. The
                // definite article is what separates the two readings: a
                // calculation is written "how much is 250 lira with...", never
                // "how much is THE ...".
                "how much is the",
                "dollar",
                "euro",
                "stock market",
                "who is",
                // "what is" WAS TRIED AND REMOVED: it also pushed questions about
                // Tacet itself ("what is tacet", "what is a tool") towards the web;
                // a weight with no payoff.
            ],
            IntentProfile::Repo => &[
                "git",
                "commit",
                "branch",
                "repository",
                "repo",
                "staged",
                "my changes",
                "have i changed",
                "diff",
                "pull request",
                // "Summarize my git changes and write me a commit message."
                // scored 9 on Repo and 6 on Document, and 6 x create_document's
                // hint mass beat 9 x git's. The phrase names the one thing only
                // this tool can supply.
                "commit message",
            ],
            IntentProfile::Files => &[
                "file",
                "folder",
                "directory",
                "search",
                "find",
                "locate",
                "list",
                "read",
                "summarize",
                // CAME FROM THE ROUTING SET. "Show me the entire text of
                // notes.txt", "Give me a preview of readme.md" and "Read the
                // latest entries from app.log" name a file by its EXTENSION and
                // nothing else; without these the sentences scored only through
                // the word "note" (which is a Memory word) and `remember`
                // headed the list on three requests to read a file.
                "show me",
                "entire text",
                "preview of",
                "contents",
                "txt",
                "log",
                "workspace",
                "where is",
            ],
            IntentProfile::Memory => &[
                "remember",
                "forget",
                "keep in mind",
                "note down",
                "make a note",
                "do not forget",
                "my name is",
                // "List the notes you keep about me." reached NO memory trigger
                // and `remember` fell out of the budget entirely — the one
                // outcome this measurement calls a hard failure, because a tool
                // that is not in the prompt cannot be called. "note" cannot be
                // the trigger (see below); "about me" can.
                "about me",
                "you keep about",
                // "note" ALONE IS DELIBERATELY ABSENT and it is the whole reason
                // this profile was split out. It sits inside "notes.txt",
                // "notes.md" and "meeting notes" — three FILE requests — so the
                // one word that reads as memory to a human is the one word that
                // cannot be a trigger here.
            ],
            IntentProfile::Archive => &[
                // BARE "zip" WORKS ON A FILE NAME, and it was checked rather
                // than assumed. `tacet_skills::matching::contains` requires a
                // match to START at a term boundary, and in "backup.zip" the
                // character before the match is '.', which is not alphanumeric —
                // so the trigger fires. The three-character whole-term rule then
                // keeps it out of "zipper", and out of "unzip" (preceded by
                // 'n'), which is why the prefixed forms are listed separately.
                //
                // ".zip" IS DELIBERATELY NOT USED, and the reason is the same
                // rule read the other way: in "backup.zip" the character before
                // the '.' IS alphanumeric, so a trigger spelled ".zip" could
                // never fire on the one string it was written for. That is
                // quietly true of the existing ".md", ".txt", ".py", ".log" and
                // ".js" triggers as well — a separate defect, measured while
                // writing this list and NOT fixed here.
                "zip",
                "unzip",
                "unpack",
                "archive",
                "compressed",
                // "extract" WAS TRIED AND DROPPED. It also occurs in "extract
                // the table from report.xlsx", which is a document request, and
                // dropping it costs nothing: "unzip invoices.zip" and "what is
                // in backup.zip" both already fire on "zip".
            ],
            IntentProfile::Data => &[
                // WRITTEN FROM THE WORDS A PERSON USES, not from the tool's own
                // vocabulary. "sqlite" and "sql" are what someone who knows what
                // the file is writes; "how many rows", "records" and "database"
                // are what someone who does not writes. All five natural
                // phrasings in the finding that produced this profile are
                // carried by one of the two halves.
                "sql", "sqlite", "database",
                // "db" IS TWO CHARACTERS, so `matching::contains` requires it to
                // be a WHOLE term — which is exactly what makes it safe and what
                // makes it work: `app.db` matches (the `.` is a boundary and the
                // name ends there) while `dbus` and `adb` do not.
                "db", "rows", "records",
                "query",
                // "table" IS DELIBERATELY ABSENT. It is a Document trigger and a
                // document word — "put this in a table" is a request to WRITE
                // one. "table of" was tried and dropped for the same reason:
                // "the table of contents". The database sense is carried by the
                // company it keeps ("rows in the users table" fires on "rows").
            ],
            IntentProfile::Clipboard => &[
                "clipboard",
                "copied",
                "copy this",
                "paste",
                // BARE "copy" IS NOT HERE. "copy the file to backups" is file
                // work and would take a slot from `find_file` on every one of
                // them. The clipboard senses all carry a second word.
            ],
            IntentProfile::Extract => &[
                // THE SEARCH-FILTER HALF: a request for places or things to do,
                // with the qualifiers a person actually writes.
                //
                // THIS LIST WAS FITTED TO SIXTEEN CASES AND IT SHOWED. Growing
                // the benchmark to sixty-one found that thirty-seven of them
                // never reached the tool at all — "any cheap museums in Dublin
                // this weekend" and "orta fiyatli oteller" share no substring
                // with anything below. What is added is the CATEGORY, not the
                // sentence: the nouns a request for somewhere to go is built
                // from, in both languages. Fitting one trigger per failing case
                // would turn the check green and measure nothing.
                "places to",
                "things to do",
                "to do in",
                "where can i take",
                "somewhere to",
                "somewhere for",
                "somewhere quiet",
                "what is there to do",
                "free places",
                "kid friendly",
                "with the kids",
                "family friendly",
                // The nouns. Nothing else in the catalog answers a question
                // about a restaurant or a museum, so these are unambiguous even
                // though they are single words.
                "attractions",
                "museums",
                "restaurant",
                "hotels",
                "bars in",
                "events in",
                "outing",
                "tasting menus",
                "walking routes",
                "dinner spots",
                "for lunch",
                // THE MESSAGE half. "message" alone is deliberately absent: it
                // sits inside "commit message", which belongs to `git`.
                "what does this message",
                "what does this reply",
                "reply means",
                "reply was",
                "reply says",
                "the reply",
                "classify this",
                "classify it",
                "what do they mean by",
                "sent me this",
                "they wrote",
                "he wrote",
                "she wrote",
                "they replied",
                "they sent",
                "message says",
                "customer says",
                "what is this?",
            ],
            IntentProfile::Integrity => &[
                "checksum",
                // BOTH SPELLINGS, AND THE HYPHEN IS THE REASON. Matching is a
                // literal comparison after `simplify`, which folds case but not
                // punctuation, so "sha256" cannot match the string "sha-256" and
                // vice versa. Publishers write it both ways on the same page.
                "sha256",
                "sha-256",
                "md5",
                "digest",
                // "hash of" AND "hash", NOT ONE OF THEM. Bare "hash" is four
                // characters, so it matches as a PREFIX — which is what carries
                // "hashes", "hashed" and "hash value" — and the longer phrase is
                // there because scoring sums lengths, so the sentence that names
                // the intent outright scores above one that merely uses the word.
                "hash",
                "hash of",
                "fingerprint",
                // THE QUESTION ASKED WITHOUT THE VOCABULARY. "Are these two the
                // SAME FILE", "is the download CORRUPTED" — a user who does not
                // know the word "checksum" still wants this tool, and none of
                // these phrases occurs in any other profile's case set.
                "same file",
                "identical",
                "corrupted",
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
            // The seven-language shape of the same two questions. Turkish first
            // because it is the one with a natively-authored suite behind it.
            IntentProfile::Data => &[
                "veritabani",
                "veri tabani",
                "sorgu",
                "kayit",
                // "satir" (row) IS ABSENT and it was the first thing tried: it
                // is the ordinary word for a LINE of text, so "dosyanin ilk on
                // satiri" — read the first ten lines of a file — scored as a
                // database question. "kayit" (record) has no such second
                // reading in this catalog.
            ],
            IntentProfile::Clipboard => &[
                "pano",
                "kopyaladigim",
                "panoya",
                "panodaki",
                // "kopyala" ALONE IS ABSENT for the reason bare "copy" is: it is
                // the verb for copying a file too.
            ],
            IntentProfile::Extract => &[
                "gidilebilecek",
                "cocukla",
                "ucretsiz yerler",
                "gezilecek yerler",
                "gidilecek yerler",
                "nereye gidebilirim",
                "ne yapilir",
                // THE SAME WIDENING as the English side, by category. Turkish
                // agglutinates, so the stems carry it: "yerler" reaches
                // "gidilecek yerler" and "sakin yerler" alike, and "gez" reaches
                // "gezilecek", "gezebilecegim" and "gezmek".
                "yerler",
                "mekan",
                "gidilecek",
                "gezilecek",
                "gezebilecegim",
                "ne yapabilir",
                "nereye gid",
                "nerede yenir",
                "konaklama",
                "restoran",
                "otel",
                "parkur",
                "etkinlik",
                // The quoting frames, which is how a message to classify
                // arrives in Turkish: someone else's words plus a verb of
                // saying.
                "bu mesaj ne",
                "ne demek istiyor",
                "bu ne demek",
                "ne anlama geliyor",
                "yazmis",
                "demis",
                "cevabi geldi",
                "mesaji geldi",
                "nasil siniflandir",
                "ne diyor",
                "que hacer",
                "sitios para",
                "que quiere decir",
                "que faire",
                "endroits",
                "que veut dire",
                "was kann man",
                "orte fur",
                "was bedeutet",
                "куда сходить",
                "что делать",
                "что значит",
                "去哪里",
                "有什么好玩",
                "什么意思",
            ],
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
                // "adiyla" ("under the name ..."), NOT "kaydet". The verb was
                // moved here first and had to be moved back, and the pair of
                // sentences that settled it is worth writing down:
                //
                //   "Toplanti kararlarini toplanti.md adiyla kaydet"  -> create_document
                //   "Arabami 2. kat B blok park yerine koydugumu kaydet" -> remember
                //
                // Turkish uses one verb for saving a file and for noting a fact,
                // so whichever profile owns "kaydet" loses the other sentence —
                // with it here `remember` fell out of the budget, with it under
                // Memory `create_document` did. What separates them is not the
                // verb but what is being named: the file sentences say ADIYLA,
                // the memory sentence does not. The verb stays under Memory and
                // the naming word carries the file half.
                "adiyla",
                "olustur",
            ],
            IntentProfile::Clock => &[
                "yarin",
                "bugun",
                "saat kac",
                "ayin kaci",
                "kac gun",
                "hangi gun",
                "yilbasi",
                "tarihi",
                "hangi ay",
                "hangi yil",
            ],
            IntentProfile::Calendar => &[
                "takvim",
                "toplanti",
                "etkinlik",
                "hatirlat",
                "randevu",
                "ajanda",
                "gelecek hafta",
                "bu hafta",
            ],
            IntentProfile::Calc => &[
                "hesapla",
                "kac eder",
                "yuzde",
                "carp",
                "topla",
                "ortalama",
                "kod calistir",
                // "betik" AND "betig" ARE BOTH NEEDED and that is the softening
                // rule, not a typo: Turkish softens the final k to g before a
                // vowel, so "betik" becomes "betigi" — and since matching is by
                // PREFIX, "betik" is not a prefix of "betigi" and matched
                // nothing on the two sentences that asked for a script by name.
                "betik",
                "betig",
                "program yaz",
                "asal sayi",
                "sirala",
                "fibonacci",
                // "donustur" (convert/transform) is the same kind of marker as
                // "sirala": it says "produce this with a script, do not write it
                // out from memory". It settles "Sicaklik donusumu yapan betigi
                // donusturucu.py adiyla kaydet", where the Document word
                // "adiyla" was outscoring the two Calc words and sending a
                // request for a SCRIPT to `create_document`.
                "donustur",
                "donusum",
            ],
            IntentProfile::Web => &[
                // THE OTHER FIVE LANGUAGES, added when a natively-authored
                // benchmark in each of them was gated and the SAME FOUR TOOLS
                // fell out of the nine in every one: `remember`, `web_search`,
                // `archive` and `checksum`. That is not a coincidence — the
                // catalog puts exactly those four LAST, because none of them is
                // ever the right answer without an explicit trigger, and this
                // list had triggers in two languages only. In a third language
                // they could not be reached at all.
                //
                // SPELLED AS `simplify` LEAVES THEM. That function folds the
                // Turkish letters and lowercases, and touches nothing else — so
                // German "über" must be written "uber" here, while Cyrillic and
                // Chinese pass through untouched and French "météo" keeps its
                // accent. A trigger spelled the other way is a dead trigger, of
                // which this file has already had one.
                "el tiempo en",
                "qué tiempo",
                "que tiempo",
                "cotizacion",
                "quién ganó",
                "quien gano",
                "quel temps",
                "换多少",
                "meteo",
                "météo",
                "prix du",
                "wetter",
                "wechselkurs",
                "погода",
                "курс",
                "посмотри в интернете",
                "天气",
                "汇率",
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
                // "euro" IS NOT REPEATED HERE. It is spelled identically in both
                // languages, so it was already carried by the Web
                // `message_triggers` list — and `score_intent` chains the two
                // lists without dedup, so every Turkish exchange-rate sentence
                // scored it twice. The locale list is for what the English list
                // CANNOT reach; a word that is the same in both belongs in one
                // place. See `no_trigger_is_listed_twice_in_a_profile`.
                // "altin" ALONE WAS A FALSE POSITIVE, found by `tacet why` on a
                // routing case: Turkish "altinda" / "altindaki" (under, beneath)
                // folds to a string that STARTS WITH "altin", and prefix
                // matching is on for anything four characters and up. So
                // "Dizin altindaki markdown dosyalarini arat" — a request to
                // search a folder — scored 5 points as a GOLD PRICE question.
                // Same family as "url" inside "tesekkurler"; the cure is the
                // same too, name the phrase rather than the root.
                "altin fiyat",
                "gram altin",
                "altin kac",
                "haber",
                "guncel",
                "son durum",
                "borsa",
                "hava durumu",
                // "hava durumu" IS NOT HOW THE QUESTION IS ASKED. The eval's own
                // Turkish weather case reads "Istanbul'da yarin hava nasil
                // olacak?" — no "durumu" in it — so the sentence scored only
                // through the Clock word "yarin" and the model was shown the
                // device clock for a weather question.
                "hava nasil",
                "kac tl",
                "kac dolar",
                "internette",
                "sitesi",
                "adresi",
                "sefer saatleri",
                "canli",
            ],
            IntentProfile::DocEdit => &[
                // "sil" is three letters and therefore matches as a whole term
                // only — which is what is wanted: "sil" (delete) must not be
                // reached from "silinmis" or "silahli".
                "sil", "degistir", "ekle", "guncelle", "satiri", "satirini",
            ],
            IntentProfile::Repo => &[
                // The repository is named in English even in a Turkish
                // sentence ("git reposunda hangi dosyalar degisti") — what
                // Turkish supplies is the verb around it.
                "degisti",
                "degisiklik",
                "commit mesaji",
            ],
            IntentProfile::Files => &[
                "dosya",
                "klasor",
                "dizin",
                "listele",
                // "ara" and "bul" are three letters: the term-boundary rule is
                // what stops "ara" from catching "aralik" (December) — the
                // measured failure the rule was born from, one crate over.
                "ara",
                "bul",
                "oku",
                "goster",
                "nerede",
                "hangi klasor",
                "metnini",
                // "arat" IS NOT REACHED BY "ara". The three-letter root has to
                // match as a WHOLE term (the rule that keeps "ara" out of
                // "aralik"), so the causative "arat" — which is how the request
                // "Dizin altindaki markdown dosyalarini arat" is actually
                // written — matched nothing at all.
                "arat",
                // "hangi dosyaya yazmistim" is a question about WHERE something
                // is, but it contains "dosyaya yaz", which is a Document
                // trigger; without a Files phrase of its own the sentence asked
                // to CREATE a file.
                "hangi dosya",
            ],
            IntentProfile::Memory => &[
                // THE OTHER FIVE LANGUAGES, added when a natively-authored
                // benchmark in each of them was gated and the SAME FOUR TOOLS
                // fell out of the nine in every one: `remember`, `web_search`,
                // `archive` and `checksum`. That is not a coincidence — the
                // catalog puts exactly those four LAST, because none of them is
                // ever the right answer without an explicit trigger, and this
                // list had triggers in two languages only. In a third language
                // they could not be reached at all.
                //
                // SPELLED AS `simplify` LEAVES THEM. That function folds the
                // Turkish letters and lowercases, and touches nothing else — so
                // German "über" must be written "uber" here, while Cyrillic and
                // Chinese pass through untouched and French "météo" keeps its
                // accent. A trigger spelled the other way is a dead trigger, of
                // which this file has already had one.
                // es / fr / de / ru / zh
                "recuerda",
                // ACCENTED AND BARE, because `simplify` folds the Turkish
                // letters and NOTHING ELSE: "acuérdate" keeps its é, so a
                // trigger spelled without it can never fire on the sentence it
                // was written for.
                "acuérdate",
                "acuerdate",
                "apúntate",
                "apunta",
                "sobre mí",
                "记一下",
                "记下",
                "忘掉",
                "olvida",
                "apuntado",
                "retiens",
                "souviens",
                "oublie",
                "sur moi",
                "merk dir",
                "merke dir",
                "gemerkt",
                "vergiss",
                "uber mich",
                "запомни",
                "запомнил",
                "забудь",
                "обо мне",
                "记住",
                "记得",
                "忘了",
                "关于我",
                // MEMORY, the way it is asked: "unutma" (do not forget) and
                // "unut" (forget) point at the SAME tool — one writes a note,
                // the other removes it, and both are `remember`.
                "hatirla",
                "unutma",
                "unut",
                "aklinda tut",
                "not al",
                // "kaydet" STAYS HERE. It is also the verb for saving a file;
                // see the note under `Document`'s locale list for the two
                // sentences that decided which profile owns it and which owns
                // the word next to it.
                "kaydet",
            ],
            IntentProfile::Archive => &[
                // THE OTHER FIVE LANGUAGES, added when a natively-authored
                // benchmark in each of them was gated and the SAME FOUR TOOLS
                // fell out of the nine in every one: `remember`, `web_search`,
                // `archive` and `checksum`. That is not a coincidence — the
                // catalog puts exactly those four LAST, because none of them is
                // ever the right answer without an explicit trigger, and this
                // list had triggers in two languages only. In a third language
                // they could not be reached at all.
                //
                // SPELLED AS `simplify` LEAVES THEM. That function folds the
                // Turkish letters and lowercases, and touches nothing else — so
                // German "über" must be written "uber" here, while Cyrillic and
                // Chinese pass through untouched and French "météo" keeps its
                // accent. A trigger spelled the other way is a dead trigger, of
                // which this file has already had one.
                "arsiv",
                "comprimido",
                "descomprim",
                "decompress",
                "entpack",
                "dézipper",
                "dezipper",
                "archiv",
                "архив",
                "распакуй",
                "压缩包",
                "解压",
                // "arsiv" (which folds from "arşiv") IS ALREADY IN THIS LIST,
                // fifteen lines up. It was here a second time and
                // `score_intent` chains the two lists without dedup, so the word
                // was counted twice — `tacet why "arsiv"` printed
                // `archive 10 arsiv, arsiv`. The guard against a third is
                // `no_trigger_is_listed_twice_in_a_profile`.
                // "zipten" / "zipi": Turkish glues the case suffix onto the
                // borrowed noun, and bare "zip" is three characters, so it has
                // to match as a WHOLE term — "zipten" is not reachable from it.
                // The four-character forms match as prefixes and cover the rest
                // of the paradigm ("zipten", "zipteki", "zipi", "zipin").
                "zipte",
                "zipi",
                "sikistirilmis",
            ],
            IntentProfile::Integrity => &[
                // THE OTHER FIVE LANGUAGES, added when a natively-authored
                // benchmark in each of them was gated and the SAME FOUR TOOLS
                // fell out of the nine in every one: `remember`, `web_search`,
                // `archive` and `checksum`. That is not a coincidence — the
                // catalog puts exactly those four LAST, because none of them is
                // ever the right answer without an explicit trigger, and this
                // list had triggers in two languages only. In a third language
                // they could not be reached at all.
                //
                // SPELLED AS `simplify` LEAVES THEM. That function folds the
                // Turkish letters and lowercases, and touches nothing else — so
                // German "über" must be written "uber" here, while Cyrillic and
                // Chinese pass through untouched and French "météo" keeps its
                // accent. A trigger spelled the other way is a dead trigger, of
                // which this file has already had one.
                "birebir ayni",
                "identico",
                "idéntico",
                "byte a byte",
                "identique",
                "octet pour octet",
                "empreinte",
                "一模一样",
                "identisch",
                "prufsumme",
                "контрольн",
                "байт в байт",
                "одинаковые",
                "校验",
                "一样吗",
                "dogrula",
                "ozet degeri",
                "ayni dosya",
                "ayni mi",
                "bozulmus",
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
            // WHAT SEPARATES THE TWO TOOLS THAT SHARE THIS PROFILE. The profile
            // score is the same for both; the product with these hints is not.
            IntentProfile::Extract => &["filter", "search", "intent", "message", "classif"],
            // "db" IS THE TOOL'S WHOLE NAME, which is the strongest evidence the
            // router has (`NAME_WEIGHT` is 4x a description match) and the thing
            // this profile was missing: before it, `db` scored only on generic
            // description hints capped at `DESCRIPTION_CAP`, so any tool whose
            // NAME matched a fired profile beat it — which is how
            // `create_document` won a question about rows in a table.
            IntentProfile::Data => &[
                "db", "sql", "sqlite", "database", "rows", "records", "query",
            ],
            IntentProfile::Clipboard => &["clipboard", "paste", "copy"],
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
                // "create" AND "excel" EARN THEIR PLACE ON THE NAME SIDE.
                // Since the name and the description are priced apart (see
                // `tool_score`), a hint that appears in a tool's NAME is the
                // strongest evidence the router has — and `create_document` is
                // the only tool in the catalog whose name says what it does to
                // a document. Without "create" the three document tools were
                // separated only by their prose and `read_document` headed
                // every request to MAKE one.
                "create", "excel", "markdown",
            ],
            IntentProfile::Clock => &[
                // "time" AND "clock" NAME THE TOOL, and their absence was the
                // whole `calendar`-beats-`time` defect: `calendar` matched the
                // Time hint "calendar" in its own NAME while `time` matched
                // nothing in its own, so the diary won every question about the
                // clock.
                "time",
                "clock",
                "date",
                "weekday",
                "day",
                "difference",
            ],
            IntentProfile::Calendar => &[
                "calendar",
                "event",
                "meeting",
                "appointment",
                "reminder",
                "schedule",
            ],
            IntentProfile::Calc => &[
                "calculate",
                "code",
                "python",
                "run",
                "formula",
                "numeric",
                "arithmetic",
                "expression",
                // "write" AND "script" ARE HINTS, NOT TRIGGERS, and the
                // distinction is what makes them safe. The comment on the Calc
                // TRIGGER list rules bare "write" out because "write a report"
                // is document work — that argument is about the MESSAGE side.
                // Here the text being matched is the TOOL, and `write_code` is
                // the only tool in the catalog whose name carries both words.
                // Without them the Calc profile ordered its three tools
                // `calculate` > `run_code` > `write_code` by hint mass alone,
                // so a request to SAVE a script never put the saving tool near
                // the top.
                "write",
                "script",
            ],
            IntentProfile::Web => &[
                "web", "internet", "page", "address", "url", "search", "fetch", "online",
            ],
            IntentProfile::Repo => &["git", "commit", "branch", "repository", "diff"],
            IntentProfile::Files => &[
                // "search" IS DELIBERATELY ABSENT, and it was here for one
                // measurement. It is in `web_search`'s NAME, so with the name
                // weighted (see `tool_score`) every device-file question put the
                // INTERNET search tool second: "Export the product list into a
                // spreadsheet file" came back web_search at rank 2. `find_file`
                // needs no help from it — it matches "file" and "find" in its
                // own name already.
                "file",
                "folder",
                "find",
                "read",
                "list",
                "directory",
                "locate",
            ],
            IntentProfile::DocEdit => &["edit", "change", "modify", "append", "replace", "insert"],
            IntentProfile::Memory => &["memory", "remember", "forget", "recall", "note"],
            IntentProfile::Archive => &["archive", "zip", "unzip", "unpack", "extract"],
            IntentProfile::Integrity => &[
                // "checksum" AND "sha" CARRY THIS PROFILE FROM THE NAME SIDE:
                // `checksum` is the only tool whose NAME says what it does to a
                // file's identity, and a name match is worth `NAME_WEIGHT` times
                // a description one.
                "checksum",
                "sha",
                "hash",
                "digest",
                "fingerprint",
                // "verify" IS DELIBERATELY ABSENT. The only tool in the catalog
                // whose description contains it is `write_code`
                // ("Writes a code file for the user, VERIFIES it"), so a hint
                // here would score the code WRITER as an integrity tool on every
                // sentence about a checksum.
            ],
        }
    }
}

/// The profile scores coming out of a message; the highest-scoring profile is the
/// dominant intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntentScores {
    scores: Vec<(IntentProfile, usize)>,
    /// Did any WRITTEN TRIGGER fire, before the learned gate had its say?
    ///
    /// A SEPARATE QUESTION FROM "IS ANY SCORE NON-ZERO", and conflating them
    /// broke the MCP reservation. That reservation exists so a question in a
    /// language the trigger table has never seen can still reach a connected
    /// server, and it is gated on nothing having matched. The learned gate is
    /// explicitly a supplement for messages the table cannot reach — so letting
    /// its boost answer "did the table recognise this" turned the reservation
    /// off on precisely the messages it was written for.
    matched_by_trigger: bool,
}

impl IntentScores {
    /// Did a written trigger fire? See the field's note: this is not the same
    /// as "some score is non-zero" once the learned gate can add one.
    pub fn matched_by_trigger(&self) -> bool {
        self.matched_by_trigger
    }

    pub fn score(&self, profile: IntentProfile) -> usize {
        self.scores
            .iter()
            .find(|(p, _)| *p == profile)
            .map(|(_, s)| *s)
            .unwrap_or(0)
    }

    /// The dominant profile. If no trigger matched, `Files` — forcing an unknown
    /// message into a specific profile brings the wrong tool forward.
    ///
    /// `Files` INHERITS THE FALLBACK from the old `General`, and the choice is
    /// narrower than it looks: the ONE caller that reads this value is
    /// `Router::select`, which asks `scores.score(scores.dominant()) == 0` —
    /// "did anything fire at all". When nothing did, every profile scores zero
    /// and WHICH profile is named changes nothing about the selection. The
    /// variant here is a placeholder for "no intent", not a claim about the
    /// message.
    pub fn dominant(&self) -> IntentProfile {
        self.scores
            .iter()
            .filter(|(_, s)| *s > 0)
            // max_by_key picks THE LAST on a tie; the iterator is reversed so the
            // first profile in ALL wins — a determinism requirement.
            .rev()
            .max_by_key(|(_, s)| *s)
            .map(|(p, _)| *p)
            .unwrap_or(IntentProfile::Files)
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
    let mut scores: Vec<(IntentProfile, usize)> = scores;
    let matched_by_trigger = scores.iter().any(|(_, total)| *total > 0);

    // THE LEARNED HALF, AND IT ONLY ADDS. `slot_gate` is 48 KiB of int8 that
    // answers "is this a request for one of the two extraction tools" where the
    // trigger list above cannot: 58 of 105 benchmark requests reach neither
    // tool by substring, and the ones carrying no place noun at all cannot be
    // reached by a list without naming them one at a time, which is fitting the
    // router to its own test.
    //
    // IT RAISES A SCORE AND NEVER OVERRULES ONE. Everything else the router
    // decides is unchanged, so a wrong prediction costs one slot of the nine
    // rather than the right tool — and `eval --routing` is the guard that says
    // so, at 166/166 with this on.
    //
    // The boost is the length of a typical trigger, so a learned hit weighs
    // about what one written trigger does rather than swamping the profile.
    if crate::slot_gate::predict(message).is_some() {
        const LEARNED_BOOST: usize = 12;
        for (profile, total) in scores.iter_mut() {
            if *profile == IntentProfile::Extract {
                *total += LEARNED_BOOST;
            }
        }
    }
    IntentScores {
        scores,
        matched_by_trigger,
    }
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
    budget_override: Option<usize>,
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

    /// Measurement-only budget override: CAN exceed `MAX_TOOLS`.
    /// `0` means the whole catalog.
    pub fn budget_override(mut self, count: usize) -> Self {
        self.budget_override = Some(count);
        self
    }

    fn budget(&self, catalog_len: usize) -> usize {
        if let Some(b) = self.budget_override {
            if b == 0 { catalog_len } else { b }
        } else {
            self.max.unwrap_or(MAX_TOOLS)
        }
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
        let budget = self.budget(catalog.tools().len());
        // THE RESERVATION IS FOR SILENCE, NOT FOR COMPETITION. It exists so a
        // question in a language the trigger table has never seen can still
        // reach a connected server. When a profile DID fire, the message has
        // said what it is about and speculative remote tools should not be
        // pushing scoring ones out of the budget — measured on the same
        // exchange-rate question, where five of nine slots had gone to the
        // server.
        // THE WRITTEN TABLE, NOT THE TOTAL. The learned gate adds a score for
        // exactly the messages this reservation exists for — ones the trigger
        // table cannot reach — so asking "is any score zero" let the gate
        // silence the reservation on its own best cases.
        let nothing_matched = !scores.matched_by_trigger();
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
                // MAKE ROOM FROM THE TAIL, SKIPPING THE RESERVED — a blind
                // `truncate` deleted the very tools this block exists to keep.
                //
                // A reserved tool that EARNED a tail slot on merit is counted in
                // `already` (so `wanted` shrinks) and was then cut off by the
                // truncate, so the reservation delivered fewer slots than it
                // promised the moment merit and reservation agreed. MEASURED at
                // production geometry — 17 built-ins plus 29 MCP tools, budget
                // 9, three reserved slots — 1080 of 3772 non-scoring probe
                // messages got 2 of 3.
                let mut room = extra.len();
                let mut i = chosen.len();
                while room > 0 && i > 0 {
                    i -= 1;
                    if !self.reserved.iter().any(|n| n == chosen[i].name()) {
                        chosen.remove(i);
                        room -= 1;
                    }
                }
                chosen.extend(extra);
            }
        }

        // AN ADDRESS IN THE MESSAGE IS A FETCH, NOT A SEARCH.
        //
        // MEASURED on the model set: three of the twenty-seven failures were a
        // message carrying a URL — "Summarize https://example.com/…", "https://
        // news.ycombinator.com adresindeki başlıkları al" — answered with
        // `web_search`. Both tools were in the prompt and `web_fetch` was second,
        // so this is not a tool that never arrived; the model took the first
        // plausible one on the list. `tacet why` shows why it was second:
        // `web_search` and `web_fetch` share the Web profile, so they rise
        // together, and the tie goes to whichever tool's own text matched more
        // Web hints. Nothing in that arithmetic knows the message already
        // contains the address one of them needs.
        //
        // IT REORDERS AND NEVER ADDS OR DROPS, which is what keeps it safe: it
        // runs on the tools that already earned their place in the budget, so a
        // stray "http" in a sentence cannot pull an unrelated tool in, and no
        // tool that scored its way in is pushed out.
        //
        // IT IS KEYED ON THE SCHEMA, NOT ON A NAME. A tool qualifies by having a
        // REQUIRED field called `url` — the question being asked is "does this
        // tool want the thing the message is holding", and answering it by
        // matching the string "web_fetch" would leave the `http` addon, which
        // takes the same argument, behind for the same reason.
        if Self::carries_address(message) {
            let (addressed, rest): (Vec<_>, Vec<_>) = chosen
                .into_iter()
                .partition(|t| Self::takes_an_address(t.as_ref()));
            chosen = addressed.into_iter().chain(rest).collect();
        }
        chosen
    }

    /// Does the message hold a literal web address?
    ///
    /// THE THREE FORMS THE TRIGGER TABLE ALREADY TRUSTS, and no more. A looser
    /// test (a dot between two words, say) would fire on "report.md" and on
    /// every Turkish sentence ending in an abbreviation. These three are the
    /// same markers the Web profile scores on, so a message this returns true
    /// for has already scored as a web request — the reorder only settles WHICH
    /// web tool, never whether the message is one.
    ///
    /// It reads the raw message rather than the folded one on purpose: folding
    /// exists to make Turkish comparable, and it is what puts "url" inside
    /// "teşekkürler". None of the three below survives that kind of accident.
    fn carries_address(message: &str) -> bool {
        let lower = message.to_lowercase();
        ["http://", "https://", "www."]
            .iter()
            .any(|m| lower.contains(m))
    }

    /// Does this tool WANT an address — a required field called `url`?
    ///
    /// Asked of the schema rather than the name so that any tool taking the
    /// argument qualifies: `web_fetch` today, the `http` addon on the same
    /// footing, and an MCP server's page reader without a change here. Required,
    /// because a tool that merely accepts an optional url is not a tool the
    /// address is FOR.
    fn takes_an_address(tool: &dyn Tool) -> bool {
        tool.schema()
            .fields()
            .iter()
            .any(|f| f.required && f.name == "url")
    }

    /// A tool's score: for every profile the tool belongs to, the product of the
    /// score that profile got from the message and the length of the hints matched
    /// on the tool.
    ///
    /// A product was preferred over a sum: if the message contains no time phrase,
    /// the calendar tool scores 0 even if a hint matches. Summed, an unrelated tool
    /// would climb the list just for having a long name.
    ///
    /// THE NAME AND THE DESCRIPTION ARE NO LONGER ONE STRING, and this is the
    /// correction of a defect that had made the router hand the top of the list
    /// to the wrong tool on almost every file question. Measured with
    /// `tacet eval --routing` (the model-free routing set added for exactly
    /// this) on "Find the file about the budget.":
    ///
    ///   1. run_code    200      6. create_document 104
    ///   2. write_code  152      7. find_file        64   <- its own home turf
    ///
    /// The cause is arithmetic, not judgement. The hints were matched against
    /// `name + description` glued together and every match added its length, so
    /// a tool's score grew with the SIZE OF ITS PROSE. `run_code` has a
    /// thousand-character description that says, correctly, that it cannot open
    /// a FILE, cannot see a FOLDER, must not LIST from MEMORY and should not be
    /// used to WRITE one — five General hints, 25 characters, all of them
    /// earned by DENYING the thing the message asked for. `find_file`'s shorter
    /// description matched two, so the tool that cannot read files outscored
    /// the tool that finds them three to one.
    ///
    /// The fix separates the two texts and prices them differently:
    ///
    /// * THE NAME IS EVIDENCE. `find_file` is called find_file because that is
    ///   what it does; a name is chosen once and cannot pad itself. It is worth
    ///   `NAME_WEIGHT` times a description match.
    /// * THE DESCRIPTION IS A HINT, AND IT IS CAPPED. Above `DESCRIPTION_CAP`
    ///   the extra matches stop being evidence about the tool and start being
    ///   evidence about how much the author wrote. The cap is what makes the
    ///   score independent of prose length — which is the property that was
    ///   missing, not the weighting.
    ///
    /// NEITHER NUMBER IS A TUNING KNOB TO TASTE: both were chosen against the
    /// routing set and the effect is recorded in this file's git history. The
    /// cap has to sit near the length of a real hint match (a word or two) or
    /// it stops binding; the weight has to be large enough that ONE name match
    /// beats a description that matched everything.
    fn tool_score(&self, tool: &dyn Tool, scores: &IntentScores) -> usize {
        /// A hint found in the tool's NAME counts this many times one found in
        /// its description.
        const NAME_WEIGHT: usize = 4;
        /// The most a description may contribute, in matched characters.
        const DESCRIPTION_CAP: usize = 10;

        let name = simplify(tool.name());
        let description = simplify(tool.description());
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
                let in_name: usize = p
                    .tool_hints()
                    .iter()
                    .filter(|t| tacet_skills::matching::contains(&name, t))
                    .map(|t| t.len())
                    .sum();
                let in_description: usize = p
                    .tool_hints()
                    .iter()
                    .filter(|t| tacet_skills::matching::contains(&description, t))
                    .map(|t| t.len())
                    .sum();
                let hint = NAME_WEIGHT * in_name + in_description.min(DESCRIPTION_CAP);
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

    // --- The trigger table, as data ------------------------------------------

    /// A TRIGGER LISTED TWICE IS WEIGHTED TWICE, and nothing said so.
    ///
    /// `score_intent` chains `message_triggers` and `locale_triggers` and sums
    /// the length of everything that matched, with no dedup. Two words had
    /// drifted into both lists of their own profile — `arsiv` (twice inside
    /// Archive's locale list) and `euro` (in Web's message list AND its locale
    /// list, being spelled the same in both languages). `tacet why "arsiv"`
    /// printed `archive 10 arsiv, arsiv`: a silent double weight on one word,
    /// which is exactly the kind of thing that makes a hand-tuned table
    /// impossible to reason about.
    #[test]
    fn no_trigger_is_listed_twice_in_a_profile() {
        for profile in IntentProfile::ALL {
            let mut seen: Vec<&str> = Vec::new();
            for trigger in profile
                .message_triggers()
                .iter()
                .chain(profile.locale_triggers().iter())
            {
                assert!(
                    !seen.contains(trigger),
                    "profile `{}` lists `{trigger}` twice — `score_intent` chains \
                     the two lists without dedup, so the word is weighted twice",
                    profile.name()
                );
                seen.push(trigger);
            }
        }
    }

    /// A TRIGGER THAT CANNOT FIRE IS FALSE DOCUMENTATION.
    ///
    /// Five rules were file extensions written with the dot — `.md`, `.txt`,
    /// `.log`, `.py`, `.js`. `matching::contains` requires a non-alphanumeric
    /// character BEFORE the match, and a real filename puts a letter or a digit
    /// there: `budget-2026.md` has a `6`. All 137 extension occurrences across
    /// `benchmarks/` are of that shape, so none of the five had ever fired.
    /// Nothing depended on them — sibling phrase triggers added in the same
    /// commits carry those sentences — which is precisely why they survived: a
    /// dead rule costs nothing until someone tunes the list believing it works.
    ///
    /// The test is written over the whole table rather than over the five, so
    /// the next one is caught the day it is added.
    #[test]
    fn a_trigger_that_looks_like_a_file_extension_fires_on_a_real_filename() {
        for profile in IntentProfile::ALL {
            for trigger in profile
                .message_triggers()
                .iter()
                .chain(profile.locale_triggers().iter())
            {
                let Some(extension) = trigger.strip_prefix('.') else {
                    continue;
                };
                // The shape a filename actually has: a stem ending in a digit,
                // which is what defeats the boundary rule.
                let filename = format!("budget-2026.{extension}");
                assert!(
                    tacet_skills::matching::contains(&filename, trigger),
                    "profile `{}` lists `{trigger}`, which cannot match `{filename}` — \
                     `matching::contains` wants a non-alphanumeric character before the \
                     match and a filename stem ends in one. Drop the dot: the bare \
                     extension matches as a whole term.",
                    profile.name()
                );
            }
        }
    }

    /// AND THE REPLACEMENTS DO FIRE, on the filenames they were written for and
    /// not on the words they must stay out of.
    #[test]
    fn the_bare_extension_roots_match_a_filename_and_not_a_word() {
        for (extension, filename, innocent) in [
            (
                "md",
                "summarize budget-2026.md for me",
                "mdadm is a raid tool",
            ),
            ("txt", "read notes.txt", "the context of the message"),
            ("log", "show me server1.log", "logistics for the trip"),
            ("py", "run prime_numbers.py", "pyramid schemes"),
            ("js", "open bundle3.js", "jsonnet templates"),
        ] {
            assert!(
                tacet_skills::matching::contains(filename, extension),
                "`{extension}` must reach `{filename}`"
            );
            assert!(
                !tacet_skills::matching::contains(innocent, extension),
                "`{extension}` must not reach `{innocent}`"
            );
        }
    }

    // --- The two tools no profile could see ----------------------------------

    /// A CATALOG WITH THE ADDON TOOLS IN IT, whatever this machine has.
    ///
    /// `db` needs a `sqlite3`, `clipboard` needs a clipboard helper, and CI runs
    /// on hosts that have neither — but the router only ever reads a tool's
    /// NAME and DESCRIPTION, so a stand-in carrying the real ones measures the
    /// real thing. The description is a `pub const` in each module rather than a
    /// copy here, so the two cannot drift; when the real tool IS present it is
    /// used, which also proves the constant is what the tool returns.
    fn catalog_with_addons() -> ToolCatalog {
        let store = Arc::new(crate::data_store::SharedStore::new());
        let memory = crate::memory::SharedMemory::in_memory();
        let (mut catalog, _, _) = crate::catalog::production_catalog_gated(
            &store,
            &memory,
            Some(0),
            crate::catalog::AddonGates::all_open(),
        );
        if catalog.find("db").is_none() {
            catalog.add(tool("db", crate::db::DESCRIPTION));
        }
        if catalog.find("clipboard").is_none() {
            catalog.add(tool("clipboard", crate::clipboard::DESCRIPTION));
        }
        catalog
    }

    /// `tacet why "how many rows are in the users table of app.db"` LEFT `db`
    /// OFF THE LIST.
    ///
    /// Measured on a live install with the addon open: the message scored
    /// `document 5` on "table" and `calc 8` on "how many", `create_document`
    /// came first, and the database tool was in the "left out" line — where a
    /// tool cannot be called however well the model reasons. It had no trigger
    /// anywhere (nothing for sql/sqlite/database/veritabani/query/rows) and no
    /// name-side hint, so it scored only through generic description matches
    /// capped at `DESCRIPTION_CAP` and lost to any tool whose NAME matched a
    /// profile that had fired.
    ///
    /// THE PHRASINGS ARE THE PLAINEST ONES, deliberately: if a tool cannot be
    /// reached by the most obvious way of asking for its own job, the trigger
    /// table is not tuned, it is absent.
    #[test]
    fn the_database_tool_is_reachable_by_the_plainest_phrasings() {
        let catalog = catalog_with_addons();
        let router = Router::new();
        for message in [
            "how many rows are in the users table of app.db",
            "run a sql query on data/app.db",
            "list the records in the customers database",
            "what is in my sqlite file",
            "query the database for last month",
            // Turkish, through the locale list.
            "app.db dosyasindaki kayit sayisi ne",
            "veritabaninda kac kayit var",
        ] {
            let shown = router.select(message, &catalog);
            assert!(
                shown.iter().any(|t| t.name() == "db"),
                "`db` is not among the {} tools shown for {message:?}: {:?}",
                shown.len(),
                shown.iter().map(|t| t.name()).collect::<Vec<_>>()
            );
        }
    }

    /// The same hole, the same shape. `clipboard`'s description goes out of its
    /// way to tell the model to use it ONLY on an explicit request — an
    /// instruction that never arrived, because the tool never reached the
    /// prompt.
    #[test]
    fn the_clipboard_tool_is_reachable_when_the_user_names_it() {
        let catalog = catalog_with_addons();
        let router = Router::new();
        for message in [
            "read my clipboard",
            "what did i copy",
            "copy this to the clipboard",
            "paste what is on the clipboard",
            "panodaki metni oku",
            "kopyaladigim seyi yaz",
        ] {
            let shown = router.select(message, &catalog);
            assert!(
                shown.iter().any(|t| t.name() == "clipboard"),
                "`clipboard` is not among the {} tools shown for {message:?}: {:?}",
                shown.len(),
                shown.iter().map(|t| t.name()).collect::<Vec<_>>()
            );
        }
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
    fn no_more_than_max_tools_are_returned() {
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
            forget.score(IntentProfile::Memory) > 0,
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
            december.score(IntentProfile::Calendar) > 0,
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
    /// THE RESERVATION DELIVERS ITS SLOTS EVEN WHEN MERIT AGREES WITH IT.
    ///
    /// The old code made room with `chosen.truncate(budget - extra.len())`,
    /// which cuts the tail without looking at what is in it. A reserved tool
    /// that had EARNED a tail slot was counted in `already` — shrinking how many
    /// extras were fetched — and then deleted by the same truncate. So the
    /// reservation under-delivered exactly when the router and the reservation
    /// wanted the same tool, which is the case nobody thinks to test.
    ///
    /// The existing guard above checks the opposite direction (a reserved tool
    /// reachable from an unseen language) and cannot see this.
    #[test]
    fn a_reserved_tool_that_earned_its_slot_is_not_dropped_to_make_room() {
        // ORDER IS THE WHOLE TEST. With no profile firing every tool scores
        // zero and catalog order decides, so `srv_disk` lands in the LAST slot
        // of the budget — it earned its place, and the blind truncate that made
        // room for the other two reserved tools then deleted it.
        let mut catalog = tacet_kernel::ToolCatalog::new();
        for name in ["qqq_alpha", "qqq_beta", "qqq_gamma"] {
            catalog.add(Arc::new(FakeTool {
                name,
                description: "zzz yyy xxx.",
            }));
        }
        for name in ["srv_disk", "srv_net", "srv_proc"] {
            catalog.add(Arc::new(FakeTool {
                name,
                description: "zzz yyy xxx.",
            }));
        }
        let reserved: Vec<String> = ["srv_disk", "srv_net", "srv_proc"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let router = Router::new().max(4).reserving(reserved.clone());

        let picked: Vec<String> = router
            .select("ワードプロセッサの状態", &catalog)
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        let kept = picked.iter().filter(|n| reserved.contains(n)).count();
        assert_eq!(
            kept,
            RESERVED_SLOTS.min(reserved.len()),
            "the reservation promised {} slots and delivered {kept}: {picked:?}",
            RESERVED_SLOTS.min(reserved.len())
        );
    }

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
        //
        // The last two arrived with the `Data` profile and are the cleanest
        // possible example of why the boundary rule is not optional: the hint is
        // `db`, the tool's whole NAME, and the letters hide inside the word
        // "sandbox" — which `run_code` and `write_code` each use several times
        // to say the one thing they are about. Without term boundaries a hint
        // worth `NAME_WEIGHT` would fire on both code tools for every database
        // question. Nothing to fix on either side: the prose is right, the hint
        // is right, and the two have no relationship whatsoever.
        let recorded = [
            ("calendar", "write"),
            ("read_document", "sheet"),
            ("run_code", "db"),
            ("time", "write"),
            ("write_code", "db"),
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
        let short = score_intent("find");
        assert_eq!(long.score(IntentProfile::Calendar), "appointment".len());
        assert!(long.score(IntentProfile::Calendar) > short.score(IntentProfile::Files));
    }

    #[test]
    fn case_is_folded_for_scoring() {
        assert_eq!(
            score_intent("Show Tomorrow's MEETING").score(IntentProfile::Calendar),
            score_intent("show tomorrow's meeting").score(IntentProfile::Calendar)
        );
        assert!(score_intent("MEETING").score(IntentProfile::Calendar) > 0);
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
        assert!(score_intent("MEETİNG TOMORROW").score(IntentProfile::Calendar) > 0);
        assert_eq!(
            score_intent("MEETİNG TOMORROW").score(IntentProfile::Calendar),
            score_intent("meeting tomorrow").score(IntentProfile::Calendar)
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
        // Even if 50 is asked for, `MAX_TOOLS` is the cap.
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
            IntentProfile::Calendar
        );
        assert_eq!(
            score_intent("calculate the percent").dominant(),
            IntentProfile::Calc
        );
        // With no trigger at all it falls back to the placeholder profile —
        // see `dominant`: the only caller asks whether ANYTHING fired, so which
        // variant is named here changes no selection.
        assert_eq!(score_intent("zzz qqq").dominant(), IntentProfile::Files);
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

/// AN ADDRESS IN THE MESSAGE PUTS THE FETCHER FIRST.
///
/// MEASURED on the model set: three of twenty-seven failures were a message
/// carrying a URL answered with `web_search`. Both tools were in the prompt and
/// `web_fetch` was second, so the model took the first plausible one on the
/// list — which makes the ORDER of the nine, not their membership, the thing
/// that had to change.
#[cfg(test)]
mod address_first {
    use super::*;
    use crate::data_store::SharedStore;
    use crate::memory::SharedMemory;

    /// The production catalog with the web addon forced open, so `web_search`
    /// and `web_fetch` are both present regardless of the machine running the
    /// test — the same reason `production_catalog_with_gates` exists.
    fn web_catalog() -> tacet_kernel::ToolCatalog {
        let store = std::sync::Arc::new(SharedStore::new());
        let memory = SharedMemory::in_memory();
        let (catalog, _, _) =
            crate::catalog::production_catalog_with(&store, &memory, Some(0), true);
        catalog
    }

    fn order(message: &str) -> Vec<String> {
        Router::new()
            .select(message, &web_catalog())
            .into_iter()
            .map(|t| t.name().to_string())
            .collect()
    }

    fn rank(names: &[String], want: &str) -> usize {
        names
            .iter()
            .position(|n| n == want)
            .unwrap_or_else(|| panic!("{want} is not in {names:?}"))
    }

    #[test]
    fn a_url_in_the_message_puts_web_fetch_ahead_of_web_search() {
        for message in [
            "Summarize https://example.com/election-results",
            "https://news.ycombinator.com adresindeki başlıkları al",
            "read www.example.org and tell me what it says",
        ] {
            let names = order(message);
            assert!(
                rank(&names, "web_fetch") < rank(&names, "web_search"),
                "{message:?} carries an address, so the fetcher must come first: {names:?}"
            );
        }
    }

    /// NOT VACUOUS, and this is the assertion that keeps the rule narrow: a web
    /// question with no address must be unchanged, with `web_search` in front.
    /// A rule that promoted the fetcher always would pass the test above for
    /// free and be wrong here.
    #[test]
    fn a_web_question_without_an_address_still_leads_with_web_search() {
        for message in [
            "How much is the dollar today?",
            "Find flight schedules from London to Paris",
        ] {
            let names = order(message);
            assert!(
                rank(&names, "web_search") < rank(&names, "web_fetch"),
                "{message:?} names no page, so searching comes first: {names:?}"
            );
        }
    }

    /// The reorder must not change WHICH tools are in the budget — only their
    /// order. A rule that could drop a tool would be a routing regression
    /// wearing a fix's clothes.
    #[test]
    fn the_reorder_changes_the_order_and_not_the_membership() {
        let catalog = web_catalog();
        let with = Router::new().select("Summarize https://example.com/x", &catalog);
        let without = Router::new().select("Summarize the page about x", &catalog);
        assert_eq!(with.len(), without.len(), "the budget is unchanged");
        let mut a: Vec<&str> = with.iter().map(|t| t.name()).collect();
        let names_before = a.clone();
        a.sort_unstable();
        assert_eq!(
            a.len(),
            a.iter().collect::<std::collections::BTreeSet<_>>().len(),
            "no tool appears twice after partitioning: {names_before:?}"
        );
    }

    /// `carries_address` must not fire on the accidents the folded-text triggers
    /// already had to be defended from — "teşekkürler" contains "url", and a
    /// filename contains a dot. Neither is an address.
    #[test]
    fn a_thank_you_and_a_filename_are_not_addresses() {
        assert!(!Router::carries_address("Çok teşekkürler, harikaydı!"));
        assert!(!Router::carries_address("read report.md and summarise it"));
        assert!(!Router::carries_address("dosya türleri nelerdir"));
        assert!(Router::carries_address("HTTPS://EXAMPLE.COM"));
    }
}
