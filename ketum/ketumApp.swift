//
//  ketumApp.swift
//  ketum
//

import SwiftUI
import SwiftData
import UserNotifications

@main
struct ketumApp: App {
    let konteyner: ModelContainer
    @State private var dil = DilTercihi.paylasilan

    init() {
        #if DEBUG
        if CommandLine.arguments.contains("--ototest") {
            OtoTest.calistir()
        }
        if CommandLine.arguments.contains("--test") {
            Degerlendirme.calistir()
        }
        if CommandLine.arguments.contains("--dil") {
            DilTesti.calistir()
        }
        #endif
        konteyner = Self.konteynerKur()
        #if DEBUG
        if CommandLine.arguments.contains("--tablodemo") {
            let s = Sohbet(baslik: "Tablo demo")
            konteyner.mainContext.insert(s)
            let u = Mesaj(rol: .sen, icerik: "Haftalık yemek planı yapar mısın?")
            u.sohbet = s
            let k = Mesaj(rol: .ketum, icerik: """
            İşte haftalık yemek planın:

            | Gün | Öğle | Akşam |
            | --- | --- | --- |
            | Pazartesi | Mercimek çorbası | Izgara tavuk |
            | Salı | Ezogelin | Karnıyarık |
            | Çarşamba | Domates çorbası | Balık |

            Afiyet olsun. Değişiklik istersen söyle.
            """)
            k.sohbet = s
            konteyner.mainContext.insert(u)
            konteyner.mainContext.insert(k)
            try? konteyner.mainContext.save()
        }
        #endif
    }

    var body: some Scene {
        WindowGroup {
            // Yalnız SwiftUI metinleri bu locale'i anında izler; servislerdeki
            // String(localized:) çağrıları izlemez, onlar için yeniden başlatma gerekir.
            ContentView()
                .environment(\.locale, dil.arayuzLocale ?? Locale.autoupdatingCurrent)
        }
        .modelContainer(konteyner)
    }

    /// Sohbet + Mesaj şemasıyla konteyner. Şema değişiminde eski mağaza uyumsuzsa
    /// (geliştirme sürümlerinde) sıfırlayıp yeniden kurar — göçük yerine temiz başlangıç.
    private static func konteynerKur() -> ModelContainer {
        let sema = Schema([Sohbet.self, Mesaj.self, Nobet.self, NobetKaydi.self,
                           KullaniciBecerisi.self])
        let yapilandirma = ModelConfiguration(schema: sema)
        do {
            return try ModelContainer(for: sema, configurations: yapilandirma)
        } catch {
            // Mağaza gidiyor: karşılığı kalmayan yinelenen nöbet bildirimleri de iptal
            // edilmeli, yoksa kullanıcı olmayan bir nöbetin bildirimini almaya devam eder.
            UNUserNotificationCenter.current().removeAllPendingNotificationRequests()
            try? FileManager.default.removeItem(at: yapilandirma.url)
            // WAL/SHM yan dosyalarını da temizle.
            for ek in ["-wal", "-shm"] {
                try? FileManager.default.removeItem(
                    at: yapilandirma.url.appendingPathExtension(ek))
            }
            if let temiz = try? ModelContainer(for: sema, configurations: yapilandirma) {
                return temiz
            }
            // Disk hâlâ açılamıyor: açılışta çökmek yerine bu oturumu bellekte sürdür.
            // Veri kaydedilmez; sohbetler uygulama kapanınca kalmaz.
            let bellek = ModelConfiguration(schema: sema, isStoredInMemoryOnly: true)
            return try! ModelContainer(for: sema, configurations: bellek)
        }
    }
}
