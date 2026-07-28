//! Markdown formatting — designed for STREAMING output.
//!
//! WHY IT EXISTS: the model produces `**bold**`, `` `code` ``, `# heading`,
//! `- bullet` and tables; on screen these came out with their raw markers. What
//! the user saw was markdown source code, not text.
//!
//! THE HARD PART IS THE STREAM. Text arrives token by token: the second star of
//! a `**` pair, the space after a heading line's `#`, the second row of a table
//! — none of them may be in hand "right now". There are two wrong solutions and
//! we rejected both:
//!
//!  * **Buffer everything and print at the end.** It would kill the stream;
//!    filling the 5-15 second wait with flowing text is one of the reasons this
//!    shell exists.
//!  * **Open the style the moment a marker is seen.** Printing BOLD as soon as
//!    `**` arrives and then waiting for the close bolds the rest of the line if
//!    the close never comes (the model ended the sentence some other way,
//!    generation was cut off). A broken screen is exactly this.
//!
//! THE PATH CHOSEN — **defer the decision, smallest possible buffer**:
//! everything flows character by character; ONLY the fragment awaiting a
//! decision is held.
//!   * at the START OF A LINE: the first few characters that determine the
//!     structure (`#`, `- `, `|`, ```` ``` ````) — a delay of a few characters
//!     at most,
//!   * INSIDE a line: the body of an opened but unclosed marker — printed with
//!     its style when the close arrives, printed RAW if the line ends before it
//!     closes (so in the worst case the user sees today's output, not a broken
//!     screen).
//!
//! A TABLE is the one exception: column width can only be known once all the
//! rows have arrived, so a block starting with `|` is collected and printed when
//! the block ends. The alternative was printing an unaligned table — the exact
//! opposite of the request.
//!
//! IF IT IS NOT A TTY NONE OF THIS HAPPENS. A formatter built with
//! `colored=false` adds nothing of its own: no markdown is interpreted and no
//! escape is emitted. Piped output staying parseable (and today's scripts not
//! breaking) matters more than decoration.
//!
//! ONE THING HAPPENS ON BOTH PATHS — `defang`. TERMINAL CONTROL CHARACTERS ARE
//! REMOVED FROM THE MODEL'S TEXT, tty or not. The model's output is
//! attacker-reachable (a fetched page, a read file) and a terminal EXECUTES
//! escape sequences, so passing them through hands an injected page control of
//! the screen the user reads to check what the assistant did. The pipe is not
//! exempt: redirected output becomes a file, and the escapes fire when someone
//! `cat`s it.

use crate::ui::{BOLD, REVERSE, dim_code, reset_code};

/// The most characters held while awaiting a decision at the start of a line.
/// Only lines made up entirely of whitespace hit this cap; hitting it means
/// "plain line".
const PREFIX_CAP: usize = 16;

/// How much body an unclosed marker may hold at most. If the model produces a
/// single `*` or `` ` `` and carries on, the whole text must not end up in the
/// buffer — past a certain point, saying "that was not a marker" and printing
/// raw is better than holding the answer hostage.
const MARKER_CAP: usize = 200;

/// Neutralises one character of MODEL TEXT before it can reach the terminal.
///
/// WHY THIS EXISTS: a terminal does not display escape sequences, it EXECUTES
/// them, and the text flowing through this formatter is attacker-reachable —
/// `web_fetch` hands the model a page someone else wrote, `read_file` hands it
/// a file someone else sent. Qwen2.5's tokenizer is byte-level BPE, so the
/// model can emit a bare `0x1b` when a fetched page tells it to; the call
/// filter upstream only strips tool calls and everything else is copied
/// through to `write_all`. With the escapes intact an injected page can clear
/// the screen, move the cursor over what was already printed, turn the text
/// invisible (`ESC[8m`), rewrite the window title, or write the user's
/// clipboard (`OSC 52`) — i.e. it controls the surface the user reads to check
/// what the assistant actually did.
///
/// THE PLACE MATTERS. It is done HERE, on the way in, BEFORE any of our own
/// styling is added, so the shell's own escapes stay safe by construction; a
/// filter at `Screen::write` would have to tell our escapes from the model's,
/// and it cannot.
///
/// `char::is_control()` covers C0, DEL **and** the C1 range — U+009B is a
/// single-character CSI and is just as good as `ESC [` on a UTF-8 terminal, so
/// all three are needed. `\n` and `\t` survive: they are layout, not commands.
/// The replacement character is used rather than deletion so a hostile payload
/// leaves a visible mark instead of vanishing.
fn defang(c: char) -> char {
    match c {
        '\n' | '\t' => c,
        c if c.is_control() => '\u{fffd}',
        c => c,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// The line's structure has not been decided yet.
    LineStart,
    /// The line's body is flowing.
    Body,
    /// A table row starting with `|` is being collected raw.
    TableRow,
    /// A code block line: printed with NO parsing at all until end of line.
    RawLine,
    /// The rest of the line is dropped; if `nl`, a newline is printed at the end.
    Skip { nl: bool },
}

#[derive(Clone, Copy)]
enum Marker {
    Code,
    Bold,
}

impl Marker {
    fn raw_prefix(self) -> &'static str {
        match self {
            Marker::Code => "`",
            Marker::Bold => "**",
        }
    }
    fn style(self) -> &'static str {
        match self {
            Marker::Code => REVERSE,
            Marker::Bold => BOLD,
        }
    }
}

/// The start-of-line decision.
enum Decision {
    Plain,
    Heading,
    Bullet(String),
    Ordered(String, String),
    Quote,
    Table,
    /// ```` ``` ```` — a code block fence (opens or closes).
    Fence,
    /// `---` horizontal separator.
    Separator,
}

/// The state machine that turns streaming text into ANSI.
pub struct Formatter {
    colored: bool,
    mode: Mode,
    /// Raw characters at the start of a line awaiting the decision.
    prefix: String,
    /// An opened but unclosed marker and the body collected so far.
    marker: Option<(Marker, String)>,
    /// A single `*` was seen; bold opens if a second one arrives.
    star: bool,
    /// The style applied to the whole line (heading/quote) — REPRINTED when an
    /// inner style closes, otherwise `RESET` would erase that one too.
    line_style: Option<&'static str>,
    /// Are we between ```` ``` ```` fences — markdown is not parsed inside.
    code_block: bool,
    table: Vec<String>,
    table_row: String,
    output: String,
}

impl Formatter {
    pub fn new(colored: bool) -> Self {
        Self {
            colored,
            mode: Mode::LineStart,
            prefix: String::new(),
            marker: None,
            star: false,
            line_style: None,
            code_block: false,
            table: Vec::new(),
            table_row: String::new(),
            output: String::new(),
        }
    }

    /// Feeds a chunk; returns the part that IS PRINTABLE to the screen.
    pub fn feed(&mut self, chunk: &str) -> String {
        if !self.colored {
            // NOT A LOOPHOLE. This branch used to hand the input back byte for
            // byte, and "it is only a pipe" is not a defence: the output is
            // redirected into a file and the same escapes execute the moment
            // someone `cat`s it. Same attack, delayed.
            return chunk.chars().map(defang).collect();
        }
        self.output.clear();
        for c in chunk.chars() {
            self.char(defang(c));
        }
        std::mem::take(&mut self.output)
    }

    /// The stream ended: whatever is left in the buffer is printed RAW. NO
    /// newline is added — the caller (the end of the answer) prints that itself.
    pub fn finish(&mut self) -> String {
        if !self.colored {
            return String::new();
        }
        self.output.clear();
        if self.mode == Mode::TableRow {
            let s = std::mem::take(&mut self.table_row);
            self.table.push(s);
        }
        self.drain_table();
        // An unclosed marker: print raw. Showing the user today's output is
        // preferred over breaking the screen.
        if self.star {
            self.star = false;
            self.output.push('*');
        }
        if let Some((marker, body)) = self.marker.take() {
            self.output.push_str(marker.raw_prefix());
            self.output.push_str(&body);
        }
        let prefix = std::mem::take(&mut self.prefix);
        self.output.push_str(&prefix);
        if self.line_style.take().is_some() {
            self.output.push_str(reset_code());
        }
        self.mode = Mode::LineStart;
        std::mem::take(&mut self.output)
    }

    /// One-shot formatting (no stream).
    pub fn all(colored: bool, text: &str) -> String {
        let mut f = Formatter::new(colored);
        let mut s = f.feed(text);
        s.push_str(&f.finish());
        s
    }

    // -----------------------------------------------------------------------

    fn char(&mut self, c: char) {
        match self.mode {
            Mode::Skip { nl } => {
                if c == '\n' {
                    if nl {
                        self.output.push('\n');
                    }
                    self.mode = Mode::LineStart;
                }
            }
            Mode::RawLine => {
                if c == '\n' {
                    if self.line_style.take().is_some() && self.colored {
                        self.output.push_str(reset_code());
                    }
                    self.output.push('\n');
                    self.mode = Mode::LineStart;
                } else {
                    self.output.push(c);
                }
            }
            Mode::TableRow => {
                if c == '\n' {
                    let s = std::mem::take(&mut self.table_row);
                    self.table.push(s);
                    self.mode = Mode::LineStart;
                } else {
                    self.table_row.push(c);
                }
            }
            Mode::LineStart => self.line_start_char(c),
            Mode::Body => self.body_char(c),
        }
    }

    fn line_start_char(&mut self, c: char) {
        if c == '\n' {
            // An empty line CLOSES the table: the block is over, it is printed
            // in its aligned form.
            let prefix = std::mem::take(&mut self.prefix);
            self.drain_table();
            self.output.push_str(&prefix);
            self.output.push('\n');
            return;
        }
        self.prefix.push(c);

        // INSIDE a code block the only decision is `is the fence closing`; every
        // other line is printed RAW (in code, `*` and `#` are not markdown).
        if self.code_block {
            let t = self.prefix.trim_start();
            if t.chars().all(|c| c == '`') && t.len() < 3 {
                return; // could become ```, wait
            }
            if t.starts_with("```") {
                self.code_block = false;
                self.prefix.clear();
                self.mode = Mode::Skip { nl: false };
                return;
            }
            // Inline markdown IS NOT LOOKED FOR on a code line: `**` is an
            // operator there.
            let prefix = std::mem::take(&mut self.prefix);
            if self.colored {
                self.output.push_str(dim_code());
                self.line_style = Some(dim_code());
            }
            self.output.push_str("  ");
            self.output.push_str(&prefix);
            self.mode = Mode::RawLine;
            return;
        }

        match prefix_decision(&self.prefix) {
            Some(d) => self.apply_decision(d),
            None => {
                if self.prefix.chars().count() >= PREFIX_CAP {
                    self.apply_decision(Decision::Plain);
                }
            }
        }
    }

    fn apply_decision(&mut self, decision: Decision) {
        // The table block must be printed BEFORE the output of the line that
        // comes after it.
        if !matches!(decision, Decision::Table) {
            self.drain_table();
        }
        let prefix = std::mem::take(&mut self.prefix);
        match decision {
            Decision::Plain => {
                self.mode = Mode::Body;
                // The held prefix was UNDECIDED raw text: it is fed back into
                // the body machine so `**bold**` works at the start of a line
                // too.
                for c in prefix.chars() {
                    self.body_char(c);
                }
            }
            Decision::Heading => {
                let s = paint_open(self.colored, BOLD);
                self.output.push_str(&s);
                self.line_style = Some(BOLD);
                self.mode = Mode::Body;
            }
            Decision::Bullet(space) => {
                self.output.push_str(&format!("{space}  • "));
                self.mode = Mode::Body;
            }
            Decision::Ordered(space, tag) => {
                self.output.push_str(&format!("{space}  {tag}"));
                self.mode = Mode::Body;
            }
            Decision::Quote => {
                let s = paint_open(self.colored, dim_code());
                self.output.push_str(&s);
                self.output.push_str("  │ ");
                self.line_style = Some(dim_code());
                self.mode = Mode::Body;
            }
            Decision::Table => {
                self.table_row = prefix;
                self.mode = Mode::TableRow;
            }
            Decision::Fence => {
                self.code_block = true;
                self.mode = Mode::Skip { nl: false };
            }
            Decision::Separator => {
                let line = paint(self.colored, dim_code(), "  ────────");
                self.output.push_str(&line);
                self.mode = Mode::Skip { nl: true };
            }
        }
    }

    fn body_char(&mut self, c: char) {
        if c == '\n' {
            self.end_line();
            return;
        }
        if let Some((marker, mut body)) = self.marker.take() {
            match marker {
                Marker::Code if c == '`' => {
                    self.write_span(marker, &body);
                    return;
                }
                _ => {}
            }
            body.push(c);
            if matches!(marker, Marker::Bold) && body.ends_with("**") {
                body.truncate(body.len() - 2);
                self.write_span(marker, &body);
                return;
            }
            if body.chars().count() > MARKER_CAP {
                // The close never came: this was not a marker. Print raw.
                self.output.push_str(marker.raw_prefix());
                self.output.push_str(&body);
                return;
            }
            self.marker = Some((marker, body));
            return;
        }
        if self.star {
            self.star = false;
            if c == '*' {
                self.marker = Some((Marker::Bold, String::new()));
                return;
            }
            // A single star: we do not distinguish markdown italics, it is
            // printed raw.
            self.output.push('*');
        }
        match c {
            '`' => self.marker = Some((Marker::Code, String::new())),
            '*' => self.star = true,
            _ => self.output.push(c),
        }
    }

    fn write_span(&mut self, marker: Marker, body: &str) {
        if !self.colored {
            self.output.push_str(body);
            return;
        }
        self.output.push_str(marker.style());
        self.output.push_str(body);
        self.output.push_str(reset_code());
        // The line style (heading/quote/code line) was erased by RESET; put it
        // back.
        if let Some(s) = self.line_style {
            self.output.push_str(s);
        }
    }

    fn end_line(&mut self) {
        if self.star {
            self.star = false;
            self.output.push('*');
        }
        if let Some((marker, body)) = self.marker.take() {
            self.output.push_str(marker.raw_prefix());
            self.output.push_str(&body);
        }
        if self.line_style.take().is_some() && self.colored {
            self.output.push_str(reset_code());
        }
        self.output.push('\n');
        self.mode = Mode::LineStart;
    }

    /// Prints the collected table rows ALIGNED.
    fn drain_table(&mut self) {
        if self.table.is_empty() {
            return;
        }
        let raw = std::mem::take(&mut self.table);
        let text = draw_table(&raw, self.colored);
        self.output.push_str(&text);
    }
}

/// Decides the line's structure. `None` = not clear yet, wait for a character.
fn prefix_decision(prefix: &str) -> Option<Decision> {
    let body = prefix.trim_start_matches([' ', '\t']);
    let space = prefix[..prefix.len() - body.len()].to_string();
    let first = body.chars().next()?; // whitespace only: wait
    match first {
        '`' => {
            if body.chars().all(|c| c == '`') && body.chars().count() < 3 {
                return None;
            }
            if body.starts_with("```") {
                return Some(Decision::Fence);
            }
            Some(Decision::Plain)
        }
        '#' => {
            let n = body.chars().take_while(|&c| c == '#').count();
            let Some(next) = body.chars().nth(n) else {
                // Still stacking `#`: it can be a heading up to 6.
                return if n <= 6 { None } else { Some(Decision::Plain) };
            };
            if next == ' ' && n <= 6 {
                Some(Decision::Heading)
            } else {
                Some(Decision::Plain)
            }
        }
        '-' | '*' | '+' => match body.chars().nth(1) {
            None => None,
            Some(' ') => Some(Decision::Bullet(space)),
            Some(x) if x == first && first != '*' => match body.chars().nth(2) {
                None => None,
                Some(y) if y == first => Some(Decision::Separator),
                _ => Some(Decision::Plain),
            },
            _ => Some(Decision::Plain),
        },
        '|' => Some(Decision::Table),
        '>' => match body.chars().nth(1) {
            None => None,
            Some(' ') => Some(Decision::Quote),
            _ => Some(Decision::Plain),
        },
        c if c.is_ascii_digit() => {
            let digits = body.chars().take_while(|c| c.is_ascii_digit()).count();
            let remaining: Vec<char> = body.chars().skip(digits).collect();
            match remaining.first() {
                None => {
                    if digits <= 3 {
                        None
                    } else {
                        Some(Decision::Plain)
                    }
                }
                Some('.') | Some(')') => match remaining.get(1) {
                    None => None,
                    Some(' ') => {
                        let tag: String = body.chars().take(digits + 2).collect();
                        Some(Decision::Ordered(space, tag))
                    }
                    _ => Some(Decision::Plain),
                },
                _ => Some(Decision::Plain),
            }
        }
        _ => Some(Decision::Plain),
    }
}

fn paint(colored: bool, code: &str, text: &str) -> String {
    if colored {
        format!("{code}{text}{}", reset_code())
    } else {
        text.to_string()
    }
}

fn paint_open(colored: bool, code: &str) -> String {
    if colored {
        code.to_string()
    } else {
        String::new()
    }
}

/// Turns raw `| a | b |` rows into text with aligned columns.
///
/// THE PIPE CHARACTERS ARE DROPPED. Drawing a frame with `|` in a terminal
/// drowns the line in noise; the alignment itself already reads as a column.
/// The header row is bold, with a thin separator line under it.
pub fn draw_table(raw: &[String], colored: bool) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for s in raw {
        let t = s.trim();
        let t = t.strip_prefix('|').unwrap_or(t);
        let t = t.strip_suffix('|').unwrap_or(t);
        let cells: Vec<String> = t.split('|').map(|c| c.trim().to_string()).collect();
        // A `|---|:--:|` separator row: not data, a format declaration.
        let separator = cells
            .iter()
            .all(|c| !c.is_empty() && c.chars().all(|c| c == '-' || c == ':' || c == ' '));
        if separator {
            continue;
        }
        rows.push(cells);
    }
    if rows.is_empty() {
        return String::new();
    }
    let columns = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut width = vec![0usize; columns];
    for r in &rows {
        for (i, c) in r.iter().enumerate() {
            width[i] = width[i].max(c.chars().count());
        }
    }
    let mut output = String::new();
    for (i, r) in rows.iter().enumerate() {
        let mut line = String::from("  ");
        for (j, c) in r.iter().enumerate() {
            line.push_str(c);
            if j + 1 < r.len() {
                line.push_str(&" ".repeat(width[j] - c.chars().count() + 2));
            }
        }
        let line = line.trim_end().to_string();
        if i == 0 {
            output.push_str(&paint(colored, BOLD, &line));
            output.push('\n');
            let divider: Vec<String> = width.iter().map(|w| "─".repeat(*w)).collect();
            output.push_str(&paint(
                colored,
                dim_code(),
                &format!("  {}", divider.join("  ")),
            ));
        } else {
            output.push_str(&line);
        }
        output.push('\n');
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::RESET;

    /// A helper that feeds the stream CHUNK BY CHUNK and joins the result: this
    /// is how a real stream arrives and every bug shows up exactly at a chunk
    /// boundary.
    fn chunked(text: &str, size: usize) -> String {
        let mut f = Formatter::new(true);
        let mut output = String::new();
        let chars: Vec<char> = text.chars().collect();
        for set in chars.chunks(size) {
            let p: String = set.iter().collect();
            output.push_str(&f.feed(&p));
        }
        output.push_str(&f.finish());
        output
    }

    /// Removes the `ESC[..m` colour codes THIS formatter emits, then asserts
    /// that nothing executable is left — i.e. that every escape on screen is
    /// one we put there ourselves.
    fn assert_no_escapes_of_its_own(what: &str, output: &str) {
        let mut rest = output;
        let mut plain = String::new();
        while let Some(i) = rest.find('\u{1b}') {
            plain.push_str(&rest[..i]);
            let tail = &rest[i + 1..];
            // Our own codes are exactly `[` + digits/`;` + `m`.
            let end = tail.find('m');
            let is_ours = tail.starts_with('[')
                && end.is_some_and(|e| tail[1..e].chars().all(|c| c.is_ascii_digit() || c == ';'));
            if is_ours {
                rest = &tail[end.unwrap() + 1..];
            } else {
                panic!("{what}: an escape we did not write reached the screen: {output:?}");
            }
        }
        plain.push_str(rest);
        for bad in ['\u{1b}', '\u{9b}', '\u{7}', '\u{7f}', '\r'] {
            assert!(!plain.contains(bad), "{what}: {bad:?} survived: {output:?}");
        }
    }

    /// A TERMINAL EXECUTES WHAT IT IS HANDED; MODEL TEXT MUST NOT REACH IT WITH
    /// ESCAPES INTACT.
    ///
    /// The attack this closes: a fetched page carries an injected instruction
    /// ("start your answer with these bytes"), the byte-level tokenizer lets
    /// the model emit a bare `0x1b`, and from there the page owns the screen —
    /// `ESC[2J` wipes it, `ESC[8m` hides text, `OSC 52` writes the clipboard,
    /// `OSC 0` renames the window. What the user reads to check the assistant
    /// would then be written by the attacker.
    ///
    /// Every route the text can take out of this formatter is measured: the
    /// plain path, the chunked stream, a table cell, a code block, and the
    /// non-tty pipe.
    #[test]
    fn escape_sequences_from_the_model_are_neutralised() {
        let out = Formatter::all(true, "hi \u{1b}[2J\u{1b}]0;x\u{7}\u{9b}2J\u{7f}bye");
        assert!(!out.contains('\u{1b}'), "ESC reached the screen: {out:?}");
        // U+009B is a SINGLE-CHARACTER CSI: as good as `ESC [` on a UTF-8
        // terminal, and `is_control()` is what catches it.
        assert!(
            !out.contains('\u{9b}'),
            "C1 CSI reached the screen: {out:?}"
        );
        assert!(!out.contains('\u{7}'), "BEL reached the screen: {out:?}");
        assert!(!out.contains('\u{7f}'), "DEL reached the screen: {out:?}");
        assert!(out.contains("hi") && out.contains("bye"), "{out:?}");

        // The real stream shape: a chunk boundary must not become a hole.
        for size in [1, 2, 3, 7] {
            let s = chunked("a\u{1b}[2Jb", size);
            assert!(!s.contains('\u{1b}'), "chunk size {size}: {s:?}");
        }

        // A table cell and a code block are separate code paths inside this
        // file (buffered rows, raw lines) and both end up on the screen. These
        // two DO carry escapes — OUR OWN styling — so the assertion strips the
        // `ESC[..m` colour codes this formatter emits and then insists nothing
        // escape-shaped is left. Asserting "no ESC at all" would be a weaker
        // test that happens to pass on the unstyled paths above.
        let table = Formatter::all(true, "| \u{1b}[2Jx | b |\n| - | - |\n| c | d |\n");
        assert_no_escapes_of_its_own("table cell", &table);
        let code = Formatter::all(true, "```\n\u{1b}[2Jx\n```\n");
        assert_no_escapes_of_its_own("code block", &code);

        // THE PIPE IS NOT A LOOPHOLE: redirected output is a file, and the
        // escapes fire when it is `cat`ed later.
        let piped = Formatter::all(false, "x\u{1b}[2J\u{9b}2Jy");
        assert!(!piped.contains('\u{1b}'), "piped: {piped:?}");
        assert!(!piped.contains('\u{9b}'), "piped: {piped:?}");

        // Layout survives — the fix must not eat newlines or tabs.
        assert_eq!(Formatter::all(false, "a\nb\tc"), "a\nb\tc");
    }

    /// It must give the same result REGARDLESS OF CHUNK SIZE. This test breaking
    /// is "the screen is broken" measured by machine.
    ///
    /// The text is deliberately NOT pure ASCII: multi-byte characters are what
    /// separate `chars().count()` from `len()`, and dropping them would take the
    /// column-width and buffer arithmetic out of coverage.
    #[test]
    fn chunk_size_does_not_change_the_result() {
        let text = "# Heading\n\nThis has **bold** and `code` — ölçü.\n\n- first\n- second\n";
        let single = chunked(text, 1000);
        for size in [1, 2, 3, 5, 7, 13] {
            assert_eq!(chunked(text, size), single, "chunk size {size}");
        }
    }

    #[test]
    fn bold_and_code_are_rendered() {
        let c = chunked("This is **bold** and `code`.\n", 1);
        assert!(c.contains(&format!("{BOLD}bold{RESET}")), "{c:?}");
        assert!(c.contains(&format!("{REVERSE}code{RESET}")), "{c:?}");
        assert!(
            !c.contains("**"),
            "the stars must not stay on screen: {c:?}"
        );
    }

    /// An UNCLOSED marker DOES NOT BREAK the screen: it is printed raw, no style
    /// is opened.
    #[test]
    fn an_unclosed_marker_is_printed_raw() {
        let c = chunked("2 **3 = 6 and it goes on\n", 1);
        assert!(c.contains("**3 = 6 and it goes on"), "{c:?}");
        assert!(
            !c.contains(BOLD),
            "an unopened bold style must not be written: {c:?}"
        );
        // Even when the line ends, an unclosed buffer is released.
        let d = chunked("half `code", 1);
        assert!(d.contains("half `code"), "{d:?}");
    }

    #[test]
    fn heading_and_bullet() {
        let c = chunked("## Notes\n- one\n  - inner\n1. numbered\n", 1);
        assert!(c.contains(&format!("{BOLD}Notes")), "{c:?}");
        assert!(!c.contains("## "), "{c:?}");
        assert!(c.contains("  • one"), "{c:?}");
        assert!(c.contains("    • inner"), "{c:?}");
        assert!(c.contains("  1. numbered"), "{c:?}");
    }

    /// The non-ASCII cell (`ölçü`) is deliberate: with ASCII-only data a width
    /// computed from BYTES instead of CHARACTERS would still pass.
    #[test]
    fn the_table_is_aligned() {
        let c = chunked(
            "| name | ölçü |\n|---|---|\n| a | 1 |\n| longname | 22 |\n\n",
            1,
        );
        assert!(
            !c.contains('|'),
            "the pipe characters must not remain: {c:?}"
        );
        assert!(c.contains("longname  22"), "{c:?}");
        // The header's 'name' column must line up with the width of 'longname'.
        assert!(c.contains("name      ölçü"), "{c:?}");
    }

    /// WITH NO TTY IT IS TRANSPARENT: the output is the input verbatim, not a
    /// single ANSI byte.
    #[test]
    fn without_color_it_is_transparent() {
        let text = "# Heading\n**bold** `code`\n| a | b |\n";
        let mut f = Formatter::new(false);
        let mut c = f.feed(text);
        c.push_str(&f.finish());
        assert_eq!(c, text);
        assert!(!c.contains('\x1b'));
    }

    /// Markdown IS NOT PARSED inside a code block — `**` is code there.
    #[test]
    fn no_markers_inside_a_code_block() {
        let c = chunked("```rust\nlet a = **b;\n```\nend\n", 1);
        assert!(c.contains("let a = **b;"), "{c:?}");
        assert!(!c.contains("```"), "{c:?}");
        assert!(c.contains("end"), "{c:?}");
    }

    /// A one-word answer IS NOT SWALLOWED (even with no newline).
    #[test]
    fn a_single_word_is_not_swallowed() {
        assert_eq!(chunked("Yes.", 1), "Yes.");
        assert_eq!(chunked("Yes", 1), "Yes");
    }
}
