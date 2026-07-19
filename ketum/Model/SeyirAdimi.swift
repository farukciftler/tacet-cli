//
//  SeyirAdimi.swift
//  ketum
//
//  Bir yanıt turunun deterministik olay dizisi (seyir-spec §5.1).
//  SwiftData @Model DEĞİL — `AracIzi` gibi Codable struct; mesaja gömülü
//  JSON olarak kalıcılaşır.
//
//  Tek doğruluk kaynağı kuralı: araç adımında `metin` BOŞTUR, satır metni
//  `aracIziID` ile bağlanan `AracIzi.metin`den okunur. Aynı metnin iki yerde
//  tutulması, çipin araç tarafından üretilmesi ilkesini bozardı.
//

import Foundation

/// Adımın türü. Kaynakları hepsi zaten var olan deterministik noktalardır
/// (seyir-spec §5.2 tablosu); uydurma durum fiili yoktur.
enum SeyirTuru: String, Codable, CaseIterable, Hashable {
    /// Yönlendirici bir niyet profili seçti.
    case yonlendirme
    /// İsteme beceri / hafıza iliştirildi.
    case zenginlestirme
    /// Bir araç çalıştı — metin `AracIzi`den okunur.
    case arac
    /// Yanıt akışından ilk parça geldi.
    case yazim
    /// İptal ya da sahne kesintisi; tur yarıda kaldı.
    case kesinti
}

/// Seyir şeridinin tek satırı. Adımlar ardışıktır: bir adım açılınca önceki kapanır.
struct SeyirAdimi: Identifiable, Codable, Hashable {
    var id: UUID = UUID()
    var tur: SeyirTuru
    /// Satır metni ("yönlendirildi · takvim profili").
    /// `tur == .arac` ise BOŞ — metin `AracIzi`den okunur.
    var metin: String
    /// `tur == .arac` ise ilgili `AracIzi.id`.
    var aracIziID: UUID?
    var baslangic: Date
    /// `nil` = sürüyor ya da yarıda kaldı.
    var bitis: Date?

    init(id: UUID = UUID(),
         tur: SeyirTuru,
         metin: String = "",
         aracIziID: UUID? = nil,
         baslangic: Date = Date(),
         bitis: Date? = nil) {
        self.id = id
        self.tur = tur
        self.metin = tur == .arac ? "" : metin
        self.aracIziID = aracIziID
        self.baslangic = baslangic
        self.bitis = bitis
    }

    /// Adım kapandı mı.
    var bittiMi: Bool { bitis != nil }

    /// Saniye cinsinden süre; sürüyorsa `nil`. Saat geri alınırsa negatif
    /// olmasın diye sıfıra kırpılır (seyir-spec §6 test ölçütü).
    var sure: TimeInterval? {
        guard let bitis else { return nil }
        return max(0, bitis.timeIntervalSince(baslangic))
    }
}
