//
//  OtoTest.swift
//  ketum
//
//  DEBUG kendi kendine testi: gerçek motor kodunu (model olmadan) büyük veriyle
//  çalıştırır. Bu, büyük dosya kanalının ta kendisidir — uygulama katmanı tabloyu
//  doğrudan motora verir, model devrede değildir. "--ototest" argümanıyla açılır.
//

#if DEBUG
import Foundation

enum OtoTest {
    @MainActor
    static func calistir() {
        var kayit = ["=== KETUM OTOTEST ==="]
        let klasor = BelgeBaglami.testKlasoru()

        // Büyük tablo: 500 satır, sayısal "Tutar" sütunu (Excel'de =SUM tetikler).
        var satirlar: [Satir] = []
        var beklenenToplam = 0
        for i in 1...500 {
            let tutar = i * 7
            beklenenToplam += tutar
            satirlar.append(Satir(hucreler: [
                "2026-07-\(String(format: "%02d", (i % 28) + 1))",
                "Kalem \(i)",
                "\(tutar)",
            ]))
        }
        let tablo = Tablo(basliklar: ["Tarih", "Açıklama", "Tutar"], satirlar: satirlar)
        kayit.append("Tablo: \(tablo.satirlar.count) satır, beklenen Tutar toplamı=\(beklenenToplam)")

        let isler: [(BelgeBicimi, String, Bool)] = [
            (.xlsx, "ototest-excel", true),
            (.docx, "ototest-word", false),
            (.pdf,  "ototest-pdf",  false),
            (.md,   "ototest-markdown", false),
        ]

        for (bicim, ad, tabloMu) in isler {
            do {
                let motor = BelgeMotorlari.motor(bicim)
                let govde = tabloMu ? nil : "# Oto Test Raporu\n\n" + tablo.markdown
                let url = try motor.yaz(dosyaAdi: ad,
                                        baslik: "Oto Test",
                                        govde: govde,
                                        tablo: tabloMu ? tablo : nil,
                                        klasor: klasor)
                let oznitelikler = try? FileManager.default.attributesOfItem(atPath: url.path)
                let boyut = (oznitelikler?[.size] as? Int) ?? 0
                // Geri oku (round-trip).
                let geri = try motor.oku(url: url)
                let satirSayi = geri.tablo?.satirlar.count ?? geri.metin.split(separator: "\n").count
                kayit.append("\(bicim.uzanti): YAZILDI \(url.lastPathComponent) \(boyut)B · OKUNDU satır/blok=\(satirSayi)")
            } catch {
                kayit.append("\(bicim.uzanti): HATA \(error)")
            }
        }
        // Belge bağlamı: üretilen dosya sonradan okunabilir/düzenlenebilir olmalı.
        // "Excel yap" → "onu tablo olarak göster" akışının dayandığı davranış.
        kayit.append("--- BELGE BAĞLAMI ---")
        let baglam = BelgeBaglami()
        kayit.append("Başlangıçta çalışılabilir belge: \(baglam.calisilabilirBelge == nil ? "yok ✓" : "var ✗")")
        let uretilen = klasor.appendingPathComponent("ototest-excel.xlsx")
        baglam.ciktiEklendi(uretilen)
        let devam = baglam.calisilabilirBelge
        kayit.append("Üretimden sonra: \(devam?.ad ?? "yok") \(devam?.ad == "ototest-excel.xlsx" ? "✓" : "✗")")
        kayit.append("  biçim: \(devam?.bicim.etiket ?? "-") \(devam?.bicim == .xlsx ? "✓" : "✗")")
        // Kullanıcının eklediği belge, üretilenin önüne geçmeli.
        let ekliURL = klasor.appendingPathComponent("ototest-pdf.pdf")
        baglam.belgeEkle(url: ekliURL)
        kayit.append("Ekli belge öncelikli: \(baglam.calisilabilirBelge?.ad ?? "yok") "
                     + "\(baglam.calisilabilirBelge?.ad == "ototest-pdf.pdf" ? "✓" : "✗")")
        // Ekli kaldırılınca üretilene geri düşmeli.
        baglam.belgeKaldir()
        kayit.append("Ekli kaldırılınca üretilene döner: "
                     + "\(baglam.calisilabilirBelge?.ad == "ototest-excel.xlsx" ? "✓" : "✗")")
        // Yeni sohbet: bağlam tamamen temizlenmeli, yoksa eski dosya sızar.
        baglam.uretimiUnut()
        kayit.append("Yeni sohbette temizlendi: \(baglam.calisilabilirBelge == nil ? "✓" : "✗")")

        // Modele dönen tablo GEÇERLİ markdown olmalı — sohbette tablo çizilmesi
        // Tablo.markdownDan'ın bunu ayrıştırabilmesine bağlı.
        kayit.append("--- MODELE DÖNEN TABLO ---")
        let mdTam = tablo.markdownKirpik(enFazlaSatir: 30)
        let geriAyristirilan = Tablo.markdownDan(mdTam)
        kayit.append("Markdown geri ayrıştırıldı: \(geriAyristirilan != nil ? "✓" : "✗")")
        kayit.append("  satır: \(geriAyristirilan?.satirlar.count ?? 0)/30 "
                     + "\(geriAyristirilan?.satirlar.count == 30 ? "✓" : "✗")")
        kayit.append("  başlık: \(geriAyristirilan?.basliklar.joined(separator: ",") ?? "-") "
                     + "\(geriAyristirilan?.basliklar == tablo.basliklar ? "✓" : "✗")")
        kayit.append("  kırpma notu var: \(mdTam.contains("+470 satır daha") ? "✓" : "✗")")
        // Kırpma gerekmeyen küçük tablo olduğu gibi dönmeli.
        let kucuk = Tablo(basliklar: ["Gün", "Yemek"],
                          satirlar: [Satir(hucreler: ["Pazartesi", "Mercimek"])])
        kayit.append("Küçük tablo kırpılmadı: "
                     + "\(kucuk.markdownKirpik(enFazlaSatir: 30) == kucuk.markdown ? "✓" : "✗")")

        // Beceri (SKILL.md) katmanı testi.
        kayit.append("--- BECERİLER (SKILL.md) ---")
        kayit.append("Bundle'dan yüklenen beceri: \(BeceriDeposu.paket.count)")
        let denemeler = [
            ("bir excel yap haftalık", "belge-olustur"),
            ("yarın ne var takvimimde", "takvim"),
            ("beni 18'de aramamı hatırlat", "hatirlatici"),
            ("125*8 kaç eder", "hesap"),
            ("bu belgeyi özetle", "belge-oku"),
            ("cuma satırını düzenle", "belge-duzenle"),
            ("nasılsın", "yok"),
            // Özgüllük: "tablo olarak" (belge-oku), "tablo" (belge-olustur) genelini yenmeli.
            ("bunu tablo olarak göster", "belge-oku"),
            // Genel kelime hâlâ doğru beceriye gitmeli — özgüllük kuralı bunu bozmamalı.
            ("haftalık yemek tablosu yap", "belge-olustur"),
            ("takvimimi göster", "takvim"),
        ]
        for (soru, beklenen) in denemeler {
            let bulunan = BeceriDeposu.eslesen(soru)?.ad ?? "yok"
            let isaret = bulunan == beklenen ? "✓" : "✗ (beklenen: \(beklenen))"
            kayit.append("  '\(soru)' → \(bulunan) \(isaret)")
        }

        // Enjeksiyon bütçesi: en uzun paket becerisi bile sınırı aşmamalı.
        let enUzun = BeceriDeposu.paket
            .map { (ad: $0.ad, uzunluk: BeceriDeposu.enjeksiyonMetni($0).count) }
            .max { $0.uzunluk < $1.uzunluk }
        if let enUzun {
            // Sınır + çit metni (~200 karakter) toplamı makul kalmalı.
            let tavan = BeceriDeposu.enjeksiyonSiniri + 250
            let isaret = enUzun.uzunluk <= tavan ? "✓" : "✗ (tavan: \(tavan))"
            kayit.append("En uzun enjeksiyon: \(enUzun.ad) \(enUzun.uzunluk) krk \(isaret)")
        }

        // Kullanıcı becerisi: sınır doğrulaması + eşleşmede paketin önüne geçmesi.
        kayit.append("--- KULLANICI BECERİSİ ---")
        let uzunGovde = String(repeating: "x", count: KullaniciBecerisi.govdeSiniri + 1)
        let asiri = KullaniciBecerisi(ad: "aşırı", tetiklerHam: "zzz", govde: uzunGovde)
        kayit.append("Sınır aşan gövde reddedildi: \(asiri.gecerliMi ? "✗" : "✓")")
        let bosTetik = KullaniciBecerisi(ad: "boş", tetiklerHam: " , ,", govde: "bir şey")
        kayit.append("Tetiksiz beceri reddedildi: \(bosTetik.gecerliMi ? "✗" : "✓")")

        let ozel = KullaniciBecerisi(ad: "fatura-takibi",
                                     tetiklerHam: "fatura, gider",
                                     govde: "Fatura sorulunca önce arama, sonra hesapla aracını çağır.")
        BeceriDeposu.kullaniciyiYenile([ozel])
        let ozelEslesme = BeceriDeposu.eslesen("bu ayki fatura toplamı ne")?.ad ?? "yok"
        kayit.append("  'fatura' → \(ozelEslesme) \(ozelEslesme == "fatura-takibi" ? "✓" : "✗")")
        // Kapalı beceri modele gitmemeli. (Cümle 'hesap' becerisinin "topla"
        // tetikleyicisine de uyduğu için beklenen sonuç "yok" değil, "fatura-takibi
        // DEĞİL" — kapalı beceri devre dışı kalınca paket becerisine düşer.)
        ozel.aktif = false
        BeceriDeposu.kullaniciyiYenile([ozel])
        let kapali = BeceriDeposu.eslesen("bu ayki fatura toplamı ne")?.ad ?? "yok"
        kayit.append("  kapalıyken → \(kapali) \(kapali != "fatura-takibi" ? "✓" : "✗")")
        BeceriDeposu.kullaniciyiYenile([])

        // Dört spec'in model/ağ gerektirmeyen kabul ölçütleri. Bunlar "gözle bak"
        // değil ASSERT'tir: başarısızlık satırda işaretlenir ve sonda sayılır.
        let defter = OtoTestVakalari.calistir()
        kayit.append(contentsOf: defter.satirlar)
        kayit.append("=== İDDİA: \(defter.iddia) · BAŞARISIZ: \(defter.hata) "
                     + "\(defter.hata == 0 ? "✓" : "✗") ===")

        kayit.append("=== BİTTİ · klasör: \(klasor.path) ===")
        yaz(kayit, klasor: klasor)

        // Askıya alma gerektiren vakalar (onay kapısı) senkron init içinde
        // çalışamaz; ilk run loop turunda çalışıp sonucu aynı dosyaya ekler.
        Task { @MainActor in
            let ek = await OtoTestVakalari.asenkronCalistir()
            var tam = kayit
            tam.append(contentsOf: ek.satirlar)
            tam.append("=== ASENKRON İDDİA: \(ek.iddia) · BAŞARISIZ: \(ek.hata) "
                       + "\(ek.hata == 0 ? "✓" : "✗") ===")
            let toplamHata = defter.hata + ek.hata
            tam.append("=== TOPLAM BAŞARISIZ: \(toplamHata) "
                       + "\(toplamHata == 0 ? "✓ HEPSİ GEÇTİ" : "✗ BAŞARISIZ") ===")
            yaz(tam, klasor: klasor)
        }
    }

    /// Sonucu yalnız print'e ve Caches altındaki test dosyasına yazar.
    /// NSLog kullanılmaz: sistem log'una düşen kişisel veri kalıcı olur.
    @MainActor
    private static func yaz(_ kayit: [String], klasor: URL) {
        let metin = kayit.joined(separator: "\n")
        print(metin)
        try? metin.write(to: klasor.appendingPathComponent("ototest-sonuc.txt"),
                         atomically: true, encoding: .utf8)
    }
}
#endif
