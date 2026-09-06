use tacet_skills::SkillStore;
#[test]
fn show_truncated_guides() {
    let s = SkillStore::default_set();
    for name in ["read-document", "remember", "code", "calc", "web-search"] {
        let sk = s.skill(name).unwrap();
        let t = tacet_skills::injection::injection_text(sk);
        let cut: String = t.chars().take(700).collect();
        println!(
            "\n########## {name} (full {} chars) — WHAT THE MODEL GETS:\n{}\n<<<END>>>",
            t.chars().count(),
            cut
        );
    }
}
#[test]
fn which_skill_matches() {
    let s = SkillStore::default_set();
    for m in [
        "How much is the dollar today?",
        "Remember where I parked my car",
        "Read the latest entries from app.log",
        "List the notes you keep about me.",
        "Calculate 15% off $80",
        "How much is 250 lira with a 20 percent discount?",
        "Find flight schedules from London to Paris",
    ] {
        let hit = s
            .matching(m, None)
            .map(|k| k.name.clone())
            .unwrap_or("<none>".into());
        println!("{:50} -> {}", m, hit);
    }
}
