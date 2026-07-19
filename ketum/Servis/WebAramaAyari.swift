//
//  WebAramaAyari.swift
//  ketum
//
//  Web araması ayarı (web-arama-spec §5.1): kullanıcının kendi SearXNG kök
//  adresi + aç/kapa. Token yoktur, dolayısıyla Keychain gerekmez; UserDefaults
//  yeterli. Görünüm katmanı aynı anahtarları @AppStorage ile bağlayabilir —
//  anahtar adları burada tek yerde tanımlıdır.
//
//  VARSAYILAN KAPALI (spec §2.1): uygulama hiçbir sunucu adresi ön tanımlı
//  getirmez. Yalnızca DEBUG derlemede geliştirici örneği okunabilir ve o da
//  ancak derleme bayrağıyla verilmişse. App Store sürümünde alan boştur.
//
//  Ön tanımlı bir sunucu adresi gömmek bir kez denendi ve GERİ ALINDI: tek
//  kullanıcılı kurulumda zararsız görünüyor, ama uygulama çok kullanıcılı
//  yayına çıktığında o adresin sahibi herkesin arama sorgularını taşıyan bir
//  veri işleyicisine dönüşür ve "sorgu senin sunucundan öteye gitmez" vaadi
//  ilk kullanıcıdan sonra anlamını yitirir. Sunucuyu kullanıcı girer.
//

import Foundation

enum WebAramaAyari {

    // MARK: - UserDefaults anahtarları
    // Görünüm katmanı: @AppStorage(WebAramaAyari.aktifAnahtar)

    static let aktifAnahtar = "sirr.webArama.aktif"
    static let kokAnahtar = "sirr.webArama.kokURL"

    // MARK: - Aç/kapa

    /// Arama açık mı. Sunucu tanımlıyken bile tek dokunuşla kapatılabilir.
    ///
    /// Adres geçersiz/boşsa bayrak açık olsa bile `false` döner: "açık ama
    /// çalışmayan" bir ara durum yoktur — araç ya profile girer ya girmez.
    static var aktifMi: Bool {
        get {
            guard UserDefaults.standard.bool(forKey: aktifAnahtar) else { return false }
            return kokURL != nil
        }
        set { UserDefaults.standard.set(newValue, forKey: aktifAnahtar) }
    }

    // MARK: - Kök adres

    /// Kullanıcının yazdığı ham metin (doğrulanmamış) — Ayarlar alanı bunu gösterir.
    static var kokHam: String {
        get {
            let kayitli = UserDefaults.standard.string(forKey: kokAnahtar) ?? ""
            #if DEBUG
            // Geliştirici kolaylığı; App Store derlemesinde bu dal yok.
            if kayitli.isEmpty {
                return ProcessInfo.processInfo.environment["SIRR_SEARX_URL"] ?? ""
            }
            #endif
            return kayitli
        }
        set { UserDefaults.standard.set(newValue, forKey: kokAnahtar) }
    }

    /// Doğrulanmış kök adres. Geçersizse `nil` — çağıran ağ koduna hiç girmez.
    static var kokURL: URL? { dogrula(kokHam) }

    /// Adres kuralı (mcp §3.1 ile aynı): yalnızca `https://`. Düz `http://`
    /// YALNIZCA yerel ağ adreslerinde kabul edilir — kullanıcı kendi ağındaki
    /// bir kutuya bağlanıyorsa TLS zorlamak gereksiz bir bariyer olur, ama
    /// internete düz metin sorgu çıkması kabul edilemez.
    static func dogrula(_ ham: String) -> URL? {
        let temiz = ham.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !temiz.isEmpty,
              let url = URL(string: temiz),
              let sema = url.scheme?.lowercased(),
              let konak = url.host?.lowercased(), !konak.isEmpty
        else { return nil }

        switch sema {
        case "https":
            return url
        case "http":
            return yerelAgMi(konak) ? url : nil
        default:
            return nil
        }
    }

    /// Yerel ağ konağı mı — loopback, `.local` (Bonjour) ve RFC1918 blokları.
    static func yerelAgMi(_ konak: String) -> Bool {
        if konak == "localhost" || konak == "::1" { return true }
        if konak.hasSuffix(".local") { return true }

        let parca = konak.split(separator: ".").map(String.init)
        guard parca.count == 4, let a = Int(parca[0]), let b = Int(parca[1]),
              parca.allSatisfy({ Int($0) != nil }) else { return false }

        switch a {
        case 127: return true            // 127.0.0.0/8
        case 10: return true             // 10.0.0.0/8
        case 192: return b == 168        // 192.168.0.0/16
        case 172: return (16...31).contains(b)  // 172.16.0.0/12
        case 169: return b == 254        // 169.254.0.0/16 (link-local)
        default: return false
        }
    }
}
