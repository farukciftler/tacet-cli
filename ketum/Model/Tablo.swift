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

    /// Metindeki tüm markdown tablolarını (| … | satırları) çıkarır.
    /// Hem belge üretimi hem de sohbet içi tablo gösterimi bunu kullanır.
    static func markdownTablolari(_ metin: String) -> [Tablo] {
        var sonuc: [Tablo] = []
        let satirlar = metin.components(separatedBy: "\n")
        var i = 0
        while i < satirlar.count {
            let s = satirlar[i].trimmingCharacters(in: .whitespaces)
            // Bir tablo: "|…|" başlık + "|---|" ayraç + en az bir satır.
            if s.hasPrefix("|"), i + 1 < satirlar.count,
               satirlar[i + 1].contains("-"),
               satirlar[i + 1].trimmingCharacters(in: .whitespaces).hasPrefix("|") {
                let basliklar = hucreleAyir(s)
                var satirVerisi: [Satir] = []
                var j = i + 2
                while j < satirlar.count {
                    let r = satirlar[j].trimmingCharacters(in: .whitespaces)
                    guard r.hasPrefix("|") else { break }
                    satirVerisi.append(Satir(hucreler: hucreleAyir(r)))
                    j += 1
                }
                if !basliklar.isEmpty, !satirVerisi.isEmpty {
                    sonuc.append(Tablo(basliklar: basliklar, satirlar: satirVerisi))
                }
                i = j
            } else {
                i += 1
            }
        }
        return sonuc
    }

    /// Metindeki ilk markdown tablosu (yoksa nil).
    static func markdownDan(_ metin: String) -> Tablo? {
        markdownTablolari(metin).first
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
