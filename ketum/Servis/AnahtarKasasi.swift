//
//  AnahtarKasasi.swift
//  ketum
//
//  Bearer anahtarlarının tek saklama yeri (mcp-baglanti-spec §5.8).
//
//  `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`: kayıt iCloud Anahtar Zinciri'ne
//  ve cihaz yedeğine GİRMEZ, yalnızca bu cihazda ve yalnızca cihaz kilidi açıkken
//  okunur. SwiftData mağazası şifreli değil ve yedeğe girebilir; bu yüzden
//  `Baglanti` yalnızca buradaki kaydı bulmaya yarayan referansı tutar.
//
//  Token arayüzde bir daha GÖSTERİLMEZ: `oku` yalnızca ağ isteğine Authorization
//  başlığı koyacak olan `MCPIstemcisi` için vardır, görünüm katmanı çağırmaz.
//

import Foundation
import Security

enum AnahtarKasasi {

    /// Keychain servis alanı — uygulamanın diğer kayıtlarıyla çakışmasın.
    private static let servis = "com.sirr.baglanti"

    /// Yeni bir bağlantı için benzersiz referans üretir. Bu değer `Baglanti`de
    /// (yani SwiftData'da) durur; sırrın kendisi değildir.
    static func yeniRef() -> String { "baglanti-" + UUID().uuidString }

    /// Anahtarı yazar (varsa üzerine yazar). Boş metin verilirse kayıt silinir —
    /// "anahtarı temizle" ayrı bir çağrı gerektirmesin.
    @discardableResult
    static func yaz(_ anahtar: String, ref: String) -> Bool {
        let temiz = anahtar.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !temiz.isEmpty else { return sil(ref: ref) }
        guard let veri = temiz.data(using: .utf8) else { return false }

        // Önce mevcut kaydı düşür: SecItemUpdate'in erişilebilirlik sınıfını
        // güncellemediği kenar durumlar var; sil-yaz tek davranış bırakır.
        sil(ref: ref)

        let sorgu: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: servis,
            kSecAttrAccount as String: ref,
            kSecValueData as String: veri,
            kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
        ]
        return SecItemAdd(sorgu as CFDictionary, nil) == errSecSuccess
    }

    /// Anahtarı okur. Kayıt yoksa ya da cihaz kilitliyse nil.
    /// YALNIZCA ağ katmanı çağırır; arayüze geri dönmez.
    static func oku(ref: String) -> String? {
        let sorgu: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: servis,
            kSecAttrAccount as String: ref,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var sonuc: CFTypeRef?
        guard SecItemCopyMatching(sorgu as CFDictionary, &sonuc) == errSecSuccess,
              let veri = sonuc as? Data else { return nil }
        return String(data: veri, encoding: .utf8)
    }

    /// Kaydın var olup olmadığını sırrı okumadan söyler — arayüz "anahtar kayıtlı"
    /// bilgisini bununla gösterir, değeri asla görmez.
    static func varMi(ref: String) -> Bool {
        let sorgu: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: servis,
            kSecAttrAccount as String: ref,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        return SecItemCopyMatching(sorgu as CFDictionary, nil) == errSecSuccess
    }

    /// Bağlantı silinince çağrılır. Kayıt zaten yoksa da başarı sayılır.
    @discardableResult
    static func sil(ref: String) -> Bool {
        let sorgu: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: servis,
            kSecAttrAccount as String: ref,
        ]
        let durum = SecItemDelete(sorgu as CFDictionary)
        return durum == errSecSuccess || durum == errSecItemNotFound
    }

    /// Bu servis alanındaki tüm referanslar. Sırları OKUMAZ, yalnızca adları
    /// getirir (`kSecReturnData` yok) — sızıntı yüzeyi açmaz.
    static func refler() -> [String] {
        let sorgu: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: servis,
            kSecReturnAttributes as String: true,
            kSecMatchLimit as String: kSecMatchLimitAll,
        ]
        var sonuc: CFTypeRef?
        guard SecItemCopyMatching(sorgu as CFDictionary, &sonuc) == errSecSuccess,
              let kayitlar = sonuc as? [[String: Any]] else { return [] }
        return kayitlar.compactMap { $0[kSecAttrAccount as String] as? String }
    }

    /// Sahibi kalmamış kayıtları düşürür.
    ///
    /// Keychain kaydı SwiftData mağazasından BAĞIMSIZ yaşar: uygulama silinip
    /// yeniden kurulsa, mağaza taşınsa ya da bir kayıt geçmişte yarım yazılmış
    /// olsa token orada kalır — hiçbir `Baglanti` onu işaret etmediği için ne
    /// okunur ne silinir. Açılışta yaşayan bağlantıların referanslarıyla
    /// çağrılır; listede olmayan her kayıt silinir.
    /// - Returns: silinen kayıt sayısı.
    @discardableResult
    static func yetimleriTemizle(gecerli: Set<String>) -> Int {
        var silinen = 0
        for ref in refler() where !gecerli.contains(ref) {
            if sil(ref: ref) { silinen += 1 }
        }
        return silinen
    }
}
