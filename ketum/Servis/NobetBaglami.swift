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

    /// Yeni bir nöbet kurar, ilk brifingi üretip bildirimini planlar.
    func kur(ad: String, saat: Int, takvim: Bool, hatirlatici: Bool, notlar: Bool) async -> Bool {
        guard let kayit else { return false }
        await servis.izinIste()
        let n = Nobet(ad: ad, saat: saat, takvimDahil: takvim,
                      hatirlaticiDahil: hatirlatici, notDahil: notlar)
        kayit.insert(n)
        try? kayit.save()
        await servis.calistir(n, baglam: String(localized: "kurulumda hazırlandı"), kayit: kayit)
        return true
    }
}
