//
//  AracIzi.swift
//  ketum
//
//  Araç çipi — sistemin imzası (spec §4.4, §7.4).
//  Çipin tek doğruluk kaynağı aracın kendisidir; metin araç tarafından
//  üretilir, modele yazdırılmaz. Model çip metnini halüsine edemez.
//

import Foundation

/// Araç çipinin durumu — spec §4.4 tablosu.
enum AracDurumu: Equatable, Codable, Hashable {
    case calisiyor          // gri, spinner — "Takvime bakılıyor…"
    case okundu             // gri — bilgi okundu
    case yazildi            // onay imi — dünyada bir şey değişti
    case izinGerekli        // gri, dokunulabilir — "Takvim izni gerekli"
    case basarisiz(String)  // hata — kısa neden
}

/// Bir tool çağrısının akışta bıraktığı iz. Identifiable — çip listelenir.
/// Codable — SwiftData mesajıyla birlikte kalıcılaştırılır.
struct AracIzi: Identifiable, Codable, Hashable {
    var id: UUID = UUID()
    /// SF Symbol adı (outline). Renk daima üstündeki metnin rengi.
    var ikon: String
    /// Çip metni: en fazla ~5 kelime + isteğe bağlı `· detay`. Araç üretir.
    var metin: String
    var durum: AracDurumu
    /// Çipe dokununca açılan detay görünümü için ham girdi/çıktı (şeffaflık ikinci katman).
    var hamGirdi: String?
    var hamCikti: String?
    /// Bu çip bir dosya ürettiyse dosyanın yolu — çipe dokununca QuickLook önizlemesi açılır.
    var dosyaYolu: String?

    init(id: UUID = UUID(),
         ikon: String,
         metin: String,
         durum: AracDurumu = .calisiyor,
         hamGirdi: String? = nil,
         hamCikti: String? = nil,
         dosyaYolu: String? = nil) {
        self.id = id
        self.ikon = ikon
        self.metin = metin
        self.durum = durum
        self.hamGirdi = hamGirdi
        self.hamCikti = hamCikti
        self.dosyaYolu = dosyaYolu
    }

    /// VoiceOver için doğal cümle — spec §7.6 ("sirr takvimi okudu, yarın").
    var seslendirme: String {
        switch durum {
        case .calisiyor:      return "sirr \(metin)"
        case .okundu, .yazildi: return "sirr: \(metin)"
        case .izinGerekli:    return "\(metin). İzin vermek için dokun."
        case .basarisiz(let n): return "Başarısız: \(n)"
        }
    }
}
