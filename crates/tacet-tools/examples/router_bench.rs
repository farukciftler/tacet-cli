//! What `Router::select` costs, at production geometry.
//!
//! NOT A TEST — a stopwatch. It exists because a claim about `select` being slow
//! (or fast) that nobody timed is worth nothing, and because the shape of the
//! cost matters: the sort key's second term is a function call, so the price
//! depends on how many comparisons the sort makes, which depends on how many
//! tools score zero. Three messages with deliberately different profiles.
use std::sync::Arc;
use std::time::Instant;
use tacet_kernel::{ArgSchema, Tool, ToolContext, ToolFuture, ToolOutcome, boxed};

struct Remote(String, String);
impl Tool for Remote {
    fn name(&self) -> &str {
        &self.0
    }
    fn description(&self) -> &str {
        &self.1
    }
    fn schema(&self) -> ArgSchema {
        ArgSchema::empty()
    }
    fn run<'a>(&'a self, _a: serde_json::Value, _c: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move { ToolOutcome::read_ok("x", "x") })
    }
}

fn main() {
    let store = Arc::new(tacet_tools::data_store::SharedStore::new());
    let memory = tacet_tools::memory::SharedMemory::in_memory();
    let (mut catalog, _, _) = tacet_tools::catalog::production_catalog_gated(
        &store,
        &memory,
        Some(0),
        tacet_tools::catalog::AddonGates::all_open(),
    );
    // A CONNECTED SERVER, in the shape one actually arrives in: long Turkish
    // descriptions, because `overlap` walks the description and its cost is the
    // description's length.
    for i in 0..29 {
        catalog.add(Arc::new(Remote(
            format!("serverim_islem_{i:02}"),
            format!(
                "Sunucudaki {i}. islemi calistirir ve sonucunu dondurur. Bu arac uzak \
                 sunucuda calisir, ciktisi cihaza geri gelir ve islem tamamlanana kadar \
                 beklenir. Kullanicinin sunucu, servis, disk, log, proses veya konteyner \
                 hakkindaki sorularinda bu araci cagirin; yerel dosyalar icin degil."
            ),
        )));
    }
    let router = tacet_tools::router::Router::new();
    println!("catalog: {} tools", catalog.tools().len());

    const ITERATIONS: u32 = 2000;
    for message in [
        "Dolar kuru su an ne durumda?",
        "summarize the file budget-2026.md for me",
        "tesekkurler, cok yardimci oldun",
    ] {
        // Warm-up, so the first message does not pay for everything cold.
        for _ in 0..100 {
            std::hint::black_box(router.select(message, &catalog));
        }
        let started = Instant::now();
        for _ in 0..ITERATIONS {
            std::hint::black_box(router.select(message, &catalog));
        }
        let each = started.elapsed() / ITERATIONS;
        println!("  {each:>10.1?}  {message}");
    }
}
