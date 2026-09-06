use tacet_skills::SkillStore;
#[test]
fn which_skill_for_bare_paren_answers() {
    let s = SkillStore::default_set();
    for m in [
        "Summarize the budget-2026.md document",
        "What is inside backup.zip?",
        "Yarin Istanbul'da hava nasil?",
        "How many days until Christmas?",
        "Compute 10 factorial with a python script",
        "What is 125 times 8?",
        "What is 144 divided by 12?",
    ] {
        println!("{:45} -> {}", m, s.matching(m, None).map(|k| k.name.clone()).unwrap_or("<none>".into()));
    }
}
