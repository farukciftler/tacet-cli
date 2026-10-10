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
//!
//! THE INDICATOR IS DRAWN ON STDERR — the same choice the download progress line
//! made, and for the same reason. STDOUT IS THE ANSWER'S CHANNEL: under `--json`
//! it carries one machine-readable document and a `\r\x1b[2K` written into it is
//! corruption, whether or not a human happens to be watching. Writing the
//! decoration to stderr makes that leak IMPOSSIBLE BY CONSTRUCTION rather than by
//! remembering to pass a flag — this repo's recurring failure is a mechanism that
//! is built and then not wired up, and a rule that needs no wiring cannot fail
//! that way.

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tacet_kernel::{Reporter, ToolState, ToolTrace, TraceCollector, TraceId, TraceUpdate};

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
        Self {
            tty: std::io::stdout().is_terminal(),
        }
    }
    /// A palette that paints nothing, whatever fd 1 is. For tests that measure
    /// the bytes a line carries: `setup` answers for the developer's terminal
    /// when `cargo test` runs in one.
    #[cfg(test)]
    pub fn plain() -> Self {
        Self { tty: false }
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
                KeyCode::Char('y')
                | KeyCode::Char('Y')
                | KeyCode::Char('e')
                | KeyCode::Char('E') => {
                    answer = true;
                    break;
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Enter | KeyCode::Esc => break,
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
    /// May the indicator be drawn at all — BOTH streams must be terminals.
    ///
    /// stdout, because a redirected stdout means somebody is reading the bytes
    /// and the erase sequences would land in their file. stderr, because that is
    /// where the indicator is actually written: with `2>log` the line would fill
    /// a log file with carriage returns. Requiring both keeps the decoration on
    /// the screen and only on the screen.
    indicator_tty: bool,
    inner: Mutex<ScreenInner>,
}

impl Screen {
    pub fn setup() -> Arc<Self> {
        Arc::new(Self {
            tty: std::io::stdout().is_terminal(),
            indicator_tty: std::io::stdout().is_terminal() && std::io::stderr().is_terminal(),
            inner: Mutex::new(ScreenInner {
                raw: false,
                has_indicator: false,
                last_chip: None,
            }),
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

    /// Removes ANSI CSI sequences from text on its way to a NON-TERMINAL stdout.
    ///
    /// MEASURED, and it is why the interactive shell had to be run rather than
    /// reasoned about: `tacet chat --engine fake -m hello | od -c` ended with
    ///
    /// ```text
    /// f a k e   e n g i n e ) 033 [ 0 m
    /// ```
    ///
    /// a raw reset in the middle of the answer. `Color::paint` already checks for
    /// a terminal, so the coloured parts were clean — but `paper_code()` and
    /// `RESET` are written straight through `Screen::write`, which only ever
    /// translated line endings. Anyone redirecting the answer to a file, or
    /// piping it to another program, got the escape as text. Measured on macOS
    /// and on Ubuntu 24.04, 4 Sep 2026, so it is not a platform quirk.
    ///
    /// STRIPPED HERE RATHER THAN AT THE CALL SITES, deliberately. There are
    /// several places that emit a reset or a theme colour and a guard at each is
    /// a guard one of them will be missing after the next edit — this is the one
    /// door stdout goes through. The indicator writes to STDERR by another path
    /// and is untouched, which is correct: it is drawn for a person, and a person
    /// is a terminal.
    fn strip_ansi(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '\x1b' {
                out.push(c);
                continue;
            }
            // CSI: ESC '[' … final byte in @..~. Anything else after ESC is a
            // two-character sequence; both are dropped whole.
            if chars.peek() == Some(&'[') {
                chars.next();
                for f in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&f) {
                        break;
                    }
                }
            } else {
                chars.next();
            }
        }
        out
    }

    /// Erases the indicator line, IN THE STREAM IT WAS DRAWN IN.
    ///
    /// The erase has to travel with the drawing: the cursor belongs to the
    /// terminal, not to a stream, but the BYTES belong to a stream, and sending
    /// `\r\x1b[2K` down stdout to clean up something stderr drew is exactly the
    /// leak the module header forbids. Idempotent — nothing is written when no
    /// indicator is standing.
    fn wipe_indicator(inner: &mut ScreenInner) {
        if inner.has_indicator {
            let mut err = std::io::stderr().lock();
            let _ = err.write_all(CLEAR_LINE.as_bytes());
            let _ = err.flush();
            inner.has_indicator = false;
        }
    }

    /// Prints free text. If an indicator is waiting on the screen it clears it
    /// FIRST — so "remove the indicator when the first token arrives" does not
    /// have to be coded separately, the act of writing removes it.
    pub fn write(&self, text: &str) {
        let mut inner = self.inner.lock().expect("screen lock");
        Self::wipe_indicator(&mut inner);
        let mut out = std::io::stdout().lock();
        inner.last_chip = None;
        let translated = Self::translate(inner.raw, text);
        let painted = if self.tty {
            translated
        } else {
            Self::strip_ansi(&translated)
        };
        let _ = out.write_all(painted.as_bytes());
        let _ = out.flush();
    }

    pub fn line(&self, text: &str) {
        self.write(&format!("{text}\n"));
    }

    /// Draws the indicator on the last line (overwritable, ON STDERR).
    fn indicator(&self, text: &str) {
        if !self.indicator_tty {
            return;
        }
        let mut inner = self.inner.lock().expect("screen lock");
        let mut err = std::io::stderr().lock();
        let _ = err.write_all(CLEAR_LINE.as_bytes());
        let _ = err.write_all(text.as_bytes());
        let _ = err.flush();
        inner.has_indicator = true;
        inner.last_chip = None;
    }

    /// Clears a waiting indicator. Called at the end of a turn.
    pub fn clear_indicator(&self) {
        let mut inner = self.inner.lock().expect("screen lock");
        Self::wipe_indicator(&mut inner);
    }

    /// A new chip line.
    fn print_chip(&self, id: u64, line: &str) {
        let mut inner = self.inner.lock().expect("screen lock");
        Self::wipe_indicator(&mut inner);
        let mut out = std::io::stdout().lock();
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

    /// Something was written to the terminal OUTSIDE this screen — a consent
    /// question on stderr. The last line no longer belongs to the last chip,
    /// and an in-place update would go one line up and erase the user's answer
    /// instead of the chip; forgetting the chip makes its update a new line.
    fn forget_last_chip(&self) {
        let mut inner = self.inner.lock().expect("screen lock");
        inner.last_chip = None;
    }
}

// ---------------------------------------------------------------------------
// The turn indicator — input lock + spinner
// ---------------------------------------------------------------------------

const STATE_THINKING: u8 = 0;
const STATE_QUIET: u8 = 1;

/// WHAT THE SHELL IS DOING RIGHT NOW — the word that stands next to the ensō.
///
/// WHY IT EXISTS: between pressing enter and seeing an answer the screen showed a
/// spinning mark and nothing else, for anywhere up to half a minute. Three
/// different waits hide in there and they need three different amounts of
/// patience: reading 2.3 GB of weights off disk (once per process), pushing a few
/// thousand prompt tokens through the model, and a tool that went to the network.
/// A user who cannot tell them apart reads all three as "frozen".
///
/// EVERY NUMBER HERE IS ONE THE CALLER ALREADY HAS. There is deliberately no
/// percentage and no "time remaining": the same rule the download progress line
/// follows (see `progress_text` in main.rs) — an unmeasured figure on the
/// interface is worse than no figure, because it will be believed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Weights are being read off disk. THE LONGEST WAIT and the only one that
    /// happens once per process, which is why it is worth naming: the second
    /// turn will not do it again. `model` may be empty when the name is not
    /// known at the call site.
    Loading { model: String },
    /// The prompt is being built/consumed. `tokens` is the CLI's own prompt size,
    /// which is `TokenCounter::estimate` — an estimate, and the line says so with
    /// a `~` exactly as `--show-prompt` does. If an exact count ever reaches this
    /// layer, the tilde is the thing to drop.
    Prefill { tokens: usize },
    /// Tokens are coming out. `tokens` MUST BE A COUNT THE CALLER KEPT, not a
    /// guess: the engine's streaming callback fires at most once per accepted
    /// token, so counting callbacks is a true lower bound (it misses only the
    /// steps whose decode added no new text — see `candle_engine::run_loop`).
    Generating { tokens: usize },
    /// A tool is running. THE NAME IS THE POINT: "working…" for eight seconds
    /// says nothing, "running web_search…" says the machine went to the network.
    Tool { name: String },
    /// A tool's own check before it acts — write_code's syntax pass, run_code's
    /// sandbox setup. Separated from `Tool` because it is the step that fails,
    /// and a failure is much easier to read when the screen already said which
    /// step was in progress.
    Verifying { name: String },
    /// Nothing more specific is known; the indicator falls back to the label it
    /// was started with.
    Thinking,
}

/// The words for a stage. PURE — this is the part worth testing, and testing it
/// must not need a terminal.
fn stage_words(stage: &Stage, fallback: &str) -> String {
    match stage {
        Stage::Loading { model } if model.is_empty() => "loading the model".to_string(),
        Stage::Loading { model } => format!("loading {model}"),
        Stage::Prefill { tokens } => format!("prefill ~{tokens} tok"),
        // A zero here is the first instant of generation, before anything has
        // been counted. Printing "0 tok" would be a true number that reads as a
        // stall, so the count joins the line only once there is one.
        Stage::Generating { tokens } if *tokens == 0 => "generating".to_string(),
        Stage::Generating { tokens } => format!("generating {tokens} tok"),
        Stage::Tool { name } => format!("running {name}"),
        Stage::Verifying { name } => format!("checking {name}"),
        Stage::Thinking => fallback.to_string(),
    }
}

/// The hint at the end of the line. First thing dropped when the terminal is
/// narrow: it is a reminder, the stage is the news.
const STOP_HINT: &str = " · ctrl-c to stop";

/// Builds the whole indicator line. PURE: no clock, no terminal, no shared
/// state — everything it needs is an argument, so the layout can be measured in
/// a test that never opens a tty.
///
/// IT NEVER RETURNS MORE THAN `width - 1` VISIBLE COLUMNS. That last column is
/// not caution, it is the rule this line lives by: at exactly `width` characters
/// terminals wrap, the next `\r` then lands on the WRONG line, and the indicator
/// starts leaving a trail of half-erased lines behind it — the same failure the
/// download progress line was written to avoid.
fn indicator_line(
    frame: &str,
    stage: &Stage,
    fallback: &str,
    elapsed_secs: u64,
    width: usize,
    colored: bool,
) -> String {
    let clock = format!("{elapsed_secs}s");
    let words = stage_words(stage, fallback);
    // The ensō and the space after it are outside `rest`, because they are
    // painted in a different ink; every truncation below works on `rest` alone.
    let head_cols = frame.chars().count() + 1;
    let budget = width.saturating_sub(1).saturating_sub(head_cols);

    let full = format!("{words}… {clock}{STOP_HINT}");
    let rest = if full.chars().count() <= budget {
        full
    } else {
        // Step 1: drop the hint.
        let short = format!("{words}… {clock}");
        if short.chars().count() <= budget {
            short
        } else {
            // Step 2: cut the words, keeping the clock — a wait with no elapsed
            // time is the frozen screen this whole thing exists to fix.
            let tail = format!("… {clock}");
            let room = budget.saturating_sub(tail.chars().count());
            let cut: String = words.chars().take(room).collect();
            let line = format!("{cut}{tail}");
            // Step 3: a terminal too narrow even for the clock. Whatever fits.
            line.chars().take(budget).collect()
        }
    };
    if colored {
        format!(
            "{}{frame}{} {}{rest}{}",
            brass_code(),
            reset_code(),
            dim_code(),
            reset_code()
        )
    } else {
        format!("{frame} {rest}")
    }
}

/// How wide the terminal is right now. Asked EVERY DRAW rather than once: a
/// window resized mid-answer would otherwise keep wrapping until the turn ended.
/// 80 is the fallback when the size cannot be read (not a terminal, a platform
/// that does not answer) — it is the conventional width, not a measurement, and
/// it only ever makes the line shorter than it could be.
fn terminal_width() -> usize {
    crossterm::terminal::size()
        .map(|(w, _)| w as usize)
        .unwrap_or(80)
}

/// Does a new stage take the line back from a running answer?
///
/// THE ONE CASE THAT MUST NOT: the token counter. Once the answer is flowing,
/// the last line belongs to the text; an indicator redrawing itself there would
/// erase the words being written. Every OTHER stage marks a phase that is
/// genuinely silent — a tool that went to the network, the next round's prefill —
/// and taking the line back is precisely the point of naming it.
fn stage_wakes(quiet: bool, stage: &Stage) -> bool {
    !quiet || !matches!(stage, Stage::Generating { .. })
}

/// The terminal's INPUT side, as the indicator uses it: raw mode and the next
/// key.
///
/// A TRAIT FOR ONE REASON: the suspend/resume state machine below is what
/// audit H1 was about, and a state machine that can only be exercised with a
/// real terminal attached is one that `cargo test` never runs. Production
/// passes `Crossterm`; the tests pass a keyboard that counts.
trait Keyboard: Send + Sync {
    /// Turns raw mode on or off; `true` when the terminal is now in the
    /// requested mode.
    fn set_raw(&self, on: bool) -> bool;
    /// The next input event, waiting at most `wait`. `None` on a timeout.
    fn next_event(&self, wait: Duration) -> Option<Event>;
}

struct Crossterm;

impl Keyboard for Crossterm {
    fn set_raw(&self, on: bool) -> bool {
        if on {
            enable_raw_mode().is_ok()
        } else {
            disable_raw_mode().is_ok()
        }
    }

    fn next_event(&self, wait: Duration) -> Option<Event> {
        match event::poll(wait) {
            Ok(true) => event::read().ok(),
            Ok(false) => None,
            // A poll that fails does so at once, and the drawing loop calls this
            // as its clock: without the sleep a broken terminal would spin a core
            // at 100% for the whole turn.
            Err(_) => {
                std::thread::sleep(wait);
                None
            }
        }
    }
}

/// The part of the indicator that a suspend stops and a resume restarts.
struct Spinner {
    stop: Arc<AtomicBool>,
    handle: std::thread::JoinHandle<()>,
}

struct RunState {
    spinner: Option<Spinner>,
    raw_on: bool,
    /// How many prompts are holding the indicator suspended. A count, not a
    /// flag: an MCP question can arrive while an approval is being asked for
    /// on another thread, and the first one to finish must not restart a
    /// spinner the second one is still sitting under.
    paused: usize,
    finished: bool,
}

/// Everything the indicator owns, shared between the turn loop (which holds
/// the `TurnIndicator`) and a consent prompt (which finds it through
/// `ACTIVE_INDICATOR`).
struct IndicatorCore {
    state: Arc<AtomicU8>,
    stage: Arc<Mutex<Stage>>,
    screen: Arc<Screen>,
    keyboard: Arc<dyn Keyboard>,
    cancel: &'static AtomicBool,
    label: &'static str,
    /// Kept across a suspend so the clock does not restart at 0s after the user
    /// answers a question: the turn did not start again, it waited.
    started: Instant,
    run: Mutex<RunState>,
}

/// THE INDICATOR THAT IS ON SCREEN RIGHT NOW, if any.
///
/// WHY A GLOBAL and not a handle threaded to the gates: the consent prompts
/// (`TerminalApproval`, `TerminalWriteConfirm`, `TerminalAsk`) are built once,
/// at session start, deep inside the executor and the tool catalog, while an
/// indicator lives for one turn. Threading the turn's indicator through
/// `ApprovalGate::request` would change a trait three crates implement, for a
/// fact that is about the terminal — and there is only one terminal. `Weak`, so
/// a finished indicator that somebody forgot to unregister cannot be revived.
static ACTIVE_INDICATOR: Mutex<Option<std::sync::Weak<IndicatorCore>>> = Mutex::new(None);

/// Serialises the tests that touch `ACTIVE_INDICATOR`. The indicator tests
/// register a real active indicator, and anything that opens a `Prompt` at the
/// same moment (the gate tests in main.rs) would suspend it under their feet.
#[cfg(test)]
pub(crate) static ACTIVE_INDICATOR_TEST: Mutex<()> = Mutex::new(());

impl IndicatorCore {
    fn spawn_spinner(core: &Arc<IndicatorCore>) -> Spinner {
        let stop = Arc::new(AtomicBool::new(false));
        let (thread_stop, state, screen, stage, keyboard, cancel, label, started) = (
            Arc::clone(&stop),
            Arc::clone(&core.state),
            Arc::clone(&core.screen),
            Arc::clone(&core.stage),
            Arc::clone(&core.keyboard),
            core.cancel,
            core.label,
            core.started,
        );
        let handle = std::thread::spawn(move || {
            let mut frame = 0usize;
            while !thread_stop.load(Ordering::Relaxed) {
                // The key wait doubles as the indicator's clock: it returns at
                // once if an event arrives, and after 100 ms if not.
                if let Some(home) = keyboard.next_event(Duration::from_millis(100)) {
                    swallow_key(&home, cancel, &screen);
                }
                // A stop that arrived during the wait must not draw one more
                // frame: the suspend is about to put a question on this line.
                if thread_stop.load(Ordering::Relaxed) {
                    break;
                }
                if state.load(Ordering::Relaxed) == STATE_THINKING {
                    // The stage is CLONED and the lock released before
                    // drawing: `screen.indicator` takes a lock of its own,
                    // and holding two while writing to a terminal is how a
                    // shell ends up deadlocked against its own spinner.
                    let now = stage.lock().expect("stage lock").clone();
                    // The ensō turns in brass, the words stay dim. Two
                    // ticks per quadrant: a full turn takes 800 ms — calm,
                    // not frantic. A PHASE CHANGE LEAVES NO TRACE: the line
                    // is erased and rewritten in place, so a turn that went
                    // through five stages still occupies one line.
                    screen.indicator(&indicator_line(
                        FRAMES[(frame / 2) % FRAMES.len()],
                        &now,
                        label,
                        started.elapsed().as_secs(),
                        terminal_width(),
                        true,
                    ));
                    frame += 1;
                }
            }
        });
        Spinner { stop, handle }
    }

    fn halt(spinner: Option<Spinner>) {
        if let Some(s) = spinner {
            s.stop.store(true, Ordering::Relaxed);
            let _ = s.handle.join();
        }
    }

    /// HANDS THE TERMINAL TO A QUESTION (audit H1).
    ///
    /// MEASURED, under a pty driving the real binary: with the indicator left
    /// running, the approval prompt was printed and then erased by the next
    /// frame's `\r\x1b[2K`; the `y` and the `\r` the user typed were read by the
    /// spinner thread's `event::read` and swallowed like any other key; and the
    /// `read_line` waiting for the answer sat on a raw-mode terminal that never
    /// turns `\r` into the `\n` it was waiting for. Four seconds of typing did
    /// nothing; the shell hung. Each of the three steps below removes one of
    /// those causes, and the order matters — the thread must be GONE before the
    /// typeahead is drained, or it could take a key the drain was about to.
    fn suspend(&self) {
        let mut run = self.run.lock().expect("indicator run lock");
        run.paused += 1;
        if run.paused > 1 || run.finished {
            return;
        }
        // 1. Nobody reads the keyboard but the question.
        Self::halt(run.spinner.take());
        if run.raw_on {
            // TYPEAHEAD MUST NOT ANSWER A CONSENT QUESTION. A key pressed
            // during generation is swallowed by design; one that arrived in the
            // last 100 ms is still queued, and a `y` sitting there would be read
            // as the answer to a question the user has not yet seen. Drained
            // under the same rule as every other key of the turn (ctrl-c still
            // cancels), and bounded so a held key cannot keep us here.
            for _ in 0..1024 {
                match self.keyboard.next_event(Duration::ZERO) {
                    Some(key) => swallow_key(&key, self.cancel, &self.screen),
                    None => break,
                }
            }
        }
        // 2. Nothing will erase the question.
        self.screen.clear_indicator();
        self.screen.forget_last_chip();
        // 3. The terminal is a line terminal again: echo on, and the enter key
        // arrives as the newline a line reader waits for.
        if run.raw_on {
            self.keyboard.set_raw(false);
            self.screen.raw_mode(false);
            run.raw_on = false;
        }
    }

    fn resume(core: &Arc<IndicatorCore>) {
        let mut run = core.run.lock().expect("indicator run lock");
        if run.paused == 0 {
            return;
        }
        run.paused -= 1;
        if run.paused > 0 || run.finished {
            return;
        }
        // The input lock comes back for the rest of the turn: a tool still has
        // to finish, and keys pressed while it runs must not spill out.
        run.raw_on = core.keyboard.set_raw(true);
        core.screen.raw_mode(run.raw_on);
        run.spinner = Some(Self::spawn_spinner(core));
    }

    fn finish(core: &Arc<IndicatorCore>) {
        let mut run = core.run.lock().expect("indicator run lock");
        if run.finished {
            return;
        }
        run.finished = true;
        Self::halt(run.spinner.take());
        core.screen.clear_indicator();
        if run.raw_on {
            core.keyboard.set_raw(false);
            core.screen.raw_mode(false);
            run.raw_on = false;
        }
        drop(run);
        let mut active = ACTIVE_INDICATOR.lock().expect("active indicator lock");
        if active
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
            .is_some_and(|a| Arc::ptr_eq(&a, core))
        {
            *active = None;
        }
    }
}

/// While this is alive, no indicator draws, reads keys, or holds raw mode.
///
/// Returned by `pause_indicator`; dropping it gives the turn its indicator back
/// — unless the turn ended in the meantime, in which case there is nothing to
/// give back and nothing is restarted.
pub struct IndicatorPause(Option<Arc<IndicatorCore>>);

impl Drop for IndicatorPause {
    fn drop(&mut self) {
        if let Some(core) = self.0.take() {
            IndicatorCore::resume(&core);
        }
    }
}

/// Suspends the indicator that is on screen, if there is one. EVERY PROMPT
/// THAT READS AN ANSWER DURING A TURN GOES THROUGH HERE (see `Prompt::open`);
/// with no indicator running it costs a lock and does nothing.
pub fn pause_indicator() -> IndicatorPause {
    let core = ACTIVE_INDICATOR
        .lock()
        .expect("active indicator lock")
        .as_ref()
        .and_then(std::sync::Weak::upgrade);
    if let Some(core) = &core {
        core.suspend();
    }
    IndicatorPause(core)
}

/// The watcher that owns the screen and the keyboard for the duration of one
/// user turn.
///
/// Its lifetime is the turn's lifetime: opened with `start`, closed with
/// `finish`. The reason it is a layer of its own is that raw mode MUST NOT LEAK
/// — a terminal left in raw mode is, from the user's point of view, a broken
/// shell, so `finish` is called on every path and `Drop` does the same job as a
/// second safety net.
///
/// IT STEPS ASIDE FOR A QUESTION. A tool that needs the user's consent runs
/// while this is open; `pause_indicator` (used by every consent prompt) stops
/// the spinner and gives the terminal back for the length of the question.
pub struct TurnIndicator {
    core: Arc<IndicatorCore>,
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
            return Self::disabled(screen);
        }
        Self::start_with(screen, cancel, label, Arc::new(Crossterm))
    }

    fn start_with(
        screen: Arc<Screen>,
        cancel: &'static AtomicBool,
        label: &'static str,
        keyboard: Arc<dyn Keyboard>,
    ) -> TurnIndicator {
        // If raw mode cannot be turned on (an odd terminal, a restricted
        // environment) THE PROGRAM DOES NOT CRASH: the spinner still turns, only
        // the input lock is missing. The absence of a decoration beats the
        // absence of a shell.
        let raw_on = keyboard.set_raw(true);
        screen.raw_mode(raw_on);
        let core = Arc::new(IndicatorCore {
            state: Arc::new(AtomicU8::new(STATE_THINKING)),
            stage: Arc::new(Mutex::new(Stage::Thinking)),
            screen,
            keyboard,
            cancel,
            label,
            started: Instant::now(),
            run: Mutex::new(RunState {
                spinner: None,
                raw_on,
                paused: 0,
                finished: false,
            }),
        });
        let spinner = IndicatorCore::spawn_spinner(&core);
        core.run.lock().expect("indicator run lock").spinner = Some(spinner);
        *ACTIVE_INDICATOR.lock().expect("active indicator lock") = Some(Arc::downgrade(&core));
        TurnIndicator { core }
    }

    /// AN INDICATOR THAT DOES NOTHING: no raw mode, no thread, no drawing, and
    /// every method on it is a no-op the caller does not have to guard.
    ///
    /// TWO CALLERS, TWO DIFFERENT REASONS. `start` returns this when there is no
    /// tty — there is nothing to draw on. `main.rs` asks for it directly under
    /// `--json`, and that reason is not the same: a `--json` run in a terminal
    /// HAS a tty, but the user asked for a machine-readable run, and a spinner
    /// scribbling stage words over their stderr is decoration they did not ask
    /// for. The cost of the second case is the input lock going too; that is
    /// accepted, because the thing on the other end of a `--json` run is a
    /// program, not a pair of hands.
    ///
    /// WHY THE SAME TYPE and not an `Option`: `finish`/`quiet`/`stage` are
    /// called from a dozen places in the turn loop, several of them on error
    /// paths. An `Option` would put a `if let Some` at every one of them, and
    /// the one that got forgotten would be on the path nobody exercises.
    ///
    /// IT IS NEVER REGISTERED as the active indicator: there is nothing for a
    /// prompt to suspend.
    pub fn disabled(screen: Arc<Screen>) -> TurnIndicator {
        TurnIndicator {
            core: Arc::new(IndicatorCore {
                state: Arc::new(AtomicU8::new(STATE_QUIET)),
                stage: Arc::new(Mutex::new(Stage::Thinking)),
                screen,
                keyboard: Arc::new(Crossterm),
                cancel: &NEVER_CANCELLED,
                label: "",
                started: Instant::now(),
                run: Mutex::new(RunState {
                    spinner: None,
                    raw_on: false,
                    paused: 0,
                    finished: false,
                }),
            }),
        }
    }

    /// Says which phase the turn is in. The whole caller-facing API is this one
    /// method plus `quiet`/`finish`: the flow lives in `main.rs` and the screen
    /// lives here, so anything wider would drag terminal knowledge back into the
    /// turn loop.
    ///
    /// SAFE TO CALL WITHOUT A TTY and safe to call at any rate — with no
    /// terminal there is no thread reading it, and the drawing thread reads
    /// whatever the latest stage is on its own 100 ms beat rather than once per
    /// call. So a per-token `Generating` update costs a lock and nothing else; it
    /// does not paint the screen a thousand times.
    pub fn stage(&self, stage: Stage) {
        let quiet = self.core.state.load(Ordering::Relaxed) == STATE_QUIET;
        let wakes = stage_wakes(quiet, &stage);
        *self.core.stage.lock().expect("stage lock") = stage;
        if wakes {
            self.core.state.store(STATE_THINKING, Ordering::Relaxed);
        }
    }

    /// The first token arrived: the indicator GOES QUIET but the input lock
    /// stays. Generation is still running, and what the user types would still
    /// get into the output.
    ///
    /// The line now belongs to the answer. A later `stage` call takes it back
    /// for a phase that is silent again (a tool running) but NOT for a token
    /// count — see `stage_wakes`.
    pub fn quiet(&self) {
        self.core.state.store(STATE_QUIET, Ordering::Relaxed);
        self.core.screen.clear_indicator();
    }

    pub fn finish(&mut self) {
        IndicatorCore::finish(&self.core);
        // A disabled indicator owns nothing, but its screen may still carry an
        // indicator someone else drew; the old `finish` cleared it on every
        // path and callers rely on that.
        self.core.screen.clear_indicator();
    }
}

/// The cancel flag of an indicator that reads no keys.
static NEVER_CANCELLED: AtomicBool = AtomicBool::new(false);

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
    let Event::Key(KeyEvent {
        code,
        modifiers,
        kind,
        ..
    }) = home
    else {
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
        Self {
            inner,
            screen,
            live,
        }
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
    // EVERY FIELD GOES THROUGH `one_line`, HERE, and not only the one that
    // looked dangerous (audit M13). The core already cleans `text` on its way
    // in, so this used to clean nothing — and the failure REASON rode past it
    // untouched: an `edit_document` call with `source_ref:
    // "\u001b]52;c;…\u0007"` failed with the reference quoted in its reason,
    // and the OSC 52 sequence reached the pty raw (a clipboard write and a
    // title change); an HTTP error body put `\x1b[2J` on the screen. A chip is
    // one line by construction, so `one_line` and not `printable`.
    let text = one_line(&trace.text);
    let text = text.trim_end_matches('…').trim_end();
    let body = match &trace.state {
        ToolState::Running => format!("{text}…"),
        // Some tools ALREADY write the error reason into the chip text ("File
        // not found."); appending it once more produced
        // `File not found. · File not found.`. Compared AFTER sanitising, so a
        // reason the text already carries is still recognised as a repeat.
        ToolState::Failed(reason) => {
            let reason = one_line(reason);
            if text.contains(reason.trim_end_matches('.')) {
                text.to_string()
            } else {
                format!("{text} · {reason}")
            }
        }
        _ => text.to_string(),
    };
    let body = format!("  ⏺ {} · {body}", one_line(&trace.icon));
    if colored {
        format!("{}{body}{}", dim_code(), reset_code())
    } else {
        body
    }
}

/// A copy of untrusted text that a terminal will DISPLAY rather than OBEY.
///
/// Newlines and tabs survive (a dump is meant to be readable); every other
/// control character becomes a visible marker. `is_control()` covers C0, DEL
/// and the C1 range — U+009B is a one-character CSI and is exactly as dangerous
/// as `ESC [`.
pub fn printable(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\n' | '\t' => c,
            c if c.is_control() => '\u{fffd}',
            c => c,
        })
        .collect()
}

/// Like `printable`, but a newline is a control character too.
///
/// FOR PROMPT LINES. Where the reader is about to answer a question, an extra
/// line is a weapon of its own: it can paint a second, fake prompt under the
/// real one, or scroll the real one out of view. A line that asks for consent
/// must be exactly one line.
pub fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { '·' } else { c })
        .collect()
}

// ---------------------------------------------------------------------------
// Consent prompts — the one door every "may I?" goes through
// ---------------------------------------------------------------------------

/// Where the answer to a consent question comes from.
///
/// A TRAIT so the gates can be held to their two promises in a unit test —
/// they ask nobody when nobody can answer, and the indicator is out of the way
/// while they read — without a terminal attached. Production uses `Terminal`.
pub trait Console {
    /// Is there a person on the other end who could answer? `false` for a pipe.
    fn is_terminal(&self) -> bool;
    /// One answer line, or `None` when the person declined to give one
    /// (escape, ctrl-d, the input ended).
    fn read_answer(&self) -> Option<String>;
}

/// The real terminal: stdin, read THROUGH CROSSTERM.
///
/// WHY CROSSTERM AND NOT `stdin().read_line` — the same reason `ask_yes_no`
/// gives: the input field and the indicator both read the tty through
/// crossterm, which keeps bytes it has already pulled off the fd in its own
/// buffer, where a std `read_line` would never see them and would hang.
pub struct Terminal;

impl Console for Terminal {
    fn is_terminal(&self) -> bool {
        std::io::stdin().is_terminal()
    }

    fn read_answer(&self) -> Option<String> {
        collect_answer(std::iter::from_fn(|| event::read().ok()))
    }
}

/// Turns key events into one answer line.
///
/// THE LINE ENDS ON `\r` AND ON `\n` ALIKE. Which of the two the enter key
/// produces depends on the terminal's mode — `\r` raw, `\n` cooked — and the
/// old `read_line` answered only one of them; under the spinner's raw mode
/// that was the half of H1 that turned a typed `y⏎` into nothing. Crossterm
/// reports `\r` as Enter in every mode and `\n` as Enter in cooked mode but as
/// ctrl-J in raw mode, so ctrl-J is an enter here too.
///
/// PURE over its input so the line discipline can be tested without a tty.
fn collect_answer(events: impl IntoIterator<Item = Event>) -> Option<String> {
    let mut line = String::new();
    for event in events {
        match event {
            // A paste answers with its first line only — a consent question is
            // one line, and the rest of a paste is not an answer to it.
            Event::Paste(text) => {
                line.push_str(text.lines().next().unwrap_or(""));
                return Some(line);
            }
            Event::Key(k) if k.kind != KeyEventKind::Release => {
                let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                match k.code {
                    KeyCode::Enter => return Some(line),
                    KeyCode::Char('j') | KeyCode::Char('m') if ctrl => return Some(line),
                    // Declining is always available and always means no.
                    KeyCode::Char('c') | KeyCode::Char('d') if ctrl => return None,
                    KeyCode::Esc => return None,
                    KeyCode::Backspace => {
                        line.pop();
                    }
                    KeyCode::Char(c) if !ctrl => line.push(c),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    // The input ended (or the terminal stopped answering) mid-line: nobody said
    // yes.
    None
}

/// Is this answer a yes? Only `y` and `yes`, any case — the `[y/N]` the
/// question prints. Everything else, including nothing, is a no.
pub fn is_yes(answer: Option<&str>) -> bool {
    answer.is_some_and(|a| matches!(a.trim().to_lowercase().as_str(), "y" | "yes"))
}

/// An open consent question: nobody else is reading the keyboard or drawing on
/// the line while it exists.
pub struct Prompt<'a> {
    console: &'a dyn Console,
    _pause: IndicatorPause,
}

impl<'a> Prompt<'a> {
    /// Opens a question, or refuses to.
    ///
    /// `None` WHEN NOBODY CAN ANSWER (audit L31), and in that case NOT ONE BYTE
    /// of stdin is read. Without a terminal, stdin is the user's script — with
    /// no `--message` the shell reads its MESSAGES from it line by line — so a
    /// `read_line` here took the next message and treated it as the answer:
    /// `printf 'run it\ny\nsecond message\n' | tacet chat` approved a tainted
    /// `shell` call with the `y` that was meant to be a message (measured on
    /// the scripted engine, 10 Oct 2026). The caller treats `None` as a no.
    ///
    /// Otherwise the turn's indicator is suspended (audit H1) for as long as
    /// the returned `Prompt` lives — open it BEFORE printing the question, or
    /// the spinner's next frame erases it.
    pub fn open(console: &'a dyn Console) -> Option<Prompt<'a>> {
        if !console.is_terminal() {
            return None;
        }
        Some(Prompt {
            console,
            _pause: pause_indicator(),
        })
    }

    /// One answer line. The question must already be on screen and flushed.
    pub fn read(&self) -> Option<String> {
        self.console.read_answer()
    }
}

impl Reporter for LiveReporter {
    fn start(&self, icon: &str, text: &str) -> TraceId {
        let id = self.inner.start(icon, text);
        if self.live
            && let Some(trace) = self.inner.traces().into_iter().find(|t| t.id == id)
        {
            self.screen
                .print_chip(id.0, &chip_line(&trace, self.screen.tty()));
        }
        id
    }

    fn update(&self, id: TraceId, update: TraceUpdate) {
        self.inner.update(id, update);
        if self.live
            && let Some(trace) = self.inner.traces().into_iter().find(|t| t.id == id)
        {
            self.screen
                .update_chip(id.0, &chip_line(&trace, self.screen.tty()));
        }
        // DIAGNOSTIC DUMP (gated by an env var): what the model produced and
        // what the tool saw — from the REAL trace store, not through a separate
        // path (the lesson of --show-prompt: a diagnostic tool lies the moment
        // it diverges from the production path). As long as it is off the cost
        // and the output are zero; without it, diagnosing tools that carry
        // model-written code (write_code/run_code) in the field was impossible
        // (there is no UI in a terminal where you can touch a chip).
        if tacet_kernel::env_var("TACET_TRACE_DUMP").is_some()
            && let Some(trace) = self.inner.traces().into_iter().find(|t| t.id == id)
        {
            // THE RECORD IS RAW, THE COPY ON SCREEN IS NOT. `raw_input` holds
            // the model's bytes verbatim on purpose — a diagnostic that
            // sanitises its own record is useless. But this is the one place
            // that puts those bytes on a terminal, and a terminal EXECUTES
            // escape sequences: a dump of a poisoned tool call would erase the
            // very lines the developer opened the dump to read.
            if let Some(input) = &trace.raw_input {
                eprintln!(
                    "\n--- trace {} input ({}) ---\n{}",
                    id.0,
                    trace.icon,
                    printable(input)
                );
            }
            if let Some(output) = &trace.raw_output {
                eprintln!("--- trace {} output ---\n{}\n---", id.0, printable(output));
            }
        }
    }
}

// ── The interactive menu ──────────────────────────────────────────────────

/// One row of a menu: a label, the value sitting on it right now, and a hint.
pub struct MenuRow {
    pub label: String,
    /// What the setting currently is. Drawn in the accent colour, because it is
    /// the thing the reader is looking for.
    pub value: String,
    pub hint: String,
}

/// What the reader did.
pub enum MenuOutcome {
    /// Enter on this row.
    Chose(usize),
    /// Esc, q, or ctrl-c.
    Cancelled,
}

/// Draws a list, moves through it with the arrows, and returns on Enter.
///
/// WHY THIS EXISTS: settings used to be `get`/`set` with a key and a value typed
/// by hand, which means the reader has to know the key, the spelling, and the
/// legal values before they can change anything — three chances to be wrong
/// before the first success. A list shows all three at once and Enter is the
/// only verb.
///
/// It draws with RELATIVE cursor motion and erases what it drew, exactly like
/// the input field: the terminal may scroll between frames, so an absolute
/// position would point at the wrong row.
pub fn menu(color: &Color, title: &str, rows: &[MenuRow], start: usize) -> MenuOutcome {
    if rows.is_empty() {
        return MenuOutcome::Cancelled;
    }
    // NO TTY, NO MENU. Drawing it into a pipe writes cursor motion and colour
    // codes into output someone is parsing, and then blocks on a key that will
    // never arrive. The caller falls back to printing a plain list.
    if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        return MenuOutcome::Cancelled;
    }
    let mut selected = start.min(rows.len() - 1);
    let mut drawn = 0usize;
    let mut out = std::io::stdout();

    let raw = enable_raw_mode().is_ok();
    let outcome = loop {
        let mut frame = String::new();
        if drawn > 0 {
            frame.push_str(&format!("\x1b[{drawn}A"));
        }
        frame.push_str("\r\x1b[J");

        let (d, r, b) = (dim_code(), reset_code(), brass_code());
        frame.push_str(&format!("  {d}{title}{r}\r\n"));
        for (i, row) in rows.iter().enumerate() {
            if i == selected {
                frame.push_str(&format!(
                    "  {b}›{r} {BOLD}{}{RESET}  {b}{}{r}\r\n     {d}{}{r}\r\n",
                    row.label, row.value, row.hint
                ));
            } else {
                frame.push_str(&format!("    {d}{}  {}{r}\r\n", row.label, row.value));
            }
        }
        frame.push_str(&format!("  {d}↑↓ move · enter change · esc close{r}"));
        let _ = out.write_all(frame.as_bytes());
        let _ = out.flush();
        // Two lines for the selected row (label + hint), one for every other,
        // plus the title and the footer.
        drawn = rows.len() + 3;

        match event::read() {
            Ok(Event::Key(k)) if k.kind != KeyEventKind::Release => match k.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    selected = if selected == 0 {
                        rows.len() - 1
                    } else {
                        selected - 1
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => selected = (selected + 1) % rows.len(),
                KeyCode::Enter | KeyCode::Char(' ') => break MenuOutcome::Chose(selected),
                KeyCode::Esc | KeyCode::Char('q') => break MenuOutcome::Cancelled,
                // ctrl-c has to leave too: a menu that swallows it looks hung.
                KeyCode::Char('c') if k.modifiers.contains(event::KeyModifiers::CONTROL) => {
                    break MenuOutcome::Cancelled;
                }
                _ => {}
            },
            Ok(_) => {}
            Err(_) => break MenuOutcome::Cancelled,
        }
    };
    if raw {
        let _ = disable_raw_mode();
    }
    // Erase the menu: it was a control surface, not a message. Leaving it in the
    // transcript would push the conversation off the screen every time someone
    // looked at their settings.
    let mut tail = String::new();
    if drawn > 0 {
        tail.push_str(&format!("\x1b[{drawn}A"));
    }
    tail.push_str("\r\x1b[J");
    let _ = out.write_all(tail.as_bytes());
    let _ = out.flush();
    let _ = color;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    /// THE LINES THAT ASK FOR CONSENT, AND THE CHIP LINE, MUST NOT BE
    /// REWRITABLE BY THE THING THEY DESCRIBE.
    ///
    /// `one_line` guards the approval prompt: a `\r` in the payload would let it
    /// erase the sentence saying what is about to leave the machine and print a
    /// harmless one in its place, and a `\n` would let it paint a second, fake
    /// `[y/N]` prompt. `printable` guards the diagnostic dump, where newlines
    /// are legitimate layout but everything else executes.
    #[test]
    fn consent_and_diagnostic_lines_carry_no_terminal_commands() {
        let payload = "send secret\u{1b}[2K\rsend weather\nDo you allow it? [y/N] y";

        let asked = one_line(payload);
        for bad in ['\u{1b}', '\r', '\n', '\u{9b}', '\u{7}'] {
            assert!(!asked.contains(bad), "{bad:?} survived: {asked:?}");
        }
        // WHAT IS BEING SENT IS STILL READABLE — hiding it would defeat the same
        // gate from the other side.
        assert!(asked.contains("secret"), "{asked:?}");

        let dumped = printable(payload);
        assert!(!dumped.contains('\u{1b}'), "{dumped:?}");
        assert!(!dumped.contains('\r'), "{dumped:?}");
        // A dump is meant to be read across lines, so those survive.
        assert!(dumped.contains('\n'), "{dumped:?}");
    }

    /// A screen that is NOT a terminal, built by hand.
    ///
    /// `Screen::setup` cannot be used for this: `cargo test` in a terminal
    /// inherits fd 1, so `is_terminal()` there answers TRUE and the test would
    /// measure the developer's shell instead of the code. Constructing the state
    /// directly is the only way to ask the question the test is asking.
    fn headless_screen() -> Arc<Screen> {
        Arc::new(Screen {
            tty: false,
            indicator_tty: false,
            inner: Mutex::new(ScreenInner {
                raw: false,
                has_indicator: false,
                last_chip: None,
            }),
        })
    }

    /// IT DOES NOT CRASH WITHOUT A TTY. The tests run in a piped environment; if
    /// `start` tried to turn on raw mode there, the whole shell would fall over.
    #[test]
    fn the_indicator_runs_silently_without_a_tty() {
        static CANCEL: AtomicBool = AtomicBool::new(false);
        let screen = headless_screen();
        let mut indicator = TurnIndicator::start(Arc::clone(&screen), &CANCEL, "thinking");
        indicator.stage(Stage::Prefill { tokens: 2586 });
        indicator.quiet();
        indicator.finish();
        // A second `finish` (it also arrives through Drop) must not panic.
        indicator.finish();
    }

    /// WITHOUT A TERMINAL NOTHING IS DRAWN — measured, not assumed.
    ///
    /// `has_indicator` is the honest witness: it is set ONLY on the line that
    /// actually writes bytes, so a false flag after a full round of stage changes
    /// means no byte left the process. This is the guarantee `--json` rests on
    /// from the other side (the bytes go to stderr, never stdout — see the module
    /// header); together they say a piped run stays parseable.
    #[test]
    fn no_terminal_means_not_one_byte_of_indicator() {
        static CANCEL: AtomicBool = AtomicBool::new(false);
        let screen = headless_screen();
        screen.indicator("◜ loading the model… 4s");
        assert!(
            !screen.inner.lock().expect("lock").has_indicator,
            "the indicator drew itself with no terminal"
        );

        let indicator = TurnIndicator::start(Arc::clone(&screen), &CANCEL, "thinking");
        for stage in [
            Stage::Loading {
                model: "qwen3-4b".into(),
            },
            Stage::Prefill { tokens: 2586 },
            Stage::Generating { tokens: 41 },
            Stage::Tool {
                name: "run_code".into(),
            },
        ] {
            indicator.stage(stage);
        }
        assert!(
            !screen.inner.lock().expect("lock").has_indicator,
            "a stage change drew itself with no terminal"
        );
        // Writing through the screen must not try to erase a line that was never
        // painted either.
        screen.clear_indicator();
        assert!(!screen.inner.lock().expect("lock").has_indicator);
    }

    /// EACH PHASE SAYS SOMETHING DIFFERENT, and says the number it was given.
    ///
    /// This is the whole point of the stage line: the six seconds spent reading
    /// weights and the six seconds spent inside a web search must not look
    /// identical on screen.
    #[test]
    fn every_stage_names_itself_and_carries_its_number() {
        let cases = [
            (
                Stage::Loading {
                    model: "qwen3-4b".into(),
                },
                "loading qwen3-4b",
            ),
            (
                Stage::Loading {
                    model: String::new(),
                },
                "loading the model",
            ),
            (Stage::Prefill { tokens: 2586 }, "prefill ~2586 tok"),
            (Stage::Generating { tokens: 0 }, "generating"),
            (Stage::Generating { tokens: 41 }, "generating 41 tok"),
            (
                Stage::Tool {
                    name: "run_code".into(),
                },
                "running run_code",
            ),
            (
                Stage::Verifying {
                    name: "write_code".into(),
                },
                "checking write_code",
            ),
            (Stage::Thinking, "thinking"),
        ];
        let mut seen: Vec<String> = Vec::new();
        for (stage, expected) in cases {
            let words = stage_words(&stage, "thinking");
            assert_eq!(words, expected, "{stage:?}");
            let line = indicator_line("◜", &stage, "thinking", 6, 80, false);
            assert!(line.contains(expected), "{line}");
            // The elapsed clock rides along on every phase: it is the number that
            // separates "slow" from "hung".
            assert!(line.contains("6s"), "{line}");
            seen.push(words);
        }
        // A zero-token generation and a started generation are the only pair
        // allowed to share a prefix; no two phases may read the same.
        seen.sort();
        let before = seen.len();
        seen.dedup();
        assert_eq!(before, seen.len(), "two stages produced the same words");
    }

    /// NO PHASE INVENTS A NUMBER. Only what the caller measured may appear: no
    /// percentage, no "time left". The same rule the download progress line
    /// follows, applied to the other long wait in this shell.
    #[test]
    fn the_stage_line_predicts_nothing() {
        for stage in [
            Stage::Loading {
                model: "qwen3-8b".into(),
            },
            Stage::Prefill { tokens: 2586 },
            Stage::Generating { tokens: 512 },
            Stage::Tool {
                name: "web_search".into(),
            },
        ] {
            let line = indicator_line("◜", &stage, "thinking", 12, 80, false);
            for forbidden in ["%", "left", "remaining", "eta", "min", "estimated"] {
                assert!(
                    !line.to_lowercase().contains(forbidden),
                    "{line:?} promises something it did not measure: {forbidden}"
                );
            }
        }
        // The prompt size the CLI holds IS an estimate, and the line admits it
        // rather than presenting it as a count.
        assert!(
            indicator_line(
                "◜",
                &Stage::Prefill { tokens: 2586 },
                "thinking",
                3,
                80,
                false
            )
            .contains("~2586"),
        );
    }

    /// ONE LINE, ALWAYS — a narrow terminal must not make the indicator wrap.
    ///
    /// A wrapped indicator is not a cosmetic fault: the next `\r` lands on the
    /// wrong line, the erase misses, and every frame leaves a corpse behind until
    /// the answer is buried in spinner debris.
    #[test]
    fn the_line_never_reaches_the_terminals_edge() {
        let stage = Stage::Tool {
            name: "web_search".into(),
        };
        for width in [8usize, 12, 20, 24, 30, 40, 60, 80, 120] {
            let line = indicator_line("◜", &stage, "thinking", 137, width, false);
            assert!(
                line.chars().count() < width,
                "width {width}: {} columns — {line:?}",
                line.chars().count()
            );
        }
        // Wide enough: the hint is there. Too narrow: the hint is the first
        // thing to go, and the elapsed clock is the last.
        let wide = indicator_line("◜", &stage, "thinking", 9, 80, false);
        assert!(wide.contains("ctrl-c"), "{wide}");
        let narrow = indicator_line("◜", &stage, "thinking", 9, 30, false);
        assert!(!narrow.contains("ctrl-c"), "{narrow}");
        assert!(narrow.contains("9s"), "{narrow}");
        assert!(narrow.contains("web_search"), "{narrow}");

        // Colour is a PARAMETER, exactly as in `chip_line`: the uncoloured form
        // is the one the tests can reason about, and the one a pipe would get.
        assert!(!indicator_line("◜", &stage, "thinking", 1, 80, false).contains('\x1b'));
    }

    /// A TOKEN COUNT MUST NOT STEAL THE LINE BACK FROM THE ANSWER.
    ///
    /// Once text is flowing, the last line is the answer's. Only a phase that is
    /// silent again — a tool that went to the network, the next round's prefill —
    /// may draw there.
    #[test]
    fn only_a_silent_phase_takes_the_line_back() {
        let generating = Stage::Generating { tokens: 240 };
        assert!(
            !stage_wakes(true, &generating),
            "the token count redrew itself over a streaming answer"
        );
        assert!(stage_wakes(false, &generating));
        for silent in [
            Stage::Tool {
                name: "web_search".into(),
            },
            Stage::Verifying {
                name: "write_code".into(),
            },
            Stage::Prefill { tokens: 900 },
            Stage::Loading {
                model: String::new(),
            },
        ] {
            assert!(
                stage_wakes(true, &silent),
                "{silent:?} stayed invisible after the answer streamed"
            );
        }
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
        assert!(
            !chip_line(&running, false).contains('\x1b'),
            "an uncoloured chip must carry no ANSI"
        );
        assert!(chip_line(&running, true).contains('\x1b'));
        // A tool whose text already ends in "…" must not get TWO ellipses.
        let already = ToolTrace {
            text: "searching…".into(),
            ..running.clone()
        };
        assert!(
            chip_line(&already, false).ends_with("searching…"),
            "{}",
            chip_line(&already, false)
        );
        // If the error reason is already in the chip text it must not be added
        // again.
        let repeated = ToolTrace {
            text: "File not found.".into(),
            state: ToolState::Failed("File not found.".into()),
            ..running.clone()
        };
        assert_eq!(chip_line(&repeated, false).matches("not found").count(), 1);
        let finished = ToolTrace {
            state: ToolState::Read,
            text: "27 results".into(),
            ..running
        };
        let s = chip_line(&finished, false);
        assert!(s.contains("27 results"), "{s}");
        assert!(!s.contains('…'), "{s}");
    }

    /// AUDIT M13: NO FIELD OF A CHIP CAN TALK TO THE TERMINAL.
    ///
    /// The payload is the audit's own: an OSC 52 clipboard write terminated by
    /// BEL, the form `edit_document` quoted back in its failure reason, plus
    /// the `\x1b[2J` an HTTP error body carried and a one-character C1 CSI.
    /// Each field gets it in turn — the reason is the one that leaked, the
    /// others are here so the next field added to a chip is not the next leak.
    #[test]
    fn a_chip_carries_no_escape_in_any_field() {
        let payload = "x\u{1b}]52;c;aGk=\u{7}\u{1b}[2J\u{9b}31m\rreal\nfake";
        let base = ToolTrace {
            id: TraceId(9),
            icon: "doc".into(),
            text: "edit".into(),
            state: ToolState::Failed(format!("no such source: {payload}")),
            raw_input: None,
            raw_output: None,
            file_path: None,
        };
        let cases = [
            base.clone(),
            ToolTrace {
                text: payload.into(),
                ..base.clone()
            },
            ToolTrace {
                icon: payload.into(),
                ..base.clone()
            },
            ToolTrace {
                text: payload.into(),
                state: ToolState::Running,
                ..base.clone()
            },
        ];
        for trace in cases {
            for colored in [false, true] {
                let line = chip_line(&trace, colored);
                // The palette's own escapes are allowed around the line; strip
                // exactly those and nothing else may remain.
                let bare = if colored {
                    line.strip_prefix(dim_code())
                        .and_then(|l| l.strip_suffix(reset_code()))
                        .unwrap_or_else(|| panic!("unexpected framing: {line:?}"))
                        .to_string()
                } else {
                    line.clone()
                };
                for bad in ['\u{1b}', '\u{7}', '\u{9b}', '\r', '\n'] {
                    assert!(!bare.contains(bad), "{bad:?} reached the chip: {line:?}");
                }
                assert!(
                    bare.contains("real"),
                    "the text must stay readable: {bare:?}"
                );
            }
        }
        // The repeat check still recognises a reason the text already carries
        // once both are cleaned.
        let repeated = ToolTrace {
            text: format!("no such source: {payload}"),
            ..base
        };
        assert_eq!(
            chip_line(&repeated, false)
                .matches("no such source")
                .count(),
            1
        );
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
    }

    fn typed(text: &str) -> Vec<Event> {
        text.chars()
            .map(|c| key(KeyCode::Char(c), KeyModifiers::NONE))
            .collect()
    }

    /// The answer line ends on `\r` and on `\n` alike (audit H1 — under raw
    /// mode the enter key is a `\r`, which the old `read_line` never ended
    /// on). Crossterm hands `\r` over as Enter and a raw-mode `\n` as ctrl-J.
    #[test]
    fn an_answer_ends_on_either_line_ending_and_can_be_declined() {
        let enter = key(KeyCode::Enter, KeyModifiers::NONE);
        let ctrl = |c| key(KeyCode::Char(c), KeyModifiers::CONTROL);

        let mut cr = typed("y");
        cr.push(enter.clone());
        cr.extend(typed("ignored"));
        assert_eq!(collect_answer(cr), Some("y".into()));

        let mut lf = typed("yes");
        lf.push(ctrl('j'));
        assert_eq!(collect_answer(lf), Some("yes".into()));

        let mut edited = typed("nx");
        edited.push(key(KeyCode::Backspace, KeyModifiers::NONE));
        edited.push(enter.clone());
        assert_eq!(collect_answer(edited), Some("n".into()));

        // Declining is always a `None`, and so is input that simply stops.
        let mut esc = typed("y");
        esc.push(key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(collect_answer(esc), None);
        let mut interrupted = typed("y");
        interrupted.push(ctrl('c'));
        assert_eq!(collect_answer(interrupted), None);
        assert_eq!(collect_answer(typed("y")), None, "no enter, no answer");

        assert!(is_yes(Some(" Y ")));
        assert!(!is_yes(Some("yeah")));
        assert!(!is_yes(None));
    }

    /// A keyboard that is a counter: raw mode is a flag, and every WAITING
    /// read (the spinner's — a prompt drain waits zero) is counted.
    ///
    /// TYPEAHEAD IS ONLY VISIBLE TO A ZERO WAIT. That is the honest model of
    /// the case the drain exists for — a key that arrived in the spinner's
    /// last instant, still queued when the question is about to be asked — and
    /// it keeps the spinner thread from racing the drain for it in the test.
    #[derive(Default)]
    struct CountingKeyboard {
        raw: AtomicBool,
        waits: std::sync::atomic::AtomicUsize,
        typeahead: Mutex<Vec<Event>>,
    }

    impl Keyboard for CountingKeyboard {
        fn set_raw(&self, on: bool) -> bool {
            self.raw.store(on, Ordering::SeqCst);
            true
        }
        fn next_event(&self, wait: Duration) -> Option<Event> {
            if wait.is_zero() {
                return self.typeahead.lock().expect("typeahead").pop();
            }
            self.waits.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(2));
            None
        }
    }

    impl CountingKeyboard {
        fn waits(&self) -> usize {
            self.waits.load(Ordering::SeqCst)
        }
        fn raw(&self) -> bool {
            self.raw.load(Ordering::SeqCst)
        }
        /// Did anything read the keyboard during the next `ms` milliseconds?
        fn read_during(&self, ms: u64) -> bool {
            let before = self.waits();
            std::thread::sleep(Duration::from_millis(ms));
            self.waits() != before
        }
    }

    /// A person at a terminal who answers `y` — and checks, AT THE MOMENT OF
    /// READING, that nothing else owns the keyboard.
    struct Watched<'a> {
        keyboard: &'a CountingKeyboard,
        seen: Mutex<Option<(bool, bool)>>,
    }

    impl Console for Watched<'_> {
        fn is_terminal(&self) -> bool {
            true
        }
        fn read_answer(&self) -> Option<String> {
            *self.seen.lock().expect("seen") =
                Some((self.keyboard.raw(), self.keyboard.read_during(60)));
            Some("y".into())
        }
    }

    /// AUDIT H1 / RG-03: WHILE A CONSENT QUESTION IS OPEN, THE TURN INDICATOR
    /// IS GONE — no thread reading keys, raw mode off — and it comes back when
    /// the question closes.
    ///
    /// The pty run in `tests/consent_reaches_the_person.rs` shows the user-visible
    /// result on macOS; this is the same contract stated on the state machine,
    /// with no terminal, so it runs on every platform CI has.
    #[test]
    fn a_consent_prompt_suspends_the_indicator_and_gives_it_back() {
        let _serial = ACTIVE_INDICATOR_TEST
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        static CANCEL: AtomicBool = AtomicBool::new(false);
        let keyboard = Arc::new(CountingKeyboard::default());
        let mut indicator = TurnIndicator::start_with(
            headless_screen(),
            &CANCEL,
            "thinking",
            Arc::clone(&keyboard) as Arc<dyn Keyboard>,
        );
        assert!(keyboard.raw(), "the turn starts with the input lock on");
        assert!(keyboard.read_during(60), "the spinner reads the keyboard");

        // Typeahead from the turn, including a ctrl-c the user really meant.
        keyboard.typeahead.lock().expect("typeahead").extend([
            key(KeyCode::Char('y'), KeyModifiers::NONE),
            key(KeyCode::Char('c'), KeyModifiers::CONTROL),
        ]);

        let person = Watched {
            keyboard: &keyboard,
            seen: Mutex::new(None),
        };
        {
            let prompt = Prompt::open(&person).expect("a terminal is somebody to ask");
            assert_eq!(prompt.read().as_deref(), Some("y"));
        }
        let (raw_while_asking, read_while_asking) =
            person.seen.lock().expect("seen").expect("the prompt read");
        assert!(
            !raw_while_asking,
            "raw mode was on while the question was asked"
        );
        assert!(
            !read_while_asking,
            "the spinner thread was still reading keys while the question waited"
        );
        assert!(
            keyboard.typeahead.lock().expect("typeahead").is_empty(),
            "typeahead was left queued for the question to read as its answer"
        );
        assert!(
            CANCEL.load(Ordering::SeqCst),
            "a ctrl-c in the typeahead must still cancel the turn"
        );

        // The question is closed: the turn has its input lock and spinner back.
        assert!(
            keyboard.raw(),
            "raw mode did not come back after the question"
        );
        assert!(keyboard.read_during(60), "the spinner did not come back");

        // And once the turn is over there is nothing left to suspend or revive.
        indicator.finish();
        assert!(!keyboard.raw(), "raw mode leaked past the turn");
        assert!(!keyboard.read_during(30), "the spinner outlived the turn");
        drop(pause_indicator());
        assert!(
            !keyboard.raw() && !keyboard.read_during(30),
            "a prompt after the turn revived a finished indicator"
        );
    }

    /// Two questions open at once (an MCP ask during an approval): the first
    /// one to close must not restart the spinner under the second.
    #[test]
    fn nested_prompts_keep_the_indicator_down_until_the_last_closes() {
        let _serial = ACTIVE_INDICATOR_TEST
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        static CANCEL: AtomicBool = AtomicBool::new(false);
        let keyboard = Arc::new(CountingKeyboard::default());
        let indicator = TurnIndicator::start_with(
            headless_screen(),
            &CANCEL,
            "thinking",
            Arc::clone(&keyboard) as Arc<dyn Keyboard>,
        );
        let outer = pause_indicator();
        let inner = pause_indicator();
        drop(inner);
        assert!(
            !keyboard.raw() && !keyboard.read_during(30),
            "the inner question's close revived the indicator"
        );
        drop(outer);
        assert!(keyboard.raw() && keyboard.read_during(60));
        drop(indicator);
        assert!(!keyboard.raw());
    }

    /// Ctrl-C raises the flag; other keys are SWALLOWED (they do not touch it).
    #[test]
    fn ctrl_c_marks_the_cancellation_and_other_keys_are_swallowed() {
        static CANCEL: AtomicBool = AtomicBool::new(false);
        let screen = Screen::setup();
        let letter = Event::Key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        swallow_key(&letter, &CANCEL, &screen);
        assert!(
            !CANCEL.load(Ordering::Relaxed),
            "an ordinary key must not cancel"
        );
        let ctrl_c = Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        swallow_key(&ctrl_c, &CANCEL, &screen);
        assert!(CANCEL.load(Ordering::Relaxed), "ctrl-c must cancel");
    }

    /// A REDIRECTED ANSWER CARRIES NO ESCAPES, and a terminal still gets them.
    ///
    /// The defect this pins was found by running the interactive shell for the
    /// first time (Ubuntu 24.04 and macOS, 4 Sep 2026): `tacet chat --engine fake
    /// -m hello | od -c` ended `f a k e   e n g i n e ) 033 [ 0 m`. `Color::paint`
    /// checks for a terminal, so the coloured spans were already clean — the leak
    /// was `RESET` and the theme colour going straight through `Screen::write`.
    ///
    /// BOTH DIRECTIONS ARE ASSERTED. Stripping everything would be an easy way to
    /// pass half this test and break the product for the person watching it work.
    #[test]
    fn escapes_leave_a_redirected_answer_and_stay_in_a_terminal() {
        let painted = format!("{DIM}Tacet {RESET}hello{RESET}\n");

        let stripped = Screen::strip_ansi(&painted);
        assert_eq!(stripped, "Tacet hello\n");
        assert!(!stripped.contains('\x1b'));

        // A two-character sequence (ESC + one byte) is dropped whole, and a CSI
        // with parameters is dropped up to its final byte — the two shapes the
        // theme actually emits.
        assert_eq!(Screen::strip_ansi("a\x1b[38;5;250mb"), "ab");
        assert_eq!(Screen::strip_ansi("a\x1b7b"), "ab");
        // Text that merely looks like a sequence is untouched.
        assert_eq!(Screen::strip_ansi("a[0mb"), "a[0mb");
    }
}
