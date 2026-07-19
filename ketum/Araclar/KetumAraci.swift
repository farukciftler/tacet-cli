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
    /// `cipliCalis`in ilerleme bildirebilen sürümü: iş sürerken çip metnini
    /// değiştirir. Uzun süren araçlar (web araması sayfa çekerken) için.
    ///
    /// Seyir ilkesiyle tutarlı: gösterilen her adım kodda GERÇEKTEN olan bir
    /// olaydır — hangi siteye bakıldığı uydurma değil, o an indirilen adres.
    /// Dramatize yok: "araştırıyor" gibi süslü fiil değil, alan adının kendisi.
    func cipliCalis(
        ikon: String,
        calisiyorMetni: String,
        hamGirdi: String? = nil,
        ilerlemeli islem: (@Sendable (String) async -> Void) async throws -> AracSonucu
    ) async -> String {
        let id = await raporlayici?.baslat(ikon: ikon, metin: calisiyorMetni)
        let raporlayici = self.raporlayici
        let ilerle: @Sendable (String) async -> Void = { metin in
            guard let id else { return }
            await raporlayici?.guncelle(id, durum: .calisiyor, metin: metin,
                                        hamGirdi: nil, hamCikti: nil, dosyaYolu: nil)
        }
        do {
            let s = try await islem(ilerle)
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
            return "tool_failed: the action could not be completed. Tell the user briefly, in their own language, that this step did not work."
        }
    }

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
            // Modele giden metin İNGİLİZCE ve sabittir: model bunu yanıtına
            // olduğu gibi yansıtsa bile ne Türkçe sızar ne de ham hata kodu.
            return "tool_failed: the action could not be completed. Tell the user briefly, in their own language, that this step did not work."
        }
    }

    /// Çipe yazılacak hata metni — kullanıcı okur, o yüzden yerelleştirilmiş ve
    /// anlaşılır olmalı. Ham `NSError.localizedDescription` ("EKErrorDomain error 1.")
    /// asla ekrana çıkmaz; tanınan alanlar insan cümlesine çevrilir.
    static func kisaHata(_ error: Error) -> String {
        // Kendi araç hatalarımız zaten String Catalog'dan gelen cümlelerdir.
        if let yerel = error as? LocalizedError,
           let metin = yerel.errorDescription, !metin.isEmpty {
            return metin
        }
        let ns = error as NSError
        switch (ns.domain, ns.code) {
        case (NSCocoaErrorDomain, NSFileWriteOutOfSpaceError):
            return String(localized: "Cihazda yer kalmadı.")
        case (NSCocoaErrorDomain, NSFileNoSuchFileError),
             (NSCocoaErrorDomain, NSFileReadNoSuchFileError):
            return String(localized: "Dosya bulunamadı.")
        case (NSCocoaErrorDomain, _), (NSPOSIXErrorDomain, _), (NSOSStatusErrorDomain, _):
            return String(localized: "Dosya işlemi tamamlanamadı.")
        case ("EKErrorDomain", _):
            return String(localized: "Takvim bu işlemi kabul etmedi.")
        case ("CNErrorDomain", _):
            return String(localized: "Kişilere şu an ulaşılamadı.")
        default:
            return String(localized: "Bu adım tamamlanamadı.")
        }
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
