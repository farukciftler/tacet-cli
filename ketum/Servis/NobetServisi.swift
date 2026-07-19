//
//  NobetServisi.swift
//  ketum
//
//  Nöbet yürütme: bildirim planlama + brifing üretimi + çalışma günlüğü.
//  Brifing cihaz verisinden (EventKit) doğrudan, şablonla üretilir — model
//  gerektirmez, hızlı ve güvenilirdir; arka planda da çalışabilecek biçimde tutuldu.
//
//  Not: iOS'ta garantili "gece-şarjda" üretim BGTaskScheduler + entitlement ister.
//  Bu sürüm dürüst yolu izler: bildirim planlanır, brifing uygulama açıldığında
//  (ya da bildirime dokununca) tazelenir — mockup'taki "açılışta hazırlandı" yolu.
//

import Foundation
import EventKit
import SwiftData
import UserNotifications

@MainActor
@Observable
final class NobetServisi {
    private let eventStore = EKEventStore()

    /// Bildirim izni ister (yalnızca ilk kez sorar).
    func izinIste() async {
        _ = try? await UNUserNotificationCenter.current()
            .requestAuthorization(options: [.alert, .sound])
    }

    /// Nöbet için her gün `saat`:00'da yinelenen bildirim planlar.
    ///
    /// Gövde bilerek tarihe bağımlı DEĞİL: tek bir yinelenen istek her gün aynı metni
    /// gösterir, o yüzden içine bugünün özeti yazılırsa yarın dünkü özet okunur. Taze
    /// özet uygulama açılınca panoda ve sohbette görünür.
    func bildirimPlanla(_ nobet: Nobet) {
        let merkez = UNUserNotificationCenter.current()
        merkez.removePendingNotificationRequests(withIdentifiers: [nobet.id.uuidString])
        guard nobet.aktif, nobet.bildirim else { return }

        let icerik = UNMutableNotificationContent()
        icerik.title = nobet.ad
        icerik.body = String(localized: "Günün özeti seni bekliyor — açınca tazelenir.")
        icerik.sound = .default

        var bilesen = DateComponents()
        bilesen.hour = nobet.saat
        bilesen.minute = 0
        let tetik = UNCalendarNotificationTrigger(dateMatching: bilesen, repeats: true)
        merkez.add(UNNotificationRequest(identifier: nobet.id.uuidString, content: icerik, trigger: tetik))
    }

    func bildirimIptal(_ nobet: Nobet) {
        UNUserNotificationCenter.current()
            .removePendingNotificationRequests(withIdentifiers: [nobet.id.uuidString])
    }

    /// Uygulama öne gelince: bugün henüz çalışmamış aktif nöbetleri çalıştırır.
    func gerekliyseCalistir(_ nobetler: [Nobet], kayit: ModelContext) async {
        for nobet in nobetler where nobet.aktif {
            if let son = nobet.sonKayit, Calendar.current.isDateInToday(son.tarih) { continue }
            await calistir(nobet, baglam: String(localized: "açılışta hazırlandı"), kayit: kayit)
        }
    }

    /// Nöbeti bir kez çalıştırır: veriyi toplar, brifing üretir, günlüğe yazar, bildirimi tazeler.
    @discardableResult
    func calistir(_ nobet: Nobet, baglam: String, kayit: ModelContext) async -> NobetKaydi {
        let (ozet, cipler, okundu) = await brifingUret(nobet)
        let k = NobetKaydi(tarih: Date(), basarili: okundu, baglam: baglam, ozet: ozet, cipler: cipler)
        k.nobet = nobet
        kayit.insert(k)
        try? kayit.save()
        bildirimPlanla(nobet)
        return k
    }

    // MARK: - Brifing üretimi (EventKit, şablon)

    /// - Returns: özet, çipler ve `okundu` — en az bir kaynağa gerçekten erişilip
    ///   erişilmediği. İzin reddedilmişse ya da hiçbir kaynak seçili değilse `false`
    ///   döner; günlük bunu "başarısız" olarak yazar (şeffaflık günlüğü dürüst kalsın).
    func brifingUret(_ nobet: Nobet) async -> (ozet: String, cipler: [String], okundu: Bool) {
        var parcalar: [String] = []
        var cipler: [String] = []
        var istenenKaynak = false
        let takvim = Calendar.current
        let bugunBas = takvim.startOfDay(for: Date())
        let bugunSon = takvim.date(byAdding: .day, value: 1, to: bugunBas)!

        if nobet.takvimDahil { istenenKaynak = true }
        if nobet.hatirlaticiDahil { istenenKaynak = true }

        if nobet.takvimDahil, await erisim(.event) {
            let yuklem = eventStore.predicateForEvents(withStart: bugunBas, end: bugunSon, calendars: nil)
            let etkinlikler = eventStore.events(matching: yuklem).sorted { $0.startDate < $1.startDate }
            cipler.append(Yerel.takvimOkundu(etkinlikler.count))
            if etkinlikler.isEmpty {
                parcalar.append(String(localized: "Bugün takviminde etkinlik yok."))
            } else {
                let sf = DateFormatter(); sf.locale = .current; sf.dateFormat = "HH:mm"
                let ilk = etkinlikler.prefix(3)
                    .map { "\(sf.string(from: $0.startDate)) \($0.title ?? "")" }
                    .joined(separator: ", ")
                parcalar.append(String(localized: "Bugün \(etkinlikler.count) etkinlik: \(ilk)."))
            }
        }

        if nobet.hatirlaticiDahil, await erisim(.reminder) {
            let bekleyen = await bekleyenHatirlaticiSayisi()
            cipler.append(String(localized: "Hatırlatıcılar okundu"))
            if bekleyen > 0 {
                parcalar.append(String(localized: "\(bekleyen) hatırlatıcı bekliyor."))
            }
        }

        // Çip listesi = gerçekten okunan kaynaklar. Boşsa hiçbir kaynağa erişilememiştir.
        let okundu = !cipler.isEmpty
        let ozet: String
        if okundu {
            ozet = parcalar.isEmpty
                ? String(localized: "Bugün için özetlenecek bir şey yok.")
                : parcalar.joined(separator: " ")
        } else if istenenKaynak {
            ozet = String(localized: "Takvim ve hatırlatıcılara erişim yok — özet hazırlanamadı.")
        } else {
            ozet = String(localized: "Bu nöbet için hiçbir kaynak seçili değil — özet hazırlanamadı.")
        }
        return (ozet, cipler, okundu)
    }

    private func erisim(_ tur: EKEntityType) async -> Bool {
        let durum = EKEventStore.authorizationStatus(for: tur)
        if durum == .fullAccess { return true }
        if durum == .notDetermined {
            if tur == .event { return (try? await eventStore.requestFullAccessToEvents()) ?? false }
            return (try? await eventStore.requestFullAccessToReminders()) ?? false
        }
        return false
    }

    private func bekleyenHatirlaticiSayisi() async -> Int {
        await withCheckedContinuation { devam in
            let yuklem = eventStore.predicateForIncompleteReminders(
                withDueDateStarting: nil, ending: nil, calendars: nil)
            eventStore.fetchReminders(matching: yuklem) { hatirlaticilar in
                devam.resume(returning: hatirlaticilar?.count ?? 0)
            }
        }
    }
}
