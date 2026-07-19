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

    init() {
        #if DEBUG
        if CommandLine.arguments.contains("--ototest") {
            OtoTest.calistir()
        }
        if CommandLine.arguments.contains("--test") {
            Degerlendirme.calistir()
        }
        // Nihai rapor: ayrı koşumların ham JSON'larını tek Excel'e toplar.
        // Rapor uygulamanın KENDİ ExcelMotor'uyla üretilir (gerçek =SUM düşsün).
        if CommandLine.arguments.contains("--eval-birlestir") {
            EvalRapor.birlestirmeKosusu()
        }
        if CommandLine.arguments.contains("--dil") {
            DilTesti.calistir()
        }
        // Kapsamlı eval: "--eval [--shard N] [--shards M]". Paralel koşum
        // ajanları aynı ikiliyi farklı shard'larla açar; dilim deterministiktir.
        if CommandLine.arguments.contains("--eval") {
            let argumanlar = CommandLine.arguments
            func sayi(_ bayrak: String, _ varsayilan: Int) -> Int {
                guard let yer = argumanlar.firstIndex(of: bayrak),
                      yer + 1 < argumanlar.count,
                      let deger = Int(argumanlar[yer + 1]) else { return varsayilan }
                return deger
            }
            Degerlendirme.kapsamliCalistir(shard: sayi("--shard", 0),
                                           toplam: sayi("--shards", 1))
        }
        // MCP (bağlantı) eval'i: "--eval-mcp". Ayrı bayrak, çünkü bu koşu
        // kullanıcının kendi sunucusuna GERÇEKTEN çıkar; "--eval" çıkmaz.
        if CommandLine.arguments.contains("--eval-mcp") {
            Degerlendirme.mcpCalistir()
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
            // Dilin tek doğruluk kaynağı AppleLanguages'tır: Bundle.main süreç
            // başında hangi .lproj'u seçtiyse hem Text(...) hem String(localized:)
            // onu kullanır. Burada locale override edilirse iki yol ayrışır ve
            // ekran yarı çevrili kalır; bu yüzden override YOK. Dil değişimi
            // DilTercihi.arayuzYenidenBaslatmaBekliyor ile bildirilir.
            ContentView()
        }
        .modelContainer(konteyner)
    }

    /// Sohbet + Mesaj şemasıyla konteyner. Açılamazsa: DEBUG'ta sıfırlar, üretimde
    /// kullanıcı verisini ASLA silmez — bozuk mağazayı yedekleyip yeniden dener.
    @MainActor
    private static func konteynerKur() -> ModelContainer {
        let sema = Schema(versionedSchema: SemaV1.self)
        let yapilandirma = ModelConfiguration(schema: sema)
        do {
            return try ModelContainer(for: sema,
                                      migrationPlan: GocPlani.self,
                                      configurations: yapilandirma)
        } catch {
            // Mağaza yerinden oynuyor: karşılığı kalmayan yinelenen nöbet bildirimleri de
            // iptal edilmeli, yoksa kullanıcı olmayan bir nöbetin bildirimini almaya devam eder.
            UNUserNotificationCenter.current().removeAllPendingNotificationRequests()

            #if DEBUG
            // Yalnızca geliştirmede: şema oynarken temiz başlangıç.
            try? FileManager.default.removeItem(at: yapilandirma.url)
            for ek in ["-wal", "-shm"] {
                try? FileManager.default.removeItem(
                    at: yapilandirma.url.appendingPathExtension(ek))
            }
            #else
            // Üretimde silme yok: bozuk mağazayı yeniden adlandırıp saklıyoruz ki
            // kullanıcı verisi diskte kalsın ve ileride kurtarılabilsin.
            DepoDurumu.paylasilan.yedeklenenMagaza = magazaYiYedekle(yapilandirma.url)
            #endif

            if let temiz = try? ModelContainer(for: sema,
                                               migrationPlan: GocPlani.self,
                                               configurations: yapilandirma) {
                return temiz
            }
            // Disk hâlâ açılamıyor: açılışta çökmek yerine bu oturumu bellekte sürdür.
            // Veri kaydedilmez; sohbetler uygulama kapanınca kalmaz — UI uyarmalı.
            DepoDurumu.paylasilan.oturumKaliciDegil = true
            let bellek = ModelConfiguration(schema: sema, isStoredInMemoryOnly: true)
            return try! ModelContainer(for: sema, configurations: bellek)
        }
    }

    /// Bozuk mağazayı ve yan dosyalarını `<ad>.bozuk-<n>` olarak yeniden adlandırır.
    /// Başarılıysa yedeğin dosya adını döndürür. Hiçbir şey silinmez.
    private static func magazaYiYedekle(_ url: URL) -> String? {
        let dm = FileManager.default
        guard dm.fileExists(atPath: url.path) else { return nil }
        // Çakışmayan bir sayaç bul.
        var sayac = 1
        var hedef = url.appendingPathExtension("bozuk-\(sayac)")
        while dm.fileExists(atPath: hedef.path) && sayac < 100 {
            sayac += 1
            hedef = url.appendingPathExtension("bozuk-\(sayac)")
        }
        do {
            try dm.moveItem(at: url, to: hedef)
        } catch {
            return nil
        }
        // WAL/SHM yan dosyaları da yedekle; geride kalırlarsa yeni mağazayı bozarlar.
        for ek in ["-wal", "-shm"] {
            let yan = url.appendingPathExtension(ek)
            guard dm.fileExists(atPath: yan.path) else { continue }
            try? dm.moveItem(at: yan, to: hedef.appendingPathExtension(ek))
        }
        return hedef.lastPathComponent
    }
}

// MARK: - Şema sürümleri

/// v1 — ilk şema. Sürüm kimliği bilerek 1.0.0'da tutuluyor.
///
/// GEREKÇE: uygulama henüz yayınlanmadı, yani sahada 1.0.0 etiketli BAŞKA bir alan
/// kümesi taşıyan hiçbir mağaza yok. Bu yüzden `Mesaj.hataMi` ve `Sohbet.baslikOtomatik`
/// gibi geliştirme sırasında eklenen alanları — ve `HafizaNotu`, `Baglanti` gibi
/// sonradan eklenen modelleri — v1'in içine almak ve sürümü artırmamak
/// güvenli; artırmak ise gereksiz bir göç aşaması yaratırdı. v1 = "ilk yayınlanan hal".
///
/// YAYINDAN SONRA BU SERBESTLİK BİTER. İlk sürüm mağazaya çıktığı an bu enum
/// DONDURULMALIDIR: alan eklenirse/çıkarılırsa yeni bir `SemaV2` yazılmalı,
/// `GocPlani.schemas`a eklenmeli ve `stages`e `.lightweight(fromVersion: SemaV1.self,
/// toVersion: SemaV2.self)` (ya da custom) girmelidir. v1'i yayından sonra düzenlemek
/// "from" şemasını yalanlar ve kullanıcının mağazası açılmaz.
enum SemaV1: VersionedSchema {
    static var versionIdentifier: Schema.Version { Schema.Version(1, 0, 0) }
    static var models: [any PersistentModel.Type] {
        [Sohbet.self, Mesaj.self, Nobet.self, NobetKaydi.self, KullaniciBecerisi.self,
         HafizaNotu.self, Baglanti.self]
    }
}

/// Göç planı. Şu an tek sürüm var, aşama listesi boş; iskelet burada dursun ki
/// v2 geldiğinde tek satırla bağlanabilsin.
enum GocPlani: SchemaMigrationPlan {
    static var schemas: [any VersionedSchema.Type] { [SemaV1.self] }
    static var stages: [MigrationStage] { [] }
}

// MARK: - Depo durumu

/// Kalıcı depoyla ilgili, UI'ın kullanıcıya bildirmesi gereken olağandışı durumlar.
/// Normal açılışta her ikisi de kapalıdır ve hiçbir şey gösterilmez.
@MainActor
@Observable
final class DepoDurumu {
    static let paylasilan = DepoDurumu()

    /// Disk mağazası açılamadı, oturum yalnızca bellekte. Uygulama kapanınca
    /// bu oturumdaki her şey kaybolur — kullanıcı uyarılmalı.
    var oturumKaliciDegil = false

    /// Bozuk mağaza yedeklendiyse yedeğin dosya adı. Eski veriler kaybolmadı
    /// ama bu oturumda görünmüyor; kullanıcıya durum bildirilebilir.
    var yedeklenenMagaza: String?

    private init() {}
}
