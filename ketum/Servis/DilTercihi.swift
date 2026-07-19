//
//  DilTercihi.swift
//  ketum
//
//  Dil tercihleri (yanıt dili + arayüz dili). İkisi ayrıdır: kullanıcı arayüzü
//  Türkçe tutup yanıtları İngilizce isteyebilir. Tercihler UserDefaults'ta
//  kalıcıdır; bu bir servis olduğu için @AppStorage değil doğrudan UserDefaults.
//

import Foundation
import Observation

@MainActor
@Observable
final class DilTercihi {

    static let paylasilan = DilTercihi()

    private static let yanitAnahtar = "sirr.yanitDili"
    private static let arayuzAnahtar = "sirr.arayuzDili"

    /// "" = otomatik (yazılan dili sapta). Aksi halde BCP47 kodu, ör. "en".
    var yanitDili: String {
        didSet {
            guard yanitDili != oldValue else { return }
            UserDefaults.standard.set(yanitDili, forKey: Self.yanitAnahtar)
        }
    }

    /// "" = cihaz dili. Aksi halde BCP47 kodu.
    var arayuzDili: String {
        didSet {
            guard arayuzDili != oldValue else { return }
            UserDefaults.standard.set(arayuzDili, forKey: Self.arayuzAnahtar)
            uygulaArayuzDili()
        }
    }

    /// Süreç başlarken FİİLEN yürürlükte olan arayüz dili. Bundle'ın yerelleştirmeleri
    /// bir kez çözülür; ekranda gördüğümüz dil bu değerdir, seçim değişse de değişmez.
    private let baslangictakiArayuzDili: String

    /// Arayüz dili değiştikten sonra yeniden başlatma gerektiğini ekranda sade bir
    /// cümleyle söylemek için — sessizce yarım uygulanmış bir dil vaat etmeyiz.
    /// Kalıcı bayrak değil, türetilmiş değer: kullanıcı ilk dile geri dönerse
    /// bekleyen bir değişiklik kalmaz ve not kendiliğinden kaybolur.
    var arayuzYenidenBaslatmaBekliyor: Bool {
        arayuzDili != baslangictakiArayuzDili
    }

    /// Desteklenen 9 dil; her adı kendi dilinde yazılır.
    static let secenekler: [(kod: String, ad: String)] = [
        ("tr", "Türkçe"),
        ("en", "English"),
        ("de", "Deutsch"),
        ("es", "Español"),
        ("fr", "Français"),
        ("ja", "日本語"),
        ("ko", "한국어"),
        ("pt-BR", "Português (Brasil)"),
        ("zh-Hans", "简体中文"),
    ]

    /// Arayüz dili seçiliyse o Locale, değilse nil (cihaz dili geçerli).
    var arayuzLocale: Locale? {
        arayuzDili.isEmpty ? nil : Locale(identifier: arayuzDili)
    }

    private init() {
        let d = UserDefaults.standard
        yanitDili = d.string(forKey: Self.yanitAnahtar) ?? ""
        let kayitliArayuz = d.string(forKey: Self.arayuzAnahtar) ?? ""
        arayuzDili = kayitliArayuz
        baslangictakiArayuzDili = kayitliArayuz
        uygulaArayuzDili()
    }

    /// AppleLanguages, bundle'ın hangi .lproj'u seçeceğini belirler; sonraki
    /// başlatmada arayüzün seçilen dilde açılmasını bu anahtar sağlar.
    private func uygulaArayuzDili() {
        let d = UserDefaults.standard
        if arayuzDili.isEmpty {
            d.removeObject(forKey: "AppleLanguages")
        } else {
            d.set([arayuzDili], forKey: "AppleLanguages")
        }
    }
}
