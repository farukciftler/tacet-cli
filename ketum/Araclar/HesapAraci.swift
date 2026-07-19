//
//  HesapAraci.swift
//  ketum
//
//  Yardımcı araç (spec §7.3). Aritmetik daima kodda çözülür, modelde değil
//  (spec §2 ilke 4). Instructions modeli her hesabı buraya yönlendirir.
//  Saf Swift — NSExpression ile değerlendirme, ağ yok.
//

import Foundation
import FoundationModels

struct HesapAraci: KetumAraci {
    let name = "hesapla"
    let description = "Evaluates arithmetic: addition, subtraction, multiplication, division, percentages, parentheses. ALWAYS call this for ANY numeric calculation, in any language; never compute the result yourself."

    weak var raporlayici: (any AracRaporlayici)?

    @Generable
    struct Arguments {
        @Guide(description: "Değerlendirilecek aritmetik ifade; yalnızca sayılar ve + - * / ( ) . operatörleri. Örn: '(1250+890)*1.20'.")
        var ifade: String
    }

    func call(arguments: Arguments) async -> String {
        await cipliCalis(ikon: "function", calisiyorMetni: Yerel.hesaplaniyor, hamGirdi: arguments.ifade) {
            let sonuc = try Self.degerlendir(arguments.ifade)
            let metin = Self.bicimle(sonuc)
            return AracSonucu(
                cipMetni: Yerel.hesaplandi,
                durum: .okundu,
                modeleDonen: "result=\(metin)",
                hamCikti: "\(arguments.ifade) = \(metin)"
            )
        }
    }

    enum HesapHatasi: LocalizedError {
        case gecersiz
        var errorDescription: String? { "İfade değerlendirilemedi" }
    }

    /// Güvenli aritmetik değerlendirme. NSExpression YAKALANAMAYAN ObjC exception
    /// atıp uygulamayı çökertebildiği için elle yazılmış özyinelemeli çözümleyici
    /// kullanılır — hatalı ifade asla çökme değil, `HesapHatasi.gecersiz` üretir.
    static func degerlendir(_ ham: String) throws -> Double {
        let izinli = CharacterSet(charactersIn: "0123456789.+-*/()% ")
        let temiz = ham.replacingOccurrences(of: ",", with: ".")
        guard temiz.unicodeScalars.allSatisfy({ izinli.contains($0) }),
              !temiz.trimmingCharacters(in: .whitespaces).isEmpty else {
            throw HesapHatasi.gecersiz
        }
        var cozucu = AritmetikCozucu(temiz)
        let sonuc = try cozucu.coz()
        guard sonuc.isFinite else { throw HesapHatasi.gecersiz }
        return sonuc
    }

    static func bicimle(_ d: Double) -> String {
        if d == d.rounded() && abs(d) < 1e15 {
            return String(Int(d))
        }
        let nf = NumberFormatter()
        nf.locale = Locale(identifier: "tr_TR")
        nf.maximumFractionDigits = 4
        nf.minimumFractionDigits = 0
        return nf.string(from: NSNumber(value: d)) ?? String(d)
    }
}

/// Güvenli özyinelemeli-iniş aritmetik çözümleyici. Dilbilgisi:
///   ifade = terim (('+'|'-') terim)*
///   terim = yuzde (('*'|'/') yuzde)*
///   yuzde = birim ('%')*                // postfix yüzde: 20% = 0.20
///   birim = sayı | '(' ifade ')' | ('+'|'-') birim
/// ObjC exception atmaz; hatalı girdide HesapAraci.HesapHatasi.gecersiz fırlatır.
private struct AritmetikCozucu {
    private let k: [Character]
    private var i = 0
    init(_ s: String) { k = Array(s) }

    mutating func coz() throws -> Double {
        let v = try ifade()
        bosluk()
        guard i >= k.count else { throw HesapAraci.HesapHatasi.gecersiz }
        return v
    }

    private mutating func bosluk() { while i < k.count, k[i] == " " { i += 1 } }
    private mutating func bak() -> Character? { bosluk(); return i < k.count ? k[i] : nil }

    private mutating func ifade() throws -> Double {
        var v = try terim()
        while let c = bak(), c == "+" || c == "-" {
            i += 1
            let t = try terim()
            v = (c == "+") ? v + t : v - t
        }
        return v
    }

    private mutating func terim() throws -> Double {
        var v = try yuzde()
        while let c = bak(), c == "*" || c == "/" {
            i += 1
            let t = try yuzde()
            if c == "*" { v *= t }
            else {
                guard t != 0 else { throw HesapAraci.HesapHatasi.gecersiz }
                v /= t
            }
        }
        return v
    }

    private mutating func yuzde() throws -> Double {
        var v = try birim()
        while bak() == "%" { i += 1; v /= 100 }
        return v
    }

    private mutating func birim() throws -> Double {
        guard let c = bak() else { throw HesapAraci.HesapHatasi.gecersiz }
        if c == "-" { i += 1; return -(try birim()) }
        if c == "+" { i += 1; return try birim() }
        if c == "(" {
            i += 1
            let v = try ifade()
            guard bak() == ")" else { throw HesapAraci.HesapHatasi.gecersiz }
            i += 1
            return v
        }
        return try sayi()
    }

    private mutating func sayi() throws -> Double {
        bosluk()
        var j = i
        while j < k.count, k[j].isNumber || k[j] == "." { j += 1 }
        guard j > i, let d = Double(String(k[i..<j])) else {
            throw HesapAraci.HesapHatasi.gecersiz
        }
        i = j
        return d
    }
}
