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
    /// Tablo OLMAYAN büyük içerik (okunan belgenin düz gövdesi — P2-6).
    ///
    /// Ayrı bir sözlük tutulur çünkü düzyazıyı tek sütunluk bir `Tablo`ya
    /// sıkıştırmak, o ref'ten üretilen HER belgeyi (docx/pdf/md) tabloya
    /// çevirirdi: metni sessizce başka bir şeye dönüştüren yeni bir bozulma
    /// sınıfı. Tip ayrımı bunu baştan imkânsız kılar.
    private var metinDepo: [String: String] = [:]
    private var sayac = 0

    /// Tabloyu saklar, kısa bir referans ID döndürür. Model bu ID'yi taşır.
    func koy(_ tablo: Tablo, etiket: String) -> String {
        sayac += 1
        let ref = "\(etiket)-\(sayac)"
        depo[ref] = tablo
        return ref
    }

    /// Düz metin gövdeyi saklar (aynı ref uzayı — ID'ler çakışmaz).
    func koyMetin(_ metin: String, etiket: String) -> String {
        sayac += 1
        let ref = "\(etiket)-\(sayac)"
        metinDepo[ref] = metin
        return ref
    }

    /// Referansla tam tabloyu çeker (üretim aracı çağırır).
    func al(_ ref: String) -> Tablo? {
        depo[Self.normalize(ref)]
    }

    /// Referansla tam METİN gövdeyi çeker.
    func alMetin(_ ref: String) -> String? {
        metinDepo[Self.normalize(ref)]
    }

    /// Ref herhangi bir kanalda çözülüyor mu? (P0-2 hata dalı bunu sorar.)
    func cozulurMu(_ ref: String) -> Bool {
        let anahtar = Self.normalize(ref)
        return depo[anahtar] != nil || metinDepo[anahtar] != nil
    }

    /// Model referansı çıplak vermez: araçlar modele "data_ref=takvim-1" diye
    /// döndüğü için model bu ANAHTAR-DEĞER ÇİFTİNİN TAMAMINI kaynakRef'e
    /// kopyalıyor. Eski eşleme yalnız tırnak/boşluk soyduğundan depo ıskalanıyor,
    /// ıska da (P0-2) sessiz boş belgeye dönüşüyordu. Önek soymak bu sınıfın
    /// tamamını kapatır; eşleşmeyen ref artık GERÇEKTEN yok demektir.
    static func normalize(_ ham: String) -> String {
        let cop = CharacterSet(charactersIn: " \t\n\r\"'`()[]{},;")
        var s = ham.trimmingCharacters(in: cop)
        // "data_ref=takvim-1", "kaynakRef: takvim-1", "ref = takvim-1" …
        for onek in ["data_ref", "kaynakref", "kaynak_ref", "dataref", "ref"] {
            guard s.lowercased().hasPrefix(onek) else { continue }
            let kalan = s.dropFirst(onek.count).drop(while: { $0 == " " })
            // Ayırıcı ZORUNLU: yoksa "reference-1" gibi meşru bir ID kırpılırdı.
            guard let ilk = kalan.first, ilk == "=" || ilk == ":" else { continue }
            s = String(kalan.dropFirst()).trimmingCharacters(in: cop)
            break
        }
        return s
    }

    var bosMu: Bool { depo.isEmpty && metinDepo.isEmpty }

    /// Yalnız anahtarlar — çözülemeyen ref hatasında modele "elde bunlar var"
    /// diye OLGU dönmek için (imperatif yönerge değil, mevcut adres listesi).
    var refAnahtarlari: [String] { (Array(depo.keys) + Array(metinDepo.keys)).sorted() }

    /// Elde duran referanslar — "ref (etiket, N satır)" biçiminde.
    ///
    /// Profil değişimi oturumu YENİDEN KURAR ve transcript özete iner; bu sırada
    /// araçların döndürdüğü `kaynakRef`ler modelin bağlamından düşüyordu.
    /// Ölçülen sonuç: "namaz vakitleri" aranıp bulunduktan sonra "tablo yapsana"
    /// denince belge profiline geçiliyor, model elindeki veriyi göremiyor ve
    /// beceri dosyasındaki ÖRNEĞİ gerçek içerik sanıp alakasız bir dosya
    /// üretiyordu. Veri kaybolmuş değildi — yalnızca adresi unutulmuştu.
    var referanslar: [String] {
        let tablolar = depo.keys.sorted().compactMap { ref -> String? in
            guard let t = depo[ref] else { return nil }
            return "\(ref) (\(t.basliklar.joined(separator: "/")), \(t.satirlar.count) satır)"
        }
        let metinler = metinDepo.keys.sorted().compactMap { ref -> String? in
            guard let m = metinDepo[ref] else { return nil }
            return "\(ref) (metin, \(m.count) karakter)"
        }
        return tablolar + metinler
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
        metinDepo.removeAll()
    }
}
