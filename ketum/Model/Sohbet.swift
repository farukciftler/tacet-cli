//
//  Sohbet.swift
//  ketum
//
//  Bir sohbet oturumu (spec §4.7). Geçmiş cihazda SwiftData ile saklanır;
//  kullanıcı yeni sohbet açabilir ve eskilere erişebilir. Her sohbetin
//  mesajları ilişki üzerinden bağlıdır (silinince mesajlar da silinir).
//

import Foundation
import SwiftData

@Model
final class Sohbet {
    var id: UUID = UUID()
    var baslik: String = "Yeni sohbet"
    var olusturulma: Date = Date()
    var guncelleme: Date = Date()

    @Relationship(deleteRule: .cascade, inverse: \Mesaj.sohbet)
    var mesajlar: [Mesaj] = []

    init(baslik: String = "Yeni sohbet") {
        self.id = UUID()
        self.baslik = baslik
        self.olusturulma = Date()
        self.guncelleme = Date()
    }

    /// Zamana göre sıralı mesajlar (ilişki sırasız gelebilir).
    var siraliMesajlar: [Mesaj] {
        mesajlar.sorted { $0.olusturulma < $1.olusturulma }
    }

    var bosMu: Bool { mesajlar.isEmpty }

    /// Listede gösterilecek son satır önizlemesi.
    var onizleme: String {
        siraliMesajlar.last?.icerik ?? "Henüz mesaj yok"
    }
}
