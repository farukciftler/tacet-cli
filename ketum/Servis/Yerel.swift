//
//  Yerel.swift
//  ketum
//
//  Kod içinde üretilen kullanıcı-görünür metinlerin (araç çipleri, durum ve hata
//  mesajları, tarih ayraçları) tek yerelleştirme noktası. Cihaz diline göre
//  Localizable.xcstrings'ten çözülür (8 dil + tr). Araçlar buradan çeker.
//

import Foundation

enum Yerel {
    // MARK: - Çip "çalışıyor" metinleri
    static var takvimBakiliyor: String { String(localized: "Takvime bakılıyor…") }
    static var etkinlikEkleniyor: String { String(localized: "Etkinlik ekleniyor…") }
    static var hatirlaticiKuruluyor: String { String(localized: "Hatırlatıcı kuruluyor…") }
    static var kisiAraniyor: String { String(localized: "Kişilerde aranıyor…") }
    static var notAraniyor: String { String(localized: "Notlarda aranıyor…") }
    static var hesaplaniyor: String { String(localized: "Hesaplanıyor…") }
    static var belgeAraniyor: String { String(localized: "Belge aranıyor…") }
    static func belgeOlusturuluyor(_ b: String) -> String { String(localized: "\(b) oluşturuluyor…") }
    static func belgeOkunuyor(_ b: String) -> String { String(localized: "\(b) okunuyor…") }
    static func belgeDuzenleniyor(_ b: String) -> String { String(localized: "\(b) düzenleniyor…") }

    // MARK: - Çip "tamamlandı" metinleri
    static func takvimOkundu(_ n: Int) -> String { String(localized: "Takvim okundu · \(n) etkinlik") }
    static var takvimOkunduBos: String { String(localized: "Takvim okundu · boş") }
    static var etkinlikEklendi: String { String(localized: "Etkinlik eklendi") }
    static var takvimIzni: String { String(localized: "Takvim izni gerekli") }
    static func hatirlaticiKuruldu(saat: String?) -> String {
        if let saat { return String(localized: "Hatırlatıcı kuruldu · \(saat)") }
        return String(localized: "Hatırlatıcı kuruldu")
    }
    static var hatirlaticiIzni: String { String(localized: "Hatırlatıcı izni gerekli") }
    static var kisiArandi: String { String(localized: "Kişilerde arandı") }
    static var kisiArandiYok: String { String(localized: "Kişilerde arandı · sonuç yok") }
    static var kisiIzni: String { String(localized: "Kişiler izni gerekli") }
    static func notArandi(_ n: Int) -> String { String(localized: "Notlarda arandı · \(n) sonuç") }
    static var notArandiYok: String { String(localized: "Notlarda arandı · sonuç yok") }
    static var hesaplandi: String { String(localized: "Hesaplandı") }
    static func belgeOlusturuldu(_ b: String, _ ad: String) -> String { String(localized: "\(b) oluşturuldu · \(ad)") }
    static func belgeDuzenlendi(_ b: String, _ ad: String) -> String { String(localized: "\(b) düzenlendi · \(ad)") }
    static func belgeOkundu(_ b: String, _ ad: String) -> String { String(localized: "\(b) okundu · \(ad)") }
    static var duzenlenecekYok: String { String(localized: "Düzenlenecek belge yok") }
    static var paylasilanYok: String { String(localized: "Paylaşılan belge yok") }

    // MARK: - Model durum/hata mesajları
    static var modelHazirDegil: String { String(localized: "Model bu cihazda hazır değil.") }
    static var oncekiBitiyor: String { String(localized: "Bir saniye, önceki yanıtı bitiriyorum.") }
    static var sinirDisi: String { String(localized: "Bunu yapamam; sınırlarımın dışında.") }
    static var dilDesteklenmiyor: String { String(localized: "Bu dili şu an tam desteklemiyorum.") }
    static var tekrarDene: String { String(localized: "Şu an bunu yapamadım. Bir daha sorar mısın?") }

    // MARK: - Nöbet (zamanlanmış ajan)
    static var nobetKuruluyor: String { String(localized: "Nöbet kuruluyor…") }
    static var nobetKuruldu: String { String(localized: "Nöbet kuruldu · her gün") }
    static var nobetHata: String { String(localized: "Nöbet kurulamadı") }

    // MARK: - Tarih ayraçları
    static var bugun: String { String(localized: "BUGÜN") }
    static var dunUst: String { String(localized: "DÜN") }
    static var dun: String { String(localized: "Dün") }
}
