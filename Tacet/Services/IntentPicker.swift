//
//  NiyetSecici.swift
//  Tacet
//
//  Niyet → araç profili yönlendirmesi (spec §7.3.1). `ModelServisi` içinden
//  OLDUĞU GİBİ çıkarıldı: liste içerikleri, sıralama ve eşikler bire bir aynı.
//
//  TEK FARK YAPIDA: karar artık örnek durumu OKUMAZ, girdileri parametre alır
//  (`Girdi`) ve yazdığı iki bayrağı (`yerelAramayiGizle`, `kisiSinyaliVar`)
//  döndürür (`Sonuc`). Böylece model, cihaz ya da oturum olmadan deterministik
//  test edilebilir — eskiden tüm doğrulama gürültü tabanı yüksek cihaz-üstü
//  eval'e yaslanıyordu.
//

import Foundation

enum IntentPicker {

    /// Kararın TÜM girdileri. Fonksiyonlar bunun dışında hiçbir şey okumaz.
    struct Input {
        let question: String
        /// Şu an kurulu olan profil — yapışkanlık kararının girdisi.
        let mevcutProfil: ModelService.Profile
        /// Ekli ya da az önce üretilmiş, üzerinde çalışılabilir bir belge var mı.
        let belgeVar: Bool
        let aramaKullanilabilir: Bool
        let baglantiKullanilabilir: Bool
        /// Seçili bağlantının adı — cümlede geçmesi tek başına bağlantı sinyalidir.
        let connectionName: String
        /// Önceki turda arama çipi düştü mü (web-arama §5.4 sinyali).
        let oncekiTurArama: Bool
        /// Önceki turda MCP çipi düştü mü (mcp §5.4 sinyali).
        let oncekiTurBaglanti: Bool
    }

    /// Kararın TÜM çıktıları. Profilin yanında, gündelik araç setini
    /// biçimlendiren iki bayrak da buradan döner — eskiden bunlar fonksiyonun
    /// yan etkisiydi ve bu yüzden karar test edilemiyordu.
    struct Outcome: Equatable {
        let profile: ModelService.Profile
        /// Bu turda Spotlight araması gizlensin mi.
        let yerelAramayiGizle: Bool
        /// Bu turda rehber sorusu mu — Kişi ↔ web araması takasını belirler.
        let kisiSinyaliVar: Bool
    }

    // MARK: - Birincil seçim

    /// Niyet sınıflandırması (spec §7.3.1). Gereksiz oturum yeniden kurmayı önlemek için
    /// mevcut profil isteği karşılayabiliyorsa DEĞİŞTİRMEZ — yalnızca o profilde olmayan
    /// bir araç gerektiğinde geçiş yapar. İki profil de Takvim/Kişi/Hesap/Zaman'ı paylaşır.
    static func select(_ input: Input) -> Outcome {
        let s = input.question.lowercased()

        // AÇIK web niyeti her şeyin önünde: kullanıcı "internette ara" dediyse
        // niyeti tartışmalı değil. Arama açıksa doğrudan arama profiline;
        // KAPALIYSA yerel arama aracını oturumdan düşür ki model onu yedek
        // olarak çağırıp "internette aradım" diyemesin (yalanı kod engeller,
        // talimat değil).
        let acikWeb = acikWebIzleri.contains(where: s.contains)
        let yerelAramayiGizle = acikWeb && !input.aramaKullanilabilir
        // Gündelik sette Kişi mi web araması mı duracağını bu belirler; her
        // turda tazelenir, çünkü kullanıcı konudan konuya geçebilir.
        //
        // BU ÜÇ HESAP BELGE KİLİDİNDEN ÖNCE YAPILIR ve bu bir düzeltmedir
        // (denetim P2-1): eskiden kilit fonksiyonun İLK satırıydı, dolayısıyla
        // belge ekliyken `kisiSinyaliVar` hiç hesaplanmıyor, gündelik setin
        // Kişi ↔ web takası da o oturumda hiç çalışmıyordu.
        let kisiSinyaliVar = kisiIzleri.contains(where: s.contains)

        func outcome(_ profile: ModelService.Profile) -> Outcome {
            Outcome(profile: profile,
                  yerelAramayiGizle: yerelAramayiGizle,
                  kisiSinyaliVar: kisiSinyaliVar)
        }

        // Ekli belge ya da az önce üretilmiş dosya varsa devam isteği belge
        // profilinde kalmalı — "onu tablo olarak göster" gibi cümlelerde biçim
        // adı geçmeyebilir. Kilit KOŞULSUZ DEĞİL artık: belge-dışı GÜÇLÜ ve
        // AÇIK bir niyet varsa kaçış verilir, çünkü mutlak kilit kişi/kod
        // araçlarını o oturumda tamamen erişilemez yapıyordu ("Ali'nin numarası
        // ne" → araçsız tur → "bulamadım").
        //
        // Kaçış DAR: yalnız (a) belge sinyali YOKKEN ve (b) belge-dışı sinyal
        // AÇIKÇA beyan edilmişken. "bunu tablo olarak göster" ve "cumartesi
        // satırını ekle" kaçmaz — birincisinde hiçbir kaçış izi yok, ikincisi
        // zaten "satır" ile belge sinyali taşıyor.
        if input.belgeVar {
            let belgeSinyali = belgeIzleri.contains(where: s.contains)
            if !belgeSinyali {
                if kisiSinyaliVar { return outcome(.gundelik) }
                if acikWeb, input.aramaKullanilabilir { return outcome(.search) }
                if kilitKacisIzleri.contains(where: s.contains) { return outcome(.gundelik) }
            }
            return outcome(.document)
        }

        if acikWeb, input.aramaKullanilabilir { return outcome(.search) }
        // Gündelik profile ÖZGÜ araçlar (Hatırlatıcı, Arama) — 8 dilde tetikleyiciler.
        //
        // Cihaz DIŞI profillerden ÖNCE bakılır ve bu bilinçlidir: "toplantı
        // notlarımı sunucuya issue aç" cümlesi hem gündelik hem bağlantı
        // sinyali taşır; spec §5.4'ün iki aşamalı akışı önce veriyi cihazda
        // toplar, sonraki tur bağlantıya geçer. Ters sırada tek adımda dışarı
        // çıkma denenirdi.
        // ÇOK ARAÇLI CÜMLE: iki sinyal birdenmiş. "Export my calendar to a file",
        // "Hatırlatıcılarımı Excel'e dök", "Remind me and save it as a PDF" —
        // hepsi hem gündelik hem belge izi taşır. Ölçüm (yönlendirme ikilisi,
        // vaka 26): gündelik önce bakıldığı için belge profili SEÇİLEMİYOR,
        // belge_olustur oturumda hiç bulunmuyor ve iş iki tura yayılıyordu;
        // ikinci turda profil değişimi veriyi düşürüyordu.
        //
        // Belge seti gündeliğin kişisel araçlarını ZATEN KAPSIYOR (takvim,
        // hatırlatıcı, arama + hesap/zaman); gündeliğe özgü olan yalnızca
        // Kişi, Kod ve web. Yani iki sinyal çakıştığında belge profili
        // katı üstün: iki işi de TEK turda yapabilir. Tek istisna rehber —
        // KisiAraci belge setinde yok, o yüzden kişi sinyali gündeliği tutar.
        let gundelikSinyali = gundelikIzleri.contains(where: s.contains)
        let belgeSinyali = belgeIzleri.contains(where: s.contains)
        if belgeSinyali, !kisiSinyaliVar { return outcome(.document) }
        if gundelikSinyali { return outcome(.gundelik) }
        if belgeSinyali { return outcome(.document) }
        // Bağlantı: yalnızca kullanılabilir araç VARSA (mcp §5.4).
        if input.baglantiKullanilabilir, baglantiSinyali(s, input: input) { return outcome(.connection) }
        // Arama: yalnızca sunucu tanımlı VE açıksa (web-arama §5.4). Kapalıysa
        // profil hiç seçilmez, araç modele hiç görünmez ve bugünkü dürüst
        // "cihazında böyle bir bilgi yok" yanıtı aynen sürer.
        // HESAP KAÇIŞI (denetim küme 1'in ikinci yarısı). Arama profilinde artık
        // `hesapla` YOK; bu yüzden oturum aramaya YAPIŞMIŞKEN gelen saf aritmetik
        // sorusu araçsız kalır ve model kafadan hesaplar — düzelttiğimiz arızanın
        // aynısını başka kapıdan geri getirirdi.
        //
        // Kaçış YALNIZ yapışkanlıkta verilir: cümlenin KENDİSİNDE açık bir arama
        // izi varsa ("euro", "fiyat", "kur ", "puan durumu") soru canlı veri
        // sorusudur ve aramada KALIR — kaçış onu kurtarmaz, kurtarmamalı da.
        // Yani "Euro kaç lira" hep aramada, arama turundan sonra gelen
        // "peki 250 ile 890'ı topla" gündelikte çözülür.
        let acikAramaIzi = aramaIzleri.contains(where: s.contains)
        if input.aramaKullanilabilir, aramaSinyali(s, input: input) {
            if !acikAramaIzi, hesapNiyeti(s) { return outcome(.gundelik) }
            return outcome(.search)
        }
        // Aksi halde mevcut profili koru — ama cihaz dışı profiller yalnızca
        // hâlâ kullanılabilirken yapışkandır. Kullanıcı arada sunucuyu kapattıysa
        // ya da bağlantıyı sildiyse oturum gündeliğe düşer.
        switch input.mevcutProfil {
        case .search:
            // Yapışkan aramadan aritmetik kaçışı burada da geçerli: önceki tur
            // globe çipi düşürmemişse aramaSinyali false döner ve akış buraya
            // gelir; kaçış tek yerde dursa oturum yine hesapsız kalırdı.
            if !acikAramaIzi, hesapNiyeti(s) { return outcome(.gundelik) }
            return outcome(input.aramaKullanilabilir ? .search : .gundelik)
        case .connection: return outcome(input.baglantiKullanilabilir ? .connection : .gundelik)
        case .gundelik, .document: return outcome(input.mevcutProfil)
        }
    }

    // MARK: - Tur-içi profil kurtarma (denetim P1-2)

    /// Deterministik seçici yanıldığında denenecek İKİNCİ profil.
    ///
    /// Sorun şuydu: `sec` tek bir profil döndürüyor ve yanılırsa gereken araç
    /// o oturumda HİÇ bulunmuyordu. Model bunu bir yetenek eksikliği gibi
    /// anlatıyor ("bunu yapamıyorum"), tur araçsız bitiyordu — sessiz bir
    /// yetenek boşluğu, kullanıcı için görünmez bir arıza.
    ///
    /// Saf fonksiyon: sinyal listelerinden okur, durum yazmaz, model gerektirmez.
    ///
    /// `kisiSinyaliVar` PARAMETRE: birinci seçimde hesaplanan değerin ta kendisi
    /// verilir (eskiden aynı örnek alanından okunuyordu).
    static func second_pass(_ input: Input,
                       birinci: ModelService.Profile,
                       kisiSinyaliVar: Bool) -> ModelService.Profile? {
        let s = input.question.lowercased()
        // Aday sırası SİNYAL GÜCÜNE göre: cümlede izi olan profiller önce.
        var adaylar: [ModelService.Profile] = []
        if belgeIzleri.contains(where: s.contains)             { adaylar.append(.document) }
        if gundelikIzleri.contains(where: s.contains)          { adaylar.append(.gundelik) }
        if kisiSinyaliVar                                      { adaylar.append(.gundelik) }
        if input.aramaKullanilabilir, aramaSinyali(s, input: input)       { adaylar.append(.search) }
        if input.baglantiKullanilabilir, baglantiSinyali(s, input: input) { adaylar.append(.connection) }
        // Hiç ikinci sinyal yoksa en geniş araç seti son çare. Belge kilidi
        // varken gündelik yerine belgeye düşülür: kullanıcının ekli belgesi
        // ortadayken onu masadan kaldırmak yeni bir boşluk açardı.
        adaylar.append(input.belgeVar ? .document : .gundelik)
        return adaylar.first { $0 != birinci }
    }

    // MARK: - Sinyaller

    /// Bağlantı sinyali: bağlantının kendi adı, "sunucu" sözcüğü ya da önceki
    /// turda düşmüş bir MCP çipi.
    static func baglantiSinyali(_ s: String, input: Input) -> Bool {
        if input.oncekiTurBaglanti { return true }
        let name = input.connectionName.lowercased()
        if !name.isEmpty, s.contains(name) { return true }
        return baglantiIzleri.contains(where: s.contains)
    }

    /// Arama sinyali: güncel-bilgi kalıpları (hava/kur/haber/fiyat/skor) ve
    /// "nedir/kimdir" türü genel bilgi soruları.
    static func aramaSinyali(_ s: String, input: Input) -> Bool {
        if input.oncekiTurArama { return true }
        return aramaIzleri.contains(where: s.contains)
    }

    /// AÇIK aritmetik niyeti — yapışkan arama oturumundan gündeliğe kaçışın
    /// tek ölçütü (denetim küme 1). Saf fonksiyon: durum okumaz, model gerektirmez.
    ///
    /// İKİ şart birden aranır — yalnız sözcük YETMEZ, RAKAM da gerekir. Tek
    /// başına sözcük listesi "bölge", "bölüm", "toplantı", "çarpıcı" gibi
    /// gündelik sözcüklerin içinde geçip canlı veri sorusunu aramadan kaçırırdı;
    /// rakam şartı bu yanlış pozitiflerin neredeyse hepsini kesiyor.
    ///
    /// Liste bilerek DAR ve canlı veri sözlüğüyle KESİŞMEZ: "fiyat", "kaç para",
    /// "kaç tl", "kur " burada YOK — onlar `aramaIzleri`nin işi ve orada kalmalı.
    /// Aramada SONDAKİ boşluk bilerek eklenir: "…'e böl" gibi izler sözcük SONU
    /// aramak zorunda ve tümce sonunda da eşleşmeleri gerekiyor. Boşluksuz
    /// "e böl" yazılsaydı "bu bölgede" içindeki "e bölge"ye takılırdı.
    static func hesapNiyeti(_ s: String) -> Bool {
        guard s.contains(where: \.isNumber) else { return false }
        let yastik = s + " "
        return hesapIzleri.contains(where: yastik.contains)
    }

    // MARK: - İz listeleri
    //
    // Bu listelerin İÇERİĞİ kurumsal hafızadır: her satır bir ölçülmüş
    // gerilemenin karşılığı. Buradaki yorumlar listelerle birlikte taşındı.

    /// Rehber niyeti. Web araması gündelik sete girdiği için gerekli: hangisinin
    /// oturuma alınacağına bu karar verir.
    static let kisiIzleri = [
        "kişi", "kisi", "rehber", "numara", "telefonu", "telefon numar",
        "mail adresi", "e-posta adres", "eposta adres", "adresi ne",
        "contact", "phone number", "email address", "in numarası",
    ]

    /// Belge kilidinden kaçış izleri (denetim P2-1). Rehber ve açık web ayrı
    /// hesaplanır; burası yalnızca belge setinde HİÇ karşılığı olmayan tek
    /// yetenek: kod çalıştırma.
    ///
    /// Liste bilerek çok DAR ve hep ÇOK SÖZCÜKLÜ/ayırt edici: kaçışın yanlış
    /// pozitifi, kullanıcının ekli belgesi üzerinde çalışmayı reddetmek demek
    /// — kilidin kendisinden daha kötü bir arıza. Çıplak "kod" yok ("kodu",
    /// "barkod", "kodlanmış" içinde geçer); çıplak "ajan" yok.
    static let kilitKacisIzleri = [
        "kod çalıştır", "kodu çalıştır", "javascript", "js ile hesapla",     // tr — kod_calistir
        "run this code", "execute this code", "in javascript",               // en
    ]

    /// Aritmetik izleri. Hepsi ya fiil ya da hesap-özgü kalıp; canlı değer adı yok.
    ///
    /// TUZAKLAR (hepsi ölçülüp elendi): çıplak "böl" yasak — "bölge/bölüm" içinde
    /// geçer, o yüzden yalnız çekimli hâlleri ve "…e böl " biçimi alınır. "eksi"
    /// yasak — "eksik" içinde geçer. "topla" tek başına "toplantı" içinde geçer
    /// ama rakam şartı bunu zaten kesiyor ("yarınki toplantım kaçta" rakamsız).
    static let hesapIzleri = [
        "hesapla", "hesab", "topla", "toplamı", "çarp", "carp",             // tr
        "böler", "bölers", "böleceğ", "bölün", "bölüp",                     // tr — çekimli
        "e böl ", "a böl ", "ye böl", "ya böl", "i böl ", "ı böl ",         // tr — "24'e böl"
        "yüzde", "yuzde", "kdv", "kaç eder", "kac eder", "kaç yapar",       // tr
        "kaç kalır", "farkı ne", "kaçtan",                                  // tr
        "calculate", "compute", "multiply", "divide", "subtract",           // en
        "sum of", "percent of", "times what",                               // en
    ]

    /// Hatırlatıcı/arama niyeti (gündelik profil) — tr/en/zh/ja/es/de/fr/ko/pt.
    static let gundelikIzleri = [
        "hatırlat", "hatirlat", "anımsat", "notlarım", "notlarda",          // tr
        "dosyalarımda", "dosyam var mı", "dosyalarım",                      // tr — yerel dosya ARAMASI
        "remind", "reminder", "my note", "notes", "search my",              // en
        // Kişisel-veri İngilizce kalıpları: gündelik izler arama izlerinden
        // ÖNCE bakıldığı için "What is John's phone number?" burada yakalanır
        // ve KisiAraci oturumda kalır (aksi halde web aramasına kaçıyordu).
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

    /// Belge/dosya niyeti (belge profil) — biçim adları + 8 dilde ad-fiiller.
    /// "site/html/sayfa/landing" izleri kod-spec §7: .html biçimi araç
    /// eklemez, `belge_olustur` zaten belge profilindedir.
    ///
    /// "site" BİLEREK çıplak değil: arama alt-dizgeyledir ve çıplak "site",
    /// "üniversite(si)/kapasite/opposite" gibi sözcüklerin içinde geçip soruyu
    /// belgeye kilitlerdi ("Boğaziçi Üniversitesi nedir?" aramaya gidemezdi) —
    /// "kur " izindeki sondaki boşlukla aynı tuzak. " site" sözcük başını
    /// arar; "site yap/kur" cümle başını, "websit" bitişik yazımı kapsar.
    static let belgeIzleri = [
        "excel", "xlsx", "pdf", "word", "docx", "markdown", ".md",          // dil-nötr
        "html", " site", "site yap", "site kur", "websit", "landing",       // web sayfası (kod-spec §7)
        "web page",                                                         // en — GenerateDocumentIntent.promptWord(.html)
        // ÇIPLAK "tablo" BİLEREK YOK — ve karşılıkları da (table/tabla/tabelle/
        // tableau/表格/표). "Tablo yap" bir GÖRÜNTÜLEME isteğidir, dosya isteği
        // değil: kullanıcı ekranda görmek ister, isterse `SohbetTablo`nun kendi
        // indirme düğmesiyle Excel'e çevirir. Bunlar burada durduğu sürece her
        // tablo isteği belge profiline kaçıyor, belge_olustur çalışıyor ve model
        // içerik yerine bir dosya adı uydurup gereksiz .xlsx üretiyordu.
        // Dosya niyeti ayrı sözcüklerle ("excel", "dosya", "indir") zaten yakalanır.
        "belge", "dosya", "indir", "rapor", "döküm", "dök",                 // tr
        "sayfa",                                                            // tr — web sayfası
        // Tablo/belge YAPI sözcükleri: bunlar olmadan "Çarşamba Köfte satırını
        // ekle" gündelikte kalıp TakvimAraci'na kaçıyordu (belge_duzenle
        // oturumda hiç bulunmuyordu).
        // İngilizce karşılıkları ÇIPLAK DEĞİL, baştaki boşlukla — " site" ve
        // "kur " ile aynı alt-dizge tuzağı. Çıplak "row" TOMORROW'un içinde
        // geçiyor: "What's the weather tomorrow?" belge profiline düşüyor,
        // web_arama oturumda hiç bulunmuyordu (ölçüldü). Aynısı "cell" ↔
        // "excellent", "arrow", "borrow", "narrow", "grow" için de geçerli.
        "satır", "sütun", "kolon", "hücre", " row", " column", " cell",
        // Not olarak KAYDETME — üretim isteği. Çıplak "not"/"kaydet" BİLEREK
        // yok: "kur " izindeki alt-dizge tuzağı ("nota"→"nokta" değil ama
        // "not"→"nota/nokta/motor" bol yanlış pozitif verirdi).
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

    /// Güncel/dünya bilgisi niyeti (arama profili) — web-arama §5.4.
    ///
    /// Bilerek DAR tutuldu: yanlış pozitif, kişisel veri araçlarını o turdan
    /// çıkarıp "hatırlatıcı kuramadım"a yol açar. Genel bilgi kalıpları
    /// ("nedir", "kimdir") burada, kişisel içerik kalıpları gündelik listede.
    /// Kullanıcının web'i AÇIKÇA istediği kalıplar. Konu tahmini değil, niyet
    /// beyanı — bu yüzden diğer tüm sinyallerin önünde değerlendirilir.
    static let acikWebIzleri = [
        "internette", "internetten", "internete", "web'de", "webde", "web de",
        "webte", "web'te", "internette ara", "google", "googlela", "çevrimiçi",
        "on the web", "on the internet", "search online", "look it up online",
        "search the internet", "google it",
        // Kalan yedi dil: komşu `aramaIzleri` dokuz dili kapsarken en yüksek
        // öncelikli sinyal yalnız tr+en olduğu için Almanca/Japonca konuşan
        // kullanıcının açık web isteği hiç eşleşmiyordu.
        "im internet", "im netz", "online suchen", "im web",
        "en internet", "en la web", "buscar en internet", "buscar en línea",
        "sur internet", "sur le web", "cherche en ligne", "recherche en ligne",
        "ネットで", "インターネットで", "ウェブで", "オンラインで",
        "인터넷에서", "웹에서", "온라인으로", "인터넷으로",
        "na internet", "na web", "pesquise online", "buscar na internet",
        "在网上", "上网查", "网上搜", "在线搜索",
    ]

    /// Dünyaya dair, kullanıcının cihazında BULUNAMAYACAK bilgi kalıpları.
    /// Liste doğası gereği eksik kalır — bkz. `dunyaSorusuMu`.
    static let aramaIzleri = [
        // Ulaşım / tarife / mekân — "vapur saatleri" vakasından sonra eklendi.
        "vapur", "feribot", "sefer saat", "tarife", "kalkış saat", "otobüs saat",
        "metro saat", "tren saat", "uçuş", "kaçta kalk", "kaçta açıl", "açık mı",
        "nasıl gidilir", "ne kadar sürer", "kaç durak",
        // Günlük kamu bilgisi — "namaz vakitleri" vakasından sonra eklendi.
        // Model bunları BİLMİYOR ve sorulduğunda uyduruyordu (İstanbul için
        // 05:00/12:00/15:00 gibi yuvarlak, tamamen hayalî vakitler verdi).
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
        // ÇIPLAK "what is"/"who is" BİLEREK YOK: İngilizce'de neredeyse her
        // soru bu kalıpla başlıyor ve "What is John's phone number?" gibi
        // KİŞİSEL veri sorularını arama profiline atıp KisiAraci'yı oturumdan
        // düşürüyordu. Yalnız daraltılmış hâlleri alınır.
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

    /// Bağlantı niyeti (bağlantı profili) — mcp §5.4. Bağlantının KENDİ adı
    /// ayrıca `baglantiSinyali`de aranır; burası yalnızca genel sözcükler.
    static let baglantiIzleri = [
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
