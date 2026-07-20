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
    // "fark" AYRI bir eylem olarak anılır. Ölçülen hata: takvim aritmetiği
    // yapabilen HİÇBİR araç yokken model soruyu yanıtsız bırakmıyordu —
    // uyduruyordu (19 Temmuz → 2 Aralık arasına "6 gün" dedi). Yeteneği
    // vermeden "kendin hesaplama" demek, kendinden emin uydurma üretiyor.
    // HesapAraci bunu çözemez: yalnız rakam ve operatör kabul eder, takvim
    // bilmez (artık yıl, ay uzunlukları). Fark burada, Calendar ile hesaplanır.
    let description = "Gives the current date/time/day of week, and counts days between today and another date. Call this whenever the user asks for the current time/date OR asks how many days/weeks until (or since) a date — in any language. NEVER compute a date difference yourself: calendar arithmetic needs leap years and month lengths, so it must be calculated here."

    weak var raporlayici: (any AracRaporlayici)?

    /// Tür serbest metin DEĞİL (P0-4): beş serbest değeri @Guide'da saymak,
    /// modelin altıncısını uydurmasını engellemiyordu; `tur.lowercased() == "fark"`
    /// dışındaki HER değer sessizce "hepsi"ye düşüyordu — "difference" yazan
    /// model gün farkı yerine saat/tarih alıyordu.
    @Generable
    enum Tur: String, Equatable, CaseIterable {
        case saat
        case tarih
        case gun
        case hepsi
        case fark
    }

    @Generable
    struct Arguments {
        @Guide(description: "What to return: 'saat' (time), 'tarih' (date), 'gun' (day of week), 'hepsi' (all), or 'fark' (days until/since the date given in 'hedef'). If unsure use 'hepsi'.")
        var tur: Tur
        @Guide(description: "Only for tur='fark': the other date, exactly as the user wrote it (e.g. '2 aralık 2026', '2026-12-02', 'next Friday'). Leave empty otherwise.")
        var hedef: String
    }

    func call(arguments: Arguments) async throws -> String {
        // "saat kaç" ÇİPSİZ kalır (spec: önemsiz, akışı kalabalıklaştırır).
        // "fark" ÇİP DÜŞÜRÜR: bir sayı üretiyor ve bu sayı AYRIŞTIRILMIŞ bir
        // girdiye dayanıyor — tarih yanlış okunmuş olabilir. Çip detayında
        // "from=… to=… days=…" görünür, kullanıcı yanlış ayrıştırmayı yakalar.
        // Kullanıcının doğrulaması gereken bir sayıyı gizlemek, seyrin
        // "sirr yaptığını gizlemez" ilkesini deler.
        guard arguments.tur == .fark else {
            return Self.simdi(tur: arguments.tur)
        }
        let hedefHam = arguments.hedef
        return await cipliCalis(ikon: "calendar",
                                calisiyorMetni: Yerel.gunSayiliyor,
                                hamGirdi: hedefHam) {
            let cikti = Self.fark(hedefHam: hedefHam)
            guard !cikti.hasPrefix("error:") else {
                return AracSonucu(
                    cipMetni: Yerel.tarihAnlasilmadi,
                    durum: .basarisiz(Yerel.tarihAnlasilmadi),
                    modeleDonen: cikti,
                    hamCikti: cikti
                )
            }
            return AracSonucu(
                cipMetni: Yerel.gunSayildi,
                durum: .okundu,
                modeleDonen: cikti,
                hamCikti: cikti
            )
        }
    }

    /// Bugün ile hedef tarih arasındaki TAM gün sayısı. Her iki tarih de gün
    /// başına indirgenir; aksi halde saat farkı sayıyı bir gün kaydırır.
    /// Geçmiş tarihte negatif döner — model "geçti" diyebilsin.
    static func fark(hedefHam: String) -> String {
        guard let cozum = ZamanCozucu.coz(hedefHam) else {
            // Sessizce bugüne düşmek YOK: çözülemeyen tarih hata olarak döner,
            // yoksa model "0 gün" görüp bunu cevap sanır.
            return "error: could not parse the date \"\(hedefHam)\". Ask the user to restate it."
        }
        let takvim = Calendar.current
        let bugun = takvim.startOfDay(for: Date())
        let hedef = takvim.startOfDay(for: cozum.tarih)
        guard let gun = takvim.dateComponents([.day], from: bugun, to: hedef).day else {
            return "error: date difference could not be computed."
        }

        let bicim = DateFormatter()
        bicim.locale = Locale(identifier: "en_US_POSIX")
        bicim.dateFormat = "yyyy-MM-dd"
        // Dil-nötr çıktı; model kullanıcının diline çevirir. İşaret bilinçli
        // olarak korunur (negatif = geçmiş) — modelin yön uydurmasını engeller.
        return "from=\(bicim.string(from: bugun)) to=\(bicim.string(from: hedef)) days=\(gun)"
    }

    static func simdi(tur: Tur) -> String {
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

        switch tur {
        case .saat:  return "time=\(saat)"
        case .tarih: return "date=\(tarih)"
        case .gun:   return "weekday=\(gun)"
        // `.fark` buraya gelmez (çağıran ayırır) ama exhaustive switch
        // gereği açıkça yazılır: sessiz `default` dalı bırakmıyoruz.
        case .hepsi, .fark: return "time=\(saat) date=\(tarih) weekday=\(gun)"
        }
    }
}
