//
//  AracYurutucu.swift
//  ketum
//
//  Araç çipi ↔ tool eşlemesi (spec §7.4). Tek doğruluk kaynağı: aracın kendisi.
//  Tool çağrısı başlayınca "çalışıyor" çipi düşer; tool dönünce çip son
//  durumuna geçer. Çip metni araç tarafından üretilir, modele yazdırılmaz.
//

import Foundation
import Observation

/// Araçların çip yaşam döngüsünü bildirdiği arayüz. MainActor — UI durumu.
@MainActor
protocol AracRaporlayici: AnyObject, Sendable {
    /// "Çalışıyor" çipi düşürür, çip kimliğini döndürür.
    func baslat(ikon: String, metin: String) -> UUID
    /// Çipi son durumuna geçirir. Verilmeyen alanlar korunur.
    func guncelle(_ id: UUID, durum: AracDurumu, metin: String?, hamGirdi: String?, hamCikti: String?, dosyaYolu: String?)
}

/// Aktif turdaki araç çiplerini biriktiren yürütücü. ModelServisi sahibidir;
/// SohbetGorunumu canlı çipleri buradan gözlemler, tur bitince Mesaj'a taşınır.
@MainActor
@Observable
final class AracYurutucu: AracRaporlayici {
    /// Aktif turda düşen çipler, çağrı sırasına göre.
    private(set) var izler: [AracIzi] = []

    /// Yeni tur — önceki turun çipleri Mesaj'a taşındıktan sonra sıfırlanır.
    func yeniTur() { izler = [] }

    func baslat(ikon: String, metin: String) -> UUID {
        let iz = AracIzi(ikon: ikon, metin: metin, durum: .calisiyor)
        izler.append(iz)
        return iz.id
    }

    func guncelle(_ id: UUID,
                  durum: AracDurumu,
                  metin: String? = nil,
                  hamGirdi: String? = nil,
                  hamCikti: String? = nil,
                  dosyaYolu: String? = nil) {
        guard let i = izler.firstIndex(where: { $0.id == id }) else { return }
        izler[i].durum = durum
        if let metin { izler[i].metin = metin }
        if let hamGirdi { izler[i].hamGirdi = hamGirdi }
        if let hamCikti { izler[i].hamCikti = hamCikti }
        if let dosyaYolu { izler[i].dosyaYolu = dosyaYolu }
    }
}
