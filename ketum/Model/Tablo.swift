//
//  Tablo.swift
//  ketum
//
//  Dosya üretim deseni 1 (spec §7.3.2): model @Generable Tablo üretir → üretim
//  aracı dosyayı yazar. Tip güvenli satırlar; model serbest metin parse ettirmez.
//  Pratik sınır ~50–100 satır (bağlam penceresi). Toplu cihaz verisi buraya girmez.
//

import Foundation
import FoundationModels

/// Tek satır — hücreleri başlıklarla aynı sırada.
@Generable
struct Satir: Equatable {
    @Guide(description: "Bu satırın hücreleri; başlıklarla aynı sırada ve sayıda, metin olarak.")
    var hucreler: [String]

    init(hucreler: [String]) { self.hucreler = hucreler }
}

/// Yapılandırılmış tablo — elektronik tablo (xlsx) ve markdown tablosu için.
@Generable
struct Tablo: Equatable {
    @Guide(description: "Sütun başlıkları, soldan sağa.")
    var basliklar: [String]

    @Guide(description: "Satırlar. Her satır başlık sayısı kadar hücre içerir.")
    var satirlar: [Satir]

    init(basliklar: [String], satirlar: [Satir]) {
        self.basliklar = basliklar
        self.satirlar = satirlar
    }

    /// Modele/okuyucuya dönecek düz metin özeti (kısa).
    var ozet: String {
        let bas = basliklar.joined(separator: " | ")
        let ilk = satirlar.prefix(5).map { $0.hucreler.joined(separator: " | ") }.joined(separator: "\n")
        let fazla = satirlar.count > 5 ? "\n… (+\(satirlar.count - 5) satır)" : ""
        return "\(bas)\n\(ilk)\(fazla)"
    }

    /// Modele dönecek markdown — satır sayısı sınırlı (4096 bağlam bütçesi).
    /// Düz `ozet` yerine bu kullanılır: model metni neredeyse olduğu gibi aktarır,
    /// `markdownDan` onu ayrıştırır ve sohbette gerçek tablo çizilir.
    func markdownKirpik(enFazlaSatir: Int) -> String {
        guard satirlar.count > enFazlaSatir else { return markdown }
        let kisa = Tablo(basliklar: basliklar, satirlar: Array(satirlar.prefix(enFazlaSatir)))
        // Not satırı tablo bloğundan sonra gelir; "|" ile başlamadığı için ayrıştırmayı bozmaz.
        return kisa.markdown + "\n… (+\(satirlar.count - enFazlaSatir) satır daha)"
    }

    // MARK: - Ayrıştırma (toleranslı)

    /// Metnin tablo/metin bloklarına ayrılmış hâli.
    ///
    /// Tek doğruluk kaynağı budur: `markdownTablolari` de sohbet gövdesi de
    /// bunu kullanır. Böylece "tanınmayan pipe satırı sessizce kayboldu"
    /// hatası yapısal olarak imkânsız — tanınmayan her satır `.metin` bloğuna
    /// düşer (denetim P1-5).
    enum MetinBlogu: Equatable {
        case metin(String)
        case tablo(Tablo)
    }

    /// Metni sırayı koruyarak bloklara böler. HİÇBİR satır düşürülmez.
    static func bloklara(_ metin: String) -> [MetinBlogu] {
        let satirlar = metin.components(separatedBy: "\n")
        var sonuc: [MetinBlogu] = []
        var tampon: [String] = []
        var kodBloguIcinde = false

        func metniBosalt() {
            let t = tampon.joined(separator: "\n")
                .trimmingCharacters(in: .whitespacesAndNewlines)
            if !t.isEmpty { sonuc.append(.metin(t)) }
            tampon = []
        }

        var i = 0
        while i < satirlar.count {
            let s = satirlar[i].trimmingCharacters(in: .whitespaces)
            // ``` çitleri: kod bloğu içindeki "|" tabloya dönüşmez.
            if s.hasPrefix("```") {
                kodBloguIcinde.toggle()
                tampon.append(satirlar[i])
                i += 1
                continue
            }
            if !kodBloguIcinde, let (tablo, son) = tabloDene(satirlar, i) {
                metniBosalt()
                sonuc.append(.tablo(tablo))
                i = son
            } else {
                tampon.append(satirlar[i])
                i += 1
            }
        }
        metniBosalt()
        return sonuc
    }

    /// Metindeki tüm markdown tablolarını çıkarır.
    static func markdownTablolari(_ metin: String) -> [Tablo] {
        bloklara(metin).compactMap { if case .tablo(let t) = $0 { return t } else { return nil } }
    }

    /// Metindeki ilk markdown tablosu (yoksa nil).
    static func markdownDan(_ metin: String) -> Tablo? {
        markdownTablolari(metin).first
    }

    /// `i` konumundan başlayan bir tablo var mı? Varsa tablo + bittiği satır indisi.
    ///
    /// KABUL SINIRI (yanlış pozitif üretmemek için):
    /// 1. Aday satırda en az bir `|` VE ayrıştırınca en az 2 hücre olmalı.
    /// 2. En az İKİ ardışık aday satır gerekir (başlık + veri). Düz metindeki
    ///    tek bir `|` asla tablo olmaz.
    /// 3. Ayraç satırı (`|---|`, `|:--|--:|`) OPSİYONELDİR; varsa atlanır.
    /// 4. Dış boru (satır başı/sonu `|`) OPSİYONELDİR — ancak dış boru YOKSA
    ///    tolerans daraltılır: bloktaki bütün satırların hücre sayısı BİRE BİR
    ///    aynı olmalı. Serbest metnin ("Ali | Veli" gibi) kazara tabloya
    ///    dönüşmesini engelleyen sınır budur.
    private static func tabloDene(_ satirlar: [String], _ i: Int) -> (Tablo, Int)? {
        // Ardışık aday satır dizisi.
        var j = i
        while j < satirlar.count, adaySatirMi(satirlar[j]) { j += 1 }
        guard j - i >= 2 else { return nil }

        let basliklar = hucreleAyir(satirlar[i])
        guard basliklar.count >= 2, basliklar.contains(where: { !$0.isEmpty }) else { return nil }

        // Dış boru tutarlılığı: hepsi "|" ile başlıyor mu?
        let hepsiDisBorulu = (i..<j).allSatisfy {
            satirlar[$0].trimmingCharacters(in: .whitespaces).hasPrefix("|")
        }
        if !hepsiDisBorulu {
            // Gevşek biçim — sütun sayısı birebir tutmalı.
            let tutar = (i..<j).allSatisfy { hucreleAyir(satirlar[$0]).count == basliklar.count }
            guard tutar else { return nil }
        }

        // Ayraç satırı varsa atla.
        var veriBas = i + 1
        if ayracSatiriMi(satirlar[veriBas]) { veriBas += 1 }
        guard veriBas < j else { return nil }

        var satirVerisi: [Satir] = []
        for k in veriBas..<j {
            // Gövde ortasına düşmüş ikinci bir ayraç satırı veriye girmez.
            if ayracSatiriMi(satirlar[k]) { continue }
            let h = hucreleAyir(satirlar[k])
            guard h.contains(where: { !$0.isEmpty }) else { continue }
            satirVerisi.append(Satir(hucreler: h))
        }
        guard !satirVerisi.isEmpty else { return nil }
        return (Tablo(basliklar: basliklar, satirlar: satirVerisi), j)
    }

    /// Tablo satırı adayı: `|` içerir ve en az 2 hücreye ayrılır.
    private static func adaySatirMi(_ satir: String) -> Bool {
        let s = satir.trimmingCharacters(in: .whitespaces)
        guard s.contains("|") else { return false }
        return hucreleAyir(s).count >= 2
    }

    /// `---`, `:---`, `---:`, `:---:` hücrelerinden oluşan ayraç satırı.
    private static func ayracSatiriMi(_ satir: String) -> Bool {
        let h = hucreleAyir(satir).filter { !$0.isEmpty }
        guard !h.isEmpty else { return false }
        return h.allSatisfy { hucre in
            var s = Substring(hucre)
            if s.hasPrefix(":") { s = s.dropFirst() }
            if s.hasSuffix(":") { s = s.dropLast() }
            return !s.isEmpty && s.allSatisfy { $0 == "-" }
        }
    }

    private static func hucreleAyir(_ satir: String) -> [String] {
        var s = satir.trimmingCharacters(in: .whitespaces)
        if s.hasPrefix("|") { s.removeFirst() }
        if s.hasSuffix("|") { s.removeLast() }
        return s.components(separatedBy: "|").map { $0.trimmingCharacters(in: .whitespaces) }
    }

    /// Markdown tablosu gösterimi.
    var markdown: String {
        guard !basliklar.isEmpty else { return "" }
        let bas = "| " + basliklar.joined(separator: " | ") + " |"
        let ayrac = "| " + basliklar.map { _ in "---" }.joined(separator: " | ") + " |"
        let govde = satirlar.map { s in
            let h = (0..<basliklar.count).map { i in i < s.hucreler.count ? s.hucreler[i] : "" }
            return "| " + h.joined(separator: " | ") + " |"
        }.joined(separator: "\n")
        return ([bas, ayrac] + [govde]).joined(separator: "\n")
    }
}
