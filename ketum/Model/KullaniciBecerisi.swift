//
//  KullaniciBecerisi.swift
//  ketum
//
//  Kullanıcının kendi yazdığı beceri (SKILL.md karşılığı). Paket becerileri
//  Beceriler/*.md içinde salt-okunur durur; bu model kullanıcının eklediklerini
//  SwiftData'da saklar. Gövde 4096 token penceresini korumak için sert sınırlı:
//  eşleşen beceri o oturuma bir kez enjekte edilir, dolayısıyla her karakter bütçedir.
//

import Foundation
import SwiftData

@Model
final class KullaniciBecerisi {
    /// Gövde üst sınırı — paket becerilerinin gövdesiyle aynı mertebede (~150 token).
    static let govdeSiniri = 500
    /// Tetikleyici üst sınırı — eşleşme taraması her mesajda döndüğü için sınırlı.
    static let tetikSiniri = 12

    var id: UUID = UUID()
    /// Görünen ad; enjeksiyon başlığı olarak da kullanılır.
    var ad: String = ""
    /// Ham tetikleyici metni ("fatura, gider, harcama"). Ayrıştırma `tetikler`de.
    var tetiklerHam: String = ""
    /// Kılavuz metni — modele verilecek talimat.
    var govde: String = ""
    var aktif: Bool = true
    var olusturulma: Date = Date()

    init(ad: String = "", tetiklerHam: String = "", govde: String = "") {
        self.id = UUID()
        self.ad = ad
        self.tetiklerHam = tetiklerHam
        self.govde = govde
        self.olusturulma = Date()
    }

    /// Normalize tetikleyiciler: küçük harf, boşluk kırpılmış, boşlar atılmış, sınırlı.
    var tetikler: [String] {
        tetiklerHam
            .split(separator: ",")
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
            .filter { !$0.isEmpty }
            .prefix(Self.tetikSiniri)
            .map { $0 }
    }

    /// Kaydedilebilir mi — üçü de dolu ve gövde sınır içinde olmalı.
    var gecerliMi: Bool {
        !ad.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !tetikler.isEmpty
            && !govde.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && govde.count <= Self.govdeSiniri
    }

    /// Panoda gösterilen altyazı: "fatura · gider · harcama".
    var ozet: String { tetikler.joined(separator: " · ") }
}
