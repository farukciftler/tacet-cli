//
//  Mesaj.swift
//  ketum
//
//  Sohbet geçmişi — cihazda SwiftData ile saklanır (spec §4.7, §7.5).
//  Kalıcı model kayıtları; canlı akış durumu view model'de tutulur.
//

import Foundation
import SwiftData

/// Kim konuşuyor. Ham string olarak saklanır (SwiftData enum uyumu için basit).
enum Rol: String, Codable {
    case sen    // kullanıcı
    case ketum  // asistan
}

/// Kalıcı sohbet mesajı. Araç izleri mesaja gömülü (JSON) saklanır —
/// çipler yanıtla birlikte kalıcıdır ve tek doğruluk kaynağı araç katmanıdır.
@Model
final class Mesaj {
    var id: UUID = UUID()
    private var rolHam: String = Rol.ketum.rawValue
    var icerik: String = ""
    var olusturulma: Date = Date()
    /// Ait olduğu sohbet (spec §4.7). Ters ilişki Sohbet.mesajlar'da tanımlı.
    var sohbet: Sohbet?
    /// Kodlanmış [AracIzi]. SwiftData transformable yerine düz Data — taşınabilir.
    private var izlerVeri: Data?

    var rol: Rol {
        get { Rol(rawValue: rolHam) ?? .ketum }
        set { rolHam = newValue.rawValue }
    }

    /// Bu yanıtın hemen üstüne düşen araç çipleri (spec §4.4).
    var izler: [AracIzi] {
        get {
            guard let izlerVeri else { return [] }
            return (try? JSONDecoder().decode([AracIzi].self, from: izlerVeri)) ?? []
        }
        set { izlerVeri = try? JSONEncoder().encode(newValue) }
    }

    init(rol: Rol, icerik: String, izler: [AracIzi] = [], olusturulma: Date = Date()) {
        self.id = UUID()
        self.rolHam = rol.rawValue
        self.icerik = icerik
        self.olusturulma = olusturulma
        self.izlerVeri = (try? JSONEncoder().encode(izler))
    }
}
