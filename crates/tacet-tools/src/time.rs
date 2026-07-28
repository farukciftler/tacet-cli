//! The time tool — the current date/time + the FULL day difference between two dates.
//!
//! WHY "diff" IS A SEPARATE ACTION: a failure measured on the Swift side. With
//! no tool able to do calendar arithmetic the model did not leave the question
//! unanswered, it MADE THE ANSWER UP (it said "6 days" between 19 July and
//! 2 December). Saying "do not compute it yourself" without giving the ability
//! produces confident invention. The calc tool cannot solve this: it knows only
//!
//! AN UNRESOLVABLE TIME = A FAILURE. Swift's silent `?? now()` fallback was
//! REMOVED, because in 8 of 9 languages it created the wrong event: the model
//! saw "0 days" or today's date and took it for THE ANSWER. Here too every
//! unresolvable input comes back as an explicit error; no path silently falls back to "now".
//!
//! ZERO DEPENDENCY: chrono WAS NOT ADDED. All we need is the Gregorian
//! calendar <-> day count conversion; the two functions below (`to_day_count`,
//! `weekday_to_date`) do that, leap-year rules included, in ~20 lines. Pulling
//! in a thousands-of-lines dependency for the sake of a date library would
//! contradict the "everything is written by hand" identity.
//!
//! THE TIME ZONE IS EXPLICIT, NEVER GUESSED. The offset is GIVEN by the caller
//! and written out plainly in the output as the `tz=` field, so the model can
//! see a wrong zone and correct it.
//!
//! LEAVING THE DEFAULT AT UTC WAS A MISTAKE and it was fixed. The "do not
//! guess" rule was right but its application was wrong: the price of not
//! knowing the zone was telling the user a time 3 hours behind ("what time is
//! it" -> 07:07, really 10:07). A silent wrong answer is exactly what this file
//! avoids. The fix is not to guess but to ASK: `local_offset_minutes` reads the
//! offset from the operating system (`date +%z` on unix, PowerShell
//! `DateTimeOffset.Now.Offset` on Windows — both with daylight saving applied)
//! and stays on UTC if it cannot read it. The production catalog uses this;

use crate::router::simplify;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};
use tacet_kernel::{
    ArgSchema, Field, Tool, ToolContext, ToolError, ToolFuture, ToolOutcome, ToolState,
    TraceUpdate, boxed,
};

// ---------------------------------------------------------------------------
// Calendar arithmetic
// ---------------------------------------------------------------------------

/// From a Gregorian date to a day count relative to 1970-01-01 (Howard Hinnant's algorithm).
/// The leap-year rules live inside the divisions; there is no separate `is_leap`
/// branch, so the 400-year exception comes out right for free.
fn to_day_count(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// The inverse of `to_day_count`.
fn weekday_to_date(day_count: i64) -> (i64, u32, u32) {
    let z = day_count + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if month <= 2 { y + 1 } else { y }, month, day)
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// English weekday names. LANGUAGE-NEUTRAL OUTPUT: the model translates this
/// into the user's language. Localizing the output here made the model parrot
/// the text back in a multilingual flow.
const WEEKDAY_NAMES: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// A calendar instant (a wall clock read in a particular time zone).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateTime {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub clock: u32,
    pub minute: u32,
    pub second: u32,
}

impl DateTime {
    /// An invalid component (month 13, 31 February, hour 25) returns `None`.
    /// NO CLAMPING: rounding 31 February to 28 February silently turns the
    /// user's typo into a different date — exactly the silent drift we avoid.
    pub fn new(
        year: i64,
        month: u32,
        day: u32,
        clock: u32,
        minute: u32,
        second: u32,
    ) -> Option<Self> {
        if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
            return None;
        }
        if clock > 23 || minute > 59 || second > 59 {
            return None;
        }
        Some(Self {
            year,
            month,
            day,
            clock,
            minute,
            second,
        })
    }

    pub fn epoch(&self) -> i64 {
        to_day_count(self.year, self.month, self.day) * 86_400
            + self.clock as i64 * 3600
            + self.minute as i64 * 60
            + self.second as i64
    }

    pub fn from_epoch(second: i64) -> Self {
        let day_count = second.div_euclid(86_400);
        let remaining = second.rem_euclid(86_400);
        let (year, month, day) = weekday_to_date(day_count);
        Self {
            year,
            month,
            day,
            clock: (remaining / 3600) as u32,
            minute: ((remaining % 3600) / 60) as u32,
            second: (remaining % 60) as u32,
        }
    }

    /// The start of the day. REQUIRED in the diff computation: if the two ends
    /// are not reduced to the start of the day the time-of-day difference shifts
    pub fn start_of_day(&self) -> Self {
        Self {
            clock: 0,
            minute: 0,
            second: 0,
            ..*self
        }
    }

    pub fn day_number(&self) -> i64 {
        to_day_count(self.year, self.month, self.day)
    }

    /// 0 = Sunday. 1970-01-01 was a Thursday, hence the +4 shift.
    pub fn weekday(&self) -> u32 {
        (self.day_number() + 4).rem_euclid(7) as u32
    }

    pub fn weekday_name(&self) -> &'static str {
        WEEKDAY_NAMES[self.weekday() as usize]
    }

    pub fn iso_date(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    pub fn iso_time(&self) -> String {
        format!("{:02}:{:02}", self.clock, self.minute)
    }

    fn add_days(&self, day: i64) -> Self {
        let (year, month, g) = weekday_to_date(self.day_number() + day);
        Self {
            year,
            month,
            day: g,
            ..*self
        }
    }
}

// ---------------------------------------------------------------------------
// Time resolver
// ---------------------------------------------------------------------------

/// The resolved instant + whether the text carried an EXPLICIT time of day.
///
/// `has_clock` is carried separately because "tomorrow" and "tomorrow 14:00"
/// are two different things of the same type: in the first, 00:00 is a DEFAULT,
/// in the second it is DATA. A caller that cannot tell them apart turns a
/// day-level reminder into a midnight event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolution {
    pub an: DateTime,
    pub has_clock: bool,
}

/// The ordered resolver that extracts a date/time out of free text.
///
/// THE ORDER MATTERS and is arranged by narrowing precision: first the formats
/// with a single meaning (ISO), then language-neutral numeric patterns, and the
/// language-bound shorthands last. In the reverse order the standalone numbers
/// inside "2026-07-20" would be taken for a clock time in the shorthand branch.
pub struct TimeResolver;

impl TimeResolver {
    /// `now` is given from outside (testability + a single clock reading point).
    /// `now` is the wall clock read in the caller's time zone; the resolution
    /// comes back in the same zone.
    pub fn resolve(raw: &str, now: DateTime) -> Option<Resolution> {
        Self::resolve_with_offset(raw, now, 0)
    }

    /// `local_offset_min`: `now`'s offset relative to UTC. It is used ONLY when
    /// the input carries its own time zone ("...T18:00+03:00"); it is needed to
    /// convert that instant into the caller's wall clock. Text with no offset is
    /// already taken to be written in the caller's zone — the least surprising assumption.
    pub fn resolve_with_offset(
        raw: &str,
        now: DateTime,
        local_offset_min: i64,
    ) -> Option<Resolution> {
        let text = raw.trim();
        if text.is_empty() {
            return None;
        }
        // 1) ISO 8601 and language-neutral numeric patterns (both use the same
        //    resolver: the only difference is the separator and field order).
        if let Some(c) = resolve_absolute(text, local_offset_min) {
            return Some(c);
        }
        // 2) Relative-day shorthands ("tomorrow 14:00", "tuesday", "next week friday").
        if let Some(c) = relative_day_shorthand(text, now) {
            return Some(c);
        }
        // 3) Dates with a month name ("2 december 2026", "20 july").
        if let Some(c) = named_month_date(text, now) {
            return Some(c);
        }
        // 4) Named fixed days ("new year").
        if let Some(c) = named_weekday(text, now) {
            return Some(c);
        }
        // NO FALLBACK. Input that reaches here could not be resolved and the
        // caller MUST treat that as an error.
        None
    }

    /// Is there an explicit clock trace in the text ("18:00", "18.30", "6 pm")?
    pub fn clock_trace(text: &str) -> bool {
        let plain = simplify(text);
        if look_up_clock(&plain).is_some() {
            return true;
        }
        ["am", "pm", "oo", "os"].iter().any(|suffix| {
            plain
                .split(|c: char| !c.is_ascii_alphanumeric())
                .any(|p| p == *suffix || (p.len() > suffix.len() && p.ends_with(suffix)))
        })
    }
}

/// "2026-07-20T18:00:00+03:00", "2026-07-20 18:00", "20.07.2026", "20/07/2026 09:30".
fn resolve_absolute(text: &str, local_offset_min: i64) -> Option<Resolution> {
    let mut body = text;
    let mut external_offset_min: Option<i64> = None;

    if let Some(remaining) = body.strip_suffix('Z').or_else(|| body.strip_suffix('z')) {
        external_offset_min = Some(0);
        body = remaining;
    } else if let Some((remaining, offset)) = last_offset(body) {
        external_offset_min = Some(offset);
        body = remaining;
    }

    let (date_part, time_part) = if let Some(i) = body.find(['T', 't']) {
        (&body[..i], Some(body[i + 1..].trim()))
    } else if let Some(i) = body.find(' ') {
        (&body[..i], Some(body[i + 1..].trim()))
    } else {
        (body, None)
    };

    let (year, month, day) = date_piece(date_part.trim())?;

    let (clock, minute, second, has_clock) = match time_part {
        Some(s) if !s.is_empty() => {
            let (h, m, s) = time_piece(s)?;
            (h, m, s, true)
        }
        // With no clock time an external offset is meaningless too: something
        // like "2026-07-20+03:00" already breaks the date part and never lands here.
        _ => (0, 0, 0, false),
    };

    let an = DateTime::new(year, month, day, clock, minute, second)?;
    // If the input carries its own time zone, first convert to the absolute
    // instant, then read it back in the caller's zone.
    let an = match external_offset_min {
        Some(external) if has_clock => {
            DateTime::from_epoch(an.epoch() - external * 60 + local_offset_min * 60)
        }
        _ => an,
    };
    Some(Resolution { an, has_clock })
}

/// Splits off a trailing "+03:00" / "-0500" / "+03" offset. Looked for only in
/// text that has a 'T' separator: so a date like "20-07-2026" does not get its dash read as an offset.
fn last_offset(s: &str) -> Option<(&str, i64)> {
    let t = s.rfind(['T', 't'])?;
    let p = s[t..].rfind(['+', '-'])?;
    let emit = t + p;
    let sign: i64 = if s.as_bytes()[emit] == b'+' { 1 } else { -1 };
    let o = &s[emit + 1..];
    let (h, m) = if let Some((a, b)) = o.split_once(':') {
        (a, b)
    } else if o.len() == 4 {
        (&o[..2], &o[2..])
    } else if o.len() == 2 {
        (o, "0")
    } else {
        return None;
    };
    let h: i64 = h.parse().ok()?;
    let m: i64 = m.parse().ok()?;
    if h > 14 || m > 59 {
        return None;
    }
    Some((&s[..emit], sign * (h * 60 + m)))
}

/// "yyyy-MM-dd", "yyyy/MM/dd", "dd.MM.yyyy", "dd/MM/yyyy".
///
/// DAY/MONTH ORDER: wherever the four-digit piece sits, that is the year; the
/// remaining two pieces are read dd/MM (the same decision as Swift). An MM/dd
/// guess WAS NOT ADDED: "03/04" gives a valid date under both readings, so the
/// wrong reading succeeds silently — more dangerous than an unresolvable input.
fn date_piece(s: &str) -> Option<(i64, u32, u32)> {
    let ayr = s.chars().find(|c| *c == '-' || *c == '/' || *c == '.')?;
    let p: Vec<&str> = s.split(ayr).collect();
    if p.len() != 3
        || p.iter()
            .any(|x| x.is_empty() || !x.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let (year, month, day) = if p[0].len() == 4 {
        (p[0], p[1], p[2])
    } else if p[2].len() == 4 {
        (p[2], p[1], p[0])
    } else {
        return None;
    };
    Some((year.parse().ok()?, month.parse().ok()?, day.parse().ok()?))
}

/// "HH:mm" or "HH:mm:ss".
fn time_piece(s: &str) -> Option<(u32, u32, u32)> {
    let p: Vec<&str> = s.split(':').collect();
    if !(2..=3).contains(&p.len())
        || p.iter()
            .any(|x| x.is_empty() || !x.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let second = if p.len() == 3 { p[2].parse().ok()? } else { 0 };
    Some((p[0].parse().ok()?, p[1].parse().ok()?, second))
}

/// Whole-word containment on the SIMPLIFIED text. The Turkish shorthands need
/// it: "dun" (yesterday) is a substring of "dunya" (world) and "sali"
/// (tuesday) of names like "salih" — a `contains` there resolves dates out of
/// words that have nothing to do with time. Split on anything non-alphanumeric.
fn has_word(text: &str, word: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric()).any(|w| w == word)
}

/// Relative day + an optional clock time. "today", "tomorrow", "day after
/// tomorrow", "yesterday", weekday names ("tuesday", "next week friday") — and
/// their Turkish counterparts in the simplified (diacritic-folded) form:
/// "yarin", "obur gun", "bugun", "dun", "onumuzdeki sali", "haftaya cuma".
fn relative_day_shorthand(raw: &str, now: DateTime) -> Option<Resolution> {
    let text = simplify(raw);

    let day_offset = if text.contains("day after tomorrow") || text.contains("obur gun") {
        Some(2)
    } else if text.contains("tomorrow") || has_word(&text, "yarin") {
        Some(1)
    } else if text.contains("today") || has_word(&text, "bugun") {
        Some(0)
    } else if text.contains("yesterday") || has_word(&text, "dun") {
        Some(-1)
    } else {
        weekday_offset(&text, now)
    }?;

    let target = now.start_of_day().add_days(day_offset);

    // The clock time: first "18:00"/"18.30"; otherwise, because the day is
    // EXPLICIT, a standalone number can safely be read as an hour ("tomorrow 9").
    if let Some((h, m)) = look_up_clock(&text) {
        return DateTime::new(target.year, target.month, target.day, h, m, 0).map(|an| {
            Resolution {
                an,
                has_clock: true,
            }
        });
    }
    if let Some(h) = bare_clock(&text) {
        return DateTime::new(target.year, target.month, target.day, h, 0, 0).map(|an| {
            Resolution {
                an,
                has_clock: true,
            }
        });
    }
    Some(Resolution {
        an: target,
        has_clock: false,
    })
}

/// The day offset from a weekday name.
///
/// "tuesday" = the first tuesday AFTER TODAY (1..=7). Today is not included: in
/// a mid-day conversation "tuesday" usually means the future, and picking today
/// risks putting the event at an hour already past. A "next week/next" prefix adds a week.
///
/// The long names are tried first: "monday" also contains "mon", and in the
/// Turkish table this used to matter ("pazartesi" contains "pazar") — the order
/// is kept so a longer name is never silently swallowed by a shorter one.
fn weekday_offset(plain: &str, now: DateTime) -> Option<i64> {
    // English and Turkish (simplified form) in one table, longest first.
    // Matching is WHOLE-WORD (see `has_word`), so "cumartesi" no longer
    // depends on ordering to beat "cuma" — but the order is kept as
    // documentation of the old failure all the same.
    const WEEKDAYS: [(&str, u32); 14] = [
        ("cumartesi", 6),
        ("pazartesi", 1),
        ("wednesday", 3),
        ("carsamba", 3),
        ("persembe", 4),
        ("saturday", 6),
        ("thursday", 4),
        ("tuesday", 2),
        ("monday", 1),
        ("friday", 5),
        ("sunday", 0),
        ("pazar", 0),
        ("cuma", 5),
        ("sali", 2),
    ];
    let target = WEEKDAYS
        .iter()
        .find(|(name, _)| has_word(plain, name))
        .map(|(_, n)| *n)?;
    let today = now.weekday() as i64;
    let mut offset = (target as i64 - today).rem_euclid(7);
    if offset == 0 {
        offset = 7;
    }
    // "next tuesday" adds a week; Turkish differs by PHRASE, not by habit:
    // "onumuzdeki sali" means the COMING Tuesday (the base rule already gives
    // that), while "haftaya" / "gelecek hafta" mean next week's.
    if plain.contains("next week")
        || plain.contains("next")
        || plain.contains("haftaya")
        || plain.contains("gelecek hafta")
    {
        offset += 7;
    }
    Some(offset)
}

/// Named fixed days. For now only NEW YEAR'S DAY.
///
/// WHY IT IS SEPARATE AND WHY IT IS KEPT SHORT: "how many days until new year"
/// was a failure measured in the field — the resolver did not know "new year",
/// the tool returned "Date not understood", and on the second attempt the model
/// MADE THE DATE UP (it picked a 1 January in the past and said "0 days").
/// That is exactly the point where an unresolvable input pushes the model to invent.
///
/// The list IS NOT A HOLIDAY CALENDAR and will not become one: religious
/// holidays need the Hijri calendar and public holidays need a country and a
/// year; both are things that could be silently wrong here. New Year's Day is
/// something this file knows because it is a constant of the Gregorian calendar itself.
///
/// A 1 JANUARY IN THE PAST IS NEVER PICKED: unless today is 1 January the NEXT
/// new year is always taken; that is what the "how many days left" question
/// asks. If today is 1 January then today itself (0 days) is the right answer.
fn named_weekday(raw: &str, now: DateTime) -> Option<Resolution> {
    let plain = simplify(raw);
    if !(plain.contains("new year") || has_word(&plain, "yilbasi")) {
        return None;
    }
    let year = if now.month == 1 && now.day == 1 {
        now.year
    } else {
        now.year + 1
    };
    DateTime::new(year, 1, 1, 0, 0, 0).map(|an| Resolution {
        an,
        has_clock: false,
    })
}

/// A date with a month name: "2 december 2026", "20 july", "december 2".
///
/// If no year is given the current year is taken; if that date is IN THE PAST a
/// year is added. Rationale: this tool is mostly called on forward-looking
/// questions ("how many days left"); a user saying "3 january" in December does not mean last January.
fn named_month_date(raw: &str, now: DateTime) -> Option<Resolution> {
    const MONTHS: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    let plain = simplify(raw);
    let month = MONTHS
        .iter()
        .position(|a| plain.contains(a))
        .map(|i| i as u32 + 1)?;

    let clock = look_up_clock(&plain);
    // If a clock time was found its digits are removed from the text so they are not taken for a day/year.
    let clean = match clock {
        Some((h, m)) => plain
            .replace(&format!("{h}:{m:02}"), " ")
            .replace(&format!("{h}.{m:02}"), " "),
        None => plain.clone(),
    };

    let mut day = None;
    let mut year = None;
    for part in clean
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
    {
        match part.len() {
            1 | 2 if day.is_none() => day = part.parse::<u32>().ok(),
            4 if year.is_none() => year = part.parse::<i64>().ok(),
            _ => {}
        }
    }
    let day = day?;

    let (an, has_clock) = match clock {
        Some((h, m)) => (
            DateTime::new(year.unwrap_or(now.year), month, day, h, m, 0)?,
            true,
        ),
        None => (
            DateTime::new(year.unwrap_or(now.year), month, day, 0, 0, 0)?,
            false,
        ),
    };
    if year.is_none() && an.day_number() < now.day_number() {
        let forward = DateTime::new(an.year + 1, month, day, an.clock, an.minute, 0)?;
        return Some(Resolution {
            an: forward,
            has_clock,
        });
    }
    Some(Resolution { an, has_clock })
}

/// Scans the "18:00" / "18.30" pattern by hand (no regex dependency).
///
/// A guard against swallowing dates: if a separator and digits follow the
/// minutes ("20.07.2026") this is a date, not a clock time — it is skipped.
fn look_up_clock(plain: &str) -> Option<(u32, u32)> {
    let b = plain.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let emit = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let length = i - emit;
        if length <= 2 && i < b.len() && (b[i] == b':' || b[i] == b'.') {
            let min_start = i + 1;
            let mut j = min_start;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let looks_like_date = j < b.len() && (b[j] == b'.' || b[j] == b'/' || b[j] == b'-');
            if j - min_start == 2 && !looks_like_date {
                let clock: u32 = plain[emit..emit + length].parse().ok()?;
                let minute: u32 = plain[min_start..j].parse().ok()?;
                if clock <= 23 && minute <= 59 {
                    return Some((clock, minute));
                }
            }
            i = j;
        }
    }
    None
}

/// A standalone 1-2 digit number ("tomorrow 9"). Called only when the day is
/// EXPLICITLY determined; otherwise it would read every number as an hour.
fn bare_clock(plain: &str) -> Option<u32> {
    let b = plain.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let emit = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let previous_separator = emit > 0 && matches!(b[emit - 1], b':' | b'.' | b'/' | b'-');
        let next_separator = i < b.len() && matches!(b[i], b':' | b'.' | b'/' | b'-');
        if i - emit <= 2 && !previous_separator && !next_separator {
            let s: u32 = plain[emit..i].parse().ok()?;
            if s <= 23 {
                return Some(s);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// The tool
// ---------------------------------------------------------------------------

/// The kind of information the tool can return.
///
/// NOT FREE TEXT: the failure measured in Swift was that EVERY value other than
/// `kind.lowercased() == "diff"` silently fell through to "all" — a model that
/// wrote "difference" got the clock/date instead of the day difference. A
/// closed set makes it impossible at the grammar level for the model to invent a sixth value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Clock,
    Date,
    Weekday,
    All,
    Diff,
}

impl Kind {
    pub const ALL: [&'static str; 5] = ["clock", "date", "weekday", "all", "diff"];

    pub fn resolve(raw: &str) -> Option<Self> {
        match raw {
            "clock" => Some(Kind::Clock),
            "date" => Some(Kind::Date),
            "weekday" => Some(Kind::Weekday),
            "all" => Some(Kind::All),
            "diff" => Some(Kind::Diff),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// The local time zone
// ---------------------------------------------------------------------------

/// The local UTC offset (in minutes) READ from the operating system. `None` if it cannot be resolved.
///
/// WHY IT IS NEEDED: the "the zone is NEVER GUESSED" rule at the top of the file
/// stands, but leaving the default at UTC produced a WRONG ANSWER in the field —
/// at 10:07 the user asked "what time is it in turkey", the tool worked
/// correctly and returned "07:07". The model did not invent anything, the
/// tool's zone was wrong. So UTC was not a "safe default", it was a silent error.
///
/// NOT A GUESS, A QUESTION: the offset is not estimated here; the operating
/// system is asked. std does not give the local zone, reading `/etc/localtime`
/// wants a TZif parser (and applying the daylight-saving rule by hand would be
/// a new source of silent drift). `date +%z` gives the same information, WITH
/// DAYLIGHT SAVING APPLIED, in a single line. The result is still written out
/// EXPLICITLY in the output as `tz=`; if it is wrong the user sees it.
///
/// PLATFORM: there used to be a SINGLE FIXED unix path here (`/bin/date`).
/// No such file exists on Windows, `Command::output()` fails silently and the
/// function returns `None` — meaning that under a claim of "cross platform"
/// every Windows user fell back to UTC; exactly the silent wrong this function
/// was written to close. The candidate list is now built PER PLATFORM
/// (`offset_candidates`) and if none of them resolves it still returns `None` —
/// silently saying "probably +03" would be the very mistake we are avoiding.
///
/// `TACET_TZ_OFFSET` (signed minutes, e.g. `180`) OVERRIDES everything: in a
/// container or an environment with no `date`, the user keeps one explicit lever.
pub fn local_offset_minutes() -> Option<i64> {
    // The variable is read through `env_var` so that the "an empty value counts
    // as undefined" rule holds here too. A script that writes `TACET_TZ_OFFSET=`
    // to clear it must not try to parse the empty string and silently fall to
    // `None` — it must go straight to the system path.
    if let Some(raw) = tacet_kernel::env_var("TACET_TZ_OFFSET")
        && let Ok(min) = raw.to_string_lossy().trim().parse::<i64>()
        && min.abs() <= 14 * 60
    {
        return Some(min);
    }
    for (program, args) in offset_candidates() {
        let Ok(output) = std::process::Command::new(&program).args(&args).output() else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        if let Some(min) = parse_offset(String::from_utf8_lossy(&output.stdout).trim()) {
            return Some(min);
        }
    }
    None
}

/// The system commands that can print the offset in `+0300` form, IN ORDER OF PREFERENCE.
///
/// `cfg!(windows)` WAS USED, NOT `#[cfg(windows)]`, and that is a deliberate
/// decision: `cfg!` is a `bool`, so BOTH BRANCHES compile and type check on
/// every platform. Written with `#[cfg]` the Windows branch would never be
/// checked on this machine (nor in any `cargo check` run in this repo) and
/// would blow up on the first Windows build — that is exactly this repo's recurring failure.
///
/// UNIX: `date` may not be in `/bin` (a minimal container, NixOS), so both
/// paths are tried. The path is FIXED, `PATH` is not searched: `PATH` comes
/// from the caller's environment and can be poisoned.
///
/// WINDOWS: `.NET`'s `DateTimeOffset.Now.Offset` gives the real offset WITH
/// daylight saving APPLIED; the `zzz` format produces `+03:00`, and once
/// `Replace` drops the `:` the format is THE SAME as `date +%z`, so a single
/// parser suffices. PowerShell is again invoked from a FIXED path
/// (`%SystemRoot%\System32\...`), not from `PATH`. THIS BRANCH IS NOT MEASURED —
/// there is no Windows on this machine, no `rustup`, no cross compilation. If
/// it cannot resolve it returns `None` and the caller stays on UTC; so the
fn offset_candidates() -> Vec<(std::path::PathBuf, Vec<&'static str>)> {
    use std::path::PathBuf;
    if cfg!(windows) {
        // `[System.DateTimeOffset]::Now.ToString('zzz').Replace(':','')`
        // IT CONTAINS NO SPACES — deliberately, so there is no shell quoting trap.
        const EXPRESSION: &str = "[System.DateTimeOffset]::Now.ToString('zzz').Replace(':','')";
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
        vec![
            (
                PathBuf::from(&root).join("System32\\WindowsPowerShell\\v1.0\\powershell.exe"),
                vec!["-NoProfile", "-NonInteractive", "-Command", EXPRESSION],
            ),
            // The last chance on a machine with only PowerShell 7 installed.
            (
                PathBuf::from("C:\\Program Files\\PowerShell\\7\\pwsh.exe"),
                vec!["-NoProfile", "-NonInteractive", "-Command", EXPRESSION],
            ),
        ]
    } else {
        vec![
            (PathBuf::from("/bin/date"), vec!["+%z"]),
            (PathBuf::from("/usr/bin/date"), vec!["+%z"]),
        ]
    }
}

/// Minutes from the "+0300" / "-0530" form. `None` if the format is malformed.
fn parse_offset(s: &str) -> Option<i64> {
    let byte = s.as_bytes();
    if byte.len() != 5 || !byte[1..].iter().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let sign = match byte[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let clock: i64 = s[1..3].parse().ok()?;
    let minute: i64 = s[3..5].parse().ok()?;
    if clock > 14 || minute > 59 {
        return None;
    }
    Some(sign * (clock * 60 + minute))
}

/// The current date/time and day difference tool.
///
/// Stateless and does not use the network. `fixed_epoch` is only for
/// test/eval: a test that depends on the real clock cannot be deterministic.
pub struct TimeTool {
    offset_minutes: i64,
    fixed_epoch: Option<i64>,
}

impl Default for TimeTool {
    fn default() -> Self {
        Self::new()
    }
}

impl TimeTool {
    /// The default zone is UTC. See the file header: the zone is NEVER guessed.
    pub fn new() -> Self {
        Self {
            offset_minutes: 0,
            fixed_epoch: None,
        }
    }

    /// The offset in minutes relative to UTC (e.g. 180 for Turkey).
    pub fn offset_minutes(mut self, minute: i64) -> Self {
        self.offset_minutes = minute;
        self
    }

    /// A fixed "now" — for eval and unit tests.
    pub fn fixed_epoch(mut self, epoch: i64) -> Self {
        self.fixed_epoch = Some(epoch);
        self
    }

    /// The wall clock in the caller's zone.
    pub fn now(&self) -> DateTime {
        let epoch = self.fixed_epoch.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                // If the system clock is set before 1970 we fall back to the
                // epoch instead of panicking: one tool call must not bring the whole flow down.
                .unwrap_or(0)
        });
        DateTime::from_epoch(epoch + self.offset_minutes * 60)
    }

    fn tz_text(&self) -> String {
        let sign = if self.offset_minutes < 0 { '-' } else { '+' };
        let m = self.offset_minutes.abs();
        format!("UTC{sign}{:02}:{:02}", m / 60, m % 60)
    }

    /// Language-neutral instant information. The model translates it into the user's language.
    ///
    /// `Weekday` NOW CARRIES THE DATE TOO. The rationale is a measurement: on
    /// "what day of the month is it" Qwen3-4B insistently picks `kind='weekday'`
    /// — even with the distinction written out plainly in the tool description.
    /// The word "day" means both "day of the week" and "day of the month" and
    /// the model does not separate them. Fixing the choice with the prompt was
    /// tried and did not hold; what held was REMOVING THE COST OF THE WRONG
    /// CHOICE: `weekday=Tuesday date=2026-07-21` answers both questions. One token more, one invention less.
    ///
    /// `Date` is left plain — whoever asks for the date already gets the date;
    /// there is no failure on the inflated side.
    pub fn now_text(&self, kind: Kind) -> String {
        let an = self.now();
        let (clock, date, day, tz) = (
            an.iso_time(),
            an.iso_date(),
            an.weekday_name(),
            self.tz_text(),
        );
        match kind {
            Kind::Clock => format!("time={clock} tz={tz}"),
            Kind::Date => format!("date={date}"),
            Kind::Weekday => format!("weekday={day} date={date}"),
            // Diff does not reach here (the caller splits it off) but we do not
            // leave a silent `_` arm: when a new variant is added the compiler should warn.
            Kind::All | Kind::Diff => {
                format!("time={clock} date={date} weekday={day} tz={tz}")
            }
        }
    }

    /// The FULL day count between today and the target. Both ends are reduced to the start of the day.
    /// Returns NEGATIVE for a date in the past — the sign is deliberately kept
    /// so the model does not have to invent the direction.
    pub fn diff_text(&self, target_raw: &str) -> Result<String, String> {
        let now = self.now();
        // The tool's offset is told to the resolver so it can convert ISO input
        // with an external offset into the local wall clock.
        let Some(resolution) =
            TimeResolver::resolve_with_offset(target_raw, now, self.offset_minutes)
        else {
            // NO SILENT FALLBACK TO TODAY: the model sees "0 days" and takes it for the answer.
            return Err(format!(
                "error: unparsable_date \"{target_raw}\". Nothing was computed. \
                 Call the tool again with \"target\" as an ISO 8601 date, e.g. 2026-12-02."
            ));
        };
        let today = now.start_of_day();
        let target = resolution.an.start_of_day();
        Ok(format!(
            "from={} to={} days={}",
            today.iso_date(),
            target.iso_date(),
            target.day_number() - today.day_number()
        ))
    }
}

impl Tool for TimeTool {
    fn name(&self) -> &str {
        "time"
    }

    fn description(&self) -> &str {
        // THIS IS THE ONLY TEXT THE MODEL SEES. The field descriptions in the
        // schema DO NOT ENTER the prompt — the prompt lists the tools in a short
        // signature form (`time(kind: 'clock'|..., target?: text)`), so every
        // rule written into a field description NEVER REACHES the model. Two
        // field failures came out of this and their fixes therefore live here:
        //
        //   * "what time is it in turkey" -> the model took the question for a
        //     geography question and invented the time. The device's clock is
        //     the user's clock; a country name does not change the question.
        //   * "what day of the month is it" -> the model picked kind='weekday'
        //     and returned the name of the weekday ("Tuesday"), while the user
        //     was asking which day of the month it was.
        //   * "how many days until new year" -> the model turned the target into
        //     "1 January 2025" by itself (a year left over from the training
        //     data) and got -566 days. A model that does not know the current
        //     year adding a year is a silent invention made in front of the resolver.
        //
        // THE "I DO NOT HAVE TIMETABLES" SENTENCE WAS TRIED AND REVERTED — the
        // record stays here so it is not tried again next time.
        //
        // The problem was real: on "what are the ortakoy uskudar ferry times"
        // the model picked this tool and told the user the wall clock. The fix
        // that was tried added five lines to the end of the description: "It
        // nothing else: ... ferry, bus, train and flight departure times ...
        // are NOT on this device".
        //
        // MEASURED ON THE REAL MODEL: the ferry question got better but "what is
        // the weather in istanbul" BROKE — instead of a valid call the model
        // started producing something XML-like, `<web_search>query:
        // ...</web_search>`, in all three runs (sampling off, so this is not
        // noise). The call never ran and the raw text spilled to the user.
        // Reverting the addition removed the failure too; verified one by one with a bisect.
        //
        // THE LESSON is the same as item 5 of `SYSTEM_INSTRUCTION` one layer
        // down: in a model this size a tool description also behaves GLOBALLY,
        // NOT LOCALLY. Lengthening one tool's description can break the FORMAT
        // of a call produced for ANOTHER tool. Descriptions must stay short.
        //
        // The distinction now lives in the router and costs nothing:
        // "ferry/service/timetable/times" phrases pull the message into the Web
        // profile, `web_search` moves to the head of the list and `time` goes
        // down (see `router.rs`, Web triggers). The choice was fixed not with
        // the prompt but with the ORDER — in a small model order is the probability of choice.
        "Gives the current date/time/day of week, and counts days between today and another \
         date. You do NOT know the current time or date on your own; without this tool any \
         answer you give is a guess. Call it whenever the user asks what time it is, what \
         today's date is or what day it is - in any language, and also when they name a \
         country or city, because the device clock is the user's clock. Pick kind='clock' for \
         clock time, kind='date' for the calendar date - that includes \"what day of the \
         month is it\" - and kind='weekday' only when the weekday NAME is asked. For \"how \
         many days until X\" use kind='diff' and copy X into 'target' WORD FOR WORD from the \
         user ('new year', 'tomorrow', '2 december'); never rewrite it and never add a year, \
         you do not know the current year. NEVER compute a date difference yourself: calendar \
         arithmetic needs leap years and month lengths, so it must be calculated here."
    }

    fn schema(&self) -> ArgSchema {
        ArgSchema::object(vec![
            Field::new(
                "kind",
                ArgSchema::choice(Kind::ALL).description(
                    // "what day of the month" -> the 'weekday' FAILURE WAS
                    // MEASURED: the model saw the word "day" and returned the
                    // weekday, while the user was asking which day of the month
                    // they were on. The distinction is now given with an example
                    // question.
                    "What to return: 'clock' = clock time (\"what time is it\"), 'date' = \
                     calendar date, use it for \"what day of the month is it\" and \"today's \
                     date\" too, 'weekday' = name of the weekday only (\"what day are we \
                     on\"), 'all' = all three, 'diff' = days until/since the date given in \
                     'target'. If unsure use 'all'.",
                ),
            )
            .required(),
            Field::new(
                "target",
                ArgSchema::text().description(
                    // "DO NOT add a year" WAS MEASURED: on "how many days until
                    // new year" the model turned the target into "1 January
                    // 2025" by itself — a year left over from the training data —
                    // and the tool ran correctly and returned -566 days. A model
                    // that does not know the current year adding a year is a
                    // silent invention made in front of the resolver. The
                    // resolver already understands relative phrases.
                    "Only for kind='diff': the other date, copied WORD FOR WORD from the \
                     user's message ('new year', 'tomorrow', '2 december', '2026-12-02'). Do \
                     NOT rewrite it and do NOT add a year - you do not know what year it is, \
                     this tool does. Leave empty otherwise.",
                ),
            ),
        ])
    }

    /// Reading the clock is not reading personal data; it does not taint the session.
    fn taints_session(&self) -> bool {
        false
    }

    fn run<'a>(&'a self, args: Value, ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move {
            let Some(kind_raw) = args.get("kind").and_then(|v| v.as_str()) else {
                return ToolOutcome::failed(&ToolError::MissingField("kind".into()));
            };
            let Some(kind) = Kind::resolve(kind_raw) else {
                return ToolOutcome::failed(&ToolError::InvalidArgument(format!(
                    "unknown kind \"{kind_raw}\""
                )));
            };

            // AN INSTANT ANSWER DROPS A CHIP TOO. It used to not ("unimportant,
            // it clutters the flow"); the cost of that became visible in a user
            // session: when the model INVENTED the time and when it relayed the
            // tool's CORRECT answer the screen looked exactly the same, so there
            // was NO WAY to tell which had happened. The "Tacet does not hide
            // what it does" principle exists precisely for this; the chip costs
            // one line, its absence costs trust.
            if kind != Kind::Diff {
                let output = self.now_text(kind);
                let trace = ctx.start_chip("clock", "Device clock read");
                ctx.update_chip(
                    trace,
                    TraceUpdate::state(ToolState::Read).text(output.clone()),
                );
                return ToolOutcome::read_ok("", output.clone()).raw_output(output);
            }

            // "diff" DROPS A CHIP: it produces a NUMBER and that number rests on
            // a parsed input, so the date may have been read wrong. The chip
            // detail shows "from=... to=... days=..."; the user catches a wrong
            // parse. Hiding a number that needs verifying would break the
            // "Tacet does not hide what it does" principle.
            let target = args
                .get("target")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let trace = ctx.start_chip("calendar", "Counting days");

            if target.is_empty() {
                let error = ToolError::MissingField("target".into());
                ctx.update_chip(
                    trace,
                    TraceUpdate::state(ToolState::Failed(error.short_error()))
                        .text("No date given"),
                );
                return ToolOutcome::failed(&error);
            }

            match self.diff_text(target) {
                Ok(output) => {
                    ctx.update_chip(
                        trace,
                        // THE CHIP TEXT CARRIES THE NUMBER. Writing "day
                        // difference computed" would hide exactly the number
                        // that needs verifying: the user could not compare the
                        // model's answer with the tool's result. In the field the
                        // model said "566 days" for a 164-day result and there
                        // was no trace on screen to catch it.
                        TraceUpdate::state(ToolState::Read)
                            .text(output.clone())
                            .raw_input(target)
                            .raw_output(output.clone()),
                    );
                    ToolOutcome::read_ok("Day difference computed", output.clone())
                        .raw_output(output)
                }
                // DELIBERATELY NOT `failed()`: core's fixed ERROR_MODEL_TEXT is
                // intentionally silent for internal malfunctions. Here the
                // situation is a recoverable INPUT problem; the model must learn
                // what to do (call again with ISO 8601), otherwise it keeps
                // coming back with the same unresolvable text.
                Err(message) => {
                    let chip = "Date not understood";
                    ctx.update_chip(
                        trace,
                        TraceUpdate::state(ToolState::Failed(chip.into()))
                            .text(chip)
                            .raw_input(target)
                            .raw_output(message.clone()),
                    );
                    ToolOutcome::new(chip, ToolState::Failed(chip.into()), message.clone())
                        .raw_output(message)
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::task::{Context, Poll};
    use tacet_kernel::{InMemoryDataStore, SilentReporter};

    /// 2026-07-20 12:00:00 UTC — a Monday.
    const NOW: i64 = 1_784_548_800;

    fn an(year: i64, month: u32, day: u32, clock: u32, minute: u32) -> DateTime {
        DateTime::new(year, month, day, clock, minute, 0).expect("a valid instant")
    }

    /// The Turkish shorthands resolve like their English twins. NOW is a
    /// Monday: "onumuzdeki sali" (the coming tuesday) is +1 day, "haftaya
    /// sali" (next week's) is +8, and short tokens only fire as WHOLE WORDS —
    /// "dunya" (world) must not read as "dun" (yesterday).
    #[test]
    fn turkish_shorthands_resolve() {
        let r = TimeResolver::resolve("yarin 9", now()).expect("yarin");
        assert_eq!((r.an.day, r.an.clock, r.has_clock), (21, 9, true));

        let r = TimeResolver::resolve("onumuzdeki sali 15:00", now()).expect("sali");
        assert_eq!((r.an.day, r.an.clock, r.has_clock), (21, 15, true));

        let r = TimeResolver::resolve("haftaya sali", now()).expect("haftaya");
        assert_eq!(r.an.day, 28);

        let r = TimeResolver::resolve("obur gun", now()).expect("obur gun");
        assert_eq!(r.an.day, 22);

        let r = TimeResolver::resolve("cumartesi", now()).expect("cumartesi");
        assert_eq!(r.an.day, 25);

        assert!(TimeResolver::resolve("dunya haritasi", now()).is_none());
        assert!(TimeResolver::resolve("yılbaşı", now()).is_some());
    }

    fn now() -> DateTime {
        an(2026, 7, 20, 12, 0)
    }

    /// there is no tokio (we do not pull in a network/executor dependency); a
    /// minimal poll loop with `Waker::noop` is enough — our tools do not rely on a real wake-up.
    fn run_and_wait(gelecek: ToolFuture<'_>) -> ToolOutcome {
        let mut gelecek = gelecek;
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        loop {
            if let Poll::Ready(outcome) = gelecek.as_mut().poll(&mut cx) {
                return outcome;
            }
        }
    }

    fn context() -> ToolContext {
        ToolContext::new(
            Arc::new(InMemoryDataStore::new()),
            ".",
            Arc::new(SilentReporter),
        )
    }

    #[test]
    fn now_is_monday_and_the_round_trip_is_consistent() {
        let a = DateTime::from_epoch(NOW);
        assert_eq!(a.iso_date(), "2026-07-20");
        assert_eq!(a.iso_time(), "12:00");
        assert_eq!(a.weekday_name(), "Monday");
        assert_eq!(a.epoch(), NOW);
        // A round trip over a wide range including leap years and century boundaries.
        for day in [-25_000_i64, -1, 0, 1, 19_000, 20_500, 40_000] {
            let e = day * 86_400 + 3661;
            assert_eq!(DateTime::from_epoch(e).epoch(), e, "gun={day}");
        }
        assert!(is_leap(2024) && !is_leap(2100) && is_leap(2000));
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2026, 2), 28);
    }

    #[test]
    fn iso_8601_is_resolved() {
        let c = TimeResolver::resolve("2026-12-02T18:30:00", now()).expect("iso");
        assert_eq!(c.an, an(2026, 12, 2, 18, 30));
        assert!(c.has_clock);

        // ISO with no clock time: the time of day is a DEFAULT, not DATA.
        let g = TimeResolver::resolve("2026-12-02", now()).expect("iso date");
        assert_eq!(g.an, an(2026, 12, 2, 0, 0));
        assert!(!g.has_clock);

        // Z and an explicit offset: since the tool is on UTC, +03:00 is read three hours back.
        assert_eq!(
            TimeResolver::resolve("2026-12-02T18:30:00Z", now())
                .unwrap()
                .an
                .iso_time(),
            "18:30"
        );
        assert_eq!(
            TimeResolver::resolve("2026-12-02T18:30:00+03:00", now())
                .unwrap()
                .an
                .iso_time(),
            "15:30"
        );
    }

    #[test]
    fn language_neutral_patterns_are_resolved() {
        for (text, expected, has_clock_time) in [
            ("2026-12-02 18:30", an(2026, 12, 2, 18, 30), true),
            ("2026/12/02 18:30", an(2026, 12, 2, 18, 30), true),
            ("02.12.2026 18:30", an(2026, 12, 2, 18, 30), true),
            ("02/12/2026 09:05", an(2026, 12, 2, 9, 5), true),
            ("02.12.2026", an(2026, 12, 2, 0, 0), false),
            ("2026/12/02", an(2026, 12, 2, 0, 0), false),
        ] {
            let c = TimeResolver::resolve(text, now()).unwrap_or_else(|| panic!("{text}"));
            assert_eq!(c.an, expected, "{text}");
            assert_eq!(c.has_clock, has_clock_time, "{text}");
        }
        // A day that does not exist in the calendar is not clamped, it is REJECTED.
        assert!(TimeResolver::resolve("2026-02-31", now()).is_none());
        assert!(TimeResolver::resolve("2026-13-01", now()).is_none());
        // 2024 is a leap year: 29 February is valid, in 2026 it is not.
        assert!(TimeResolver::resolve("2024-02-29", now()).is_some());
        assert!(TimeResolver::resolve("2026-02-29", now()).is_none());
    }

    #[test]
    fn turkish_shorthands_are_resolved() {
        // 2026-07-20 is a Monday.
        let c = TimeResolver::resolve("tomorrow 14:00", now()).expect("tomorrow");
        assert_eq!(c.an, an(2026, 7, 21, 14, 0));
        assert!(c.has_clock);

        // Mixed case must give the same result.
        assert_eq!(
            TimeResolver::resolve("Tomorrow 14:00", now()).unwrap().an,
            c.an
        );

        // Because the day is given explicitly a bare number counts as an hour.
        assert_eq!(
            TimeResolver::resolve("day after tomorrow 9", now())
                .unwrap()
                .an,
            an(2026, 7, 22, 9, 0)
        );

        // A shorthand with no clock time: start of day + has_clock=false.
        let b = TimeResolver::resolve("today", now()).expect("today");
        assert_eq!(b.an, an(2026, 7, 20, 0, 0));
        assert!(!b.has_clock);

        assert_eq!(
            TimeResolver::resolve("yesterday", now()).unwrap().an,
            an(2026, 7, 19, 0, 0)
        );
    }

    #[test]
    fn a_weekday_moves_forward() {
        // Today is Monday. "tuesday" -> the next day.
        assert_eq!(
            TimeResolver::resolve("tuesday 14:00", now()).unwrap().an,
            an(2026, 7, 21, 14, 0)
        );
        // "monday" does not go to today, it goes to NEXT week (today is not included).
        assert_eq!(
            TimeResolver::resolve("monday", now()).unwrap().an,
            an(2026, 7, 27, 0, 0)
        );
        // A long name must not swallow a short one.
        assert_eq!(
            TimeResolver::resolve("sunday", now()).unwrap().an,
            an(2026, 7, 26, 0, 0)
        );
        assert_eq!(
            TimeResolver::resolve("friday", now()).unwrap().an,
            an(2026, 7, 24, 0, 0)
        );
        assert_eq!(
            TimeResolver::resolve("saturday", now()).unwrap().an,
            an(2026, 7, 25, 0, 0)
        );
        // "next week" adds a week.
        assert_eq!(
            TimeResolver::resolve("next week tuesday", now())
                .unwrap()
                .an,
            an(2026, 7, 28, 0, 0)
        );
    }

    #[test]
    fn turkish_month_names_are_resolved() {
        assert_eq!(
            TimeResolver::resolve("2 december 2026", now()).unwrap().an,
            an(2026, 12, 2, 0, 0)
        );
        let has_clock_time =
            TimeResolver::resolve("2 december 2026 18:30", now()).expect("with a clock time");
        assert_eq!(has_clock_time.an, an(2026, 12, 2, 18, 30));
        assert!(has_clock_time.has_clock);
        // With no year the current year; if that is in the past, the next year.
        assert_eq!(
            TimeResolver::resolve("20 december", now()).unwrap().an,
            an(2026, 12, 20, 0, 0)
        );
        assert_eq!(
            TimeResolver::resolve("3 january", now()).unwrap().an,
            an(2027, 1, 3, 0, 0)
        );
    }

    #[test]
    fn an_unresolvable_time_does_not_silently_fall_back_to_now() {
        for raw in ["", "   ", "lorem ipsum", "zzz", "red car", "99/99/9999"] {
            assert!(
                TimeResolver::resolve(raw, now()).is_none(),
                "should not have resolved: {raw:?}"
            );
        }
    }

    #[test]
    fn diff_counts_the_leap_year_and_the_month_length_correctly() {
        // The points the model could not invent: 29 February and a month boundary.
        let tool = TimeTool::new().fixed_epoch(to_day_count(2024, 2, 28) * 86_400);
        assert_eq!(
            tool.diff_text("2024-03-01").unwrap(),
            "from=2024-02-28 to=2024-03-01 days=2"
        );

        let tool2 = TimeTool::new().fixed_epoch(to_day_count(2023, 2, 28) * 86_400);
        assert_eq!(
            tool2.diff_text("2023-03-01").unwrap(),
            "from=2023-02-28 to=2023-03-01 days=1"
        );

        // The case the model answered wrongly in Swift: 19 July -> 2 December.
        let tool3 = TimeTool::new().fixed_epoch(to_day_count(2026, 7, 19) * 86_400);
        assert_eq!(
            tool3.diff_text("2 december 2026").unwrap(),
            "from=2026-07-19 to=2026-12-02 days=136"
        );
    }

    #[test]
    fn diff_returns_negative_in_the_past() {
        let tool = TimeTool::new().fixed_epoch(NOW);
        // The sign is kept: so the model can answer "has it passed" without inventing.
        assert_eq!(
            tool.diff_text("2026-07-10").unwrap(),
            "from=2026-07-20 to=2026-07-10 days=-10"
        );
        // A time-of-day difference must NOT SHIFT the day: even asked at 12:00, tomorrow is 1 day.
        assert!(
            tool.diff_text("2026-07-21T01:00")
                .unwrap()
                .ends_with("days=1")
        );
    }

    #[test]
    fn if_diff_cannot_be_resolved_the_router_returns_an_error() {
        let tool = TimeTool::new().fixed_epoch(NOW);
        let error = tool
            .diff_text("blue cat")
            .expect_err("an error is expected");
        assert!(error.starts_with("error: unparsable_date"), "{error}");
        // The model must know what to do, otherwise it keeps coming back with the same input.
        assert!(error.contains("ISO 8601"), "{error}");
        assert!(
            !error.contains("days="),
            "the day count must not be invented: {error}"
        );
    }

    #[test]
    fn the_tool_runs_diff_and_drops_a_chip() {
        let tool = TimeTool::new().fixed_epoch(NOW);
        let mut ctx = context();
        let args = serde_json::json!({ "kind": "diff", "target": "2026-12-02" });
        let outcome = run_and_wait(tool.run(args, &mut ctx));
        assert_eq!(outcome.state, ToolState::Read);
        assert_eq!(outcome.to_model, "from=2026-07-20 to=2026-12-02 days=135");
        assert!(!outcome.chip_text.is_empty(), "diff must drop a chip");
    }

    #[test]
    fn the_tool_returns_a_failed_state_on_an_unresolvable_date() {
        let tool = TimeTool::new().fixed_epoch(NOW);
        let mut ctx = context();
        let args = serde_json::json!({ "kind": "diff", "target": "blue cat" });
        let outcome = run_and_wait(tool.run(args, &mut ctx));
        assert!(matches!(outcome.state, ToolState::Failed(_)));
        assert!(
            outcome.to_model.contains("unparsable_date"),
            "{}",
            outcome.to_model
        );
        assert!(!outcome.to_model.contains("days="));

        // An empty target does not silently fall back to today either.
        let empty = run_and_wait(tool.run(
            serde_json::json!({ "kind": "diff", "target": "" }),
            &mut ctx,
        ));
        assert!(matches!(empty.state, ToolState::Failed(_)));
    }

    #[test]
    fn the_tool_gives_the_instant_information_language_neutrally() {
        let tool = TimeTool::new().fixed_epoch(NOW).offset_minutes(180);
        assert_eq!(tool.now_text(Kind::Date), "date=2026-07-20");
        assert_eq!(
            tool.now_text(Kind::Weekday),
            "weekday=Monday date=2026-07-20"
        );
        assert_eq!(tool.now_text(Kind::Clock), "time=15:00 tz=UTC+03:00");
        assert_eq!(
            tool.now_text(Kind::All),
            "time=15:00 date=2026-07-20 weekday=Monday tz=UTC+03:00"
        );
        // A negative offset is formatted correctly too.
        assert!(TimeTool::new().offset_minutes(-330).tz_text() == "UTC-05:30");
    }

    #[test]
    fn the_tool_returns_an_error_on_an_invalid_kind() {
        let tool = TimeTool::new().fixed_epoch(NOW);
        let mut ctx = context();
        // An invented value like "difference" must NOT silently fall through to "all".
        let outcome = run_and_wait(tool.run(serde_json::json!({ "kind": "difference" }), &mut ctx));
        assert!(matches!(outcome.state, ToolState::Failed(_)));
        let missing = run_and_wait(tool.run(serde_json::json!({}), &mut ctx));
        assert!(matches!(missing.state, ToolState::Failed(_)));
    }

    #[test]
    fn the_schema_forces_the_model_into_a_closed_set() {
        let schema = TimeTool::new().schema();
        let js = schema.json_schema();
        assert_eq!(js["additionalProperties"], serde_json::json!(false));
        assert_eq!(js["required"], serde_json::json!(["kind"]));
        assert_eq!(
            js["properties"]["kind"]["enum"],
            serde_json::json!(Kind::ALL)
        );
        assert!(
            schema
                .validate(&serde_json::json!({ "kind": "diff", "target": "x" }))
                .is_ok()
        );
        assert!(
            schema
                .validate(&serde_json::json!({ "kind": "difference" }))
                .is_err()
        );
        assert!(
            schema
                .validate(&serde_json::json!({ "target": "x" }))
                .is_err()
        );
    }

    #[test]
    fn a_clock_trace_is_told_apart() {
        assert!(TimeResolver::clock_trace("tomorrow 18:00"));
        assert!(TimeResolver::clock_trace("tomorrow 18.30"));
        assert!(TimeResolver::clock_trace("tomorrow 6 pm"));
        assert!(!TimeResolver::clock_trace("tomorrow"));
        // A date must not be taken for a clock time.
        assert!(!TimeResolver::clock_trace("20.07.2026"));
    }

    // --- The local time zone ---

    #[test]
    fn offset_parsing_resolves_a_valid_format() {
        assert_eq!(parse_offset("+0300"), Some(180));
        assert_eq!(parse_offset("-0530"), Some(-330));
        assert_eq!(parse_offset("+0000"), Some(0));
    }

    /// A malformed format must NOT SILENTLY turn into a number: the whole rule
    /// of this file is that an unresolvable time is an error.
    #[test]
    fn offset_parsing_rejects_a_malformed_format() {
        for bad in ["", "0300", "+03:00", "+03", "+9900", "+0399", "abcde"] {
            assert_eq!(
                parse_offset(bad),
                None,
                "should not have been accepted: {bad}"
            );
        }
    }

    /// The offset read from the operating system is accepted if it is in a REASONABLE range.
    /// We cannot pin the value (it varies by machine), but we can verify the
    /// bounds — what is really wanted is that the call does not crash and does not return junk.
    #[test]
    fn the_local_offset_is_in_a_reasonable_range() {
        if let Some(min) = local_offset_minutes() {
            assert!(min.abs() <= 14 * 60, "unrealistic offset: {min}");
        }
    }

    /// On this machine (unix) the offset must REALLY resolve. The test above
    /// passes on `None` too, so it did not catch the "the `/bin/date` path
    /// broke" failure — this test does.
    #[cfg(unix)]
    #[test]
    fn on_unix_the_offset_is_really_resolved() {
        // The environment variable must not short-circuit the test.
        assert!(
            std::env::var_os("TACET_TZ_OFFSET").is_none(),
            "the environment offset is set, this test cannot measure the system path"
        );
        let min = local_offset_minutes().expect("on unix the offset must be read from the system");
        assert!(min.abs() <= 14 * 60, "unrealistic offset: {min}");
    }

    /// The candidate list must consist of ABSOLUTE paths: a `PATH` search can be poisoned.
    /// The Windows branch DOES NOT RUN on this machine but it DOES COMPILE thanks
    /// to `cfg!`; the test only verifies the list of the platform being run.
    #[test]
    fn the_offset_candidates_are_absolute_paths() {
        let candidates = offset_candidates();
        assert!(!candidates.is_empty(), "the candidate list is empty");
        for (path, args) in &candidates {
            assert!(path.is_absolute(), "relative path: {}", path.display());
            assert!(
                !args.is_empty(),
                "a candidate with no arguments: {}",
                path.display()
            );
        }
    }

    // --- Named days ---

    #[test]
    fn new_years_day_resolves_to_the_next_first_of_january() {
        let now = DateTime::new(2026, 7, 21, 10, 0, 0).unwrap();
        let c = TimeResolver::resolve("new year", now).expect("new year must resolve");
        assert_eq!(c.an.iso_date(), "2027-01-01");
        assert!(!c.has_clock);
        // "new year's day" gives the same day.
        assert_eq!(
            TimeResolver::resolve("new year's day", now)
                .unwrap()
                .an
                .iso_date(),
            "2027-01-01"
        );
    }

    /// Asked on 1 January, TODAY is the right answer (0 days), not next year.
    #[test]
    fn on_new_years_day_it_gives_today() {
        let now = DateTime::new(2027, 1, 1, 9, 0, 0).unwrap();
        assert_eq!(
            TimeResolver::resolve("new year", now)
                .unwrap()
                .an
                .iso_date(),
            "2027-01-01"
        );
    }

    #[test]
    fn the_new_years_day_diff_returns_positive() {
        let tool =
            TimeTool::new().fixed_epoch(DateTime::new(2026, 7, 21, 10, 0, 0).unwrap().epoch());
        let output = tool.diff_text("new year").expect("must be computable");
        assert!(output.contains("to=2027-01-01"), "{output}");
        assert!(output.contains("days=164"), "{output}");
    }

    /// The `weekday` kind CARRIES THE DATE TOO — on "what day of the month is it"
    /// the model insistently picks this kind (see `now_text`). If it is lost the failure comes back.
    #[test]
    fn the_weekday_kind_carries_the_date_too() {
        let tool =
            TimeTool::new().fixed_epoch(DateTime::new(2026, 7, 21, 10, 0, 0).unwrap().epoch());
        let output = tool.now_text(Kind::Weekday);
        assert!(output.contains("weekday=Tuesday"), "{output}");
        assert!(output.contains("date=2026-07-21"), "{output}");
    }
}
