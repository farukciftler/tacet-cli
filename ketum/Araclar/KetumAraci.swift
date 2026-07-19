//
//  KetumAraci.swift
//  ketum
//
//  Araç kataloğu sözleşmesi (spec §7.3). Tüm araçlar FoundationModels `Tool`
//  protokolüyle, argümanları @Generable/@Guide ile tip güvenli tanımlanır.
//  Model serbest metin üretip parse ettirmez. Hiçbir araç ağ çağrısı yapmaz.
//
//  Her araç bir `AracRaporlayici` taşır ve çalışırken/bittiğinde çip günceller.
//  Çip metni araçta üretilir — model çip metnini halüsine edemez (spec §7.4).
//

import Foundation
import FoundationModels

/// sirr aracı: FoundationModels Tool + çip raporlama.
protocol KetumAraci: Tool {
    /// Çiplerin düşürüleceği yürütücü. Zayıf referans — döngü olmaz.
    var raporlayici: AracRaporlayici? { get }
}

extension KetumAraci {
    /// Bir aracın işini çip yaşam döngüsüne sarar: başlat → iş → son durum.
    /// - `ikon`: SF Symbol. `calisiyorMetni`: spinner yanındaki metin.
    /// - `is`: gerçek işi yapar; ekranda gösterilecek son çip metnini, durumu
    ///   ve isteğe bağlı ham çıktıyı döndürür; modele dönecek String'i de döndürür.
    func cipliCalis(
        ikon: String,
        calisiyorMetni: String,
        hamGirdi: String? = nil,
        is islem: () async throws -> AracSonucu
    ) async -> String {
        let id = await raporlayici?.baslat(ikon: ikon, metin: calisiyorMetni)
        do {
            let s = try await islem()
            if let id {
                await raporlayici?.guncelle(id, durum: s.durum, metin: s.cipMetni,
                                            hamGirdi: hamGirdi, hamCikti: s.hamCikti,
                                            dosyaYolu: s.dosyaYolu)
            }
            return s.modeleDonen
        } catch {
            let neden = Self.kisaHata(error)
            if let id {
                await raporlayici?.guncelle(id, durum: .basarisiz(neden), metin: nil,
                                            hamGirdi: hamGirdi, hamCikti: neden, dosyaYolu: nil)
            }
            // Model akışı kesilmesin diye hata metni String olarak döner.
            return "Araç başarısız: \(neden)"
        }
    }

    static func kisaHata(_ error: Error) -> String {
        let m = (error as NSError).localizedDescription
        return m.count > 60 ? String(m.prefix(60)) + "…" : m
    }
}

/// Bir aracın çalışmasının sonucu: çipe ne yazılacağı + modele ne döneceği.
struct AracSonucu {
    /// Çipte gösterilecek son metin (araç üretir; ~5 kelime + isteğe bağlı · detay).
    var cipMetni: String
    /// Çip son durumu — okuma (`.okundu`), yazma onay imiyle (`.yazildi`).
    var durum: AracDurumu
    /// Modele dönecek metin — kısa/özet; ham toplu veri bağlama dökülmez (spec §7.2).
    var modeleDonen: String
    /// Çip detay görünümü için ham çıktı (şeffaflık ikinci katman).
    var hamCikti: String?
    /// Bir dosya üretildiyse yolu — çipe dokununca önizleme açılır.
    var dosyaYolu: String?

    init(cipMetni: String, durum: AracDurumu, modeleDonen: String, hamCikti: String? = nil, dosyaYolu: String? = nil) {
        self.cipMetni = cipMetni
        self.durum = durum
        self.modeleDonen = modeleDonen
        self.hamCikti = hamCikti
        self.dosyaYolu = dosyaYolu
    }
}
