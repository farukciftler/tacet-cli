//
//  ZamanAraci.swift
//  ketum
//
//  Yardımcı araç (spec §7.3). Şu anki tarih/saati verir. Foundation, ağ yok.
//  Spec notu: bu araç için çip gösterilmez — önemsiz. Yine de raporlayıcı
//  taşır (sözleşme), fakat çalışırken çip düşürmez.
//

import Foundation
import FoundationModels

struct ZamanAraci: KetumAraci {
    let name = "zaman"
    let description = "Gives the current date, time, or day of week. Call this whenever the user asks for the current time/date (e.g. 'what time is it', 'what day is today'), in any language; never guess the date."

    weak var raporlayici: (any AracRaporlayici)?

    @Generable
    struct Arguments {
        @Guide(description: "İstenen bilgi: 'saat', 'tarih', 'gun' veya 'hepsi'. Emin değilsen 'hepsi'.")
        var tur: String
    }

    func call(arguments: Arguments) async throws -> String {
        let simdi = Date()
        // Dil-nötr çıktı: ISO tarih + 24s saat + İngilizce gün adı. Model bunu
        // kullanıcının diline çevirir (çok dilli — tarih/saat metnini asla papağanlamaz).
        let tf = DateFormatter()
        tf.locale = Locale(identifier: "en_US_POSIX")
        tf.dateFormat = "HH:mm"
        let saat = tf.string(from: simdi)
        tf.dateFormat = "yyyy-MM-dd"
        let tarih = tf.string(from: simdi)
        tf.dateFormat = "EEEE"
        let gun = tf.string(from: simdi)

        switch arguments.tur.lowercased() {
        case "saat": return "time=\(saat)"
        case "tarih": return "date=\(tarih)"
        case "gun": return "weekday=\(gun)"
        default: return "time=\(saat) date=\(tarih) weekday=\(gun)"
        }
    }
}
