//
//  Tema.swift
//  ketum
//
//  Tasarım dili — spec §3. Tek zemin, vurgu rengi yok; renk yalnızca
//  mürekkep/gri tonlarından ibarettir. İki tipografik ses
//  (SF Pro + New York serif), 4/8/12/16/22 boşluk ölçeği.
//

import SwiftUI
import UIKit

// MARK: - Renk

/// Spec §3.1 renk token'ları. Açık/koyu moda göre dinamik çözülür.
enum Renk {
    private static func dinamik(acik: UInt, koyu: UInt) -> Color {
        Color(uiColor: UIColor { trait in
            trait.userInterfaceStyle == .dark ? UIColor(hex: koyu) : UIColor(hex: acik)
        })
    }

    static let zemin   = dinamik(acik: 0xFFFFFF, koyu: 0x141413) // ekran arka planı
    static let murekkep = dinamik(acik: 0x1C1C1A, koyu: 0xECECEA) // birincil metin, gönder dolgusu
    static let gri     = dinamik(acik: 0x8A8A84, koyu: 0x9A9A93) // ikincil metin, çip metni
    static let soluk   = dinamik(acik: 0xB9B9B2, koyu: 0x6E6E68) // placeholder, etiketler
    static let cizgi   = dinamik(acik: 0xE9E9E4, koyu: 0x2A2A28) // hairline kenarlık, çip çerçevesi
    static let dolgu   = dinamik(acik: 0xF4F4F1, koyu: 0x222220) // kullanıcı balonu arka planı
    static let hata    = dinamik(acik: 0xB4483C, koyu: 0xD46A5E) // yalnızca başarısız çip
}

extension UIColor {
    fileprivate convenience init(hex: UInt) {
        self.init(
            red:   CGFloat((hex >> 16) & 0xFF) / 255,
            green: CGFloat((hex >> 8) & 0xFF) / 255,
            blue:  CGFloat(hex & 0xFF) / 255,
            alpha: 1
        )
    }
}

// MARK: - Tipografi

/// Spec §3.2 — iki ses: kullanıcı SF Pro, sirr yanıtı New York serif.
/// Boyutlar Dynamic Type'a göreli tanımlanır; sabit px yalnızca referans.
enum Yazi {
    /// Kullanıcı metni — SF Pro 15/1.5.
    static func kullanici() -> Font { .system(.callout, design: .default) }
    /// sirr yanıtı — New York serif 17/1.6.
    static func ketum() -> Font { .system(.body, design: .serif) }
    /// Marka — New York medium 19.
    static func marka() -> Font { .system(.title3, design: .serif).weight(.medium) }
    /// Etiket — SF Pro 10, uppercase, tracking. (Tracking view tarafında uygulanır.)
    static func etiket() -> Font { .system(size: 11, weight: .regular, design: .default) }
    /// Çip / meta — SF Pro 11.
    static func cip() -> Font { .system(size: 12, weight: .regular, design: .default) }
}

// MARK: - Ölçek

/// Spec §3.3 — boşluk, köşe, çizgi.
enum Olcek {
    static let s1: CGFloat = 4
    static let s2: CGFloat = 8
    static let s3: CGFloat = 12
    static let s4: CGFloat = 16
    static let s5: CGFloat = 22   // ekran yatay kenar boşluğu

    static let mesajAraligi: CGFloat = 14   // mesajlar arası dikey
    static let cipYanitAraligi: CGFloat = 10 // çip ↔ ilişkili yanıt

    static let hairline: CGFloat = 1
    static let girisKose: CGFloat = 24
    static let cipKose: CGFloat = 20

    static let kullaniciBalonGenislik: CGFloat = 0.80 // ekran genişliği oranı
    static let ketumYanitGenislik: CGFloat = 0.88
}
