//
//  IntentPicker.swift
//  Tacet
//
//  Intent → tool profile routing (spec §7.3.1). Lifted out of `ModelService` AS
//  IT WAS: the list contents, the ordering and the thresholds are identical.
//
//  THE ONLY DIFFERENCE IS STRUCTURAL: the decision no longer READS instance state,
//  it takes its inputs as parameters (`Input`) and returns the two flags it used to
//  write (`hideLocalSearch`, `hasContactSignal`) in an `Outcome`. That makes it
//  deterministically testable without a model, a device or a session — all
//  verification used to lean on the on-device eval, whose noise floor is high.
//

import Foundation

enum IntentPicker {

    /// ALL the inputs of the decision. The functions read nothing outside this.
    struct Input {
        let question: String
        /// The profile currently installed — the input of the stickiness decision.
        let currentProfile: ModelService.Profile
        /// Is there a workable document, attached or just produced?
        let hasDocument: Bool
        let searchAvailable: Bool
        let connectionAvailable: Bool
        /// The selected connection's name — its presence in the sentence is a connection
        /// signal on its own.
        let connectionName: String
        /// Did a search chip drop in the previous turn (the web-search §5.4 signal)?
        let previousTurnSearch: Bool
        /// Did an MCP chip drop in the previous turn (the mcp §5.4 signal)?
        let previousTurnConnection: Bool
    }

    /// ALL the outputs of the decision. Alongside the profile, the two flags that shape
    /// the everyday tool set come back from here too — they used to be side effects of the
    /// function, which is why the decision could not be tested.
    struct Outcome: Equatable {
        let profile: ModelService.Profile
        /// Should the Spotlight search be hidden in this turn?
        let hideLocalSearch: Bool
        /// Is this turn an address-book question — it decides the Contact ↔ web search swap.
        let hasContactSignal: Bool
    }

    // MARK: - The primary selection

    /// Intent classification (spec §7.3.1). To avoid needlessly rebuilding the session it
    /// DOES NOT CHANGE the current profile if it can serve the request — it switches only
    /// when a tool absent from that profile is needed. Both profiles share
    /// Calendar/Contact/Calculate/Time.
    static func select(_ input: Input) -> Outcome {
        let s = input.question.lowercased()

        // An EXPLICIT web intent comes before everything: if the user said "search the
        // internet", their intent is not debatable. If search is on, straight to the search
        // profile; if it is OFF, drop the local search tool from the session so the model
        // cannot call it as a substitute and say "I searched the internet" (the lie is
        // blocked by code, not by an instruction).
        let explicitWeb = explicitWebTraces.contains(where: s.contains)
        let hideLocalSearch = explicitWeb && !input.searchAvailable
        // This decides whether Contact or web search stands in the everyday set; it is
        // refreshed every turn, because the user can move from topic to topic.
        //
        // THESE THREE COMPUTATIONS HAPPEN BEFORE THE DOCUMENT LOCK and that is a fix
        // (audit P2-1): the lock used to be the FIRST line of the function, so with a
        // document attached `hasContactSignal` was never computed and the everyday set's
        // Contact ↔ web swap never worked in that session.
        let hasContactSignal = contactTraces.contains(where: s.contains)

        func outcome(_ profile: ModelService.Profile) -> Outcome {
            Outcome(profile: profile,
                    hideLocalSearch: hideLocalSearch,
                    hasContactSignal: hasContactSignal)
        }

        // If there is an attached document or a file just produced, a follow-up request
        // must stay in the document profile — a sentence like "show it as a table" may not
        // name a format. The lock IS NO LONGER UNCONDITIONAL: if there is a STRONG and
        // EXPLICIT non-document intent an escape is granted, because an absolute lock made
        // the contact/code tools completely unreachable in that session ("what is Ali's
        // number" → a tool-less turn → "I couldn't find it").
        //
        // The escape is NARROW: only (a) while there is NO document signal and (b) while a
        // non-document signal is EXPLICITLY declared. "show this as a table" and "add the
        // saturday row" do not escape — the first carries no escape trace at all, and the
        // second already carries a document signal through "row".
        if input.hasDocument {
            let documentSignal = documentTraces.contains(where: s.contains)
            if !documentSignal {
                if hasContactSignal { return outcome(.everyday) }
                if explicitWeb, input.searchAvailable { return outcome(.search) }
                if lockEscapeTraces.contains(where: s.contains) { return outcome(.everyday) }
            }
            return outcome(.document)
        }

        if explicitWeb, input.searchAvailable { return outcome(.search) }
        // Tools SPECIFIC to the everyday profile (Reminder, Search) — triggers in 8
        // languages.
        //
        // It is checked BEFORE the off-device profiles and that is deliberate: the sentence
        // "open an issue on the server from my meeting notes" carries both an everyday and a
        // connection signal; spec §5.4's two-stage flow gathers the data on the device
        // first and moves to the connection on the next turn. In the reverse order, going
        // outside in a single step would be attempted.
        // A MULTI-TOOL SENTENCE: two signals at once. "Export my calendar to a file", "dump
        // my reminders into Excel", "Remind me and save it as a PDF" — all of them carry
        // both an everyday and a document trace. Measured (the routing pair, case 26):
        // because everyday was checked first, the document profile COULD NOT BE SELECTED,
        // create_document was never in the session and the work spread over two turns; on
        // the second turn the profile change dropped the data.
        //
        // The document set ALREADY COVERS everyday's personal tools (calendar, reminder,
        // search + calculate/time); what is specific to everyday is only Contact, Code and
        // web. So when the two signals collide the document profile is strictly superior:
        // it can do both jobs in ONE turn. The only exception is the address book —
        // ContactTool is not in the document set, so a contact signal holds everyday.
        let everydaySignal = everydayTraces.contains(where: s.contains)
        let documentSignal = documentTraces.contains(where: s.contains)
        if documentSignal, !hasContactSignal { return outcome(.document) }
        if everydaySignal { return outcome(.everyday) }
        if documentSignal { return outcome(.document) }
        // Connection: only IF there is an available tool (mcp §5.4).
        if input.connectionAvailable, connectionSignal(s, input: input) { return outcome(.connection) }
        // Search: only if a server is configured AND on (web-search §5.4). If it is off the
        // profile is never selected, the tool is never visible to the model and today's
        // honest "there is no such information on your device" answer continues unchanged.
        // THE CALCULATION ESCAPE (the second half of audit cluster 1). The search profile no
        // longer has `calculate`; so a pure arithmetic question arriving while the session
        // is STUCK to search would be left tool-less and the model would compute it in its
        // head — bringing back the very fault we fixed, through another door.
        //
        // The escape is granted ONLY on stickiness: if the sentence ITSELF carries an
        // explicit search trace ("euro", "price", "exchange rate", "league table") the
        // question is a live-data question and it STAYS in search — the escape does not
        // rescue it, and must not. So "how many lira is a euro" always stays in search,
        // while "and now add 250 and 890", arriving after a search turn, is resolved in
        // everyday.
        let explicitSearchTrace = searchTraces.contains(where: s.contains)
        if input.searchAvailable, searchSignal(s, input: input) {
            if !explicitSearchTrace, calcIntent(s) { return outcome(.everyday) }
            return outcome(.search)
        }
        // Otherwise keep the current profile — but the off-device profiles are sticky only
        // while they are still available. If the user turned the server off or deleted the
        // connection in the meantime, the session falls back to everyday.
        switch input.currentProfile {
        case .search:
            // The arithmetic escape from a sticky search applies here too: if the previous
            // turn dropped no globe chip, `searchSignal` returns false and the flow arrives
            // here; had the escape stood in a single place, the session would still be left
            // without a calculator.
            if !explicitSearchTrace, calcIntent(s) { return outcome(.everyday) }
            return outcome(input.searchAvailable ? .search : .everyday)
        case .connection: return outcome(input.connectionAvailable ? .connection : .everyday)
        case .everyday, .document: return outcome(input.currentProfile)
        }
    }

    // MARK: - In-turn profile recovery (audit P1-2)

    /// The SECOND profile to try when the deterministic picker was wrong.
    ///
    /// The problem was this: `select` returns a single profile, and when it was wrong the
    /// required tool was NOT IN that session at all. The model reported it as a missing
    /// capability ("I can't do that") and the turn ended without a tool — a silent
    /// capability gap, an invisible fault for the user.
    ///
    /// A pure function: it reads from the signal lists, writes no state and needs no model.
    ///
    /// `hasContactSignal` IS A PARAMETER: the very value computed in the first selection is
    /// passed in (it used to be read from the same instance field).
    static func secondPass(_ input: Input,
                           first: ModelService.Profile,
                           hasContactSignal: Bool) -> ModelService.Profile? {
        let s = input.question.lowercased()
        // The candidate order follows SIGNAL STRENGTH: profiles with a trace in the
        // sentence come first.
        var candidates: [ModelService.Profile] = []
        if documentTraces.contains(where: s.contains)          { candidates.append(.document) }
        if everydayTraces.contains(where: s.contains)          { candidates.append(.everyday) }
        if hasContactSignal                                    { candidates.append(.everyday) }
        if input.searchAvailable, searchSignal(s, input: input)         { candidates.append(.search) }
        if input.connectionAvailable, connectionSignal(s, input: input) { candidates.append(.connection) }
        // If there is no second signal at all, the widest tool set is the last resort. With
        // the document lock in place it falls to document rather than everyday: taking the
        // user's attached document off the table while it is right there would open a new gap.
        candidates.append(input.hasDocument ? .document : .everyday)
        return candidates.first { $0 != first }
    }

    // MARK: - Signals

    /// The connection signal: the connection's own name, the word "server", or an MCP chip
    /// dropped in the previous turn.
    static func connectionSignal(_ s: String, input: Input) -> Bool {
        if input.previousTurnConnection { return true }
        let name = input.connectionName.lowercased()
        if !name.isEmpty, s.contains(name) { return true }
        return connectionTraces.contains(where: s.contains)
    }

    /// The search signal: current-information patterns (weather/exchange rate/news/price/
    /// score) and general-knowledge questions of the "what is / who is" kind.
    static func searchSignal(_ s: String, input: Input) -> Bool {
        if input.previousTurnSearch { return true }
        return searchTraces.contains(where: s.contains)
    }

    /// An EXPLICIT arithmetic intent — the single criterion of the escape from a sticky
    /// search session to everyday (audit cluster 1). A pure function: it reads no state
    /// and needs no model.
    ///
    /// TWO conditions are required at once — the word ALONE IS NOT ENOUGH, A DIGIT is
    /// needed too. A word list on its own occurs inside everyday words like "bölge",
    /// "bölüm", "toplantı", "çarpıcı" and would pull a live-data question out of search;
    /// the digit requirement cuts almost all of those false positives.
    ///
    /// The list is deliberately NARROW and DOES NOT INTERSECT the live-data vocabulary:
    /// "fiyat", "kaç para", "kaç tl", "kur " are NOT here — those are `searchTraces`'s job
    /// and must stay there. The TRAILING space in the lookup is deliberate: traces like
    /// "…'e böl" have to match a word END and must also match at the end of a sentence.
    /// Written without the space, "e böl" would catch "e bölge" inside "bu bölgede".
    static func calcIntent(_ s: String) -> Bool {
        guard s.contains(where: \.isNumber) else { return false }
        let padded = s + " "
        return calcTraces.contains(where: padded.contains)
    }

    // MARK: - Trace lists
    //
    // THE CONTENT of these lists is institutional memory: every line is the counterpart of
    // a measured regression. The comments here travelled with the lists.
    //
    // THE ENTRIES ARE MULTILINGUAL USER-INPUT DATA, not code identifiers: they match what
    // the user types (tr/en/zh/ja/es/de/fr/ko/pt) and must stay in their own languages,
    // exactly like a keyword list. Translating them would silently disable routing for
    // every language but English.

    /// Address-book intent. Needed because web search enters the everyday set: this
    /// decides which of the two is taken into the session.
    static let contactTraces = [
        "kişi", "kisi", "rehber", "numara", "telefonu", "telefon numar",
        "mail adresi", "e-posta adres", "eposta adres", "adresi ne",
        "contact", "phone number", "email address", "in numarası",
    ]

    /// The traces that escape the document lock (audit P2-1). The address book and
    /// explicit web are computed separately; this holds only the single capability that
    /// has NO counterpart in the document set: running code.
    ///
    /// The list is deliberately very NARROW and always MULTI-WORD/distinctive: a false
    /// positive on the escape means refusing to work on the user's attached document — a
    /// worse fault than the lock itself. There is no bare "kod" (it occurs inside "kodu",
    /// "barkod", "kodlanmış"); no bare "ajan".
    static let lockEscapeTraces = [
        "kod çalıştır", "kodu çalıştır", "javascript", "js ile hesapla",     // tr — run_code
        "run this code", "execute this code", "in javascript",               // en
    ]

    /// Arithmetic traces. All of them are either a verb or a calculation-specific pattern;
    /// no live-value name appears.
    ///
    /// TRAPS (all measured and eliminated): bare "böl" is forbidden — it occurs inside
    /// "bölge/bölüm", so only its inflected forms and the "…e böl " shape are taken.
    /// "eksi" is forbidden — it occurs inside "eksik". "topla" on its own occurs inside
    /// "toplantı", but the digit requirement already cuts that ("yarınki toplantım kaçta"
    /// has no digit).
    static let calcTraces = [
        "hesapla", "hesab", "topla", "toplamı", "çarp", "carp",             // tr
        "böler", "bölers", "böleceğ", "bölün", "bölüp",                     // tr — çekimli
        "e böl ", "a böl ", "ye böl", "ya böl", "i böl ", "ı böl ",         // tr — "24'e böl"
        "yüzde", "yuzde", "kdv", "kaç eder", "kac eder", "kaç yapar",       // tr
        "kaç kalır", "farkı ne", "kaçtan",                                  // tr
        "calculate", "compute", "multiply", "divide", "subtract",           // en
        "sum of", "percent of", "times what",                               // en
    ]

    /// Reminder/search intent (the everyday profile) — tr/en/zh/ja/es/de/fr/ko/pt.
    static let everydayTraces = [
        "hatırlat", "hatirlat", "anımsat", "notlarım", "notlarda",          // tr
        "dosyalarımda", "dosyam var mı", "dosyalarım",                      // tr — LOCAL FILE search
        "remind", "reminder", "my note", "notes", "search my",              // en
        // English personal-data patterns: because the everyday traces are checked BEFORE
        // the search traces, "What is John's phone number?" is caught here and ContactTool
        // stays in the session (otherwise it escaped to web search).
        "phone number", "'s number", "contact", "email address",
        "my calendar", "my schedule", "my files",
        "提醒", "备忘", "笔记", "搜索",                                          // zh
        "リマインド", "思い出させ", "メモ", "検索",                                // ja
        "recuérda", "recordar", "recordatorio", "mis notas", "buscar",      // es
        "erinner", "notiz", "suche", "meine noti",                          // de
        "rappelle", "rappel", "mes notes", "cherche",                       // fr
        "알림", "리마인더", "메모", "검색",                                       // ko
        "lembre", "lembrete", "minhas notas", "procur",                     // pt
    ]

    /// Document/file intent (the document profile) — format names + noun-verbs in 8
    /// languages. The "site/html/sayfa/landing" traces are code-spec §7: the .html format
    /// adds no tool, `create_document` is already in the document profile.
    ///
    /// "site" is DELIBERATELY not bare: the lookup is by substring and a bare "site" would
    /// occur inside words like "üniversite(si)/kapasite/opposite" and lock the question
    /// into document ("Boğaziçi Üniversitesi nedir?" could not go to search) — the same
    /// trap as the trailing space in the "kur " trace. " site" looks for a word start;
    /// "site yap/kur" covers the start of a sentence, "websit" the joined spelling.
    static let documentTraces = [
        "excel", "xlsx", "pdf", "word", "docx", "markdown", ".md",          // language-neutral
        "html", " site", "site yap", "site kur", "websit", "landing",       // web page (code-spec §7)
        "web page",                                                         // en — GenerateDocumentIntent.promptWord(.html)
        // A BARE "tablo" IS DELIBERATELY ABSENT — and so are its counterparts
        // (table/tabla/tabelle/tableau/表格/표). "Make a table" is a DISPLAY request, not a
        // file request: the user wants to see it on screen and, if they want, converts it to
        // Excel with the chat table's own download button. As long as these stood here every
        // table request escaped to the document profile, create_document ran, and the model
        // invented a file name instead of content and produced a pointless .xlsx.
        // File intent is already caught by separate words ("excel", "dosya", "indir").
        "belge", "dosya", "indir", "rapor", "döküm", "dök",                 // tr
        "sayfa",                                                            // tr — web sayfası
        // Table/document STRUCTURE words: without them "add the Wednesday meatballs row"
        // stayed in everyday and escaped to CalendarTool (edit_document was never in the
        // session).
        // Their English counterparts are NOT BARE, they carry a leading space — the same
        // substring trap as " site" and "kur ". A bare "row" occurs inside TOMORROW:
        // "What's the weather tomorrow?" fell into the document profile and web_search was
        // never in the session (measured). The same holds for "cell" ↔ "excellent",
        // "arrow", "borrow", "narrow", "grow".
        "satır", "sütun", "kolon", "hücre", " row", " column", " cell",
        // SAVING as a note — a production request. A bare "not"/"kaydet" is DELIBERATELY
        // absent: the same substring trap as the "kur " trace ("not" → "nota/nokta/motor"
        // would give plenty of false positives).
        "nota kaydet", "nota yaz", "not olarak", "as a note", "save this as",
        "document", "file", "spreadsheet", "report", "export", "download",  // en
        "文档", "文件", "报告", "列表",                                          // zh
        "ドキュメント", "ファイル", "レポート", "リスト",                           // ja
        "documento", "archivo", "informe", "hoja de",                       // es
        "dokument", "datei", "bericht",                                     // de
        "fichier", "rapport", "feuille",                                    // fr
        "문서", "파일", "보고서", "목록",                                        // ko
        "arquivo", "tabela", "relatório", "planilha",                       // pt
    ]

    /// Current/world-knowledge intent (the search profile) — web-search §5.4.
    ///
    /// Kept deliberately NARROW: a false positive removes the personal-data tools from that
    /// turn and leads to "I couldn't set the reminder". General-knowledge patterns ("what
    /// is", "who is") live here, personal-content patterns in the everyday list.
    /// The patterns in which the user asks for the web EXPLICITLY. Not a topic guess but a
    /// declaration of intent — which is why it is evaluated ahead of every other signal.
    static let explicitWebTraces = [
        "internette", "internetten", "internete", "web'de", "webde", "web de",
        "webte", "web'te", "internette ara", "google", "googlela", "çevrimiçi",
        "on the web", "on the internet", "search online", "look it up online",
        "search the internet", "google it",
        // The remaining seven languages: while the neighbouring `searchTraces` covers nine
        // languages, the highest-priority signal was tr+en only, so an explicit web request
        // from a German- or Japanese-speaking user never matched.
        "im internet", "im netz", "online suchen", "im web",
        "en internet", "en la web", "buscar en internet", "buscar en línea",
        "sur internet", "sur le web", "cherche en ligne", "recherche en ligne",
        "ネットで", "インターネットで", "ウェブで", "オンラインで",
        "인터넷에서", "웹에서", "온라인으로", "인터넷으로",
        "na internet", "na web", "pesquise online", "buscar na internet",
        "在网上", "上网查", "网上搜", "在线搜索",
    ]

    /// Patterns of world information that CANNOT BE FOUND on the user's device.
    /// The list is incomplete by its nature.
    static let searchTraces = [
        // Transport / timetable / venue — added after the "ferry times" case.
        "vapur", "feribot", "sefer saat", "tarife", "kalkış saat", "otobüs saat",
        "metro saat", "tren saat", "uçuş", "kaçta kalk", "kaçta açıl", "açık mı",
        "nasıl gidilir", "ne kadar sürer", "kaç durak",
        // Daily public information — added after the "prayer times" case. The model DOES
        // NOT KNOW these and made them up when asked (for Istanbul it gave round, entirely
        // imaginary times such as 05:00/12:00/15:00).
        "namaz vak", "namaz saat", "ezan", "imsak", "iftar", "sahur",
        "güneş doğ", "güneş bat", "gün doğ", "gün bat",
        "nöbetçi ecz", "eczane nöbet", "vizite", "randevu saat",
        "resmi tatil", "tatil mi", "maç saat", "kaçta başlıyor",
        "hava durumu", "hava nasıl", "hava kaç", "derece", "yağmur", "kar yağ",
        "sıcaklık", "dolar", "euro", "kur ", "borsa", "endeks", "bist",
        "gram altın", "kaç tl",
        "haber", "ne oldu", "fiyat", "kaç para", "kaça", "maç", "skor",
        "puan durumu", "kimler kazandı", "kim kazandı",
        "nedir", "kimdir", "ne demek", "kim oldu", "son dakika", "web'de",   // tr
        "weather", "forecast", "temperature", "rain", "exchange rate",
        "stock", "news", "price", "how much is", "score", "who won",
        "search the web",                                                    // en
        // A BARE "what is"/"who is" IS DELIBERATELY ABSENT: in English almost every
        // question starts with that pattern, and it threw PERSONAL data questions such as
        // "What is John's phone number?" into the search profile and dropped ContactTool
        // from the session. Only the narrowed forms are taken.
        "what is the price", "what is the weather", "what is the exchange",
        "who is the president", "who is the ceo",
        "天气", "汇率", "新闻", "价格", "比分", "是什么", "是谁",                  // zh
        "天気", "為替", "ニュース", "値段", "とは", "誰",                         // ja
        "clima", "tiempo", "noticias", "precio", "cuánto cuesta", "quién es",// es
        "wetter", "nachrichten", "preis", "wechselkurs", "wer ist",          // de
        "météo", "actualités", "prix", "taux de change", "qui est",          // fr
        "날씨", "환율", "뉴스", "가격", "누구",                                   // ko
        "clima", "notícias", "preço", "cotação", "quem é",                   // pt
    ]

    /// Connection intent (the connection profile) — mcp §5.4. The connection's OWN name is
    /// looked up separately in `connectionSignal`; this holds only the generic words.
    static let connectionTraces = [
        "sunucu", "sunucuma", "sunucuda", "bağlantı",                        // tr
        "server", "my server", "connection", "remote",                       // en
        "服务器", "远程",                                                      // zh
        "サーバー", "リモート",                                                 // ja
        "servidor", "remoto",                                                // es/pt
        "server", "entfernt",                                                // de
        "serveur", "distant",                                                // fr
        "서버", "원격",                                                        // ko
    ]
}
