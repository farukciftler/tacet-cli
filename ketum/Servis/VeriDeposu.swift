//
//  VeriDeposu.swift
//  ketum
//
//  Bağlam penceresini aşmadan büyük veri taşıma kanalı (spec §7.2, §7.3.2 akış 2).
//  Kaynak araç (ör. Takvim) topladığı yapılandırılmış veriyi buraya koyar ve modele
//  YALNIZCA kısa bir özet + referans ID döner. Üretim aracı (belge_olustur) veriyi
//  bu depodan çeker — toplu veri hiçbir zaman bağlam penceresine girmez. Dosya
//  boyutunu pencere değil cihaz sınırlar.
//

import Foundation
import Observation

@MainActor
@Observable
final class VeriDeposu {
    /// Referans ID → tablo. Büyük tablolar burada yaşar, model bunları görmez.
    private var depo: [String: Tablo] = [:]
    private var sayac = 0

    /// Tabloyu saklar, kısa bir referans ID döndürür. Model bu ID'yi taşır.
    func koy(_ tablo: Tablo, etiket: String) -> String {
        sayac += 1
        let ref = "\(etiket)-\(sayac)"
        depo[ref] = tablo
        return ref
    }

    /// Referansla tam tabloyu çeker (üretim aracı çağırır).
    func al(_ ref: String) -> Tablo? {
        // Model bazen tırnak/boşluk ekleyebilir — toleranslı eşle.
        let anahtar = ref.trimmingCharacters(in: CharacterSet(charactersIn: " \"'"))
        return depo[anahtar] ?? depo.first(where: { $0.key == anahtar })?.value
    }

    var bosMu: Bool { depo.isEmpty }

    /// Yeni sohbete geçişte depoyu boşaltır.
    func temizle() {
        depo.removeAll()
        sayac = 0
    }
}
