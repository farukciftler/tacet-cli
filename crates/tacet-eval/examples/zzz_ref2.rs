use tacet_skills::{SkillStore, matching::{lowercase, score}};
fn main() {
    let store = SkillStore::default_set();
    // calc without `how much is`
    let msgs = [
      ("web_search-current","How much is the dollar today?"),
      ("web_search-flight","Find flight schedules from London to Paris"),
      ("calculate-percent","How much is 250 lira with a 20 percent discount?"),
      ("calculate-multiply","What is 125 times 8?"),
      ("calculate-discount","Calculate 15% off $80"),
      ("calc-money","How much money do I have left after 3 payments of 120?"),
      ("run_code-sum","Calculate the sum of squares from 1 to 100 using python"),
    ];
    let base: Vec<(String, Vec<String>)> = store.all().map(|s| (s.name.clone(), s.triggers.clone())).collect();
    let mut nohm = base.clone();
    for (n,t) in nohm.iter_mut() { if n=="calc" { t.retain(|x| x != "how much is"); } }
    let pick = |sk: &Vec<(String,Vec<String>)>, msg: &str| -> Option<(String,usize)> {
        let m = lowercase(msg); let mut b: Option<(String,usize)> = None;
        for (n,tr) in sk { let p = score(&m,tr); if p>0 && p>b.as_ref().map_or(0,|(_,x)|*x) { b=Some((n.clone(),p)); } }
        b
    };
    for (n,msg) in msgs {
        let intent = tacet_tools::router::score_intent(msg);
        println!("{n:<22} dominant={:?}  base={:?}  no-how-much-is={:?}",
            intent.dominant(), pick(&base,msg).map(|x|x.0), pick(&nohm,msg).map(|x|x.0));
    }
    // and: does dropping `how much is` change any of the 190 steps?
    let mut changed = vec![];
    for (_lang, cases) in [("EN", tacet_eval::tool_selection::selection_cases()), ("TR", tacet_eval::tool_selection::turkish_selection_cases())] {
        for c in &cases { for s in &c.steps {
            let a = pick(&base,&s.message).map(|x|x.0); let b = pick(&nohm,&s.message).map(|x|x.0);
            if a!=b { changed.push((c.name.clone(), s.message.clone(), a, b)); }
        }}
    }
    println!("\nsteps whose guide changes when `how much is` is dropped: {}", changed.len());
    for (n,m,a,b) in &changed { println!("   {n:<24} {a:?} -> {b:?}   {m}"); }
}
