//! The input field — frame, slash command list, token counter.
//!
//! WHY IT EXISTS: input was read with `std::io::stdin().read_line` and the
//! screen held a single `›`. That had three concrete consequences:
//!   1. Where the user typed WAS NOT DISTINGUISHABLE from where the model
//!      typed — a `›` following a long answer looked like a continuation of the
//!      answer.
//!   2. Slash commands could only be used FROM MEMORY; you had to type
//!      `/help` and come back.
//!   3. The tokens spent and the room left in the 4096 window WERE NOT VISIBLE
//!      — yet when the budget fills, old turns drop SILENTLY.
//!
//! All three require configuring the terminal's INPUT side: without raw mode
//! there is no key-by-key filtering, no caret, and no list you can overwrite.
//! `crossterm` was ALREADY in this crate (see Cargo.toml); no new dependency was
//! added, an existing capability was used.
//!
//! IF THERE IS NO TTY NONE OF THIS HAPPENS. On piped input (a script, CI,
//! `--message`) `read` falls straight through to `read_line` and writes NOT ONE
//! EXTRA BYTE to the screen — today's scripts see exactly the same output.
//!
//! BRAND: NO green dot / status dot / badge. The frame and the counter are DIM
//! (grey), the typed text is normal ink. There is no accent colour. The name is
//! "Tacet", capitalised; lowercase `tacet` is only THE BINARY's name (the
//! command typed in a shell), not the brand's.

use crate::ui::{BOLD, RESET, Screen, brass_code, dim_code, reset_code};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use std::io::Write;

/// The shell's slash commands — the list is HERE, the single source.
///
/// Descriptions are one line: while the list is open there is room for one line
/// per command on screen; a two-line description scrolls the list and makes the
/// selection impossible to follow.
pub struct Command {
    pub name: &'static str,
    pub description: &'static str,
}

pub const COMMANDS: &[Command] = &[
    Command {
        name: "/help",
        description: "the list of commands",
    },
    Command {
        name: "/tools",
        description: "the tools in the catalog and their descriptions",
    },
    Command {
        name: "/grammar",
        description: "the grammar generated from a tool's schema",
    },
    Command {
        name: "/eval",
        description: "run the logic eval set (fake engine, seconds)",
    },
    Command {
        name: "/memory",
        description: "the saved memory notes",
    },
    Command {
        name: "/history",
        description: "this session's conversation history",
    },
    Command {
        name: "/model",
        description: "the active engine, template and constraint",
    },
    Command {
        name: "/clear",
        description: "delete the conversation history and start a new transcript",
    },
    Command {
        name: "/sessions",
        description: "the conversations kept on disk, and where they live",
    },
    Command {
        name: "/preview",
        description: "show the last file Tacet saved (ctrl-o)",
    },
    Command {
        name: "/plugins",
        description: "the installed addons and their state",
    },
    Command {
        name: "/addon",
        description: "install, remove or toggle an addon (e.g. /addon install web-search)",
    },
    Command {
        name: "/config",
        description: "view or set personal defaults (model, engine, theme)",
    },
    Command {
        name: "/themes",
        description: "list colour themes or switch: /themes night",
    },
    Command {
        name: "/quit",
        description: "exit",
    },
];

/// The most rows shown in the list.
const LIST_CAP: usize = 8;

/// How many preview rows a highlighted command may add under the list.
const PREVIEW_CAP: usize = 6;

/// The LIVE CONTENT behind a highlighted command, before Enter is pressed.
///
/// WHY: `/plugins` in the list read only "the installed addons and their
/// state" — to learn WHICH addons and WHICH state you had to enter first
/// (measured: the user asked to see the contents without clicking). The rows
/// come from the same sources the real command reads (the addon registry on
/// disk, the theme table, the config file), so the preview can never disagree
/// with what Enter will show. Commands whose output is prose (/help, /tools…)
/// return nothing — previewing a paragraph in three clipped rows misleads.
fn preview(name: &str) -> Vec<String> {
    match name {
        "/plugins" | "/addons" => {
            let record = tacet_web::addon::read().ok();
            tacet_web::addon::DEFINITIONS
                .iter()
                .map(|d| {
                    let (inst, open) = record
                        .as_ref()
                        .map(|r| (r.find(d.name).is_some(), r.is_open(d.name)))
                        .unwrap_or((false, false));
                    let state = if !inst {
                        "not installed"
                    } else if open {
                        "installed · on"
                    } else {
                        "installed · off"
                    };
                    format!("{:<14} {state}", d.name)
                })
                .collect()
        }
        "/themes" => crate::ui::THEMES
            .iter()
            .map(|t| {
                let mark = if t.name == crate::ui::active_theme().name {
                    " · active"
                } else {
                    ""
                };
                format!("{:<14} {}{mark}", t.name, t.description)
            })
            .collect(),
        "/config" => crate::config::known_keys()
            .iter()
            .map(|(k, _)| {
                let v = crate::config::get_str(k).unwrap_or_else(|| "(unset)".into());
                format!("{k} = {v}")
            })
            .collect(),
        _ => Vec::new(),
    }
}
/// The frame's maximum width. On a wide terminal, drawing the screen edge to
/// edge makes the input field a "wall" rather than a "window".
const MAX_WIDTH: usize = 78;
const MIN_WIDTH: usize = 28;

pub enum Input {
    Line(String),
    /// EOF (ctrl-d) or a read error — the session ends.
    Done,
}

/// Case-folds so a typed prefix matches regardless of capitalisation.
///
/// HISTORICAL RECORD: this used to fold Turkish letters to their ASCII
/// counterparts (`ı`->`i`, `ç`->`c`, `ğ`->`g`, `ş`->`s`, `ö`->`o`, `ü`->`u`)
/// because the command names were Turkish (`/yardım`, `/çık`) and a user whose
/// keyboard had no Turkish letters could not reach the list at all. The command
/// names are English now, so the folding matched nothing and was dropped. IF A
/// LOCALIZED COMMAND NAME EVER COMES BACK, THE FOLDING HAS TO COME BACK WITH
/// IT.
fn simplify(s: &str) -> String {
    s.chars().flat_map(|c| c.to_lowercase()).collect()
}

/// The commands matching the typed prefix. A bare `/` lists everything.
pub fn matches(buffer: &str) -> Vec<&'static Command> {
    let query = simplify(buffer);
    COMMANDS
        .iter()
        .filter(|c| simplify(c.name).starts_with(&query))
        .collect()
}

/// Should the buffer open the slash command list: it MUST START with `/` and
/// NO SPACE MAY HAVE BEEN TYPED yet (what follows a space is an argument, not a
/// command).
fn list_needed(buffer: &str) -> bool {
    buffer.starts_with('/') && !buffer.contains(char::is_whitespace)
}

/// Is this line a slash command, or a message that happens to start with `/`?
///
/// THE FAILURE THAT FORCED IT, from a real session:
///
/// ```text
/// › /Users/farukciftler/Desktop bu klasöre bir md dosyası oluştur
/// (unknown command; /help)
/// ```
///
/// The shell asked only `starts_with('/')`, so an absolute path — the most
/// natural way there is to name a directory — was eaten by the command parser
/// and never reached the model. The user's way around it was to retype the line
/// with a `›` glued to the front, which is not a workaround anybody should have
/// to find.
///
/// THE RULE IS ABOUT THE FIRST TOKEN, NOT ABOUT SPACES. `list_needed` above can
/// demand "no whitespace" because it only decides whether to open a MENU while
/// typing; this decides whether to dispatch, and commands take arguments
/// (`/addon install http`), so a whitespace rule would break them.
///
/// WHAT SEPARATES THE TWO: a command NAME never carries a second `/`, and an
/// absolute path always does. `/plut` therefore stays a (mistyped) command and
/// still earns "unknown command" — the user was reaching for one — while
/// `/Users/...`, `/tmp/notes.md` and `/etc/hosts` are messages.
pub fn is_command(line: &str) -> bool {
    let Some(rest) = line.trim_start().strip_prefix('/') else {
        return false;
    };
    let first = rest.split_whitespace().next().unwrap_or("");
    !first.contains('/')
}

/// The guard that closes raw mode on every exit. Even on an early return via
/// `?` or a panic, the terminal is not left in raw mode for the user — in that
/// state the shell is broken in practice.
struct RawMode(bool);

impl RawMode {
    fn open() -> Self {
        let open = enable_raw_mode().is_ok();
        if open {
            // Let a paste arrive as a single event: otherwise the newlines
            // inside the pasted text count as ENTER and the first line is sent
            // while the rest is lost.
            let mut out = std::io::stdout();
            let _ = out.write_all(b"\x1b[?2004h");
            let _ = out.flush();
        }
        RawMode(open)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        if self.0 {
            let mut out = std::io::stdout();
            let _ = out.write_all(b"\x1b[?2004l");
            let _ = out.flush();
            let _ = disable_raw_mode();
        }
    }
}

/// Reads one line. `state` is the counter line sitting under the input field.
///
/// `history` is the messages sent in this session; up/down arrow walks them. It
/// is a list that stays inside the shell, it IS NOT WRITTEN to disk.
pub fn read(screen: &Screen, state: &str, history: &mut Vec<String>) -> Input {
    if !screen.tty() {
        // Piped input: the old behaviour, verbatim. No prompt is printed.
        let mut line = String::new();
        return match std::io::stdin().read_line(&mut line) {
            Ok(0) | Err(_) => Input::Done,
            Ok(_) => Input::Line(line.trim().to_string()),
        };
    }
    let _raw = RawMode::open();
    // The block bounds the BORROW of `history` by the editor's lifetime: the
    // same list is WRITTEN TO below.
    let result = {
        let mut e = Editor::new(state, history);
        e.run()
    };
    drop(_raw);
    match result {
        Input::Line(s) => {
            if !s.trim().is_empty() && history.last().map(String::as_str) != Some(s.as_str()) {
                history.push(s.clone());
            }
            Input::Line(s)
        }
        other => other,
    }
}

struct Editor<'a> {
    buffer: String,
    /// The caret's BYTE index — multi-byte letters exist, and holding a
    /// character index would mean rescanning on every edit.
    caret: usize,
    state: &'a str,
    history: &'a [String],
    history_i: Option<usize>,
    /// What was typed when the walk STARTED, kept for as long as it lasts.
    ///
    /// WHY IT IS A FIELD AND NOT A LOCAL: the prefix was read off the buffer
    /// inside the walk, but the walk REPLACES the buffer with the entry it
    /// lands on — so the filter existed on the first press of Up and was gone
    /// on the second, which then walked the whole history. Measured with
    /// history ["git status", "tacet why a", "git diff", "tacet why b"] and
    /// "tacet" typed: the first Up gave "tacet why b" and the second gave
    /// "git diff".
    history_prefix: Option<String>,
    selection: usize,
    /// Esc closed the list — it does not open again until the user types `/`.
    /// STICKY ON PURPOSE: a user who dismissed the list while writing a long
    /// message does not want it back on the next keystroke.
    dismissed: bool,
    /// A completion just filled the line, so the list is closed FOR NOW.
    ///
    /// SEPARATE FROM `dismissed`, and the separation is the whole fix: while
    /// completing shared the Esc flag, pressing Tab and then deleting a single
    /// letter left the list shut for good — the only way back was to erase the
    /// leading `/` entirely. Reported from real use. Editing after a completion
    /// is a NEW query and must list again; dismissing with Esc still is not.
    completed: bool,
    /// The index, within the drawn block, of the line the caret was left on in
    /// the last draw.
    caret_line: usize,
    first_draw: bool,
    note: Option<&'static str>,
}

impl<'a> Editor<'a> {
    fn new(state: &'a str, history: &'a [String]) -> Self {
        Self {
            buffer: String::new(),
            caret: 0,
            state,
            history,
            history_i: None,
            history_prefix: None,
            selection: 0,
            dismissed: false,
            completed: false,
            caret_line: 0,
            first_draw: true,
            note: None,
        }
    }

    fn list_open(&self) -> bool {
        !self.dismissed && !self.completed && list_needed(&self.buffer)
    }

    fn run(&mut self) -> Input {
        loop {
            self.draw();
            let event = match event::read() {
                Ok(e) => e,
                Err(_) => return Input::Done,
            };
            match event {
                Event::Paste(text) => {
                    let clean: String = text.replace("\r\n", "\n").replace('\r', "\n");
                    self.add(&clean);
                }
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    if let Some(result) = self.key(k) {
                        self.clear();
                        return result;
                    }
                }
                _ => {}
            }
        }
    }

    /// `Some(...)` = the read is finished.
    fn key(&mut self, k: KeyEvent) -> Option<Input> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let lower = k.modifiers.contains(KeyModifiers::ALT);
        self.note = None;
        match k.code {
            KeyCode::Enter => {
                // ALT/SHIFT+Enter: add a line. Most terminals do not
                // distinguish SHIFT; ALT arrives everywhere, both are accepted.
                if lower || k.modifiers.contains(KeyModifiers::SHIFT) {
                    self.add("\n");
                    return None;
                }
                // While the list is open ENTER SELECTS (it does not send): the
                // behaviour a completion list should have — the user does not
                // end up having sent the command before seeing it.
                if self.list_open()
                    && let Some(c) = matches(&self.buffer).get(self.selection)
                {
                    self.buffer = c.name.to_string();
                    self.caret = self.buffer.len();
                    self.completed = true;
                    self.selection = 0;
                    return None;
                }
                let line = self.buffer.trim().to_string();
                return Some(Input::Line(line));
            }
            KeyCode::Tab => {
                if self.list_open() {
                    let m = matches(&self.buffer);
                    if let Some(c) = m.get(self.selection) {
                        self.buffer = c.name.to_string();
                        self.caret = self.buffer.len();
                        self.completed = true;
                        self.selection = 0;
                    }
                } else if self.buffer.starts_with('/') {
                    // Tab with the list already closed still completes: the
                    // list being shut is not a statement about what the user
                    // wants the key to do.
                    let m = matches(&self.buffer);
                    if let Some(c) = m.first() {
                        self.buffer = c.name.to_string();
                        self.caret = self.buffer.len();
                        self.completed = true;
                        self.selection = 0;
                    }
                }
            }
            KeyCode::Esc => {
                if self.list_open() {
                    self.dismissed = true;
                }
            }
            KeyCode::Up => {
                if self.list_open() {
                    let n = matches(&self.buffer).len();
                    if n > 0 {
                        self.selection = (self.selection + n - 1) % n;
                    }
                } else {
                    self.walk_history(-1);
                }
            }
            KeyCode::Down => {
                if self.list_open() {
                    let n = matches(&self.buffer).len();
                    if n > 0 {
                        self.selection = (self.selection + 1) % n;
                    }
                } else {
                    self.walk_history(1);
                }
            }
            KeyCode::Left => self.caret = previous_boundary(&self.buffer, self.caret),
            KeyCode::Right => self.caret = next_boundary(&self.buffer, self.caret),
            KeyCode::Home => self.caret = 0,
            KeyCode::End => self.caret = self.buffer.len(),
            KeyCode::Backspace => {
                let new = previous_boundary(&self.buffer, self.caret);
                if new < self.caret {
                    self.buffer.replace_range(new..self.caret, "");
                    self.caret = new;
                    self.selection = 0;
                    // Editing after a completion is a new query.
                    self.completed = false;
                    if !self.buffer.starts_with('/') {
                        self.dismissed = false;
                    }
                }
            }
            KeyCode::Delete => {
                let new = next_boundary(&self.buffer, self.caret);
                if new > self.caret {
                    self.buffer.replace_range(self.caret..new, "");
                    self.selection = 0;
                    self.completed = false;
                }
            }
            // On an empty line ctrl-c NEITHER CRASHES NOR EXITS: exiting must
            // be an explicit intent (see the empty-line note in main.rs).
            KeyCode::Char('c') if ctrl => {
                if self.buffer.is_empty() {
                    self.note = Some("/quit or ctrl-d to exit");
                } else {
                    self.buffer.clear();
                    self.caret = 0;
                    self.dismissed = false;
                }
            }
            // ctrl-o: peek at the last saved file without typing anything. It
            // SUBMITS `/preview` — the slash path owns the actual printing, so
            // the key and the command can never drift apart.
            KeyCode::Char('o') if ctrl => {
                if self.buffer.is_empty() {
                    return Some(Input::Line("/preview".to_string()));
                }
            }
            // CTRL-D LEAVES, WHATEVER IS TYPED.
            //
            // readline's rule is that ctrl-d only means EOF on an empty line and
            // deletes forward otherwise, and this followed it. The measured cost
            // of that rule here: a user who wanted to leave had to clear the
            // line first, and pressing ctrl-d with text in the field did nothing
            // visible at the end of a line — a key that appears to be ignored
            // reads as a hung program.
            //
            // The draft is the only thing lost, and it is the only thing that
            // was never sent; the conversation itself is on disk (see
            // `session.rs`). Forward-delete keeps working through the `delete`
            // key, which is where people look for it anyway.
            KeyCode::Char('d') if ctrl => {
                return Some(Input::Done);
            }
            KeyCode::Char('a') if ctrl => self.caret = 0,
            KeyCode::Char('e') if ctrl => self.caret = self.buffer.len(),
            KeyCode::Char('u') if ctrl => {
                self.buffer.replace_range(..self.caret, "");
                self.caret = 0;
            }
            KeyCode::Char('k') if ctrl => {
                self.buffer.truncate(self.caret);
            }
            KeyCode::Char('w') if ctrl => {
                let new = word_start(&self.buffer, self.caret);
                self.buffer.replace_range(new..self.caret, "");
                self.caret = new;
            }
            KeyCode::Char(c) if !ctrl => {
                let s = c.to_string();
                self.add(&s);
            }
            _ => {}
        }
        None
    }

    fn without_controls(text: &str) -> String {
        text.chars()
            .filter_map(|c| match c {
                '\n' => Some('\n'),
                '\t' => Some(' '),
                c if c.is_control() => None,
                c => Some(c),
            })
            .collect()
    }

    /// Strips terminal control bytes from text that ENTERS THE BUFFER.
    ///
    /// WHY: pasted text is UNTRUSTED — the clipboard may come from a web page
    /// that told the reader to "paste this to your assistant". Bracketed paste
    /// hands the bytes over VERBATIM, and this buffer is BOTH drawn to the
    /// terminal (`draw`) AND sent to the model as the user's own message. A raw
    /// ESC therefore does two separate things at once: it EXECUTES as a
    /// terminal escape (cursor motion, erase, colour, even turning bracketed
    /// paste back off), and it makes WHAT IS ON SCREEN DIFFER FROM WHAT IS
    /// SENT. `chars().count()` also counts it as one printable column, so the
    /// caret arithmetic in `lines` drifts and the input frame comes apart.
    ///
    /// FILTERED HERE, AT THE SINGLE ENTRANCE, not in the paste branch:
    /// cleaning it up only while drawing would leave the raw bytes in the
    /// buffer, which is the half of the problem that actually reaches the
    /// model. `\n` SURVIVES — alt+enter uses it; `\t` becomes a space so the
    /// column arithmetic stays honest.
    fn add(&mut self, text: &str) {
        let text = &Self::without_controls(text);
        self.buffer.insert_str(self.caret, text);
        self.caret += text.len();
        self.selection = 0;
        self.completed = false;
        self.history_i = None;
        self.history_prefix = None;
        if text.starts_with('/') && self.caret == 1 {
            self.dismissed = false;
        }
    }

    /// Test seams for the three keystrokes whose INTERACTION is the behaviour
    /// under test. Driving them through `run()` would need a terminal.
    #[cfg(test)]
    fn complete_for_test(&mut self) {
        if let Some(c) = matches(&self.buffer).first() {
            self.buffer = c.name.to_string();
            self.caret = self.buffer.len();
            self.completed = true;
            self.selection = 0;
        }
    }

    #[cfg(test)]
    fn backspace_for_test(&mut self) {
        let new = previous_boundary(&self.buffer, self.caret);
        if new < self.caret {
            self.buffer.replace_range(new..self.caret, "");
            self.caret = new;
            self.selection = 0;
            self.completed = false;
            if !self.buffer.starts_with('/') {
                self.dismissed = false;
            }
        }
    }

    #[cfg(test)]
    fn dismiss_for_test(&mut self) {
        self.dismissed = true;
    }

    fn walk_history(&mut self, direction: i32) {
        if self.history.is_empty() {
            return;
        }
        // The prefix is captured ONCE, when the walk begins, and then held: by
        // the second press the buffer is no longer what the user typed, it is
        // the entry we just landed on.
        if self.history_i.is_none() {
            self.history_prefix = if self.buffer.is_empty() {
                None
            } else {
                Some(self.buffer.clone())
            };
        }
        let prefix = self.history_prefix.clone();
        let matches: Vec<(usize, &String)> = if let Some(ref p) = prefix {
            self.history
                .iter()
                .enumerate()
                .filter(|(_, h)| h.starts_with(p))
                .collect()
        } else {
            self.history.iter().enumerate().collect()
        };

        if matches.is_empty() {
            return;
        }

        let curr_match_idx = self
            .history_i
            .and_then(|hi| matches.iter().position(|(idx, _)| *idx == hi));
        let next_match_idx = match (curr_match_idx, direction) {
            (None, -1) => Some(matches.len() - 1),
            (None, _) => None,
            (Some(0), -1) => Some(0),
            (Some(i), -1) => Some(i - 1),
            (Some(i), _) if i >= matches.len() - 1 => None,
            (Some(i), _) => Some(i + 1),
        };

        match next_match_idx {
            Some(idx) => {
                let (real_idx, val) = matches[idx];
                self.history_i = Some(real_idx);
                self.buffer = val.clone();
            }
            None => {
                self.history_i = None;
                self.buffer = prefix.unwrap_or_default();
            }
        }
        self.caret = self.buffer.len();
    }

    // -----------------------------------------------------------------------
    // Drawing
    // -----------------------------------------------------------------------

    /// The lines to draw + the caret's (line, column) position.
    fn lines(&self) -> (Vec<String>, usize, usize) {
        let width = width();
        let inner = width.saturating_sub(4); // "│ " ... " │"
        let text_width = inner.saturating_sub(2); // the "› " prefix
        let mut lines = Vec::new();
        let rule = "─".repeat(width.saturating_sub(2));
        lines.push(dim(&format!("╭{rule}╮")));

        // Find which logical line the caret is on and which character of that
        // line it sits at.
        let ahead = &self.buffer[..self.caret];
        let caret_line_no = ahead.matches('\n').count();
        let caret_column = ahead.rsplit('\n').next().unwrap_or("").chars().count();

        let logical: Vec<&str> = self.buffer.split('\n').collect();
        let mut caret_at = (1usize, 0usize);
        for (i, line) in logical.iter().enumerate() {
            let marker = if i == 0 { "› " } else { "  " };
            // A long line IS SHIFTED RIGHT (not wrapped): wrapping makes the
            // number of visual lines drawn depend on the content and breaks the
            // overwrite arithmetic (how many lines to move up).
            let (visible, shift) = window(
                line,
                text_width,
                if i == caret_line_no { caret_column } else { 0 },
            );
            let fill = " ".repeat(text_width.saturating_sub(visible.chars().count()));
            // The prompt marker is BRASS — the same role the landing page's
            // demo gives its `$` and `tacet>` symbols: "you speak here".
            let (d, r, b) = (dim_code(), reset_code(), brass_code());
            lines.push(format!("{d}│{r} {b}{marker}{r}{visible}{fill} {d}│{r}"));
            if i == caret_line_no {
                caret_at = (lines.len() - 1, 2 + 2 + (caret_column - shift));
            }
        }
        lines.push(dim(&format!("╰{rule}╯")));

        if self.list_open() {
            let hits = matches(&self.buffer);
            if hits.is_empty() {
                lines.push(dim("  (no matching command — esc closes)"));
            }
            // THE WINDOW FOLLOWS THE SELECTION. With a fixed `take(LIST_CAP)`
            // the ninth row existed but was never drawn: arrowing past the
            // eighth entry moved the selection onto an invisible row and the
            // brass caret simply vanished (measured — "… 7 more" that could
            // not be reached). The window is stateless: once the selection
            // passes the cap it rides at the bottom edge, and the counts of
            // the rows hidden above and below are said out loud.
            let selection = self.selection.min(hits.len().saturating_sub(1));
            let first = selection.saturating_sub(LIST_CAP.saturating_sub(1));
            if first > 0 {
                lines.push(dim(&format!("    … {first} above")));
            }
            for (i, c) in hits.iter().enumerate().skip(first).take(LIST_CAP) {
                let selected = i == selection;
                let name = format!("{:<10}", c.name);
                // THE DESCRIPTION IS CLAMPED to the terminal width. A row that
                // wraps adds a visual line the overwrite arithmetic knows
                // nothing about; measured, walking the list then ate the line
                // ABOVE the frame on every keypress and Enter appeared to
                // scroll. The prefix is 15 visible columns ("  › " + padded
                // name + space); everything must fit in ONE terminal row.
                let room = width.saturating_sub(17);
                let description: String = if c.description.chars().count() > room {
                    let mut s: String =
                        c.description.chars().take(room.saturating_sub(1)).collect();
                    s.push('…');
                    s
                } else {
                    c.description.to_string()
                };
                let (d, r, b) = (dim_code(), reset_code(), brass_code());
                // The caret on the selected row is BRASS, the same accent the
                // prompt marker uses. The selected row was marked only by being
                // bold, which on a dim palette is a difference you have to look
                // for; the accent is the one colour this interface spends, and
                // "where am I in this list" is exactly what it is for.
                let line = if selected {
                    format!("  {b}›{r} {BOLD}{name}{RESET} {d}{description}{r}")
                } else {
                    format!("    {d}{name} {description}{r}")
                };
                lines.push(line);
            }
            let below = hits.len().saturating_sub(first + LIST_CAP);
            if below > 0 {
                lines.push(dim(&format!("    … {below} more")));
            }
            // THE PREVIEW: the highlighted command's live content, shown before
            // Enter. Every row is truncated to the terminal width — a wrapped
            // row breaks the overwrite arithmetic (see the description clamp
            // above) — and capped, with the hidden count said out loud.
            if let Some(c) = hits.get(selection) {
                let rows = preview(c.name);
                for row in rows.iter().take(PREVIEW_CAP) {
                    lines.push(dim(&truncate(&format!("      ┆ {row}"), width)));
                }
                if rows.len() > PREVIEW_CAP {
                    lines.push(dim(&format!(
                        "      ┆ … {} more inside",
                        rows.len() - PREVIEW_CAP
                    )));
                }
            }
            lines.push(dim("  ↑↓ select · tab/enter complete · esc close"));
        }

        let lower = match self.note {
            Some(n) => format!("  {n}"),
            None => format!("  {}", self.state),
        };
        lines.push(dim(&truncate(&lower, width)));
        (lines, caret_at.0, caret_at.1)
    }

    fn draw(&mut self) {
        let (lines, caret_line, caret_column) = self.lines();
        let mut out = String::new();
        // ERASE THE PREVIOUS DRAW. The caret was last left on line
        // `caret_line`; we move UP relative to that. The absolute caret position
        // IS NOT USED: the terminal may have scrolled in between and the
        // absolute position would point at the wrong place; relative motion
        // scrolls along with it.
        if !self.first_draw {
            if self.caret_line > 0 {
                out.push_str(&format!("\x1b[{}A", self.caret_line));
            }
            out.push_str("\r\x1b[J");
        }
        self.first_draw = false;
        out.push_str(&lines.join("\r\n"));
        // Bring the caret back to the input line.
        let last = lines.len() - 1;
        if last > caret_line {
            out.push_str(&format!("\x1b[{}A", last - caret_line));
        }
        out.push('\r');
        if caret_column > 0 {
            out.push_str(&format!("\x1b[{caret_column}C"));
        }
        self.caret_line = caret_line;
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(out.as_bytes());
        let _ = stdout.flush();
    }

    /// Erases the input field from the screen — the sent message is reprinted by
    /// `main`, the frame must not be left behind.
    fn clear(&mut self) {
        let mut out = String::new();
        if self.caret_line > 0 {
            out.push_str(&format!("\x1b[{}A", self.caret_line));
        }
        out.push_str("\r\x1b[J");
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(out.as_bytes());
        let _ = stdout.flush();
    }
}

// ---------------------------------------------------------------------------
// The menu — the arrow-key picker every submenu shares
// ---------------------------------------------------------------------------

/// An interactive list: ↑↓ moves, Enter returns `Some(index)`, Esc (and
/// ctrl-c / ctrl-d) returns `None`. The drawing dialect is the slash list's —
/// brass caret on the selected row, a window that FOLLOWS the selection, every
/// row clamped to one terminal line — so the two cannot drift apart visually.
///
/// WITHOUT A TTY IT RETURNS `None` IMMEDIATELY; the caller must treat that as
/// "fall back to the printed form", never as "the user said no" — piped
/// sessions still get the old text output.
///
/// THE MENU ERASES ITSELF on the way out: what stays in the transcript is the
/// OUTCOME (the command the choice produced), not the furniture. A transcript
/// full of dead menus reads like a screenshot, not a conversation.
pub fn menu(screen: &Screen, title: &str, items: &[(String, String)]) -> Option<usize> {
    if !screen.tty() || items.is_empty() {
        return None;
    }
    let _raw = RawMode::open();
    let mut selection = 0usize;
    let mut drawn = 0usize;
    let wide = width();

    let draw = |selection: usize, drawn: usize| -> usize {
        let mut out = String::new();
        if drawn > 0 {
            out.push_str(&format!("\x1b[{drawn}A\r\x1b[J"));
        }
        let mut lines: Vec<String> = Vec::new();
        lines.push(dim(&format!("  {title}")));
        let first = selection.saturating_sub(LIST_CAP.saturating_sub(1));
        if first > 0 {
            lines.push(dim(&format!("    … {first} above")));
        }
        let label_width = items
            .iter()
            .map(|(l, _)| l.chars().count())
            .max()
            .unwrap_or(0)
            .min(24);
        for (i, (label, hint)) in items.iter().enumerate().skip(first).take(LIST_CAP) {
            let room = wide.saturating_sub(label_width + 9);
            let hint: String = if hint.chars().count() > room {
                let mut s: String = hint.chars().take(room.saturating_sub(1)).collect();
                s.push('…');
                s
            } else {
                hint.clone()
            };
            let padded = format!("{label:<label_width$}");
            let (d, r, b) = (dim_code(), reset_code(), brass_code());
            lines.push(if i == selection {
                format!("  {b}›{r} {BOLD}{padded}{RESET} {d}{hint}{r}")
            } else {
                format!("    {d}{padded} {hint}{r}")
            });
        }
        let below = items.len().saturating_sub(first + LIST_CAP);
        if below > 0 {
            lines.push(dim(&format!("    … {below} more")));
        }
        lines.push(dim("  ↑↓ move · enter select · esc back"));
        out.push_str(&lines.join("\r\n"));
        out.push_str("\r\n");
        let mut so = std::io::stdout();
        let _ = so.write_all(out.as_bytes());
        let _ = so.flush();
        lines.len()
    };

    drawn = draw(selection, drawn);
    let result = loop {
        let Ok(event) = event::read() else { break None };
        let Event::Key(k) = event else { continue };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Up => selection = (selection + items.len() - 1) % items.len(),
            KeyCode::Down => selection = (selection + 1) % items.len(),
            KeyCode::Enter => break Some(selection),
            KeyCode::Esc => break None,
            KeyCode::Char('c') if ctrl => break None,
            KeyCode::Char('d') if ctrl => break None,
            _ => continue,
        }
        drawn = draw(selection, drawn);
    };
    // Erase the furniture; the caller narrates the outcome.
    let mut so = std::io::stdout();
    let _ = so.write_all(format!("\x1b[{drawn}A\r\x1b[J").as_bytes());
    let _ = so.flush();
    result
}

fn dim(text: &str) -> String {
    format!("{}{text}{}", dim_code(), reset_code())
}

/// The terminal width — a sensible default if it cannot be measured.
fn width() -> usize {
    let w = crossterm::terminal::size()
        .map(|(w, _)| w as usize)
        .unwrap_or(80);
    w.clamp(MIN_WIDTH, MAX_WIDTH)
}

/// Fits the text into a window `wide` characters across; SHIFTS RIGHT if needed
/// so the caret stays visible. The second value returned is the shift amount (in
/// characters).
fn window(line: &str, wide: usize, caret: usize) -> (String, usize) {
    let n = line.chars().count();
    if n <= wide {
        return (line.to_string(), 0);
    }
    let shift = caret.saturating_sub(wide.saturating_sub(1));
    let visible: String = line.chars().skip(shift).take(wide).collect();
    (visible, shift)
}

fn truncate(text: &str, wide: usize) -> String {
    if text.chars().count() <= wide {
        return text.to_string();
    }
    text.chars()
        .take(wide.saturating_sub(1))
        .collect::<String>()
        + "…"
}

/// The previous CHARACTER boundary — so we never land in the middle of a
/// multi-byte letter.
fn previous_boundary(s: &str, i: usize) -> usize {
    let mut j = i;
    loop {
        if j == 0 {
            return 0;
        }
        j -= 1;
        if s.is_char_boundary(j) {
            return j;
        }
    }
}

fn next_boundary(s: &str, i: usize) -> usize {
    let mut j = i;
    loop {
        if j >= s.len() {
            return s.len();
        }
        j += 1;
        if s.is_char_boundary(j) {
            return j;
        }
    }
}

/// The start of the word to the left of the caret (ctrl-w).
fn word_start(s: &str, i: usize) -> usize {
    let mut j = i;
    while j > 0 {
        let k = previous_boundary(s, j);
        if !s[k..j].chars().all(char::is_whitespace) {
            break;
        }
        j = k;
    }
    while j > 0 {
        let k = previous_boundary(s, j);
        if s[k..j].chars().all(char::is_whitespace) {
            break;
        }
        j = k;
    }
    j
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PASTED TEXT IS UNTRUSTED AND MUST NOT REACH EITHER THE TERMINAL OR THE
    /// MODEL WITH ITS ESCAPES INTACT.
    ///
    /// The attack: a page says "paste this to your assistant" and the clipboard
    /// carries `ESC[2K` and a carriage return. Bracketed paste hands the bytes
    /// over verbatim, the buffer is drawn straight to stdout (the escapes
    /// EXECUTE — the frame is erased and repainted) and the same buffer is sent
    /// to the model, so what the user reads on screen and what they send stop
    /// being the same text.
    ///
    /// It goes through `Editor::add`, the single entrance, because that is
    /// where the fix lives: a filter applied only while drawing would leave the
    /// raw bytes in what gets sent.
    /// REPORTED FROM REAL USE: Tab completes, one letter is deleted, and the
    /// list never comes back.
    ///
    /// The cause was one flag doing two jobs. Completing shared the flag Esc
    /// sets, and that flag is sticky by design — so after a completion the only
    /// way to see the list again was to erase the leading `/` entirely.
    /// Dismissing with Esc still survives editing; completing does not.
    #[test]
    fn the_list_comes_back_after_editing_a_completion() {
        let history: Vec<String> = Vec::new();
        let mut e = Editor::new("", &history);
        e.add("/hel");
        assert!(e.list_open(), "typing a slash command lists");

        e.complete_for_test();
        assert_eq!(e.buffer, "/help");
        assert!(
            !e.list_open(),
            "a completed line does not keep the list open"
        );

        e.backspace_for_test();
        assert_eq!(e.buffer, "/hel");
        assert!(
            e.list_open(),
            "deleting a letter after a completion is a new query and must list again"
        );
    }

    /// And the rule the fix must not break: Esc means Esc.
    #[test]
    fn esc_keeps_the_list_shut_while_editing() {
        let history: Vec<String> = Vec::new();
        let mut e = Editor::new("", &history);
        e.add("/hel");
        e.dismiss_for_test();
        assert!(!e.list_open());
        e.backspace_for_test();
        assert!(
            !e.list_open(),
            "a list dismissed with Esc stays shut until a new slash is typed"
        );
    }

    /// THE PREFIX MUST SURVIVE THE SECOND PRESS.
    ///
    /// Prefix-filtered history is only useful if the filter lasts the whole
    /// walk. As first written the prefix was read off the buffer inside the
    /// walk — but the walk replaces the buffer with the entry it lands on, so
    /// the filter applied to the first Up and vanished on the second, which
    /// then stepped through unrelated commands.
    #[test]
    fn walking_history_keeps_filtering_by_what_was_typed() {
        let history: Vec<String> = ["git status", "tacet why a", "git diff", "tacet why b"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut e = Editor::new("", &history);
        e.add("tacet");

        e.walk_history(-1);
        assert_eq!(e.buffer, "tacet why b", "the newest match comes first");
        e.walk_history(-1);
        assert_eq!(
            e.buffer, "tacet why a",
            "the second press must skip 'git diff' — it does not match the prefix"
        );
        // And back down again, still filtered.
        e.walk_history(1);
        assert_eq!(e.buffer, "tacet why b");
    }

    /// With nothing typed, the walk is the whole history — the ordinary
    /// behaviour, and the one the filter must not take away.
    #[test]
    fn an_empty_line_walks_everything() {
        let history: Vec<String> = ["one", "two"].iter().map(|s| s.to_string()).collect();
        let mut e = Editor::new("", &history);
        e.walk_history(-1);
        assert_eq!(e.buffer, "two");
        e.walk_history(-1);
        assert_eq!(e.buffer, "one");
    }

    /// Editing the line ends the walk: the next Up starts a NEW filter from
    /// what is now typed, rather than carrying the old one.
    #[test]
    fn editing_after_a_walk_starts_a_new_filter() {
        let history: Vec<String> = ["git status", "tacet why a"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut e = Editor::new("", &history);
        e.add("tacet");
        e.walk_history(-1);
        assert_eq!(e.buffer, "tacet why a");

        let mut e = Editor::new("", &history);
        e.add("git");
        e.walk_history(-1);
        assert_eq!(e.buffer, "git status");
    }

    #[test]
    fn pasted_control_bytes_never_enter_the_buffer() {
        let history: Vec<String> = Vec::new();

        let mut e = Editor::new("", &history);
        e.add("hi \u{1b}[2K\rthere\u{9b}2J\u{7}");
        for bad in ['\u{1b}', '\r', '\u{9b}', '\u{7}'] {
            assert!(
                !e.buffer.contains(bad),
                "{bad:?} entered the buffer: {:?}",
                e.buffer
            );
        }
        assert_eq!(e.buffer, "hi [2Kthere2J");
        // The caret must count what is really there, or the frame arithmetic
        // drifts by exactly the number of invisible bytes.
        assert_eq!(e.caret, e.buffer.len());

        // ALT+ENTER STILL WORKS: a newline is layout, not a command.
        let mut n = Editor::new("", &history);
        n.add("a\nb");
        assert_eq!(n.buffer, "a\nb");

        // A tab becomes a space so the column count stays honest.
        let mut t = Editor::new("", &history);
        t.add("a\tb");
        assert_eq!(t.buffer, "a b");

        // WHAT IS DRAWN CARRIES NO ESCAPE OF THE PASTE'S OWN. Colour codes from
        // our own styling are expected; the payload's are not.
        let mut d = Editor::new("", &history);
        d.add("x\u{1b}[2Jy");
        let drawn = d.lines().0.join("");
        // The frame carries OUR OWN colour codes, so what is measured is that
        // the PAYLOAD's escape is gone: the bare text `[2J` left behind is
        // inert, an ESC in front of it would not be.
        assert!(
            !drawn.contains("\u{1b}[2J"),
            "the payload was drawn as an escape: {drawn:?}"
        );
        assert!(
            drawn.contains("x[2Jy"),
            "the text itself was lost: {drawn:?}"
        );
    }

    /// The list opens ONLY while a command is being typed: a user typing
    /// `/grammar web_search` is entering an argument, not picking a command.
    #[test]
    fn when_the_list_opens() {
        assert!(list_needed("/"));
        assert!(list_needed("/he"));
        assert!(!list_needed("/grammar "));
        assert!(!list_needed("hello"));
        assert!(!list_needed(""));
    }

    /// CTRL-D LEAVES EVEN WITH A DRAFT IN THE FIELD.
    ///
    /// The readline rule (EOF only on an empty line) was measured to be wrong
    /// here: with text typed, ctrl-d at the end of a line did nothing visible,
    /// and a key that appears to be ignored reads as a hung program.
    #[test]
    fn ctrl_d_exits_whether_or_not_something_is_typed() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let history = Vec::new();

        let mut empty = Editor::new("", &history);
        assert!(matches!(
            empty.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)),
            Some(Input::Done)
        ));

        let mut typed = Editor::new("", &history);
        typed.add("half a sentence");
        assert!(
            matches!(
                typed.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)),
                Some(Input::Done)
            ),
            "ctrl-d must leave with a draft in the field, not delete a character"
        );
    }

    /// Forward delete did not disappear with the rule above — it lives on the
    /// key people reach for. Losing it silently would be trading one surprise
    /// for another.
    #[test]
    fn the_delete_key_still_deletes_forward() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let history = Vec::new();
        let mut e = Editor::new("", &history);
        e.add("abc");
        e.caret = 0;
        e.key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(e.buffer, "bc");
    }

    /// Matching DOES NOT CARE about capitalisation.
    #[test]
    fn matching_ignores_capitalisation() {
        let h: Vec<&str> = matches("/HeL").iter().map(|c| c.name).collect();
        assert_eq!(h, vec!["/help"]);
        let q: Vec<&str> = matches("/Q").iter().map(|c| c.name).collect();
        assert_eq!(q, vec!["/quit"]);
        let g: Vec<&str> = matches("/gr").iter().map(|c| c.name).collect();
        assert_eq!(g, vec!["/grammar"]);
        assert_eq!(matches("/").len(), COMMANDS.len());
        assert!(matches("/zzz").is_empty());
    }

    /// A MESSAGE STARTING WITH A PATH IS A MESSAGE.
    ///
    /// The regression this pins: `/Users/…/Desktop bu klasöre bir md dosyası
    /// oluştur` was dispatched as a command and answered "(unknown command)".
    #[test]
    fn an_absolute_path_is_not_a_slash_command() {
        for line in [
            "/Users/farukciftler/Desktop bu klasöre bir md dosyası oluştur",
            "/tmp/notes.md dosyasını özetle",
            "/etc/hosts",
            "  /var/log/system.log nedir",
        ] {
            assert!(!is_command(line), "should be a message: {line}");
        }
    }

    /// …and a command is still a command, arguments and typos included.
    #[test]
    fn a_command_is_still_a_command_with_arguments() {
        for line in ["/help", "/addon install http", "/plugins", "/plut", " /clear"] {
            assert!(is_command(line), "should be a command: {line}");
        }
        // No leading slash at all: plainly a message.
        assert!(!is_command("merhaba"));
        assert!(!is_command(""));
    }

    /// Every command must have a description — an explicit condition of the
    /// request.
    #[test]
    fn every_command_has_a_description() {
        for c in COMMANDS {
            assert!(!c.description.is_empty(), "{}", c.name);
            assert!(c.name.starts_with('/'));
        }
    }

    /// Caret motion over multi-byte letters DOES NOT PANIC.
    ///
    /// The data stays non-ASCII deliberately: with ASCII input every byte is a
    /// character boundary and this test would pass even with the boundary walk
    /// removed.
    #[test]
    fn the_caret_walks_on_character_boundaries() {
        let s = "çığ";
        assert_eq!(previous_boundary(s, s.len()), 4);
        assert_eq!(next_boundary(s, 0), 2);
        assert_eq!(previous_boundary(s, 0), 0);
        assert_eq!(next_boundary(s, s.len()), s.len());
    }

    #[test]
    fn word_deletion() {
        let s = "hello world";
        assert_eq!(word_start(s, s.len()), 6);
        assert_eq!(&s[..word_start(s, s.len())], "hello ");
        assert_eq!(word_start("one", 3), 0);
    }

    /// A long line SHIFTS RIGHT and the caret stays inside the window.
    #[test]
    fn a_long_line_shifts() {
        let s: String = "abcdefghij".repeat(5);
        // The caret is at the END of the text: the window shows the last 9
        // characters, the 10th column is left to the caret itself (otherwise the
        // caret fell outside the frame).
        let (v, shift) = window(&s, 10, 50);
        assert!(v.chars().count() <= 10, "{}", v.chars().count());
        assert!(50 - shift < 10, "the caret must stay inside the window");
        // It is visible with the caret in the middle too.
        let (v3, shift3) = window(&s, 10, 25);
        assert!(v3.chars().count() <= 10);
        assert!(25 - shift3 < 10);
        let (v2, shift2) = window("short", 10, 2);
        assert_eq!(v2, "short");
        assert_eq!(shift2, 0);
    }
}
