use tacet_skills::SkillStore;
fn main() {
    let skills = SkillStore::default_set();
    let mut n = 0;
    let mut m = 0;
    for (lang, cases) in [
        ("EN", tacet_eval::tool_selection::selection_cases()),
        ("TR", tacet_eval::tool_selection::turkish_selection_cases()),
    ] {
        for c in &cases {
            for s in &c.steps {
                n += 1;
                let hit = skills.matching(&s.message, None).map(|k| k.name.clone());
                if hit.is_some() {
                    m += 1;
                }
                println!(
                    "{lang}\t{}\t{}\t{}\t{}",
                    c.name,
                    s.expected.clone().unwrap_or("-".into()),
                    hit.unwrap_or("<none>".into()),
                    s.message
                );
            }
        }
    }
    eprintln!("steps={n} matched={m}");
}
