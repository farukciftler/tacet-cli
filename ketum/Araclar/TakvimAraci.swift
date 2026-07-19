import Foundation
import EventKit
import FoundationModels

// Takvim aracı: EventKit ile etkinlik okuma ve ekleme.
// Okuma gri (.okundu), ekleme yeşil (.yazildi) çip düşürür.
struct TakvimAraci: KetumAraci {
    let name = "takvim"
    let description = """
    Reads calendar events and adds new ones. Call this whenever the user asks about \
    their schedule ("what's on tomorrow", "my week", "am I free Friday") or asks to add \
    an event ("add a meeting Friday at 2pm"), in any language. eylem="oku" to read, "ekle" to add.
    ONLY for real calendar events. If the user is adding or deleting a ROW, COLUMN or \
    SECTION of a document, table or list, use the document tools instead — a weekday name \
    on its own ("Wednesday - Pizza") is NOT a calendar request.
    """
    weak var raporlayici: (any AracRaporlayici)?
    /// Büyük veri taşıma kanalı — okunan tüm etkinlikler burada saklanıp modele ref döner.
    weak var veriDeposu: VeriDeposu?

    @Generable struct Arguments {
        @Guide(description: "The operation to perform: \"oku\" to read, \"ekle\" to add. Use these exact values.")
        var eylem: String
        @Guide(description: "Start of the range for \"oku\", or the event time for \"ekle\". ALWAYS give ISO 8601: \"2026-07-20T13:00\". Resolve relative wording ('tomorrow', 'next Friday') into a date yourself and write it as ISO; call the time tool first if you need today's date. Required for \"ekle\".")
        var baslangic: String?
        @Guide(description: "End of the range or of the event. ALWAYS ISO 8601: \"2026-07-20T14:00\". Leave empty if unknown.")
        var bitis: String?
        @Guide(description: "Event title for \"ekle\", e.g. \"Dentist\".")
        var baslik: String?
    }

    func call(arguments: Arguments) async -> String {
        let ekleMi = arguments.eylem.lowercased().contains("ekle")
        let ikon = ekleMi ? "calendar.badge.plus" : "calendar"
        let calisiyorMetni = ekleMi ? Yerel.etkinlikEkleniyor : Yerel.takvimBakiliyor
        let hamGirdi = [arguments.eylem, arguments.baslangic, arguments.bitis, arguments.baslik]
            .compactMap { $0 }
            .joined(separator: " · ")

        return await cipliCalis(ikon: ikon, calisiyorMetni: calisiyorMetni, hamGirdi: hamGirdi) {
            let depo = EKEventStore()

            // Yetki: iOS 17+ tam erişim iste, reddedilirse izin çipi döndür.
            let durum = EKEventStore.authorizationStatus(for: .event)
            if durum == .denied || durum == .restricted {
                return AracSonucu(cipMetni: Yerel.takvimIzni,
                                  durum: .izinGerekli,
                                  modeleDonen: "permission_required (user can grant access in Settings)")
            }
            if durum != .fullAccess {
                let verildi = try await depo.requestFullAccessToEvents()
                if !verildi {
                    return AracSonucu(cipMetni: Yerel.takvimIzni,
                                      durum: .izinGerekli,
                                      modeleDonen: "permission_required (user can grant access in Settings)")
                }
            }

            // Buradan sonrası GERÇEK takvim erişimidir; sonucu kirlilik
            // bayrağına bağlarız (mcp §5.6). İzin reddi yukarıda döndü —
            // erişilemeyen veri oturumu kirletmez.
            if ekleMi {
                let eklendi = try Self.ekle(depo: depo, arguments: arguments)
                return await kirletEgerBasarili(eklendi)
            } else {
                let (ham, tablo) = Self.oku(depo: depo, arguments: arguments)
                let sonuc = await kirletEgerBasarili(ham)
                // Toplu veri kanalı: tüm kayıtları depoya koy, modele yalnızca ref döndür.
                // Böylece "takvimi excel'e dök" gibi işlerde veri bağlam penceresine girmez.
                if let tablo, tablo.satirlar.count > 1, let depo2 = veriDeposu {
                    let ref = await depo2.koy(tablo, etiket: "takvim")
                    return AracSonucu(
                        cipMetni: sonuc.cipMetni,
                        durum: sonuc.durum,
                        // Yalnızca veri/olgu; imperatif yönerge yazma (model papağanlıyor).
                        modeleDonen: sonuc.modeleDonen + " (all \(tablo.satirlar.count) records ready, data_ref=\(ref))",
                        hamCikti: sonuc.hamCikti)
                }
                return sonuc
            }
        }
    }

    // Okuma: aralıktaki etkinlikleri özetler (modele) ve tam tabloyu (depo için) döndürür.
    private static func oku(depo: EKEventStore, arguments: Arguments) -> (AracSonucu, Tablo?) {
        // Verilmiş ama çözülemeyen bir zaman varsa YANLIŞ aralığı okuyup
        // "takvimin boş" demektense hata dön; model ISO ile tekrar dener.
        let baslangic: Date
        switch cozZorunlu(arguments.baslangic) {
        case .hata(let s): return (s, nil)
        case .tamam(let t): baslangic = t
        case .yok: baslangic = Calendar.current.startOfDay(for: Date())
        }

        let bitis: Date
        switch cozZorunlu(arguments.bitis) {
        case .hata(let s): return (s, nil)
        case .tamam(let t): bitis = t
        case .yok: bitis = Calendar.current.date(byAdding: .day, value: 7, to: baslangic)!
        }

        let yuklem = depo.predicateForEvents(withStart: baslangic, end: bitis, calendars: nil)
        let hepsi = depo.events(matching: yuklem).sorted { $0.startDate < $1.startDate }

        // Biçimler cihaz diline göre — kullanıcı Japonca kullanıyorsa Excel'in
        // "Tarih" sütununda Türkçe ay adı görmemeli.
        let saatBicim = DateFormatter()
        saatBicim.locale = Locale.current
        saatBicim.dateFormat = "HH:mm"

        let tamBicim = DateFormatter()
        tamBicim.locale = Locale.current
        tamBicim.setLocalizedDateFormatFromTemplate("d MMM HH:mm")

        let gunBicim = DateFormatter()
        gunBicim.locale = Locale.current
        gunBicim.setLocalizedDateFormatFromTemplate("d MMM yyyy")

        if hepsi.isEmpty {
            return (AracSonucu(cipMetni: Yerel.takvimOkunduBos,
                               durum: .okundu,
                               modeleDonen: "no_events_in_range",
                               hamCikti: "Etkinlik bulunamadı."), nil)
        }

        // Modele yalnızca ilk ~10'un kısa özeti gider (bağlam bütçesi — spec §7.2).
        let onizleme = Array(hepsi.prefix(10))
        let ozet = onizleme
            .map { "\(saatBicim.string(from: $0.startDate)) \($0.title ?? "Etkinlik")" }
            .joined(separator: "; ")
        let ham = onizleme
            .map { "\(tamBicim.string(from: $0.startDate)) — \($0.title ?? "Etkinlik")" }
            .joined(separator: "\n")

        // Depoya konacak TAM tablo (tüm kayıtlar, yapılandırılmış sütunlar).
        let satirlar = hepsi.map { e -> Satir in
            Satir(hucreler: [
                gunBicim.string(from: e.startDate),
                saatBicim.string(from: e.startDate),
                saatBicim.string(from: e.endDate),
                e.title ?? "Etkinlik",
                e.location ?? "",
            ])
        }
        let tablo = Tablo(basliklar: ["Tarih", "Başlangıç", "Bitiş", "Başlık", "Konum"],
                          satirlar: satirlar)

        return (AracSonucu(cipMetni: Yerel.takvimOkundu(hepsi.count),
                           durum: .okundu,
                           modeleDonen: ozet,
                           hamCikti: ham), tablo)
    }

    // Ekleme: yeni etkinlik oluşturup kaydeder.
    private static func ekle(depo: EKEventStore, arguments: Arguments) throws -> AracSonucu {
        // Zaman çözülemezse ETKİNLİĞİ ŞU ANA EKLEME. Kullanıcı "kuruldu" görüp
        // takvimde yanlış saatte bulmaktansa modelin ISO ile tekrar denemesi iyi.
        guard let cozum = ZamanCozucu.coz(arguments.baslangic) else {
            return AracSonucu(
                cipMetni: zamanAnlasilmadi,
                durum: .basarisiz(zamanAnlasilmadi),
                modeleDonen: "error: unparsable_or_missing_start_time"
                    + (arguments.baslangic.map { " \"\($0)\"" } ?? "")
                    + ". Nothing was created. Call the tool again with \"baslangic\" as an "
                    + "ISO 8601 timestamp, e.g. 2026-07-20T13:00."
            )
        }
        let baslangic = cozum.tarih

        let bitis: Date
        switch cozZorunlu(arguments.bitis) {
        case .hata(let s): return s
        case .tamam(let t): bitis = t
        case .yok: bitis = Calendar.current.date(byAdding: .hour, value: 1, to: baslangic)!
        }
        let baslik = arguments.baslik?.isEmpty == false ? arguments.baslik! : String(localized: "Etkinlik")

        let etkinlik = EKEvent(eventStore: depo)
        etkinlik.title = baslik
        etkinlik.startDate = baslangic
        etkinlik.endDate = bitis
        etkinlik.calendar = depo.defaultCalendarForNewEvents

        try depo.save(etkinlik, span: .thisEvent)

        let tamBicim = DateFormatter()
        tamBicim.locale = Locale.current
        tamBicim.setLocalizedDateFormatFromTemplate("d MMM HH:mm")

        return AracSonucu(cipMetni: Yerel.etkinlikEklendi,
                          durum: .yazildi,
                          modeleDonen: "event_added",
                          hamCikti: "\(baslik) — \(tamBicim.string(from: baslangic))")
    }

    // Zaman ayrıştırma ortak `ZamanCozucu`'da (HatirlaticiAraci.swift) —
    // dilden bağımsız: ISO 8601 → sabit kalıplar → Türkçe kestirme →
    // Locale.current → NSDataDetector. Hepsi cihaz-üstü, ağ yok.

    /// Boş bırakılabilen ama verilirse çözülmesi ZORUNLU alanlar için.
    private enum ZorunluSonuc {
        case yok                    // alan boş — çağıran varsayılanını kullanabilir
        case tamam(Date)
        case hata(AracSonucu)
    }

    private static func cozZorunlu(_ metin: String?) -> ZorunluSonuc {
        guard let ham = metin?.trimmingCharacters(in: .whitespacesAndNewlines), !ham.isEmpty else {
            return .yok
        }
        guard let cozum = ZamanCozucu.coz(ham) else {
            return .hata(AracSonucu(
                cipMetni: zamanAnlasilmadi,
                durum: .basarisiz(zamanAnlasilmadi),
                modeleDonen: "error: unparsable_time \"\(ham)\". Nothing was read or created. "
                    + "Call the tool again with ISO 8601 timestamps, e.g. 2026-07-20T13:00."
            ))
        }
        return .tamam(cozum.tarih)
    }

    static var zamanAnlasilmadi: String { String(localized: "Zaman anlaşılmadı") }
}
