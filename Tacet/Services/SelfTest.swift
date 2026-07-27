//
//  OtoTest.swift
//  Tacet
//
//  DEBUG kendi kendine testi: gerçek motor kodunu (model olmadan) büyük veriyle
//  çalıştırır. Bu, büyük dosya kanalının ta kendisidir — uygulama katmanı tabloyu
//  doğrudan motora verir, model devrede değildir. "--selftest" argümanıyla açılır.
//

#if DEBUG
import Foundation

enum SelfTest {
    @MainActor
    static func run() {
        var record = ["=== TACET OTOTEST ==="]
        let folder = DocumentContext.testKlasoru()

        // Büyük tablo: 500 satır, sayısal "Tutar" sütunu (Excel'de =SUM tetikler).
        var rows: [Row] = []
        var beklenenToplam = 0
        for i in 1...500 {
            let holds = i * 7
            beklenenToplam += holds
            rows.append(Row(cells: [
                "2026-07-\(String(format: "%02d", (i % 28) + 1))",
                "Kalem \(i)",
                "\(holds)",
            ]))
        }
        let table = Table(headers: ["Tarih", "Açıklama", "Tutar"], rows: rows)
        record.append("Tablo: \(table.rows.count) satır, beklenen Tutar toplamı=\(beklenenToplam)")

        let isler: [(DocumentFormat, String, Bool)] = [
            (.xlsx, "ototest-excel", true),
            (.docx, "ototest-word", false),
            (.pdf,  "ototest-pdf",  false),
            (.md,   "ototest-markdown", false),
            (.html, "ototest-sayfa", false),   // kod-spec §4: sayfa da belge kanalıdır
        ]

        for (format, name, tabloMu) in isler {
            do {
                let engine = DocumentEngines.engine(format)
                let body = tabloMu ? nil : "# Oto Test Raporu\n\n" + table.markdown
                let url = try engine.write(fileName: name,
                                        title: "Oto Test",
                                        body: body,
                                        table: tabloMu ? table : nil,
                                        folder: folder)
                let oznitelikler = try? FileManager.default.attributesOfItem(atPath: url.path)
                let size = (oznitelikler?[.size] as? Int) ?? 0
                // Geri oku (round-trip).
                let back = try engine.read(url: url)
                let satirSayi = back.table?.rows.count ?? back.text.split(separator: "\n").count
                record.append("\(format.fileExtension): YAZILDI \(url.lastPathComponent) \(size)B · OKUNDU satır/blok=\(satirSayi)")
            } catch {
                record.append("\(format.fileExtension): HATA \(error)")
            }
        }
        // Belge bağlamı: üretilen dosya sonradan okunabilir/düzenlenebilir olmalı.
        // "Excel yap" → "onu tablo olarak göster" akışının dayandığı davranış.
        record.append("--- BELGE BAĞLAMI ---")
        let context = DocumentContext()
        record.append("Başlangıçta çalışılabilir belge: \(context.runnableDocument == nil ? "none ✓" : "var ✗")")
        let uretilen = folder.appendingPathComponent("ototest-excel.xlsx")
        context.outputAdded(uretilen)
        let continued = context.runnableDocument
        record.append("Üretimden sonra: \(continued?.name ?? "none") \(continued?.name == "ototest-excel.xlsx" ? "✓" : "✗")")
        record.append("  biçim: \(continued?.format.tag ?? "-") \(continued?.format == .xlsx ? "✓" : "✗")")
        // Kullanıcının eklediği belge, üretilenin önüne geçmeli.
        let ekliURL = folder.appendingPathComponent("ototest-pdf.pdf")
        context.addDocument(url: ekliURL)
        record.append("Ekli belge öncelikli: \(context.runnableDocument?.name ?? "none") "
                     + "\(context.runnableDocument?.name == "ototest-pdf.pdf" ? "✓" : "✗")")
        // Ekli kaldırılınca üretilene geri düşmeli.
        context.removeDocument()
        record.append("Ekli kaldırılınca üretilene döner: "
                     + "\(context.runnableDocument?.name == "ototest-excel.xlsx" ? "✓" : "✗")")
        // Yeni sohbet: bağlam tamamen temizlenmeli, yoksa eski dosya sızar.
        context.uretimiUnut()
        record.append("Yeni sohbette temizlendi: \(context.runnableDocument == nil ? "✓" : "✗")")

        // Modele dönen tablo GEÇERLİ markdown olmalı — sohbette tablo çizilmesi
        // Tablo.markdownDan'ın bunu ayrıştırabilmesine bağlı.
        record.append("--- MODELE DÖNEN TABLO ---")
        let mdTam = table.truncatedMarkdown(maxRows: 30)
        let geriAyristirilan = Table.fromMarkdown(mdTam)
        record.append("Markdown geri ayrıştırıldı: \(geriAyristirilan != nil ? "✓" : "✗")")
        record.append("  satır: \(geriAyristirilan?.rows.count ?? 0)/30 "
                     + "\(geriAyristirilan?.rows.count == 30 ? "✓" : "✗")")
        record.append("  başlık: \(geriAyristirilan?.headers.joined(separator: ",") ?? "-") "
                     + "\(geriAyristirilan?.headers == table.headers ? "✓" : "✗")")
        record.append("  kırpma notu var: \(mdTam.contains("+470 satır daha") ? "✓" : "✗")")
        // Kırpma gerekmeyen küçük tablo olduğu gibi dönmeli.
        let small = Table(headers: ["Gün", "Yemek"],
                          rows: [Row(cells: ["Pazartesi", "Mercimek"])])
        record.append("Küçük tablo kırpılmadı: "
                     + "\(small.truncatedMarkdown(maxRows: 30) == small.markdown ? "✓" : "✗")")

        // Beceri (SKILL.md) katmanı testi.
        record.append("--- BECERİLER (SKILL.md) ---")
        // kod + web-sayfa aktifleşince (kod-spec adım 7) paket 10 beceriye çıktı.
        let paketSayisi = SkillStore.package.count
        record.append("Bundle'dan yüklenen beceri: \(paketSayisi) "
                     + "\(paketSayisi == 10 ? "✓" : "✗ (expected: 10)")")
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
            // "tablo yap" ARTIK belge-olustur'u tetiklemez ve bu bilinçlidir:
            // çıplak "tablo" tetikleyicisi kaldırıldı, çünkü tablo bir GÖSTERİM
            // isteğidir, dosya isteği değil (Yonlendirici.belgeEki). Kullanıcı
            // ekranda tablo isterken sessizce .xlsx üretiliyordu.
            ("haftalık yemek tablosu yap", "yok"),
            // Dosya AÇIKÇA istenince belge-olustur yine tetiklenmeli — kaldırma
            // bu yolu kapatmamalı (asıl regresyon riski burada).
            ("haftalık yemek listesini excel yap", "belge-olustur"),
            ("takvimimi göster", "takvim"),
            // kod-spec §8: yeni beceriler. "python ile" tetikleyicisi olmasa
            // "hesapla" (7) "python"u (6) yenerdi — özgüllük kuralının gereği.
            ("python ile hesaplar mısın", "kod"),
            ("kahve dükkanım için site yap", "web-sayfa"),
        ]
        for (question, expected) in denemeler {
            let bulunan = SkillStore.eslesen(question)?.name ?? "yok"
            let marker = bulunan == expected ? "✓" : "✗ (beklenen: \(expected))"
            record.append("  '\(question)' → \(bulunan) \(marker)")
        }

        // Enjeksiyon bütçesi: en uzun paket becerisi bile sınırı aşmamalı.
        let enUzun = SkillStore.package
            .map { (name: $0.name, length: SkillStore.enjeksiyonMetni($0).count) }
            .max { $0.length < $1.length }
        if let enUzun {
            // Sınır + çit metni (~200 karakter) toplamı makul kalmalı.
            let cap = SkillStore.enjeksiyonSiniri + 250
            let marker = enUzun.length <= cap ? "✓" : "✗ (tavan: \(cap))"
            record.append("En uzun enjeksiyon: \(enUzun.name) \(enUzun.length) krk \(marker)")
        }

        // Kullanıcı becerisi: sınır doğrulaması + eşleşmede paketin önüne geçmesi.
        record.append("--- KULLANICI BECERİSİ ---")
        let uzunGovde = String(repeating: "x", count: UserSkill.bodyLimit + 1)
        let asiri = UserSkill(name: "aşırı", rawTriggers: "zzz", body: uzunGovde)
        record.append("Sınır aşan gövde reddedildi: \(asiri.isValid ? "✗" : "✓")")
        let bosTetik = UserSkill(name: "boş", rawTriggers: " , ,", body: "bir şey")
        record.append("Tetiksiz beceri reddedildi: \(bosTetik.isValid ? "✗" : "✓")")

        let ozel = UserSkill(name: "fatura-takibi",
                                     rawTriggers: "fatura, gider",
                                     body: "Fatura sorulunca önce arama, sonra hesapla aracını çağır.")
        SkillStore.reloadUser([ozel])
        let ozelEslesme = SkillStore.eslesen("bu ayki fatura toplamı ne")?.name ?? "yok"
        record.append("  'fatura' → \(ozelEslesme) \(ozelEslesme == "fatura-takibi" ? "✓" : "✗")")
        // Kapalı beceri modele gitmemeli. (Cümle 'hesap' becerisinin "topla"
        // tetikleyicisine de uyduğu için beklenen sonuç "yok" değil, "fatura-takibi
        // DEĞİL" — kapalı beceri devre dışı kalınca paket becerisine düşer.)
        ozel.isActive = false
        SkillStore.reloadUser([ozel])
        let closed = SkillStore.eslesen("bu ayki fatura toplamı ne")?.name ?? "yok"
        record.append("  kapalıyken → \(closed) \(closed != "fatura-takibi" ? "✓" : "✗")")
        SkillStore.reloadUser([])

        // Dört spec'in model/ağ gerektirmeyen kabul ölçütleri. Bunlar "gözle bak"
        // değil ASSERT'tir: başarısızlık satırda işaretlenir ve sonda sayılır.
        var ledger = SelfTestCases.run()
        // kod-spec §8: HtmlMotor vakaları (saf motor, model/ağ/WKWebView gerekmez).
        ledger.add(htmlVakalari(folder: folder))
        record.append(contentsOf: ledger.lines)
        record.append("=== İDDİA: \(ledger.iddia) · BAŞARISIZ: \(ledger.error) "
                     + "\(ledger.error == 0 ? "✓" : "✗") ===")

        record.append("=== BİTTİ · klasör: \(folder.path) ===")
        write(record, folder: folder)

        // Askıya alma gerektiren vakalar (onay kapısı) senkron init içinde
        // çalışamaz; ilk run loop turunda çalışıp sonucu aynı dosyaya ekler.
        Task { @MainActor in
            var ek = await SelfTestCases.asenkronCalistir()
            // kod-spec §8: KodMotoru + deneme sayacı vakaları (await gerektirir;
            // zaman aşımı vakası ~3 sn sürer, launch-anı senkron akışına sığmaz).
            ek.add(await kodVakalari())
            var tam = record
            tam.append(contentsOf: ek.lines)
            tam.append("=== ASENKRON İDDİA: \(ek.iddia) · BAŞARISIZ: \(ek.error) "
                       + "\(ek.error == 0 ? "✓" : "✗") ===")
            let toplamHata = ledger.error + ek.error
            tam.append("=== TOPLAM BAŞARISIZ: \(toplamHata) "
                       + "\(toplamHata == 0 ? "✓ HEPSİ GEÇTİ" : "✗ BAŞARISIZ") ===")
            write(tam, folder: folder)
        }
    }

    // MARK: - kod-spec §8: HtmlMotor

    /// Markdown → HTML dökümü + `oku` geri çıkarımı. SayfaDogrulayici BİLEREK
    /// çağrılmaz (WKWebView launch-anı senkron akışında güvenilmez) — buradaki
    /// iddialar doğrulamanın koruduğu şeyin ta kendisini (markdown ayrıştırması
    /// ve şablonun kendine yeterliği) statik olarak sınar.
    @MainActor
    private static func htmlVakalari(folder: URL) -> SelfTestLedger {
        var d = SelfTestLedger()
        d.title("HTML MOTOR (kod-spec §4)")

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

        let engine = HtmlEngine()
        do {
            let url = try engine.write(fileName: "ototest-html-vaka", title: nil,
                                    body: markdown, table: nil, folder: folder)
            defer { try? FileManager.default.removeItem(at: url) }
            let raw = try String(contentsOf: url, encoding: .utf8)

            // Kendine yeterlik: şablonda ve üretimde HİÇBİR harici istek izi yok.
            // "http" araması boş dönmeli — sayfa da ağ vaadi taşır (kod-spec §4.2).
            d.dogru(!raw.contains("http"), "üretilen HTML'de 'http' geçmez (harici istek yok)")
            d.dogru(!raw.contains("<script"), "üretilen HTML betik içermez")

            // Markdown → şablon eşlemesinin izleri.
            d.dogru(raw.contains("<header class=\"hero\">"), "ilk '# ' hero bölümü olur")
            d.dogru(raw.contains("<h1>Köşe Kahve</h1>"), "hero başlığı h1 taşır")
            d.dogru(raw.contains("class=\"tagline\""), "hero altındaki paragraf tagline olur")
            d.dogru(raw.contains("<title>Köşe Kahve</title>"), "sayfa başlığı hero'dan gelir")
            d.dogru(raw.contains("<h2>Menü</h2>"), "'## ' bölüm başlığı olur")
            d.dogru(raw.contains("<h3>Adres</h3>"), "'### ' bölüm içi alt başlık olur")
            d.dogru(raw.contains("<table>"), "markdown tablosu <table> olur")
            d.dogru(raw.contains("class=\"kartlar\""), "'- ' listesi kart grid'i olur")
            d.dogru(raw.contains("&lt;Espresso&gt;"), "içerikteki < > kaçışlanır")
            d.dogru(raw.contains("&amp; günlük"), "içerikteki & kaçışlanır")
            d.dogru(raw.contains("<strong>Taze</strong>"), "**kalın** strong olur")

            // Round-trip: oku etiketleri ayıklayıp markdown öneklerine geri çevirir —
            // "siteye bölüm ekle" akışı (belge_oku → belge_duzenle) buna dayanır.
            let back = try engine.read(url: url).text
            d.dogru(back.contains("# Köşe Kahve"), "oku: hero '# ' önekiyle geri döner")
            d.dogru(back.contains("Taze kavrulmuş & günlük."), "oku: & kaçışı geri alınır")
            d.dogru(back.contains("## Menü"), "oku: bölüm '## ' önekiyle geri döner")
            d.dogru(back.contains("### Adres"), "oku: alt başlık '### ' önekiyle geri döner")
            d.dogru(back.contains("| Filtre | 90 |"), "oku: tablo markdown satırına geri döner")
            d.dogru(back.contains("| <Espresso> | 120 |"), "oku: hücre kaçışları geri alınır")
            d.dogru(back.contains("- Hızlı servis"), "oku: kart '- ' maddesine geri döner")
            d.dogru(!back.contains("<style"), "oku: stil bloğu tamamen ayıklanır")
            d.dogru(!back.contains("</"), "oku: hiçbir kapanış etiketi sızmaz")
        } catch {
            d.dogru(false, "HtmlMotor yaz/oku turu", "\(error)")
        }
        return d
    }

    // MARK: - kod-spec §8: KodMotoru + deneme sayacı

    @MainActor
    private static func kodVakalari() async -> SelfTestLedger {
        var d = SelfTestLedger()
        d.title("KOD MOTORU (kod-spec §5)")

        // print(6*7) → "42".
        switch await CodeEngine.run("print(6*7)") {
        case .succeeded(let output, _):
            d.equal(output, "42", "print(6*7) çıktısı 42")
        default:
            d.dogru(false, "print(6*7) çıktısı 42", "başarılı sonuç dönmedi")
        }

        // print çağrılmadıysa son ifadenin değeri çıktı sayılır.
        switch await CodeEngine.run("6*7") {
        case .succeeded(let output, _):
            d.equal(output, "42", "son ifade biçimi (6*7) de 42 verir")
        default:
            d.dogru(false, "son ifade biçimi (6*7) de 42 verir", "başarılı sonuç dönmedi")
        }

        // Sözdizimi hatası → error + satır numarası.
        switch await CodeEngine.run("let a=1;\nlet x = ;") {
        case .error(let message):
            d.dogru(true, "sözdizimi hatası hata döner")
            d.dogru(message.contains("line"), "hata satır numarası taşır", message)
        default:
            d.dogru(false, "sözdizimi hatası hata döner", "hata dönmedi")
        }

        // Sonsuz döngü → ~3 sn'de zaman aşımı; sessiz donma yok.
        let start = Date()
        let loop = await CodeEngine.run("while(true){}")
        let gecen = Date().timeIntervalSince(start)
        if case .timeout = loop {
            d.dogru(true, "sonsuz döngü zaman aşımı döner")
        } else {
            d.dogru(false, "sonsuz döngü zaman aşımı döner", "\(loop)")
        }
        d.dogru(gecen >= 2.5 && gecen < 10,
                "zaman aşımı ~3 sn'de gerçekleşir", String(format: "%.1f sn", gecen))

        // 10k üstü çıktı kırpılır ve kırpıldığı söylenir.
        switch await CodeEngine.run("let s='x'.repeat(20000); print(s)") {
        case .succeeded(let output, _):
            d.dogru(output.contains(L10n.codeOutputTruncated), "kırpma notu çıktıya eklenir")
            d.dogru(output.count <= CodeEngine.outputCap + L10n.codeOutputTruncated.count + 1,
                    "kırpılan çıktı tavanı aşmaz", "\(output.count)")
        default:
            d.dogru(false, "10k üstü çıktı kırpılarak döner", "başarılı sonuç dönmedi")
        }

        // Sandbox: fetch/require tanımsızdır — hata döner, ÇALIŞMAZ.
        switch await CodeEngine.run("fetch('https://ornek.com')") {
        case .error(let message):
            d.dogru(message.contains("fetch"), "fetch tanımsız (ağ köprüsü yok)", message)
        default:
            d.dogru(false, "fetch tanımsız (ağ köprüsü yok)", "hata dönmedi")
        }
        switch await CodeEngine.run("require('fs')") {
        case .error(let message):
            d.dogru(message.contains("require"), "require tanımsız (dosya köprüsü yok)", message)
        default:
            d.dogru(false, "require tanımsız (dosya köprüsü yok)", "hata dönmedi")
        }

        // Deneme sayacı (kod-spec §5.4): 3. çağrı motoru HİÇ görmeden reddedilir.
        d.title("KOD ARACI · DENEME SAYACI (kod-spec §5.4)")
        let state = CodeState()
        var tool = RunCodeTool()
        tool.state = state

        let a1 = await tool.call(arguments: .init(code: "print(1)"))
        d.dogru(a1.hasPrefix("ok"), "1. çağrı çalışır", a1)
        let a2 = await tool.call(arguments: .init(code: "print(2)"))
        d.dogru(a2.hasPrefix("ok"), "2. çağrı çalışır", a2)
        d.equal(state.attempt, 2, "sayaç iki denemeyi saydı")
        let a3 = await tool.call(arguments: .init(code: "print(3)"))
        d.dogru(a3.hasPrefix("error_final"), "3. çağrı error_final ile reddedilir", a3)
        d.dogru(!a3.contains("ok ("), "3. çağrıda motor çalıştırılmaz")

        // Canlıda sıfırlama ModelServisi.yanitSonucu turu, hataKurtar retry
        // dalları ve sohbetiSifirla'dadır — burada sözleşme doğrulanır.
        state.newTurn()
        d.equal(state.attempt, 0, "newTurn() sayacı sıfırlar")
        let a4 = await tool.call(arguments: .init(code: "print(4)"))
        d.dogru(a4.hasPrefix("ok"), "yeni turda çağrı yeniden çalışır", a4)

        return d
    }

    /// Sonucu yalnız print'e ve Caches altındaki test dosyasına yazar.
    /// NSLog kullanılmaz: sistem log'una düşen kişisel veri kalıcı olur.
    @MainActor
    private static func write(_ record: [String], folder: URL) {
        let text = record.joined(separator: "\n")
        print(text)
        try? text.write(to: folder.appendingPathComponent("ototest-sonuc.txt"),
                         atomically: true, encoding: .utf8)
    }
}
#endif
