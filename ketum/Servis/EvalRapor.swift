//
//  EvalRapor.swift
//  ketum
//
//  Eval puanlama + raporlama katmanı. Degerlendirme.swift'in "geçti/kaldı"
//  ikili kararı, bir vakanın NEDEN kaldığını göstermez: yanlış araç seçimiyle
//  uydurma bilgi aynı ✗ ile loglanır. Burada her vaka 0-100 arası boyutlara
//  ayrılmış puan alır, sonuçlar Excel'e ve markdown özete dökülür.
//
//  Not: Degerlendirme gibi bu dosya da NSLog/os_log KULLANMAZ — eval çıktısı
//  gerçek takvim/kişi verisi içerebilir, sistem log'una sızmamalı.
//

#if DEBUG
import Foundation

/// Tek bir vaka koşusunun puanlanmış sonucu.
/// `mod` iki koşu biçimini ayırır: "tekil" — her vaka öncesi `sohbetiSifirla()`
/// çağrılmış bağımsız oturum; "zincir" — aynı oturumda sıralı adımlar.
/// Kullanıcının asıl sorusu bu ikisinin farkı olduğu için mod her satırda taşınır.
/// `Codable`: kapsamlı koşu ham satırları JSON'a döküyor, analiz ajanları
/// Excel yerine bu dosyayı okuyor.
struct EvalSonuc: Codable {
    let vakaAd: String
    let kategori: String
    let mod: String          // "tekil" | "zincir"
    var adimNo: Int = 1      // zincirde kaçıncı adım; tekilde 1
    let istem: String
    var beklenenCipler: [String] = []
    var gercekCipler: [String] = []
    var yanit: String = ""
    var puan: Int = 0
    var sorunlar: [String] = []
    var sureMs: Int = 0
    /// Tur bekçiye takıldı: yanıt YARIDA kesildi, dolayısıyla bu vaka
    /// ÖLÇÜLEMEDİ. Kalite sinyali DEĞİLDİR — ölçüm artefaktıdır ve bu yüzden
    /// hiçbir ortalamaya girmez (bkz. `EvalRapor.ortalama`). Kesilmiş turu 0
    /// puan saymak, koşum ortamının yavaşlığını modelin kusuru gibi raporlar;
    /// önceki koşumda ham ortalamayı 92.3'ten 66.0'a düşüren hata buydu.
    var olculemedi: Bool = false
}

/// Vaka başına 0-100 puanlama. Boyut ağırlıkları bilinçli seçildi:
///
/// - **Araç doğruluğu 40** — sistemin tek ayırt edici iddiası doğru aracı doğru
///   anda çağırması. Yanlış araç seçimi kullanıcının verisine yanlış yerden
///   dokunur (takvime yazar, kişi okur); dil kalitesinden kat kat pahalıdır.
/// - **Dürüstlük 30** — cihaz-üstü küçük model için en büyük risk uydurmadır.
///   Yanlış bir cevap, "bilmiyorum" diyen bir cevaptan kötüdür: kullanıcı
///   doğrulayamaz. Bu yüzden `yanitIcermemeli` eşleşmesi (uydurma) boyutu
///   doğrudan 0'a düşürür — kısmi puan yok, ceza en ağırı.
/// - **İçerik 20** — beklenen bilginin yanıtta bulunması. Araç doğru çağrılıp
///   sonucu yanıta taşınmazsa vaka kısmen başarılıdır; sıfırlanmaz.
/// - **Biçim 10** — okunabilirlik ve dil tutarlılığı. Gerçek bir kusur ama
///   veri bütünlüğü riski taşımaz; en düşük ağırlık.
enum EvalPuan {
    static let aracAgirlik = 40
    static let durustlukAgirlik = 30
    static let icerikAgirlik = 20
    static let bicimAgirlik = 10

    /// Yanıtı hata olarak işaretleyen kalıplar (Degerlendirme ile aynı küme).
    static let hataKaliplari = ["yapamadım", "hazır değil", "sorun oldu"]
    /// Kullanıcıya sızmaması gereken iç/meta ifadeler.
    static let metaKaliplari = ["önizle", "paylaşabilir"]

    /// Türkçe istem → Türkçe yanıt beklenir. Sezgisel: yaygın İngilizce işlev
    /// kelimeleri boşluklarla sarılı aranır ("the " gibi) — "these"/"other"
    /// gibi kelimelerin içinde yakalanmaması için. Tek eşleşme yeterli değil;
    /// eşik 2, çünkü tek bir teknik terim ("the API") yanlış pozitif üretir.
    static let ingilizceIsaretler = [
        " the ", "the ", " is ", " are ", " you ", " your ", " and ", " with ",
        " for ", " that ", " this ", " here ", " i'll ", " i can ", " let me ",
        " sorry", " please ", " it's ", " don't ", " will be ", " of the "
    ]

    /// Yanıtta İngilizce sızıntısı var mı (Türkçe istem varsayımıyla).
    static func ingilizceSizintisi(_ yanit: String) -> Bool {
        let d = " " + yanit.lowercased().replacingOccurrences(of: "\n", with: " ") + " "
        var sayi = 0
        for isaret in ingilizceIsaretler where d.contains(isaret) {
            sayi += 1
            if sayi >= 2 { return true }
        }
        return false
    }

    /// Bir vakayı puanlar. `sonuc` içindeki gerçek alanlar doldurulmuş olmalı;
    /// dönen değer `puan` ve `sorunlar` alanları yazılmış kopyadır.
    static func puanla(
        _ sonuc: EvalSonuc,
        cipYok: Bool = false,
        yanitIcermeli: String? = nil,
        yanitIcermemeli: String? = nil,
        basarisizCipVar: Bool = false
    ) -> EvalSonuc {
        var s = sonuc
        var sorunlar: [String] = []
        let yanit = s.yanit
        let bos = yanit.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty

        // — Araç doğruluğu (40) —
        var aracPuan = aracAgirlik
        if cipYok {
            // Normal sohbette araç çağrılması ciddi hata: boyut sıfırlanır.
            if !s.gercekCipler.isEmpty {
                aracPuan = 0
                sorunlar.append("beklenmeyen-arac:\(s.gercekCipler.joined(separator: ","))")
            }
        } else if !s.beklenenCipler.isEmpty {
            var eksik: [String] = []
            for beklenen in s.beklenenCipler
            where !s.gercekCipler.contains(where: { $0.hasPrefix(beklenen) }) {
                eksik.append(beklenen)
            }
            if !eksik.isEmpty {
                // Eksik çip oranınca düşür — 2 beklenenden 1'i düşmüşse yarım puan.
                let tutan = s.beklenenCipler.count - eksik.count
                aracPuan = aracAgirlik * tutan / s.beklenenCipler.count
                sorunlar.append("eksik-arac:\(eksik.joined(separator: ","))")
            }
            // Fazladan çip küçük ceza (10): model gereksiz araç çağırmış ama
            // doğru olanı da çağırmış — yanlış araç seçiminden hafif bir kusur.
            let fazla = s.gercekCipler.filter { c in
                !s.beklenenCipler.contains(where: { c.hasPrefix($0) })
            }
            if !fazla.isEmpty {
                aracPuan = max(0, aracPuan - 10)
                sorunlar.append("fazla-arac:\(Set(fazla).sorted().joined(separator: ","))")
            }
        }
        if basarisizCipVar {
            aracPuan = max(0, aracPuan - 20)
            sorunlar.append("basarisiz-cip")
        }

        // — Dürüstlük (30) —
        var durustlukPuan = durustlukAgirlik
        if let uydurma = yanitIcermemeli,
           yanit.localizedCaseInsensitiveContains(uydurma) {
            // EN AĞIR CEZA: uydurma tespiti boyutu tamamen sıfırlar.
            durustlukPuan = 0
            sorunlar.append("uydurma:\(uydurma)")
        } else {
            if hataKaliplari.contains(where: { yanit.localizedCaseInsensitiveContains($0) }) {
                durustlukPuan -= 15
                sorunlar.append("hata-yaniti")
            }
            if metaKaliplari.contains(where: { yanit.localizedCaseInsensitiveContains($0) }) {
                durustlukPuan -= 10
                sorunlar.append("meta-sizinti")
            }
            durustlukPuan = max(0, durustlukPuan)
        }

        // — İçerik (20) —
        var icerikPuan = icerikAgirlik
        if bos {
            icerikPuan = 0
            sorunlar.append("bos-yanit")
        } else if let beklenen = yanitIcermeli,
                  !yanit.localizedCaseInsensitiveContains(beklenen) {
            icerikPuan = 0
            sorunlar.append("yanit-icermiyor:\(beklenen)")
        }

        // — Biçim (10) —
        var bicimPuan = bicimAgirlik
        if yanit.count > 1200 {
            bicimPuan -= 5
            sorunlar.append("asiri-uzun:\(yanit.count)")
        }
        if !bos, ingilizceSizintisi(yanit) {
            bicimPuan -= 5
            sorunlar.append("ingilizce-sizinti")
        }
        bicimPuan = max(0, bicimPuan)

        s.puan = max(0, min(100, aracPuan + durustlukPuan + icerikPuan + bicimPuan))
        s.sorunlar = sorunlar
        return s
    }
}

extension EvalRapor {
    /// Deterministik dilim: paralel koşularda her ajan kendi payını alır.
    /// `i % toplam == shard`. Sıralı bloklama yerine modulo — vakalar dosyada
    /// kategoriye göre kümelendiği için blok bölmesi bir shard'a yalnız takvim,
    /// diğerine yalnız hesap düşürürdü; modulo kategorileri dengeli dağıtır.
    static func shardSec<T>(_ hepsi: [T], shard: Int, toplam: Int) -> [T] {
        guard toplam > 1 else { return hepsi }
        let s = ((shard % toplam) + toplam) % toplam
        return hepsi.enumerated().compactMap { $0.offset % toplam == s ? $0.element : nil }
    }
}

/// Excel çıktısı ve markdown özeti.
enum EvalRapor {
    static let sutunlar = [
        "Kategori", "Vaka", "Mod", "Adım", "İstem", "Beklenen çip",
        "Gerçek çip", "Puan", "Sorunlar", "Süre(ms)", "Yanıt"
    ]

    /// Sonuçları xlsx olarak yazar. Puan ve Süre sütunları tamamen sayısal
    /// tutulur ki ExcelMotor altlarına =SUM eklesin (ortalama elle alınır).
    /// Not: aynı ada yazmak dosyayı EZMEZ, motor "-2" ekler; çağıran üzerine
    /// yazmak istiyorsa önce dosyayı silmeli.
    static func excelYaz(_ sonuclar: [EvalSonuc], klasor: URL,
                         dosyaAdi: String = "eval-rapor") throws -> URL {
        let satirlar = sonuclar.map { s in
            Satir(hucreler: [
                s.kategori,
                s.vakaAd,
                s.mod,
                "\(s.adimNo)",
                s.istem,
                s.beklenenCipler.joined(separator: " "),
                s.gercekCipler.joined(separator: " "),
                // Kesilen turun puanı YARIM yanıttan hesaplanmıştır; sayı olarak
                // yazılırsa tabloda sütun ortalaması alan herkes onu gerçek bir
                // düşük puan sanır. Hücre bilerek metin.
                s.olculemedi ? "ÖLÇÜLEMEDİ" : "\(s.puan)",
                s.sorunlar.joined(separator: "; "),
                "\(s.sureMs)",
                kirp(s.yanit, 120)
            ])
        }
        return try ExcelMotor().yaz(
            dosyaAdi: dosyaAdi,
            baslik: "Ketum Eval",
            govde: nil,
            tablo: Tablo(basliklar: sutunlar, satirlar: satirlar),
            klasor: klasor)
    }

    // MARK: - Birleşik nihai rapor

    /// Nihai raporun sütunları — vaka sütunlarının önüne KOŞUM gelir.
    static let birlesikSutunlar = ["Koşum"] + sutunlar

    /// Birden fazla koşumun ham JSON'unu tek sayfada birleştirir.
    ///
    /// `kaynaklar`: (koşum etiketi, ham JSON dosyası) çiftleri. Etiket satır
    /// başına yazılır ki "eski koşum mu, düzeltme sonrası mı" sorusu tabloda
    /// tek bakışta yanıtlanabilsin — üç koşumu tek dosyada birleştirip ayırt
    /// edici sütun koymamak raporu okunamaz yapardı.
    ///
    /// Süre sütunu tamamen sayısal tutulur ki `ExcelMotor` altına GERÇEK bir
    /// `=SUM` düşürsün. Puan sütununda "ÖLÇÜLEMEDİ" metni geçebildiği için
    /// orada formül beklenmez; ölçülemeyen turu sayıya çevirmek raporu
    /// yanıltırdı (bkz. `EvalSonuc.olculemedi`).
    static func birlesikExcelYaz(_ kaynaklar: [(etiket: String, url: URL)],
                                 klasor: URL,
                                 dosyaAdi: String) throws -> URL {
        let cozucu = JSONDecoder()
        var satirlar: [Satir] = []
        for kaynak in kaynaklar {
            guard let veri = try? Data(contentsOf: kaynak.url),
                  let sonuclar = try? cozucu.decode([EvalSonuc].self, from: veri) else { continue }
            for s in sonuclar {
                satirlar.append(Satir(hucreler: [
                    kaynak.etiket,
                    s.kategori,
                    s.vakaAd,
                    s.mod,
                    "\(s.adimNo)",
                    s.istem,
                    s.beklenenCipler.joined(separator: " "),
                    s.gercekCipler.joined(separator: " "),
                    s.olculemedi ? "ÖLÇÜLEMEDİ" : "\(s.puan)",
                    s.sorunlar.joined(separator: "; "),
                    "\(s.sureMs)",
                    kirp(s.yanit, 200)
                ]))
            }
        }
        return try ExcelMotor().yaz(
            dosyaAdi: dosyaAdi,
            baslik: "sirr — nihai eval raporu",
            govde: nil,
            tablo: Tablo(basliklar: birlesikSutunlar, satirlar: satirlar),
            klasor: klasor)
    }

    /// `--eval-birlestir`: `sirr-test/birlestir/` altındaki `<etiket>.json`
    /// dosyalarını tek Excel'e toplar. Dosya adı koşum etiketidir.
    @MainActor
    static func birlestirmeKosusu() {
        let klasor = BelgeBaglami.testKlasoru()
        let kaynakKlasor = klasor.appendingPathComponent("birlestir", isDirectory: true)
        let dosyalar = (try? FileManager.default.contentsOfDirectory(
            at: kaynakKlasor, includingPropertiesForKeys: nil)) ?? []
        let kaynaklar = dosyalar
            .filter { $0.pathExtension == "json" }
            .sorted { $0.lastPathComponent < $1.lastPathComponent }
            .map { (etiket: $0.deletingPathExtension().lastPathComponent, url: $0) }
        guard !kaynaklar.isEmpty else {
            print("BİRLEŞTİR: \(kaynakKlasor.path) altında json yok")
            return
        }
        let hedef = klasor.appendingPathComponent("sirr-eval-raporu.xlsx")
        try? FileManager.default.removeItem(at: hedef)
        do {
            let url = try birlesikExcelYaz(kaynaklar, klasor: klasor,
                                           dosyaAdi: "sirr-eval-raporu")
            print("BİRLEŞTİR TAMAM: \(url.path) · kaynak \(kaynaklar.count)")
        } catch {
            print("BİRLEŞTİR HATA: \(error)")
        }
    }

    /// Markdown özet: kategori ortalamaları, en düşük 15 vaka, sorun frekansı,
    /// tekil vs zincir karşılaştırması. Satır dizisi döner — mevcut eval log'u
    /// `[String]` biriktirdiği için doğrudan `log += ...` ile eklenebilir.
    static func ozet(_ sonuclar: [EvalSonuc]) -> [String] {
        guard !sonuclar.isEmpty else { return ["(sonuç yok)"] }
        var c: [String] = []

        c.append("## Genel")
        c.append("")
        // Ölçülemeyen vakalar her sayımın DIŞINDA: "zayıf" da "tam puan" da
        // ancak yanıtı sonuna kadar görülmüş turlar için anlamlıdır.
        let olculen = sonuclar.filter { !$0.olculemedi }
        let kesilen = sonuclar.count - olculen.count
        c.append("- Vaka: \(sonuclar.count) (ölçülen \(olculen.count), kesilen \(kesilen))")
        c.append("- Ortalama puan: \(ortalamaMetni(sonuclar))")
        c.append("- Tam puan (100): \(olculen.filter { $0.puan == 100 }.count)")
        c.append("- Zayıf (<60): \(olculen.filter { $0.puan < 60 }.count)")
        if kesilen > 0 {
            let oran = 100.0 * Double(kesilen) / Double(sonuclar.count)
            c.append("- ⚠︎ Bekçiye takılan (ÖLÇÜLEMEDİ, ortalamaya girmez):"
                     + " \(kesilen) (%\(bicim(oran)))")
            c.append("  Bu bir KALİTE bulgusu değil, ölçüm kaybıdır. Oran birkaç"
                     + " yüzdeyi aşıyorsa koşum ortamı yüklüdür: eval süreçlerini"
                     + " PARALEL değil SIRAYLA koşun (aynı Mac'te eşzamanlı"
                     + " simülatörler cihaz-üstü modeli birbirine yavaşlatır).")
        }
        c.append("")

        // — Kategori tablosu —
        c.append("## Kategori bazlı ortalama")
        c.append("")
        c.append("| Kategori | Vaka | Ortalama | Zayıf(<60) |")
        c.append("| --- | ---: | ---: | ---: |")
        for kategori in Set(sonuclar.map(\.kategori)).sorted() {
            let grup = sonuclar.filter { $0.kategori == kategori }
            let zayif = grup.filter { $0.puan < 60 }.count
            c.append("| \(kategori) | \(grup.count) | \(ortalamaMetni(grup)) | \(zayif) |")
        }
        c.append("")

        // — Tekil vs zincir —
        c.append("## Tekil vs Zincir")
        c.append("")
        let tekil = sonuclar.filter { $0.mod == "tekil" }
        let zincir = sonuclar.filter { $0.mod == "zincir" }
        if tekil.isEmpty || zincir.isEmpty {
            c.append("(karşılaştırma için iki mod da gerekli — tekil: \(tekil.count), zincir: \(zincir.count))")
        } else {
            c.append("| Mod | Vaka | Ortalama |")
            c.append("| --- | ---: | ---: |")
            c.append("| tekil | \(tekil.count) | \(ortalamaMetni(tekil)) |")
            c.append("| zincir | \(zincir.count) | \(ortalamaMetni(zincir)) |")
            c.append("")
            let fark = ortalama(zincir) - ortalama(tekil)
            let yon = fark < 0 ? "zincir DAHA KÖTÜ" : (fark > 0 ? "zincir daha iyi" : "fark yok")
            c.append("Fark: \(bicim(fark)) puan — \(yon).")
            // Ortak vakalar iki modda da koşulduysa en çok bozulanı göster.
            let ortak = ortakBozulmalar(tekil: tekil, zincir: zincir)
            if !ortak.isEmpty {
                c.append("")
                c.append("Zincirde en çok bozulan vakalar:")
                for (ad, d) in ortak.prefix(10) {
                    c.append("- \(ad): \(bicim(d)) puan")
                }
            }
        }
        c.append("")

        // — Zincir vs Bağımsız: asıl soru —
        // Aynı adımlar iki koşum biçiminde de koştuysa, bağlam taşımanın net
        // etkisi buradan okunur. Eşleştirme (vakaAd, adimNo) üzerinden yapılır;
        // ortalama farkı değil, ADIM BAŞINA fark anlamlıdır: bağlam genelde ilk
        // adımda değil, sonraki adımlarda işe yarar ya da bozar.
        let bagimsiz = sonuclar.filter { $0.mod == "bagimsiz" }
        if !zincir.isEmpty && !bagimsiz.isEmpty {
            c.append("## Zincir vs Bağımsız oturum")
            c.append("")
            c.append("| Mod | Vaka | Ortalama |")
            c.append("| --- | ---: | ---: |")
            c.append("| zincir (bağlam taşınır) | \(zincir.count) | \(ortalamaMetni(zincir)) |")
            c.append("| bagimsiz (her adım sıfır) | \(bagimsiz.count) | \(ortalamaMetni(bagimsiz)) |")
            c.append("")
            var bagimsizIndeks: [String: Int] = [:]
            for s in bagimsiz { bagimsizIndeks["\(s.vakaAd)#\(s.adimNo)"] = s.puan }
            var farklar: [(String, Int)] = []
            for s in zincir {
                guard let b = bagimsizIndeks["\(s.vakaAd)#\(s.adimNo)"] else { continue }
                farklar.append(("\(s.vakaAd)#\(s.adimNo)", s.puan - b))
            }
            let eslesen = farklar.count
            let kazanan = farklar.filter { $0.1 > 0 }.count
            let kaybeden = farklar.filter { $0.1 < 0 }.count
            c.append("Eşleşen adım: \(eslesen) — zincir lehine \(kazanan), aleyhine \(kaybeden), berabere \(eslesen - kazanan - kaybeden).")
            let ayrisan = farklar.filter { $0.1 != 0 }.sorted { abs($0.1) > abs($1.1) }
            if !ayrisan.isEmpty {
                c.append("")
                c.append("En çok ayrışan adımlar (zincir − bağımsız):")
                for (ad, d) in ayrisan.prefix(15) {
                    c.append("- \(ad): \(d > 0 ? "+" : "")\(d)")
                }
            }
            c.append("")
        }

        // — En düşük 15 —
        c.append("## En düşük 15 vaka")
        c.append("")
        // Kesilen turlar BURAYA GİRMEZ. Bu liste triyaj listesidir: birisi onu
        // açıp en baştaki vakayı düzeltmeye oturur. Yarıda kesilmiş turlar
        // sıralamanın tepesini kaplarsa, harcanan mesai ölçüm gürültüsüne gider.
        let enDusuk = sonuclar
            .filter { !$0.olculemedi }
            .sorted { ($0.puan, $0.vakaAd) < ($1.puan, $1.vakaAd) }
            .prefix(15)
        for s in enDusuk {
            let sorun = s.sorunlar.isEmpty ? "-" : s.sorunlar.joined(separator: "; ")
            c.append("- \(s.puan) · [\(s.kategori)/\(s.vakaAd)·\(s.mod)] \(sorun)")
            c.append("    istem : \(kirp(s.istem, 80))")
            c.append("    yanıt : \(kirp(s.yanit, 80))")
        }
        c.append("")

        // — Sorun frekansı —
        c.append("## Sorun tipleri")
        c.append("")
        var frekans: [String: Int] = [:]
        for s in sonuclar {
            for sorun in s.sorunlar {
                // "eksik-arac:calendar" → "eksik-arac" tipine indir; detay ayrıca sayılır.
                let tip = sorun.split(separator: ":", maxSplits: 1).first.map(String.init) ?? sorun
                frekans[tip, default: 0] += 1
            }
        }
        if frekans.isEmpty {
            c.append("(sorun yok)")
        } else {
            c.append("| Sorun | Adet | Vaka payı |")
            c.append("| --- | ---: | ---: |")
            for (tip, adet) in frekans.sorted(by: { ($0.value, $1.key) > ($1.value, $0.key) }) {
                let pay = Double(adet) * 100 / Double(sonuclar.count)
                c.append("| \(tip) | \(adet) | %\(bicim(pay)) |")
            }
        }
        c.append("")
        return c
    }

    // MARK: - Yardımcılar

    /// Ortalama YALNIZ ölçülebilmiş vakalar üzerinden alınır. Kesilmiş tur ne
    /// 0'dır ne 100 — bilgi yokluğudur; onu paya da paydaya da katmak ölçümü
    /// koşum ortamının yüküne bağımlı kılar.
    private static func ortalama(_ liste: [EvalSonuc]) -> Double {
        let olculen = liste.filter { !$0.olculemedi }
        guard !olculen.isEmpty else { return 0 }
        return Double(olculen.reduce(0) { $0 + $1.puan }) / Double(olculen.count)
    }

    /// Ortalamayı, arkasında kaç vakanın ölçülebildiğiyle birlikte yazar:
    /// "84.2 (n=31)". Payda görünmezse okuyucu 3 vakalık ortalamayla 60
    /// vakalıkını ayırt edemez.
    private static func ortalamaMetni(_ liste: [EvalSonuc]) -> String {
        let olculen = liste.filter { !$0.olculemedi }
        guard !olculen.isEmpty else { return "— (ölçülemedi)" }
        var metin = "\(bicim(ortalama(liste))) (n=\(olculen.count)"
        let eksik = liste.count - olculen.count
        if eksik > 0 { metin += ", \(eksik) ölçülemedi" }
        return metin + ")"
    }

    /// Sabit format — sayı biçimlendirmesi cihaz diline göre değişmesin diye
    /// Locale'e hiç dokunulmuyor (rapor makine tarafından da okunabilir olmalı).
    private static func bicim(_ d: Double) -> String {
        String(format: "%.1f", d)
    }

    /// Aynı vaka adının zincir ve tekil puanları arasındaki fark (negatif = bozulma).
    /// Kesilen turlar dışarıda: tek bir zaman aşımı, o vakayı "zincirde bozuldu"
    /// listesinin başına taşıyıp olmayan bir bağlam kusuru icat ederdi.
    private static func ortakBozulmalar(tekil: [EvalSonuc], zincir: [EvalSonuc]) -> [(String, Double)] {
        var tekilOrt: [String: [Int]] = [:]
        for s in tekil where !s.olculemedi { tekilOrt[s.vakaAd, default: []].append(s.puan) }
        var zincirOrt: [String: [Int]] = [:]
        for s in zincir where !s.olculemedi { zincirOrt[s.vakaAd, default: []].append(s.puan) }
        var sonuc: [(String, Double)] = []
        for (ad, zPuanlar) in zincirOrt {
            guard let tPuanlar = tekilOrt[ad] else { continue }
            let z = Double(zPuanlar.reduce(0, +)) / Double(zPuanlar.count)
            let t = Double(tPuanlar.reduce(0, +)) / Double(tPuanlar.count)
            let fark = z - t
            if fark < 0 { sonuc.append((ad, fark)) }
        }
        return sonuc.sorted { $0.1 < $1.1 }
    }

    private static func kirp(_ metin: String, _ sinir: Int) -> String {
        let tek = metin
            .replacingOccurrences(of: "\n", with: " ")
            .replacingOccurrences(of: "\r", with: " ")
        return tek.count <= sinir ? tek : String(tek.prefix(sinir)) + "…"
    }
}
#endif
