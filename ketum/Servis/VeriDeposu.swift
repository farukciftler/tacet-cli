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

    /// Elde duran referanslar — "ref (etiket, N satır)" biçiminde.
    ///
    /// Profil değişimi oturumu YENİDEN KURAR ve transcript özete iner; bu sırada
    /// araçların döndürdüğü `kaynakRef`ler modelin bağlamından düşüyordu.
    /// Ölçülen sonuç: "namaz vakitleri" aranıp bulunduktan sonra "tablo yapsana"
    /// denince belge profiline geçiliyor, model elindeki veriyi göremiyor ve
    /// beceri dosyasındaki ÖRNEĞİ gerçek içerik sanıp alakasız bir dosya
    /// üretiyordu. Veri kaybolmuş değildi — yalnızca adresi unutulmuştu.
    var referanslar: [String] {
        depo.keys.sorted().compactMap { ref in
            guard let t = depo[ref] else { return nil }
            return "\(ref) (\(t.basliklar.joined(separator: "/")), \(t.satirlar.count) satır)"
        }
    }

    /// Yeni sohbete geçişte depoyu boşaltır.
    ///
    /// Sayaç KASITLI olarak sıfırlanmaz: monotonik artmaya devam eder. Sıfırlansaydı
    /// yeni sohbette yine "takvim-1" üretilirdi ve modelin bağlamında (ya da özet
    /// metninde) kalmış eski bir referans yepyeni bir tabloya çarpardı — kullanıcı
    /// başka bir sohbetin verisinden belge üretilmiş olurdu. Sessiz ve teşhisi zor
    /// bir veri karışması; ID'leri ucuza benzersiz tutmak bunu tümden ortadan kaldırır.
    func temizle() {
        depo.removeAll()
    }
}
