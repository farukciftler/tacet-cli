//
//  Baglanti.swift
//  ketum
//
//  Kullanıcının elle eklediği MCP sunucusu (mcp-baglanti-spec §5.1).
//
//  TOKEN BURADA DURMAZ. Bearer anahtarı Keychain'de
//  (`kSecAttrAccessibleWhenUnlockedThisDeviceOnly`) saklanır; modelde yalnızca
//  o kaydı bulmaya yarayan `anahtarRefi` durur. SwiftData mağazası şifreli
//  değildir ve iCloud'a girebilir — sır oraya yazılmaz.
//
//  Araç açıklamaları ham haliyle 4096 pencereye sığmaz (100–500 token/araç);
//  ekleme anında cihaz-üstü modele özetletilip `aracOzetleri`nde önbelleklenir
//  (spec §5.3). Oturuma giren tanım bu özettir.
//

import Foundation
import SwiftData

/// Bu bağlantıya cihaz verisi gönderilebilir mi (spec §3.1).
/// "her zaman izin ver" v1'de BİLİNÇLİ OLARAK YOKTUR — onay kapısı devre dışı
/// bırakılabilir olsaydı §3.3'teki savunma anlamını yitirirdi.
enum CihazVerisiAyari: String, Codable, CaseIterable {
    /// Varsayılan: kirli oturumda çağrı hiç yapılmaz.
    case hicbirZaman
    /// Kirli oturumda her çağrı onay sayfasına düşer.
    case herSeferindeSor
}

/// Sunucudan içe aktarılmış tek araç: adı + sıkıştırılmış açıklaması.
struct AracOzeti: Codable, Hashable, Identifiable {
    var id: String { ad }
    /// Uzak araç adı (MCP `tools/list` içindeki ad).
    var ad: String
    /// 1–2 satıra indirilmiş açıklama — oturuma giren metin budur.
    var ozet: String
    /// Şema düzleştirilemediği için atlandıysa true; detayda
    /// "desteklenmiyor" diye listelenir, sessizce yutulmaz (spec §5.2).
    var desteklenmiyor: Bool = false

    init(ad: String, ozet: String, desteklenmiyor: Bool = false) {
        self.ad = ad
        self.ozet = ozet
        self.desteklenmiyor = desteklenmiyor
    }
}

@Model
final class Baglanti {
    var id: UUID = UUID()
    /// Serbest metin, ör. "ev sunucusu". Çip metninin başına bu ad gelir.
    var ad: String = ""
    /// Streamable HTTP endpoint'i. Düz `http://` yalnızca yerel ağda kabul edilir
    /// (doğrulama servis katmanında).
    var urlHam: String = ""
    /// `CihazVerisiAyari` ham değeri. Bilinmeyen değer okunursa en kısıtlı
    /// seçenek (`hicbirZaman`) uygulanır.
    private var cihazVerisiHam: String = CihazVerisiAyari.hicbirZaman.rawValue
    /// Keychain hesap anahtarı — TOKEN'IN KENDİSİ DEĞİL, yalnızca referans.
    /// Anahtar girilmediyse nil. Bağlantı silinince bu kayıt Keychain'den kaldırılır.
    var anahtarRefi: String?
    /// Kodlanmış [AracOzeti]. Düz Data — `Mesaj.izlerVeri` deseni, taşınabilir.
    private var aracOzetleriVeri: Data?
    var olusturulma: Date = Date()
    /// En son ne zaman bir araç çağrıldı; listede gösterilir. Hiç kullanılmadıysa nil.
    var sonKullanim: Date?

    init(ad: String = "",
         urlHam: String = "",
         cihazVerisi: CihazVerisiAyari = .hicbirZaman,
         anahtarRefi: String? = nil,
         aracOzetleri: [AracOzeti] = []) {
        self.id = UUID()
        self.ad = ad
        self.urlHam = urlHam
        self.cihazVerisiHam = cihazVerisi.rawValue
        self.anahtarRefi = anahtarRefi
        self.aracOzetleriVeri = try? JSONEncoder().encode(aracOzetleri)
        self.olusturulma = Date()
    }

    var cihazVerisi: CihazVerisiAyari {
        get { CihazVerisiAyari(rawValue: cihazVerisiHam) ?? .hicbirZaman }
        set { cihazVerisiHam = newValue.rawValue }
    }

    /// Önbelleklenmiş araç tanımları. Sunucu listesi değişirse tazelenir.
    var aracOzetleri: [AracOzeti] {
        get {
            guard let aracOzetleriVeri else { return [] }
            return (try? JSONDecoder().decode([AracOzeti].self, from: aracOzetleriVeri)) ?? []
        }
        set { aracOzetleriVeri = try? JSONEncoder().encode(newValue) }
    }

    /// Modele sunulabilecek araçlar — düzleştirilemeyenler dışarıda.
    var kullanilabilirAraclar: [AracOzeti] {
        aracOzetleri.filter { !$0.desteklenmiyor }
    }

    var url: URL? { URL(string: urlHam) }

    /// Kaydedilebilir mi — ad dolu ve URL ayrıştırılabilir bir http(s) adresi.
    var gecerliMi: Bool {
        guard !ad.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              let url, let sema = url.scheme?.lowercased(),
              sema == "https" || sema == "http",
              url.host != nil else { return false }
        return true
    }
}
