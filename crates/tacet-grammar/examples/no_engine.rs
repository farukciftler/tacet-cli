//! Constrained generation with NO inference engine anywhere in sight.
//!
//! WHY THIS FILE EXISTS. "Reusable component" is a claim, and a claim in a
//! README is worth what the reader is willing to assume. This is the same claim
//! as thirty lines that compile: a schema is turned into a constraint, a
//! pretend runtime hands over a logit slice, and the tokens the schema forbids
//! come back closed. No model, no tokenizer, no GGUF, no candle — the only
//! dependency in the tree is `tacet-kernel`, which depends on serde and
//! thiserror.
//!
//! Substitute a real runtime for `PretendRuntime` — llama.cpp through a
//! binding, ONNX, your own loop — and the guarantee is unchanged: a token the
//! grammar forbids is `f32::NEG_INFINITY` before the sampler ever sees it, so
//! no sampling strategy can pick it. That is the whole difference between
//! checking output after the fact and making the bad output unrepresentable.
//!
//! Run it: `cargo run -p tacet-grammar --example no_engine`

use std::sync::Arc;
use tacet_grammar::CallConstraint;
use tacet_kernel::{
    ArgSchema, Constrainer, Field, Tool, ToolCatalog, ToolContext, ToolFuture, boxed,
};

/// A tool that does nothing. Only its NAME and SCHEMA matter here — those are
/// what the grammar compiles.
struct Weather;

impl Tool for Weather {
    fn name(&self) -> &str {
        "weather"
    }
    fn description(&self) -> &str {
        "Looks up the weather for a city."
    }
    fn schema(&self) -> ArgSchema {
        ArgSchema::object(vec![
            Field::new("city", ArgSchema::text()).required(),
            // A CLOSED VOCABULARY IS WHERE THIS BUYS THE MOST. `units` compiles
            // into a literal alternation: the automaton cannot emit a third
            // value, so no amount of sampling temperature invents one.
            Field::new("units", ArgSchema::choice(["celsius", "fahrenheit"])),
        ])
    }
    fn run<'a>(&'a self, _args: serde_json::Value, _ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move { unreachable!("this example never executes the call") })
    }
}

/// Stands in for whatever produces logits. A real one would run a model.
struct PretendRuntime {
    vocab: Vec<String>,
}

impl PretendRuntime {
    /// One logit per token, all equally likely — the sampler's job before the
    /// constraint has its say.
    fn logits(&self) -> Vec<f32> {
        vec![0.0; self.vocab.len()]
    }
    fn id_of(&self, text: &str) -> u32 {
        self.vocab.iter().position(|t| t == text).expect("in vocab") as u32
    }
}

fn main() {
    // A vocabulary small enough to print. A real one has 150k entries; nothing
    // in the contract cares which.
    let vocab: Vec<String> = [
        "weather", "(", "{", "}", "\"", "city", ":", "units", "celsius", "kelvin", ",", "London",
        " ", ")",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let runtime = PretendRuntime {
        vocab: vocab.clone(),
    };

    let mut catalog = ToolCatalog::new();
    catalog.add(Arc::new(Weather));

    let constraint = CallConstraint::new(&vocab, &catalog);
    let mut session = constraint.session();

    // Feed the opening of a call, exactly as a generation loop would: mask,
    // pick an allowed token, advance.
    for piece in [
        "weather", "(", "{", "\"", "city", "\"", ":", "\"", "London", "\"", ",",
    ] {
        let mut logits = runtime.logits();
        session.mask(&mut logits);
        let id = runtime.id_of(piece);
        assert!(
            logits[id as usize].is_finite(),
            "{piece:?} should be allowed here"
        );
        session.advance(id).expect("allowed token advances");
    }

    // Now the interesting part. The next field is `units`, whose vocabulary is
    // exactly {celsius, fahrenheit}. Ask what the constraint permits.
    let mut logits = runtime.logits();
    session.mask(&mut logits);

    let open: Vec<&str> = vocab
        .iter()
        .enumerate()
        .filter(|(i, _)| logits[*i].is_finite())
        .map(|(_, t)| t.as_str())
        .collect();

    println!("after `weather({{\"city\":\"London\",` the grammar allows: {open:?}");
    println!();
    println!("`kelvin` is in the vocabulary and closed: {}", {
        let id = runtime.id_of("kelvin") as usize;
        logits[id].is_infinite()
    });
    println!(
        "It is not rejected after the fact — its logit is {} before the sampler runs,",
        logits[runtime.id_of("kelvin") as usize]
    );
    println!("so no temperature, top-p or beam search can select it.");
}
