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
//! calendar would be both a privacy surprise and a false green on CI.

use crate::time::{DateTime, TimeResolver, local_offset_minutes};
use tacet_kernel::{
    ArgSchema, Field, ToolContext, ToolError, ToolFuture, ToolOutcome, ToolState, TraceUpdate,
    boxed,
};

pub struct CalendarTool {
    /// Test seam: the epoch "now" resolves against. `None` = the real clock.
    fixed_epoch: Option<i64>,
}

impl CalendarTool {
    pub fn new() -> Self {
        Self { fixed_epoch: None }
    }

    fn now(&self) -> DateTime {
        let epoch = self.fixed_epoch.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)
        });
        // The user's wall clock, the same convention as TimeTool.
        DateTime::from_epoch(epoch + local_offset_minutes().unwrap_or(0) * 60)
    }
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
fn date_build(var: &str, d: &DateTime) -> String {
    format!(
        "set {var} to current date\n\
         set year of {var} to {}\n\
         set month of {var} to {}\n\
         set day of {var} to {}\n\
         set hours of {var} to {}\n\
         set minutes of {var} to {}\n\
         set seconds of {var} to 0\n",
        d.year, d.month, d.day, d.clock, d.minute
    )
}

/// The script that lists one day's events, one per line: "HH:MM · title".
fn events_script(day: &DateTime) -> String {
    let mut start = day.start_of_day();
    start.clock = 0;
    start.minute = 0;
    format!(
        "{}set d2 to d1 + 1 * days\n\
         set out to {{}}\n\
         tell application \"Calendar\"\n\
         repeat with c in calendars\n\
         repeat with e in (every event of c whose start date is greater than or equal to d1 and start date is less than d2)\n\
         set end of out to (time string of (start date of e)) & \" · \" & (summary of e)\n\
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
        "Reads the user's calendar and creates reminders ON THIS DEVICE (macOS Calendar and \
         Reminders apps; nothing touches the network). kind='events' lists the events of one \
         day: put the day into 'day' in the user's own words ('today', 'tomorrow', 'yarin', \
         'onumuzdeki sali') — the date is resolved by code, never guess or rewrite it. \
         kind='remind' creates a reminder: 'title' is what to be reminded of, 'when' is the \
         moment in the user's own words ('tomorrow 9', 'yarin 18:30'). Call this for any \
         question about the user's schedule and any 'remind me' request, in any language. \
         NEVER answer schedule questions from memory: if this tool did not return an event, \
         it is not on the calendar."
    }

    fn schema(&self) -> ArgSchema {
        ArgSchema::object(vec![
            Field::new(
                "kind",
                ArgSchema::text().description("'events' to read a day, 'remind' to create a reminder."),
            )
            .required(),
            Field::new(
                "day",
                ArgSchema::text()
                    .description("Only for kind='events': the day, copied from the user's words. Default: today."),
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
            let kind = args.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let text_arg = |k: &str| {
                args.get(k)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string()
            };

            let trace = ctx.start_chip("calendar", "Looking at the calendar…");
            let outcome = match kind {
                "events" => {
                    let raw_day = {
                        let d = text_arg("day");
                        if d.is_empty() { "today".to_string() } else { d }
                    };
                    match TimeResolver::resolve(&raw_day, self.now()) {
                        None => ToolOutcome::failed(&ToolError::InvalidArgument(format!(
                            "the day '{raw_day}' was not understood — pass it exactly as the user said it"
                        ))),
                        Some(r) => match run_osascript(&events_script(&r.an)) {
                            Err(e) => ToolOutcome::failed(&ToolError::Io(std::io::Error::other(e))),
                            Ok(text) if text.trim().is_empty() => ToolOutcome::new(
                                "calendar read · empty day",
                                ToolState::Read,
                                format!(
                                    "no events on {:04}-{:02}-{:02}",
                                    r.an.year, r.an.month, r.an.day
                                ),
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
                    }
                }
                "remind" => {
                    let title = text_arg("title");
                    let when = text_arg("when");
                    if title.is_empty() || when.is_empty() {
                        ToolOutcome::failed(&ToolError::MissingField("title and when".into()))
                    } else {
                        match TimeResolver::resolve(&when, self.now()) {
                            None => ToolOutcome::failed(&ToolError::InvalidArgument(format!(
                                "the moment '{when}' was not understood — pass it exactly as the user said it"
                            ))),
                            Some(r) => match run_osascript(&reminder_script(&title, &r.an)) {
                                Err(e) => {
                                    ToolOutcome::failed(&ToolError::Io(std::io::Error::other(e)))
                                }
                                Ok(_) => ToolOutcome::new(
                                    format!("reminder set · {:02}:{:02}", r.an.clock, r.an.minute),
                                    ToolState::Written,
                                    format!(
                                        "reminder set: '{title}' at {:04}-{:02}-{:02} {:02}:{:02}",
                                        r.an.year, r.an.month, r.an.day, r.an.clock, r.an.minute
                                    ),
                                ),
                            },
                        }
                    }
                }
                other => ToolOutcome::failed(&ToolError::InvalidArgument(format!(
                    "kind '{other}' is not one of: events, remind"
                ))),
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

    fn day() -> DateTime {
        DateTime::new(2026, 7, 29, 15, 30, 0).expect("valid")
    }

    #[test]
    fn dates_are_injected_numerically_not_as_locale_literals() {
        let s = events_script(&day());
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
