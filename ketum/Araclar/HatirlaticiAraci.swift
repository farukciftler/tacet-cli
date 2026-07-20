//
//  HatirlaticiAraci.swift
//  ketum
//
//  Hatırlatıcı aracı (spec §7.3). EventKit Reminders üzerine okuma + yazma.
//  Model serbest metin değil, tip güvenli argüman verir; zamanı Swift parse eder.
//  Ağ yok — yalnızca yerel EKEventStore.
//

import Foundation
import FoundationModels
import EventKit

// MARK: - Dilden bağımsız zaman çözümleme

/// Araçların ortak zaman çözücüsü.
///
/// Ürün 9 dilde konuşuyor; tarih ayrıştırma yalnızca Türkçe bilirse diğer
/// dillerde SESSİZ VERİ HATASI olur (etkinlik yanlış saate kurulur). Bu yüzden
/// çözümleme katmanlı ve dilden bağımsız:
///   1. Katı ISO 8601 (modelden beklediğimiz biçim)
///   2. Dil-nötr sabit kalıplar (en_US_POSIX)
///   3. Türkçe kestirmeler (ana dil — hızlı yol, tek yol değil)
///   4. Locale.current ile tarih/saat stilleri (cihaz dili ne ise)
///   5. NSDataDetector — sistem bileşeni, cihaz-üstü, ağ yok; gizlilik vaadi bozulmaz
///
/// Hiçbiri tutmazsa `nil`. Çağıran ASLA sessizce "şimdi"ye düşmez; hata döner.
enum ZamanCozucu {
    /// Çözülen an + metinde açık bir saat bilgisi olup olmadığı.
    struct Cozum {
        var tarih: Date
        /// false ise yalnızca gün çözüldü (saat varsayılan/gün başı).
        var saatVar: Bool
    }

    /// Metni tarihe çevirir. Çözülemezse nil.
    static func coz(_ ham: String?) -> Cozum? {
        guard let metin = ham?.trimmingCharacters(in: .whitespacesAndNewlines),
              !metin.isEmpty else { return nil }

        // 1) Katı ISO 8601 — "2026-07-20T18:00:00Z" gibi tam biçim.
        if let tarih = try? Date(metin, strategy: .iso8601) {
            return Cozum(tarih: tarih, saatVar: true)
        }

        // 2) Dil-nötr sabit kalıplar. Saatli olanlar önce denenir.
        let bicim = DateFormatter()
        bicim.locale = Locale(identifier: "en_US_POSIX")
        bicim.timeZone = Calendar.current.timeZone
        for kalip in saatliKaliplar {
            bicim.dateFormat = kalip
            if let tarih = bicim.date(from: metin) { return Cozum(tarih: tarih, saatVar: true) }
        }
        for kalip in saatsizKaliplar {
            bicim.dateFormat = kalip
            if let tarih = bicim.date(from: metin) { return Cozum(tarih: tarih, saatVar: false) }
        }

        // 3) Türkçe kestirmeler — "bugün 18:00", "yarın", "öbür gün 9".
        if let cozum = turkceKestirme(metin) { return cozum }

        // 4) Cihaz dilinin kendi tarih biçimleri ("7/20/26, 6:00 PM", "20.07.2026" …).
        if let cozum = yerelBicim(metin) { return cozum }

        // 5) Son çare: sistemin veri algılayıcısı. Yereldir, ağ kullanmaz.
        if let cozum = algilayici(metin) { return cozum }

        return nil
    }

    private static let saatliKaliplar = [
        "yyyy-MM-dd'T'HH:mm:ss", "yyyy-MM-dd'T'HH:mm",
        "yyyy-MM-dd HH:mm:ss", "yyyy-MM-dd HH:mm",
        "yyyy/MM/dd HH:mm", "dd.MM.yyyy HH:mm", "dd/MM/yyyy HH:mm",
    ]
    private static let saatsizKaliplar = [
        "yyyy-MM-dd", "yyyy/MM/dd", "dd.MM.yyyy", "dd/MM/yyyy",
    ]

    /// Metinde açık saat izi var mı ("18:00", "6 pm", "18.30")?
    /// Yerel biçim ve algılayıcı sonuçlarında saatin gerçekten verilip
    /// verilmediğini ayırt etmek için kullanılır.
    static func saatIzi(_ metin: String) -> Bool {
        let kucuk = metin.lowercased()
        if kucuk.range(of: #"\d{1,2}\s*[:.]\s*\d{2}"#, options: .regularExpression) != nil { return true }
        if kucuk.range(of: #"\d\s*(am|pm|öö|ös)"#, options: .regularExpression) != nil { return true }
        return false
    }

    /// Türkçe göreli gün + isteğe bağlı saat.
    private static func turkceKestirme(_ ham: String) -> Cozum? {
        let metin = ham.lowercased()
        let takvim = Calendar.current

        var gunOfseti = 0
        var gunBelirtildi = false
        if metin.contains("öbür gün") || metin.contains("obur gun") {
            gunOfseti = 2; gunBelirtildi = true
        } else if metin.contains("yarın") || metin.contains("yarin") {
            gunOfseti = 1; gunBelirtildi = true
        } else if metin.contains("bugün") || metin.contains("bugun") {
            gunOfseti = 0; gunBelirtildi = true
        }
        guard gunBelirtildi else { return nil }

        // Saat: "18:00" / "18.00" ya da gün belirtildiği için güvenle tek sayı ("yarın 9").
        var saat: Int?
        var dakika = 0
        if let aralik = metin.range(of: #"(\d{1,2})[:.](\d{2})"#, options: .regularExpression) {
            let parca = metin[aralik].replacingOccurrences(of: ".", with: ":").split(separator: ":")
            saat = Int(parca[0]); dakika = Int(parca[1]) ?? 0
        } else if let aralik = metin.range(of: #"(?<!\d)(\d{1,2})(?!\d)"#, options: .regularExpression) {
            saat = Int(metin[aralik])
        }

        guard let hedefGun = takvim.date(byAdding: .day, value: gunOfseti, to: Date()) else { return nil }
        let gunBasi = takvim.startOfDay(for: hedefGun)
        if let s = saat, (0...23).contains(s) {
            let tarih = takvim.date(bySettingHour: s, minute: dakika, second: 0, of: gunBasi) ?? gunBasi
            return Cozum(tarih: tarih, saatVar: true)
        }
        return Cozum(tarih: gunBasi, saatVar: false)
    }

    /// Cihaz dilinin kendi kısa/orta/uzun tarih-saat biçimleri.
    private static func yerelBicim(_ metin: String) -> Cozum? {
        let stiller: [(DateFormatter.Style, DateFormatter.Style)] = [
            (.short, .short), (.medium, .short), (.long, .short), (.full, .short),
            (.short, .none), (.medium, .none), (.long, .none), (.full, .none),
        ]
        let bicim = DateFormatter()
        bicim.locale = Locale.current
        bicim.timeZone = Calendar.current.timeZone
        for (tarihStili, saatStili) in stiller {
            bicim.dateStyle = tarihStili
            bicim.timeStyle = saatStili
            if let tarih = bicim.date(from: metin) {
                return Cozum(tarih: tarih, saatVar: saatStili != .none)
            }
        }
        return nil
    }

    /// NSDataDetector: "next friday at 6", "明日 18時", "mañana a las 9" gibi
    /// serbest ifadeleri sistemin kendi dil verisiyle çözer. Tamamen yerel.
    private static func algilayici(_ metin: String) -> Cozum? {
        guard let dedektor = try? NSDataDetector(types: NSTextCheckingResult.CheckingType.date.rawValue)
        else { return nil }
        let aralik = NSRange(metin.startIndex..., in: metin)
        guard let eslesme = dedektor.firstMatch(in: metin, options: [], range: aralik),
              let tarih = eslesme.date else { return nil }
        // Algılayıcı saat verilmediğinde varsayılan bir saat uydurur; metinde
        // saat izi yoksa "saat yok" say ve çağırana bırak.
        return Cozum(tarih: tarih, saatVar: saatIzi(metin))
    }
}

// MARK: - Araç

struct HatirlaticiAraci: KetumAraci {
    let name = "hatirlatici"
    let description = """
    Creates a reminder (a to-do / task) or lists pending reminders. Call this whenever the \
    user asks to be reminded of something, in any language (e.g. 'remind me to call at 6pm', \
    'remind me to buy milk tomorrow'), or asks what is on their to-do list ('what are my \
    pending reminders'). eylem="kur" to create, "oku" to list. For a fixed appointment use \
    the calendar tool instead.
    """

    weak var raporlayici: (any AracRaporlayici)?
    /// Büyük veri taşıma kanalı — listelenen hatırlatıcılar burada saklanıp modele ref döner.
    weak var veriDeposu: VeriDeposu?

    /// Eylem serbest metin DEĞİL (P0-4). `contains("oku") || contains("liste")`
    /// İngilizce "list" için false dönüyor ve akış SESSİZCE kurma dalına
    /// düşüyordu: kullanıcı listesini isterken başlıksız bir hatırlatıcı
    /// kurulma girişimi oluyordu. Enum bu değeri üretilemez yapar.
    @Generable
    enum Eylem: String, Equatable, CaseIterable {
        case kur
        case oku
    }

    @Generable
    struct Arguments {
        @Guide(description: "The operation to perform: create a reminder, or list the pending ones.")
        var eylem: Eylem
        @Guide(description: "Title of the reminder for 'kur'; short and action-oriented, e.g. 'Call Ali', 'Buy milk'.")
        var baslik: String?
        @Guide(description: "When to be reminded. ISO 8601, e.g. \"2026-07-20T18:00\". Resolve relative wording ('tomorrow', 'tonight') yourself; call the time tool first if you need today's date. Leave empty if no time was asked for.")
        var zaman: String?
    }

    func call(arguments: Arguments) async -> String {
        // Exhaustive switch: bulanık eşleme yok, sessiz yanlış dal yok.
        let ikon: String
        let calisiyorMetni: String
        switch arguments.eylem {
        case .oku:
            ikon = "checklist"
            calisiyorMetni = Self.hatirlaticilaraBakiliyor
        case .kur:
            ikon = "bell"
            calisiyorMetni = Yerel.hatirlaticiKuruluyor
        }
        let hamGirdi = [arguments.eylem.rawValue, arguments.baslik, arguments.zaman]
            .compactMap { $0 }
            .joined(separator: " · ")

        return await cipliCalis(ikon: ikon, calisiyorMetni: calisiyorMetni, hamGirdi: hamGirdi) {
            let store = EKEventStore()

            // Yetki (spec §7.3): denied/restricted kalıcıdır — her çağrıda yeniden
            // sormak anlamsız. Yalnızca notDetermined durumunda bir kez iste.
            let durum = EKEventStore.authorizationStatus(for: .reminder)
            if durum == .denied || durum == .restricted {
                return AracSonucu(cipMetni: Yerel.hatirlaticiIzni,
                                  durum: .izinGerekli,
                                  modeleDonen: "permission_denied (user must grant access in Settings)")
            }
            if durum != .fullAccess {
                let verildi = try await store.requestFullAccessToReminders()
                if !verildi {
                    return AracSonucu(cipMetni: Yerel.hatirlaticiIzni,
                                      durum: .izinGerekli,
                                      modeleDonen: "permission_required (user can grant access in Settings)")
                }
            }

            // İzin geçildi, gerçek hatırlatıcı erişimi burada. Sonuç `.okundu`
            // ya da `.yazildi` ise oturum kirlenir (mcp §5.6); zaman
            // çözülemeyip `.basarisiz` dönen yol kirletmez.
            switch arguments.eylem {
            case .oku:
                let okunan = await Self.oku(store: store, veriDeposu: veriDeposu)
                return await kirletEgerBasarili(okunan)
            case .kur:
                let kurulan = try Self.kur(store: store, arguments: arguments)
                return await kirletEgerBasarili(kurulan)
            }
        }
    }

    // MARK: Kurma

    private static func kur(store: EKEventStore, arguments: Arguments) throws -> AracSonucu {
        let baslik = arguments.baslik?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !baslik.isEmpty else {
            return AracSonucu(cipMetni: baslikEksik,
                              durum: .basarisiz(baslikEksik),
                              modeleDonen: "error: missing_title. Call the tool again with a short title in \"baslik\".")
        }

        // Zaman verildiyse ÇÖZÜLMEK ZORUNDA. Çözülemeyeni sessizce zamansız
        // kurmak kullanıcıya "kuruldu" deyip hatırlatmamak demektir.
        var bilesenler: DateComponents?
        if let ham = arguments.zaman?.trimmingCharacters(in: .whitespacesAndNewlines), !ham.isEmpty {
            guard let cozum = ZamanCozucu.coz(ham) else {
                return AracSonucu(
                    cipMetni: zamanAnlasilmadi,
                    durum: .basarisiz(zamanAnlasilmadi),
                    modeleDonen: "error: unparsable_time \"\(ham)\". Nothing was created. "
                        + "Call the tool again with \"zaman\" as an ISO 8601 timestamp, e.g. 2026-07-20T18:00."
                )
            }
            let takvim = Calendar.current
            bilesenler = cozum.saatVar
                ? takvim.dateComponents([.year, .month, .day, .hour, .minute], from: cozum.tarih)
                : takvim.dateComponents([.year, .month, .day], from: cozum.tarih)
        }

        guard let takvimListesi = store.defaultCalendarForNewReminders() else {
            throw HatirlaticiHatasi.takvimYok
        }

        let reminder = EKReminder(eventStore: store)
        reminder.title = baslik
        reminder.calendar = takvimListesi
        reminder.dueDateComponents = bilesenler

        try store.save(reminder, commit: true)

        let saatMetni = saatMetni(bilesenler)
        let ham = baslik + (arguments.zaman.map { " — \($0)" } ?? " — zamansız")
        return AracSonucu(cipMetni: Yerel.hatirlaticiKuruldu(saat: saatMetni),
                          durum: .yazildi,
                          modeleDonen: "reminder_created",
                          hamCikti: ham)
    }

    // MARK: Okuma

    /// EKReminder'ın düz, Sendable karşılığı (EventKit nesneleri aktör sınırını geçemez).
    private struct Bekleyen: Sendable {
        var baslik: String
        var zaman: Date?
        var liste: String
    }

    /// Bekleyen (tamamlanmamış) hatırlatıcıları listeler.
    /// Bağlam bütçesi (spec §7.2): modele yalnızca sayı + ilk birkaç başlık gider,
    /// tam liste VeriDeposu'na konur ve modele ref döner.
    private static func oku(store: EKEventStore, veriDeposu: VeriDeposu?) async -> AracSonucu {
        let yuklem = store.predicateForIncompleteReminders(
            withDueDateStarting: nil, ending: nil, calendars: nil)
        // EKReminder Sendable değil — continuation sınırını yalnızca düz veri geçsin.
        let takvim = Calendar.current
        let hatirlaticilar: [Bekleyen] = await withCheckedContinuation { devam in
            store.fetchReminders(matching: yuklem) { sonuc in
                let duz = (sonuc ?? []).map { r in
                    Bekleyen(baslik: r.title ?? "-",
                             zaman: r.dueDateComponents.flatMap { takvim.date(from: $0) },
                             liste: r.calendar?.title ?? "")
                }
                devam.resume(returning: duz)
            }
        }

        if hatirlaticilar.isEmpty {
            return AracSonucu(cipMetni: hatirlaticiOkunduBos,
                              durum: .okundu,
                              modeleDonen: "no_pending_reminders",
                              hamCikti: "Bekleyen hatırlatıcı yok.")
        }

        // Yakın zamanlı olan önce; zamansızlar sona.
        let sirali = hatirlaticilar.sorted { ($0.zaman ?? .distantFuture) < ($1.zaman ?? .distantFuture) }

        // Biçim cihaz diline göre — sabit tr_TR yok.
        let tamBicim = DateFormatter()
        tamBicim.locale = Locale.current
        tamBicim.dateStyle = .medium
        tamBicim.timeStyle = .short

        func zamanMetni(_ r: Bekleyen) -> String {
            guard let t = r.zaman else { return "" }
            return tamBicim.string(from: t)
        }

        // Modele yalnızca ilk ~10'un kısa özeti gider.
        let onizleme = Array(sirali.prefix(10))
        let ozet = onizleme
            .map { r -> String in
                let z = zamanMetni(r)
                return z.isEmpty ? r.baslik : "\(r.baslik) (\(z))"
            }
            .joined(separator: "; ")
        let ham = onizleme
            .map { r -> String in
                let z = zamanMetni(r)
                return z.isEmpty ? "• \(r.baslik)" : "• \(r.baslik) — \(z)"
            }
            .joined(separator: "\n")

        let sonuc = AracSonucu(cipMetni: hatirlaticiOkundu(sirali.count),
                               durum: .okundu,
                               modeleDonen: "\(sirali.count) pending: \(ozet)",
                               hamCikti: ham)

        // Toplu veri kanalı: tam liste depoya, modele yalnızca ref.
        if sirali.count > 1, let depo = veriDeposu {
            let satirlar = sirali.map { r in
                Satir(hucreler: [r.baslik, zamanMetni(r), r.liste])
            }
            let tablo = Tablo(basliklar: ["Başlık", "Zaman", "Liste"], satirlar: satirlar)
            let ref = await depo.koy(tablo, etiket: "hatirlatici")
            return AracSonucu(cipMetni: sonuc.cipMetni,
                              durum: sonuc.durum,
                              modeleDonen: sonuc.modeleDonen
                                + " (all \(sirali.count) records ready, data_ref=\(ref))",
                              hamCikti: sonuc.hamCikti)
        }
        return sonuc
    }

    // MARK: - Metinler
    // Not: Yerel.swift bu fazda başka bir ajanın dosyası; yeni anahtarlar burada
    // String(localized:) ile tanımlı — String Catalog'a otomatik girer.

    static var hatirlaticilaraBakiliyor: String { String(localized: "Hatırlatıcılara bakılıyor…") }
    static func hatirlaticiOkundu(_ n: Int) -> String {
        String(localized: "Hatırlatıcılar okundu · \(n) bekliyor")
    }
    static var hatirlaticiOkunduBos: String { String(localized: "Hatırlatıcılar okundu · boş") }
    static var zamanAnlasilmadi: String { String(localized: "Zaman anlaşılmadı") }
    static var baslikEksik: String { String(localized: "Başlık eksik") }

    enum HatirlaticiHatasi: LocalizedError {
        case takvimYok
        var errorDescription: String? { String(localized: "Hatırlatıcı listesi bulunamadı") }
    }

    /// dueDateComponents içindeki saati "HH.mm" biçiminde döndürür; saat yoksa nil.
    static func saatMetni(_ bilesen: DateComponents?) -> String? {
        guard let saat = bilesen?.hour else { return nil }
        let dakika = bilesen?.minute ?? 0
        return String(format: "%02d.%02d", saat, dakika)
    }
}
