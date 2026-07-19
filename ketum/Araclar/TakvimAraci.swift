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
    """
    weak var raporlayici: (any AracRaporlayici)?
    /// Büyük veri taşıma kanalı — okunan tüm etkinlikler burada saklanıp modele ref döner.
    weak var veriDeposu: VeriDeposu?

    @Generable struct Arguments {
        @Guide(description: "Yapılacak işlem: okuma için \"oku\", ekleme için \"ekle\".")
        var eylem: String
        @Guide(description: "Doğal dil ya da ISO tarih. Oku için aralık başı, ekle için etkinlik zamanı. Örn \"yarın 13:00\" ya da \"2026-07-20T13:00\".")
        var baslangic: String?
        @Guide(description: "Aralık ya da etkinlik bitişi. Doğal dil ya da ISO tarih.")
        var bitis: String?
        @Guide(description: "Ekle işleminde etkinlik başlığı. Örn \"Diş hekimi\".")
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

            if ekleMi {
                return try Self.ekle(depo: depo, arguments: arguments)
            } else {
                let (sonuc, tablo) = Self.oku(depo: depo, arguments: arguments)
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
        let baslangic = ayristir(arguments.baslangic) ?? Calendar.current.startOfDay(for: Date())
        let bitis = ayristir(arguments.bitis) ?? Calendar.current.date(byAdding: .day, value: 7, to: baslangic)!

        let yuklem = depo.predicateForEvents(withStart: baslangic, end: bitis, calendars: nil)
        let hepsi = depo.events(matching: yuklem).sorted { $0.startDate < $1.startDate }

        let saatBicim = DateFormatter()
        saatBicim.locale = Locale(identifier: "tr_TR")
        saatBicim.dateFormat = "HH:mm"

        let tamBicim = DateFormatter()
        tamBicim.locale = Locale(identifier: "tr_TR")
        tamBicim.dateFormat = "d MMM HH:mm"

        let gunBicim = DateFormatter()
        gunBicim.locale = Locale(identifier: "tr_TR")
        gunBicim.dateFormat = "d MMM yyyy"

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
        let baslangic = ayristir(arguments.baslangic) ?? Date()
        let bitis = ayristir(arguments.bitis)
            ?? Calendar.current.date(byAdding: .hour, value: 1, to: baslangic)!
        let baslik = arguments.baslik?.isEmpty == false ? arguments.baslik! : "Etkinlik"

        let etkinlik = EKEvent(eventStore: depo)
        etkinlik.title = baslik
        etkinlik.startDate = baslangic
        etkinlik.endDate = bitis
        etkinlik.calendar = depo.defaultCalendarForNewEvents

        try depo.save(etkinlik, span: .thisEvent)

        let tamBicim = DateFormatter()
        tamBicim.locale = Locale(identifier: "tr_TR")
        tamBicim.dateFormat = "d MMM HH:mm"

        return AracSonucu(cipMetni: Yerel.etkinlikEklendi,
                          durum: .yazildi,
                          modeleDonen: "event_added",
                          hamCikti: "\(baslik) — \(tamBicim.string(from: baslangic))")
    }

    // Basit tarih ayrıştırma: doğal dil ("bugün"/"yarın" + isteğe bağlı saat) ve ISO.
    private static func ayristir(_ metin: String?) -> Date? {
        guard let ham = metin?.trimmingCharacters(in: .whitespacesAndNewlines), !ham.isEmpty else {
            return nil
        }
        let kucuk = ham.lowercased()
        let takvim = Calendar.current

        // Önce ISO/biçimli deneyelim.
        let bicimler = ["yyyy-MM-dd'T'HH:mm:ss", "yyyy-MM-dd'T'HH:mm", "yyyy-MM-dd HH:mm", "yyyy-MM-dd"]
        for kalip in bicimler {
            let bicim = DateFormatter()
            bicim.locale = Locale(identifier: "tr_TR")
            bicim.dateFormat = kalip
            if let tarih = bicim.date(from: ham) {
                return tarih
            }
        }

        // Doğal dil: gün + isteğe bağlı "HH:mm".
        var taban = takvim.startOfDay(for: Date())
        if kucuk.contains("yarın") {
            taban = takvim.date(byAdding: .day, value: 1, to: taban) ?? taban
        } else if kucuk.contains("bugün") {
            // taban zaten bugün.
        } else {
            return nil
        }

        // Metindeki saati yakala (örn "13:00").
        if let aralik = kucuk.range(of: #"(\d{1,2}):(\d{2})"#, options: .regularExpression) {
            let parca = kucuk[aralik].split(separator: ":")
            if let saat = Int(parca[0]), let dakika = Int(parca[1]) {
                return takvim.date(bySettingHour: saat, minute: dakika, second: 0, of: taban) ?? taban
            }
        }
        return taban
    }
}
