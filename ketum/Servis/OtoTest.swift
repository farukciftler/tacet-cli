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
            (.html, "ototest-sayfa", false),   // kod-spec §4: sayfa da belge kanalıdır
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
        // kod + web-sayfa aktifleşince (kod-spec adım 7) paket 10 beceriye çıktı.
        let paketSayisi = BeceriDeposu.paket.count
        kayit.append("Bundle'dan yüklenen beceri: \(paketSayisi) "
                     + "\(paketSayisi == 10 ? "✓" : "✗ (beklenen: 10)")")
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
            // kod-spec §8: yeni beceriler. "python ile" tetikleyicisi olmasa
            // "hesapla" (7) "python"u (6) yenerdi — özgüllük kuralının gereği.
            ("python ile hesaplar mısın", "kod"),
            ("kahve dükkanım için site yap", "web-sayfa"),
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
        var defter = OtoTestVakalari.calistir()
        // kod-spec §8: HtmlMotor vakaları (saf motor, model/ağ/WKWebView gerekmez).
        defter.ekle(htmlVakalari(klasor: klasor))
        kayit.append(contentsOf: defter.satirlar)
        kayit.append("=== İDDİA: \(defter.iddia) · BAŞARISIZ: \(defter.hata) "
                     + "\(defter.hata == 0 ? "✓" : "✗") ===")

        kayit.append("=== BİTTİ · klasör: \(klasor.path) ===")
        yaz(kayit, klasor: klasor)

        // Askıya alma gerektiren vakalar (onay kapısı) senkron init içinde
        // çalışamaz; ilk run loop turunda çalışıp sonucu aynı dosyaya ekler.
        Task { @MainActor in
            var ek = await OtoTestVakalari.asenkronCalistir()
            // kod-spec §8: KodMotoru + deneme sayacı vakaları (await gerektirir;
            // zaman aşımı vakası ~3 sn sürer, launch-anı senkron akışına sığmaz).
            ek.ekle(await kodVakalari())
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

    // MARK: - kod-spec §8: HtmlMotor

    /// Markdown → HTML dökümü + `oku` geri çıkarımı. SayfaDogrulayici BİLEREK
    /// çağrılmaz (WKWebView launch-anı senkron akışında güvenilmez) — buradaki
    /// iddialar doğrulamanın koruduğu şeyin ta kendisini (markdown ayrıştırması
    /// ve şablonun kendine yeterliği) statik olarak sınar.
    @MainActor
    private static func htmlVakalari(klasor: URL) -> OtoTestDefteri {
        var d = OtoTestDefteri()
        d.baslik("HTML MOTOR (kod-spec §4)")

        let markdown = """
        # Köşe Kahve
        Taze kavrulmuş & günlük.

        ## Menü
        | Kahve | Fiyat |
        | --- | --- |
        | Filtre | 90 |
        | <Espresso> | 120 |

        ## Özellikler
        - Hızlı servis
        - **Taze** çekirdek

        ### Adres
        Mah. Cad. No 3
        """

        let motor = HtmlMotor()
        do {
            let url = try motor.yaz(dosyaAdi: "ototest-html-vaka", baslik: nil,
                                    govde: markdown, tablo: nil, klasor: klasor)
            defer { try? FileManager.default.removeItem(at: url) }
            let ham = try String(contentsOf: url, encoding: .utf8)

            // Kendine yeterlik: şablonda ve üretimde HİÇBİR harici istek izi yok.
            // "http" araması boş dönmeli — sayfa da ağ vaadi taşır (kod-spec §4.2).
            d.dogru(!ham.contains("http"), "üretilen HTML'de 'http' geçmez (harici istek yok)")
            d.dogru(!ham.contains("<script"), "üretilen HTML betik içermez")

            // Markdown → şablon eşlemesinin izleri.
            d.dogru(ham.contains("<header class=\"hero\">"), "ilk '# ' hero bölümü olur")
            d.dogru(ham.contains("<h1>Köşe Kahve</h1>"), "hero başlığı h1 taşır")
            d.dogru(ham.contains("class=\"tagline\""), "hero altındaki paragraf tagline olur")
            d.dogru(ham.contains("<title>Köşe Kahve</title>"), "sayfa başlığı hero'dan gelir")
            d.dogru(ham.contains("<h2>Menü</h2>"), "'## ' bölüm başlığı olur")
            d.dogru(ham.contains("<h3>Adres</h3>"), "'### ' bölüm içi alt başlık olur")
            d.dogru(ham.contains("<table>"), "markdown tablosu <table> olur")
            d.dogru(ham.contains("class=\"kartlar\""), "'- ' listesi kart grid'i olur")
            d.dogru(ham.contains("&lt;Espresso&gt;"), "içerikteki < > kaçışlanır")
            d.dogru(ham.contains("&amp; günlük"), "içerikteki & kaçışlanır")
            d.dogru(ham.contains("<strong>Taze</strong>"), "**kalın** strong olur")

            // Round-trip: oku etiketleri ayıklayıp markdown öneklerine geri çevirir —
            // "siteye bölüm ekle" akışı (belge_oku → belge_duzenle) buna dayanır.
            let geri = try motor.oku(url: url).metin
            d.dogru(geri.contains("# Köşe Kahve"), "oku: hero '# ' önekiyle geri döner")
            d.dogru(geri.contains("Taze kavrulmuş & günlük."), "oku: & kaçışı geri alınır")
            d.dogru(geri.contains("## Menü"), "oku: bölüm '## ' önekiyle geri döner")
            d.dogru(geri.contains("### Adres"), "oku: alt başlık '### ' önekiyle geri döner")
            d.dogru(geri.contains("| Filtre | 90 |"), "oku: tablo markdown satırına geri döner")
            d.dogru(geri.contains("| <Espresso> | 120 |"), "oku: hücre kaçışları geri alınır")
            d.dogru(geri.contains("- Hızlı servis"), "oku: kart '- ' maddesine geri döner")
            d.dogru(!geri.contains("<style"), "oku: stil bloğu tamamen ayıklanır")
            d.dogru(!geri.contains("</"), "oku: hiçbir kapanış etiketi sızmaz")
        } catch {
            d.dogru(false, "HtmlMotor yaz/oku turu", "\(error)")
        }
        return d
    }

    // MARK: - kod-spec §8: KodMotoru + deneme sayacı

    @MainActor
    private static func kodVakalari() async -> OtoTestDefteri {
        var d = OtoTestDefteri()
        d.baslik("KOD MOTORU (kod-spec §5)")

        // print(6*7) → "42".
        switch await KodMotoru.calistir("print(6*7)") {
        case .basarili(let cikti, _):
            d.esit(cikti, "42", "print(6*7) çıktısı 42")
        default:
            d.dogru(false, "print(6*7) çıktısı 42", "başarılı sonuç dönmedi")
        }

        // print çağrılmadıysa son ifadenin değeri çıktı sayılır.
        switch await KodMotoru.calistir("6*7") {
        case .basarili(let cikti, _):
            d.esit(cikti, "42", "son ifade biçimi (6*7) de 42 verir")
        default:
            d.dogru(false, "son ifade biçimi (6*7) de 42 verir", "başarılı sonuç dönmedi")
        }

        // Sözdizimi hatası → error + satır numarası.
        switch await KodMotoru.calistir("let a=1;\nlet x = ;") {
        case .hata(let mesaj):
            d.dogru(true, "sözdizimi hatası hata döner")
            d.dogru(mesaj.contains("line"), "hata satır numarası taşır", mesaj)
        default:
            d.dogru(false, "sözdizimi hatası hata döner", "hata dönmedi")
        }

        // Sonsuz döngü → ~3 sn'de zaman aşımı; sessiz donma yok.
        let baslangic = Date()
        let dongu = await KodMotoru.calistir("while(true){}")
        let gecen = Date().timeIntervalSince(baslangic)
        if case .zamanAsimi = dongu {
            d.dogru(true, "sonsuz döngü zaman aşımı döner")
        } else {
            d.dogru(false, "sonsuz döngü zaman aşımı döner", "\(dongu)")
        }
        d.dogru(gecen >= 2.5 && gecen < 10,
                "zaman aşımı ~3 sn'de gerçekleşir", String(format: "%.1f sn", gecen))

        // 10k üstü çıktı kırpılır ve kırpıldığı söylenir.
        switch await KodMotoru.calistir("let s='x'.repeat(20000); print(s)") {
        case .basarili(let cikti, _):
            d.dogru(cikti.contains(Yerel.kodCiktiKirpildi), "kırpma notu çıktıya eklenir")
            d.dogru(cikti.count <= KodMotoru.ciktiTavani + Yerel.kodCiktiKirpildi.count + 1,
                    "kırpılan çıktı tavanı aşmaz", "\(cikti.count)")
        default:
            d.dogru(false, "10k üstü çıktı kırpılarak döner", "başarılı sonuç dönmedi")
        }

        // Sandbox: fetch/require tanımsızdır — hata döner, ÇALIŞMAZ.
        switch await KodMotoru.calistir("fetch('https://ornek.com')") {
        case .hata(let mesaj):
            d.dogru(mesaj.contains("fetch"), "fetch tanımsız (ağ köprüsü yok)", mesaj)
        default:
            d.dogru(false, "fetch tanımsız (ağ köprüsü yok)", "hata dönmedi")
        }
        switch await KodMotoru.calistir("require('fs')") {
        case .hata(let mesaj):
            d.dogru(mesaj.contains("require"), "require tanımsız (dosya köprüsü yok)", mesaj)
        default:
            d.dogru(false, "require tanımsız (dosya köprüsü yok)", "hata dönmedi")
        }

        // Deneme sayacı (kod-spec §5.4): 3. çağrı motoru HİÇ görmeden reddedilir.
        d.baslik("KOD ARACI · DENEME SAYACI (kod-spec §5.4)")
        let durum = KodDurumu()
        var arac = KodCalistirAraci()
        arac.durum = durum

        let a1 = await arac.call(arguments: .init(kod: "print(1)"))
        d.dogru(a1.hasPrefix("ok"), "1. çağrı çalışır", a1)
        let a2 = await arac.call(arguments: .init(kod: "print(2)"))
        d.dogru(a2.hasPrefix("ok"), "2. çağrı çalışır", a2)
        d.esit(durum.deneme, 2, "sayaç iki denemeyi saydı")
        let a3 = await arac.call(arguments: .init(kod: "print(3)"))
        d.dogru(a3.hasPrefix("error_final"), "3. çağrı error_final ile reddedilir", a3)
        d.dogru(!a3.contains("ok ("), "3. çağrıda motor çalıştırılmaz")

        // Canlıda sıfırlama ModelServisi.yanitSonucu turu, hataKurtar retry
        // dalları ve sohbetiSifirla'dadır — burada sözleşme doğrulanır.
        durum.yeniTur()
        d.esit(durum.deneme, 0, "yeniTur() sayacı sıfırlar")
        let a4 = await arac.call(arguments: .init(kod: "print(4)"))
        d.dogru(a4.hasPrefix("ok"), "yeni turda çağrı yeniden çalışır", a4)

        return d
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
