//
//  DosyaIkonu.swift
//  ketum
//
//  Dosya tipi ikonları ve tür etiketleri — seyir-spec §9.3.
//
//  İkon seti asset kataloğunda (Assets.xcassets/DosyaIkonlari) tek renk
//  template vektör olarak yaşar: ortak 24×24 sayfa silueti + iç işaret.
//  Üçüncü taraf marka renkleri (Excel yeşili, PDF kırmızısı) yoktur; ikon
//  daima üstündeki metnin rengini alır.
//
//  Buradaki iki fonksiyon SAF'tır: aynı girdi hep aynı çıktıyı verir, yan
//  etkisi yoktur, dosya sistemine bakmaz. OtoTest'te 20 tip + eş anlamlılar
//  + geri düşüş vakalarıyla doğrulanır.
//

import SwiftUI
import UniformTypeIdentifiers

enum DosyaIkonu {

    // MARK: - Set

    /// §9.3'teki 20 tip. Asset adı "dosya-" + bu değer.
    static let bilinenTipler: [String] = [
        "pdf", "docx", "md", "txt", "rtf",       // belge
        "xlsx", "csv", "json",                   // tablo / veri
        "pptx",                                  // sunum
        "png", "jpg", "heic", "gif", "svg",      // görsel
        "mp3", "m4a", "wav",                     // ses
        "mp4", "mov",                            // video
        "zip"                                    // arşiv
    ]

    /// Jenerik geri düşüş — listede olmayan her uzantı buna düşer.
    /// Kart asla ikonsuz çizilmez.
    static let jenerikTip = "belge"

    /// Aynı içeriğin başka yazımları. Yeni bir ikon gerektirmezler, tabloya katlanırlar.
    /// Eski Office biçimleri (xls/doc/ppt) de yeni muadillerinin ikonunu kullanır:
    /// kullanıcı için ikisi de "hesap tablosu"dur, ayrı bir çizim marka gürültüsüdür.
    private static let esAnlamlilar: [String: String] = [
        "jpeg": "jpg", "jpe": "jpg",
        "markdown": "md", "mdown": "md", "mkd": "md",
        "text": "txt",
        "heif": "heic",
        "tsv": "csv",
        "xls": "xlsx", "doc": "docx", "ppt": "pptx",
        "m4v": "mp4", "qt": "mov",
        "wave": "wav", "aac": "m4a",
        "zipx": "zip"
    ]

    // MARK: - Normalleştirme

    /// Uzantıyı sadeleştirir: baştaki nokta düşer, boşluk kırpılır, küçük harfe iner.
    /// Tam dosya adı ("rapor.xlsx") verilse bile son bileşen alınır.
    static func normalUzanti(_ ham: String) -> String {
        var u = ham.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if let son = u.split(separator: ".").last, !u.isEmpty {
            u = String(son)
        }
        return u
    }

    /// Uzantı → set içindeki tip. Bilinmeyen her şey jenerik tipe düşer.
    static func tip(uzanti: String) -> String {
        let u = normalUzanti(uzanti)
        let katlanmis = esAnlamlilar[u] ?? u
        return bilinenTipler.contains(katlanmis) ? katlanmis : jenerikTip
    }

    /// Asset kataloğundaki varlık adı.
    static func varlikAdi(uzanti: String) -> String {
        "dosya-" + tip(uzanti: uzanti)
    }

    // MARK: - Genel arayüz

    /// Uzantının template vektör ikonu. Renk çağıran taraftan gelir
    /// (`.foregroundStyle`); ikonun kendi rengi yoktur.
    static func ikon(uzanti: String) -> Image {
        Image(varlikAdi(uzanti: uzanti))
            .renderingMode(.template)
    }

    /// Tür etiketi — uzantıdan değil `UTType` yerelleştirmesinden gelir
    /// ("Hesap tablosu", "PNG görseli"). Sistem çözemezse uzantı büyük
    /// harfle tek başına yazılır.
    static func turEtiketi(uzanti: String) -> String {
        let u = normalUzanti(uzanti)
        guard !u.isEmpty else { return "" }
        let buyuk = u.uppercased()
        guard let tur = UTType(filenameExtension: u),
              let tanim = tur.localizedDescription?
                  .trimmingCharacters(in: .whitespacesAndNewlines),
              !tanim.isEmpty
        else { return buyuk }
        return bastanBuyuk(tanim)
    }

    /// Sistem tanımları bazen küçük harfle başlar ("PDF belgesi" / "elektronik
    /// tablo"); etiket satır başı olduğu için ilk harf büyütülür. Zaten büyük
    /// harfle başlayan kısaltmalara (PDF, JPEG) dokunulmaz.
    private static func bastanBuyuk(_ metin: String) -> String {
        guard let ilk = metin.first, ilk.isLowercase else { return metin }
        return ilk.uppercased() + metin.dropFirst()
    }
}
