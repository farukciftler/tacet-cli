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
    static var gunSayiliyor: String { String(localized: "Gün sayılıyor…") }
    static var gunSayildi: String { String(localized: "Gün sayıldı") }
    static var tarihAnlasilmadi: String { String(localized: "Tarih anlaşılmadı") }
    static func belgeOlusturuldu(_ b: String, _ ad: String) -> String { String(localized: "\(b) oluşturuldu · \(ad)") }
    static func belgeDuzenlendi(_ b: String, _ ad: String) -> String { String(localized: "\(b) düzenlendi · \(ad)") }
    static func belgeOkundu(_ b: String, _ ad: String) -> String { String(localized: "\(b) okundu · \(ad)") }
    static var duzenlenecekYok: String { String(localized: "Düzenlenecek belge yok") }
    static var paylasilanYok: String { String(localized: "Paylaşılan belge yok") }

    // MARK: - Model durum/hata mesajları
    // Her biri "ne oldu + ne yapmalı" söyler; birbiriyle çelişmemesi için
    // availability nedenine göre AYRILMIŞTIR (hazırlanıyor ≠ hiç çalışmayacak).
    /// modelNotReady — model indiriliyor; bu geçici, birazdan çalışacak.
    static var modelHazirlaniyor: String {
        String(localized: "Model hâlâ indiriliyor. Birkaç dakika sonra tekrar dene.")
    }
    /// appleIntelligenceNotEnabled — kullanıcı düzeltebilir, yolu tarif et.
    static var appleIntelligenceKapali: String {
        String(localized: "Apple Intelligence kapalı. Ayarlar > Apple Intelligence ve Siri'den açtıktan sonra tekrar dene.")
    }
    /// deviceNotEligible — kalıcı; boşuna beklememesi için açıkça söyle.
    static var cihazUygunDegil: String {
        String(localized: "Bu cihaz cihaz-üstü modeli çalıştıramıyor; sirr burada yanıt üretemiyor.")
    }
    /// Yan etki oluştuktan sonraki hata: retry yapılmadı, çünkü tekrarlardı.
    static var yazmaSonrasiHata: String {
        String(localized: "Yanıtı tamamlayamadım ama bu sırada yaptığım değişiklik kaydedildi. Aynı şeyi iki kez yapmamak için yeniden denemedim; kontrol edip gerekirse tekrar sor.")
    }
    static var oncekiBitiyor: String { String(localized: "Bir saniye, önceki yanıtı bitiriyorum.") }
    static var sinirDisi: String { String(localized: "Bunu yapamam; sınırlarımın dışında.") }
    static var dilDesteklenmiyor: String { String(localized: "Bu dili şu an tam desteklemiyorum.") }
    static var tekrarDene: String { String(localized: "Şu an bunu yapamadım. Bir daha sorar mısın?") }

    // MARK: - Hata sınıfına göre düşüş cümleleri
    // Tek bir "Şu an bunu yapamadım" üç ayrı arızayı örtüyordu: kullanıcı ne
    // olduğunu da, kendi tarafında yapabileceği bir şey olup olmadığını da
    // bilemiyordu. Her cümle NE olduğunu söyler ama İÇ AYRINTI vermez —
    // hata metni, satır numarası, araç adı, model adı hiçbirinde geçmez.
    /// Geriye yalnız yarım araç çağrısı kaldı: model cümleyi hiç kurmadı.
    static var yanitToparlanamadi: String {
        String(localized: "Cevabı toparlayamadım — sana yarım bir şey göstermek istemem. Bir daha sorar mısın?")
    }
    /// Turda araç düştü ve model üstüne bir şey söylemedi. Çip zaten NE
    /// düştüğünü gösteriyor; cümle uydurmadığımızı söyler, ayrıntıyı çipe bırakır.
    static var aracDustuYanit: String {
        String(localized: "Denedim ama bu adım yürümedi; sonucu uydurmaktansa söylemeyi tercih ederim. Yukarıdaki adıma dokunursan ne olduğunu görebilirsin.")
    }
    /// Bağlam taştı: kullanıcının yapabileceği somut bir şey VAR, onu söyle.
    static var konusmaUzadi: String {
        String(localized: "Konuşma benim için fazla uzadı. Yeni bir sohbet açıp tekrar sorarsan bunu yapabilirim.")
    }

    // MARK: - Kod çalıştırma (kod-spec §5)
    static var kodCalisiyor: String { String(localized: "Kod çalıştırılıyor…") }
    static func kodCalisti(_ ms: Int) -> String { String(localized: "Kod çalıştırıldı · \(ms) ms") }
    static var kodYenidenDeneniyor: String { String(localized: "Hata · yeniden deneniyor") }
    static var kodCalistirilamadi: String { String(localized: "Kod çalıştırılamadı") }
    static var kodZamanAsimi: String { String(localized: "Kod zaman aşımına uğradı") }
    static var kodZamanAsimiNeden: String { String(localized: "Kod 3 saniyede tamamlanmadı.") }
    static var kodDenemeSiniri: String { String(localized: "Deneme sınırına ulaşıldı") }
    static var kodIkiDeneme: String { String(localized: "Kod iki denemede çalışmadı.") }
    static var kodCiktiKirpildi: String { String(localized: "… çıktı 10.000 karakterde kırpıldı.") }

    // MARK: - Sayfa doğrulama (kod-spec §4.3)
    static var sayfaDogrulanamadi: String { String(localized: "Sayfa doğrulanamadı") }
    static var sayfaYuklenmedi: String { String(localized: "Sayfa 3 saniye içinde yüklenmedi") }
    static var sayfaDisIstek: String { String(localized: "Sayfa dosya dışı bir adrese istek yaptı") }
    static func sayfaYuklenemedi(_ hata: String) -> String { String(localized: "Sayfa yüklenemedi: \(hata)") }
    static func sayfaBetikHatasi(_ mesaj: String) -> String { String(localized: "Sayfada betik hatası: \(mesaj)") }

    // MARK: - Nöbet (zamanlanmış ajan)
    static var nobetKuruluyor: String { String(localized: "Nöbet kuruluyor…") }
    static var nobetKuruldu: String { String(localized: "Nöbet kuruldu · her gün") }
    static var nobetHata: String { String(localized: "Nöbet kurulamadı") }

    // MARK: - Tarih ayraçları
    static var bugun: String { String(localized: "BUGÜN") }
    static var dunUst: String { String(localized: "DÜN") }
    static var dun: String { String(localized: "Dün") }
}
