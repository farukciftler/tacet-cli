use std::sync::Arc;
use tacet_grammar::{CallConstraint, Grammar, TokenMask};
use tacet_kernel::{ArgSchema, ConstraintSession, Field, Tool, ToolContext, ToolFuture, ToolOutcome, boxed, Constrainer};

#[test]
fn seventeen_spaces_inside_a_string_body() {
    let schema = ArgSchema::object(vec![Field::new("code", ArgSchema::text()).required()]);
    let g = Grammar::compile(&schema);
    let mut st = g.state();
    st.advance(r#"{"code":""#).expect("open");
    // 16 spaces fine?
    for i in 0..40 {
        match st.advance(" ") {
            Ok(()) => {}
            Err(e) => { println!("space #{} rejected: {e:?}", i + 1); break; }
        }
    }
}

#[test]
fn the_mask_offers_a_space_the_automaton_refuses() {
    let schema = ArgSchema::object(vec![Field::new("code", ArgSchema::text()).required()]);
    let g = Grammar::compile(&schema);
    let mut st = g.state();
    st.advance(r#"{"code":""#).unwrap();
    st.advance(&" ".repeat(16)).unwrap();
    let allowed = st.allowed_prefixes();
    println!("contains(' ') after 16 spaces = {}", allowed.contains(' '));
    println!("is_space_free = {}", allowed.is_space_free());
    println!("is_text_body = {}", allowed.is_text_body());
    let vocab: Vec<String> = vec!["a".into(), " ".into(), "  ".into(), "x".into()];
    let m = TokenMask::new(&vocab);
    let mask = m.mask(&st);
    println!("mask for [a, ' ', '  ', x] = {mask:?}");
    println!("advance(' ') = {:?}", st.clone().advance(" ").is_ok());
}

struct CodeTool;
impl Tool for CodeTool {
    fn name(&self) -> &str { "write_code" }
    fn description(&self) -> &str { "writes code" }
    fn schema(&self) -> ArgSchema {
        ArgSchema::object(vec![Field::new("code", ArgSchema::text()).required()])
    }
    fn run<'a>(&'a self, _a: serde_json::Value, _c: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move { ToolOutcome::read_ok("ok", "ok") })
    }
}

#[test]
fn a_deeply_indented_python_line_breaks_the_call() {
    let mut cat = tacet_kernel::ToolCatalog::new();
    cat.add(Arc::new(CodeTool));
    let vocab: Vec<String> = (0..0x1000u32)
        .map(|i| char::from_u32(i).map(String::from).unwrap_or_default())
        .collect();
    let k = CallConstraint::new(&vocab, &cat);
    let mut s = k.session();
    let text = r#"write_code({"code":"def f():"#;
    for c in text.chars() { s.advance(c as u32).expect("prefix ok"); }
    // 20 spaces of indentation (5 levels of 4)
    for i in 0..24 {
        if let Err(e) = s.advance(' ' as u32) {
            println!("indent space #{} refused by the constraint: {e:?}", i + 1);
            return;
        }
    }
    println!("24 spaces accepted");
}
