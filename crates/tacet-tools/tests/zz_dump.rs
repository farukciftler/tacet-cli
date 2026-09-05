use std::sync::Arc;
use tacet_tools::catalog::{production_catalog_gated, AddonGates};
use tacet_tools::data_store::SharedStore;
use tacet_tools::memory::SharedMemory;

#[test]
fn dump() {
    let store = Arc::new(SharedStore::new());
    let memory = SharedMemory::in_memory();
    let (c, _, _) = production_catalog_gated(&store, &memory, None, AddonGates::all_open());
    for t in c.tools() {
        println!("=== {} :: {}", t.name(), t.schema().short_signature());
        for f in t.schema().fields() {
            let ch = f.schema.choices().map(|c| c.join("|")).unwrap_or_default();
            println!("    - {} req={} choices=[{}] desc={:?}", f.name, f.required, ch, &f.schema.description);
        }
        println!("    DESC: {}", t.description());
    }
}
