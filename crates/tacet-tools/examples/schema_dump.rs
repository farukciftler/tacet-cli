//! Prints every catalog tool's fields, so a fixture for a recovery test can be
//! chosen from what the catalog actually declares rather than from memory.
fn main() {
    let store = std::sync::Arc::new(tacet_tools::data_store::SharedStore::new());
    let memory = tacet_tools::memory::SharedMemory::in_memory();
    let (catalog, _, _) = tacet_tools::catalog::production_catalog(&store, &memory, Some(0));
    for tool in catalog.tools() {
        let schema = tool.schema();
        let fields: Vec<String> = schema
            .fields()
            .iter()
            .map(|f| {
                let req = if f.required { "*" } else { "" };
                match f.schema.choices() {
                    Some(c) => format!("{}{req}:choice[{}]", f.name, c.join("|")),
                    None => format!("{}{req}:free", f.name),
                }
            })
            .collect();
        println!("{:<16} {}", tool.name(), fields.join("  "));
    }
}
