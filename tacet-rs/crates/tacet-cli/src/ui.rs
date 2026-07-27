//! The terminal UI — input lock, spinner, live tool chips.
//!
//! WHY A SEPARATE MODULE: `main.rs` tells the flow (the turn loop, tool
//! execution, prompt construction); this file only knows THE SCREEN. When the
//! two lived in the same file, cursor escapes got mixed in among the logic and
//! made both sides unreadable.
//!
//! IT CLOSES THREE FAILURES — all three seen in a real user session:
//!
//! 1. **INPUT MIXING.** While the model was generating, the keys the user
//!    pressed got into the output through the terminal's own ECHO:
//!    `tacet> kindHello! How can I ihelp you?`. This is NOT a streaming bug —
//!    the one-shot output is spotless. The fix is to turn on RAW MODE for the
//!    duration of generation and SWALLOW the key events: the echo is off, the
//!    keys do not pile up in a buffer either, and nothing spills onto the screen
//!    when generation ends.
//! 2. **A FROZEN SCREEN.** 5-15 seconds pass before the first token and nothing
//!    happened on screen. A spinner + the elapsed time makes the wait read as
//!    "working".
//! 3. **AN INVISIBLE TOOL.** The chips were only printed AFTER the turn ended;
//!    while a tool was running the screen was silent. Now a chip lands the
//!    moment it starts and, when it finishes, turns into the result by being
//!    written OVER THE SAME LINE.
//!
//! IF IT IS NOT A TTY, NONE OF THIS HAPPENS. Piped input (a script, CI) cannot
//! turn on raw mode; trying to would also error. `Screen::setup` measures the tty
//! once, and with no tty the whole module falls back to plain text — the shell
//! DOES NOT CRASH, it merely runs without the decoration.

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tacet_core::{Reporter, ToolState, ToolTrace, TraceCollector, TraceId, TraceUpdate};

// ANSI escapes. The palette is INK/GREY: NO accent colour, NO status dot. State
// is told in words and marks — not in colour.
pub const RESET: &str = "\x1b[0m";
pub const DIM: &str = "\x1b[2m";
pub const BOLD: &str = "\x1b[1m";
/// Reverse video — for inline `code`. NOT a colour; the same ink is inverted in
/// tone, so a code fragment separates from prose without breaking the palette
/// rule (no accent colour).
pub const REVERSE: &str = "\x1b[7m";
/// A warning. The single colour exception, and not a BRAND accent: letting the
/// "something went wrong" information disappear into shades of grey would be
/// hiding what the user needs to know.
pub const YELLOW: &str = "\x1b[33m";
/// Brass — the only accent of the "Night ink" palette (the 256-colour
/// approximation of the brand colour 0xC9A227). ONLY for brand moments (the
/// banner's full stop, the spinning ensō): state, a warning or information is
/// NEVER written in this colour — those stay in words and shades of grey.
pub const BRASS: &str = "\x1b[38;5;178m";

// ---------------------------------------------------------------------------
// Themes
// ---------------------------------------------------------------------------
// A theme is THREE inks on the user's own background: paper (the answer),
// dim (the quiet lines) and an accent (brand moments + the prompt marker).
// The terminal's background is never painted — every theme assumes the user's
// canvas, which is why they are tuned for dark terminals except `mono`, which
// simply uses whatever the terminal already is. The active theme can change
// AT RUNTIME (`/themes`), so the selection is an atomic index, not a OnceLock;
// the escape strings are per-theme literals so every accessor stays
// `&'static str` and the hot paint path allocates nothing.
//
// The reset of a themed palette immediately re-applies the paper ink, so any
// inner style closing (bold, inline code) falls back to paper, not to the
// terminal default — this single trick keeps a whole answer readable in one
// colour without threading state through the formatter.

pub struct Theme {
    pub name: &'static str,
    pub description: &'static str,
    paper: &'static str,
    dim: &'static str,
    accent: &'static str,
    reset: &'static str,
}

/// The catalogue. The palettes come from the brand exploration: night ink is
/// the landing page, the other three were its finalist siblings.
pub const THEMES: &[Theme] = &[
    Theme {
        name: "mono",
        description: "your terminal's own colours, brass accent",
        paper: "",
        dim: DIM,
        accent: BRASS,
        reset: RESET,
    },
    Theme {
        name: "night",
        description: "night ink — paper, mist and brass (the brand)",
        paper: "\x1b[38;2;233;228;216m",
        dim: "\x1b[38;2;151;161;180m",
        accent: "\x1b[38;2;201;162;39m",
        reset: "\x1b[0m\x1b[38;2;233;228;216m",
    },
    Theme {
        name: "sage",
        description: "sage quiet — cream, sage and terracotta",
        paper: "\x1b[38;2;237;240;232m",
        dim: "\x1b[38;2;134;164;147m",
        accent: "\x1b[38;2;201;111;74m",
        reset: "\x1b[0m\x1b[38;2;237;240;232m",
    },
    Theme {
        name: "graphite",
        description: "graphite and amber — cool greys, warm accent",
        paper: "\x1b[38;2;242;240;235m",
        dim: "\x1b[38;2;154;157;166m",
        accent: "\x1b[38;2;224;164;88m",
        reset: "\x1b[0m\x1b[38;2;242;240;235m",
    },
    Theme {
        name: "violet",
        description: "violet dusk — lavender greys, violet accent",
        paper: "\x1b[38;2;241;239;248m",
        dim: "\x1b[38;2;175;166;214m",
        accent: "\x1b[38;2;124;108;217m",
        reset: "\x1b[0m\x1b[38;2;241;239;248m",
    },
];

static ACTIVE_THEME: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static THEME_INIT: OnceLock<()> = OnceLock::new();

/// Applies the config file's `theme` key once, lazily, and only on a tty —
/// piped output stays free of every escape, theme or not.
fn ensure_theme() {
    THEME_INIT.get_or_init(|| {
        if std::io::stdout().is_terminal()
            && let Some(name) = crate::config::get_str("theme")
        {
            let _ = set_theme(&name);
        }
    });
}

/// Switches the live theme. `false` when the name is unknown.
///
/// MUST NOT touch THEME_INIT: `ensure_theme`'s initializer calls this
/// function, and a `get_or_init` from inside its own initializer deadlocks the
/// OnceLock (measured: the shell froze before the banner). The store alone is
/// enough — if a switch somehow lands before the lazy init, the init only
/// re-applies the same config value afterwards.
pub fn set_theme(name: &str) -> bool {
    match THEMES.iter().position(|t| t.name == name) {
        Some(i) => {
            ACTIVE_THEME.store(i, Ordering::Relaxed);
            true
        }
        None => false,
    }
}

pub fn active_theme() -> &'static Theme {
    ensure_theme();
    &THEMES[ACTIVE_THEME.load(Ordering::Relaxed).min(THEMES.len() - 1)]
}

/// Painting a sample in a THEME'S OWN accent — the `/themes` list uses this so
/// each row previews itself.
pub fn theme_accent(theme: &Theme) -> &'static str {
    theme.accent
}

pub fn reset_code() -> &'static str {
    active_theme().reset
}

pub fn dim_code() -> &'static str {
    active_theme().dim
}

pub fn brass_code() -> &'static str {
    active_theme().accent
}

/// The answer ink: paper in the themed palettes, nothing in mono — the mono
/// answer is the terminal's own ink.
pub fn paper_code() -> &'static str {
    active_theme().paper
}

/// Clears the line from the start (move the cursor to the start + erase to end
/// of line).
const CLEAR_LINE: &str = "\r\x1b[2K";
/// One line up.
const ONE_UP: &str = "\x1b[1A";

/// The spinner frames. Quarter-circle arcs — a spinning ensō: the brand mark
/// is an open brush circle, and these four glyphs rotate the same open circle
/// one quadrant at a time. One character wide; painted BRASS at the call site,
/// the single brand moment the wait is allowed to have.
const FRAMES: [&str; 4] = ["◜", "◝", "◞", "◟"];

// ---------------------------------------------------------------------------
// Colour
// ---------------------------------------------------------------------------

/// If it is not a tty (piped output, CI) NO colour is written.
pub struct Color {
    tty: bool,
}

impl Color {
    pub fn setup() -> Self {
        Self { tty: std::io::stdout().is_terminal() }
    }
    pub fn paint(&self, code: &str, text: &str) -> String {
        if self.tty {
            // The theme translation lives HERE so no call site changes when the
            // user switches themes: the symbolic constants stay the API.
            let code = match code {
                DIM => dim_code(),
                BRASS => brass_code(),
                _ => code,
            };
            format!("{code}{text}{}", reset_code())
        } else {
            text.to_string()
        }
    }
}

/// A single-key y/N question, read THROUGH CROSSTERM, not through std stdin.
///
/// WHY NOT `stdin().read_line`: the input field reads through crossterm, and
/// crossterm slurps every byte already waiting on the tty into its own parser
/// buffer. Input that arrived fast — a paste, a script feeding lines — is
/// therefore sitting where std stdin can never see it, and a `read_line` would
/// hang on an fd that looks empty. Reading the answer through the same channel
/// consumes that buffer first. A human typing after the prompt behaves the
/// same either way.
pub fn ask_yes_no(color: &Color, question: &str) -> bool {
    print!("{} [y/N]: ", color.paint(YELLOW, question));
    let _ = std::io::stdout().flush();

    let raw = enable_raw_mode().is_ok();
    let mut answer = false;
    loop {
        match event::read() {
            Ok(Event::Key(k)) if k.kind != KeyEventKind::Release => match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y')
                | KeyCode::Char('e') | KeyCode::Char('E') => {
                    answer = true;
                    break;
                }
                KeyCode::Char('n') | KeyCode::Char('N')
                | KeyCode::Enter | KeyCode::Esc => break,
                _ => {}
            },
            Ok(_) => {}
            Err(_) => break,
        }
    }
    if raw {
        let _ = disable_raw_mode();
    }
    println!("{}", if answer { "y" } else { "n" });
    answer
}

// ---------------------------------------------------------------------------
// Screen
// ---------------------------------------------------------------------------

struct ScreenInner {
    /// Is raw mode on — if it is, line endings MUST be `\r\n`.
    raw: bool,
    /// Is there an OVERWRITABLE indicator sitting on the screen's last line.
    has_indicator: bool,
    /// The id of the chip sitting on the last line; its update is written to the
    /// same line.
    last_chip: Option<u64>,
}

/// The single write point. The thread drawing the spinner and the main thread
/// printing the text SHARE the same line; if both do not go through here, the
/// spinner gets mixed into the answer.
pub struct Screen {
    tty: bool,
    inner: Mutex<ScreenInner>,
}

impl Screen {
    pub fn setup() -> Arc<Self> {
        Arc::new(Self {
            tty: std::io::stdout().is_terminal(),
            inner: Mutex::new(ScreenInner { raw: false, has_indicator: false, last_chip: None }),
        })
    }

    pub fn tty(&self) -> bool {
        self.tty
    }

    /// In raw mode `\n` DOES NOT RETURN the cursor to the start of the line; if
    /// we do not translate it, the streaming text drifts to the right like a
    /// staircase.
    fn translate(raw: bool, text: &str) -> String {
        if raw && text.contains('\n') {
            text.replace('\n', "\r\n")
        } else {
            text.to_string()
        }
    }

    /// Prints free text. If an indicator is waiting on the screen it clears it
    /// FIRST — so "remove the indicator when the first token arrives" does not
    /// have to be coded separately, the act of writing removes it.
    pub fn write(&self, text: &str) {
        let mut inner = self.inner.lock().expect("screen lock");
        let mut out = std::io::stdout().lock();
        if inner.has_indicator {
            let _ = out.write_all(CLEAR_LINE.as_bytes());
            inner.has_indicator = false;
        }
        inner.last_chip = None;
        let _ = out.write_all(Self::translate(inner.raw, text).as_bytes());
        let _ = out.flush();
    }

    pub fn line(&self, text: &str) {
        self.write(&format!("{text}\n"));
    }

    /// Draws the indicator on the last line (overwritable).
    fn indicator(&self, text: &str) {
        if !self.tty {
            return;
        }
        let mut inner = self.inner.lock().expect("screen lock");
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(CLEAR_LINE.as_bytes());
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
        inner.has_indicator = true;
        inner.last_chip = None;
    }

    /// Clears a waiting indicator. Called at the end of a turn.
    pub fn clear_indicator(&self) {
        let mut inner = self.inner.lock().expect("screen lock");
        if inner.has_indicator {
            let _ = std::io::stdout().write_all(CLEAR_LINE.as_bytes());
            let _ = std::io::stdout().flush();
            inner.has_indicator = false;
        }
    }

    /// A new chip line.
    fn print_chip(&self, id: u64, line: &str) {
        let mut inner = self.inner.lock().expect("screen lock");
        let mut out = std::io::stdout().lock();
        if inner.has_indicator {
            let _ = out.write_all(CLEAR_LINE.as_bytes());
            inner.has_indicator = false;
        }
        let _ = out.write_all(Self::translate(inner.raw, &format!("{line}\n")).as_bytes());
        let _ = out.flush();
        inner.last_chip = Some(id);
    }

    /// Updates the chip IN PLACE: if the last line still belongs to that chip it
    /// is overwritten, otherwise it is dropped as a new line. The "otherwise"
    /// branch is essential — if some other output got in between, going up and
    /// writing would erase somebody else's line.
    fn update_chip(&self, id: u64, line: &str) {
        let mut inner = self.inner.lock().expect("screen lock");
        let same = inner.last_chip == Some(id);
        let mut out = std::io::stdout().lock();
        if self.tty && same {
            let _ = out.write_all(ONE_UP.as_bytes());
            let _ = out.write_all(CLEAR_LINE.as_bytes());
        }
        let _ = out.write_all(Self::translate(inner.raw, &format!("{line}\n")).as_bytes());
        let _ = out.flush();
        inner.last_chip = Some(id);
    }

    fn raw_mode(&self, on: bool) {
        let mut inner = self.inner.lock().expect("screen lock");
        inner.raw = on;
    }
}

// ---------------------------------------------------------------------------
// The turn indicator — input lock + spinner
// ---------------------------------------------------------------------------

const STATE_THINKING: u8 = 0;
const STATE_QUIET: u8 = 1;

/// The watcher that owns the screen and the keyboard for the duration of one
/// user turn.
///
/// Its lifetime is the turn's lifetime: opened with `start`, closed with
/// `finish`. The reason it is a layer of its own is that raw mode MUST NOT LEAK
/// — a terminal left in raw mode is, from the user's point of view, a broken
/// shell, so `finish` is called on every path and `Drop` does the same job as a
/// second safety net.
pub struct TurnIndicator {
    stop: Arc<AtomicBool>,
    state: Arc<AtomicU8>,
    handle: Option<std::thread::JoinHandle<()>>,
    screen: Arc<Screen>,
    raw_on: bool,
}

impl TurnIndicator {
    /// With no tty it does NOTHING: no raw mode, no thread, no drawing. Scripts
    /// running with piped input take this path and see plain text.
    pub fn start(
        screen: Arc<Screen>,
        cancel: &'static AtomicBool,
        label: &'static str,
    ) -> TurnIndicator {
        if !screen.tty() {
            return TurnIndicator {
                stop: Arc::new(AtomicBool::new(true)),
                state: Arc::new(AtomicU8::new(STATE_QUIET)),
                handle: None,
                screen,
                raw_on: false,
            };
        }
        // If raw mode cannot be turned on (an odd terminal, a restricted
        // environment) THE PROGRAM DOES NOT CRASH: the spinner still turns, only
        // the input lock is missing. The absence of a decoration beats the
        // absence of a shell.
        let raw_on = enable_raw_mode().is_ok();
        screen.raw_mode(raw_on);

        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(AtomicU8::new(STATE_THINKING));
        let handle = {
            let (stop, state, screen) =
                (Arc::clone(&stop), Arc::clone(&state), Arc::clone(&screen));
            std::thread::spawn(move || {
                let started = Instant::now();
                let mut frame = 0usize;
                while !stop.load(Ordering::Relaxed) {
                    // `poll` both waits for a key and keeps the indicator's
                    // clock: it returns immediately if an event arrives, and
                    // after 100 ms if not.
                    if matches!(event::poll(Duration::from_millis(100)), Ok(true))
                        && let Ok(home) = event::read()
                    {
                        swallow_key(&home, cancel, &screen);
                    }
                    if state.load(Ordering::Relaxed) == STATE_THINKING {
                        let elapsed = started.elapsed().as_secs();
                        // The ensō turns in brass, the words stay dim. Two
                        // ticks per quadrant: a full turn takes 800 ms — calm,
                        // not frantic.
                        screen.indicator(&format!(
                            "{}{}{} {}{label}… {elapsed}s · ctrl-c to stop{}",
                            brass_code(),
                            FRAMES[(frame / 2) % FRAMES.len()],
                            reset_code(),
                            dim_code(),
                            reset_code()
                        ));
                        frame += 1;
                    }
                }
            })
        };
        TurnIndicator { stop, state, handle: Some(handle), screen, raw_on }
    }

    /// The first token arrived: the indicator GOES QUIET but the input lock
    /// stays. Generation is still running, and what the user types would still
    /// get into the output.
    pub fn quiet(&self) {
        self.state.store(STATE_QUIET, Ordering::Relaxed);
        self.screen.clear_indicator();
    }

    pub fn finish(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        self.screen.clear_indicator();
        if self.raw_on {
            let _ = disable_raw_mode();
            self.screen.raw_mode(false);
            self.raw_on = false;
        }
    }
}

impl Drop for TurnIndicator {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Handles a key event arriving in raw mode. THE DEFAULT BEHAVIOUR IS TO
/// SWALLOW: no key pressed while generation is running is written to the screen
/// or piles up for the next line.
///
/// Ctrl-C DOES NOT KILL THE PROGRAM. In raw mode the operating system does not
/// produce SIGINT anyway; we interpret the signal ourselves and cancel THE TURN.
/// That is what the user wanted: to cut off an answer that is dragging on, not
/// to close the shell.
fn swallow_key(home: &Event, cancel: &AtomicBool, screen: &Screen) {
    let Event::Key(KeyEvent { code, modifiers, kind, .. }) = home else {
        return;
    };
    // Pressing a key can produce Press + Release; marking the cancellation twice
    // is harmless, printing the message twice is not.
    if *kind != KeyEventKind::Press {
        return;
    }
    let ctrl_c = *code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL);
    if ctrl_c && !cancel.swap(true, Ordering::Relaxed) {
        screen.line(&format!("{}  (cancelling…){}", dim_code(), reset_code()));
    }
}

// ---------------------------------------------------------------------------
// The live reporter — chips on screen the moment they are created
// ---------------------------------------------------------------------------

/// The reporter that prints the chips THE MOMENT THEY ARE CREATED.
///
/// It wraps `TraceCollector`, it does not replace it: the single source of truth
/// for the trace record stays in the core (`world_changed`, the end-of-turn
/// summary are all read from there), and this layer only adds THE SCREEN. Making
/// the core print instead of wrapping it would tie the core to the terminal.
pub struct LiveReporter {
    inner: Arc<TraceCollector>,
    screen: Arc<Screen>,
    /// When off it prints nothing — in single-message mode the chips are printed
    /// in bulk at the end of the turn, let us not double them.
    live: bool,
}

impl LiveReporter {
    pub fn new(inner: Arc<TraceCollector>, screen: Arc<Screen>, live: bool) -> Self {
        Self { inner, screen, live }
    }

    pub fn traces(&self) -> Vec<ToolTrace> {
        self.inner.traces()
    }

    pub fn reset(&self) {
        self.inner.reset();
    }
}

/// The text of a chip line. A running chip ends in "…", a finished chip writes
/// the result; both start with the SAME prefix so that when the line is updated
/// in place the eye does not jump.
///
/// `colored` is a PARAMETER, not a constant: in piped output an ANSI escape
/// shows up inside the text as a raw string (`[2m  ⏺ ...`) and pollutes the line
/// scripts parse. The caller makes the colour decision, just like with
/// `Color::paint`.
pub fn chip_line(trace: &ToolTrace, colored: bool) -> String {
    let text = trace.text.trim_end_matches('…').trim_end();
    let body = match &trace.state {
        ToolState::Running => format!("{text}…"),
        // Some tools ALREADY write the error reason into the chip text ("File
        // not found."); appending it once more produced
        // `File not found. · File not found.`.
        ToolState::Failed(reason) if !text.contains(reason.trim_end_matches('.')) => {
            format!("{text} · {reason}")
        }
        _ => text.to_string(),
    };
    let body = format!("  ⏺ {} · {body}", trace.icon);
    if colored { format!("{}{body}{}", dim_code(), reset_code()) } else { body }
}

impl Reporter for LiveReporter {
    fn start(&self, icon: &str, text: &str) -> TraceId {
        let id = self.inner.start(icon, text);
        if self.live && let Some(trace) = self.inner.traces().into_iter().find(|t| t.id == id) {
            self.screen.print_chip(id.0, &chip_line(&trace, self.screen.tty()));
        }
        id
    }

    fn update(&self, id: TraceId, update: TraceUpdate) {
        self.inner.update(id, update);
        if self.live && let Some(trace) = self.inner.traces().into_iter().find(|t| t.id == id) {
            self.screen.update_chip(id.0, &chip_line(&trace, self.screen.tty()));
        }
        // DIAGNOSTIC DUMP (gated by an env var): what the model produced and
        // what the tool saw — from the REAL trace store, not through a separate
        // path (the lesson of --show-prompt: a diagnostic tool lies the moment
        // it diverges from the production path). As long as it is off the cost
        // and the output are zero; without it, diagnosing tools that carry
        // model-written code (write_code/run_code) in the field was impossible
        // (there is no UI in a terminal where you can touch a chip).
        if tacet_core::env_var("TACET_TRACE_DUMP").is_some()
            && let Some(trace) = self.inner.traces().into_iter().find(|t| t.id == id)
        {
            if let Some(input) = &trace.raw_input {
                eprintln!("\n--- trace {} input ({}) ---\n{input}", id.0, trace.icon);
            }
            if let Some(output) = &trace.raw_output {
                eprintln!("--- trace {} output ---\n{output}\n---", id.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// IT DOES NOT CRASH WITHOUT A TTY. The tests run in a piped environment; if
    /// `start` tried to turn on raw mode there, the whole shell would fall over.
    #[test]
    fn the_indicator_runs_silently_without_a_tty() {
        static CANCEL: AtomicBool = AtomicBool::new(false);
        let screen = Screen::setup();
        let mut indicator = TurnIndicator::start(Arc::clone(&screen), &CANCEL, "thinking");
        indicator.quiet();
        indicator.finish();
        // A second `finish` (it also arrives through Drop) must not panic.
        indicator.finish();
    }

    /// In raw mode line endings are translated; otherwise the text is untouched.
    #[test]
    fn line_endings_are_translated_in_raw_mode() {
        assert_eq!(Screen::translate(true, "a\nb"), "a\r\nb");
        assert_eq!(Screen::translate(false, "a\nb"), "a\nb");
        assert_eq!(Screen::translate(true, "ab"), "ab");
    }

    /// A running chip ends in "…", a finished chip carries the result — the two
    /// lines start with the same prefix so that an in-place update does not make
    /// the eye jump.
    #[test]
    fn the_chip_line_changes_with_the_state() {
        let running = ToolTrace {
            id: TraceId(1),
            icon: "globe".into(),
            text: "searching".into(),
            state: ToolState::Running,
            raw_input: None,
            raw_output: None,
            file_path: None,
        };
        assert!(chip_line(&running, false).contains("searching…"));
        assert!(!chip_line(&running, false).contains('\x1b'), "an uncoloured chip must carry no ANSI");
        assert!(chip_line(&running, true).contains('\x1b'));
        // A tool whose text already ends in "…" must not get TWO ellipses.
        let already = ToolTrace { text: "searching…".into(), ..running.clone() };
        assert!(chip_line(&already, false).ends_with("searching…"), "{}", chip_line(&already, false));
        // If the error reason is already in the chip text it must not be added
        // again.
        let repeated = ToolTrace {
            text: "File not found.".into(),
            state: ToolState::Failed("File not found.".into()),
            ..running.clone()
        };
        assert_eq!(chip_line(&repeated, false).matches("not found").count(), 1);
        let finished = ToolTrace { state: ToolState::Read, text: "27 results".into(), ..running };
        let s = chip_line(&finished, false);
        assert!(s.contains("27 results"), "{s}");
        assert!(!s.contains('…'), "{s}");
    }

    /// Ctrl-C raises the flag; other keys are SWALLOWED (they do not touch it).
    #[test]
    fn ctrl_c_marks_the_cancellation_and_other_keys_are_swallowed() {
        static CANCEL: AtomicBool = AtomicBool::new(false);
        let screen = Screen::setup();
        let letter = Event::Key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        swallow_key(&letter, &CANCEL, &screen);
        assert!(!CANCEL.load(Ordering::Relaxed), "an ordinary key must not cancel");
        let ctrl_c = Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        swallow_key(&ctrl_c, &CANCEL, &screen);
        assert!(CANCEL.load(Ordering::Relaxed), "ctrl-c must cancel");
    }
}
