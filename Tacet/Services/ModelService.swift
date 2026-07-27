//
//  ModelServisi.swift
//  Tacet
//
//  Model katmanı (spec §7.1, §7.2). Yalnızca on-device SystemLanguageModel;
//  Private Cloud Compute kullanılmaz. Availability bir feature flag gibi ele
//  alınır. Bağlam penceresi (4096) aktif yönetilir; taşma sessizce kurtarılır.
//

import Foundation
import Observation
import FoundationModels
import NaturalLanguage
import SwiftData

@MainActor
@Observable
final class ModelService {

    /// Oturumun durumu. Ekranda renkli gösterge yoktur; durum yalnızca sözle anlatılır.
    enum State: Equatable {
        case ready                    // sessiz — anlatılacak bir şey yok
        case preparing             // gri · "hazırlanıyor…"
        case unavailable(String)     // gri · neden

        var tag: String {
            switch self {
            case .ready: return String(localized: "On your device")
            case .preparing: return String(localized: "preparing…")
            case .unavailable(let cause):
                return cause.isEmpty ? String(localized: "not available on this device") : cause
            }
        }
        var isReady: Bool { self == .ready }
    }

    /// Neden yanıt üretemiyoruz — kullanıcıya "ne oldu + ne yapmalı" demek için tutulur.
    /// `Durum.etiket` kısa rozet metnidir; bu ise sohbete düşen tam cümleyi seçer.
    enum Block { case device, closed, preparing }

    /// Bir turun sonucu. Hata olup olmadığı METİNDEN ÇIKARILMAZ — servis açıkça
    /// bildirir. (Eskiden UI, dönen metni bilinen hata dizgileriyle karşılaştırıyordu;
    /// metin her değiştiğinde hata balonu sessizce ölüyordu.)
    /// Turun NEDEN düştüğü. Tek bir "Şu an bunu yapamadım" cümlesi üç ayrı
    /// arızayı örtüyordu (ölçüm: 5 vaka, hepsi aynı metin, hiçbiri aynı sebep).
    /// Sınıf iki işi birden görür: kullanıcıya doğru cümleyi seçer (`L10n`)
    /// ve eval ham JSON'una yazılır — bir sonraki teşhis log eklemeden yapılır.
    ///
    /// İÇ AYRINTI SIZDIRMAZ: sınıf ADI loglanır, kullanıcı yalnız ona karşılık
    /// gelen sade cümleyi görür (hata metni, satır no, model adı geçmez).
    enum ErrorClass: String, Codable, Sendable {
        case none
        /// Model konuştu ama geriye yalnız ayrıştırılamamış araç çağrısı kaldı.
        case bosYanit
        /// Turda bir araç düştü ve model üstüne söyleyecek metin üretmedi.
        case aracDustu
        /// Bağlam penceresi taştı; özetleyip yeniden kurma da tutmadı.
        case baglamTasmasi
        /// Guardrail — kurtarılamaz, retry yok.
        case sinirDisi
        /// Dil desteklenmiyor — kurtarılamaz, retry yok.
        case dilDisi
        /// Yan etki oluştuktan sonraki hata; retry bilerek yapılmadı.
        case yazmaSonrasi
        /// Diğer üretim hataları; taze oturumla retry da tutmadı.
        case uretimHatasi
    }

    struct ReplyOutcome {
        let text: String
        let traces: [ToolTrace]
        /// Hata balonu olarak çizilsin mi. İPTAL HATA DEĞİLDİR (false).
        var isError: Bool = false
        /// Aynı istem güvenle tekrar gönderilebilir mi. Yan etki oluşmuşsa false.
        var isRetryable: Bool = false
        /// Kalıcı değil, yalnızca anlık durum bildirimi — SwiftData'ya yazılmamalı.
        var isTransient: Bool = false
        /// Hata sınıfı — `isError` false ise daima `.yok`.
        var errorClass: ErrorClass = .none
    }

    private(set) var state: State = .preparing
    private(set) var block: Block? = .preparing

    /// Model kullanılamazken sohbete düşecek açıklama. Duruma göre değişir ki
    /// "hazırlanıyor" ile "bu cihazda hazır değil" mesajları çelişmesin.
    var blockMessage: String {
        switch block {
        case .preparing: return L10n.modelPreparing
        case .closed:       return L10n.appleIntelligenceClosed
        case .device, nil:   return L10n.deviceNotEligible
        }
    }
    /// Araç çipi tek doğruluk kaynağı — tools buraya rapor eder, UI buradan okur.
    let executor = ToolExecutor()
    /// Engel geçici mi (bekleyip yeniden denemek anlamlı mı) — cihaz uygun değilse değil.
    var engelTekrarDenenebilir: Bool {
        switch block {
        case .preparing, .closed: return true
        case .device, nil:            return false
        }
    }
    /// Sohbete paylaşılan/üretilen belgeler — belge araçları buraya erişir, UI önizler.
    let documentContext = DocumentContext()
    /// Büyük veri taşıma kanalı (spec §7.3.2) — toplu veri modelden geçmeden araçlar arası taşınır.
    let dataStore = DataStore()
    /// Kod çalıştırma deneme sayacı (kod-spec §5.4): tur başına en fazla 2
    /// gerçek çalıştırma. Sayaç araçta değil burada yaşar ve `init` içinde
    /// `yurutucu.turKancasi`na bağlanır — sıfırlama spec'in dediği yerde,
    /// AracYurutucu.newTurn içinde TEK noktadan olur; çağrı noktalarında elle
    /// eşleme yoktur, unutulan bir yol tavanı oturum ömürlü yapamaz.
    let codeState = CodeState()

    /// Turun seyri (seyir-spec §5.2). SALT GÖZLEMCİ: buradaki hiçbir metin
    /// isteme ya da talimata girmez, modelden hiçbir durum bildirimi istenmez.
    /// Kaydedicinin varlığıyla yokluğu arasında model çıktısı bit düzeyinde
    /// aynıdır (§6 kabul ölçütü) — bu yüzden tüm çağrılar tek yönlü bildirimdir.
    let timeline = TimelineRecorder()

    /// Sohbet sıfırlandığı anda hafıza ayıklamasını tetikleyen kanca
    /// (hafiza-spec §4.1). `ModelServisi`nin ne `Sohbet` ne `ModelContext`
    /// erişimi vardır; ikisini de bilen katman (ContentView) bunu bağlar.
    var hafizaTetigi: (() -> Void)?

    /// Bağlantı profilinde oturuma girecek MCP araçları (mcp §5.4).
    ///
    /// Burada kurulmazlar: MCP aracının şeması sunucudan çalışma anında gelir
    /// ve `oturumKur` senkrondur. Bağlantıyı bilen katman hazır araçları
    /// `baglantiAraclariniAyarla` ile verir; boşsa profil hiç seçilmez.
    private var mcpAraclari: [MCPTool] = []
    /// Seçili bağlantının adı — yönlendirme sinyali ve seyir satırı için.
    private var connectionName: String = ""

    /// Seçili bağlantının araçlarını oturuma hazırlar. Boş dizi = bağlantı yok;
    /// bağlantı profili o an seçilemez hâle gelir (araç modele hiç görünmez).
    ///
    /// Aktif oturum bağlantı profilindeyse liste değiştiğinde oturum
    /// geçersizleşir: bir sonraki turda yeni araç setiyle yeniden kurulur.
    func baglantiAraclariniAyarla(_ tools: [MCPTool], name: String = "") {
        mcpAraclari = tools
        connectionName = name
        if aktifProfil == .connection { session = nil }
    }

    /// En fazla kaç MCP aracı oturuma girer (mcp §5.4: "gerekirse ilk 4–6").
    /// Hesap + Zaman ile birlikte 6–8 bütçesi korunur.
    private static let mcpAracTavani = 6

    /// Alaka sıralamasının içinden seçtiği havuz. Yuvadan geniş, ama şema
    /// çevirisi bedava olmadığı için sınırsız değil (mcp §5.2).
    private static let mcpAracHavuzu = 24

    /// Üretim seçenekleri (denetim P0-5'in eksik yarısı).
    ///
    /// ÜRETİMDE VARSAYILAN DEĞİŞMEZ: `GenerationOptions()` bugünkü davranışın
    /// ta kendisidir; buraya dokunmak kullanıcının gördüğü yanıtları değiştirir
    /// ve bu ayrı bir karardır. Değişken YALNIZCA eval için vardır.
    ///
    /// NEDEN GEREKLİ (ölçüldü, 20 Temmuz 2026): aynı ikiliyle kod değişmeden
    /// iki koşum arasında vakaların %27'si puan değiştirdi, değişenlerde
    /// ortalama oynama 21.8 puan, kategori ortalamaları kendiliğinden ±15 puan
    /// kaydı. Bu gürültü tabanında bir düzeltmenin işe yarayıp yaramadığı tek
    /// koşumla SÖYLENEMEZ — her "iyileştirme" iddiası zayıf kalır.
    ///
    /// `.greedy` sıcaklık 0'dan daha kesindir: sıcaklık 0 hâlâ örnekleme
    /// yapabilir, greedy her adımda en olası jetonu alır.
    ///
    /// MainActor'a BAĞLI: okumalar zaten buradan (`akisYut`), yazma yalnız
    /// DEBUG eval yollarından (`Degerlendirme`, `OtoTestVakalari` — ikisi de
    /// MainActor). `nonisolated(unsafe)` bu değişmezi doğru ama derleyiciden
    /// SAKLI tutuyordu; ilk nonisolated yazma sessizce veri yarışı olurdu.
    @MainActor static var uretimSecenekleri = GenerationOptions()

    /// Eval koşumları için örneklemeyi kapatır. Yalnız DEBUG yollarından
    /// çağrılır; üretim ikilisinde çağıran yoktur.
    static func orneklemeyiKapat() {
        uretimSecenekleri = GenerationOptions(sampling: .greedy)
    }

    /// Bu turun ham istemi — yuva alaka sıralamasının tek girdisi (P1-6).
    /// Oturum kurulumu senkron olduğu için soru bir alan üzerinden taşınır.
    private var sonSoru: String = ""

    /// Yuvaya girecek MCP araçları: havuzdan, kullanıcının o turki isteğine
    /// ALAKAYA göre seçilir (P1-6). Eski davranış `prefix(tavan)` idi, yani
    /// yuvalar sunucunun `tools/list` sırasına göre KÖR doluyordu; 20 araçlı
    /// bir sunucuda "issue aç" isteği, `issue_olustur` 14. sıradaysa masaya
    /// hiç gelmiyordu.
    ///
    /// `sirala` kararlıdır: soru hiçbir sinyal taşımıyorsa sonuç eski kör
    /// prefix'in AYNISI — bu bir gerileme güvencesidir.
    private func secilenMCPAraclari() -> [MCPTool] {
        Array(ToolRelevance.sort(mcpAraclari, question: sonSoru,
                               name: \.remoteName, summary: \.summary)
            .prefix(Self.mcpAracTavani))
    }

    // MARK: - Bağlantı köprüsü (mcp §5.4 — fişe takma)

    /// Uzak çağrı yolu ve istemci sahipliği. `MCPAraci` yalnızca bu sözleşmeyi
    /// görür; ağ kodu hâlâ tek yerde (`MCPIstemcisi`).
    let baglantiKopru = MCPToolBridge()

    /// Araçları hangi bağlantı listesinden kurduğumuzun imzası. Aynı listede
    /// ikinci kez ağa çıkmayı önler; liste (ad/adres/araç adları) değişirse
    /// tazelenir.
    private var baglantiImzasi = ""
    /// Süren araç kurma görevi — liste değişince eskisi iptal edilir.
    private var baglantiGorevi: Task<Void, Never>?

    /// Kayıtlı bağlantılardan MCP araçlarını kurup oturuma besler (mcp §5.4).
    ///
    /// Şema sunucudan çalışma anında geldiği için kurulum ASENKRONDUR: araçlar
    /// hazır olana kadar `mcpAraclari` boştur, yani bağlantı profili seçilemez ve
    /// bugünkü davranış aynen sürer. Hiç bağlantı yoksa ağa hiç çıkılmaz (§2.1).
    ///
    /// Seçili bağlantı: en son kullanılan, hiç kullanılmadıysa en son eklenen.
    /// v1'de ayrı bir seçim yüzeyi yok; 4096 pencerede aynı anda tek sunucunun
    /// araçları taşınabildiği için tek bağlantı seçilir.
    func refreshConnections(_ baglantilar: [Connection]) {
        let canli = baglantilar.filter { !$0.isDeleted && $0.isValid }
        let secili = canli
            .sorted { ($0.lastUsed ?? $0.createdAt) > ($1.lastUsed ?? $1.createdAt) }
            .first { !$0.availableTools.isEmpty }

        guard let secili, let url = secili.url else {
            // Bağlantı yok (ya da hiçbirinin desteklenen aracı yok): profil hiç
            // seçilemez hâle döner, istemciler bırakılır.
            baglantiGorevi?.cancel()
            baglantiGorevi = nil
            baglantiImzasi = ""
            baglantiKopru.forget()
            baglantiAraclariniAyarla([], name: "")
            return
        }

        // SwiftData tuzağı: nesneye await'ten ÖNCE dokunulur.
        let identity = secili.id
        let name = secili.name
        let deviceData = secili.deviceData
        // Desteklenmeyen şemalı araçlar zaten burada eleniyor (§5.2).
        let ozetler = secili.availableTools
        let key = secili.keyRef.flatMap { Keychain.read(ref: $0) }
        // İmza cihazVerisi'ni İÇERİR: kullanıcı ayarı BaglantiDetayi'nde
        // değiştirdiğinde araçlar yeniden kurulsun, yeni ayar oturum boyunca
        // beklemesin.
        let signature = "\(identity)|\(name)|\(url.absoluteString)|\(deviceData.rawValue)|\(ozetler.map(\.name).joined(separator: ","))"
        guard signature != baglantiImzasi else { return }
        baglantiImzasi = signature
        baglantiKopru.save(identity: identity, url: url, key: key)

        baglantiGorevi?.cancel()
        baglantiGorevi = Task { [weak self] in
            guard let self else { return }
            let tools = await baglantiKopru.araclariKur(
                connectionID: identity, name: name, ozetler: ozetler,
                pool: Self.mcpAracHavuzu, deviceData: deviceData,
                gate: executor, reporter: executor)
            guard !Task.isCancelled, baglantiImzasi == signature else { return }
            // Sunucuya erişilemediyse imzayı düşür: bir sonraki tazelemede
            // yeniden denensin, kullanıcı ağ dönünce uygulamayı yeniden
            // başlatmak zorunda kalmasın.
            if tools.isEmpty { baglantiImzasi = "" }
            baglantiAraclariniAyarla(tools, name: name)
        }
    }

    /// Araç profili (spec §7.3.1): 4096 pencerede oturuma en fazla 6–8 araç verilir.
    ///
    /// `arama` ve `baglanti` cihaz DIŞINA çıkan profillerdir ve kişisel veri
    /// araçlarını BİLEREK içermez (web-arama §5.4, mcp §5.4): modelin bir
    /// argümana kişisel veri yazması ihtimaline karşı yapısal savunma —
    /// araç oturumda yoksa veri de çıkamaz.
    enum Profile {
        case gundelik, document, search, connection

        /// Seyir satırında görünen ad. Profil, kullanıcının göremediği bir iç
        /// karar değil — hangi araçların masada olduğunu belirler, o yüzden
        /// anlatılır.
        var seyirAdi: String {
            switch self {
            case .gundelik: return String(localized: "everyday profile")
            case .document:    return String(localized: "document profile")
            case .search:    return String(localized: "search profile")
            case .connection: return String(localized: "connection profile")
            }
        }
    }
    private var aktifProfil: Profile = .gundelik
    /// Mevcut oturumun araç setinin imzası. PROFİL ADI YETMEZ: gündelik setin
    /// içeriği tur içinde değişiyor (Kişi ↔ web takası, Spotlight'ın gizlenmesi)
    /// ve yeniden kurma koşulu yalnızca `istenen != aktifProfil`e baksaydı bu
    /// takas ilk turdan sonra HİÇ uygulanmazdı — mekanizma tanımlı olur ama
    /// çalışmazdı. Ölçülen yol: 1. tur "hatırlatıcı kur" (set web'li kurulur),
    /// 2. tur "annemin telefon numarası" → profil hâlâ .gundelik olduğu için
    /// oturum korunuyor ve KisiAraci oturuma hiç girmiyordu.
    private var oturumAracImzasi = ""
    /// Oturuma gömülü dil adı (yanıt-dili çapası). Kullanıcının dili saptanınca güncellenir.
    private var aktifDil: String = ""
    /// Mevcut oturumun kurulduğu dil — gereksiz yeniden kurmayı önler.
    private var oturumDili: String = ""
    /// Aktif dil kullanıcının açık seçiminden mi geliyor (saptamadan değil) — talimat metnini değiştirir.
    private var dilSecildi: Bool = false
    /// Oturumun kurulduğu andaki seçim durumu — talimat metni buna göre yazıldığı için izlenir.
    private var oturumDilSecildi: Bool = false


    private let models = SystemLanguageModel.default
    private var session: LanguageModelSession?

    /// Bağlam bütçesi eşiği: contextSize'ın %80'i (araştırma raporu §5.2).
    private let esikOran = 0.80

    init() {
        // Uzak çıktı da 4096 bütçesine göre işlenir (§5.5): büyük sonuç modelden
        // geçmeden veri deposuna konur, modele özet + kaynakRef gider.
        baglantiKopru.dataStore = dataStore
        // P0-3: uzak çağrı başarıyla dönünce retry kapısını kapatan bayrağı
        // kuracak olan taraf. Köprü TEK huni: her uzak çağrı buradan geçer,
        // dolayısıyla yeni bir MCP çağrı yolu eklenirse bayrağı kurmayı
        // unutmak mümkün değil.
        baglantiKopru.executor = executor
        // Kod deneme sayacı tur yaşam döngüsüne buradan bağlanır (kod-spec
        // §5.4: sıfırlama AracYurutucu.newTurn'dadır). sohbetiSifirla da
        // newTurn'u içeriden çağırdığı için tüm yollar tek kancadan geçer.
        executor.turKancasi = { [codeState] in codeState.newTurn() }
        checkAvailability()
    }

    // MARK: - Availability

    func checkAvailability() {
        let oncekiHazir = state.isReady
        switch models.availability {
        case .available:
            state = .ready
            block = nil
            // Zaten hazırdıysak oturuma DOKUNMA: yeniden kurmak transcript'i (yani
            // sohbetin bağlamını) sessizce silerdi ve `sohbetiSifirla`nın tembel
            // kurulumunu bozardı. Yalnızca hazır-değil → hazır geçişinde kur.
            if !oncekiHazir { oturumKur(profile: aktifProfil) }
        case .unavailable(let cause):
            switch cause {
            case .deviceNotEligible:
                state = .unavailable(String(localized: "not available on this device"))
                block = .device
            case .appleIntelligenceNotEnabled:
                state = .unavailable(String(localized: "Apple Intelligence is off"))
                block = .closed
            case .modelNotReady:
                state = .preparing
                block = .preparing
            @unknown default:
                state = .unavailable(String(localized: "not available on this device"))
                block = .device
            }
        }
    }

    /// Availability'yi yeniden okur. Model indirmesi biten ya da Apple Intelligence
    /// sonradan açılan cihazda uygulamayı yeniden başlatmadan asistanı açar.
    /// Sahneye dönüşte (UstBar/ContentView) ve her istek başında çağrılır.
    /// Zaten hazırsa hiçbir şeyi sıfırlamaz — güvenle sık çağrılabilir.
    func reloadAvailability() { checkAvailability() }

    // MARK: - Oturum ve profiller

    /// Gündelik profil (spec §8, v1): Takvim, Hatırlatıcı, Kişi/Arama, Hesap,
    /// Zaman + Kod (kod-spec §7). Tavan 6–8 araç (spec §7.3); cihaz ölçümü kötü
    /// çıkarsa `zaman` düşürülür ya da kod niyeti ayrı profil olur
    /// (kod-spec §9 açık soru 2).
    private func gundelikAraclar() -> [any Tool] {
        var calendar = CalendarTool();          calendar.reporter = executor; calendar.dataStore = dataStore
        var reminder = ReminderTool(); reminder.reporter = executor; reminder.dataStore = dataStore
        var calc = CalcTool();            calc.reporter = executor
        var time = TimeTool();            time.reporter = executor
        var code = RunCodeTool();        code.reporter = executor; code.state = codeState
        var tools: [any Tool] = [calendar, reminder, calc, time, code]

        // KİŞİ ↔ WEB ARAMASI TAKASI. İkisi aynı sette DURMAZ ve bu iki sebepten:
        //
        // 1) Bütçe: 6–8 araç tavanı (spec §7.3). İkisi birden tavanı zorlar.
        // 2) Güvenlik: web-arama §5.4 kişisel veri araçlarını web ile aynı
        //    profilde istemez — model rehberden okuduğunu sorguya yazabilir.
        //
        // Hangisinin gireceğini SORU belirler: cümlede rehber sinyali varsa
        // Kişi, yoksa web araması. Böylece "annemin numarası" da çalışır,
        // "namaz vakitleri" de — ve ikisi asla yan yana gelmez.
        if kisiSinyaliVar {
            var contact = ContactTool(); contact.reporter = executor
            tools.insert(contact, at: 2)
        } else if aramaKullanilabilir {
            var web = WebSearchTool()
            web.reporter = executor; web.executor = executor; web.dataStore = dataStore
            tools.insert(web, at: 2)
        }

        // Spotlight araması, kullanıcı AÇIKÇA web istediyse ve web araması
        // kapalıysa oturuma HİÇ GİRMEZ.
        //
        // Ölçülen hata: "internette ara" dendiğinde model `not_arama`yı yedek
        // olarak çağırıyor, sonuç boş dönüyor ve yanıt "internette arama
        // yapıldı, cihazda bulunamadı" oluyordu — YAPILMAMIŞ bir işi yaptım
        // demek, üretebileceği en kötü çıktı. Talimata ve beceri gövdesine
        // yazılan yasak İKİ KEZ denendi ve tutmadı; 3B modelde yasak metni
        // davranışı kontrol etmiyor. Yeteneği elinden almak kontrol ediyor.
        if !yerelAramayiGizle {
            var search = SearchNotesTool(); search.reporter = executor
            tools.insert(search, at: 3)
        }
        return tools
    }

    /// Bu turda Spotlight araması gizlensin mi — `NiyetSecici.sec` her turda tazeler.
    private var yerelAramayiGizle = false
    /// Bu turda rehber sorusu mu — Kişi ↔ web araması takasını belirler.
    private var kisiSinyaliVar = false

    /// Belge/üretim profili (spec §7.3.1): Oluştur, Oku, Düzenle + veri kaynağı ve yardımcılar.
    ///
    /// Bir belge ekliyken `niyetProfili` profili .belge'ye KİLİTLER; bu yüzden bu sette
    /// olmayan bir araç kullanıcı için tamamen erişilemez hâle gelir. "Bunu yarın
    /// hatırlat" / "notlarımda ara" belge ekliyken en sık gelen iki istek olduğu için
    /// Hatırlatıcı ve Arama buraya alındı. 6–8 araç bütçesine sığmak adına KisiAraci
    /// çıkarıldı: belge üretiminde kişi araması en az kullanılan yol (belge içeriği
    /// zaten ekli belgeden ya da takvim/veri deposundan geliyor), ayrıca Kişiler izni
    /// gerektirdiği için çoğu turda zaten sonuçsuz dönüyordu.
    private func belgeAraclar() -> [any Tool] {
        var create = CreateDocumentTool(); create.reporter = executor; create.context = documentContext; create.dataStore = dataStore
        var read = ReadDocumentTool();         read.reporter = executor;      read.context = documentContext
        var duzenle = EditDocumentTool(); duzenle.reporter = executor;  duzenle.context = documentContext
        var calendar = CalendarTool();        calendar.reporter = executor;   calendar.dataStore = dataStore
        var reminder = ReminderTool(); reminder.reporter = executor; reminder.dataStore = dataStore
        var search = SearchNotesTool();          search.reporter = executor
        var calc = CalcTool();          calc.reporter = executor
        var time = TimeTool();          time.reporter = executor
        return [create, read, duzenle, calendar, reminder, search, calc, time]
    }

    /// Arama profili (web-arama §5.4): web_arama + Zaman. HESAP YOK.
    ///
    /// Takvim/Kişi/Arama(Spotlight)/Belge/Hatırlatıcı BİLEREK YOK. Model
    /// sorguyu kendi üretir; kişisel veri aracı aynı oturumda dururken
    /// "notlarımdaki adresi ara" tek adımda dışarı sızabilirdi. Kişisel veri
    /// gerektiren karma iş iki turda akar ve ikinci tur oturumu kirletmiş
    /// olduğu için onay kapısına düşer.
    ///
    /// HESAP ARACI ÇIKARILDI (denetim küme 1 — canlı veri uydurması). Ölçülen
    /// üç vaka, bu profilde `hesapla`nın tek işlevinin UYDURMAYA MEŞRU KILIF
    /// olduğunu gösterdi — model arayıp bulamadığı canlı değeri kendi kafasından
    /// atıp aritmetiği araca yaptırıyor, çıkan sayıyı "araçtan geldi" diye
    /// sunuyordu:
    ///
    ///   web-euro    → hesapla("(1.00 / 0.85) * 100") → "Euro 117,6471 TL"
    ///   web-benzin  → hesapla("(1.60 * 1.20)")       → "Benzin 1.92 TL"
    ///   web-lig     → hesapla("(139+30)*1.20")       → "202.8 puanla lider Beşiktaş"
    ///
    /// Üçünde de girdi sayıları HİÇBİR arama sonucundan gelmiyor; uydurma zaten
    /// `ifade` alanında olup bitiyor. Araç doğru çalışıyor, sonuç yine de yalan.
    /// Bunu talimatla kapatmak modelin iyi niyetine bel bağlamaktır (aynı ders
    /// `yerelAramayiGizle`de iki kez alındı: 3B modelde yasak metni davranışı
    /// kontrol etmiyor, yeteneği elinden almak kontrol ediyor).
    ///
    /// BEDELİ ve neden kabul edilebilir olduğu: "100 dolar kaç TL" gibi zincir
    /// iki tura yayılır — kur bu turda web'den gelir, çarpma sonraki turda
    /// gündelik profilde `hesapla` ile yapılır. `niyetProfili`ndeki HESAP KAÇIŞI
    /// o ikinci turu gündeliğe yönlendirir, yani zincir kopmaz, yalnız uzar.
    /// Aritmetiğin bir tur gecikmesi, uydurulmuş bir kurun anında sunulmasından
    /// her koşulda iyidir.
    private func aramaAraclar() -> [any Tool] {
        var web = WebSearchTool(); web.reporter = executor; web.executor = executor; web.dataStore = dataStore
        var time = TimeTool();  time.reporter = executor
        return [web, time]
    }

    /// Bağlantı profili (mcp §5.4): seçili bağlantının araçları + Hesap + Zaman.
    /// Kişisel veri araçları arama profilindeki gerekçenin aynısıyla yoktur.
    private func baglantiAraclar() -> [any Tool] {
        var calc = CalcTool(); calc.reporter = executor
        var time = TimeTool(); time.reporter = executor
        return secilenMCPAraclari().map { $0 as any Tool } + [calc, time]
    }

    /// Kurulacak araç setini tek dizgede özetler. Yalnızca seti GERÇEKTEN
    /// değiştiren girdiler yazılır; aksi halde her turda gereksiz yeniden
    /// kurma (ve bir özetleme turu) maliyeti çıkardı.
    private func aracImzasi(_ profile: Profile) -> String {
        switch profile {
        case .gundelik:
            let takas = kisiSinyaliVar ? "kisi" : (aramaKullanilabilir ? "web" : "yok")
            return "gundelik|\(takas)|\(yerelAramayiGizle ? "spotsuz" : "spot")"
        case .document:    return "belge"
        case .search:    return "arama"
        // SAYI YETMEZ: alaka sıralaması turdan tura FARKLI altılıyı seçebilir,
        // sayı ise aynı kalır. İmza seçilen ADLARI yazar; aksi halde mekanizma
        // tanımlı olur ama ilk turdan sonra oturum hiç yeniden kurulmadığı için
        // hiç çalışmazdı (gündelik setteki Kişi ↔ web takasının aynı tuzağı).
        case .connection: return "baglanti|" + secilenMCPAraclari().map(\.name).joined(separator: ",")
        }
    }

    private func araclariYap(_ profile: Profile) -> [any Tool] {
        switch profile {
        case .gundelik:  return gundelikAraclar()
        case .document:     return belgeAraclar()
        case .search:     return aramaAraclar()
        case .connection:  return baglantiAraclar()
        }
    }

    // MARK: - Beceriler (progressive disclosure)

    /// Tur-başına istem zenginleştirme (hafıza notu + beceri kılavuzu + veri
    /// referansları + ekli belge satırı + dil hatırlatması). Mantık ve oturum
    /// ömürlü enjeksiyon durumu `IstemZenginlestirici`de yaşar; buradan yalnız
    /// tetiklenir ve `sifirla()` ile oturum sınırında temizlenir.
    private let zenginlestirici = PromptEnricher()

    /// Mevcut oturumdaki araç adları. Beceri kapısının TEK doğruluk kaynağı
    /// (denetim P1-4): kılavuz, emrettiği araç oturumda YOKSA enjekte edilmez —
    /// aksi halde model var olmayan bir aracı çağırmaya yönlendirilirdi
    /// (belge kilidindeyken "kodla…" isteği kod.md'yi tetikliyor ama
    /// `kod_calistir` belge setinde yok).
    ///
    /// Eskiden burada elle yazılmış bir `[beceri: Set<Profil>]` haritası vardı.
    /// O harita ARAÇ SETİNİN KOPYASIYDI ve kopya olduğu için sessizce
    /// eskiyordu: bir araç profiller arasında taşındığında (Hatırlatıcı ve
    /// Arama'nın belge setine alınması gibi) haritayı güncellemeyi unutmak
    /// hiçbir derleme hatası vermiyordu. Artık kapı, oturumun GERÇEK araç
    /// adlarından okunuyor; harita ile set arasında sapma kavramsal olarak
    /// imkânsız.
    private var oturumAracAdlari: Set<String> = []

    /// Zenginleştiriciyi bu turun durumuyla besler ve seyir satırını açar.
    /// Seyir kaydedici `ModelServisi`de kaldığı için beceri adı geri bildirim
    /// olarak döner; satırı burada açarız (sıra değişmez: yönlendirme → beceri).
    private func istemZenginlestir(_ question: String) -> String {
        let outcome = zenginlestirici.enrich(
            question,
            oturumAracAdlari: oturumAracAdlari,
            veriRefleri: dataStore.referanslar,
            ekliBelgeAdi: documentContext.runnableDocument?.name,
            aktifDil: aktifDil)
        if let name = outcome.eklenenBeceri {
            timeline.begin(kind: .enrichment, text: Self.beceriEklendi(name))
        }
        return outcome.prompt
    }

    private func oturumKur(profile: Profile, resuming summary: String? = nil) {
        aktifProfil = profile
        // Yeni oturum = yeni bağlam: enjekte edilmiş beceriler ve notlar artık
        // transcript'te yok, ikisi de yeniden enjekte edilebilir olmalı.
        zenginlestirici.reset()
        // Kirli oturum bayrağı BİLEREK taşınır (mcp §5.6): `ozet` metni kişisel
        // veri içerebilir, dolayısıyla yeni session da kirlidir. `AracYurutucu`
        // bayrağı yalnızca `sohbetiSifirla`da temizlediği için burada yapılacak
        // bir şey yoktur — bu yorum o sessiz bağımlılığı görünür kılar.
        let language = aktifDil
        let secildi = dilSecildi
        let tools = araclariYap(profile)
        // Beceri kapısının ve ekli-belge satırının okuduğu tek kaynak.
        oturumAracAdlari = Set(tools.map(\.name))
        let temel = LanguageModelSession(tools: tools) {
            // ÇEKİRDEK + PROFİL EKİ (P1-1): bu oturumun araç setinde anlamı
            // olmayan hiçbir kural taşınmaz.
            Router.instructions(profile)
            if !language.isEmpty {
                // Yanıt-dili çapası: adlandırılmış dil direktifi (sızıntıyı azaltır).
                //
                // ARAÇ ÇIKTISI AÇIKÇA ANILIR. Ölçülen sürüklenme: web araması gibi
                // araçlar bağlama İngilizce bir blok bırakıyor ("found 5 results
                // for …" + yabancı özetler) ve bu blok üretimden HEMEN ÖNCE geldiği
                // için 3B model kullanıcının diline değil o bloğun diline uyuyordu.
                // Dili araç çıktısıyla ilişkilendirerek adlandırmak, tek satırlık
                // genel "reply in X" direktifinden ölçülür biçimde daha iyi tutuyor.
                let capa = secildi
                    ? "The user has chosen \(language) as the reply language. Reply ONLY in \(language), never in another language, even if the user writes in a different language."
                    : "The user is writing in \(language). Reply ONLY in \(language), never in another language."
                // KISALTILDI (P1-1): eski metin sekiz satırdı ve aynı şeyi üç
                // kez söylüyordu. Ölçülen etkiyi taşıyan tek fikir korundu —
                // "araç çıktısı veri, dil örneği değil"; gerisi tekrardı.
                """
                \n\n\(capa)
                Tool results are data, not a language example: they are often English. \
                Read them, then write your answer in \(language), translating every label, \
                weekday and month you take from them.
                """
            }
            if let summary {
                "\n\nSummary of the earlier conversation: \(summary)"
            }
        }
        session = temel
        oturumDili = language
        oturumDilSecildi = secildi
        oturumAracImzasi = aracImzasi(profile)
        // Prewarm: executor'ı ısıt, ilk-token gecikmesini düşür (rapor §5.1).
        temel.prewarm()
    }

    /// Kullanıcı mesajının dilini cihaz-üstü saptar (NaturalLanguage). Kısa/belirsizde nil.
    private func algilananDil(_ text: String) -> String? {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard t.count >= 2 else { return nil }
        let tanıyıcı = NLLanguageRecognizer()
        tanıyıcı.processString(t)
        guard let language = tanıyıcı.dominantLanguage else { return nil }
        let olasilik = tanıyıcı.languageHypotheses(withMaximum: 1)[language] ?? 0
        guard olasilik >= 0.5 else { return nil }
        return Self.dilAdlari[language.rawValue]
    }

    private static let dilAdlari: [String: String] = [
        "tr": "Turkish", "en": "English", "zh-Hans": "Chinese", "zh-Hant": "Chinese",
        "zh": "Chinese", "ja": "Japanese", "es": "Spanish", "de": "German",
        "fr": "French", "ko": "Korean", "pt": "Portuguese", "pt-BR": "Portuguese",
        "it": "Italian",
    ]

    /// Kullanıcı bir yanıt dili seçtiyse onun İngilizce adı; "otomatik" ise nil.
    /// Seçim varken dil saptama devre dışıdır — tercih her turda kazanır.
    private var secilenDilAdi: String? {
        let code = LanguagePreference.shared.replyLanguage
        guard !code.isEmpty else { return nil }
        return Self.dilAdlari[code]
    }

    /// Sohbet yüzeyi görünür olunca çağrılır — kullanıcı yazmadan modeli ısıtır.
    func prepare() { session?.prewarm() }

    /// Yeni sohbete geçişte model bağlamını, veri deposunu ve ekli belgeyi sıfırlar.
    func resetChat() {
        // Hafıza ayıklaması TAM BURADA tetiklenir (hafiza-spec §4.1): tur içinde
        // asla, sohbetten çıkarken bir kez. Ayıklama ayrı ve kısa ömürlü bir
        // oturumda çalışır; aşağıdaki sıfırlama onu etkilemez.
        hafizaTetigi?()
        // Uçuştaki üretim de kesilmeli: yalnızca dış görev iptal edilirse `isProducing`
        // true takılı kalır ve yeni sohbette gönder düğmesi stop ikonunda donardı.
        stop()
        dataStore.clear()
        documentContext.removeDocument()
        documentContext.uretimiUnut()
        documentContext.toPreview = nil
        // GERÇEK sohbet sıfırlaması: `newTurn()` değil. Kirli oturum bayrağı ve
        // ret önbelleği oturum ömürlüdür (mcp §5.6, §3.3) ve ancak burada biter;
        // `newTurn()` çağırmak yeni sohbeti eski sohbetin kirliliğiyle başlatırdı.
        executor.resetChat()   // turKancasi kod deneme sayacını da sıfırlar
        timeline.reset()
        // Sıfırlama saptanan dili unutur ama kullanıcının açık seçimini EZMEZ.
        aktifDil = secilenDilAdi ?? ""
        dilSecildi = secilenDilAdi != nil
        zenginlestirici.reset()
        oturumAracAdlari = []
        // Yeni sohbetin transcript'i başka: eski özet önbelleği tutulmaz.
        ozetOnbellegi = nil
        sonButceOlcumHatasi = nil
        // Yönlendirme sinyalleri de oturum ömürlü: yeni sohbet, önceki sohbetin
        // arama/bağlantı çipleri yüzünden cihaz dışı profille başlamamalı.
        oncekiTurArama = false
        oncekiTurBaglanti = false
        aktifProfil = .gundelik
        // Tembel: oturumu şimdi kurma. İlk mesajda dil saptanıp tek seferde kurulur
        // (yeni sohbet başına çift kurulumu önler).
        session = nil
        oturumDili = ""
        oturumDilSecildi = false
        oturumAracImzasi = ""
    }

    /// Yönlendirme kararının tüm girdilerini o anki durumdan toplar. Karar
    /// mantığı `NiyetSecici`de yaşar ve örnek durumu OKUMAZ; bu fonksiyon
    /// durum ile saf seçici arasındaki TEK köprüdür.
    private func niyetGirdisi(_ question: String, available: Profile) -> IntentPicker.Input {
        IntentPicker.Input(question: question,
                          mevcutProfil: available,
                          belgeVar: documentContext.runnableDocument != nil,
                          aramaKullanilabilir: aramaKullanilabilir,
                          baglantiKullanilabilir: baglantiKullanilabilir,
                          connectionName: connectionName,
                          oncekiTurArama: oncekiTurArama,
                          oncekiTurBaglanti: oncekiTurBaglanti)
    }

    /// Niyet sınıflandırması (spec §7.3.1). Karar `NiyetSecici.sec`te; burada
    /// yalnızca gündelik araç setini biçimlendiren iki bayrak örneğe yazılır.
    /// Bayraklar DÖNÜŞTEN ÖNCE kurulur: `aracImzasi` ikisini de okur ve seti
    /// bu turun kararına göre imzalar.
    private func intentProfile(_ question: String, available: Profile) -> Profile {
        let outcome = IntentPicker.select(niyetGirdisi(question, available: available))
        yerelAramayiGizle = outcome.yerelAramayiGizle
        kisiSinyaliVar = outcome.kisiSinyaliVar
        return outcome.profile
    }

    // MARK: - Tur-içi profil kurtarma (denetim P1-2)

    /// Deterministik seçici yanıldığında denenecek İKİNCİ profil
    /// (`NiyetSecici.ikinci`). `kisiSinyaliVar` bu turun birinci seçiminde
    /// hesaplanmış değerdir — seçiciye parametre olarak verilir.
    private func ikinciProfil(_ question: String, birinci: Profile) -> Profile? {
        IntentPicker.second_pass(niyetGirdisi(question, available: birinci),
                           birinci: birinci,
                           kisiSinyaliVar: kisiSinyaliVar)
    }

    /// "Yapamadım" kalıpları — 8 dilde YETENEK reddi. Kasıtlı olarak DAR:
    /// olgusal "bulamadım" (aranmış ama sonuç yok) BURAYA GİRMEZ, çünkü o
    /// doğru ve dürüst bir yanıttır; yeniden çalıştırmak modeli aynı boş
    /// sonuca ikinci kez götürüp pil yakmaktan başka bir şey yapmaz.
    private static let yapamadimIzleri = [
        "yapamıyorum", "yapamam", "yapamadım", "edemiyorum", "edemem",
        "bu konuda yardımcı olamıyorum", "yardımcı olamam", "erişimim yok",
        "yeteneğim yok", "elimde böyle bir araç", "bir aracım yok",
        "i can't", "i cannot", "i'm unable", "i am unable", "i don't have access",
        "i don't have the ability", "i'm not able",
        "no puedo", "ich kann nicht", "je ne peux pas", "não posso",
        "できません", "无法", "할 수 없",
    ]

    /// Tur-içi kurtarma tetiği. ÜÇ şart birden:
    /// 1. Hiç araç çalışmadı — bir araç çalıştıysa profil doğruydu, yanıtın
    ///    içeriği tartışmalı olabilir ama YETENEK boşluğu yok.
    /// 2. Metin bir yetenek reddi kalıbı taşıyor.
    /// 3. (çağıran tarafta) bu turda daha önce kurtarma denenmedi.
    ///
    /// Saf ve statik: modelsiz test edilir.
    static func kurtarmaGerekli(traces: [ToolTrace], text: String) -> Bool {
        guard traces.isEmpty else { return false }
        let m = text.lowercased()
        return yapamadimIzleri.contains(where: m.contains)
    }

    /// Metinsiz biten bir turu sınıflar: turda düşmüş bir araç varsa arıza
    /// ARAÇtadır (`.aracDustu`), yoksa model hiç cümle kurmamıştır
    /// (`.bosYanit`). Saf ve statik: modelsiz test edilir.
    static func dususSinifi(traces: [ToolTrace]) -> ErrorClass {
        let dusenVar = traces.contains {
            if case .failed = $0.state { return true }
            return false
        }
        return dusenVar ? .aracDustu : .bosYanit
    }

    /// Sınıftan kullanıcı cümlesine tek eşleme noktası. Metin seçimi TEK
    /// yerde durur ki yeni bir sınıf eklenince cümlesiz kalması derleyicide
    /// görünsün. Saf: modelsiz test edilir.
    static func dususMetni(_ errorClass: ErrorClass) -> String {
        switch errorClass {
        case .aracDustu:      return L10n.toolFailedReply
        case .baglamTasmasi:  return L10n.conversationTooLong
        case .sinirDisi:      return L10n.outOfBounds
        case .dilDisi:        return L10n.languageUnsupported
        case .yazmaSonrasi:   return L10n.errorAfterWrite
        case .bosYanit:       return L10n.answerNotAssembled
        case .uretimHatasi, .none: return L10n.tryAgain
        }
    }

    /// Arama sunucusu tanımlı ve açık mı. `aktifMi` adres geçersizse zaten
    /// false döner — "açık ama çalışmayan" ara durum yoktur.
    private var aramaKullanilabilir: Bool { WebSearchSetting.isActive }

    /// Bağlantı seçili ve en az bir aracı oturuma girebiliyor mu.
    private var baglantiKullanilabilir: Bool { !mcpAraclari.isEmpty }

    /// Önceki turda arama çipi düştü mü (web-arama §5.4 sinyali). Takip
    /// sorusu ("peki yarın?") hiçbir anahtar kelime içermez.
    private var oncekiTurArama = false
    /// Önceki turda MCP çipi düştü mü (mcp §5.4 sinyali).
    private var oncekiTurBaglanti = false

    /// AÇIK aritmetik niyeti (denetim küme 1). Mantık `NiyetSecici`ye taşındı;
    /// bu ileti noktası `OtoTestVakalari`nin çağrı biçimini korur.
    static func hesapNiyeti(_ s: String) -> Bool { IntentPicker.hesapNiyeti(s) }

    /// Turun çiplerinden bir sonraki turun yönlendirme sinyallerini çıkarır.
    /// İkon, çipi düşüren araca aittir ve modelin ürettiği bir metin DEĞİLDİR —
    /// bu yüzden halüsinasyona kapalı bir sinyaldir.
    private func turSinyalleriniGuncelle() {
        oncekiTurArama = executor.traces.contains { $0.icon == "globe" }
        oncekiTurBaglanti = executor.traces.contains { $0.icon == "arrow.up.forward.app" }
    }

    // MARK: - Seyir satır metinleri (seyir-spec §2.4)
    //
    // Küçük harf, gerçek zaman, deterministik olay. Uydurma durum fiili yok:
    // yalnızca kodda GERÇEKTEN olan şey yazılır.

    private static func yonlendirildi(_ profile: Profile) -> String {
        String(localized: "routed · \(profile.seyirAdi)")
    }
    private static func beceriEklendi(_ name: String) -> String {
        String(localized: "skill added · \(name)")
    }
    /// Canlı yazım adımının metni. ŞİMDİKİ zaman: adım açıldığında yazım hâlâ
    /// sürüyor. Geçmiş zamana çeviren yer `SeyirDefteri.kalici`dir ve o çeviriyi
    /// metne bakarak yapar — bu yüzden dizge `SeyirDefteri.yaziyorMetni` ile
    /// birebir aynı kalmalı.
    private static var yaziyorMetni: String { String(localized: "writing") }

    // MARK: - Üretim / iptal

    /// Canlı akış görevi. Kullanıcı "dur" derse bu iptal edilir.
    private var uretimGorevi: Task<String, Error>?

    /// Süren özetleme görevi. `durdur()` bunu DA iptal eder: `ozetle()` tam bir
    /// LLM çağrısıdır ve yalnız `uretimGorevi` iptal edilseydi, özetleme
    /// sırasında basılan "dur" hiçbir şeyi kesmezdi (ölçülen bekleme: saniyeler).
    private var ozetGorevi: Task<String?, Never>?

    /// Bir turun akışında o ana kadar birikmiş metin — iptal edildiğinde
    /// kullanıcıdan SAKLANMAZ, yarım yanıt ekranda kalır (sessizce silmek
    /// kullanıcının okuduğunu geri almaktır).
    ///
    /// TUR BAŞINA YENİ ÖRNEK, örnek alanı DEĞİL. Alan olduğunda her `akisYut`
    /// onu sıfırlıyordu: iptal edilen eski turun `hataKurtar`ı `iptalSonucu()`a
    /// düştüğünde, o sırada başlamış YENİ turun kısmi metnini (ya da boşunu)
    /// okuyup eski turun Mesaj'ına yazıyordu. `uretimNo` nesil sayacı hangi
    /// turun konuştuğunu ayırıyor; bu kutu da hangi metnin o tura ait olduğunu.
    private final class StreamBuffer { var text = "" }

    /// Canlı üretim var mı — SohbetGorunumu gönder/dur düğmesini buna göre çizer.
    private(set) var isProducing: Bool = false

    /// Çalışan turun kimliği. `durdur()` bunu ilerletir; böylece iptal edilmiş turun
    /// geç gelen `defer`'ı, o sırada başlamış YENİ turun `isProducing` bayrağını düşüremez.
    private var uretimNo = 0

    /// Üretimi iptal eder. O ana kadar akan metin korunur; `yanitla` yarım metinle
    /// normal şekilde döner (hata değil, iptal).
    ///
    /// `isProducing` DERHAL false olur: alttaki akışın sönmesini beklemek, kullanıcı
    /// "dur"a bastıktan sonra gönder düğmesini saniyelerce stop ikonunda dondurup
    /// yeni isteği sessizce reddediyordu.
    func stop() {
        uretimGorevi?.cancel()
        uretimGorevi = nil
        // Özetleme de kesilir: "dur"un kestiği şey turun TAMAMI olmalı, yalnız
        // akış değil (bkz. `ozetGorevi`).
        ozetGorevi?.cancel()
        ozetGorevi = nil
        uretimNo &+= 1          // eski turun defer'ı artık bayrağa dokunamaz
        isProducing = false
        // BEKLEYEN ONAY DA ÇÖZÜLÜR. `AracYurutucu.onayDevami` bir
        // `withCheckedContinuation`dır ve Task iptalinden ETKİLENMEZ: yalnız
        // görevi iptal etmek, onay sayfasını ekranda bırakıp araç görevini bir
        // sonraki mesaja kadar askıda sızdırıyordu. Karar `.iptal`dir, ret
        // değil — kaynak kara listeye alınmaz (bkz. `onayKarariniIste`).
        executor.bekleyenOnayiCoz()
        // Sessiz kaybolma yok (seyir-spec §3.4): o ana kadarki adımlar kalır,
        // sona kapalı bir "yarıda kaldı" satırı eklenir.
        seyriEsitle()
        timeline.interrupt()
    }

    // MARK: - Seyir köprüsü

    /// `AracYurutucu`daki çipleri seyre araç adımı olarak yansıtır.
    ///
    /// Doğru yer `AracYurutucu.baslat`tır (seyir-spec §5.2: "Seyir'e tek ek
    /// satır, `baslat` anında adım açmaktır") — o dosya bu fazda başka bir
    /// ajana ait olduğu için köprü burada kuruldu. OLAY tabanlı bağlamayı
    /// `SohbetGorunumu` yapıyor (`onChange(of: yurutucu.izler)`), yani ekranda
    /// adım anında açılıyor; buradaki yoklama görünümsüz çağrılar (DilTesti,
    /// Degerlendirme) için emniyet ağıdır. Adım sırası korunur: araç
    /// çağrıları akış parçalarının ARASINDA çözülür ve `seyriEsitle` her
    /// parçada çalışır. `traceID`ye göre tekilleştirdiği için `baslat` içine
    /// doğrudan çağrı eklendiğinde bu köprü kendiliğinden etkisizleşir —
    /// çift adım üretmez, silinmesi gerekmez.
    private func seyriEsitle() {
        let bagli = Set(timeline.steps.compactMap(\.toolTraceID))
        for trace in executor.traces where !bagli.contains(trace.id) {
            timeline.bindTool(traceID: trace.id)
        }
    }

    // MARK: - Yanıt (streaming)

    /// Eski çağrı biçimi (DilTesti/Degerlendirme): yalnızca metin + çipler.
    func yanitla(_ question: String, stream: @escaping (String) -> Void) async -> (text: String, traces: [ToolTrace]) {
        let s = await replyOutcome(question, stream: stream)
        return (s.text, s.traces)
    }

    /// Kullanıcı sorusuna akışlı yanıt üretir. `akis` her kısmi metinle çağrılır.
    /// Dönüş, metnin YANINDA hata/tekrar bayraklarını taşır — UI metin karşılaştırmaz.
    func replyOutcome(_ question: String, stream: @escaping (String) -> Void) async -> ReplyOutcome {
        // Hazır değilsek sessizce yeniden bak: model indirmesi bitmiş ya da kullanıcı
        // Ayarlar'dan Apple Intelligence'ı açmış olabilir (uygulama yeniden başlamadan).
        if !state.isReady { reloadAvailability() }
        guard state.isReady else {
            return ReplyOutcome(text: blockMessage, traces: [],
                               isError: true, isRetryable: engelTekrarDenenebilir)
        }
        // Paralel istek kilidi (rapor §5.1). Kendi bayrağımıza dayanır: `isResponding`
        // iptalden sonra bir süre true kaldığı için kullanıcıyı boşuna kilitliyordu.
        if isProducing {
            return ReplyOutcome(text: L10n.previousFinishing, traces: [], isTransient: true)
        }
        uretimNo &+= 1
        let benimTur = uretimNo
        // Bu turun akış tamponu. Turla birlikte doğar, turla birlikte ölür.
        let buffer = StreamBuffer()
        isProducing = true
        // İptal edilmiş (uretimNo ilerlemiş) bir turun geç biten defer'ı, o sırada
        // başlamış yeni turun bayrağını düşürmemeli.
        defer { if uretimNo == benimTur { isProducing = false } }
        // Sinyaller ÖNCEKİ turun çiplerinden okunur; çipler sıfırlanmadan önce.
        turSinyalleriniGuncelle()
        // Yuva alaka sıralamasının girdisi (P1-6). Profil kararından ÖNCE
        // yazılır: `aracImzasi` bu turun sorusuna göre seçilen araçları
        // yazacak ve seçim değiştiyse oturum yeniden kurulacak.
        sonSoru = question
        executor.newTurn()   // turKancasi kod deneme sayacını da sıfırlar (kod-spec §5.4)
        timeline.reset()

        // Profil + dil yönlendirmesi: oturum yoksa ya da profil/dil değişince tek seferde kur.
        let istenen = intentProfile(question, available: aktifProfil)
        timeline.begin(kind: .routing, text: Self.yonlendirildi(istenen))
        // Tercih varsa saptama çalışmaz; tercih değişince aktifDil de değişir ve
        // aşağıdaki `aktifDil != oturumDili` koşulu oturumu yeni dille yeniden kurar.
        if let secilen = secilenDilAdi {
            aktifDil = secilen
            dilSecildi = true
        } else {
            // Açık seçimden Otomatik'e dönüşte zorlanmış dil takılı kalmasın:
            // saptama temiz sayfadan başlasın, aksi halde eşiği geçemeyen kısa
            // girdilerde yanıt eski seçilen dilde gelmeye resuming ederdi.
            if dilSecildi { aktifDil = "" }
            dilSecildi = false
            aktifDil = algilananDil(question) ?? aktifDil
        }
        if session == nil || istenen != aktifProfil || aracImzasi(istenen) != oturumAracImzasi
            || aktifDil != oturumDili || dilSecildi != oturumDilSecildi {
            oturumKur(profile: istenen, resuming: await summarize())
        }
        await butceKontrol()
        // Kurulum + özetleme + ölçüm hepsi await: bu pencerede basılan "dur"
        // eskiden hiçbir şeyi kesmiyor, üretim yine de başlıyordu. İptal hata
        // değildir — akan metin (henüz yoksa boş) neyse o döner.
        guard uretimNo == benimTur else { return iptalSonucu(buffer.text) }
        guard let session else {
            return ReplyOutcome(text: blockMessage, traces: [],
                               isError: true, isRetryable: engelTekrarDenenebilir)
        }

        // Eşleşen beceri kılavuzu + hafıza notları bu turun istemine iliştirilir
        // (her ikisi de oturum başına bir kez).
        let prompt = istemZenginlestir(question)
        do {
            let raw = try await akisYut(session, question: prompt, buffer: buffer, stream: stream)
            // Ayrıştırılamamış araç çağrısı kullanıcıya GİTMEZ.
            var sonMetin = Self.aracSizintisiniTemizle(raw)

            // TUR-İÇİ PROFİL KURTARMA (P1-2). Araç izi YOK + "yapamıyorum"
            // kalıbı VAR = deterministik seçici büyük olasılıkla yanıldı ve
            // gereken araç bu oturumda hiç bulunmuyor. Bir kez, ikinci en
            // olası profille tekrar denenir. `seyir.bitir()`den ÖNCE olmak
            // zorunda: kaydedici kapandıktan sonra yeni adım açılamaz.
            if Self.kurtarmaGerekli(traces: executor.traces, text: sonMetin),
               uretimNo == benimTur,
               let second_pass = ikinciProfil(question, birinci: aktifProfil) {
                sonMetin = await profilKurtar(second_pass, question: question, benimTur: benimTur,
                                              ilkMetin: sonMetin, stream: stream)
            }

            // Normal tamamlanma ya da iptal (yarım metin) — ikisi de hata DEĞİL.
            // İptal edilmiş turda `durdur()` seyri zaten kesti; `bitir()` kapalı
            // kaydediciye dokunmaz.
            seyriEsitle()
            timeline.finish()
            // Geriye yalnız sızıntı kalmışsa turda söylenecek bir şey yok:
            // yarım JSON göstermektense tekrar denenebilir hata daha dürüst.
            //
            // AMA "hangi hata" ayrımı burada yapılır: turda bir araç DÜŞTÜYSE
            // bu bir üretim arızası değil, araç arızasıdır ve kullanıcı zaten
            // çipte görüyor. Aynı cümleyi ikisine de vermek, ölçümde beş
            // vakanın beşini de aynı metne düşürüp sebebi görünmez kılmıştı.
            if sonMetin.isEmpty, !raw.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                stream("")
                let errorClass = Self.dususSinifi(traces: executor.traces)
                return ReplyOutcome(text: Self.dususMetni(errorClass), traces: executor.traces,
                                   isError: true, isRetryable: executor.retryGuvenli,
                                   errorClass: errorClass)
            }
            if sonMetin != raw { stream(sonMetin) }
            return ReplyOutcome(text: sonMetin, traces: executor.traces)
        } catch {
            let outcome = await hataKurtar(error, question: question, benimTur: benimTur,
                                         buffer: buffer, stream: stream)
            seyriEsitle()
            timeline.finish()
            return outcome
        }
    }

    /// İkinci profille turu BİR KEZ yeniden çalıştırır. Kurtarma tutmazsa
    /// `ilkMetin` geri gelir — girişim kullanıcının elindeki yanıtı asla
    /// kötüleştirmez, en kötü ihtimalle bir üretim süresi harcar.
    ///
    /// Hangi metinle dönerse dönsün ekrana O metin yazılır (`akis`), çünkü
    /// ilk denemenin metni akıştan silinmiş oluyor.
    ///
    /// Özet TAŞINIR: konuşma bağlamını düşürmek kurtarmanın kendisinden daha
    /// büyük bir kayıp olurdu. Özete giren "yapamıyorum" cümlesinin ikinci
    /// denemeyi yanlı kılma riski var; buna karşı ikinci denemede araç
    /// GERÇEKTEN oturumda olduğu için modelin önünde somut bir yol duruyor.
    private func profilKurtar(_ second_pass: Profile,
                              question: String,
                              benimTur: Int,
                              ilkMetin: String,
                              stream: @escaping (String) -> Void) async -> String {
        stream("")   // ilk denemenin "yapamıyorum" metnini ekrandan kaldır
        // Kurtarma turu: çipler ve yan etki bayrakları KORUNUR (P2-8).
        executor.newTurn(yanEtkiyiUnut: false)
        oturumKur(profile: second_pass, resuming: await summarize())
        // Özetleme uzun bir await: bu sırada "dur" basıldıysa ikinci üretimi
        // hiç başlatma — iptal edilmiş turun görünmez bir devamı olurdu.
        guard uretimNo == benimTur else {
            stream(ilkMetin)
            return ilkMetin
        }
        timeline.begin(kind: .routing, text: Self.yonlendirildi(second_pass))
        // İstem YENİDEN kurulur: oturum değişti, dolayısıyla yeni profilin
        // beceri kılavuzu ve ekli-belge satırı bu sette geçerli olanlardır.
        if let new = session,
           let raw = try? await akisYut(new, question: istemZenginlestir(question),
                                        buffer: StreamBuffer(), stream: stream) {
            let text = Self.aracSizintisiniTemizle(raw)
            // Boş ya da yine "yapamıyorum" ise kurtarma tutmadı.
            if !text.isEmpty,
               !Self.kurtarmaGerekli(traces: executor.traces, text: text) {
                stream(text)
                return text
            }
        }
        stream(ilkMetin)
        return ilkMetin
    }

    /// Hata taksonomisi (rapor §5.5): taşma → görünmez kurtarma; guardrail/dil → retry YOK.
    private func hataKurtar(_ error: Error,
                            question: String,
                            benimTur: Int,
                            buffer: StreamBuffer,
                            stream: @escaping (String) -> Void) async -> ReplyOutcome {
        stream("")  // yarım akan metni temizle
        // Retry, AYNI istemi ikinci kez gönderir. Bu turda bir araç dünyayı zaten
        // değiştirdiyse (etkinlik yazıldı, hatırlatıcı kuruldu, belge üretildi)
        // ikinci deneme aynı yan etkiyi TEKRARLAR — çift etkinlik, çift hatırlatıcı.
        //
        // YEREL yan etki tek eksen DEĞİL (denetim P0-3): uzak bir MCP yazması
        // (Jira issue, kayıt, e-posta) `.yazildi` çipi düşürmez ve `dunyaDegisti`
        // bayrağını hiç kurmazdı, dolayısıyla o yan etkiden SONRA oluşan genel
        // bir hata retry'a giriyor ve ikinci issue açılıyordu. `retryGuvenli`
        // iki ekseni birden okur.
        if !executor.retryGuvenli {
            // Hata balonu evet; "yeniden dene" HAYIR — yan etki tekrarlanırdı.
            return ReplyOutcome(text: L10n.errorAfterWrite, traces: executor.traces,
                               isError: true, isRetryable: false,
                               errorClass: .yazmaSonrasi)
        }
        if let g = error as? LanguageModelSession.GenerationError {
            switch g {
            case .guardrailViolation:
                // Kurtarılamaz — pil yakmadan tek cümle (retry yok).
                return ReplyOutcome(text: L10n.outOfBounds, traces: executor.traces,
                                   isError: true, isRetryable: false,
                                   errorClass: .sinirDisi)
            case .unsupportedLanguageOrLocale:
                return ReplyOutcome(text: L10n.languageUnsupported, traces: executor.traces,
                                   isError: true, isRetryable: false,
                                   errorClass: .dilDisi)
            case .exceededContextWindowSize:
                // Kurtarılabilir: özetle, oturumu yeniden kur, bir kez dene.
                guard uretimNo == benimTur else { return iptalSonucu(buffer.text) }
                executor.newTurn(yanEtkiyiUnut: false)
                oturumKur(profile: aktifProfil, resuming: await summarize())
                // Özetleme uzun sürerken "dur" basılmış olabilir.
                guard uretimNo == benimTur else { return iptalSonucu(buffer.text) }
                if let new = session,
                   let m = try? await akisYut(new, question: question,
                                              buffer: StreamBuffer(), stream: stream) {
                    return ReplyOutcome(text: m, traces: executor.traces)
                }
                return ReplyOutcome(text: L10n.conversationTooLong, traces: executor.traces,
                                   isError: true, isRetryable: true,
                                   errorClass: .baglamTasmasi)
            default:
                break
            }
        }
        // Diğer geçici hatalar: taze oturumla bir kez daha dene.
        guard uretimNo == benimTur else { return iptalSonucu(buffer.text) }
        executor.newTurn(yanEtkiyiUnut: false)
        oturumKur(profile: aktifProfil)
        if let new = session,
           let m = try? await akisYut(new, question: question,
                                      buffer: StreamBuffer(), stream: stream) {
            return ReplyOutcome(text: m, traces: executor.traces)
        }
        // Retry de tutmadı. Turda düşmüş bir araç varsa kullanıcının gördüğü
        // arıza ODUR (çip zaten orada); yoksa arıza üretim tarafındadır.
        let errorClass: ErrorClass =
            Self.dususSinifi(traces: executor.traces) == .aracDustu ? .aracDustu : .uretimHatasi
        return ReplyOutcome(text: Self.dususMetni(errorClass), traces: executor.traces,
                           isError: true, isRetryable: true,
                           errorClass: errorClass)
    }

    /// İPTAL EDİLMİŞ TURUN KURTARMASI YAPILMAZ (denetim P1-7).
    ///
    /// `hataKurtar` eskiden `uretimNo`ya hiç bakmıyordu: kullanıcı "dur"a
    /// bastıktan sonra gelen hata yeni bir `oturumKur` + `akisYut` başlatıyor,
    /// yani görünmez bir üretim yarışı açıyordu. Daha kötüsü `oturumKur`
    /// `aktifProfil`i yazdığı için o sırada başlamış YENİ turun oturumunu
    /// ezebiliyordu.
    ///
    /// İptal hata DEĞİLDİR: `isError` false, `isRetryable` false — akan
    /// yarım metin ekranda kalır, kullanıcı zaten kendi durdurduğunu biliyor.
    ///
    /// Metin PARAMETREDİR: tur başında yakalanan tamponun kopyası verilir.
    /// Paylaşılan bir alandan okunsaydı yeni turun metnini eski turun sonucuna
    /// yazma yarışı açık kalırdı.
    private func iptalSonucu(_ kismiMetin: String) -> ReplyOutcome {
        ReplyOutcome(text: kismiMetin, traces: executor.traces)
    }

    private func akisYut(_ session: LanguageModelSession,
                         question: String,
                         buffer: StreamBuffer,
                         stream: @escaping (String) -> Void) async throws -> String {
        buffer.text = ""
        // Akış ayrı bir Task'ta yürür ki `durdur()` onu iptal edebilsin.
        let task = Task { @MainActor [weak self] () throws -> String in
            var last = ""
            // Akış hiç parça üretmeden uzun sürebilir; ilk parçayı beklemeden de
            // iptali onurlandır (checkCancellation yalnız döngü içinde kalırsa
            // "dur" ilk token gelene kadar etkisiz kalıyordu).
            try Task.checkCancellation()
            let responseStream = session.streamResponse(to: question, options: Self.uretimSecenekleri)
            var ilkParca = true
            for try await chunk in responseStream {
                // Kullanıcı "dur" dediyse burada çıkarız; tampon son gördüğü
                // metni tutar, üstteki catch onu geri döndürür.
                try Task.checkCancellation()
                // Araç adımları parça sınırlarında yakalanır: araç çağrıları
                // akışın parçaları ARASINDA çözülür, yani burada sıraları doğrudur.
                self?.seyriEsitle()
                if ilkParca {
                    // Tek adım — parça başına DEĞİL. Yazım başladığı an bir kez.
                    ilkParca = false
                    self?.timeline.begin(kind: .writing, text: ModelService.yaziyorMetni)
                }
                last = chunk.content
                // Tampon TURUN kendisine ait; `self` üzerinden paylaşılmaz.
                buffer.text = last
                stream(last)
            }
            return last
        }
        uretimGorevi = task
        defer { uretimGorevi = nil }
        do {
            return try await task.value
        } catch is CancellationError {
            // İptal hata değildir: yarım yanıt ekranda kalsın, kullanıcı okusun.
            return buffer.text
        }
    }

    // MARK: - Bağlam bütçesi (rapor §5.2 — gerçek token ölçümü)

    /// Ölçüm düşerse dolan teşhis kanalı. Bütçe kontrolü sessizce atlanınca
    /// taşma bir tur sonra `exceededContextWindowSize` olarak patlıyor ve
    /// geriye dönüp "ölçüm mü düştü, eşik mi yanlış" sorusunu yanıtlayacak
    /// hiçbir iz kalmıyordu. Kullanıcıya GÖSTERİLMEZ (karar yerinde) — yalnız
    /// eval/DEBUG okur.
    private(set) var sonButceOlcumHatasi: String?

    /// Transcript'in gerçek token sayısını `tokenCount` ile ölçer; contextSize'ın
    /// %80'ini aşınca oturumu özetle yeniden kurar. Tahmin değil, ölçüm.
    private func butceKontrol() async {
        let kind = uretimNo
        guard let session else { return }
        // contextSize 26.0'da var, tokenCount 26.4'te geldi. Dağıtım hedefi 26.0
        // olduğu için ölçüm sürüm kapısının arkasında: 26.0–26.3'te bütçe
        // kontrolü yapılamaz, taşımayı FoundationModels'ın kendi hatası bildirir.
        guard #available(iOS 26.4, *) else {
            sonButceOlcumHatasi = "tokenCount iOS 26.4 gerektirir"
            return
        }
        let threshold = Int(Double(models.contextSize) * esikOran)
        let number: Int
        do {
            number = try await models.tokenCount(for: session.transcript)
        } catch {
            // `try?` idi: düşen ölçüm bütçe kontrolünü SESSİZCE atlıyordu.
            // Karar aynı (kullanıcıya hata gösterme, tur resuming etsin) ama artık
            // iz bırakıyor.
            sonButceOlcumHatasi = String(describing: error)
            #if DEBUG
            print("Tacet: token ölçümü düştü, bütçe kontrolü atlandı — \(error)")
            #endif
            return
        }
        sonButceOlcumHatasi = nil
        guard number > threshold else { return }
        // Ölçüm de bir await: bu sırada "dur" basılmış olabilir, o zaman
        // özetleyip yeniden kurmanın (saniyeler) hiçbir alıcısı yok.
        guard uretimNo == kind else { return }
        oturumKur(profile: aktifProfil, resuming: await summarize())
    }

    /// Bütçe aşımında yeni oturuma taşınacak ham tur sayısı (spec §146: son 4–6 tur korunur).
    private let korunanTurSayisi = 6

    /// Son özetin girdisi ve sonucu. Aynı transcript ikinci kez ÖZETLENMEZ.
    ///
    /// Bir kullanıcı turu birden çok kez oturum kurabiliyor (profil/dil/imza
    /// değişimi, bütçe aşımı, tur-içi profil kurtarma, hata kurtarma) ve her
    /// kurulum tam bir LLM özet çağrısı demekti. Girdi bire bir aynıysa sonuç
    /// da aynıdır; ikinci çağrı yalnızca ilk token gecikmesine yazılır.
    private var ozetOnbellegi: (input: String, outcome: String)?

    /// Eski geçmişi tek paragrafa özetletir; özetleme başarısız olursa en azından
    /// son turların ham metnini döndürür. Hiçbir koşulda bağlamı sessizce
    /// düşürmez — asistanın hafızasını kaybetmesi kullanıcı için görünmez bir
    /// arıza, en kötü hata türü.
    private func summarize() async -> String? {
        let kind = uretimNo
        guard let session else { return nil }
        let dokum = session.transcript.compactMap(Self.turMetni)
        guard !dokum.isEmpty else { return nil }

        // Son turların ham metni: hem özet istemine girdi hem de yedek bağlam.
        let sonTurlar = dokum.suffix(korunanTurSayisi).joined(separator: "\n")
        let hamKuyruk = "Recent conversation:\n" + Self.truncate(sonTurlar, 2000)

        // KISA GEÇMİŞTE ÖZET ÇAĞRISI YAPILMAZ. Korunan tur sayısına kadar olan
        // her şey zaten BİREBİR taşınıyor; üstüne bir de LLM özeti almak aynı
        // bilgiyi ikinci kez, daha kötü (3B model, sayılar yuvarlanıyor) ve
        // saniyeler pahasına üretmek olurdu. Konudan konuya geçen kullanıcıda
        // oturum sık yeniden kuruluyor ve o oturumlar tam da bu kısa olanlar —
        // ilk token gecikmesinin ana kaynağı buydu.
        guard dokum.count > korunanTurSayisi else { return hamKuyruk }

        let history = Self.truncate(dokum.suffix(24).joined(separator: "\n"), 4000)
        if let cache = ozetOnbellegi, cache.input == history { return cache.outcome }

        // Özet AYRI ve kısa bir oturumda üretilir. Eskiden bütçesi zaten dolmuş
        // oturuma bir istem daha ekleniyordu — özetin kendisi taşmayı büyütüyordu.
        //
        // AYRI GÖREVDE: `durdur()` bunu iptal edebilsin diye. Yalnız
        // `uretimGorevi` iptal edilirken burada beklenen saniyeler "dur"a
        // rağmen akıp gidiyordu.
        let task = Task { @MainActor () -> String? in
            let ozetleyici = LanguageModelSession {
                "You summarize conversations. Reply with ONE short paragraph, no preamble."
            }
            guard !Task.isCancelled,
                  let summary = try? await ozetleyici.respond(
                      to: "Conversation:\n\(history)\n\nSummarize it in one short paragraph.").content,
                  !Task.isCancelled,
                  !summary.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            else { return nil }
            return summary
        }
        ozetGorevi = task
        let summary = await task.value
        if uretimNo == kind { ozetGorevi = nil }

        // İptal edildiyse ya da özetleme düştüyse BAĞLAM SESSİZCE DÜŞMEZ: ham
        // kuyruk her koşulda taşınır (asistanın hafızasını kaybetmesi
        // kullanıcı için görünmez bir arıza, en kötü hata türü).
        guard let summary, uretimNo == kind else { return hamKuyruk }

        // ÖZET TEK BAŞINA TAŞINMAZ. Özeti üreten de 3B model ve olguyu ona
        // emanet etmek bu projenin 1. dersinin ihlali: namaz vakti, kur,
        // sefer saati gibi SAYILAR özetlenirken yuvarlanıyor, düşüyor ya da
        // makul görünen başka bir sayıya dönüşüyor — üstelik profil değişimi
        // tam da bu turlarda oluyor ("vakitleri bul" → "tablo yap").
        // Son turların ham metni birebir eklenir: özet bağlamı, ham kuyruk
        // olguyu taşır. `turMetni` araç çıktısını bağlama almadığı için
        // sayılar yalnızca asistanın kendi cümlelerinde duruyor; kaybolan
        // tam olarak buydu. Maliyet ~1200 karakter (≈400 token), sınırlı.
        let outcome = summary + "\n\nRecent turns (verbatim — use exactly as written):\n"
            + Self.truncate(sonTurlar, 1200)
        ozetOnbellegi = (history, outcome)
        return outcome
    }

    /// Transcript girdisinden düz metin çıkarır. Yalnızca kullanıcı/asistan turları
    /// alınır; araç çağrıları ve çıktıları bağlam olarak taşınmaz (hacimli ve
    /// yeni oturumda yeniden çağrılabilir).
    private static func turMetni(_ input: Transcript.Entry) -> String? {
        func plain(_ segmentler: [Transcript.Segment]) -> String {
            segmentler.compactMap { if case .text(let t) = $0 { return t.content } else { return nil } }
                .joined(separator: " ")
                .trimmingCharacters(in: .whitespacesAndNewlines)
        }
        switch input {
        case .prompt(let p):
            let m = plain(p.segments)
            return m.isEmpty ? nil : "User: " + truncate(m, 400)
        case .response(let r):
            // Sızıntı ÖZETE de girmemeli: girerse model onu kendi geçmişinde
            // geçerli çıktı sanıp bir sonraki oturumda tekrarlıyor.
            let m = aracSizintisiniTemizle(plain(r.segments))
            return m.isEmpty ? nil : "Assistant: " + truncate(m, 400)
        default:
            return nil
        }
    }

    private static func truncate(_ text: String, _ limit: Int) -> String {
        text.count <= limit ? text : String(text.suffix(limit))
    }

    // MARK: - Araç-çağrısı sızıntısı süzgeci

    /// Ayrıştırılamamış araç-çağrısı yükünü metinden ayıklar.
    ///
    /// FoundationModels çağrı bloğunu tanıyamadığında onu düz METİN segmenti
    /// olarak geçiriyor; süzgeç olmadan `{"name": "hesapla", …}<executable_end>`
    /// doğrudan kullanıcıya gidiyordu. Daha kötüsü kendini besliyordu: sızıntı
    /// bir `.response` metni olduğu için `ozetle()` onu özete taşıyor, model
    /// yeni oturumda kendi geçmişinde araç-çağrısı sözdizimini "geçerli asistan
    /// çıktısı" olarak görüp ALAKASIZ yükleri kopyalıyordu (laktoz sorusuna
    /// `hesapla`, gizlilik sorusuna `kisi("Ali")`). Bu yüzden süzgeç hem
    /// kullanıcıya giden metne hem `turMetni`ye uygulanır.
    ///
    /// Boş dönebilir — çağıran bunu "tur boşa gitti" olarak ele almalı.
    static func aracSizintisiniTemizle(_ text: String) -> String {
        var m = text
        // 1) ```function … ``` / … <executable_end> blokları (gövdesiyle birlikte).
        m = delete("(?s)```[ \\t]*function\\b.*?(?:```|<executable_end>|\\z)", m)
        // 2) Çıplak JSON araç çağrısı, tekil ya da [ … ] dizisi içinde.
        m = delete("(?s)\\[?\\s*\\{\\s*\"name\"\\s*:\\s*\"[^\"]*\"\\s*,\\s*\"arguments\"\\s*:\\s*\\{.*?\\}\\s*\\}\\s*\\]?", m)
        m = delete("<executable_(?:end|start)>", m)
        // 3) Yetim kapanış artıkları YALNIZCA yukarıda gerçekten bir çağrı
        //    soyulduysa temizlenir. Koşulsuz uygulanırsa MEŞRU bir markdown
        //    kod bloğunun kapanış ``` satırını da silip yanıtı bozardı —
        //    kod becerisi tam olarak böyle bloklar üretiyor.
        if m != text {
            m = delete("(?m)^\\s*(?:\\]|```)\\s*$", m)
        }
        return m.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func delete(_ pattern: String, _ text: String) -> String {
        guard let re = try? NSRegularExpression(pattern: pattern) else { return text }
        return re.stringByReplacingMatches(in: text,
                                           range: NSRange(text.startIndex..., in: text),
                                           withTemplate: "")
    }
}
