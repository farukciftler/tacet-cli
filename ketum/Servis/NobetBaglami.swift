//
//  NobetBaglami.swift
//  ketum
//
//  Nöbet kurma bağlamı: sohbet aracının (NobetAraci) nöbet oluşturmak için
//  eriştiği katman. ModelServisi sahibidir; ContentView SwiftData bağlamını verir.
//

import Foundation
import SwiftData
import Observation

@MainActor
@Observable
final class NobetBaglami {
    var kayit: ModelContext?
    let servis = NobetServisi()

    /// Son kurulumda yutulmayan ama kurulumu başarısız da saymayan hata: ilk brifing
    /// üretilemedi. Nöbet kurulmuştur, yalnızca günlüğün ilk satırı yazılamamıştır.
    /// Sunan yüzey (YeniNobet / NobetAraci) isterse okuyup kullanıcıya söyleyebilir.
    var sonBrifingHatasi: String?

    /// Yeni bir nöbet kurar, ilk brifingi üretip bildirimini planlar.
    /// - Returns: nöbetin kendisi diske yazılabildiyse `true`.
    func kur(ad: String, saat: Int, takvim: Bool, hatirlatici: Bool, notlar: Bool) async -> Bool {
        guard let kayit else { return false }
        sonBrifingHatasi = nil
        await servis.izinIste()
        let n = Nobet(ad: ad, saat: saat, takvimDahil: takvim,
                      hatirlaticiDahil: hatirlatici, notDahil: notlar)
        kayit.insert(n)
        do {
            try kayit.save()
        } catch {
            // Nöbet diske yazılamadı: yarım kurulum bırakmıyoruz, kurulum başarısız.
            kayit.rollback()
            sonBrifingHatasi = error.localizedDescription
            return false
        }
        do {
            try await servis.calistir(n, baglam: String(localized: "kurulumda hazırlandı"), kayit: kayit)
        } catch {
            // Nöbet kuruldu ve bildirimi planlandı; yalnızca ilk günlük satırı yazılamadı.
            // Kurulumu başarısız saymak yanıltıcı olurdu — hatayı sessizce yutmak yerine
            // saklıyoruz; brifing bir sonraki açılışta yeniden denenir.
            sonBrifingHatasi = error.localizedDescription
        }
        return true
    }
}
