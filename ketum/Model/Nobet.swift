//
//  Nobet.swift
//  ketum
//
//  Zamanlanmış ajan — "nöbet". sirr'in kullanıcı yerine periyodik yaptığı iş
//  (ör. sabah brifingi). Cihazda SwiftData ile saklanır; her çalışması bir
//  NobetKaydi olarak günlüğe yazılır — atlanan gece bile saklanır (şeffaflık).
//

import Foundation
import SwiftData

@Model
final class Nobet {
    var id: UUID = UUID()
    var ad: String = "Sabah brifingi"
    /// İçerik kaynakları.
    var takvimDahil: Bool = true
    var hatirlaticiDahil: Bool = true
    var notDahil: Bool = false
    /// Teslimat saati (0–23). Bildirim bu saatte planlanır.
    var saat: Int = 7
    var bildirim: Bool = true
    var aktif: Bool = true
    var olusturulma: Date = Date()

    @Relationship(deleteRule: .cascade, inverse: \NobetKaydi.nobet)
    var kayitlar: [NobetKaydi] = []

    init(ad: String = "Sabah brifingi", saat: Int = 7,
         takvimDahil: Bool = true, hatirlaticiDahil: Bool = true, notDahil: Bool = false) {
        self.id = UUID()
        self.ad = ad
        self.saat = saat
        self.takvimDahil = takvimDahil
        self.hatirlaticiDahil = hatirlaticiDahil
        self.notDahil = notDahil
        self.olusturulma = Date()
    }

    /// En son çalışma kaydı.
    var sonKayit: NobetKaydi? {
        kayitlar.sorted { $0.tarih > $1.tarih }.first
    }

    /// "Her gün · Bildirim 07.00" gibi altyazı. Şarj/arka plan koşulu VAAT EDİLMEZ —
    /// brifing uygulama öne gelince tazelenir (bkz. NobetServisi başlığı).
    var ozet: String {
        let ss = String(format: "%02d.00", saat)
        return bildirim
            ? String(localized: "Her gün · Bildirim \(ss)")
            : String(localized: "Her gün · \(ss)")
    }
}

@Model
final class NobetKaydi {
    var id: UUID = UUID()
    var tarih: Date = Date()
    var basarili: Bool = true
    /// "açılışta hazırlandı" / "kurulumda hazırlandı" gibi bağlam.
    var baglam: String = ""
    var ozet: String = ""
    /// Çalışırken düşen araç çipi etiketleri (şeffaflık günlüğü).
    private var ciplerVeri: Data?
    var nobet: Nobet?

    var cipler: [String] {
        get { ciplerVeri.flatMap { try? JSONDecoder().decode([String].self, from: $0) } ?? [] }
        set { ciplerVeri = try? JSONEncoder().encode(newValue) }
    }

    init(tarih: Date = Date(), basarili: Bool = true, baglam: String = "",
         ozet: String = "", cipler: [String] = []) {
        self.id = UUID()
        self.tarih = tarih
        self.basarili = basarili
        self.baglam = baglam
        self.ozet = ozet
        self.ciplerVeri = try? JSONEncoder().encode(cipler)
    }
}
