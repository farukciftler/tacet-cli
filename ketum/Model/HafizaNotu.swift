//
//  HafizaNotu.swift
//  ketum
//
//  Hafıza katmanının tek kalıcı kaydı (hafiza-spec §3). KullaniciBecerisi
//  deseninde: sınır sabitleri modelde durur, doğrulama `gecerliMi`de toplanır,
//  virgüllü ham metin `anahtarlar`da normalize edilir.
//
//  Metin sert sınırlıdır çünkü eşleşen notlar 4096 token penceresine enjekte
//  edilir; her karakter bütçedir. Tavana gelindiğinde OTOMATİK DÜŞÜRME YOKTUR —
//  sessizce silmek, "sessizce öğrenme yok" ilkesinin simetriğidir (spec §3).
//

import Foundation
import SwiftData

/// Notun türü. Ham String olarak saklanır (SwiftData enum uyumu için basit).
/// v1'de yalnızca panoda etiket; enjeksiyon önceliğinde rol almaz (spec §9/2).
enum HafizaTuru: String, Codable, CaseIterable {
    case kimlik
    case tercih
    case iliski
    case olgu
}

@Model
final class HafizaNotu {
    /// Metin üst sınırı — yaklaşık 40 token (spec §3).
    static let metinSiniri = 160
    /// Saklanabilecek en fazla not. Dolduğunda yeni ayıklama yapılmaz.
    static let toplamTavan = 50
    /// Anahtar üst sınırı — eşleşme taraması her mesajda döndüğü için sınırlı.
    static let anahtarSiniri = 8

    var id: UUID = UUID()
    /// Tek cümlelik olgu; kullanıcının kendi ifadesinden.
    var metin: String = ""
    /// `HafizaTuru` ham değeri. Bilinmeyen değer okunursa `.olgu` sayılır.
    private var turHam: String = HafizaTuru.olgu.rawValue
    /// Ham tetikleyici metni ("yemek, restoran, akşam"). Ayrıştırma `anahtarlar`da.
    var anahtarlarHam: String = ""
    /// Hangi sohbetten ayıklandı (şeffaflık; panoda gösterilir).
    /// Kaynak sohbet silinirse not kalır, bu alan boşa düşer (spec §7).
    var kaynakSohbetID: UUID?
    var olusturulma: Date = Date()
    /// Kullanıcı kapatabilir; kapalı not enjekte edilmez.
    var aktif: Bool = true

    init(metin: String = "",
         tur: HafizaTuru = .olgu,
         anahtarlarHam: String = "",
         kaynakSohbetID: UUID? = nil) {
        self.id = UUID()
        self.metin = metin
        self.turHam = tur.rawValue
        self.anahtarlarHam = anahtarlarHam
        self.kaynakSohbetID = kaynakSohbetID
        self.olusturulma = Date()
        self.aktif = true
    }

    var tur: HafizaTuru {
        get { HafizaTuru(rawValue: turHam) ?? .olgu }
        set { turHam = newValue.rawValue }
    }

    /// Normalize anahtarlar: küçük harf, boşluk kırpılmış, boşlar atılmış, sınırlı.
    var anahtarlar: [String] {
        anahtarlarHam
            .split(separator: ",")
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
            .filter { !$0.isEmpty }
            .prefix(Self.anahtarSiniri)
            .map { $0 }
    }

    /// Tekilleştirme anahtarı — küçük harf, boşluk kırpılmış metin (spec §4.3/4).
    var normalMetin: String {
        metin.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    }

    /// Kaydedilebilir mi — metin dolu ve sınır içinde, en az bir anahtar var.
    /// 10 karakter alt sınırı spec §4.3/1'deki filtrenin aynısıdır.
    var gecerliMi: Bool {
        let kirpik = metin.trimmingCharacters(in: .whitespacesAndNewlines)
        return kirpik.count >= 10
            && kirpik.count <= Self.metinSiniri
            && !anahtarlar.isEmpty
    }

    /// Panoda gösterilen altyazı: "yemek · restoran · akşam".
    var ozet: String { anahtarlar.joined(separator: " · ") }
}
