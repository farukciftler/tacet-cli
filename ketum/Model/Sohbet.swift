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
    /// Başlık hâlâ varsayılan mı (kullanıcı/ilk mesaj tarafından belirlenmedi mi)?
    /// Metinle karşılaştırma yapmıyoruz: "Yeni sohbet" çevrildiğinde literal
    /// eşleşmesi bozulur ve başlık hiç güncellenmezdi. Varsayılan değerli olduğu
    /// için lightweight migration ile eski kayıtlar sorunsuz açılır.
    var baslikOtomatik: Bool = true

    @Relationship(deleteRule: .cascade, inverse: \Mesaj.sohbet)
    var mesajlar: [Mesaj] = []

    init(baslik: String = "Yeni sohbet", baslikOtomatik: Bool = true) {
        self.id = UUID()
        self.baslik = baslik
        self.baslikOtomatik = baslikOtomatik
        self.olusturulma = Date()
        self.guncelleme = Date()
    }

    /// İlk kullanıcı mesajından başlığı türetir. Başlık zaten elle/otomatik
    /// belirlenmişse dokunmaz.
    func basligiTuret(_ metin: String) {
        guard baslikOtomatik else { return }
        let ozet = metin.trimmingCharacters(in: .whitespacesAndNewlines).prefix(40)
        guard !ozet.isEmpty else { return }
        baslik = String(ozet)
        baslikOtomatik = false
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
