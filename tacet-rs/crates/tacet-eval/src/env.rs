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

use crate::case::{BUDGET_FILE, FIXED_EPOCH, LONG_FILE, TABLE_FILE};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tacet_core::{
    ArgSchema, Field, Tool, ToolCatalog, ToolContext, ToolFuture, ToolOutcome, boxed,
};
use tacet_tools::calc::CalcTool;
use tacet_tools::create_document::CreateDocumentTool;
use tacet_tools::data_store::SharedStore;
use tacet_tools::read_document::ReadDocumentTool;
use tacet_tools::time::TimeTool;

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

        Ok(Self { dir, store: Arc::new(SharedStore::new()) })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The catalog eval sees. `read_document` is wired to the TYPED store:
    /// keeping a table stored as a table is the condition for the `source_ref`
    /// chain.
    pub fn catalog(&self) -> ToolCatalog {
        let mut c = ToolCatalog::new();
        c.add(Arc::new(CalcTool))
            .add(Arc::new(TimeTool::new().fixed_epoch(FIXED_EPOCH)))
            .add(Arc::new(ReadDocumentTool::with_store(Arc::clone(&self.store))))
            .add(Arc::new(CreateDocumentTool::new()))
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
