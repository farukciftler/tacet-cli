//! The tool SELECTION measurement — "did the model call the right tool".
//!
//! WHY A SEPARATE SET: the set in `case.rs` measures Tacet's LOGIC and
//! therefore runs with `FakeEngine` — there the model's choice is pinned by the
//! script. What is measured here is the exact opposite: NO script, the FULL
//! catalog, and one single question — "which tool did the model call when it saw
//! the user's sentence". This measurement is only meaningful with a REAL engine;
//! run with FakeEngine, what it measures is its own script.
//!
//! TWO NUMBERS ARE REPORTED SEPARATELY — HIT RATE and IRRELEVANCE. Every change
//! that makes the tools get called more aggressively raises the hit rate and, in
//! the same move, starts calling tools for a greeting. A single "success rate"
//! hides that trade-off: 6 irrelevance cases get lost among 23 tool cases and a
//! regression looks like a couple of percent of wobble. Separate numbers make
//! the trade-off VISIBLE.
//!
//! THE CATALOG IS THE SAME AS IN PRODUCTION (see
//! `tacet_tools::catalog::production_catalog`). A selection measured with a
//! shortened catalog is not the selection the application makes: if the model
//! chooses among 10 tools, the measurement must see 10 tools too.
//!
//! NO NETWORK. `web_search`/`web_fetch` ARE DRIED OUT: the name, the description
//! and the schema are preserved EXACTLY (those are what determine the
//! selection), only the body returns a fixed string instead of opening a socket.
//! That way the question "was the web tool selected" is answered independently
//! of whether the user's search server happens to be up.
//!
//! LANGUAGE: the case messages are English. Turkish phrasings measured earlier
//! were carried over one to one, so the intents are the same ones; if
//! multilingual selection is to be measured, a separate set is written — a
//! mixed-language set makes a single hit-rate number unreadable.

use crate::case::FIXED_EPOCH;
use crate::env::Env;
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use tacet_engine::{
    EngineProvider, FINAL_PASS_INSTRUCTION, MAX_TURNS, Prompt, SYSTEM_INSTRUCTIONS,
    SamplingSetting, Turn, wait,
};
use tacet_grammar::CallConstraint;
use tacet_kernel::{
    ArgSchema, Tool, ToolCatalog, ToolContext, ToolFuture, ToolOutcome, TraceCollector, boxed,
};
use tacet_tools::executor::ToolExecutor;
use tacet_tools::memory::SharedMemory;
use tacet_tools::router::Router;

/// The names of the tools that open the network — they are dried out in this set.
const TO_DRY: &[&str] = &["web_search", "web_fetch"];

/// THE TOOLS BOUND TO THE DISCOVERY GATE — THEY MAY NOT BE IN THE CATALOG.
///
/// `run_code` and `write_code` are only in the catalog when the machine has an
/// interpreter AND a MEASURED network shield (see `RunCodeTool::discover`):
/// `sandbox-exec` on macOS, `bwrap` on Linux; there is no equivalent on Windows.
/// So the absence of these two tools IS NOT A REGRESSION, it is a fact of the
/// platform — and the whole reason this list exists is to be able to tell the
/// two apart: a tool that is NOT in this list dropping out of the catalog is
/// still a regression and fails the test.
///
/// `calendar` JOINED THE LIST WHEN LINUX CI SAID SO, and it is worth writing
/// down which of the two claims turned out to be wrong. The tool reaches the
/// user's calendar through `/usr/bin/osascript`, so it is macOS-only by
/// construction (`#[cfg(target_os = "macos")]` in `calendar.rs`) — but the
/// comment above says the list is for tools bound to a DISCOVERY gate, and
/// `calendar` is bound to a COMPILE-TIME one. The distinction does not matter to
/// the test, which asks a single question: is this name absent for a reason the
/// platform can explain, or is it a typo in a case? Both reasons are the
/// platform. What DID matter is that nobody could answer it before the tool ran
/// on something other than a Mac — this repository's own eval was red on Linux
/// for a tool that was working exactly as designed.
const DISCOVERY_BOUND: &[&str] = &["run_code", "write_code", "calendar"];

// ---------------------------------------------------------------------------
// The dry tool
// ---------------------------------------------------------------------------

/// Neutralizes a tool while PRESERVING ITS IDENTITY.
///
/// What determines the selection is the tool's name, description and schema;
/// not its body. This wrapper passes all three through unchanged and only
/// replaces `run`. The reason for wrapping the real tool instead of WRITING a
/// fake one: when the description changes in production the measurement
/// automatically measures that description, and nobody can forget to keep two
/// texts in sync.
struct DryTool(Arc<dyn Tool>);

impl Tool for DryTool {
    fn name(&self) -> &str {
        self.0.name()
    }
    fn description(&self) -> &str {
        self.0.description()
    }
    fn schema(&self) -> ArgSchema {
        self.0.schema()
    }
    fn taints_session(&self) -> bool {
        self.0.taints_session()
    }
    fn run<'a>(&'a self, _args: Value, _ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        // Fixed but NOT EMPTY: a tool returning "no results" pushes the model to
        // try another tool and would break the call sequence.
        boxed(async move {
            ToolOutcome::read_ok(
                "dry tool",
                "1. Sample result — the network is off in this case.",
            )
        })
    }
}

// ---------------------------------------------------------------------------
// The case shape
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Category {
    /// A tool must be called.
    Tool,
    /// NO tool must be called.
    Irrelevance,
    /// Several user messages; the history is preserved.
    MultiTurn,
}

impl Category {
    pub fn name(&self) -> &'static str {
        match self {
            Category::Tool => "tool",
            Category::Irrelevance => "irrelevance",
            Category::MultiTurn => "multi-turn",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Language {
    Turkish,
    English,
    Spanish,
    French,
    German,
    Russian,
    Chinese,
}

/// WHAT MAKES A LANGUAGE RECOGNISABLE, as data rather than as a `match` arm.
///
/// Two independent kinds of proof, and either is enough:
///
/// * `letters` — characters the OTHER supported languages do not write. For
///   Turkish that is `ç ğ ı İ ö ş ü`; for Russian it is the whole Cyrillic
///   block; for Chinese, any CJK ideograph. A single one of these settles the
///   question by itself.
/// * `words` — whole function words, compared for EQUALITY and never as
///   substrings. The substring version is what this file already had to remove
///   once: "için" contains "in", so every Turkish answer counted as English and
///   a gate that no input could fail was being reported as passing.
///
/// THE WORD LISTS MUST NOT OVERLAP, which is not obvious and is the thing that
/// breaks first when a language is added: Spanish and French both write "la",
/// German and English both write "in". A shared word makes the claim vacuous in
/// the same way the substring test did, so
/// `no_two_languages_claim_the_same_function_word` fails the build over it.
pub struct LanguageMarks {
    pub code: &'static str,
    pub letters: &'static str,
    pub words: &'static [&'static str],
}

impl Language {
    /// The code a benchmark file writes in its `language` field.
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|l| l.marks().code == code.to_lowercase())
    }

    pub const ALL: [Language; 7] = [
        Language::English,
        Language::Turkish,
        Language::Spanish,
        Language::French,
        Language::German,
        Language::Russian,
        Language::Chinese,
    ];

    pub fn marks(self) -> LanguageMarks {
        match self {
            // `İ` (dotted capital I) is in the list and plain `I` is NOT: the
            // two look alike in a font and are different characters, and
            // including the ASCII one would let any capitalised English
            // sentence pass as Turkish.
            Language::Turkish => LanguageMarks {
                code: "tr",
                // `ö ü` are NOT here even though Turkish writes them: German writes
                // them too, and a letter two languages share proves neither.
                // What is left is still enough — `ğ ı İ ş` occur in no other
                // language on this list.
                letters: "çÇğĞıİşŞ",
                words: &[
                    "ve", "bir", "bu", "icin", "ile", "var", "yok", "gun", "saat", "tarih",
                    "dosya", "not", "olarak", "kadar", "su", "da", "de", "sonra", "once", "kac",
                    "ne",
                ],
            },
            Language::English => LanguageMarks {
                code: "en",
                letters: "",
                words: &[
                    "the", "is", "and", "to", "in", "for", "it", "on", "of", "with", "a", "an",
                    "are", "you", "your", "was", "has", "have", "there", "that", "this", "days",
                    "day",
                ],
            },
            // Spanish and French share "la", "le", "en", "des"… so neither list
            // carries a word the other writes. `ñ` and `¿` do the heavy lifting.
            Language::Spanish => LanguageMarks {
                code: "es",
                letters: "ñÑ¿¡",
                words: &[
                    "el", "los", "una", "que", "por", "para", "con", "como", "pero", "esta",
                    "este", "son", "hay", "archivo", "fecha", "hora",
                ],
            },
            Language::French => LanguageMarks {
                code: "fr",
                // No `ç`: Turkish writes it too. The accents below do not occur
                // in any other supported language's proof set.
                letters: "àèùâêîôûëïœÀÈÙÂÊÎÔÛ",
                words: &[
                    "les", "une", "dans", "sur", "pour", "avec", "vous", "est", "sont", "cette",
                    "fichier", "heure", "jours", "aucun",
                ],
            },
            Language::German => LanguageMarks {
                code: "de",
                // `ö ü` dropped for the same reason they left the Turkish row.
                letters: "äÄß",
                words: &[
                    "der", "die", "das", "und", "nicht", "ist", "sind", "eine", "einen", "mit",
                    "auf", "datei", "uhr", "tage", "keine",
                ],
            },
            // A whole script is proof; the word list is a courtesy for an answer
            // that happens to be transliterated.
            Language::Russian => LanguageMarks {
                code: "ru",
                letters: "абвгдеёжзийклмнопрстуфхцчшщъыьэюяАБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯ",
                words: &[],
            },
            Language::Chinese => LanguageMarks {
                code: "zh",
                // The CJK Unified Ideographs block is checked by range in
                // `speaks`; a handful of the commonest characters are listed so
                // the table stays readable and testable.
                letters: "的是在有和文件时间日期没有",
                words: &[],
            },
        }
    }
}

/// A single user message and the tool expected from it.
#[derive(Debug, Clone, Serialize)]
pub struct SelectionStep {
    pub message: String,
    /// `None` = no tool must be called for this message.
    pub expected: Option<String>,
    pub evidence: Vec<String>,
    pub forbidden: Vec<String>,
    pub language: Option<Language>,
}

impl SelectionStep {
    pub fn new(message: &str, expected: Option<&str>) -> Self {
        Self {
            message: message.into(),
            expected: expected.map(Into::into),
            evidence: Vec::new(),
            forbidden: Vec::new(),
            language: None,
        }
    }

    pub fn with_evidence(mut self, ev: &[&str]) -> Self {
        self.evidence = ev.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn with_forbidden(mut self, fb: &[&str]) -> Self {
        self.forbidden = fb.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn with_language(mut self, lang: Language) -> Self {
        self.language = Some(lang);
        self
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SelectionCase {
    pub name: String,
    pub category: Category,
    pub steps: Vec<SelectionStep>,
}

impl SelectionCase {
    /// A single-message tool case.
    fn tool(name: &str, message: &str, expected: &str) -> Self {
        Self {
            name: name.into(),
            category: Category::Tool,
            steps: vec![SelectionStep::new(message, Some(expected))],
        }
    }

    fn tool_with_evidence(
        name: &str,
        message: &str,
        expected: &str,
        evidence: &[&str],
        lang: Language,
    ) -> Self {
        Self {
            name: name.into(),
            category: Category::Tool,
            steps: vec![
                SelectionStep::new(message, Some(expected))
                    .with_evidence(evidence)
                    .with_language(lang),
            ],
        }
    }

    /// A case where a tool must NOT be called.
    #[allow(dead_code)]
    fn chat(name: &str, message: &str) -> Self {
        Self {
            name: name.into(),
            category: Category::Irrelevance,
            steps: vec![SelectionStep::new(message, None)],
        }
    }

    fn chat_with_language(name: &str, message: &str, lang: Language) -> Self {
        Self {
            name: name.into(),
            category: Category::Irrelevance,
            steps: vec![SelectionStep::new(message, None).with_language(lang)],
        }
    }

    /// A multi-turn case: a sequence of `(message, expected tool)`.
    fn chain(name: &str, steps: &[(&str, &str)]) -> Self {
        Self {
            name: name.into(),
            category: Category::MultiTurn,
            steps: steps
                .iter()
                .map(|(m, e)| SelectionStep::new(m, Some(e)))
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// The case set
// ---------------------------------------------------------------------------

/// The tool selection case set.
///
/// AT LEAST TWO CASES FOR EVERY TOOL IN THE CATALOG, and the same intent in
/// different phrasings. Measuring with a single phrasing misleads: "what time is
/// it" may work while "what is the date today" does not, and a single-case set
/// reports that as "the time tool is fine". Phrasing variety is the only tool
/// that asks whether the description really covers the intent.
/// The TURKISH selection set — a SEPARATE list, exactly as the module doc
/// promises: mixing languages into one list would make the single hit-rate
/// number unreadable. The intents mirror the English set's core (arithmetic,
/// clock, dates in natural Turkish, documents, files, web, memory, smalltalk
/// that must select NOTHING), so the two reports are comparable side by side.
/// Run with `tacet eval --tool-selection --turkish`.
pub fn turkish_selection_cases() -> Vec<SelectionCase> {
    vec![
        // --- calculate ---
        SelectionCase::tool_with_evidence(
            "tr-hesap-carpma",
            "125 çarpı 8 kaç eder?",
            "calculate",
            &["1000"],
            Language::Turkish,
        ),
        SelectionCase::tool_with_evidence(
            "tr-hesap-yuzde",
            "480'in yüzde 18'i ne kadar?",
            "calculate",
            &["86.4"],
            Language::Turkish,
        ),
        SelectionCase::tool_with_evidence(
            "tr-hesap-toplama",
            "347 ile 268'i toplar mısın?",
            "calculate",
            &["615"],
            Language::Turkish,
        ),
        SelectionCase::tool_with_evidence(
            "tr-hesap-bolme",
            "144 bölü 12 kaçtır?",
            "calculate",
            &["12"],
            Language::Turkish,
        ),
        SelectionCase::tool_with_evidence(
            "tr-hesap-cikarma",
            "1000 eksi 375 kaç eder?",
            "calculate",
            &["625"],
            Language::Turkish,
        ),
        SelectionCase::tool_with_evidence(
            "tr-hesap-indirim",
            "500 liralık ürüne yüzde 25 indirim uygulanırsa kaç lira öderim?",
            "calculate",
            &["375"],
            Language::Turkish,
        ),
        SelectionCase::tool_with_evidence(
            "tr-hesap-karekok",
            "81'in karekökü nedir?",
            "calculate",
            &["9"],
            Language::Turkish,
        ),
        SelectionCase::tool_with_evidence(
            "tr-hesap-ortalama",
            "10, 20 ve 30'un ortalaması kaçtır?",
            "calculate",
            &["20"],
            Language::Turkish,
        ),
        SelectionCase::tool_with_evidence(
            "tr-hesap-us",
            "2 üzeri 8 kaçtır?",
            "calculate",
            &["256"],
            Language::Turkish,
        ),
        SelectionCase::tool_with_evidence(
            "tr-hesap-kDV",
            "1000 TL + %20 KDV ne kadar yapar?",
            "calculate",
            &["1200"],
            Language::Turkish,
        ),
        // --- time ---
        SelectionCase::tool("tr-saat", "Saat kaç şu an?", "time"),
        SelectionCase::tool("tr-tarih", "Bugün ayın kaçı?", "time"),
        SelectionCase::tool("tr-gun-farki", "Yılbaşına kaç gün kaldı?", "time"),
        SelectionCase::tool("tr-hafta-gunu", "Bugün günlerden ne?", "time"),
        SelectionCase::tool("tr-dogal-tarih", "Önümüzdeki salıya kaç gün var?", "time"),
        SelectionCase::tool(
            "tr-zaman-dakika",
            "Şu an saat ve dakika bilgisi verir misin?",
            "time",
        ),
        SelectionCase::tool("tr-zaman-ay", "Hangi aydayız?", "time"),
        SelectionCase::tool("tr-zaman-yil", "Hangi yıldayız?", "time"),
        SelectionCase::tool(
            "tr-zaman-tarih-farki",
            "15 Ağustos 2026 tarihine kaç gün var?",
            "time",
        ),
        SelectionCase::tool(
            "tr-zaman-gecimis",
            "2026 yılbaşından bugüne kaç gün geçti?",
            "time",
        ),
        // --- documents ---
        SelectionCase::tool(
            "tr-belge-olustur",
            "Alışveriş listemi bir excel tablosu yap",
            "create_document",
        ),
        SelectionCase::tool(
            "tr-belge-oku",
            "notlar.md dosyasında ne yazıyor, özetler misin?",
            "read_document",
        ),
        SelectionCase::tool(
            "tr-belge-duzenle",
            "Az önceki tabloya bir satır daha ekle",
            "edit_document",
        ),
        SelectionCase::tool(
            "tr-belge-markdown",
            "Toplantı kararlarını toplantı.md adıyla kaydet",
            "create_document",
        ),
        SelectionCase::tool(
            "tr-belge-ozet",
            "rapor.md belgesini oku ve özet çıkar",
            "read_document",
        ),
        SelectionCase::tool(
            "tr-belge-satir-sil",
            "notlar.md dosyasındaki 3. satırı sil",
            "edit_document",
        ),
        SelectionCase::tool(
            "tr-belge-baslik-ekle",
            "plan.md dosyasına yeni bir başlık ekler misin?",
            "edit_document",
        ),
        SelectionCase::tool(
            "tr-belge-icerik-oku",
            "icerik.txt dosyasının tüm metnini göster",
            "read_document",
        ),
        SelectionCase::tool(
            "tr-belge-yeni-excel",
            "Bütçe kalemi için bir bütçe.xlsx dosyası oluştur",
            "create_document",
        ),
        // --- files ---
        SelectionCase::tool(
            "tr-dosya-ara",
            "Bütçeyle ilgili notu hangi dosyaya yazmıştım?",
            "find_file",
        ),
        SelectionCase::tool(
            "tr-dosya-bul",
            "Klasörde 'rapor' içeren dosyaları bul",
            "find_file",
        ),
        SelectionCase::tool(
            "tr-dosya-arama-metin",
            "İçinde 'Lentils' geçen dosyayı bulur musun?",
            "find_file",
        ),
        SelectionCase::tool(
            "tr-dosya-nerede",
            "proje_plani.pdf nerede duruyor?",
            "find_file",
        ),
        SelectionCase::tool(
            "tr-dosya-listele",
            "Dizin altındaki markdown dosyalarını arat",
            "find_file",
        ),
        // --- code ---
        SelectionCase::tool(
            "tr-kod-calistir",
            "1'den 100'e kadar asal sayıları listeler misin?",
            "run_code",
        ),
        SelectionCase::tool(
            "tr-kod-dosya",
            "Bana fibonacci hesaplayan bir python betiği yaz ve kaydet",
            "write_code",
        ),
        SelectionCase::tool(
            "tr-kod-hesapla",
            "Python ile 1'den 50'ye kadar olan sayıların toplamını çalıştır",
            "run_code",
        ),
        SelectionCase::tool(
            "tr-kod-kaydet",
            "Sıcaklık dönüşümü yapan betiği donusturucu.py adıyla kaydet",
            "write_code",
        ),
        SelectionCase::tool(
            "tr-kod-faktoryel",
            "Python ile 10 faktöriyel değerini hesaplayıp ekrana yazdır",
            "run_code",
        ),
        // --- web ---
        SelectionCase::tool(
            "tr-hava",
            "İstanbul'da yarın hava nasıl olacak?",
            "web_search",
        ),
        SelectionCase::tool("tr-haber", "Dolar kuru şu an ne durumda?", "web_search"),
        SelectionCase::tool(
            "tr-web-site-oku",
            "https://example.com sayfasında ne anlatılıyor?",
            "web_fetch",
        ),
        SelectionCase::tool(
            "tr-web-arama",
            "Türkiye'nin 2026 yılı enflasyon oranı haberleri ne durumda?",
            "web_search",
        ),
        SelectionCase::tool(
            "tr-web-link",
            "https://news.ycombinator.com adresindeki başlıkları al",
            "web_fetch",
        ),
        SelectionCase::tool(
            "tr-web-guncel",
            "Bugünün son dakika haberlerini internette ara",
            "web_search",
        ),
        // --- memory ---
        SelectionCase::tool(
            "tr-hatirla",
            "Kardeşimin doğum günü 3 mayıs, bunu unutma",
            "remember",
        ),
        SelectionCase::tool("tr-unut", "Kahve sevdiğimi unut artık", "remember"),
        SelectionCase::tool(
            "tr-hafiza-oku",
            "Benim hakkımda aklında tuttuğun notları listele",
            "remember",
        ),
        SelectionCase::tool(
            "tr-hatirla-araba",
            "Arabamı 2. kat B blok park yerine koyduğumu kaydet",
            "remember",
        ),
        SelectionCase::tool(
            "tr-hatirla-soru",
            "Kardeşimin doğum gününü kaydetmiştin, hatırla ne zamandı?",
            "remember",
        ),
        // --- git ---
        SelectionCase::tool(
            "tr-git-durum",
            "Git reposunda hangi dosyalar değişti?",
            "git",
        ),
        SelectionCase::tool(
            "tr-git-commit",
            "Yapılan git değişiklikleri için commit mesajı öner",
            "git",
        ),
        // --- irrelevance: NOTHING must be selected ---
        SelectionCase::chat_with_language("tr-selam", "Selam, nasılsın?", Language::Turkish),
        SelectionCase::chat_with_language(
            "tr-tesekkur",
            "Çok teşekkürler, harikaydı!",
            Language::Turkish,
        ),
        SelectionCase::chat_with_language(
            "tr-sohbet",
            "Bugün biraz yorgunum ya",
            Language::Turkish,
        ),
        SelectionCase::chat_with_language(
            "tr-fikir",
            "Sence sabah sporu mu akşam sporu mu daha iyi?",
            Language::Turkish,
        ),
        SelectionCase::chat_with_language(
            "tr-kimsin",
            "Sen kimsin, ne iş yaparsın?",
            Language::Turkish,
        ),
        SelectionCase::chat_with_language(
            "tr-gizlilik",
            "Benim verilerimi başkalarıyla paylaşıyor musun?",
            Language::Turkish,
        ),
        SelectionCase::chat_with_language(
            "tr-tavsiye",
            "Bana güzel bir kitap önerir misin?",
            Language::Turkish,
        ),
        SelectionCase::chat_with_language(
            "tr-gorusuruz",
            "İyi akşamlar, sonra görüşürüz!",
            Language::Turkish,
        ),
        SelectionCase::chat_with_language(
            "tr-tebrik",
            "Tebrik ederim harika bir iş çıkardın",
            Language::Turkish,
        ),
        // --- Confusable Pairs ---
        SelectionCase::tool(
            "tr-pair-tarih-farki",
            "25 Aralık tarihine kaç gün var?",
            "time",
        ),
        SelectionCase::tool_with_evidence(
            "tr-pair-matematik",
            "25 ile 18'i topla",
            "calculate",
            &["43"],
            Language::Turkish,
        ),
        SelectionCase::tool(
            "tr-pair-dosya-ara",
            "Bütçe raporu hangi klasörde?",
            "find_file",
        ),
        SelectionCase::tool(
            "tr-pair-dosya-oku",
            "Bütçe raporunun içeriğinde ne var?",
            "read_document",
        ),
        // --- archive / checksum ---
        //
        // THE TURKISH HALF IS NOT A TRANSLATION EXERCISE. Both sentences carry
        // the word "dosya", which is a Files trigger, so each of them is also a
        // test that the new profile beats `find_file` on its own turf — which is
        // the failure mode a new profile actually has.
        SelectionCase::tool("tr-zip-icerik", "backup.zip içinde ne var?", "archive"),
        SelectionCase::tool("tr-zip-ac", "Bu sıkıştırılmış arşivi aç", "archive"),
        SelectionCase::tool(
            "tr-ozet-degeri",
            "installer.dmg dosyasının sha256 özet değeri nedir?",
            "checksum",
        ),
        SelectionCase::tool(
            "tr-ayni-dosya",
            "Bu iki dosya aynı mı, kontrol eder misin?",
            "checksum",
        ),
    ]
}

/// THE SUITE `--tool-selection` ACTUALLY RUNS: both languages, in one list.
///
/// WHY IT IS A FUNCTION AND NOT TWO CALLS AT THE CALL SITE. `baselines.rs` exists
/// to fail the build when a checked-in baseline stops pairing with the suite it
/// came from, and it did not: it compares against `selection_cases()` and
/// `turkish_selection_cases()` separately, so the moment the command started
/// running BOTH, a baseline matching either half still passed the guard while
/// pairing with no real run at all. The command and the guard now read the same
/// function, which is the only arrangement in which they cannot drift.
pub fn selection_suite() -> Vec<SelectionCase> {
    let mut all = selection_cases();
    all.extend(turkish_selection_cases());
    all
}

pub fn selection_cases() -> Vec<SelectionCase> {
    vec![
        // --- calendar ---
        SelectionCase::tool(
            "calendar-day",
            "What is on my calendar tomorrow?",
            "calendar",
        ),
        SelectionCase::tool(
            "calendar-remind",
            "Remind me to call the dentist tomorrow at 9",
            "calendar",
        ),
        SelectionCase::tool(
            "calendar-schedule",
            "Schedule a meeting with Alice for Friday at 3pm",
            "calendar",
        ),
        SelectionCase::tool(
            "calendar-events",
            "List all my calendar events for next week",
            "calendar",
        ),
        SelectionCase::tool(
            "calendar-next",
            "What is my next upcoming appointment?",
            "calendar",
        ),
        SelectionCase::tool(
            "calendar-clear",
            "Clear my schedule for tomorrow morning",
            "calendar",
        ),
        // --- calculate ---
        SelectionCase::tool_with_evidence(
            "calculate-multiply",
            "What is 125 times 8?",
            "calculate",
            &["1000"],
            Language::English,
        ),
        SelectionCase::tool_with_evidence(
            "calculate-add",
            "Could you add 347 and 268?",
            "calculate",
            &["615"],
            Language::English,
        ),
        SelectionCase::tool_with_evidence(
            "calculate-percent",
            "How much is 250 lira with a 20 percent discount?",
            "calculate",
            &["200"],
            Language::English,
        ),
        SelectionCase::tool_with_evidence(
            "calculate-divide",
            "What is 144 divided by 12?",
            "calculate",
            &["12"],
            Language::English,
        ),
        SelectionCase::tool_with_evidence(
            "calculate-subtract",
            "What is 1000 minus 375?",
            "calculate",
            &["625"],
            Language::English,
        ),
        SelectionCase::tool_with_evidence(
            "calculate-power",
            "What is 2 to the power of 10?",
            "calculate",
            &["1024"],
            Language::English,
        ),
        SelectionCase::tool_with_evidence(
            "calculate-sqrt",
            "What is the square root of 144?",
            "calculate",
            &["12"],
            Language::English,
        ),
        SelectionCase::tool_with_evidence(
            "calculate-expression",
            "Calculate (50 + 50) * 5 / 2",
            "calculate",
            &["250"],
            Language::English,
        ),
        SelectionCase::tool_with_evidence(
            "calculate-vat",
            "Calculate $500 with 10% tax added",
            "calculate",
            &["550"],
            Language::English,
        ),
        SelectionCase::tool_with_evidence(
            "calculate-average",
            "What is the average of 15, 25, and 35?",
            "calculate",
            &["25"],
            Language::English,
        ),
        SelectionCase::tool_with_evidence(
            "calculate-discount",
            "Calculate 15% off $80",
            "calculate",
            &["68"],
            Language::English,
        ),
        // --- time ---
        SelectionCase::tool("time-clock", "What time is it?", "time"),
        SelectionCase::tool(
            "time-day-of-month",
            "What day of the month is it today?",
            "time",
        ),
        SelectionCase::tool("time-todays-date", "What is today's date?", "time"),
        SelectionCase::tool(
            "time-diff",
            "How many days are left until new year?",
            "time",
        ),
        SelectionCase::tool("time-weekday", "What day of the week is it today?", "time"),
        SelectionCase::tool(
            "time-current-month",
            "Which month are we in currently?",
            "time",
        ),
        SelectionCase::tool("time-current-year", "What is the current year?", "time"),
        SelectionCase::tool(
            "time-days-to-christmas",
            "How many days until Christmas?",
            "time",
        ),
        SelectionCase::tool(
            "time-days-to-date",
            "How many days until 15 October 2026?",
            "time",
        ),
        SelectionCase::tool(
            "time-days-since",
            "How many days have passed since 1 January 2026?",
            "time",
        ),
        SelectionCase::tool("time-utc", "What is the current UTC time?", "time"),
        // --- read_document ---
        SelectionCase::tool(
            "read_document-content",
            "What does the file report.md say?",
            "read_document",
        ),
        SelectionCase::tool(
            "read_document-summary",
            "Could you summarize the file budget-2026.md?",
            "read_document",
        ),
        SelectionCase::tool(
            "read_document-full",
            "Show me the entire text of notes.txt",
            "read_document",
        ),
        SelectionCase::tool(
            "read_document-table",
            "Read the table inside report.md",
            "read_document",
        ),
        SelectionCase::tool(
            "read_document-preview",
            "Give me a preview of readme.md",
            "read_document",
        ),
        SelectionCase::tool(
            "read_document-log",
            "Read the latest entries from app.log",
            "read_document",
        ),
        // --- create_document ---
        SelectionCase::tool(
            "create_document-excel",
            "Turn the weekly meal list into an excel file.",
            "create_document",
        ),
        SelectionCase::tool(
            "create_document-markdown",
            "Create a short markdown file for me for the meeting notes.",
            "create_document",
        ),
        SelectionCase::tool(
            "create_document-report",
            "Create a new document called report.md with summary content",
            "create_document",
        ),
        SelectionCase::tool(
            "create_document-csv",
            "Export the product list into a spreadsheet file",
            "create_document",
        ),
        SelectionCase::tool(
            "create_document-notes",
            "Save a new note file named ideas.md",
            "create_document",
        ),
        SelectionCase::tool(
            "create_document-todo",
            "Make a new markdown document for my todo list",
            "create_document",
        ),
        // --- edit_document ---
        SelectionCase::tool(
            "edit_document-row",
            "Add the row 'Thursday | Chickpeas' to the file report.md.",
            "edit_document",
        ),
        SelectionCase::tool(
            "edit_document-title",
            "Change the title of the file budget-2026.md to 'New Budget'.",
            "edit_document",
        ),
        SelectionCase::tool(
            "edit_document-append",
            "Append a new section to notes.md",
            "edit_document",
        ),
        SelectionCase::tool(
            "edit_document-update-line",
            "Replace line 5 in report.md with updated figures",
            "edit_document",
        ),
        SelectionCase::tool(
            "edit_document-header",
            "Insert a header line into document.md",
            "edit_document",
        ),
        SelectionCase::tool(
            "edit_document-modify",
            "Modify the Wednesday row in the meal table inside report.md",
            "edit_document",
        ),
        // --- find_file ---
        SelectionCase::tool(
            "find_file-name",
            "Find the file about the budget.",
            "find_file",
        ),
        SelectionCase::tool(
            "find_file-content",
            "Which of my files mentions 'Lentils'?",
            "find_file",
        ),
        SelectionCase::tool(
            "find_file-where",
            "Where is the file architecture.md located?",
            "find_file",
        ),
        SelectionCase::tool(
            "find_file-pattern",
            "Search for files with .log extension",
            "find_file",
        ),
        SelectionCase::tool(
            "find_file-keyword",
            "Locate files that contain the term 'Qwen3'",
            "find_file",
        ),
        SelectionCase::tool(
            "find_file-list",
            "Search my workspace for project files",
            "find_file",
        ),
        // --- web_search ---
        SelectionCase::tool(
            "web_search-weather",
            "What is the weather like in Istanbul?",
            "web_search",
        ),
        SelectionCase::tool(
            "web_search-current",
            "How much is the dollar today?",
            "web_search",
        ),
        SelectionCase::tool(
            "web_search-news",
            "What are the latest tech news headlines today?",
            "web_search",
        ),
        SelectionCase::tool(
            "web_search-stock",
            "What is the current stock price of Apple?",
            "web_search",
        ),
        SelectionCase::tool(
            "web_search-flight",
            "Find flight schedules from London to Paris",
            "web_search",
        ),
        SelectionCase::tool(
            "web_search-score",
            "What was the score of the match in the news yesterday?",
            "web_search",
        ),
        SelectionCase::tool(
            "web_search-inflation",
            "What is the current news on the inflation rate in 2026?",
            "web_search",
        ),
        // --- web_fetch ---
        SelectionCase::tool(
            "web_fetch-page",
            "Read the content of the page https://example.com/blog.",
            "web_fetch",
        ),
        SelectionCase::tool(
            "web_fetch-address",
            "Get me the detail of the article at this address: https://example.com/article",
            "web_fetch",
        ),
        SelectionCase::tool(
            "web_fetch-url-summary",
            "Summarize the website at https://rust-lang.org",
            "web_fetch",
        ),
        SelectionCase::tool(
            "web_fetch-extract-link",
            "Fetch the content from https://news.ycombinator.com",
            "web_fetch",
        ),
        // --- remember ---
        SelectionCase::tool(
            "remember-save",
            "Remember this: I drink my coffee without milk.",
            "remember",
        ),
        SelectionCase::tool(
            "remember-list",
            "List the notes you keep about me.",
            "remember",
        ),
        SelectionCase::tool(
            "remember-birthday",
            "Remember that my sister's birthday is May 3rd",
            "remember",
        ),
        SelectionCase::tool(
            "remember-forget",
            "Remember to forget my old home address",
            "remember",
        ),
        SelectionCase::tool(
            "remember-query",
            "What note did I save about my sister's birthday?",
            "remember",
        ),
        SelectionCase::tool(
            "remember-car-park",
            "Remember where I parked my car",
            "remember",
        ),
        // --- run_code ---
        SelectionCase::tool(
            "run_code-primes",
            "List the prime numbers from 1 to 30 in python.",
            "run_code",
        ),
        SelectionCase::tool(
            "run_code-fibonacci",
            "Produce the first 15 terms of the Fibonacci sequence.",
            "run_code",
        ),
        SelectionCase::tool(
            "run_code-sum",
            "Calculate the sum of squares from 1 to 100 using python",
            "run_code",
        ),
        SelectionCase::tool(
            "run_code-factorial",
            "Compute 10 factorial with a python script",
            "run_code",
        ),
        // --- write_code ---
        SelectionCase::tool(
            "write_code-script",
            "Write me a python script that finds prime numbers, and save it as a file.",
            "write_code",
        ),
        SelectionCase::tool(
            "write_code-converter",
            "Write a script that converts temperature data from Celsius to Fahrenheit and put it in my folder.",
            "write_code",
        ),
        SelectionCase::tool(
            "write_code-web-scraper",
            "Write a python web scraper script and save it to scraper.py",
            "write_code",
        ),
        SelectionCase::tool(
            "write_code-utility",
            "Create a python script utility.py that renames files in batch",
            "write_code",
        ),
        // --- git ---
        SelectionCase::tool(
            "git-status",
            "Which files have I changed in this git repository?",
            "git",
        ),
        SelectionCase::tool(
            "git-commit-message",
            "Summarize my git changes and write me a commit message.",
            "git",
        ),
        SelectionCase::tool(
            "git-diff",
            "Show me the git diff for uncommitted changes",
            "git",
        ),
        SelectionCase::tool("git-branch", "Which git branch am I currently on?", "git"),
        // --- IRRELEVANCE (No tool must be called) ---
        SelectionCase::chat_with_language("chat-greeting", "Hello", Language::English),
        SelectionCase::chat_with_language("chat-thanks", "Thank you very much.", Language::English),
        SelectionCase::chat_with_language("chat-who-are-you", "Who are you?", Language::English),
        SelectionCase::chat_with_language(
            "chat-how-are-you",
            "How are you today?",
            Language::English,
        ),
        SelectionCase::chat_with_language(
            "chat-privacy",
            "Are you sending my data to the cloud?",
            Language::English,
        ),
        SelectionCase::chat_with_language(
            "chat-mood",
            "I'm a bit tired today, feeling low.",
            Language::English,
        ),
        SelectionCase::chat_with_language(
            "chat-recommendation",
            "Can you recommend a good movie?",
            Language::English,
        ),
        SelectionCase::chat_with_language(
            "chat-general-knowledge-paris",
            "What is the capital of France?",
            Language::English,
        ),
        SelectionCase::chat_with_language(
            "chat-general-knowledge-planet",
            "What is the largest planet in our solar system?",
            Language::English,
        ),
        SelectionCase::chat_with_language(
            "chat-farewell-see-you",
            "Goodbye, see you!",
            Language::English,
        ),
        SelectionCase::chat_with_language(
            "chat-appreciation",
            "You did a fantastic job, thanks!",
            Language::English,
        ),
        SelectionCase::chat_with_language(
            "chat-continuation-explain",
            "Tell me more about your thoughts.",
            Language::English,
        ),
        SelectionCase::chat_with_language(
            "chat-opinion-sports",
            "Which is better, morning or evening workout?",
            Language::English,
        ),
        SelectionCase::chat_with_language(
            "chat-bored",
            "I'm feeling bored, tell me a joke.",
            Language::English,
        ),
        SelectionCase::chat_with_language(
            "chat-weather-general",
            "I love nature and fresh air.",
            Language::English,
        ),
        // --- Confusable Pairs ---
        SelectionCase::tool(
            "pair-time-diff",
            "How many days until 2 December 2026?",
            "time",
        ),
        SelectionCase::tool_with_evidence(
            "pair-calc-add",
            "Add 25 and 18",
            "calculate",
            &["43"],
            Language::English,
        ),
        SelectionCase::tool("pair-find-file", "Where is the budget file?", "find_file"),
        SelectionCase::tool(
            "pair-read-doc",
            "What does the budget file say?",
            "read_document",
        ),
        SelectionCase::tool(
            "pair-web-search",
            "Who won the 2026 election?",
            "web_search",
        ),
        SelectionCase::tool(
            "pair-web-fetch",
            "Summarize https://example.com/election-results",
            "web_fetch",
        ),
        // --- archive ---
        //
        // FOUR PHRASINGS, NOT TWO, AND THE LAST ONE IS THE POINT. "List the
        // files in ..." uses the two words the Files profile is built on, so it
        // is the case that asks whether `archive` can win a sentence that also
        // reads as a file question — the thing a new profile is most likely to
        // get wrong.
        SelectionCase::tool("archive-list", "What is inside backup.zip?", "archive"),
        SelectionCase::tool(
            "archive-unzip",
            "Unzip invoices.zip into a folder",
            "archive",
        ),
        SelectionCase::tool("archive-unpack", "Unpack the archive photos.zip", "archive"),
        SelectionCase::tool(
            "archive-contents",
            "List the files in the compressed release.zip",
            "archive",
        ),
        // --- checksum ---
        //
        // The last two deliberately DO NOT say "checksum": a user who wants this
        // tool often does not know the word, and a case set written only in the
        // vocabulary of the tool measures the vocabulary, not the intent.
        SelectionCase::tool(
            "checksum-digest",
            "What is the sha256 of installer.dmg?",
            "checksum",
        ),
        SelectionCase::tool(
            "checksum-verify",
            "Check this download against the checksum they published",
            "checksum",
        ),
        SelectionCase::tool(
            "checksum-fingerprint",
            "Give me the fingerprint of setup.exe",
            "checksum",
        ),
        SelectionCase::tool(
            "checksum-compare",
            "Are these two files byte for byte identical?",
            "checksum",
        ),
        // --- MULTI-TURN ---
        SelectionCase::chain(
            "chain-document",
            &[
                (
                    "Turn the weekly meal list into an excel file.",
                    "create_document",
                ),
                ("Show it as a table.", "read_document"),
                ("Change Tuesday from Rice to Beans.", "edit_document"),
            ],
        ),
        SelectionCase::chain(
            "chain-calc-document",
            &[
                ("What is 125 times 8?", "calculate"),
                ("Write this result into a markdown file.", "create_document"),
            ],
        ),
        SelectionCase::chain(
            "chain-read-edit",
            &[
                ("What does the file report.md say?", "read_document"),
                ("Add a Thursday row with Chickpeas.", "edit_document"),
            ],
        ),
        SelectionCase::chain(
            "chain-find-read",
            &[
                ("Find the file about budget.", "find_file"),
                ("Read its contents.", "read_document"),
            ],
        ),
        SelectionCase::chain(
            "chain-code-write",
            &[
                ("Run python code to find prime numbers.", "run_code"),
                ("Save that script to primes.py.", "write_code"),
            ],
        ),
    ]
}

// ---------------------------------------------------------------------------
// The outcome shape
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct StepOutcome {
    pub message: String,
    pub expected: Option<String>,
    /// The tools called in this step, in order.
    pub called: Vec<String>,
    pub passed: bool,
    pub answer: String,
    pub answer_passed: bool,
    /// DOES THIS STEP CLAIM ANYTHING ABOUT THE ANSWER — evidence that must
    /// appear, a phrase that must not, or a language it must be written in.
    ///
    /// WHY THE REPORT NEEDS IT: `answer_total` counted every step, and most
    /// steps make no claim at all, so `check_answer_quality` returned `true`
    /// for them by having nothing to check. Those free passes went into the
    /// denominator AND the numerator of a line printed as ANSWER QUALITY, which
    /// therefore moved when the number of claimless cases changed and stood
    /// still when a real claim broke. A rate over steps that assert nothing is
    /// not a rate.
    pub claims: bool,
    /// WHY THE TURN STOPPED. See `Ending`: a step that could not be measured is
    /// not scored as a pass on either axis.
    pub ended: Ending,
}

/// WHY A TURN STOPPED, and therefore whether it can be scored at all.
///
/// The loop used to leave this implicit and the report paid for it twice.
/// `passed` was decided from `called` alone, so a turn that ran out of passes
/// while still calling tools — never producing a word for the user — scored as a
/// HIT, and a turn whose engine died scored as a PASS on the irrelevance axis,
/// because a dead engine calls nothing. Both are in the shipped baseline: three
/// steps passed with `answer == ""` after four tool calls, and the shell exits
/// non-zero on exactly that outcome (`chat.rs`, `settled`), while this file
/// claimed parity with the shell twice.
///
/// A turn that could not be measured is not a pass and not a failure. It is
/// counted separately and named, so a run in which generation broke cannot read
/// as the safety property holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Ending {
    /// The model produced text for the user. The only ending that can be scored.
    Answered,
    /// Every pass of the loop was spent calling tools and none produced an
    /// answer. The shell calls this a failed run.
    OutOfTurns,
    /// The engine returned an error.
    EngineError,
    /// Generation stopped on the token cap rather than on the model's own stop.
    CutOff,
    /// The environment could not be built — a host problem, not a model one.
    HostFailed,
}

impl Ending {
    /// Can a verdict be drawn from a turn that ended this way.
    pub fn is_measurable(self) -> bool {
        matches!(self, Ending::Answered)
    }

    /// For the table and the report.
    pub fn name(self) -> &'static str {
        match self {
            Ending::Answered => "answered",
            Ending::OutOfTurns => "out of turns",
            Ending::EngineError => "engine error",
            Ending::CutOff => "cut off",
            Ending::HostFailed => "host failed",
        }
    }
}

/// Does the step's claim about the ANSWER hold.
///
/// TWO CORRECTIONS, BOTH MEASURED ON THE SHIPPED BASELINE.
///
/// **`evidence` is checked against the answer, not against a pool that also
/// holds the tool's output.** It used to search `answer + every tool result`,
/// so a correctly-called tool satisfied the claim WHATEVER THE MODEL SAID —
/// `tr-hesap-ortalama` carries evidence `["20"]`, the model answered `"30"`, and
/// the step reads `answer_passed: true` because the tool's own result contained
/// the 20. An axis printed as ANSWER QUALITY was reporting tool output. Four
/// other places in this repository already document the answer-only rule; this
/// is the one that did it.
///
/// **`forbidden` is compared against the tools that were CALLED.** `bench.rs`
/// documents it as "tools that must NOT be called" and `bench check` reports the
/// entries to their author as tool names, while the only reader was a substring
/// search over text — 1505 such assertions across the benchmark corpus,
/// every value a tool name, none of them doing anything. It also fired the wrong
/// way: an answer that merely wrote the word `web_search` failed a step that had
/// never called it.
///
/// The verdict stays on the ANSWER axis rather than on `passed`. Moving it would
/// change `tool_total` and `step_passed` on every benchmark file at once and
/// make every published table incomparable with the next run; the axis this
/// belongs to is "did the model do something it was told not to", which is a
/// quality claim.
pub fn check_answer_quality(
    step: &SelectionStep,
    answer: &str,
    called: &[String],
    tool_outcomes: &[String],
) -> bool {
    let _ = tool_outcomes;
    let answer_lower = answer.to_lowercase();

    for ev in &step.evidence {
        if !answer_lower.contains(&ev.to_lowercase()) {
            return false;
        }
    }

    for fb in &step.forbidden {
        if called.iter().any(|c| c.eq_ignore_ascii_case(fb)) {
            return false;
        }
    }

    if let Some(lang) = step.language
        && !speaks(lang, answer)
    {
        return false;
    }

    true
}

/// Does this answer look like it is written in `lang`.
///
/// WHAT WAS WRONG WITH THE OLD CHECK, and it is worth stating plainly because
/// the number it produced was printed under the heading ANSWER QUALITY and read
/// as if it meant something: **both language gates passed everything.** They
/// tested `answer.to_lowercase().contains(marker)` over markers as short as two
/// letters, with no word boundary. The Turkish marker list carried "ve", "bu"
/// and "bir"; the English words "have", "about" and "birthday" contain them, so
/// an answer written entirely in English satisfied the Turkish gate. It ran the
/// other way too: the English list carried "in", "it" and "to", and the Turkish
/// "için" contains "in". A gate that no input can fail is not a gate, and the
/// Turkish suite had been reporting one for every case it measured.
///
/// WHAT REPLACES IT is two claims that a wrong-language answer cannot satisfy:
///
/// 1. WHOLE WORDS. The text is split into words and the function words are
///    compared for EQUALITY. "have" is not "ve".
/// 2. A LETTER THE OTHER LANGUAGE DOES NOT HAVE. `ç ğ ı ö ş ü` occur in Turkish
///    and in no English word, so one of them is proof on its own — this is the
///    half of the old list that was doing real work, kept.
///
/// A TEXT WITH NO WORDS IN IT IS NOT JUDGED. "1000." is not English and not
/// Turkish; the old code failed it, which counted a correct short answer as a
/// language defect. There is nothing there to read, so there is nothing to
/// claim.
/// The words of an answer, for the language check.
///
/// APOSTROPHES DO NOT SPLIT A WORD. Turkish attaches its case suffixes with one
/// — `480'in`, `81'in`, `625'tir` — and splitting there produced the standalone
/// token `in`, which is on the ENGLISH list. So a Turkish sentence handed
/// evidence to English while giving none to Turkish.
fn answer_words(answer: &str) -> Vec<String> {
    answer
        .split(|c: char| !c.is_alphanumeric() && c != '\'' && c != '\u{2019}')
        .map(|w| w.trim_matches(['\'', '\u{2019}']))
        .filter(|w| w.chars().any(char::is_alphabetic))
        .map(str::to_lowercase)
        .collect()
}

/// How much this answer looks like `lang`, in arbitrary units.
///
/// A proof letter is worth more than a function word because it is harder to
/// produce by accident; the absolute values do not matter, only the comparison
/// in `speaks`.
fn language_evidence(lang: Language, answer: &str, words: &[String]) -> usize {
    let marks = lang.marks();
    let mut score = 3 * answer
        .chars()
        .filter(|c| marks.letters.contains(*c))
        .count();
    // Chinese by RANGE rather than by list: the block has tens of thousands of
    // characters and an answer is free to use one that is not in the sample.
    if lang == Language::Chinese
        && answer
            .chars()
            .any(|c| matches!(c, '\u{4e00}'..='\u{9fff}' | '\u{3400}'..='\u{4dbf}'))
    {
        score += 3;
    }
    // The letters two languages share are evidence for BOTH, which is why they
    // were dropped from the proof sets — but dropping them left Turkish with no
    // evidence at all in `81'in karekökü 9'dur.` They come back here, weighted
    // like a word, and the competition below decides between the claimants.
    if matches!(lang, Language::Turkish | Language::German) {
        score += answer.chars().filter(|c| "öÖüÜ".contains(*c)).count();
    }
    score
        + words
            .iter()
            .filter(|w| marks.words.contains(&w.as_str()))
            .count()
}

/// Does this answer look like it is written in `lang`.
///
/// IT PROVES THE NEGATIVE, NOT THE POSITIVE — and it used to do the opposite.
///
/// The old rule demanded evidence FOR the asked language and failed the answer
/// when it found none. Measured on the shipped baseline, that failed four
/// answers that were written in the language asked for:
///
/// ```text
/// tr-hesap-yuzde    "480'in yüzde 18'i 86.4'tür."
/// tr-hesap-cikarma  "1000 eksi 375, yani 1000 - 375 = 625'tir."
/// tr-hesap-karekok  "81'in karekökü 9'dur."
/// chat-bored        "Why don't scientists trust atoms? …"   (English, none of the 23 words)
/// ```
///
/// Turkish's proof letters exclude `ö ü` because German writes them too, and
/// none of the three carries `ç ğ ı İ ş` or a listed function word. English has
/// no proof letters at all, so a fluent English joke with none of its 23 words
/// fails its own gate. That is four of the nine answer-quality failures being
/// the instrument rather than the model.
///
/// The rule now: score every supported language, and accept unless ANOTHER one
/// scores strictly higher. When nothing scores — a short sentence in no
/// language's evidence — it ABSTAINS, because "I cannot tell" is not "wrong".
/// `tr-tesekkur`, a genuinely English reply where Turkish was asked, still
/// fails: English scores and Turkish does not.
fn speaks(lang: Language, answer: &str) -> bool {
    let words = answer_words(answer);
    if words.is_empty() {
        return true;
    }
    let mine = language_evidence(lang, answer, &words);
    let best = Language::ALL
        .iter()
        .map(|l| language_evidence(*l, answer, &words))
        .max()
        .unwrap_or(0);
    // Nothing to go on: abstain rather than fail.
    if best == 0 {
        return true;
    }
    mine >= best
}

#[derive(Debug, Clone, Serialize)]
pub struct SelectionOutcome {
    pub name: String,
    pub category: Category,
    pub passed: bool,
    pub steps: Vec<StepOutcome>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SelectionReport {
    pub engine: String,
    /// WHAT produced these numbers — model file, quantization, device.
    pub identity: tacet_engine::EngineIdentity,
    pub wall_ms: u128,
    pub catalog: Vec<String>,
    pub cases: Vec<SelectionOutcome>,
    pub tool_passed: usize,
    pub tool_total: usize,
    pub irrelevance_passed: usize,
    pub irrelevance_total: usize,
    pub step_passed: usize,
    pub step_total: usize,
    pub answer_passed: usize,
    pub answer_total: usize,
}

impl SelectionReport {
    fn new(
        identity: tacet_engine::EngineIdentity,
        wall_ms: u128,
        catalog: Vec<String>,
        cases: Vec<SelectionOutcome>,
    ) -> Self {
        let mut r = Self {
            engine: identity.engine.clone(),
            identity,
            wall_ms,
            catalog,
            tool_passed: 0,
            tool_total: 0,
            irrelevance_passed: 0,
            irrelevance_total: 0,
            step_passed: 0,
            step_total: 0,
            answer_passed: 0,
            answer_total: 0,
            cases,
        };
        for c in &r.cases {
            match c.category {
                Category::Irrelevance => {
                    r.irrelevance_total += 1;
                    r.irrelevance_passed += c.passed as usize;
                }
                _ => {
                    r.tool_total += 1;
                    r.tool_passed += c.passed as usize;
                }
            }
            for s in &c.steps {
                r.step_total += 1;
                r.step_passed += s.passed as usize;
                // ONLY THE STEPS THAT CLAIM SOMETHING. See `StepOutcome::claims`
                // for what the old denominator was counting.
                if s.claims {
                    r.answer_total += 1;
                    r.answer_passed += s.answer_passed as usize;
                }
            }
        }
        r
    }

    pub fn tool_rate(&self) -> f64 {
        ratio(self.tool_passed, self.tool_total)
    }

    pub fn irrelevance_rate(&self) -> f64 {
        ratio(self.irrelevance_passed, self.irrelevance_total)
    }

    pub fn answer_rate(&self) -> f64 {
        ratio(self.answer_passed, self.answer_total)
    }

    pub fn table(&self) -> String {
        let width = self
            .cases
            .iter()
            .map(|c| c.name.chars().count())
            .max()
            .unwrap_or(4)
            .max(4);
        let mut s = String::new();
        s.push_str(&format!(
            "engine: {}  (tool selection set)\n",
            self.identity.line()
        ));
        s.push_str(&format!(
            "catalog: {} tools · {} ms\n\n",
            self.catalog.len(),
            self.wall_ms
        ));
        s.push_str(&format!(
            "{:<width$}  {:<6}  {}\n",
            "CASE", "STATE", "EXPECTED -> CALLED"
        ));
        s.push_str(&format!("{}\n", "-".repeat(width + 50)));
        for c in &self.cases {
            let state = if c.passed { "pass" } else { "FAIL" };
            let detail: Vec<String> = c
                .steps
                .iter()
                .map(|s| {
                    let e = s.expected.clone().unwrap_or_else(|| "-".into());
                    let called = if s.called.is_empty() {
                        "-".to_string()
                    } else {
                        s.called.join("+")
                    };
                    format!("{e}->{called}")
                })
                .collect();
            s.push_str(&format!(
                "{:<width$}  {state:<6}  {}\n",
                c.name,
                detail.join(" | ")
            ));
        }
        s.push_str(&format!(
            "\nTOOL HIT RATE   {}/{}  ({:.1}%)\n",
            self.tool_passed,
            self.tool_total,
            self.tool_rate() * 100.0
        ));
        s.push_str(&format!(
            "IRRELEVANCE     {}/{}  ({:.1}%)   <- MUST NOT drop\n",
            self.irrelevance_passed,
            self.irrelevance_total,
            self.irrelevance_rate() * 100.0
        ));
        s.push_str(&format!(
            "PER STEP        {}/{}  ({:.1}%)\n",
            self.step_passed,
            self.step_total,
            ratio(self.step_passed, self.step_total) * 100.0
        ));
        s.push_str(&format!(
            "ANSWER QUALITY  {}/{}  ({:.1}%)   <- of the steps that CLAIM something\n",
            self.answer_passed,
            self.answer_total,
            self.answer_rate() * 100.0
        ));
        s
    }

    pub fn json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|e| {
            format!("{{\"error\":\"the report could not be serialized: {e}\"}}")
        })
    }
}

pub fn ratio(passed: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        passed as f64 / total as f64
    }
}

// ---------------------------------------------------------------------------
// The runner
// ---------------------------------------------------------------------------

/// TOOLS THE SUITE DELIBERATELY DOES NOT CARRY, because they have a benchmark
/// of their own.
///
/// THIS IS A NARROW EXCEPTION TO A RULE THIS FILE OTHERWISE KEEPS — that eval and
/// the shell must see the same catalog, because a selection measured over a
/// different list measures a program nobody runs. It is made for two tools and
/// for one reason: `search_filter` and `message_intent` exist to measure SLOT
/// FILLING, which the suite has no way to score. The suite asks "was the right
/// tool called"; these two are only interesting when you also ask "were the
/// right five fields filled with values from the right closed sets", and that
/// question lives in `benchmarks/tasks/`, where `evidence` can assert on the
/// receipt they print.
///
/// The cost of NOT doing this is the reason it is done: adding them to the suite
/// would move `tool_total` from 160 and make every number this project has
/// published incomparable with the next one, to measure something the suite
/// cannot see anyway.
///
/// `the_suite_carries_every_tool_it_is_shown` pins the list at two, so it cannot
/// quietly become the place tools go to avoid being measured.
pub(crate) const BENCHED_SEPARATELY: [&str; 2] = ["search_filter", "message_intent"];

pub(crate) fn selection_catalog(env: &Env, memory: &SharedMemory) -> ToolCatalog {
    let (full, _, _) =
        tacet_tools::catalog::production_catalog(&env.store, memory, Some(FIXED_EPOCH));
    let mut c = ToolCatalog::new();
    for tool in full.tools() {
        if BENCHED_SEPARATELY.contains(&tool.name()) {
            continue;
        }
        if TO_DRY.contains(&tool.name()) {
            c.add(Arc::new(DryTool(Arc::clone(tool))));
        } else {
            c.add(Arc::clone(tool));
        }
    }
    announce_missing_tools(&c);
    c
}

fn announce_missing_tools(catalog: &ToolCatalog) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let missing: Vec<&str> = DISCOVERY_BOUND
            .iter()
            .copied()
            .filter(|n| catalog.find(n).is_none())
            .collect();
        if !missing.is_empty() {
            eprintln!(
                "WARNING: {} is not in the catalog on this machine; the cases expecting these \
                 tools will count as FAILED. This is not a regression, it is a fact of the \
                 platform. Reason: {}",
                missing.join(", "),
                tacet_tools::run_code::RunCodeTool::diagnose()
            );
        }
    });
}

/// WHERE A DISTILLATION SET IS WRITTEN, when `TACET_DISTIL_DIR` is set.
///
/// WHAT THIS IS FOR. A 270M model cannot be taught to call tools from a
/// hand-written dataset that nobody has time to write, and it should not be
/// taught from a bigger model's output either — most of which is wrong. What it
/// CAN be taught from is the subset of a bigger model's output that a benchmark
/// scored as correct. The teacher is a 4B; the definition of "correct" is not a
/// judge model but the same pass/fail the suite has always used.
///
/// ONLY PASSING STEPS ARE WRITTEN, and that is the whole discipline. A step that
/// called the wrong tool, or called nothing, contributes nothing — its prompt is
/// exactly the input where the student must NOT copy the teacher.
///
/// THE PROMPT IS THE RENDERED ONE, template and all, because that is the string
/// the student will be shown at inference. A dataset built from the logical
/// prompt would train the model on a format it never sees.
fn distillation_dir() -> Option<std::path::PathBuf> {
    tacet_kernel::env_var("TACET_DISTIL_DIR")
        .map(std::path::PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// One JSON object per line: `{"case":…,"prompt":…,"completion":…}`.
///
/// APPEND-ONLY AND ONE FILE PER PROCESS. Several benchmark files are run one
/// after another and the set is the union of all of them; a file per process
/// keeps two parallel runs from interleaving half-written lines.
fn write_distillation(case: &str, pairs: &[(String, String)]) {
    let Some(dir) = distillation_dir() else {
        return;
    };
    if pairs.is_empty() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("could not create {}: {e}", dir.display());
        return;
    }
    let path = dir.join(format!("distil-{}.jsonl", std::process::id()));
    let mut body = String::new();
    for (prompt, completion) in pairs {
        // A turn that produced nothing is not an example of anything.
        if completion.trim().is_empty() {
            continue;
        }
        let line = serde_json::json!({
            "case": case,
            "prompt": prompt,
            "completion": completion,
        });
        body.push_str(&line.to_string());
        body.push('\n');
    }
    if body.is_empty() {
        return;
    }
    use std::io::Write;
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(mut f) => {
            if let Err(e) = f.write_all(body.as_bytes()) {
                eprintln!("could not append to {}: {e}", path.display());
            }
        }
        Err(e) => eprintln!("could not open {}: {e}", path.display()),
    }
}

/// THE GENERATION BUDGET THIS MEASUREMENT GIVES THE MODEL, built the way the
/// shell builds it.
///
/// WHY IT IS NOT `SamplingSetting::default()` ANY MORE, and this was the eval
/// penalising the model for a limit the app does not impose. The default caps
/// generation at `GENERATION_SHARE` (1024 tokens) because it is the fallback
/// for call sites that have no prompt to measure. The shell has a prompt and
/// uses `TokenCounter::generation_cap`, which hands the unused part of the
/// window to generation — on qwen3-4b that is roughly fourteen thousand tokens,
/// not one. This set was calling the default.
///
/// MEASURED: `write_code-utility` ("Create a python script utility.py that
/// renames files") and `write_code-web-scraper` both came back
/// `generation was cut off halfway`. The model was writing a real script and
/// ran out of room at 1024 tokens — in the app it would have finished. The
/// suite was reporting a Tacet limit as a model failure, on the two cases most
/// likely to need length.
///
/// THE WINDOW COMES FROM THE FILE, exactly as `engine_window` does in the
/// shell: what the GGUF declares, capped by what the KV cache can afford on
/// this device, floored at `CONTEXT_BUDGET`. An engine that declares nothing
/// (FakeEngine) lands on the floor, which is the old behaviour and the safe one.
fn generation_counter(engine: &Arc<dyn EngineProvider>) -> tacet_engine::TokenCounter {
    let identity = engine.identity();
    let path = std::path::Path::new(&identity.model_path);
    let declared = engine
        .context_length()
        .or_else(|| tacet_engine::gguf_context_length(path));
    let per_token = tacet_engine::gguf_kv_bytes_per_token(path);
    let device = match identity.device.as_str() {
        "metal" => tacet_engine::Device::Metal,
        "cuda" => tacet_engine::Device::Cuda,
        _ => tacet_engine::Device::Cpu,
    };
    let window = tacet_engine::context_budget(declared, per_token, device);
    tacet_engine::TokenCounter::new(window, tacet_engine::GENERATION_SHARE)
}

/// THE SUITE RUNS ONE CASE AT A TIME, AND THAT IS THE MEASURED CHOICE.
///
/// The obvious idea, and the one this note exists to close: the cases are
/// independent — `Env::setup` already hands each one its own temporary
/// directory and its own store, for exactly this reason — so run five at once
/// and finish in a fifth of the time.
///
/// It does not work, and the reason is not the code. `CandleEngine` holds its
/// model behind a `Mutex` because the KV cache lives INSIDE the model, so one
/// engine is one generation at a time by correctness and not by oversight.
/// Parallel cases therefore mean N engines, N copies of the weights in VRAM,
/// and N streams competing for one GPU.
///
/// MEASURED 5 SEP 2026 on a rented RTX 3090 (24 GB, driver 570, CUDA 12.8),
/// qwen3-4b Q4_K_M, by running a second copy of this suite beside the first and
/// reading both their per-turn rates:
///
///   one stream alone       ~55 tok/s   (median of 133 turns)
///   two streams, steady    ~28 tok/s and ~28 tok/s
///
/// The aggregate is ~56 tok/s against ~55. There is no throughput to win: a
/// SINGLE stream already saturates this card. The instruments agree — with one
/// stream the board draws 349 W of a 380 W limit at 77% memory-controller
/// utilisation, and with two it draws 351 W at 72%. The GPU was never waiting
/// for work, so there was no gap for a second stream to fill.
///
/// WHAT THE 98% "GPU utilisation" FIGURE IS NOT. `nvidia-smi` reports the
/// fraction of time at least one kernel was resident, which reads 98% for a
/// batch-1 decode that leaves most of the card idle; it was the power draw and
/// the memory-controller figure, not that number, that answered the question.
///
/// THE GAIN THAT IS AVAILABLE IS BATCHING, which is the opposite trade: `b`
/// sequences in ONE forward read the weights once and produce `b` tokens, where
/// `b` processes read them `b` times. Measured on the same card by
/// `examples/batch_decode.rs`: 124 tok/s at batch 1 against 504 at batch 32,
/// four times the tokens from the hardware that gave nothing to running four
/// copies. What it costs is an engine that decodes several sequences together,
/// with one sampler and one grammar mask each.
///
/// AND THE LARGER GAIN WAS NOT ABOUT THE GPU AT ALL. The run that provoked all
/// of this was made with the wrong weights: `Qwen/Qwen3-4B` rather than the 2507
/// instruct model every number here was measured on. The same 184 cases took
/// 53.7 minutes on the first and 6.2 on the second, because the hybrid model
/// spends a median of 237 tokens per turn against 19 — with `thinking` empty, so
/// it is prose in front of a call and not deliberation. Before reaching for a
/// bigger card, check which file is loaded.
///
/// VRAM, for whoever measures this again on a bigger card, where the answer may
/// differ: four processes at a 40960 window took 22.8 GB of 24 GB — the weights
/// are 2.5 GB and the rest is KV cache.
/// WHERE THE TOOLS COME FROM, chosen by the caller rather than fixed here.
///
/// The suite and a benchmark want opposite things and both are right. The
/// SUITE's catalog is compiled in and deliberately narrow — no network, no
/// host-dependent tool — because it is the number this project publishes and it
/// must not move when the reader installs an addon. A BENCHMARK's catalog is the
/// one the machine actually has, MCP servers and addons included, because the
/// whole question is "does it call MY tools".
///
/// A closure rather than an enum: `tacet-eval` must not learn how to start an
/// MCP client to offer "the host catalog" as a variant, and the crate that
/// already knows how hands it in.
pub type CatalogFor<'a> = &'a dyn Fn(&Env, &SharedMemory) -> ToolCatalog;

/// The suite's own catalog, as a `CatalogFor`. Every existing caller gets this
/// and nothing about their measurement changes.
pub fn suite_catalog(env: &Env, memory: &SharedMemory) -> ToolCatalog {
    selection_catalog(env, memory)
}

pub fn run_selection(cases: &[SelectionCase], engine: &Arc<dyn EngineProvider>) -> SelectionReport {
    run_selection_with_options(cases, engine, None, false)
}

pub fn run_selection_with_options(
    cases: &[SelectionCase],
    engine: &Arc<dyn EngineProvider>,
    budget: Option<usize>,
    force_tool_name: bool,
) -> SelectionReport {
    run_selection_in(cases, engine, budget, force_tool_name, &suite_catalog)
}

/// The body, with the catalog supplied. See `CatalogFor`.
pub fn run_selection_in(
    cases: &[SelectionCase],
    engine: &Arc<dyn EngineProvider>,
    budget: Option<usize>,
    force_tool_name: bool,
    catalog_for: CatalogFor<'_>,
) -> SelectionReport {
    let started = std::time::Instant::now();
    let total = cases.len();
    let outcomes: Vec<SelectionOutcome> = cases
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let outcome = run_selection_case_in(c, engine, budget, force_tool_name, catalog_for);
            report_progress(i + 1, total, &c.name, started.elapsed());
            outcome
        })
        .collect();
    let catalog = production_catalog_names();
    SelectionReport::new(
        engine.identity(),
        started.elapsed().as_millis(),
        catalog,
        outcomes,
    )
}

/// THE LIVE TRACE — what the suite is doing RIGHT NOW, not what it did.
///
/// WHY IT GOES DOWN TO THE TURN. A per-case line answers "how far along", which
/// is a different question from "what is it doing". The run that forced this
/// distinction sat at 2 h 02 m against a measured 14 s/case — a 4.5x anomaly —
/// and a per-case line would have shown the case name and then nothing for
/// however long that case took. What was needed was the ability to see a case
/// ENTER a turn and not leave it. So the trace fires before the work, not after:
/// `generating` is printed BEFORE `engine.generate`, so a hang is visible as a
/// line with no successor rather than as silence.
///
/// STDERR, for the same reason as `report_progress`: the report goes to stdout
/// and gets redirected to a file by anyone comparing two runs.
///
/// INDENTED UNDER ITS CASE so the eye can skip it. The per-case summary lines
/// sit at the left margin; everything a case does while it is running is
/// indented four spaces, which makes `grep '^  \['` a clean list of results and
/// leaves the detail for whoever is actually watching.
fn trace(detail: &str) {
    eprintln!("      {detail}");
}

/// The message a case sends, cut to one line.
///
/// THE CUT IS BY CHARACTER, NOT BYTE: the Turkish selection set is half this
/// suite and slicing a `&str` mid-`ç` panics. It also collapses newlines — a
/// multi-line message would otherwise break the one-line-per-event shape the
/// trace depends on for being skimmable.
fn truncate_for_trace(message: &str) -> String {
    let flat: String = message
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let mut out: String = flat.chars().take(56).collect();
    if flat.chars().count() > 56 {
        out.push('…');
    }
    out
}

/// ONE LINE PER FINISHED CASE, on stderr, carrying a projection of the rest.
///
/// WHY IT EXISTS, measured rather than imagined: this suite was run on real
/// weights and took **1 h 37 min and counting** while printing NOTHING after
/// "115 cases running — takes minutes". `lsof` on the process showed only two
/// open outputs — a 0-byte stdout, because the report is serialised at the end,
/// and a 161-byte stderr holding the three startup lines. There was no way to
/// tell 12 cases from 112, or progress from a hang, without sampling the
/// process's own stack. A command that can run for over an hour and cannot
/// answer "how far along are you" is not measurable, and this repository's whole
/// argument is that unmeasurable claims are the ones that turn out false.
///
/// IT GOES TO STDERR, NOT STDOUT, and that is load-bearing rather than a habit:
/// `--json` writes the report to stdout and is redirected to a file by anyone
/// comparing two runs. A progress line on stdout would corrupt every one of
/// those files. stderr is also where the existing startup lines already go, so
/// a reader sees one stream in one order.
///
/// THE PROJECTION IS A MEAN, AND IT IS LABELLED AS ONE. Cases are not equal —
/// a multi-step case runs the model several times, and a case that engages the
/// grammar pays for a `TokenMask::walk` over the whole vocabulary at every token
/// (sampled during that same run, the mask walk was the hottest symbol in the
/// process, above the attention forward pass). So the estimate drifts, early on
/// especially. It is still the difference between "unknown" and "roughly an
/// hour", which is the decision the person watching actually has to make.
fn report_progress(done: usize, total: usize, name: &str, elapsed: std::time::Duration) {
    let secs = elapsed.as_secs_f64();
    let per_case = secs / done as f64;
    let left = per_case * (total - done) as f64;
    eprintln!(
        "  [{done:>3}/{total}] {name} · {} elapsed · {:.0}s/case · ~{} left",
        human_duration(secs),
        per_case,
        human_duration(left)
    );
}

/// `m`/`s` rather than a bare seconds count: the numbers here reach four digits,
/// and "4127s" is a number the reader has to convert before it means anything.
fn human_duration(secs: f64) -> String {
    let secs = secs.max(0.0) as u64;
    if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m{:02}s", secs / 60, secs % 60)
    }
}

fn production_catalog_names() -> Vec<String> {
    let Ok(env) = Env::setup() else {
        return Vec::new();
    };
    let memory = SharedMemory::in_memory();
    selection_catalog(&env, &memory)
        .names()
        .into_iter()
        .map(String::from)
        .collect()
}

pub fn run_selection_case(
    case: &SelectionCase,
    engine: &Arc<dyn EngineProvider>,
) -> SelectionOutcome {
    run_selection_case_with_options(case, engine, None, false)
}

pub fn run_selection_case_with_options(
    case: &SelectionCase,
    engine: &Arc<dyn EngineProvider>,
    budget: Option<usize>,
    force_tool_name: bool,
) -> SelectionOutcome {
    run_selection_case_in(case, engine, budget, force_tool_name, &suite_catalog)
}

/// The body, with the catalog supplied. See `CatalogFor`.
pub fn run_selection_case_in(
    case: &SelectionCase,
    engine: &Arc<dyn EngineProvider>,
    budget: Option<usize>,
    force_tool_name: bool,
    catalog_for: CatalogFor<'_>,
) -> SelectionOutcome {
    let env = match Env::setup() {
        Ok(e) => e,
        Err(e) => {
            return SelectionOutcome {
                name: case.name.clone(),
                category: case.category,
                passed: false,
                steps: vec![StepOutcome {
                    message: String::new(),
                    expected: None,
                    called: Vec::new(),
                    passed: false,
                    answer: format!("the environment could not be set up: {e}"),
                    answer_passed: false,
                    // The host failed, not a claim. Counting it would put a
                    // machine problem into the answer-quality denominator.
                    claims: false,
                    ended: Ending::HostFailed,
                }],
            };
        }
    };
    let memory = SharedMemory::in_memory();
    let catalog = catalog_for(&env, &memory);
    let executor = ToolExecutor::new(catalog.clone());
    let traces = Arc::new(TraceCollector::new());
    let mut ctx = ToolContext::new(
        Arc::clone(&env.store) as Arc<dyn tacet_kernel::DataStore>,
        env.dir(),
        Arc::clone(&traces) as Arc<dyn tacet_kernel::Reporter>,
    );

    let router = if let Some(b) = budget {
        Router::new().budget_override(b)
    } else {
        Router::new()
    };

    let mut history: Vec<Turn> = Vec::new();
    let mut step_outcomes: Vec<StepOutcome> = Vec::new();
    let case_started = std::time::Instant::now();
    let step_count = case.steps.len();

    // THE SKILL STORE, and its absence was the largest gap between this
    // measurement and the program it claims to measure. Production attaches ONE
    // matching skill to the turn behind a `<guidance>` fence (see the turn loop
    // in `tacet-cli`), and that block is the thing carrying the concrete
    // `tool(args)` shape the model imitates — the prompt module's own header
    // says the guide sits immediately before the question because "in a small
    // model the last blocks carry the most weight". Measuring a selection
    // without it measured a prompt no user has ever been sent.
    //
    // THE REPEAT SUPPRESSION IS NOT COPIED, and that is a deliberate
    // simplification rather than an omission: production skips re-injecting the
    // same skill on a nearby turn, which is a CONVERSATION-length behaviour,
    // and every case here is one or two steps long. Copying the state machine
    // would add a way for the two to drift with nothing measuring the
    // difference.
    let skills = tacet_skills::SkillStore::default_set();
    let counter = generation_counter(engine);

    for (step_index, step) in case.steps.iter().enumerate() {
        trace(&format!(
            "{} · step {}/{} · \"{}\"",
            case.name,
            step_index + 1,
            step_count,
            truncate_for_trace(&step.message)
        ));
        let ticket = executor.new_turn();
        let mut turn_pairs: Vec<(String, String)> = Vec::new();
        traces.reset();
        let selected: ToolCatalog = router.select(&step.message, &catalog).into_iter().collect();
        let selected_names: Vec<String> = selected.names().into_iter().map(String::from).collect();
        let mut guide = skills
            .matching(&step.message, Some(&selected_names))
            .map(tacet_skills::injection_text);
        // THE WEB NUDGE, on the same condition production uses. It is one
        // sentence and it exists because the small model does not reach for
        // `web_search` on its own; leaving it out of the measurement made the
        // web cases look harder than they are in the app.
        if tacet_tools::router::score_intent(&step.message).dominant()
            == tacet_tools::router::IntentProfile::Web
        {
            const WEB_NUDGE: &str = "this question needs live information from the internet. \
                 Call the web_search tool first; do not answer it from memory.";
            guide = Some(match guide {
                Some(g) => format!("{g}\n{WEB_NUDGE}"),
                None => WEB_NUDGE.to_string(),
            });
        }
        let constraint = engine.vocab().map(|v| {
            if force_tool_name {
                CallConstraint::new(&v, &selected)
            } else {
                CallConstraint::new(&v, &catalog)
            }
        });

        let mut turn_tools: Vec<Turn> = Vec::new();
        let mut called: Vec<String> = Vec::new();
        let mut answer = String::new();
        // OUT OF TURNS UNTIL SOMETHING ELSE HAPPENS. Falling out of the loop
        // without ever answering is the shell's failed run; making it the
        // default means the loop has to earn any other ending.
        let mut ended = Ending::OutOfTurns;
        // A duplicate call ends the tool phase — the shell does the same, and
        // this set exists to measure the shell.
        let mut must_answer = false;

        for turn in 0..MAX_TURNS {
            // THE LAST PASS IS OFFERED NO TOOLS — the shell does the same, and
            // this set exists to measure the shell. See the rationale in
            // `tacet-cli`'s turn loop.
            //
            // IT CANNOT LOWER A HIT RATE THAT WAS REAL: a case counts as a hit
            // when the expected tool is called on ANY pass, and the passes it
            // could have been called on are untouched. What it removes is the
            // fourth identical call, which was never a hit — only a way to
            // finish the turn with nothing said.
            let final_turn = turn + 1 == MAX_TURNS || must_answer;
            let first = turn_tools.is_empty();
            let question = if first { step.message.as_str() } else { "" };
            let previous: Vec<Turn> = if first {
                history.clone()
            } else {
                history
                    .iter()
                    .cloned()
                    .chain(std::iter::once(Turn::user(&step.message)))
                    .chain(turn_tools.iter().cloned())
                    .collect()
            };
            let system = if final_turn {
                format!("{SYSTEM_INSTRUCTIONS}\n\n{FINAL_PASS_INSTRUCTION}")
            } else {
                SYSTEM_INSTRUCTIONS.to_string()
            };
            let mut prompt = Prompt::new(&system, question).with_history(previous);
            if let Some(g) = &guide {
                prompt = prompt.with_guide(g);
            }
            if !final_turn {
                prompt = prompt.with_tools(&selected);
            }

            trace(&format!(
                "  turn {}/{} · generating{} · cap {} tokens · {:.0}s into this case",
                turn + 1,
                MAX_TURNS,
                if final_turn {
                    " (no tools offered)"
                } else {
                    ""
                },
                counter.generation_cap(&prompt),
                case_started.elapsed().as_secs_f64()
            ));
            // WHAT THE GENERATION COST, so a slow case is attributed rather than
            // guessed at. Two very different things look identical from outside:
            // a model producing 40 tokens slowly, and one producing 900 quickly.
            // Only the pair (count, rate) separates them. `stop` is here because
            // "ran into the cap" and "chose to end" take the same wall time and
            // are completely different defects.
            let gen_started = std::time::Instant::now();
            let generation = match wait(
                engine.generate(
                    &prompt,
                    constraint
                        .as_ref()
                        .filter(|_| !final_turn)
                        .map(|c| c as &dyn tacet_engine::Constrainer),
                    SamplingSetting {
                        max_tokens: counter.generation_cap(&prompt),
                        ..Default::default()
                    },
                ),
            ) {
                Ok(g) => g,
                Err(e) => {
                    answer = format!("engine error: {e}");
                    ended = Ending::EngineError;
                    break;
                }
            };
            let gen_secs = gen_started.elapsed().as_secs_f64();
            // THE TEACHER'S OWN WORDS, kept only long enough to find out whether
            // they were right. See `write_distillation`.
            if distillation_dir().is_some() {
                turn_pairs.push((
                    prompt.text_with_template(engine.template()),
                    generation.text.clone(),
                ));
            }
            trace(&format!(
                "  turn {}/{} · {} tokens in {:.1}s ({:.1} tok/s) · stop={:?}",
                turn + 1,
                MAX_TURNS,
                generation.token_count,
                gen_secs,
                generation.token_count as f64 / gen_secs.max(1e-9),
                generation.stop
            ));
            // A CUT-OFF PASS IS A LOST PASS, NOT A LOST TURN.
            //
            // This killed the whole turn and threw away every tool result the
            // earlier passes had already collected — so a case where the model
            // called correctly twice and then ran long on the third pass was
            // recorded as if nothing had happened. Four steps of the shipped
            // baseline ended here, and `write_code-script` had two successful
            // `write_code` calls in hand when it did.
            //
            // Going to the final pass instead keeps those results and gives the
            // model the one thing it is missing: a pass with no tools and an
            // instruction to answer. If the cut-off happens ON the final pass
            // there is nothing left to try, and it still ends the turn.
            if !generation.stop.is_complete() {
                if turn + 1 == MAX_TURNS {
                    answer = "generation was cut off halfway".into();
                    ended = Ending::CutOff;
                    break;
                }
                must_answer = true;
                continue;
            }
            // THE TOOL'S OWN TIME, separated from the model's. Without this the
            // two are one number and the wrong one gets optimised: `calendar-day`
            // reads as a 39 s case, of which 9.5 s is generation and 30 s is
            // `osascript` talking to the Calendar app.
            let tool_started = std::time::Instant::now();
            // ON THE LAST PASS, WHAT THE MODEL WRITES IS THE ANSWER.
            //
            // The last pass is offered NO tools and told to answer; running a
            // call it writes there is the harness disagreeing with the prompt it
            // just sent. Three steps of the shipped baseline died exactly this
            // way — every pass spent calling, the budget gone, nothing said —
            // and the loop had no pass left to turn the result into a sentence.
            //
            // GATED ON `turn + 1 == MAX_TURNS`, NOT ON `final_turn`, and the
            // narrowing is the whole safety argument. `final_turn` is also true
            // when `must_answer` was set by a duplicate call, which can fire as
            // early as pass 2 — SIXTEEN currently-passing steps have three or
            // more calls with a consecutive repeat, and gating on `final_turn`
            // would replace their real answer with a raw call string. Gating on
            // the true last pass captures the three four-call steps and touches
            // none of the sixteen.
            let last_pass = turn + 1 == MAX_TURNS;
            if last_pass {
                // AND A CALL WRITTEN THERE IS STILL NOT AN ANSWER. Not executing
                // it is half the rule; the other half is not counting it as the
                // sentence the user got. A turn whose last words are
                // `calculate({"expression":"125*8"})` said nothing, and calling
                // that an answer would trade one wrong verdict for another.
                if tacet_tools::executor::ToolCall::parse(&generation.text).is_some() {
                    ended = Ending::OutOfTurns;
                } else {
                    answer = generation.text.clone();
                    ended = Ending::Answered;
                }
                break;
            }
            let Some(outcome) = wait(executor.execute_raw(&generation.text, ticket, &mut ctx))
            else {
                // THE ONLY OTHER EXIT THAT PRODUCED AN ANSWER: the generation was
                // not a call, so it is what the user is told.
                answer = generation.text.clone();
                ended = Ending::Answered;
                break;
            };
            trace(&format!(
                "  turn {}/{} · {}() took {:.1}s · {:.0}s into this case",
                turn + 1,
                MAX_TURNS,
                outcome.tool_name,
                tool_started.elapsed().as_secs_f64(),
                case_started.elapsed().as_secs_f64()
            ));
            called.push(outcome.tool_name.clone());
            if outcome.reason == tacet_tools::executor::ExecutionReason::RepeatedCall {
                must_answer = true;
            }
            turn_tools.push(Turn::tool(outcome.to_model.clone()));
        }

        // A TURN THAT NEVER ANSWERED IS NOT A HIT, AND A DEAD ENGINE IS NOT AN
        // IRRELEVANCE PASS.
        //
        // `called` alone decided this, so falling out of `for turn in
        // 0..MAX_TURNS` — every pass spent calling tools, nothing said to the
        // user — scored as a hit; three steps of the shipped baseline are
        // exactly that. And an engine error leaves `called` empty, so
        // `None => called.is_empty()` scored a broken run as the safety
        // property holding.
        let passed = ended.is_measurable()
            && match &step.expected {
                Some(name) => called.iter().any(|c| c == name),
                None => called.is_empty(),
            };

        if passed {
            write_distillation(&case.name, &turn_pairs);
        }

        let tool_outcomes_text: Vec<String> = turn_tools.iter().map(|t| t.text.clone()).collect();
        let answer_passed =
            passed && check_answer_quality(step, &answer, &called, &tool_outcomes_text);

        history.push(Turn::user(&step.message));
        history.extend(turn_tools);
        if !answer.is_empty() {
            history.push(Turn::assistant(&answer));
        }

        step_outcomes.push(StepOutcome {
            message: step.message.clone(),
            expected: step.expected.clone(),
            called,
            passed,
            answer,
            answer_passed,
            claims: !step.evidence.is_empty()
                || !step.forbidden.is_empty()
                || step.language.is_some(),
            ended,
        });
    }

    SelectionOutcome {
        name: case.name.clone(),
        category: case.category,
        passed: step_outcomes.iter().all(|s| s.passed),
        steps: step_outcomes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// The set's COVERAGE of the catalog: if a new tool is added and no case is
    /// written for it, this test breaks. Otherwise the catalog grows, the
    /// measurement does not, and the new tool goes to production unmeasured.
    #[test]
    fn there_are_at_least_two_cases_for_every_tool() {
        let env = Env::setup().unwrap();
        let memory = SharedMemory::in_memory();
        let catalog = selection_catalog(&env, &memory);
        let cases = selection_cases();
        for tool in catalog.tools() {
            let count = cases
                .iter()
                .flat_map(|c| c.steps.iter())
                .filter(|s| s.expected.as_deref() == Some(tool.name()))
                .count();
            assert!(
                count >= 2,
                "there are {} cases for {}, at least 2 are needed",
                count,
                tool.name()
            );
        }
    }

    #[test]
    fn there_are_at_least_five_irrelevance_cases() {
        let n = selection_cases()
            .iter()
            .filter(|c| c.category == Category::Irrelevance)
            .count();
        assert!(
            n >= 5,
            "the irrelevance case count is {n}, it must be at least 5"
        );
    }

    #[test]
    fn there_are_at_least_three_multi_turn_cases() {
        let multi: Vec<_> = selection_cases()
            .into_iter()
            .filter(|c| c.category == Category::MultiTurn)
            .collect();
        assert!(
            multi.len() >= 3,
            "the multi-turn case count is {}",
            multi.len()
        );
        assert!(
            multi.iter().all(|c| c.steps.len() >= 2),
            "a multi-turn case cannot have a single step"
        );
    }

    /// Cases saying the same intent differently: there are four phrasings for
    /// `time`.
    #[test]
    fn the_time_intent_is_measured_with_different_phrasings() {
        let messages: Vec<String> = selection_cases()
            .iter()
            .flat_map(|c| c.steps.clone())
            .filter(|s| s.expected.as_deref() == Some("time"))
            .map(|s| s.message)
            .collect();
        assert!(messages.len() >= 4, "{messages:?}");
        // "what time is it" and "what day of the month is it" ask for the SAME
        // tool without sharing the same words — the set must carry that
        // distinction.
        assert!(messages.iter().any(|m| m.contains("What time")));
        assert!(messages.iter().any(|m| m.contains("day of the month")));
    }

    #[test]
    fn the_case_names_are_unique() {
        let c = selection_cases();
        let set: HashSet<&str> = c.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(set.len(), c.len());
    }

    /// Does every expected tool REALLY exist in the catalog — a case with a typo
    /// returns "FAIL" forever and nobody understands why.
    ///
    /// PLATFORM: this test, like
    /// `the_expected_tool_does_not_drop_out_of_the_routers_budget`, DID NOT PASS
    /// OUTSIDE macOS — the audit had only flagged the other one, but the failure
    /// is THE SAME failure. Without fixing both, `cargo test --workspace` would
    /// still be red on Linux. The typo claim IS PRESERVED: the `Regression`
    /// branch still catches every misspelled name; the tools bound to the
    /// discovery gate may legitimately be absent.
    #[test]
    fn the_expected_tools_exist_in_the_catalog() {
        let env = Env::setup().unwrap();
        let memory = SharedMemory::in_memory();
        let catalog = selection_catalog(&env, &memory);
        for c in selection_cases() {
            for s in c.steps {
                if let Some(name) = s.expected {
                    assert_ne!(
                        expectation_state(&name, &catalog),
                        Expectation::Regression,
                        "not in the catalog and its absence is not legitimate (typo?): {name}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_dried_tool_keeps_its_identity() {
        let env = Env::setup().unwrap();
        let memory = SharedMemory::in_memory();
        let (full, _, _) =
            tacet_tools::catalog::production_catalog(&env.store, &memory, Some(FIXED_EPOCH));
        let dry = selection_catalog(&env, &memory);
        for name in TO_DRY {
            let (Some(f), Some(d)) = (full.find(name), dry.find(name)) else {
                continue;
            };
            // What determines the selection is the description: drying MUST NOT
            // change it.
            assert_eq!(
                f.description(),
                d.description(),
                "{name}'s description changed while drying"
            );
            assert_eq!(f.schema().json_schema(), d.schema().json_schema());
        }
    }

    /// An expectation's state against the catalog.
    ///
    /// IT MUST BE A SEPARATE FUNCTION: had the decision been buried directly in
    /// the test's loop, the `SkippedByPlatform` branch would NEVER RUN on macOS
    /// and the Linux fix would stay unmeasured — the very "the mechanism was
    /// built, it was never seen working" failure. As a pure function it can be
    /// measured on this machine with a synthetic catalog (see
    /// `a_missing_tool_is_told_apart_from_a_regression`).
    #[derive(Debug, PartialEq, Eq)]
    enum Expectation {
        /// The tool is in the catalog; the selection can really be measured.
        Measurable,
        /// The tool is not in the catalog but ITS ABSENCE IS LEGITIMATE (the
        /// discovery gate).
        SkippedByPlatform,
        /// The tool is not in the catalog and its absence is not legitimate — a
        /// real failure.
        Regression,
    }

    fn expectation_state(expected: &str, catalog: &ToolCatalog) -> Expectation {
        if catalog.find(expected).is_some() {
            Expectation::Measurable
        } else if DISCOVERY_BOUND.contains(&expected) {
            Expectation::SkippedByPlatform
        } else {
            Expectation::Regression
        }
    }

    /// THE SKIP RULE IS MEASURED ON THIS MACHINE — without waiting for Linux.
    ///
    /// A catalog with `run_code` dropped is SYNTHESIZED (what really happens on
    /// Linux/Windows) and all three branches run here: a tool bound to the
    /// discovery gate is skipped, a tool that is not bound counts as a
    /// REGRESSION, a tool in the catalog is measured. Otherwise whether the fix
    /// picks the right branch could only be learned on a Linux machine.
    #[test]
    fn a_missing_tool_is_told_apart_from_a_regression() {
        let env = Env::setup().unwrap();
        let memory = SharedMemory::in_memory();
        let full = selection_catalog(&env, &memory);

        // Imitate a non-macOS machine by dropping the tools bound to the
        // discovery gate.
        let mut synthetic = ToolCatalog::new();
        for tool in full.tools() {
            if !DISCOVERY_BOUND.contains(&tool.name()) {
                synthetic.add(Arc::clone(tool));
            }
        }

        for name in DISCOVERY_BOUND {
            assert_eq!(
                expectation_state(name, &synthetic),
                Expectation::SkippedByPlatform,
                "{name}'s absence must count as a fact of the platform"
            );
        }
        assert_eq!(
            expectation_state("calculate", &synthetic),
            Expectation::Measurable
        );
        // The loss of a tool NOT BOUND to the discovery gate must not be
        // silenced.
        assert_eq!(
            expectation_state("create_document", &full),
            Expectation::Measurable
        );
        assert_eq!(
            expectation_state("no_such_tool", &full),
            Expectation::Regression
        );
    }

    /// THE ROUTER GATE — the layer BEFORE the model.
    ///
    /// There are 10 tools in the catalog and the budget is 8. If the expected
    /// tool falls outside those 8 the model NEVER SEES IT IN THE PROMPT; the
    /// case fails as "the model chose wrong", when the one choosing is not the
    /// model but the router. This test caught and fixed exactly that situation
    /// in four cases (run_code x2, create_document, web_fetch) and stops it from
    /// coming back.
    ///
    /// IT NEEDS NO MODEL: it takes seconds and runs in CI. Because a real
    /// model-based measurement takes minutes, protecting this layer separately
    /// and CHEAPLY is essential.
    ///
    /// PLATFORM: the test used to FAIL OUTSIDE macOS and that was a test bug,
    /// not a regression. `run_code`/`write_code` are never in the catalog on
    /// Linux (without bwrap) or on Windows; claiming that a tool which does not
    /// exist "dropped out of the budget" is the measurement measuring its own
    /// setup. The fix is not to SILENCE the test but to ASK THE RIGHT QUESTION:
    /// if the tool is not in the catalog, first ask whether its absence is
    /// LEGITIMATE (`DISCOVERY_BOUND`); if it is, the case is skipped and the
    /// skip IS PRINTED; if it is not, the test fails right there.
    #[test]
    fn the_expected_tool_does_not_drop_out_of_the_routers_budget() {
        budget_guard("english", &selection_cases());
    }

    /// THE SAME GUARD, IN TURKISH — and it is not a formality.
    ///
    /// Until the router had a Turkish trigger table, three cases here failed:
    /// "Dolar kuru şu an ne durumda?" and both `remember` cases touched no
    /// trigger at all, scored zero on every profile, and the nine-slot budget
    /// filled with the head of the catalog. The model was then marked wrong for
    /// not calling a tool it had never been shown. Running the guard over one
    /// locale only measured the locale it was written in.
    #[test]
    fn the_expected_tool_survives_the_budget_in_turkish_too() {
        budget_guard("turkish", &turkish_selection_cases());
    }

    fn budget_guard(suite: &str, cases: &[SelectionCase]) {
        let env = Env::setup().unwrap();
        let memory = SharedMemory::in_memory();
        let catalog = selection_catalog(&env, &memory);
        let router = Router::new();
        let (mut measured, mut skipped) = (0usize, 0usize);
        for c in cases {
            for s in &c.steps {
                let Some(expected) = s.expected.as_deref() else {
                    continue;
                };
                match expectation_state(expected, &catalog) {
                    Expectation::Measurable => {}
                    Expectation::SkippedByPlatform => {
                        eprintln!(
                            "{}: {expected} is not in the catalog on this platform, case skipped",
                            c.name
                        );
                        skipped += 1;
                        continue;
                    }
                    Expectation::Regression => panic!(
                        "{}: {expected} is NOT in the catalog and is not bound to the discovery \
                         gate — this is a real regression",
                        c.name
                    ),
                }
                let selection: Vec<String> = router
                    .select(&s.message, &catalog)
                    .iter()
                    .map(|x| x.name().to_string())
                    .collect();
                assert!(
                    selection.iter().any(|x| x == expected),
                    "{} / {:?}: {expected} dropped out of the budget, selection: {selection:?}",
                    c.name,
                    s.message
                );
                measured += 1;
            }
        }
        // SKIPPING MUST NOT BE AN ESCAPE HATCH: a test that skips everything
        // burns green and measures nothing. If the whole catalog has dropped,
        // that is a failure too, and it shows up here.
        assert!(
            measured > skipped,
            "{suite}: most of the cases were skipped ({skipped} skipped / {measured} measured) — \
             the catalog setup may be broken"
        );
    }

    #[test]
    fn the_report_counts_the_two_rates_separately() {
        let outcomes = vec![
            SelectionOutcome {
                name: "a".into(),
                category: Category::Tool,
                passed: false,
                steps: vec![StepOutcome {
                    message: "m".into(),
                    expected: Some("calculate".into()),
                    called: vec![],
                    passed: false,
                    answer: String::new(),
                    answer_passed: false,
                    claims: false,
                    ended: Ending::OutOfTurns,
                }],
            },
            SelectionOutcome {
                name: "b".into(),
                category: Category::Irrelevance,
                passed: true,
                steps: vec![StepOutcome {
                    message: "Hello".into(),
                    expected: None,
                    called: vec![],
                    passed: true,
                    answer: String::new(),
                    answer_passed: true,
                    claims: false,
                    ended: Ending::Answered,
                }],
            },
        ];
        let r = SelectionReport::new(
            tacet_engine::EngineIdentity {
                engine: "test".into(),
                ..Default::default()
            },
            0,
            Vec::new(),
            outcomes,
        );
        assert_eq!((r.tool_passed, r.tool_total), (0, 1));
        assert_eq!((r.irrelevance_passed, r.irrelevance_total), (1, 1));
        // A single "success rate" would melt these two into 1/2; they must stay
        // separate.
        assert!((r.tool_rate() - 0.0).abs() < f64::EPSILON);
        assert!((r.irrelevance_rate() - 1.0).abs() < f64::EPSILON);
        assert!(r.table().contains("IRRELEVANCE"));
    }
}

#[cfg(test)]
mod trigger_lint {
    use super::*;
    use tacet_tools::router::{IntentProfile, score_intent};

    /// LINT 3 — NOTHING THAT IS NOT A REQUEST MAY SCORE AS ONE.
    ///
    /// This is the corpus half of the substring guard. The message side of the
    /// router was matching triggers with a bare `contains`, so "Çok
    /// teşekkürler" folded to "cok tesekkurler", CONTAINED the three-letter web
    /// trigger "url", and a thank-you pulled both web tools to the front of the
    /// budget — on the irrelevance rate, which is the number the CLI ties its
    /// exit code to.
    ///
    /// The rule that fixed it lives in one function, and the two lints in
    /// `router.rs` keep that function in the scoring path. This one asks the
    /// question the other way round: over every greeting and every piece of
    /// small talk in BOTH suites, does anything score at all? A new trigger
    /// that collides with an everyday word shows up here the day it is added,
    /// in whichever language it was added for.
    ///
    /// IT NEEDS NO MODEL and finishes in microseconds — the whole point of
    /// catching this class of bug at the layer below the model.
    #[test]
    fn no_irrelevant_message_scores_on_any_profile() {
        let mut offenders: Vec<String> = Vec::new();
        for (suite, cases) in [
            ("english", selection_cases()),
            ("turkish", turkish_selection_cases()),
        ] {
            for case in cases.iter().filter(|c| c.category == Category::Irrelevance) {
                for step in &case.steps {
                    let scores = score_intent(&step.message);
                    for profile in IntentProfile::ALL {
                        let score = scores.score(profile);
                        if score > 0 {
                            offenders.push(format!(
                                "{suite}/{}: {:?} scored {score} on {:?}",
                                case.name,
                                step.message,
                                profile.name()
                            ));
                        }
                    }
                }
            }
        }
        // THE RECORDED LIST, and it is short on purpose.
        //
        // All three are the same word doing two jobs: "today" and its Turkish
        // twin "bugün" are genuine time triggers ("what is the date today?")
        // AND ordinary conversational filler ("how are you today?"). Unlike the
        // "url" inside "teşekkürler" case, this is not a match hiding inside a
        // longer word — the trigger fires on the word it was written for, and
        // the word is simply ambiguous.
        //
        // DELETING THE TRIGGER WOULD COST MORE THAN IT SAVES: without it a real
        // question about today's date scores nothing and the time tool falls out
        // of the budget, which is the failure this whole file exists to catch.
        // So the promotion stands and the model is left to decline — which it
        // does: all three cases pass, and the irrelevance rate is 6/6 and 4/4.
        //
        // What this lint is really for is the NEXT entry. Anything that appears
        // here and is not one of these three is a trigger colliding with an
        // everyday word, in whichever language it was added for, on the day it
        // is added.
        let accepted = [
            "english/chat-how-are-you",
            "english/chat-mood",
            "turkish/tr-sohbet",
        ];
        let unexpected: Vec<&String> = offenders
            .iter()
            .filter(|o| !accepted.iter().any(|a| o.starts_with(a)))
            .collect();
        assert!(
            unexpected.is_empty(),
            "a message that asks for nothing is scoring as a request, which means \
             a trigger is matching something it should not — most likely inside a \
             longer word:\n  {}",
            unexpected
                .iter()
                .map(|o| o.as_str())
                .collect::<Vec<_>>()
                .join("\n  ")
        );
        // AND THE LIST MAY NOT SHRINK SILENTLY EITHER: if one of the three stops
        // scoring, a trigger was removed or a rule changed, and the reasoning
        // above needs revisiting rather than quietly rotting.
        assert_eq!(
            offenders.len(),
            accepted.len(),
            "the accepted list no longer matches what the router does:\n  {}",
            offenders.join("\n  ")
        );
    }

    /// THE PROOF THAT THE PROMPT THIS SET BUILDS IS THE PROMPT THE APP SENDS.
    ///
    /// The skill guide was wired into the selection runner because production
    /// attaches it and this measurement did not — but a wiring that silently
    /// matches NOTHING would look identical to no wiring at all, and the run
    /// takes twenty minutes to disprove. This is the cheap version: with the
    /// same store and the same tool budget the runner uses, the suite's own
    /// messages must reach real guidance.
    #[test]
    fn the_suite_messages_reach_the_skill_guides_production_would_attach() {
        let env = Env::setup().expect("the sandbox is set up");
        let memory = SharedMemory::in_memory();
        let catalog = selection_catalog(&env, &memory);
        let router = Router::new();
        let skills = tacet_skills::SkillStore::default_set();
        assert!(skills.count() > 0, "the bundled skill set is empty");

        let matched = selection_cases()
            .iter()
            .flat_map(|c| c.steps.iter())
            .filter(|step| {
                let selected: Vec<String> = router
                    .select(&step.message, &catalog)
                    .iter()
                    .map(|t| t.name().to_string())
                    .collect();
                skills.matching(&step.message, Some(&selected)).is_some()
            })
            .count();
        assert!(
            matched > 0,
            "no message in the English suite matches any bundled skill — the guide \
             injection is wired but inert, which measures the same prompt as not \
             wiring it at all"
        );
    }

    /// NO MESSAGE THIS PROJECT MEASURES MAY LEAVE TWO SKILLS TIED.
    ///
    /// WHY IT LIVES HERE AND NOT IN `tacet-skills`. Exactly ONE skill is
    /// injected per turn, and `SkillStore::matching` breaks a tie with `>` — the
    /// first skill in `PACKAGE_FILES` order wins, silently, an order nobody
    /// chose. `store.rs` has a probe table for that, one message per skill; what
    /// it CANNOT have is the messages this suite uses, because `tacet-skills`
    /// does not depend on `tacet-eval` (the edge runs the other way). Hand-copying
    /// them into `store.rs` would rot the first time a case is edited — the exact
    /// drift this project writes tests against. Here the three case lists are
    /// iterated programmatically and cannot go stale.
    ///
    /// TOOLS ARE `None` ON PURPOSE: a collision hidden because one of the pair is
    /// addon-gated today is still a collision the day the addon is installed.
    ///
    /// MEASURED WHEN IT WAS WRITTEN (4 Sep 2026): 268 messages across the
    /// English selection suite, the Turkish one and `case::all()`. It found ONE
    /// tie on the first run — `run-code` and `write-code` both scoring 13 on
    /// "Write me a python script that finds prime numbers, and save it as a
    /// file." (`prime numbers` against `python script`) — which is why those two
    /// files are now the single `code` skill. That collision is exactly the class
    /// this test exists for: both guides were reasonable, the message is
    /// genuinely ambiguous, and the winner was decided by list order.
    #[test]
    fn no_suite_message_leaves_two_skills_tied() {
        let skills = tacet_skills::SkillStore::default_set();
        let messages: Vec<String> = selection_cases()
            .iter()
            .chain(turkish_selection_cases().iter())
            .flat_map(|c| c.steps.iter().map(|s| s.message.clone()))
            .chain(crate::case::all().iter().map(|c| c.input.clone()))
            .collect();
        assert!(
            messages.len() > 100,
            "the suites should supply hundreds of messages, got {}",
            messages.len()
        );

        let mut ties: Vec<String> = Vec::new();
        for message in &messages {
            let lowered = tacet_skills::lowercase(message);
            let mut scored: Vec<(&str, usize)> = skills
                .all()
                .map(|s| (s.name.as_str(), tacet_skills::score(&lowered, &s.triggers)))
                .filter(|(_, p)| *p > 0)
                .collect();
            scored.sort_by_key(|s| std::cmp::Reverse(s.1));
            // ONLY THE PAIR AT THE FRONT MATTERS: two losers tied at 5 change
            // nothing, the guide is already decided. A tie at the TOP is the one
            // that hands the choice to `PACKAGE_FILES` order.
            if let [(first, top), (second, next), ..] = scored.as_slice()
                && top == next
            {
                ties.push(format!(
                    "{message:?}: {first} and {second} both score {top}"
                ));
            }
        }
        assert!(
            ties.is_empty(),
            "a tie is an order-dependent choice of guide:\n  {}",
            ties.join("\n  ")
        );
    }

    /// THE PROOF THAT THE LANGUAGE GATE MEASURES ANYTHING, written as the pair
    /// of cases the old implementation got wrong in BOTH directions. Without
    /// this, the gate can silently return to a substring search and every
    /// Turkish case will go on reporting a perfect answer rate.
    #[test]
    fn an_answer_in_the_wrong_language_fails_the_language_gate() {
        let turkish_step = SelectionStep::new("m", None).with_language(Language::Turkish);
        let english_step = SelectionStep::new("m", None).with_language(Language::English);

        // The exact sentence the old check passed: "have" contains "ve",
        // "about" contains "bu".
        assert!(
            !check_answer_quality(&turkish_step, "I have the answer about it: 1000.", &[], &[]),
            "an English sentence must not satisfy the Turkish gate"
        );
        // And the reverse: "için" contains "in".
        assert!(
            !check_answer_quality(&english_step, "Bunun için sonuç 1000 çıkıyor.", &[], &[]),
            "a Turkish sentence must not satisfy the English gate"
        );

        // The gates still accept what they are for.
        assert!(check_answer_quality(
            &turkish_step,
            "Sonuç 1000 olarak çıktı.",
            &[],
            &[]
        ));
        assert!(check_answer_quality(
            &english_step,
            "The result is 1000.",
            &[],
            &[]
        ));
    }

    /// A NUMBER IS NOT A LANGUAGE DEFECT. "1000." is the shortest correct
    /// answer to an arithmetic question and the old gate failed it, so a case
    /// could lose its answer point for being brief.
    #[test]
    fn an_answer_with_no_words_is_not_judged_for_language() {
        for lang in [Language::Turkish, Language::English] {
            let step = SelectionStep::new("m", None).with_language(lang);
            assert!(check_answer_quality(&step, "1000.", &[], &[]));
            assert!(check_answer_quality(&step, "", &[], &[]));
        }
    }

    /// A CAPITALISED ENGLISH SENTENCE IS NOT TURKISH. `I` and `İ` look alike;
    /// only the dotted one is Turkish, and treating the ASCII letter as proof
    /// would pass every sentence that starts with "I".
    #[test]
    fn the_ascii_capital_i_is_not_a_turkish_letter() {
        let turkish = SelectionStep::new("m", None).with_language(Language::Turkish);
        assert!(!check_answer_quality(
            &turkish,
            "I READ THE REPORT.",
            &[],
            &[]
        ));
    }
}

#[cfg(test)]
mod ordering_probe {
    use super::*;

    /// A DIAGNOSTIC, not a check: prints the tool list the router hands the
    /// model for every step of the English suite, so two builds can be diffed
    /// against each other without spending a model run.
    ///
    /// WHAT IT MEASURED THE DAY IT WAS WRITTEN: a router change that fixed
    /// three real false positives moved the ordering of NINETEEN of the
    /// forty-two steps, while the suite score moved by three cases. Almost
    /// half the suite is one swap away from a different prompt — so a score
    /// difference of a case or two, on this instrument, says more about
    /// ordering luck than about the change. Run it before believing a small
    /// delta.
    ///
    /// `cargo test -p tacet-eval print_ordering -- --ignored --nocapture`
    #[test]
    #[ignore = "diagnostic: prints, asserts nothing"]
    fn print_ordering() {
        let env = Env::setup().unwrap();
        let memory = SharedMemory::in_memory();
        let catalog = selection_catalog(&env, &memory);
        let router = Router::new();
        let messages: Vec<String> = selection_cases()
            .iter()
            .flat_map(|c| c.steps.iter().map(|s| format!("{}|{}", c.name, s.message)))
            .collect();
        for entry in &messages {
            let m = entry.split_once('|').map(|(_, m)| m).unwrap_or(entry);
            let names: Vec<String> = router
                .select(m, &catalog)
                .iter()
                .map(|t| t.name().to_string())
                .collect();
            println!("{entry}\n   {}", names.join(", "));
        }
    }
}

#[cfg(test)]
mod progress {
    use super::*;

    /// The unit the reader converts in their head if it is missing.
    ///
    /// THE 3600 CASE IS THE ONE THAT MATTERS and it is why this is not just
    /// `secs / 60`: the run that made this function necessary passed an hour, and
    /// "5927s left" is a number nobody reads as "an hour and a half".
    #[test]
    fn a_duration_is_readable_at_the_scale_this_suite_actually_reaches() {
        assert_eq!(human_duration(0.0), "0s");
        assert_eq!(human_duration(59.4), "59s");
        assert_eq!(human_duration(60.0), "1m00s");
        assert_eq!(human_duration(95.0), "1m35s");
        assert_eq!(human_duration(3600.0), "60m00s");
        assert_eq!(human_duration(5927.0), "98m47s");
        // A negative can arrive from the projection when a case finishes faster
        // than the running mean; it must not underflow into a giant number.
        assert_eq!(human_duration(-5.0), "0s");
    }

    /// The projection is arithmetic, so it is pinned as arithmetic: after 10 of
    /// 115 cases in 100 s, the remaining 105 are 1050 s at the same mean.
    ///
    /// PINNED BECAUSE THE OFF-BY-ONE HERE IS SILENT: using `total - done` where
    /// `done` is the INDEX rather than the COUNT would over-report by one case
    /// forever, and nothing on screen would look wrong.
    #[test]
    fn the_projection_is_the_running_mean_over_what_is_left() {
        let done = 10.0_f64;
        let total = 115.0_f64;
        let elapsed = 100.0_f64;
        let per_case = elapsed / done;
        let left = per_case * (total - done);

        assert!((per_case - 10.0).abs() < 1e-9, "{per_case}");
        assert!((left - 1050.0).abs() < 1e-9, "{left}");
        assert_eq!(human_duration(left), "17m30s");
    }
}

#[cfg(test)]
mod trace_format {
    use super::*;

    /// THE TURKISH HALF OF THIS SUITE IS WHY THIS IS A CHARACTER CUT.
    ///
    /// `&message[..56]` would panic the moment a 56-byte boundary landed inside
    /// a `ç` or a `ğ`, and `turkish_selection_cases()` is 65 of the cases this
    /// trace runs over — so the crash would not be an edge case, it would be
    /// most Tuesdays. Asserted with a string whose byte length and character
    /// count differ, which is the only shape that can catch it.
    #[test]
    fn a_message_is_cut_by_character_so_turkish_does_not_panic() {
        let turkish = "Bugünden 14 Mart'a kaç gün kaldı, çünkü şubat çekişmeli ölçüm gerektirir";
        assert!(
            turkish.len() > turkish.chars().count(),
            "the fixture must be multi-byte or it measures nothing"
        );

        let cut = truncate_for_trace(turkish);
        assert_eq!(cut.chars().count(), 57, "56 characters plus the ellipsis");
        assert!(cut.ends_with('…'));
        assert!(cut.starts_with("Bugünden 14 Mart'a"));
    }

    /// A short message is passed through whole and gains no ellipsis — otherwise
    /// every line would claim to be truncated.
    #[test]
    fn a_short_message_is_left_alone() {
        assert_eq!(truncate_for_trace("Add 25 and 18"), "Add 25 and 18");
        assert!(!truncate_for_trace("Add 25 and 18").contains('…'));
    }

    /// ONE EVENT IS ONE LINE, and a message carrying a newline would break that
    /// silently — the trace would still print, just misaligned, which is the
    /// kind of defect nobody files.
    #[test]
    fn control_characters_cannot_break_the_one_line_shape() {
        let cut = truncate_for_trace("first line\nsecond\tline\r\n");
        assert!(!cut.contains('\n') && !cut.contains('\t') && !cut.contains('\r'));
        assert_eq!(cut, "first line second line  ");
    }
}

/// THE LANGUAGE TABLE IS ONLY A CLAIM IF THE LISTS ARE DISJOINT.
///
/// The gate this file already had to fix once was vacuous for a reason worth not
/// repeating: "için" contains "in", so every Turkish answer satisfied the
/// English claim and a check no input could fail was reported as passing.
/// Widening from two languages to seven brings the same failure back in a new
/// shape — Spanish and French both write "la", German and English both write
/// "in" — and a shared word makes both claims meaningless rather than one.
#[cfg(test)]
mod language_table {
    use super::*;

    #[test]
    fn no_two_languages_claim_the_same_function_word() {
        for a in Language::ALL {
            for b in Language::ALL {
                if a == b {
                    continue;
                }
                let (wa, wb) = (a.marks().words, b.marks().words);
                let shared: Vec<&&str> = wa.iter().filter(|w| wb.contains(w)).collect();
                assert!(
                    shared.is_empty(),
                    "{a:?} and {b:?} both claim {shared:?}; a word two languages write \
proves neither"
                );
            }
        }
    }

    /// The same rule for the letters. `ç` is written by Turkish AND French, so
    /// it cannot be proof of either — this test is what caught that, and why
    /// the French entry leans on `à è ù â ê î ô û ë ï œ` instead.
    #[test]
    fn no_two_languages_claim_the_same_letter() {
        for a in Language::ALL {
            for b in Language::ALL {
                if a == b {
                    continue;
                }
                let shared: Vec<char> = a
                    .marks()
                    .letters
                    .chars()
                    .filter(|c| b.marks().letters.contains(*c))
                    .collect();
                assert!(
                    shared.is_empty(),
                    "{a:?} and {b:?} both write {shared:?}; a letter two languages \
share is proof of neither"
                );
            }
        }
    }

    /// EVERY CODE IS DISTINCT AND ROUND-TRIPS. A benchmark file names its
    /// language by code, so a duplicate or a typo silently selects the wrong
    /// judge.
    #[test]
    fn every_language_has_a_distinct_code_that_parses_back() {
        let mut codes: Vec<&str> = Language::ALL.iter().map(|l| l.marks().code).collect();
        let before = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), before, "two languages share a code");
        for l in Language::ALL {
            assert_eq!(Language::from_code(l.marks().code), Some(l));
            assert_eq!(Language::from_code(&l.marks().code.to_uppercase()), Some(l));
        }
        assert_eq!(Language::from_code("klingon"), None);
    }

    /// NOT VACUOUS IN EITHER DIRECTION: a real sentence in each language must
    /// satisfy its own claim, and must NOT satisfy any other's.
    #[test]
    fn each_language_recognises_itself_and_no_other() {
        let samples = [
            (Language::English, "The file has 12 rows and no header."),
            (Language::Turkish, "Dosyada 12 satır var, başlık yok."),
            (
                Language::Spanish,
                "El archivo tiene 12 filas y ningún título.",
            ),
            (Language::French, "Les fichiers contiennent 12 lignes."),
            (
                Language::German,
                "Die Datei hat 12 Zeilen und keine Kopfzeile.",
            ),
            (Language::Russian, "В файле 12 строк и нет заголовка."),
            (Language::Chinese, "文件有12行，没有标题。"),
        ];
        for (lang, text) in samples {
            assert!(speaks(lang, text), "{lang:?} must recognise {text:?}");
            for other in Language::ALL {
                if other == lang {
                    continue;
                }
                assert!(
                    !speaks(other, text),
                    "{text:?} is {lang:?}, but {other:?} claimed it too"
                );
            }
        }
    }

    /// A TEXT WITH NO WORDS IN IT IS NOT JUDGED — kept from the original, and
    /// still the reason a correct short answer is not counted as a language
    /// defect.
    #[test]
    fn an_answer_with_nothing_to_read_is_not_judged() {
        for lang in Language::ALL {
            assert!(speaks(lang, "1000."));
            assert!(speaks(lang, "  "));
        }
    }
}

/// THE EXCLUSION LIST CANNOT QUIETLY GROW.
///
/// `BENCHED_SEPARATELY` is a hole in the rule that eval and the shell see the
/// same catalog, and a hole that anyone can widen is not an exception, it is a
/// policy. Two entries, both named, both with a benchmark file behind them.
#[cfg(test)]
mod suite_coverage {
    use super::*;

    #[test]
    fn the_suite_carries_every_tool_it_is_shown() {
        assert_eq!(
            BENCHED_SEPARATELY.len(),
            2,
            "a third tool has been excluded from the suite. That is allowed only when it \
has a benchmark of its own that measures something the suite cannot — say so here, and \
add the file, or give it two suite cases like every other tool."
        );
        for name in BENCHED_SEPARATELY {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../benchmarks/tasks")
                .join(format!("{name}.json"));
            assert!(
                path.exists(),
                "{name} is excluded from the suite on the promise of a benchmark, and \
{} does not exist",
                path.display()
            );
        }
    }
}
