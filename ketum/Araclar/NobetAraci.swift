//
//  NobetAraci.swift
//  ketum
//
//  Sohbetle nöbet kurma (mockup akış B). Kullanıcı "her sabah brifing hazırla"
//  dediğinde sirr nöbeti kurar ve bunu araç çipiyle kanıtlar. Yazma → onay imi.
//

import Foundation
import FoundationModels

struct NobetAraci: KetumAraci {
    let name = "nobet_kur"
    let description = "Sets up a recurring scheduled task ('nöbet'), e.g. a daily morning briefing. Call this when the user asks sirr to do something every day / every morning / regularly (e.g. 'prepare my morning briefing every day', 'give me a daily summary', in any language). Not for a one-off reminder — use the reminder tool for that."

    weak var raporlayici: (any AracRaporlayici)?
    weak var baglam: NobetBaglami?

    @Generable
    struct Arguments {
        @Guide(description: "Short name for the task, e.g. 'Sabah brifingi' / 'Morning briefing'.")
        var ad: String?
        @Guide(description: "Delivery hour of day, 0–23. Default 7 (morning).")
        var saat: Int?
    }

    func call(arguments: Arguments) async -> String {
        await cipliCalis(ikon: "moon.stars", calisiyorMetni: Yerel.nobetKuruluyor) {
            let ad = (arguments.ad?.isEmpty == false) ? arguments.ad! : String(localized: "Sabah brifingi")
            let saat = min(23, max(0, arguments.saat ?? 7))
            let ok = await baglam?.kur(ad: ad, saat: saat,
                                       takvim: true, hatirlatici: true, notlar: false) ?? false
            guard ok else {
                return AracSonucu(cipMetni: Yerel.nobetHata, durum: .basarisiz("kurulamadı"),
                                  modeleDonen: "could_not_create_shift")
            }
            let ss = String(format: "%02d.00", saat)
            return AracSonucu(
                cipMetni: Yerel.nobetKuruldu,
                durum: .yazildi,
                modeleDonen: "shift_created: '\(ad)', daily at \(ss). Every day it prepares the briefing and notifies at \(ss); if not run overnight it prepares on app open.")
        }
    }
}
