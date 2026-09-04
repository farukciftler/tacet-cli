//! The eval environment — the sandbox directory, the fixture files and the tool
//! catalog.
//!
//! WHY A SEPARATE FILE: the runner's readability. But more importantly, that the
//! eval catalog is defined in ONE place: if the CLI's `tools` command and the
//! catalog eval sees drift apart, silent errors of the "eval passed but it is
//! missing in the app" class appear.
//!
//! THE DIRECTORY IS TEMPORARY AND PER CASE: every run opens its own directory. A
//! shared directory would let a `meals.xlsx` left over from a previous run be
//! mistaken for "created" in the next one.

use crate::case::{
    ARCHIVE_ENTRIES, ARCHIVE_FILE, BUDGET_FILE, EMPTY_FILE, FIXED_EPOCH, LONG_FILE, TABLE_FILE,
};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tacet_kernel::{
    ArgSchema, Field, Tool, ToolCatalog, ToolContext, ToolFuture, ToolOutcome, boxed,
};
use tacet_tools::archive::ArchiveTool;
use tacet_tools::calc::CalcTool;
use tacet_tools::checksum::ChecksumTool;
use tacet_tools::create_document::CreateDocumentTool;
use tacet_tools::data_store::SharedStore;
use tacet_tools::edit_document::EditDocumentTool;
use tacet_tools::find_file::FindFileTool;
use tacet_tools::memory::{MemoryTool, SharedMemory};
use tacet_tools::read_document::ReadDocumentTool;
use tacet_tools::time::TimeTool;
use tacet_tools::web_search::WebSearchTool;

/// There is NO real external tool in this turn. The gate's mechanism still has
/// to be built and TESTED, otherwise the gate will be tried for the first time
/// on the day the first real external tool is added. This tool fills that gap:
/// it sends nothing, but it is marked as "sends data out" for the executor.
pub const EXTERNAL_TOOL: &str = "send_out";

pub struct FakeExternalTool;

impl Tool for FakeExternalTool {
    fn name(&self) -> &str {
        EXTERNAL_TOOL
    }

    fn description(&self) -> &str {
        "Sends the given text to the user's own server. Only for explicit send requests."
    }

    fn schema(&self) -> ArgSchema {
        ArgSchema::object(vec![
            Field::new("body", ArgSchema::text().description("Text to send.")).required(),
        ])
    }

    fn run<'a>(&'a self, _args: Value, _ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        // NO NETWORK: this crate makes no calls. Having got this far means the
        // gate opened; that is what is being measured.
        boxed(async move { ToolOutcome::read_ok("sent", "sent_ok") })
    }
}

/// `web_search` WITH ITS SOCKET REMOVED, keeping everything that decides
/// behaviour.
///
/// The name, the description and the schema are the PRODUCTION ones — they are
/// what the router scores and what the model reads — and only `run` is replaced.
/// Wrapping the real tool rather than writing a fake one means a description
/// change in production is measured automatically and nobody has to keep two
/// texts in sync. The same construction as `tool_selection::DryTool`, and it
/// exists twice on purpose: this crate's NO NETWORK rule is per module, and a
/// shared helper would put one `use` between an eval and a socket.
///
/// THE RESULT CARRIES A NUMBER. A dried tool returning prose would make
/// `EvalCase::grounded` vacuous on every web case — there would be nothing for
/// the answer to be grounded IN.
struct DryWebSearch(Arc<dyn Tool>);

impl Tool for DryWebSearch {
    fn name(&self) -> &str {
        self.0.name()
    }
    fn description(&self) -> &str {
        self.0.description()
    }
    fn schema(&self) -> ArgSchema {
        self.0.schema()
    }
    fn taints_session(&self) -> bool {
        self.0.taints_session()
    }
    fn run<'a>(&'a self, _args: Value, _ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move {
            ToolOutcome::read_ok(
                "searched",
                "1. Istanbul weather today: clear, 24 degrees, humidity 54%. \
                 (fixed result — the network is off in this set)",
            )
        })
    }
}

/// The temporary directory the case runs in; it is deleted on drop.
pub struct Env {
    dir: PathBuf,
    pub store: Arc<SharedStore>,
}

/// The counter that makes the directory names unique. A timestamp was not used:
/// two directories opened in the same millisecond would collide, and once eval
/// is parallelized that will happen sooner or later.
static COUNTER: AtomicU64 = AtomicU64::new(0);

impl Env {
    /// Opens the temporary directory and writes the fixture files.
    pub fn setup() -> std::io::Result<Self> {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("tacet-eval-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir)?;

        std::fs::write(dir.join(TABLE_FILE), TABLE_CONTENT)?;
        std::fs::write(dir.join(LONG_FILE), long_content())?;
        // For the TOOL SELECTION cases: `find_file` must have something TO
        // FIND. A model searching an empty directory may pick the right tool
        // and still get "not found" back, and whoever reads the measurement
        // mistakes that for a selection error.
        std::fs::write(dir.join(BUDGET_FILE), BUDGET_CONTENT)?;
        // A file that exists and is empty — see `EMPTY_FILE`. Written rather
        // than left out: "the file is not there" and "the file is there and has
        // nothing in it" are two different answers and the case needs the second.
        std::fs::write(dir.join(EMPTY_FILE), "")?;
        // PACKED WITH PRODUCTION'S OWN WRITER. `pack` cannot fail on this input
        // (two dozen tiny entries, far under every zip32 limit), and if it ever
        // did the error would surface here as a setup failure rather than as a
        // mysterious `archive` case — which is why the `expect` carries the
        // reason instead of a `?`: `ZipError` is not an `io::Error`.
        std::fs::write(dir.join(ARCHIVE_FILE), archive_bytes())?;

        Ok(Self {
            dir,
            store: Arc::new(SharedStore::new()),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The catalog eval sees. `read_document` is wired to the TYPED store:
    /// keeping a table stored as a table is the condition for the `source_ref`
    /// chain.
    /// WHICH TOOLS ARE HERE, AND WHY THE LIST IS NOT THE PRODUCTION CATALOG.
    ///
    /// This set measures Tacet's LOGIC, so a tool belongs here when its logic can
    /// be claimed without a network, without the host's configuration and without
    /// a side effect that outlives the temporary directory. Four were missing for
    /// that reason and turned out to qualify anyway:
    ///
    ///   `edit_document` — writes inside the sandbox, nothing else.
    ///   `find_file`     — reads the temporary directory; the fixtures are already
    ///                     written for it (`BUDGET_FILE` exists to be found).
    ///   `remember`      — an in-memory store, opened fresh per case.
    ///   `web_search`    — DRIED OUT (see `DryWebSearch`): the name, description
    ///                     and schema are the production ones, the body opens no
    ///                     socket. The rule of this crate is NO NETWORK, and it is
    ///                     not bent for a measurement.
    ///   `archive`       — pure local computation over a fixture .zip written by
    ///                     `Env::setup`; nothing it does outlives the directory.
    ///   `checksum`      — pure local computation, and the ONE tool in the
    ///                     catalog whose correct answer is a value that can be
    ///                     written down in advance (see `checksum-digest`).
    ///
    /// STILL DELIBERATELY ABSENT, one more since the list above was written:
    ///   `db_write` — it is not in the PRODUCTION catalog either (see the note in
    ///     `tacet_tools::catalog`; `tacet-cli` adds it and nobody else). It needs
    ///     a `sqlite3` binary and a confirmation sink, so a case for it would
    ///     pass or fail by host — the same reason `run_code` is out — and eval
    ///     holding a tool that WRITES to the user's databases is a trade this
    ///     crate should not make for a measurement.
    ///
    /// DELIBERATELY ABSENT, and each for a reason that is not "we forgot":
    ///   `run_code`/`write_code` — bound to a discovery gate (`sandbox-exec` on
    ///     macOS, `bwrap` on Linux, nothing on Windows). A case for them would
    ///     pass or fail by platform, which measures the host and not this code.
    ///   `git` — needs a repository. The temporary directory is not one, and
    ///     making it one per case would measure `git init`.
    ///   `web_fetch` — the same shape as `web_search` once dried; a second dried
    ///     tool would add cases without adding a claim.
    ///   `calendar` — macOS-only by construction (`osascript`).
    pub fn catalog(&self) -> ToolCatalog {
        let mut c = ToolCatalog::new();
        c.add(Arc::new(CalcTool))
            .add(Arc::new(TimeTool::new().fixed_epoch(FIXED_EPOCH)))
            .add(Arc::new(ReadDocumentTool::with_store(Arc::clone(
                &self.store,
            ))))
            .add(Arc::new(CreateDocumentTool::new()))
            .add(Arc::new(EditDocumentTool::new()))
            .add(Arc::new(FindFileTool::new()))
            // `in_memory`, NOT the user's store: an eval that wrote into the real
            // memory file would leave the maintainer's own notes behind it.
            .add(Arc::new(MemoryTool::new(SharedMemory::in_memory())))
            .add(Arc::new(DryWebSearch(Arc::new(WebSearchTool::new()))))
            // `archive` TAKES THE STORE and `checksum` DOES NOT — the same split
            // as the production catalog, and for the same reason: a listing of
            // two dozen entries is bulk data and belongs behind a `source_ref`,
            // a digest is 64 characters whatever the file's size. Passing the
            // store to `archive` here is what makes `archive-listing-by-ref`
            // measure the channel rather than the fallback.
            .add(Arc::new(ArchiveTool::with_store(Arc::clone(&self.store))))
            .add(Arc::new(ChecksumTool::new()))
            .add(Arc::new(FakeExternalTool));
        c
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        // The error is swallowed: a failed cleanup must not spoil the eval
        // result.
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

const BUDGET_CONTENT: &str = "# 2026 budget\n\n\
                              Rent: 18000 TL\n\
                              Kitchen: 9000 TL\n\
                              Transport: 2500 TL\n";

const TABLE_CONTENT: &str = "| Day | Meal |\n\
                             | --- | --- |\n\
                             | Monday | Lentils |\n\
                             | Tuesday | Rice |\n\
                             | Wednesday | Pasta |\n";

/// The bytes of `ARCHIVE_FILE`, packed by `tacet_zip::pack`.
///
/// DETERMINISTIC BY CONSTRUCTION: the entry names and bodies are generated from
/// a counter, and `pack` writes no timestamp that varies (the eval run must be
/// bit-for-bit reproducible — see `the_run_is_deterministic`).
fn archive_bytes() -> Vec<u8> {
    let entries: Vec<tacet_zip::ZipEntry> = (1..=ARCHIVE_ENTRIES)
        .map(|i| {
            tacet_zip::ZipEntry::new(
                format!("notes/entry-{i:02}.txt"),
                format!("entry {i} of the fixture archive\n").into_bytes(),
            )
        })
        .collect();
    tacet_zip::pack(&entries).expect("the fixture archive is far inside every zip32 limit")
}

/// Content that comfortably exceeds `TEXT_STORE_THRESHOLD` (1500 bytes) — more
/// than twice the threshold so that triggering the bypass channel is
/// GUARANTEED.
fn long_content() -> String {
    let mut s = String::from("# Long list\n");
    for i in 1..=200 {
        s.push_str(&format!("- line {i}: this is a filler line\n"));
    }
    s
}
