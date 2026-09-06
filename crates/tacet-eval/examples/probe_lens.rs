use tacet_skills::SkillStore;
fn main() {
    let skills = SkillStore::default_set();
    let mut rows = vec![];
    for (lang, cases) in [
        ("EN", tacet_eval::tool_selection::selection_cases()),
        ("TR", tacet_eval::tool_selection::turkish_selection_cases()),
    ] {
        for c in &cases {
            for s in &c.steps {
                let hit = skills.matching(&s.message, None).map(|k| k.name.clone());
                rows.push((
                    lang,
                    c.name.clone(),
                    s.expected.clone().unwrap_or("-".into()),
                    s.message.clone(),
                    hit,
                ));
            }
        }
    }
    // 1) every step whose expected tool is NOT a tool of the matched skill
    println!("== steps where the injected skill does not own the expected tool ==");
    for (lang, name, exp, msg, hit) in &rows {
        if let Some(h) = hit {
            let sk = skills.skill(h).unwrap();
            if exp != "-" && !sk.tools.iter().any(|t| t == exp) {
                println!("{lang} {name:<26} expected={exp:<16} skill={h:<16} {msg}");
            }
        }
    }
    // 2) which steps match create-document only through "report"
    println!("\n== steps where create-document wins and 'report' is a matched trigger ==");
    for (lang, name, exp, msg, hit) in &rows {
        if hit.as_deref() == Some("create-document") {
            let low = tacet_skills::matching::lowercase(msg);
            if tacet_skills::matching::contains(&low, "report") {
                println!("{lang} {name:<26} expected={exp:<16} {msg}");
            }
        }
    }
}
