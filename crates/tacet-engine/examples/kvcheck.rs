fn main() {
    for name in ["qwen3-4b", "gemma3-4b", "qwen2.5-3b", "qwen3-8b"] {
        let dir = format!("{}/models/{name}", std::env::var("HOME").unwrap());
        let Some(g) = std::fs::read_dir(&dir).ok().and_then(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.path())
                .find(|p| p.extension().is_some_and(|x| x == "gguf"))
        }) else {
            continue;
        };
        let ctx = tacet_engine::gguf_context_length(&g);
        let per = tacet_engine::gguf_kv_bytes_per_token(&g);
        println!(
            "  {name:<12} declared={ctx:?} bytes/token={per:?} → budget={}",
            tacet_engine::context_budget(ctx, per)
        );
    }
}
