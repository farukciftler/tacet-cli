//! TEMPORARY refuter probe — delete after use.
use std::sync::Arc;
use tacet_grammar::CallConstraint;
use tacet_kernel::{ArgSchema, Constrainer, Field, Tool, ToolCatalog, ToolContext, ToolFuture};

struct Named(&'static str, ArgSchema);
impl Tool for Named {
    fn name(&self) -> &str {
        self.0
    }
    fn description(&self) -> &str {
        "A tool."
    }
    fn schema(&self) -> ArgSchema {
        self.1.clone()
    }
    fn run<'a>(&'a self, _a: serde_json::Value, _c: &'a mut ToolContext) -> ToolFuture<'a> {
        unreachable!()
    }
}

fn catalog() -> ToolCatalog {
    let mut c = ToolCatalog::new();
    c.add(Arc::new(Named(
        "find_file",
        ArgSchema::object(vec![
            Field::new("pattern", ArgSchema::text()).required(),
            Field::new("search_content", ArgSchema::bool()),
        ]),
    )));
    c.add(Arc::new(Named(
        "read_document",
        ArgSchema::object(vec![Field::new("path", ArgSchema::text()).required()]),
    )));
    c
}

fn main() {
    // One token per character: the walk is exactly the text.
    let mut vocab: Vec<String> = Vec::new();
    for ch in "abcdefghijklmnopqrstuvwxyz_(){}\":,. \n0123456789=".chars() {
        vocab.push(ch.to_string());
    }
    let constraint = CallConstraint::new(&vocab, &catalog());
    let id = |c: char| vocab.iter().position(|t| t == &c.to_string()).unwrap() as u32;

    for text in [
        "find_file({\"pattern\":\"app.log\"})",
        "find_file ({\"pattern\":\"app.log\"})",
        "find_file (\"app.log\")",
        "find_file  ({\"pattern\":\"a\"})",
        "the (find_file({\"pattern\":\"a\"}))",
    ] {
        let mut s = constraint.session();
        let mut armed_at: Option<usize> = None;
        let mut violated = None;
        for (i, ch) in text.chars().enumerate() {
            // mask first, then advance — the loop's order.
            let mut logits = vec![0.0f32; vocab.len()];
            s.mask(&mut logits);
            let closed = logits[id(ch) as usize] == f32::NEG_INFINITY;
            if s.advance(id(ch)).is_err() {
                violated = Some((i, ch, closed));
                break;
            }
            if s.is_structural() && armed_at.is_none() {
                armed_at = Some(i);
            }
        }
        println!(
            "{text:?}\n   armed_at={armed_at:?} done={} violated={violated:?}",
            s.is_done()
        );
    }

    // What the mask leaves open right after a bare tool name, and after one space.
    for prefix in ["find_file", "find_file ", "the time"] {
        let mut s = constraint.session();
        for ch in prefix.chars() {
            s.advance(id(ch)).unwrap();
        }
        let mut logits = vec![0.0f32; vocab.len()];
        s.mask(&mut logits);
        let open: String = vocab
            .iter()
            .enumerate()
            .filter(|(i, _)| logits[*i] != f32::NEG_INFINITY)
            .map(|(_, t)| t.clone())
            .collect();
        println!("after {prefix:?} the mask leaves open: {open:?}");
    }
}
