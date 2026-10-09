//! The calendar/reminder bridge — macOS only, through `osascript`.
//!
//! WHY APPLESCRIPT AND NOT EVENTKIT: EventKit needs an Objective-C bridge and
//! an entitlement story; `osascript` talks to the same data through the
//! Calendar and Reminders APPS with zero new dependencies, and macOS's own
//! consent prompt (TCC) appears on first use — the OS asks the user, not us.
//! The trade is speed (AppleScript is slow on huge calendars) for a bridge
//! whose whole surface can be read on one screen.
//!
//! NO NETWORK, BUT PERSONAL: the events and the reminders never leave the
//! machine, and reading them still TAINTS the session — from that point on
//! anything that would push data out has to pass the approval gate, exactly
//! like `read_document`.
//!
//! DATES ARE BUILT NUMERICALLY inside the script (`set year of d to …`), never
//! as a date literal: AppleScript date literals parse in the user's LOCALE and
//! a Turkish system reads "07/28" differently than an American one. Numeric
//! fields cannot be misread.
//!
//! WHAT THE TESTS COVER AND WHAT THEY DO NOT (the addon/docker rule): the
//! script TEXT — escaping, the numeric date injection — is tested; actually
//! running `osascript` is not, because a unit test that opens the user's real
//! calendar would be both a privacy surprise and a false green on CI. The ONE
//! exception is the date arithmetic of `date_build`: it is run through
//! `osascript` where one exists, because it tells no application anything and
//! because a text assertion could not have caught the month rollover (M17) —
//! the script read correctly and AppleScript still put the date in May.

use crate::time::{
    DateTime, Resolution, TimeResolver, local_offset_minutes, local_offset_minutes_at,
};
use tacet_kernel::{
    ArgSchema, Field, ToolContext, ToolError, ToolFuture, ToolOutcome, ToolState, TraceUpdate,
    boxed,
};

pub struct CalendarTool {
    /// Test seam: the epoch "now" resolves against. `None` = the real clock.
    fixed_epoch: Option<i64>,
    /// Test seam: a fixed UTC offset in minutes. `None` = ask the system, per
    /// instant (see `zone`).
    fixed_offset: Option<i64>,
}

impl CalendarTool {
    pub fn new() -> Self {
        Self {
            fixed_epoch: None,
            fixed_offset: None,
        }
    }

    #[cfg(test)]
    fn at(epoch: i64, offset_minutes: i64) -> Self {
        Self {
            fixed_epoch: Some(epoch),
            fixed_offset: Some(offset_minutes),
        }
    }

    /// The user's UTC offset at the instant `utc`. Both the wall clock "now"
    /// and an explicit "…+03:00" go through here, so the two can no longer
    /// disagree about which zone the user is in.
    fn zone(&self, utc: i64) -> i64 {
        self.fixed_offset.unwrap_or_else(|| {
            local_offset_minutes_at(utc)
                .or_else(local_offset_minutes)
                .unwrap_or(0)
        })
    }

    fn now(&self) -> DateTime {
        let epoch = self.fixed_epoch.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)
        });
        // The user's wall clock, the same convention as TimeTool.
        DateTime::from_epoch(epoch + self.zone(epoch) * 60)
    }

    /// The user's words resolved to THEIR wall clock.
    ///
    /// AUDIT M18: this used to be `TimeResolver::resolve(raw, self.now())` —
    /// a LOCAL "now" handed to the form that converts explicit offsets to UTC.
    /// A +03:00 user asking for "2026-07-20T18:00+03:00" got a reminder at
    /// 15:00, three hours early, and the tool reported success.
    fn resolve(&self, raw: &str) -> Option<Resolution> {
        TimeResolver::resolve_in_zone(raw, self.now(), &|utc| self.zone(utc))
    }

    /// Everything `run` decides before it talks to `osascript`: the script and
    /// what it will report. Split out so the date that reaches the script can
    /// be tested without opening the user's calendar (see the module doc).
    fn plan(&self, args: &serde_json::Value) -> Result<Plan, ToolOutcome> {
        let kind = args.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let text_arg = |k: &str| {
            args.get(k)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string()
        };
        match kind {
            "events" => {
                let raw_day = {
                    let d = text_arg("day");
                    if d.is_empty() { "today".to_string() } else { d }
                };
                // CLAMPED, NOT REFUSED. The schema already bounds this, so a
                // value outside the range cannot arrive from a constrained
                // model; the clamp is for the unconstrained callers (a test,
                // a bridge) and costs one instruction.
                let span = args
                    .get("days")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(1)
                    .clamp(1, MAX_SPAN_DAYS);
                match self.resolve(&raw_day) {
                    None => Err(ToolOutcome::failed(&ToolError::InvalidArgument(format!(
                        "the day '{raw_day}' was not understood — pass it exactly as the user said it"
                    )))),
                    Some(r) => Ok(Plan::Events {
                        script: events_script(&r.an, span),
                        day: r.an,
                        span,
                    }),
                }
            }
            "remind" => {
                let title = text_arg("title");
                let when = text_arg("when");
                if title.is_empty() || when.is_empty() {
                    return Err(ToolOutcome::failed(&ToolError::MissingField(
                        "title and when".into(),
                    )));
                }
                match self.resolve(&when) {
                    None => Err(ToolOutcome::failed(&ToolError::InvalidArgument(format!(
                        "the moment '{when}' was not understood — pass it exactly as the user said it"
                    )))),
                    Some(r) => Ok(Plan::Remind {
                        script: reminder_script(&title, &r.an),
                        title,
                        at: r.an,
                    }),
                }
            }
            other => Err(ToolOutcome::failed(&ToolError::InvalidArgument(format!(
                "kind '{other}' is not one of: events, remind"
            )))),
        }
    }
}

/// What one call will do, decided before any process is spawned.
enum Plan {
    Events {
        script: String,
        day: DateTime,
        span: i64,
    },
    Remind {
        script: String,
        title: String,
        at: DateTime,
    },
}

impl Default for CalendarTool {
    fn default() -> Self {
        Self::new()
    }
}

/// AppleScript string escaping: backslash first, then the quote. The titles
/// come straight from the model's arguments — from the USER's words — and a
/// quote in a meeting name must not become script syntax.
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// `set …` lines that build an AppleScript date NUMERICALLY (see module doc).
///
/// `set day … to 1` COMES FIRST, and the order is the fix for audit M17. The
/// fields are set one at a time on `current date`, and every intermediate value
/// is a real date that AppleScript normalises: on 31 January, setting the
/// month to April gives "31 April", which rolls over to 1 May, and the day
/// then lands in May. Measured with `osascript`: base 2026-01-31, target
/// 2026-04-15 10:00 → 2026-05-15T10:00:00. Day 1 exists in every month of
/// every year, so with it set first no intermediate date can overflow — this
/// covers 29-31 February targets and a 29 February "now" moved to a common
/// year too.
fn date_build(var: &str, d: &DateTime) -> String {
    format!(
        "set {var} to current date\n\
         set day of {var} to 1\n\
         set year of {var} to {}\n\
         set month of {var} to {}\n\
         set day of {var} to {}\n\
         set hours of {var} to {}\n\
         set minutes of {var} to {}\n\
         set seconds of {var} to 0\n",
        d.year, d.month, d.day, d.clock, d.minute
    )
}

/// THE LONGEST SPAN ONE CALL MAY READ.
///
/// AppleScript walks every calendar and every event in the window, so the cost
/// is linear in the span and the bridge already has a 30-second timeout —
/// "what have I got this year" would spend it and return nothing. Thirty-one
/// days covers "this week", "next week" and "the rest of the month", which is
/// every range a person asks for in one breath.
pub const MAX_SPAN_DAYS: i64 = 31;

/// The script that lists `span` days of events from `day`, one per line.
///
/// FORMAT DEPENDS ON THE SPAN, and it has to. One day's events read fine as
/// "HH:MM · title"; the same lines over a week are unattributable — three
/// entries at 09:00 with no way to tell Monday from Thursday. Over a span the
/// date is prefixed, and the cost is paid only where it buys something.
///
/// THE SPAN USED TO BE FIXED AT ONE and there was no way to ask for more: the
/// schema had a single `day` and "what does my week look like" could only be
/// answered by calling the tool seven times, which the turn budget does not
/// allow. The `+ 1 * days` in this script was the whole limitation.
fn events_script(day: &DateTime, span: i64) -> String {
    let mut start = day.start_of_day();
    start.clock = 0;
    start.minute = 0;
    let span = span.clamp(1, MAX_SPAN_DAYS);
    let line = if span > 1 {
        "set end of out to (short date string of (start date of e)) & \" \" & \
         (time string of (start date of e)) & \" · \" & (summary of e)\n"
    } else {
        "set end of out to (time string of (start date of e)) & \" · \" & (summary of e)\n"
    };
    format!(
        "{}set d2 to d1 + {span} * days\n\
         set out to {{}}\n\
         tell application \"Calendar\"\n\
         repeat with c in calendars\n\
         repeat with e in (every event of c whose start date is greater than or equal to d1 and start date is less than d2)\n\
         {line}\
         end repeat\n\
         end repeat\n\
         end tell\n\
         set AppleScript's text item delimiters to linefeed\n\
         out as text\n",
        date_build("d1", &start)
    )
}

/// The script that creates one reminder at a resolved instant.
fn reminder_script(title: &str, at: &DateTime) -> String {
    format!(
        "{}tell application \"Reminders\" to make new reminder with properties {{name:\"{}\", remind me date:d1}}\n",
        date_build("d1", at),
        escape(title)
    )
}

/// How long ONE call may wait on the Calendar bridge.
///
/// IT WAS 30 SECONDS AND THAT WAS MEASURED AS THE WHOLE COST OF THE TOOL. On a
/// machine where the bridge does not answer — no consent granted, `Calendar.app`
/// not responding — `osascript` blocks forever, and the eval case
/// `calendar-day` came out at 39 s: 9.5 s of model and 30.1 s of nothing but
/// this deadline expiring. `osascript` run by hand on the same machine never
/// returned at all.
///
/// EIGHT SECONDS IS A TURN, NOT A PROMPT. The old comment justified 30 s with
/// the OS consent dialog, but a user who has to read a dialog and click it is
/// not going to finish inside any deadline that is also acceptable to sit
/// through when the answer is "this will never work". They get told which it was
/// (see `BRIDGE_SILENT`) and can ask again once permission is granted.
#[cfg(target_os = "macos")]
const CALL_DEADLINE: std::time::Duration = std::time::Duration::from_secs(8);

/// Has the bridge already failed to answer in THIS process.
///
/// WHY A LATCH RATHER THAN A DEADLINE PER CALL, and it is the difference between
/// a slow session and an unusable one: the failure is a property of the machine
/// (permission, or an app that is not talking), not of the request. Once it has
/// happened, every later call in the same session would pay the same wait to
/// learn the same thing — a suite with several calendar cases pays it once per
/// case. It is paid ONCE now, and everything after is refused immediately with
/// the diagnosis.
///
/// IT IS NEVER RESET. Granting permission mid-session is possible, and the price
/// of the latch is that the user has to start a new session for it to be
/// noticed. That is the same trade `RunCodeTool::discover` already makes for the
/// sandbox, and the message says so.
#[cfg(target_os = "macos")]
static BRIDGE_SILENT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(target_os = "macos")]
const BRIDGE_SILENT_NOTE: &str = "the calendar bridge did not answer, so it is not being asked again this session. \
     macOS may be waiting for permission — look for a consent dialog, or grant Calendar \
     and Reminders access in System Settings › Privacy & Security › Automation, then \
     start a new session.";

/// Runs `osascript` under `CALL_DEADLINE`, and refuses instantly once the bridge
/// has been shown to be silent.
#[cfg(target_os = "macos")]
fn run_osascript(script: &str) -> Result<String, String> {
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    if BRIDGE_SILENT.load(Ordering::Relaxed) {
        return Err(BRIDGE_SILENT_NOTE.into());
    }
    let mut child = std::process::Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("osascript could not start: {e}"))?;
    let deadline = Instant::now() + CALL_DEADLINE;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                // KILL AND THEN REAP. `kill` only sends the signal; without the
                // `wait` the child stays a zombie for the life of the process,
                // and a killed eval left four of them behind on this machine —
                // one per timed-out case.
                let _ = child.kill();
                let _ = child.wait();
                BRIDGE_SILENT.store(true, Ordering::Relaxed);
                return Err(BRIDGE_SILENT_NOTE.into());
            }
            Err(e) => return Err(format!("osascript failed: {e}")),
        }
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("osascript output could not be read: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("the Calendar bridge refused: {}", err.trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

#[cfg(not(target_os = "macos"))]
fn run_osascript(_script: &str) -> Result<String, String> {
    Err("the calendar bridge only exists on macOS today".into())
}

impl tacet_kernel::Tool for CalendarTool {
    fn name(&self) -> &str {
        "calendar"
    }

    fn description(&self) -> &str {
        // `kind` IS A CHOICE NOW, so the signature already lists its two values
        // and this text does not spell them out a second time. What it keeps is
        // the part a signature cannot carry: which argument each kind needs, and
        // that days are copied from the user's words rather than resolved here.
        "Reads the user's calendar and creates reminders ON THIS DEVICE (macOS Calendar and \
         Reminders apps; nothing touches the network). For 'events', put the day into 'day' \
         in the user's own words ('today', 'yarin', 'onumuzdeki sali') — the date is \
         resolved by code, never guess or rewrite it. For 'remind', 'title' is what to be \
         reminded of and 'when' is the moment in the user's own words ('yarin 18:30'). Call \
         this for any question about the user's schedule and any 'remind me' request, in any \
         language. NEVER answer schedule questions from memory: if this tool did not return \
         an event, it is not on the calendar."
    }

    fn schema(&self) -> ArgSchema {
        ArgSchema::object(vec![
            // A CLOSED SET WRITTEN AS ONE, which is the whole point of the
            // project applied to its own catalog. The body accepts exactly
            // `events` and `remind` and refuses everything else, but the schema
            // said `text` — so the grammar had nothing to compile into a literal
            // alternation and an invalid `kind` stayed REPRESENTABLE. The model
            // could spend a turn on `{"kind":"banana"}` and be told no
            // afterwards, which is precisely the shape this codebase exists to
            // make impossible. Every sibling with a closed action (`time`,
            // `git`, `remember`, `archive`, `clipboard`, `run_code`,
            // `write_code`) already uses `choice`.
            Field::new(
                "kind",
                ArgSchema::choice(["events", "remind"])
                    // THE VALUES ARE NOW IN THE SIGNATURE, so the sentence does
                    // not repeat them — `choice[events|remind]` is what the
                    // model reads, and spending prompt tokens saying it twice is
                    // what pushed the nine-tool block over its ceiling.
                    .description("Read a day, or create a reminder."),
            )
            .required(),
            Field::new(
                "day",
                ArgSchema::text()
                    .description("Only for kind='events': the day, copied from the user's words. Default: today."),
            ),
            // A BOUNDED INTEGER, NOT FREE TEXT, for the reason `kind` is a
            // `choice`: the grammar compiles the bound, so "the whole year"
            // cannot be written as an argument and refused afterwards.
            Field::new(
                "days",
                ArgSchema::integer()
                    .range(Some(1.0), Some(MAX_SPAN_DAYS as f64))
                    .description("Only for kind='events': how many days from `day`. Default 1; a week is 7."),
            ),
            Field::new(
                "title",
                ArgSchema::text().description("Only for kind='remind': what to be reminded of."),
            ),
            Field::new(
                "when",
                ArgSchema::text()
                    .description("Only for kind='remind': the moment, copied WORD FOR WORD from the user."),
            ),
        ])
        .description("Read the calendar or set a reminder on this device")
    }

    /// The calendar IS personal data: reading it taints the session, the same
    /// contract as `read_document`.
    fn taints_session(&self) -> bool {
        true
    }

    fn run<'a>(&'a self, args: serde_json::Value, ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move {
            if let Err(e) = self.schema().validate(&args) {
                return ToolOutcome::failed(&e);
            }
            let trace = ctx.start_chip("calendar", "Looking at the calendar…");
            let outcome = match self.plan(&args) {
                Err(refused) => refused,
                Ok(Plan::Events { script, day, span }) => match run_osascript(&script) {
                    Err(e) => ToolOutcome::failed(&ToolError::Io(std::io::Error::other(e))),
                    // THE SPAN IS IN THE SENTENCE, because "no events on
                    // 2026-09-07" after a question about the week is a true
                    // statement that answers a different question.
                    Ok(text) if text.trim().is_empty() => ToolOutcome::new(
                        "calendar read · empty",
                        ToolState::Read,
                        if span > 1 {
                            format!(
                                "no events in the {span} days from {:04}-{:02}-{:02}",
                                day.year, day.month, day.day
                            )
                        } else {
                            format!(
                                "no events on {:04}-{:02}-{:02}",
                                day.year, day.month, day.day
                            )
                        },
                    ),
                    Ok(text) => {
                        let count = text.lines().count();
                        ToolOutcome::new(
                            format!("calendar read · {count} events"),
                            ToolState::Read,
                            text,
                        )
                    }
                },
                Ok(Plan::Remind { script, title, at }) => match run_osascript(&script) {
                    Err(e) => ToolOutcome::failed(&ToolError::Io(std::io::Error::other(e))),
                    Ok(_) => ToolOutcome::new(
                        format!("reminder set · {:02}:{:02}", at.clock, at.minute),
                        ToolState::Written,
                        format!(
                            "reminder set: '{title}' at {:04}-{:02}-{:02} {:02}:{:02}",
                            at.year, at.month, at.day, at.clock, at.minute
                        ),
                    ),
                },
            };

            ctx.update_chip(
                trace,
                TraceUpdate::state(outcome.state.clone())
                    .text(outcome.chip_text.clone())
                    .raw_input(args.to_string()),
            );
            if !matches!(outcome.state, ToolState::Failed(_)) {
                ctx.taint();
            }
            outcome
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tacet_kernel::Tool;

    fn day() -> DateTime {
        DateTime::new(2026, 7, 29, 15, 30, 0).expect("valid")
    }

    #[test]
    fn dates_are_injected_numerically_not_as_locale_literals() {
        let s = events_script(&day(), 1);
        assert!(s.contains("set year of d1 to 2026"));
        assert!(s.contains("set month of d1 to 7"));
        assert!(s.contains("set day of d1 to 29"));
        assert!(
            s.contains("set hours of d1 to 0"),
            "events start at midnight"
        );
        assert!(
            !s.contains("date \""),
            "no locale-parsed date literal anywhere"
        );
    }

    /// A DAY WAS THE ONLY THING THIS TOOL COULD READ.
    ///
    /// The script said `d1 + 1 * days` and the schema had no way to say
    /// otherwise, so "what does my week look like" could only be answered by
    /// calling the tool seven times — which the turn budget does not allow, and
    /// which nothing in the description invited. The question simply had no
    /// answer.
    #[test]
    fn a_span_reads_more_than_one_day() {
        let one = events_script(&day(), 1);
        let week = events_script(&day(), 7);
        assert!(one.contains("d1 + 1 * days"));
        assert!(week.contains("d1 + 7 * days"));
        // THE FORMAT CHANGES WITH THE SPAN, and it has to: three entries at
        // 09:00 over a week are unattributable without the date.
        assert!(
            !one.contains("short date string"),
            "one day needs no date prefix and the tokens are not free"
        );
        assert!(
            week.contains("short date string"),
            "over a span, a bare HH:MM cannot be attributed to a day"
        );
    }

    /// The span is bounded in the SCHEMA — so a constrained model cannot write
    /// "the whole year" and be refused afterwards — and clamped in the script
    /// for the callers the grammar does not run through.
    #[test]
    fn the_span_is_bounded_where_the_grammar_can_see_it() {
        let schema = CalendarTool::new().schema();
        let signature = schema.short_signature();
        assert!(
            signature.contains("days"),
            "the model cannot pass what the signature does not name: {signature}"
        );
        assert!(
            events_script(&day(), 9_999).contains(&format!("d1 + {MAX_SPAN_DAYS} * days")),
            "an unconstrained caller must not be able to walk every calendar for a year"
        );
        assert!(events_script(&day(), 0).contains("d1 + 1 * days"));
        assert!(events_script(&day(), -5).contains("d1 + 1 * days"));
    }

    /// AUDIT M17, the text half: the day is parked on the 1st before the year
    /// and the month move, and only then set to its real value.
    #[test]
    fn the_day_is_parked_on_the_first_before_the_year_and_month_move() {
        let s = date_build("d1", &DateTime::new(2026, 4, 15, 10, 0, 0).unwrap());
        let at = |line: &str| {
            s.find(line)
                .unwrap_or_else(|| panic!("{line:?} missing from:\n{s}"))
        };
        let park = at("set day of d1 to 1\n");
        let year = at("set year of d1 to 2026\n");
        let month = at("set month of d1 to 4\n");
        let day = at("set day of d1 to 15\n");
        assert!(
            park < year && year < month && month < day,
            "a month-end `current date` rolls the month over unless the day is 1 first:\n{s}"
        );
    }

    /// AUDIT M17, MEASURED. The text assertion above cannot see the defect —
    /// the old script also read correctly — so where `osascript` exists the
    /// arithmetic is run. Pure date math: no application is addressed, nothing
    /// is read or written. `current date` is replaced by a fixed month-end
    /// "now" so the result does not depend on today's date.
    #[test]
    #[cfg(target_os = "macos")]
    fn a_month_end_now_does_not_roll_the_target_into_the_next_month() {
        let osascript = std::path::Path::new("/usr/bin/osascript");
        if !osascript.exists() {
            return;
        }
        // (base "now", target, what AppleScript must end on). Each base is a
        // day the target month does not have.
        let cases = [
            ((2026, 1, 31), (2026, 4, 15), "2026-4-15 10:5"),
            ((2026, 3, 31), (2026, 2, 28), "2026-2-28 10:5"),
            ((2026, 1, 31), (2028, 2, 29), "2028-2-29 10:5"),
            ((2028, 2, 29), (2027, 6, 30), "2027-6-30 10:5"),
        ];
        for ((by, bm, bd), (ty, tm, td), expected) in cases {
            let target = DateTime::new(ty, tm, td, 10, 5, 0).unwrap();
            // The base is set with the day parked too, so it is the base
            // that is fixed here and only the production lines are on trial.
            let base = format!(
                "set d1 to current date\nset day of d1 to 1\nset year of d1 to {by}\n\
                 set month of d1 to {bm}\nset day of d1 to {bd}\n"
            );
            let script = date_build("d1", &target).replacen("set d1 to current date\n", &base, 1)
                + "return ((year of d1) as text) & \"-\" & ((month of d1 as integer) as text) \
                   & \"-\" & ((day of d1) as text) & \" \" & ((hours of d1) as text) & \":\" \
                   & ((minutes of d1) as text)\n";
            let out = std::process::Command::new(osascript)
                .arg("-e")
                .arg(&script)
                .output()
                .expect("osascript runs");
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&out.stdout).trim(),
                expected,
                "base {by}-{bm}-{bd}, target {ty}-{tm}-{td}"
            );
        }
    }

    /// 2026-07-20 12:00:00 UTC.
    const NOW: i64 = 1_784_548_800;

    fn script_of(tool: &CalendarTool, args: serde_json::Value) -> String {
        match tool.plan(&args) {
            Ok(Plan::Events { script, .. } | Plan::Remind { script, .. }) => script,
            Err(o) => panic!("refused {args}: {}", o.chip_text),
        }
    }

    /// AUDIT M18: an explicit offset is resolved to the user's LOCAL wall
    /// clock. Measured before the fix: a +03:00 user's "18:00+03:00" came out
    /// as 15:00. Asserted on the script that would reach `osascript`, so the
    /// whole path from the argument to the date is covered.
    #[test]
    fn an_explicit_offset_becomes_the_users_wall_clock() {
        let tool = CalendarTool::at(NOW, 180);
        let remind = |when: &str| {
            script_of(
                &tool,
                serde_json::json!({"kind": "remind", "title": "call", "when": when}),
            )
        };
        let same_zone = remind("2026-07-20T18:00+03:00");
        assert!(same_zone.contains("set hours of d1 to 18\n"), "{same_zone}");
        let utc = remind("2026-07-20T15:00Z");
        assert!(utc.contains("set hours of d1 to 18\n"), "{utc}");
        // Across midnight the DAY moves as well: 23:30 at -02:00 is 04:30 the
        // next morning at +03:00.
        let events = script_of(
            &tool,
            serde_json::json!({"kind": "events", "day": "2026-07-20T23:30-02:00"}),
        );
        assert!(events.contains("set day of d1 to 21\n"), "{events}");
        // Text with no offset is already the user's wall clock.
        let plain = remind("2026-07-20 18:00");
        assert!(plain.contains("set hours of d1 to 18\n"), "{plain}");
    }

    /// With no test seam the offset is the machine's REAL one. On a UTC
    /// machine this cannot tell the old code from the new; on this one
    /// (+03:00) it could, and did.
    #[test]
    fn with_no_seam_the_real_local_offset_is_used() {
        if std::env::var_os("TACET_TZ_OFFSET").is_some() {
            return;
        }
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let tool = CalendarTool {
            fixed_epoch: Some(epoch),
            fixed_offset: None,
        };
        let utc = DateTime::from_epoch(epoch);
        let raw = format!("{}T{:02}:{:02}Z", utc.iso_date(), utc.clock, utc.minute);
        let offset = local_offset_minutes().unwrap_or(0);
        let expected = DateTime::from_epoch(epoch - i64::from(utc.second) + offset * 60);
        assert_eq!(tool.resolve(&raw).expect("resolves").an, expected, "{raw}");
    }

    /// AUDIT M9 through this tool: `day` and `when` are free text and reached
    /// the resolver's byte slice. A refusal, not a panic.
    #[test]
    fn a_multibyte_day_or_moment_is_refused_not_a_panic() {
        let tool = CalendarTool::at(NOW, 180);
        for raw in ["tonight-🎉", "2026-07-20T10:00+1é1"] {
            assert!(
                tool.plan(&serde_json::json!({"kind": "remind", "title": "x", "when": raw}))
                    .is_err()
            );
            assert!(
                tool.plan(&serde_json::json!({"kind": "events", "day": raw}))
                    .is_err()
            );
        }
    }

    #[test]
    fn titles_cannot_break_out_of_the_script_string() {
        let s = reminder_script("call \"mum\" \\ tonight", &day());
        assert!(s.contains("name:\"call \\\"mum\\\" \\\\ tonight\""));
        assert!(s.contains("set hours of d1 to 15"));
        assert!(s.contains("set minutes of d1 to 30"));
    }

    /// THE LATCH, MEASURED RATHER THAN REASONED ABOUT.
    ///
    /// The defect it closes was measured on this machine: `osascript` asking the
    /// Calendar bridge never returned, `run_osascript` waited out its deadline,
    /// and the eval case `calendar-day` came out at 39 s — 9.5 s of model, 30.1 s
    /// of expiry. With several calendar cases in a suite that cost is paid once
    /// PER CASE, because nothing remembered the previous answer.
    ///
    /// WHAT IS ASSERTED IS THE SHAPE, NOT THE CLOCK. A test that timed a real
    /// `osascript` would pass or fail by whether this particular Mac has granted
    /// Calendar permission, which measures the machine and not the code. So the
    /// latch is set directly and the claim is the one that matters: once the
    /// bridge has been silent, the next call returns WITHOUT spawning anything,
    /// and it returns the diagnosis rather than a bare error.
    #[test]
    #[cfg(target_os = "macos")]
    fn once_the_bridge_is_silent_it_is_not_asked_again() {
        use std::sync::atomic::Ordering;

        let restore = BRIDGE_SILENT.load(Ordering::Relaxed);
        BRIDGE_SILENT.store(true, Ordering::Relaxed);

        let started = std::time::Instant::now();
        let answer = run_osascript("return 1");
        let waited = started.elapsed();

        BRIDGE_SILENT.store(restore, Ordering::Relaxed);

        let Err(message) = answer else {
            panic!("the latch was set, so no call may reach osascript");
        };
        assert!(
            waited < std::time::Duration::from_millis(50),
            "the refusal took {waited:?} — the latch is meant to skip the wait, \
             not to shorten it"
        );
        assert!(
            message.contains("System Settings"),
            "the refusal must say how to fix it, not just that it failed: {message}"
        );
    }

    /// The budget a single call may spend. Pinned because it was 30 s and the
    /// number is the whole cost of a failing calendar turn — a change to it is a
    /// change to how long a user stares at nothing.
    #[test]
    #[cfg(target_os = "macos")]
    fn one_call_may_not_sit_for_a_whole_turn() {
        assert!(
            CALL_DEADLINE <= std::time::Duration::from_secs(8),
            "a calendar call may not out-wait the model that asked for it"
        );
    }
}
