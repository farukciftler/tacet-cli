//
//  BelgeMotoru.swift
//  ketum
//
//  Biçime özgü yazma/okuma motorları için ortak sözleşme. Araçlar bu katmanı
//  çağırır; her motor tek bir biçimden sorumludur (Excel, PDF, Word, Metin).
//  Motorlar saf mantıktır — Tool protokolü ya da UI bilmez. Ağ yok.
//

import Foundation

/// Bir belgeden okunan içerik. Prose belgeler `metin`, tablo belgeler `tablo`.
struct BelgeIcerik {
    var metin: String        // düz/markdown metin (prose + çıkarılan metin)
    var tablo: Tablo?        // varsa yapılandırılmış tablo (xlsx)

    init(metin: String, tablo: Tablo? = nil) {
        self.metin = metin
        self.tablo = tablo
    }
}

enum BelgeMotorHatasi: LocalizedError {
    case desteklenmiyor(String)
    case icerikYok
    var errorDescription: String? {
        switch self {
        case .desteklenmiyor(let m): return m
        case .icerikYok: return "Belgede içerik bulunamadı"
        }
    }
}

/// Tek biçimden sorumlu motor.
protocol BelgeMotoru {
    var bicim: BelgeBicimi { get }

    /// Dosya yazar, URL döndürür. `govde`: prose/markdown metin; `tablo`: xlsx için.
    /// `dosyaAdi` uzantısız temel ad; motor doğru uzantıyı ekler. `klasor`: hedef.
    func yaz(dosyaAdi: String, baslik: String?, govde: String?, tablo: Tablo?, klasor: URL) throws -> URL

    /// Dosyadan içerik çıkarır (okuma/düzenleme için).
    func oku(url: URL) throws -> BelgeIcerik
}

extension BelgeMotoru {
    /// Benzersiz, güvenli dosya adı üretir (çakışma olursa sayı ekler).
    func hedefURL(dosyaAdi: String, klasor: URL) -> URL {
        let temiz = dosyaAdi
            .replacingOccurrences(of: "/", with: "-")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let taban = temiz.isEmpty ? "ketum-belge" : temiz
        var aday = klasor.appendingPathComponent("\(taban).\(bicim.uzanti)")
        var i = 2
        while FileManager.default.fileExists(atPath: aday.path) {
            aday = klasor.appendingPathComponent("\(taban)-\(i).\(bicim.uzanti)")
            i += 1
        }
        return aday
    }
}

/// Biçim → motor eşlemesi. Araçlar buradan motoru alır.
enum BelgeMotorlari {
    static func motor(_ bicim: BelgeBicimi) -> BelgeMotoru {
        switch bicim {
        case .xlsx: return ExcelMotor()
        case .pdf:  return PdfMotor()
        case .docx: return DocxMotor()
        case .md, .txt: return MetinMotor(bicim: bicim)
        }
    }
}
