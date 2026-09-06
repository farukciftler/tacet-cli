use tacet_skills::{
    SkillStore,
    matching::{contains, lowercase, score},
};

fn main() {
    let store = SkillStore::default_set();
    let sk: Vec<(String, Vec<String>, Vec<String>)> = store
        .all()
        .map(|s| (s.name.clone(), s.triggers.clone(), s.tools.clone()))
        .collect();

    let mut steps: Vec<(String, String, String, String, String)> = vec![];
    for (lang, cases) in [
        ("EN", tacet_eval::tool_selection::selection_cases()),
        ("TR", tacet_eval::tool_selection::turkish_selection_cases()),
    ] {
        for c in &cases {
            for s in &c.steps {
                steps.push((
                    lang.into(),
                    format!("{:?}", c.category),
                    c.name.clone(),
                    s.expected.clone().unwrap_or("-".into()),
                    s.message.clone(),
                ));
            }
        }
    }
    println!("TOTAL STEPS {}", steps.len());
    let en = steps.iter().filter(|s| s.0 == "EN").count();
    let tr = steps.iter().filter(|s| s.0 == "TR").count();
    println!("EN {en} TR {tr}");

    let pick = |msg: &str| -> Option<(String, usize)> {
        let m = lowercase(msg);
        let mut best: Option<(String, usize)> = None;
        for (n, tr, _) in &sk {
            let p = score(&m, tr);
            if p > 0 && p > best.as_ref().map_or(0, |(_, b)| *b) {
                best = Some((n.clone(), p));
            }
        }
        best
    };
    let owns = |n: &str, exp: &str| {
        sk.iter()
            .find(|(x, _, _)| x == n)
            .map(|(_, _, t)| t.iter().any(|y| y == exp))
            .unwrap_or(false)
    };

    let mut en_hit = 0;
    let mut tr_hit = 0;
    let mut tr_wrong = 0;
    let mut wrong = 0;
    let mut irr_matched = vec![];
    for (lang, cat, name, exp, msg) in &steps {
        let h = pick(msg);
        if lang == "EN" && h.is_some() {
            en_hit += 1;
        }
        if lang == "TR" && h.is_some() {
            tr_hit += 1;
        }
        if let Some((hn, _)) = &h {
            if exp != "-" && !owns(hn, exp) {
                wrong += 1;
                if lang == "TR" {
                    tr_wrong += 1;
                }
            }
            if cat == "Irrelevance" {
                irr_matched.push((name.clone(), hn.clone(), msg.clone()));
            }
        }
    }
    println!("EN steps with a guide: {en_hit}/{en}   TR: {tr_hit}/{tr}");
    println!("WRONG-GUIDE injections (skill does not own expected tool): {wrong}  (TR {tr_wrong})");
    println!("IRRELEVANCE steps matching a skill: {}", irr_matched.len());
    for r in &irr_matched {
        println!("   IRR {} -> {} | {}", r.0, r.1, r.2);
    }
    let irr_total = steps.iter().filter(|s| s.1 == "Irrelevance").count();
    println!("IRRELEVANCE steps total: {irr_total}");

    println!("\n== the 31 recorded-failure messages ==");
    let fails = [
        (
            "calendar-events",
            "calendar",
            "List all my calendar events for next week",
        ),
        ("calculate-multiply", "calculate", "What is 125 times 8?"),
        (
            "calculate-percent",
            "calculate",
            "How much is 250 lira with a 20 percent discount?",
        ),
        ("calculate-discount", "calculate", "Calculate 15% off $80"),
        (
            "read_document-log",
            "read_document",
            "Read the latest entries from app.log",
        ),
        (
            "edit_document-row",
            "edit_document",
            "Add the row 'Thursday | Chickpeas' to the file report.md.",
        ),
        (
            "edit_document-update-line",
            "edit_document",
            "Replace line 5 in report.md with updated figures",
        ),
        (
            "web_search-current",
            "web_search",
            "How much is the dollar today?",
        ),
        (
            "web_search-flight",
            "web_search",
            "Find flight schedules from London to Paris",
        ),
        (
            "remember-list",
            "remember",
            "List the notes you keep about me.",
        ),
        (
            "remember-car-park",
            "remember",
            "Remember where I parked my car",
        ),
        (
            "run_code-factorial",
            "run_code",
            "Compute 10 factorial with a python script",
        ),
        (
            "write_code-script",
            "write_code",
            "Write me a python script that finds prime numbers, and save it as a file.",
        ),
        (
            "write_code-converter",
            "write_code",
            "Write a script that converts temperature data from Celsius to Fahrenheit and put it in my folder.",
        ),
        ("chat-bored", "-", "I'm feeling bored, tell me a joke."),
        (
            "checksum-verify",
            "checksum",
            "Check this download against the checksum they published",
        ),
        (
            "chain-document",
            "edit_document",
            "Change Tuesday from Rice to Beans.",
        ),
        (
            "chain-code-write",
            "write_code",
            "Save that script to primes.py.",
        ),
        ("tr-hesap-yuzde", "calculate", "480'in yüzde 18'i ne kadar?"),
        ("tr-hesap-cikarma", "calculate", "1000 eksi 375 kaç eder?"),
        ("tr-hesap-karekok", "calculate", "81'in karekökü nedir?"),
        (
            "tr-hesap-ortalama",
            "calculate",
            "10, 20 ve 30'un ortalaması kaçtır?",
        ),
        (
            "tr-belge-duzenle",
            "edit_document",
            "Az önceki tabloya bir satır daha ekle",
        ),
        (
            "tr-belge-markdown",
            "create_document",
            "Toplantı kararlarını toplantı.md adıyla kaydet",
        ),
        (
            "tr-belge-satir-sil",
            "edit_document",
            "notlar.md dosyasındaki 3. satırı sil",
        ),
        (
            "tr-dosya-ara",
            "find_file",
            "Bütçeyle ilgili notu hangi dosyaya yazmıştım?",
        ),
        (
            "tr-kod-hesapla",
            "run_code",
            "Python ile 1'den 50'ye kadar olan sayıların toplamını çalıştır",
        ),
        (
            "tr-web-site-oku",
            "web_fetch",
            "https://example.com sayfasında ne anlatılıyor?",
        ),
        (
            "tr-web-arama",
            "web_search",
            "Türkiye'nin 2026 yılı enflasyon oranı haberleri ne durumda?",
        ),
        (
            "tr-hafiza-oku",
            "remember",
            "Benim hakkımda aklında tuttuğun notları listele",
        ),
        ("tr-tesekkur", "-", "Çok teşekkürler, harikaydı!"),
    ];
    for (n, exp, msg) in fails {
        let m = lowercase(msg);
        let mut v: Vec<(String, usize, Vec<String>)> = sk
            .iter()
            .map(|(nm, tr, _)| {
                (
                    nm.clone(),
                    score(&m, tr),
                    tr.iter().filter(|t| contains(&m, t)).cloned().collect(),
                )
            })
            .filter(|x| x.1 > 0)
            .collect();
        v.sort_by_key(|x| std::cmp::Reverse(x.1));
        let win = v
            .first()
            .map(|x| format!("{}({}) fired={:?}", x.0, x.1, x.2))
            .unwrap_or("(none)".into());
        let second = v
            .get(1)
            .map(|x| format!(" | 2nd {}({})", x.0, x.1))
            .unwrap_or_default();
        let ok = v.first().map(|x| owns(&x.0, exp)).unwrap_or(false);
        println!(
            "{n:<26} exp={exp:<15} {} {win}{second}",
            if ok { "OK " } else { "BAD" }
        );
    }

    println!("\n== cores: does each core carry a literal `tool(` for a declared tool? ==");
    for s in store.all() {
        let (core, tail) = tacet_skills::injection::split_core(&s.text);
        let body = tacet_skills::injection::injection_body(&s.text, 700);
        let has: Vec<&String> = s
            .tools
            .iter()
            .filter(|t| core.contains(&format!("{t}(")))
            .collect();
        println!(
            "{:<16} core={:>4} tail={:>4} inject={:>4} tail_landed={} tools_shown={:?}/{:?}",
            s.name,
            core.chars().count(),
            tail.chars().count(),
            body.chars().count(),
            body.chars().count() > core.chars().count(),
            has,
            s.tools
        );
    }

    println!("\n== ALL wrong-guide injections (15 claimed) ==");
    for (lang, _cat, name, exp, msg) in &steps {
        if let Some((hn, _)) = pick(msg) {
            if exp != "-" && !owns(&hn, exp) {
                println!("{lang} {name:<28} exp={exp:<16} got={hn:<16} {msg}");
            }
        }
    }
    println!("\n== every step whose message carries http ==");
    for (lang, _cat, name, exp, msg) in &steps {
        if msg.contains("http") || msg.to_lowercase().contains("website") || msg.contains(".com") {
            println!("{lang} {name:<28} exp={exp:<16} got={:?} {msg}", pick(msg));
        }
    }
    println!("\n== steps where create-document wins AND 'report' fired ==");
    for (lang, _cat, name, exp, msg) in &steps {
        if let Some((hn, _)) = pick(msg) {
            if hn == "create-document" && contains(&lowercase(msg), "report") {
                println!("{lang} {name:<28} exp={exp:<16} {msg}");
            }
        }
    }
    println!("\n== steps where a bare format word (pdf/markdown/excel/docx/xlsx) fired ==");
    for (lang, _cat, name, exp, msg) in &steps {
        let m = lowercase(msg);
        let fired: Vec<&str> = [
            "pdf", "markdown", "excel", "docx", "xlsx", "word", "export", "dump",
        ]
        .iter()
        .filter(|w| contains(&m, w))
        .cloned()
        .collect();
        if !fired.is_empty() {
            println!(
                "{lang} {name:<28} exp={exp:<16} got={:?} fired={fired:?} {msg}",
                pick(msg).map(|x| x.0)
            );
        }
    }
    println!(
        "\n== DIACRITIC FOLD: how many of the 190 messages change their skill if the message is ASCII-folded? =="
    );
    fn fold(s: &str) -> String {
        s.chars()
            .map(|c| match c {
                'ı' => 'i',
                'ş' => 's',
                'ğ' => 'g',
                'ü' => 'u',
                'ö' => 'o',
                'ç' => 'c',
                'â' => 'a',
                'î' => 'i',
                'û' => 'u',
                x => x,
            })
            .collect()
    }
    let mut moved = 0;
    for (_l, _c, name, _e, msg) in &steps {
        let a = pick(msg).map(|x| x.0);
        let m2 = fold(&lowercase(msg));
        let mut best: Option<(String, usize)> = None;
        for (n, tr, _) in &sk {
            let p = score(&m2, tr);
            if p > 0 && p > best.as_ref().map_or(0, |(_, b)| *b) {
                best = Some((n.clone(), p));
            }
        }
        let b = best.map(|x| x.0);
        if a != b {
            moved += 1;
            println!("   FOLD CHANGES {name}: {a:?} -> {b:?}");
        }
    }
    println!("   messages whose skill changes under folding, with TODAY's triggers: {moved}/190");
}
