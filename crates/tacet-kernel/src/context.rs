//! ToolContext — the running state tools share.
//!
//! Tools are handed a single `&mut ToolContext`; there is no global state. That
//! way eval runs the same tool with a fake store and a silent reporter while
//! the app runs it with the real ones, and the tool cannot tell them apart.

use crate::data_store::{DataStore, SourceRef};
use crate::error::{ToolError, ToolResult};
use crate::reporter::{Reporter, TraceId, TraceUpdate};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// "Stop": the user pressed Ctrl-C, or the executor's deadline for this call
/// passed. A tool that runs something long (a process, a poll loop) reads it
/// and winds down; one that does not is abandoned by the executor instead.
///
/// WHY NOT A PLAIN `Arc<AtomicBool>`. Three parties can say "stop" and they
/// have different lifetimes: the shell's Ctrl-C flag (a `static`, reset every
/// turn), the executor's turn (a new one per turn, so cancelling an old turn
/// never binds the next), and the executor's per-call deadline (which must
/// stop THIS call and nothing after it). Firing one shared flag for the
/// deadline would cancel every later call of the session; so a token is its
/// OWN flag plus the flags it WATCHES, and `child` makes a token that a
/// deadline can fire without touching the parent.
#[derive(Clone, Debug, Default)]
pub struct CancelToken {
    own: Arc<AtomicBool>,
    watched: Vec<Watched>,
}

#[derive(Clone, Debug)]
enum Watched {
    Shared(Arc<AtomicBool>),
    Static(&'static AtomicBool),
}

impl Watched {
    fn is_set(&self) -> bool {
        match self {
            Watched::Shared(flag) => flag.load(Ordering::SeqCst),
            Watched::Static(flag) => flag.load(Ordering::SeqCst),
        }
    }
}

impl CancelToken {
    /// A token nobody has fired.
    pub fn new() -> Self {
        Self::default()
    }

    /// A token that also reads as cancelled whenever `flag` is set — the shell's
    /// Ctrl-C flag is a `static` the key watcher writes from another thread.
    pub fn watching(flag: &'static AtomicBool) -> Self {
        Self {
            own: Arc::default(),
            watched: vec![Watched::Static(flag)],
        }
    }

    /// A token cancelled by anything that cancels `self`, PLUS its own flag —
    /// firing the child does not fire the parent.
    pub fn child(&self) -> Self {
        Self::new().also_watching(self)
    }

    /// Adds everything that cancels `other` to what cancels `self`.
    pub fn also_watching(mut self, other: &CancelToken) -> Self {
        self.watched.push(Watched::Shared(Arc::clone(&other.own)));
        self.watched.extend(other.watched.iter().cloned());
        self
    }

    /// Fires this token (and every child of it). Never un-fires: a stop that
    /// could be taken back is a race with the tool reading it.
    pub fn cancel(&self) {
        self.own.store(true, Ordering::SeqCst);
    }

    /// Has anything this token listens to said "stop".
    pub fn is_cancelled(&self) -> bool {
        self.own.load(Ordering::SeqCst) || self.watched.iter().any(Watched::is_set)
    }
}

/// What a tool is handed when it runs: the store to put bulk data in, the
/// directory it may not leave, the reporter it draws its chip on, and the
/// session's taint.
pub struct ToolContext {
    /// The bypass channel. Bulk data goes here, a short summary goes to the model.
    pub data_store: Arc<dyn DataStore>,
    /// The sandbox for file operations. Cannot be escaped.
    pub working_dir: PathBuf,
    /// Where a tool announces what it is doing. The shell draws chips from it;
    /// a test collects them instead.
    pub reporter: Arc<dyn Reporter>,
    /// Has a personal-data tool run SUCCESSFULLY at least once this session.
    ///
    /// One-way (see `taint`) and NOT RESET between turns: when the context is
    /// summarized the summary itself may carry personal data, so the taint
    /// carries over with it. Only a real chat reset clears it.
    session_tainted: bool,
    /// Has a tool brought text written OUTSIDE this machine into the context —
    /// a web page, a remote server's answer, another agent's transcript (audit
    /// H2-c). One-way and session-long for the same reason as the taint: the
    /// summary of a context carries what the page told the model to do.
    ///
    /// SEPARATE FROM THE TAINT ON PURPOSE. The taint asks "may data LEAVE";
    /// this asks "may the model's next ACTION have been written by someone
    /// else". A web search reads nothing personal, so it must not tighten the
    /// outbound gate — but a page can say "run `git -c alias.x='!cmd' x`", and
    /// the tool that runs programs has to ask after that.
    external_content_seen: bool,
    /// "Stop" for whatever runs under this context — see `CancelToken`.
    cancel: CancelToken,
}

impl ToolContext {
    /// A fresh, untainted context.
    pub fn new(
        data_store: Arc<dyn DataStore>,
        working_dir: impl Into<PathBuf>,
        reporter: Arc<dyn Reporter>,
    ) -> Self {
        Self {
            data_store,
            working_dir: working_dir.into(),
            reporter,
            session_tainted: false,
            external_content_seen: false,
            cancel: CancelToken::new(),
        }
    }

    /// The same context, listening to `cancel` — the shell hands its Ctrl-C
    /// flag in here (`CancelToken::watching`).
    pub fn with_cancel_token(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }

    /// The "stop" this call listens to. Long-running tools poll it.
    pub fn cancel_token(&self) -> &CancelToken {
        &self.cancel
    }

    /// Shortcut for `cancel_token().is_cancelled()`.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// A copy of this context for ONE call run on another thread, listening to
    /// `cancel` instead of this context's token.
    ///
    /// WHY A COPY AND NOT A BORROW: the executor runs a tool on a worker thread
    /// so it can stop waiting for it (a deadline, Ctrl-C, a panic). A worker
    /// that is abandoned still holds whatever it was given; if that were this
    /// `&mut`, the session's context would be pinned by a thread nobody is
    /// waiting for. The store and the reporter are shared `Arc`s, so what the
    /// tool stores and the chips it draws land in the same place either way.
    /// What the tool flips on the copy comes back through `absorb`.
    pub fn fork(&self, cancel: CancelToken) -> ToolContext {
        ToolContext {
            data_store: Arc::clone(&self.data_store),
            working_dir: self.working_dir.clone(),
            reporter: Arc::clone(&self.reporter),
            session_tainted: self.session_tainted,
            external_content_seen: self.external_content_seen,
            cancel,
        }
    }

    /// Takes back what a finished worker learned. Both flags only ever SET:
    /// a copy cannot clear the session's taint by coming back clean.
    pub fn absorb(&mut self, worker: &ToolContext) {
        self.session_tainted |= worker.session_tainted;
        self.external_content_seen |= worker.external_content_seen;
    }

    /// Has outside-written text entered the context this session (see the
    /// field). The approval gate in front of the tools that ACT reads this.
    pub fn saw_external_content(&self) -> bool {
        self.external_content_seen
    }

    /// Called after a SUCCESSFUL call of a tool whose result was written
    /// outside this machine. There is no way back but a chat reset.
    pub fn mark_external_content(&mut self) {
        self.external_content_seen = true;
    }

    /// Has a personal-data tool run successfully this session. The approval
    /// gate in front of every outbound tool reads this.
    pub fn session_tainted(&self) -> bool {
        self.session_tainted
    }

    /// Called after a SUCCESSFUL call of a personal-data tool. There is no way back.
    pub fn taint(&mut self) {
        self.session_tainted = true;
    }

    /// A real chat reset — anything with session lifetime ends here.
    pub fn reset_chat(&mut self) {
        self.session_tainted = false;
        self.external_content_seen = false;
        self.data_store.clear();
    }

    // --- Bypass channel shortcuts ---

    /// Puts bulk data into the store; a reference goes to the model instead.
    pub fn store(&self, kind: &str, summary: &str, body: String) -> SourceRef {
        self.data_store.put(kind, summary, body)
    }

    /// Retrieves data stored in an earlier step.
    pub fn from_store(&self, source_ref: &SourceRef) -> Option<crate::data_store::Record> {
        self.data_store.take(source_ref)
    }

    // --- Chip shortcuts ---

    /// Opens a chip for work that is starting. Finish it with `update_chip`.
    pub fn start_chip(&self, icon: &str, text: &str) -> TraceId {
        self.reporter.start(icon, text)
    }

    /// Fills in what an open chip turned out to be — state, text, the file it
    /// wrote, the raw exchange behind it.
    pub fn update_chip(&self, id: TraceId, update: TraceUpdate) {
        self.reporter.update(id, update);
    }

    // --- Sandbox ---

    /// Resolves the given path against the working directory and verifies that
    /// it does not escape.
    ///
    /// The check runs component by component instead of waiting for the file to
    /// exist: `canonicalize` fails on a file that does not exist, yet on the
    /// WRITE path the target does not exist yet — and that is exactly where the
    /// gate is needed most.
    pub fn resolve_path(&self, path: impl AsRef<Path>) -> ToolResult<PathBuf> {
        use std::path::Component;
        let path = path.as_ref();
        let mut resolved = self.working_dir.clone();
        let relative = path.strip_prefix(&self.working_dir).unwrap_or(path);
        for component in relative.components() {
            match component {
                Component::Normal(p) => resolved.push(p),
                Component::CurDir => {}
                Component::ParentDir => {
                    if !resolved.pop() || !resolved.starts_with(&self.working_dir) {
                        return Err(ToolError::SandboxViolation(path.to_path_buf()));
                    }
                }
                // An absolute path root or a Windows prefix: an attempt to
                // disable the sandbox from the outset.
                Component::RootDir | Component::Prefix(_) => {
                    return Err(ToolError::SandboxViolation(path.to_path_buf()));
                }
            }
        }
        if resolved.starts_with(&self.working_dir) {
            Ok(resolved)
        } else {
            Err(ToolError::SandboxViolation(path.to_path_buf()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_store::InMemoryDataStore;
    use crate::reporter::SilentReporter;

    fn context() -> ToolContext {
        ToolContext::new(
            Arc::new(InMemoryDataStore::new()),
            std::env::temp_dir(),
            Arc::new(SilentReporter),
        )
    }

    /// THE REASON `CancelToken` IS NOT A PLAIN SHARED FLAG. The executor fires a
    /// call's child for that call's deadline; if that reached the parent, every
    /// later call of the session would start out cancelled.
    #[test]
    fn firing_a_child_does_not_fire_its_parent() {
        let parent = CancelToken::new();
        let child = parent.child();
        child.cancel();
        assert!(child.is_cancelled());
        assert!(
            !parent.is_cancelled(),
            "a deadline on one call must not cancel the session"
        );
        assert!(!parent.child().is_cancelled(), "nor the next call");
    }

    /// And the other direction must hold, or Ctrl-C never reaches the tool.
    #[test]
    fn firing_a_parent_or_a_watched_flag_reaches_every_child() {
        static FLAG: AtomicBool = AtomicBool::new(false);
        let parent = CancelToken::watching(&FLAG);
        let child = parent.child();
        let grandchild = child.child();
        assert!(!grandchild.is_cancelled());
        FLAG.store(true, Ordering::SeqCst);
        assert!(child.is_cancelled() && grandchild.is_cancelled());
        FLAG.store(false, Ordering::SeqCst);

        let parent = CancelToken::new();
        let child = parent.child();
        parent.cancel();
        assert!(child.is_cancelled());
    }

    /// A worker that comes back clean must not wash the session: both flags
    /// only ever SET on the way back (audit H2-c, RG-01).
    #[test]
    fn absorbing_a_worker_only_adds_flags() {
        let mut session = context();
        session.taint();
        session.mark_external_content();
        let clean_worker = context();
        session.absorb(&clean_worker);
        assert!(session.session_tainted());
        assert!(session.saw_external_content());

        let mut fresh = context();
        let mut worker = fresh.fork(CancelToken::new());
        worker.taint();
        worker.mark_external_content();
        fresh.absorb(&worker);
        assert!(fresh.session_tainted() && fresh.saw_external_content());

        fresh.reset_chat();
        assert!(!fresh.session_tainted() && !fresh.saw_external_content());
    }
}
