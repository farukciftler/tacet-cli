use tacet_tools::time::DateTime;
fn main() {
    let now = DateTime::new(2026, 9, 6, 15, 30, 0).expect("valid");
    for raw in [
        "next week",
        "this weekend",
        "next week friday",
        "tomorrow",
        "haftaya",
        "onumuzdeki hafta",
        "gelecek hafta",
    ] {
        let r = tacet_tools::time::TimeResolver::resolve(raw, now);
        println!(
            "{raw:>20} -> {}",
            match r {
                Some(x) => format!("{:?}", x.an),
                None => "NONE (tool refuses)".into(),
            }
        );
    }
}
