//! A guide must name the arguments the model has to fill.
//!
//! WHY IT LIVES IN `tacet-cli` AND NOT IN `tacet-skills`. The question spans two
//! crates: the guide text is a skill, the required fields are a tool's schema,
//! and `tacet-tools` already depends on `tacet-skills` for the trigger matcher.
//! Asking it here is the only direction that does not invert that.

/// EVERY REQUIRED ARGUMENT A GUIDE'S TOOL TAKES MUST BE NAMED SOMEWHERE IN THE
/// GUIDE.
///
/// WHAT THIS FOUND. `code.md` guides two tools and named the arguments of one.
/// `write_code`'s `lines` and `file_name` were spelled out with a rule each;
/// `run_code`'s `code` — its only required field — appeared nowhere. So the
/// model got a concrete shape for the tool it was already getting right and
/// prose for the one it was not: `run_code-primes` in the shipped baseline
/// calls `run_code` three times.
///
/// The guide is the last block before the question and this file's own header
/// says the last blocks carry the most weight in a small model. A guide that
/// describes a tool without naming the field it must fill is spending that
/// position on something the model cannot act on.
///
/// ONLY THE REQUIRED FIELDS. An optional argument the message never mentions is
/// one the model should leave out, and naming all of them would push several
/// guides past their budget to say things that are already in the signature.
#[test]
fn a_guide_names_the_required_arguments_of_the_tools_it_guides() {
    use std::sync::Arc;

    let store = Arc::new(tacet_tools::data_store::SharedStore::new());
    let memory = tacet_tools::memory::SharedMemory::in_memory();
    let (catalog, _, _) =
        tacet_tools::catalog::production_catalog_with(&store, &memory, Some(0), true);

    let skills = tacet_skills::SkillStore::default_set();
    let mut missing: Vec<String> = Vec::new();
    for skill in skills.all() {
        let text = tacet_skills::injection_text(skill);
        for tool_name in &skill.tools {
            let Some(tool) = catalog.find(tool_name) else {
                // A tool this build does not carry (an addon, or macOS-only).
                // `has_tools` already keeps its guide out of the prompt.
                continue;
            };
            for field in tool
                .schema()
                .fields()
                .iter()
                .filter(|f| f.required)
                .map(|f| f.name.clone())
            {
                if !text.contains(&format!("`{field}`")) && !text.contains(&format!("\"{field}\""))
                {
                    missing.push(format!(
                        "{} guides {tool_name} and never names its required `{field}`",
                        skill.name
                    ));
                }
            }
        }
    }
    assert!(
        missing.is_empty(),
        "a guide occupies the last block before the question; one that describes \
         a tool without naming the field it must fill spends that position on \
         something the model cannot act on:\n  {}",
        missing.join("\n  ")
    );
}

/// EVERY GUIDE MUST SHOW A COMPLETE CALL, and this one is backed by a run.
///
/// `calc` opened with "Do arithmetic with the `calculate` tool" and then showed
/// an argument VALUE — `E.g. "(1250+890)*1.2"` — and nothing else. Every other
/// guide in the package opens with a real call: `read_document({"path":…})`,
/// `git({"action":"status"})`, `web_search({"query":…})`.
///
/// MEASURED, 7 Sep 2026, qwen3-4b on Metal, 184 cases. Turkish arithmetic
/// messages started matching `calc` for the first time that night, so ten of
/// them received this guide, and TEN ARITHMETIC CASES BROKE. The model answered
/// `(347 + 268)`, `(2^10)`, `(480 * 18) / 100 = 86.4` — the shape of the
/// example it was shown, as prose, with no call made. It looks like arithmetic
/// and no arithmetic was done.
///
/// The guide is the last block before the question and the model imitates what
/// is in it. Showing an argument teaches it to write an argument.
#[test]
fn every_guide_shows_a_complete_call_for_a_tool_it_guides() {
    use std::sync::Arc;

    let store = Arc::new(tacet_tools::data_store::SharedStore::new());
    let memory = tacet_tools::memory::SharedMemory::in_memory();
    let (catalog, _, _) =
        tacet_tools::catalog::production_catalog_with(&store, &memory, Some(0), true);
    let names: Vec<String> = catalog.names().into_iter().map(String::from).collect();

    let skills = tacet_skills::SkillStore::default_set();
    let mut missing: Vec<String> = Vec::new();
    for skill in skills.all() {
        // Only guides whose tools this build actually carries: an addon guide
        // can never be injected here and its text is not this test's subject.
        if !skill.has_tools(Some(&names)) {
            continue;
        }
        let text = tacet_skills::injection_text(skill);
        let shows_a_call = skill
            .tools
            .iter()
            .any(|t| text.contains(&format!("{t}({{\"")));
        if !shows_a_call {
            missing.push(format!(
                "{} guides {:?} and shows none of them being CALLED",
                skill.name, skill.tools
            ));
        }
    }
    assert!(
        missing.is_empty(),
        "the guide is the last block before the question and the model imitates \
         what is in it — a guide that shows an argument teaches it to write an \
         argument, which cost ten arithmetic cases in a measured run:\n  {}",
        missing.join("\n  ")
    );
}

/// A GUIDE MUST NOT CONTAIN A BARE EXPRESSION — A NEGATIVE EXAMPLE IS STILL AN
/// EXAMPLE.
///
/// MEASURED TWICE, and the second time the fix was the cause.
///
/// Run A (baaeda4, 184 cases, qwen3-4b on Metal): `calc` showed
/// `E.g. "(1250+890)*1.2"` — an argument value, no call — and ten arithmetic
/// cases came back with the model ANSWERING `(347 + 268)`, `(2^10)`,
/// `(480 * 18) / 100 = 86.4`. No call made.
///
/// Run B (50c33d7), after "fixing" it by adding the call: the guide also gained
///
///     - WRITE THE CALL, not the sum. `(347 + 268)` as an answer is a failure
///
/// and the suite's own case is "Could you add 347 and 268?". The model answered
/// `(347 + 268)`. THE COUNTER-EXAMPLE WAS COPIED. Twelve arithmetic cases
/// broke and the run was a REAL LOSS at 95% — the warning against the behaviour
/// taught the behaviour.
///
/// This codebase already knew: `create-document` carries "The table above is a
/// FORMAT example, never content. Never copy its rows", written after the same
/// thing happened with a table. The rule generalises — a small model imitates
/// what is in the last block before the question, and it does not read the word
/// "not".
///
/// So an example in a guide may only ever be a CALL. Anything that looks like
/// output — a bare parenthesised expression, an `=` with numbers on both sides —
/// must not appear at all.
#[test]
fn no_guide_shows_something_that_looks_like_an_answer() {
    let skills = tacet_skills::SkillStore::default_set();
    let mut found: Vec<String> = Vec::new();

    for skill in skills.all() {
        let text = tacet_skills::injection_text(skill);
        // Blank out every real call, then look at what is left. A call is
        // exactly what a guide is FOR; the question is what else is in there.
        let mut rest = text.clone();
        for tool in &skill.tools {
            while let Some(i) = rest.find(&format!("{tool}({{")) {
                let end = rest[i..].find(")").map(|e| i + e + 1).unwrap_or(rest.len());
                rest.replace_range(i..end, " ");
            }
        }
        for line in rest.lines() {
            let l = line.trim();
            // A parenthesised group made only of digits, operators and spaces.
            let mut depth = 0usize;
            let mut group = String::new();
            for c in l.chars() {
                match c {
                    '(' => {
                        depth += 1;
                        group.clear();
                    }
                    ')' if depth > 0 => {
                        depth -= 1;
                        let g = group.trim();
                        let numeric = g.chars().any(|c| c.is_ascii_digit())
                            && g.chars()
                                .all(|c| c.is_ascii_digit() || " +-*/^%.".contains(c));
                        if numeric && g.len() >= 3 {
                            found.push(format!("{}: ({g}) in {l:?}", skill.name));
                        }
                    }
                    _ if depth > 0 => group.push(c),
                    _ => {}
                }
            }
        }
    }

    assert!(
        found.is_empty(),
        "a guide contains a bare arithmetic expression outside a call. A small \
         model imitates what is in the last block before the question and does \
         not read the word \"not\" — writing the bad answer down IS teaching it, \
         and it cost twelve arithmetic cases and a measured REAL LOSS:\n  {}",
        found.join("\n  ")
    );
}
