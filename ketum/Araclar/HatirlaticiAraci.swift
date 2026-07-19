//
//  HatirlaticiAraci.swift
//  ketum
//
//  Hatırlatıcı aracı (spec §7.3). EventKit Reminders üzerine yazma ağırlıklı.
//  Model serbest metin değil, tip güvenli argüman verir; zamanı Swift parse eder.
//  Ağ yok — yalnızca yerel EKEventStore.
//

import Foundation
import FoundationModels
import EventKit

struct HatirlaticiAraci: KetumAraci {
    let name = "hatirlatici"
    let description = "Creates a reminder (a to-do / task), with or without a time. Call this whenever the user asks to be reminded of something, in any language (e.g. 'remind me to call at 6pm', 'remind me to buy milk tomorrow'). For a fixed appointment use the calendar tool instead."

    weak var raporlayici: (any AracRaporlayici)?

    @Generable
    struct Arguments {
        @Guide(description: "Hatırlatıcının başlığı; kısa ve eylem odaklı. Örn: 'Ali'yi ara', 'Süt al'.")
        var baslik: String
        @Guide(description: "Hatırlatma zamanı; doğal Türkçe ya da ISO. Örn: 'bugün 18:00', 'yarın 09:30', '2026-07-20 14:00'. Zaman yoksa boş bırak.")
        var zaman: String?
    }

    func call(arguments: Arguments) async -> String {
        let hamGirdi = arguments.baslik + (arguments.zaman.map { " · \($0)" } ?? "")
        return await cipliCalis(ikon: "bell", calisiyorMetni: Yerel.hatirlaticiKuruluyor, hamGirdi: hamGirdi) {
            let store = EKEventStore()

            // Yetki: iOS 17+ tam erişim iste. Reddedilirse throw etme, izinGerekli dön.
            let durum = EKEventStore.authorizationStatus(for: .reminder)
            if durum != .fullAccess {
                let verildi = (try? await store.requestFullAccessToReminders()) ?? false
                if !verildi {
                    return AracSonucu(
                        cipMetni: Yerel.hatirlaticiIzni,
                        durum: .izinGerekli,
                        modeleDonen: "permission_required (user can grant access in Settings)"
                    )
                }
            }

            guard let takvim = store.defaultCalendarForNewReminders() else {
                throw HatirlaticiHatasi.takvimYok
            }

            let reminder = EKReminder(eventStore: store)
            reminder.title = arguments.baslik
            reminder.calendar = takvim

            // Zaman parse edilebilirse dueDateComponents ata; yoksa zamansız kur.
            let bilesenler = arguments.zaman.flatMap { Self.zamanParse($0) }
            reminder.dueDateComponents = bilesenler

            try store.save(reminder, commit: true)

            let saatMetni = Self.saatMetni(bilesenler)
            let cip = Yerel.hatirlaticiKuruldu(saat: saatMetni)
            let ham = arguments.baslik + (arguments.zaman.map { " — \($0)" } ?? " — zamansız")

            return AracSonucu(
                cipMetni: cip,
                durum: .yazildi,
                modeleDonen: "reminder_created",
                hamCikti: ham
            )
        }
    }

    enum HatirlaticiHatasi: LocalizedError {
        case takvimYok
        var errorDescription: String? { "Hatırlatıcı takvimi bulunamadı" }
    }

    /// Doğal Türkçe ya da ISO zamanı DateComponents'a çevirir. Çözülemezse nil.
    static func zamanParse(_ ham: String) -> DateComponents? {
        let metin = ham.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !metin.isEmpty else { return nil }

        let takvim = Calendar(identifier: .gregorian)
        let simdi = Date()

        // Gün ofsetini yakala: bugün / yarın / öbür gün.
        var gunOfseti = 0
        var gunBelirtildi = false
        if metin.contains("öbür gün") || metin.contains("obur gun") {
            gunOfseti = 2; gunBelirtildi = true
        } else if metin.contains("yarın") || metin.contains("yarin") {
            gunOfseti = 1; gunBelirtildi = true
        } else if metin.contains("bugün") || metin.contains("bugun") {
            gunOfseti = 0; gunBelirtildi = true
        }

        // Saati yakala: "18:00", "18.00", "9" gibi.
        var saat: Int?
        var dakika = 0
        if let aralik = metin.range(of: #"(\d{1,2})[:.](\d{2})"#, options: .regularExpression) {
            let parca = metin[aralik].replacingOccurrences(of: ".", with: ":").split(separator: ":")
            saat = Int(parca[0]); dakika = Int(parca[1]) ?? 0
        } else if let aralik = metin.range(of: #"(?<!\d)(\d{1,2})(?!\d)"#, options: .regularExpression),
                  gunBelirtildi {
            // Yalnızca gün de belirtilmişse tek sayıyı saat say (yanlış eşleşmeyi azalt).
            saat = Int(metin[aralik])
        }

        // ISO denemesi: gün/saat hiç yakalanmadıysa ISO8601 dene.
        if !gunBelirtildi && saat == nil {
            let isoFormatlar = ["yyyy-MM-dd'T'HH:mm", "yyyy-MM-dd HH:mm", "yyyy-MM-dd"]
            let df = DateFormatter()
            df.locale = Locale(identifier: "en_US_POSIX")
            df.timeZone = takvim.timeZone
            for f in isoFormatlar {
                df.dateFormat = f
                if let tarih = df.date(from: metin) {
                    return takvim.dateComponents([.year, .month, .day, .hour, .minute], from: tarih)
                }
            }
            return nil
        }

        guard let hedefGun = takvim.date(byAdding: .day, value: gunOfseti, to: simdi) else { return nil }
        var bilesen = takvim.dateComponents([.year, .month, .day], from: hedefGun)
        if let s = saat, (0...23).contains(s) {
            bilesen.hour = s
            bilesen.minute = dakika
        } else if !gunBelirtildi {
            return nil
        }
        return bilesen
    }

    /// dueDateComponents içindeki saati "HH.mm" biçiminde döndürür; saat yoksa nil.
    static func saatMetni(_ bilesen: DateComponents?) -> String? {
        guard let saat = bilesen?.hour else { return nil }
        let dakika = bilesen?.minute ?? 0
        return String(format: "%02d.%02d", saat, dakika)
    }
}
