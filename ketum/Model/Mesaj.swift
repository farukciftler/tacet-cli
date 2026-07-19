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
    /// Bu mesaj bir hata bildirimi mi? Hata metinleri gerçek yanıttan görsel
    /// olarak ayrılsın ve "yeniden dene" sunulsun diye işaretlenir.
    /// Varsayılan değerli — lightweight migration uyumlu.
    var hataMi: Bool = false
    /// Aynı istem yeniden gönderilebilir mi? Yan etkisi tamamlanmış hatalarda
    /// (takvime yazıldı, sonra hata) tekrar denemek İKİNCİ bir etkinlik yaratır;
    /// guardrail/dil reddinde ise tekrar zaten aynı sonucu verir. İkisinde de false.
    /// Varsayılan değerli — lightweight migration uyumlu.
    var tekrarDenenebilir: Bool = true
    /// Kodlanmış [AracIzi]. SwiftData transformable yerine düz Data — taşınabilir.
    private var izlerVeri: Data?
    /// Kodlanmış [SeyirAdimi] — izlerVeri deseninin aynısı.
    /// Varsayılan nil: eski mesajlarda boş liste döner, Seyir satırı çizilmez,
    /// geriye dönük dolgu YAPILMAZ (seyir-spec §5.1).
    private var adimlarVeri: Data?

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

    /// Bu yanıtın seyri — turun deterministik olay dizisi (seyir-spec §5.1).
    var adimlar: [SeyirAdimi] {
        get {
            guard let adimlarVeri else { return [] }
            return (try? JSONDecoder().decode([SeyirAdimi].self, from: adimlarVeri)) ?? []
        }
        set { adimlarVeri = try? JSONEncoder().encode(newValue) }
    }

    init(rol: Rol, icerik: String, izler: [AracIzi] = [],
         adimlar: [SeyirAdimi] = [],
         hataMi: Bool = false, tekrarDenenebilir: Bool = true,
         olusturulma: Date = Date()) {
        self.id = UUID()
        self.rolHam = rol.rawValue
        self.icerik = icerik
        self.hataMi = hataMi
        self.tekrarDenenebilir = tekrarDenenebilir
        self.olusturulma = olusturulma
        self.izlerVeri = (try? JSONEncoder().encode(izler))
        // Boşsa nil bırakılır: "seyir yok" ile "boş seyir" aynı şeydir ve
        // eski mesajlarla aynı yolu izler.
        self.adimlarVeri = adimlar.isEmpty ? nil : (try? JSONEncoder().encode(adimlar))
    }
}
