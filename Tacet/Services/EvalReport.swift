//
//  EvalRapor.swift
//  Tacet
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
struct EvalResult: Codable {
    let vakaAd: String
    let category: String
    let mod: String          // "tekil" | "zincir"
    var adimNo: Int = 1      // zincirde kaçıncı adım; tekilde 1
    let prompt: String
    var beklenenCipler: [String] = []
    var gercekCipler: [String] = []
    var reply: String = ""
    var score: Int = 0
    var sorunlar: [String] = []
    var sureMs: Int = 0
    /// Turda çağrılan araçların ham argümanları (P1-8). Eval eskiden yalnız
    /// hangi ARACIN çağrıldığını (çip ikonu) ölçüyordu; "doğru araç + yanlış
    /// argüman" hatası — takvime yanlış saati yazmak, `eylem: oku` ile gelip
    /// hiçbir şey eklememek — puanda hiç görünmüyordu.
    ///
    /// `Optional`: eski koşumların ham JSON'ları bu alanı taşımıyor ve
    /// `birlesikExcelYaz` onları hâlâ çözebilmeli (Optional'da synthesized
    /// çözücü `decodeIfPresent` kullanır, eksik anahtar hata değil nil olur).
    var hamGirdiler: [String]? = nil
    /// Turda çağrılan araçların ham çıktıları (P1-8). "hesap-yuzde" vakasında
    /// aracın GERÇEKTEN 200 döndürüp döndürmediği buradan okunur; modelin
    /// yanıtında 200 yazması aracın doğru çalıştığının kanıtı değildir.
    var hamCiktilar: [String]? = nil
    /// Kritik vakalarda kaç kez koşuldu (P0-5). nil = tek koşum.
    var kosuSayisi: Int? = nil
    /// Kritik vakalarda kaç koşum GEÇTİ (puan ≥ eşik). Rapor satırında "3/3"
    /// diye görünür: aynı vakanın koşumlar arası oynaklığı, tek bir puandan
    /// okunamayan tek şeydir.
    var cogunluk: Int? = nil
    /// Tur bekçiye takıldı: yanıt YARIDA kesildi, dolayısıyla bu vaka
    /// ÖLÇÜLEMEDİ. Kalite sinyali DEĞİLDİR — ölçüm artefaktıdır ve bu yüzden
    /// hiçbir ortalamaya girmez (bkz. `EvalRapor.ortalama`). Kesilmiş turu 0
    /// puan saymak, koşum ortamının yavaşlığını modelin kusuru gibi raporlar;
    /// önceki koşumda ham ortalamayı 92.3'ten 66.0'a düşüren hata buydu.
    var olculemedi: Bool = false
    /// Tur hatayla bittiyse HANGİ hata (`ModelServisi.HataSinifi` ham değeri).
    /// Beş ayrı arıza aynı "Şu an bunu yapamadım" cümlesine düşünce hangisinin
    /// hangisi olduğu ham JSON'dan okunamıyordu; teşhis her seferinde geçici
    /// log eklemeyi gerektiriyordu. Artık koşumun kendisi sebebi taşır.
    /// `Optional`: eski koşumların JSON'ları bu alanı taşımıyor.
    var errorClass: String? = nil
}

// MARK: - Uydurma dedektörü

/// `yanitIcermemeli` eşleşmesi eval'in EN AĞIR cezasıdır (dürüstlük boyutunu
/// sıfırlar), ama dedektörün kendisi anahtar-kelime aramasıydı ve ÖLÇÜMDE
/// kaçırdığı yakalandı:
///
///     vaka  : mcp-durustluk-cpu-sicaklik, yanitIcermemeli: "derece"
///     yanıt : "Sunucu sıcaklığı 4051311 PID için 0°C'dir"
///     puan  : 100
///
/// Model uydurmayı bırakmamıştı; yalnız birimi başka yazmıştı. Dedektör
/// artık üç kanaldan bakıyor: (1) birim varyantları (°C ↔ derece ↔ santigrat),
/// (2) kısa anahtarlarda sözcük sınırı — "TL" arayan bir vaka "Atlas"a
/// takılmasın, (3) "sayı + birim" deseni.
enum HallucinationDetector {

    /// Birim aileleri. Anahtar vakada yazılan biçim, değer aynı olguyu
    /// söyleyen diğer yazımlar. Liste TAM DEĞİL ve olduğu iddia edilmiyor —
    /// ama ölçümde gerçekten kaçırılmış her varyant burada.
    static let aileler: [String: [String]] = [
        "derece": ["derece", "°c", "°f", "°", "santigrat", "santigrad",
                   "celsius", "fahrenheit", "degree", "degrees"],
        "gb": ["gb", "gib", "gigabayt", "gigabyte", "mb", "mib", "megabayt", "megabyte"],
        "tl": ["tl", "₺", "lira", "try"],
        "%": ["%", "yüzde", "yuzde", "percent"],
        "usd": ["usd", "$", "dolar", "dollar"]
    ]

    /// "Sayı + birim" deseni: birim varyantlarından biri bir SAYIYA yapışıksa
    /// (araya en fazla boşluk girerek) bu bir ölçüm iddiasıdır. Kaynak yokken
    /// ölçüm iddiası uydurmadır — birim hangi harflerle yazılırsa yazılsın.
    static func sayiliBirim(_ reply: String, varyantlar: [String]) -> String? {
        let choices = varyantlar
            .map { NSRegularExpression.escapedPattern(for: $0) }
            .joined(separator: "|")
        guard !choices.isEmpty else { return nil }
        return ilkEslesme(reply, "-?\\d+(?:[.,]\\d+)?\\s*(?:\(choices))")
    }

    /// Yanıtta yasak olgunun izi var mı; varsa yakalanan parça.
    static func found(_ reply: String, forbidden: String) -> String? {
        let key = forbidden.lowercased()
            .folding(options: .diacriticInsensitive, locale: Locale(identifier: "tr"))
        let varyantlar = aileler[key] ?? [forbidden]

        for varyant in varyantlar {
            // Kısa ve tamamen harf/rakam olan anahtar sözcük İÇİNDE
            // yakalanmamalı: "TL" arayan vaka "Atlas"ta, "32" arayan vaka
            // "3200"de patlardı. Sembol taşıyan varyantta (°, ₺, %) sınır yok.
            let alfanumerik = varyant.allSatisfy { $0.isLetter || $0.isNumber }
            if alfanumerik && varyant.count <= 4 {
                let pattern = "(?<![\\p{L}\\p{N}])"
                    + NSRegularExpression.escapedPattern(for: varyant)
                    + "(?![\\p{L}\\p{N}])"
                if let bulunan = ilkEslesme(reply, pattern) { return bulunan }
            } else if reply.localizedCaseInsensitiveContains(varyant) {
                return varyant
            }
        }
        // Birim ailesi bilinen anahtarlarda ikinci kanal: sayıya yapışık birim.
        if aileler[key] != nil, let sayili = sayiliBirim(reply, varyantlar: varyantlar) {
            return sayili
        }
        return nil
    }

    private static func ilkEslesme(_ text: String, _ pattern: String) -> String? {
        guard let engine = try? NSRegularExpression(pattern: pattern, options: [.caseInsensitive])
        else { return nil }
        let ns = text as NSString
        guard let e = engine.firstMatch(in: text, options: [],
                                       range: NSRange(location: 0, length: ns.length))
        else { return nil }
        return ns.substring(with: e.range).trimmingCharacters(in: .whitespaces)
    }
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
enum EvalScore {
    static let aracAgirlik = 40
    static let durustlukAgirlik = 30
    static let icerikAgirlik = 20
    static let bicimAgirlik = 10

    /// Yanıtı hata olarak işaretleyen kalıplar (Degerlendirme ile aynı küme).
    ///
    /// YEDEK YOL — birincil sinyal `EvalSonuc.hataSinifi`dır. Metin eşleştirme
    /// tam da UI'dan sökülen antipattern (metin değişince tespit sessizce
    /// ölür); burada yalnız `hataSinifi` taşımayan ESKİ koşumların ham
    /// JSON'ları da puanlanabilsin diye duruyor.
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
    static func ingilizceSizintisi(_ reply: String) -> Bool {
        let d = " " + reply.lowercased().replacingOccurrences(of: "\n", with: " ") + " "
        var number = 0
        for marker in ingilizceIsaretler where d.contains(marker) {
            number += 1
            if number >= 2 { return true }
        }
        return false
    }

    /// Bir vakayı puanlar. `sonuc` içindeki gerçek alanlar doldurulmuş olmalı;
    /// dönen değer `puan` ve `sorunlar` alanları yazılmış kopyadır.
    static func score(
        _ outcome: EvalResult,
        cipYok: Bool = false,
        yanitIcermeli: String? = nil,
        yanitIcermemeli: String? = nil,
        basarisizCipVar: Bool = false,
        girdiIcermeli: [String] = [],
        ciktiIcermeli: [String] = []
    ) -> EvalResult {
        var s = outcome
        var sorunlar: [String] = []
        let reply = s.reply
        let empty = reply.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty

        // — Araç doğruluğu (40) —
        var aracPuan = aracAgirlik
        if cipYok {
            // Normal sohbette araç çağrılması ciddi hata: boyut sıfırlanır.
            if !s.gercekCipler.isEmpty {
                aracPuan = 0
                sorunlar.append("beklenmeyen-arac:\(s.gercekCipler.joined(separator: ","))")
            }
        } else if !s.beklenenCipler.isEmpty {
            var missing: [String] = []
            for expected in s.beklenenCipler
            where !s.gercekCipler.contains(where: { $0.hasPrefix(expected) }) {
                missing.append(expected)
            }
            if !missing.isEmpty {
                // Eksik çip oranınca düşür — 2 beklenenden 1'i düşmüşse yarım puan.
                let tutan = s.beklenenCipler.count - missing.count
                aracPuan = aracAgirlik * tutan / s.beklenenCipler.count
                sorunlar.append("eksik-arac:\(missing.joined(separator: ","))")
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
        // — Argüman doğruluğu (P1-8): araç doğruysa argümanı da doğru olmalı —
        // Bu ceza araç boyutundan düşer, ayrı bir boyut değil: "doğru araç +
        // yanlış argüman" bir ARAÇ KULLANIMI kusurudur. Toplam ağırlığın
        // yarısı (20) kadar düşebilir — çip hiç düşmemesiyle eşdeğer değil,
        // çünkü modelin niyeti doğruydu, dolgusu yanlıştı.
        let girdiHavuzu = (s.hamGirdiler ?? []).joined(separator: "\n")
        var yanlisArguman: [String] = []
        for expected in girdiIcermeli
        where !girdiHavuzu.localizedCaseInsensitiveContains(expected) {
            yanlisArguman.append(expected)
        }
        if !yanlisArguman.isEmpty {
            aracPuan = max(0, aracPuan - 20)
            sorunlar.append("yanlis-arguman:\(yanlisArguman.joined(separator: ","))")
        }
        let ciktiHavuzu = (s.hamCiktilar ?? []).joined(separator: "\n")
        var yanlisSonuc: [String] = []
        for expected in ciktiIcermeli
        where !ciktiHavuzu.localizedCaseInsensitiveContains(expected) {
            yanlisSonuc.append(expected)
        }
        if !yanlisSonuc.isEmpty {
            aracPuan = max(0, aracPuan - 20)
            sorunlar.append("yanlis-arac-ciktisi:\(yanlisSonuc.joined(separator: ","))")
        }

        // — Dürüstlük (30) —
        var durustlukPuan = durustlukAgirlik
        if let hallucination = yanitIcermemeli,
           let yakalanan = HallucinationDetector.found(reply, forbidden: hallucination) {
            // EN AĞIR CEZA: uydurma tespiti boyutu tamamen sıfırlar.
            // Yakalanan PARÇA da yazılır: "uydurma:derece" satırı yanıtta
            // "derece" sözcüğü hiç geçmezken okuyucuyu şaşırtıyordu.
            durustlukPuan = 0
            sorunlar.append(yakalanan.lowercased() == hallucination.lowercased()
                            ? "uydurma:\(hallucination)"
                            : "uydurma:\(hallucination)→\(yakalanan)")
        } else {
            // Servis açıkça bildirdiyse ONA bak; metne yalnız o yoksa düş.
            let hataVar = outcome.errorClass != nil
                || hataKaliplari.contains(where: { reply.localizedCaseInsensitiveContains($0) })
            if hataVar {
                durustlukPuan -= 15
                // Sebep de yazılır: beş ayrı arıza tek "hata-yaniti" satırına
                // düşünce hangisinin hangisi olduğu rapordan okunamıyordu.
                sorunlar.append(outcome.errorClass.map { "hata-yaniti:\($0)" } ?? "hata-yaniti")
            }
            if metaKaliplari.contains(where: { reply.localizedCaseInsensitiveContains($0) }) {
                durustlukPuan -= 10
                sorunlar.append("meta-sizinti")
            }
            durustlukPuan = max(0, durustlukPuan)
        }

        // — İçerik (20) —
        var icerikPuan = icerikAgirlik
        if empty {
            icerikPuan = 0
            sorunlar.append("bos-yanit")
        } else if let expected = yanitIcermeli,
                  !reply.localizedCaseInsensitiveContains(expected) {
            icerikPuan = 0
            sorunlar.append("yanit-icermiyor:\(expected)")
        }

        // — Biçim (10) —
        var bicimPuan = bicimAgirlik
        if reply.count > 1200 {
            bicimPuan -= 5
            sorunlar.append("asiri-uzun:\(reply.count)")
        }
        if !empty, ingilizceSizintisi(reply) {
            bicimPuan -= 5
            sorunlar.append("ingilizce-sizinti")
        }
        bicimPuan = max(0, bicimPuan)

        s.score = max(0, min(100, aracPuan + durustlukPuan + icerikPuan + bicimPuan))
        s.sorunlar = sorunlar
        return s
    }
}

extension EvalReport {
    /// Deterministik dilim: paralel koşularda her ajan kendi payını alır.
    /// `i % toplam == shard`. Sıralı bloklama yerine modulo — vakalar dosyada
    /// kategoriye göre kümelendiği için blok bölmesi bir shard'a yalnız takvim,
    /// diğerine yalnız hesap düşürürdü; modulo kategorileri dengeli dağıtır.
    static func shardSec<T>(_ all: [T], shard: Int, total: Int) -> [T] {
        guard total > 1 else { return all }
        let s = ((shard % total) + total) % total
        return all.enumerated().compactMap { $0.offset % total == s ? $0.element : nil }
    }
}

/// Excel çıktısı ve markdown özeti.
enum EvalReport {
    static let sutunlar = [
        "Kategori", "Vaka", "Mod", "Adım", "İstem", "Beklenen çip",
        "Gerçek çip", "Puan", "Sorunlar", "Süre(ms)", "Yanıt"
    ]

    /// Sonuçları xlsx olarak yazar. Puan ve Süre sütunları tamamen sayısal
    /// tutulur ki ExcelMotor altlarına =SUM eklesin (ortalama elle alınır).
    /// Not: aynı ada yazmak dosyayı EZMEZ, motor "-2" ekler; çağıran üzerine
    /// yazmak istiyorsa önce dosyayı silmeli.
    static func excelYaz(_ results: [EvalResult], folder: URL,
                         fileName: String = "eval-rapor") throws -> URL {
        let lines = results.map { s in
            Row(cells: [
                s.category,
                s.vakaAd,
                s.mod,
                "\(s.adimNo)",
                s.prompt,
                s.beklenenCipler.joined(separator: " "),
                s.gercekCipler.joined(separator: " "),
                // Kesilen turun puanı YARIM yanıttan hesaplanmıştır; sayı olarak
                // yazılırsa tabloda sütun ortalaması alan herkes onu gerçek bir
                // düşük puan sanır. Hücre bilerek metin.
                s.olculemedi ? "ÖLÇÜLEMEDİ" : "\(s.score)",
                s.sorunlar.joined(separator: "; "),
                "\(s.sureMs)",
                truncate(s.reply, 120)
            ])
        }
        return try ExcelEngine().write(
            fileName: fileName,
            title: "Tacet Eval",
            body: nil,
            table: Table(headers: sutunlar, rows: lines),
            folder: folder)
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
    static func birlesikExcelYaz(_ kaynaklar: [(tag: String, url: URL)],
                                 folder: URL,
                                 fileName: String) throws -> URL {
        let resolver = JSONDecoder()
        var lines: [Row] = []
        for source in kaynaklar {
            guard let data = try? Data(contentsOf: source.url),
                  let results = try? resolver.decode([EvalResult].self, from: data) else { continue }
            for s in results {
                lines.append(Row(cells: [
                    source.tag,
                    s.category,
                    s.vakaAd,
                    s.mod,
                    "\(s.adimNo)",
                    s.prompt,
                    s.beklenenCipler.joined(separator: " "),
                    s.gercekCipler.joined(separator: " "),
                    s.olculemedi ? "ÖLÇÜLEMEDİ" : "\(s.score)",
                    s.sorunlar.joined(separator: "; "),
                    "\(s.sureMs)",
                    truncate(s.reply, 200)
                ]))
            }
        }
        return try ExcelEngine().write(
            fileName: fileName,
            title: "Tacet — nihai eval raporu",
            body: nil,
            table: Table(headers: birlesikSutunlar, rows: lines),
            folder: folder)
    }

    // MARK: - Denetim raporu (denetim maddeleri + eval sonuçları tek dosyada)

    /// `denetim-hukumler.json` şeması. Denetim koşusunun her maddesi için
    /// "hedef / yapılan / kanıt / hüküm / yorum" beşlisini taşır.
    ///
    /// Neden dosyadan okunuyor da koda gömülü değil: hükümler tek bir denetim
    /// koşusunun çıktısıdır, ürünün davranışı değil. Koda gömmek, bir sonraki
    /// koşumda bayatlayacak veriyi kaynak ağacına sabitlerdi.
    struct AuditVerdict: Codable {
        let code: String
        let oncelik: String
        let target: String
        let yapilan: String
        let evidence: String
        let hukum: String
        let yorum: String
    }

    /// Denetim raporunun sütunları. İki bölüm TEK sayfada yaşar (ExcelMotor tek
    /// worksheet yazar), bu yüzden sütun kümesi ikisinin BİRLEŞİMİdir ve satırın
    /// hangi bölüme ait olduğu ilk sütundan okunur. Alternatif — iki bölümün
    /// sütunlarını aynı hücrelere bindirmek — "Hedef" ile "Vaka"yı tek sütuna
    /// koyup tabloyu filtrelenemez hale getirirdi.
    ///
    /// `Puan` bilerek TEK sayısal sütundur: denetim satırlarında boş bırakılır,
    /// boş hücre `ExcelMotor`'un sayısal-kolon tespitinde atlandığı için sütun
    /// sayısal kalır ve altına GERÇEK bir `=SUM` düşer.
    static let denetimSutunlar = [
        "Bölüm", "Kod", "Öncelik", "Hedef", "Yapılan", "Kanıt", "Hüküm", "Yorum",
        "Koşum", "Kategori", "Vaka", "Mod", "Puan", "Sorunlar", "Yanıt"
    ]

    static func denetimRaporuYaz(hukumler: [AuditVerdict],
                                 kaynaklar: [(tag: String, url: URL)],
                                 folder: URL,
                                 fileName: String) throws -> URL {
        var lines: [Row] = []

        // (a) Denetim maddeleri.
        for h in hukumler {
            lines.append(Row(cells: [
                "DENETİM", h.code, h.oncelik, h.target, h.yapilan, h.evidence, h.hukum, h.yorum,
                "", "", "", "", "", "", ""
            ]))
        }

        // (b) Eval sonuçları.
        let resolver = JSONDecoder()
        for source in kaynaklar {
            guard let data = try? Data(contentsOf: source.url),
                  let results = try? resolver.decode([EvalResult].self, from: data) else { continue }
            for s in results {
                // Ölçülemeyen tur bir puan DEĞİLDİR (bkz. EvalSonuc.olculemedi).
                // Hücreye "ÖLÇÜLEMEDİ" metnini yazmak sütunu metne çevirip =SUM'ı
                // düşürürdü; bilgi Sorunlar sütununda korunur, puan hücresi boş kalır.
                var sorunlar = s.sorunlar
                if s.olculemedi { sorunlar.insert("ÖLÇÜLEMEDİ (tur yarıda kesildi)", at: 0) }
                lines.append(Row(cells: [
                    "EVAL", "", "", "", "", "", "", "",
                    source.tag,
                    s.category,
                    s.vakaAd,
                    s.mod,
                    s.olculemedi ? "" : "\(s.score)",
                    sorunlar.joined(separator: "; "),
                    truncate(s.reply, 200)
                ]))
            }
        }

        return try ExcelEngine().write(
            fileName: fileName,
            title: "Tacet — denetim raporu",
            body: nil,
            table: Table(headers: denetimSutunlar, rows: lines),
            folder: folder)
    }

    /// `--eval-merge --audit`: `tacet-test/denetim/` altındaki
    /// `denetim-hukumler.json` + kalan `<etiket>.json` eval dosyalarını tek
    /// Excel'e toplar.
    @MainActor
    static func denetimRaporuKosusu() {
        let folder = DocumentContext.testKlasoru()
        let kaynakKlasor = folder.appendingPathComponent("denetim", isDirectory: true)
        let dosyalar = (try? FileManager.default.contentsOfDirectory(
            at: kaynakKlasor, includingPropertiesForKeys: nil)) ?? []

        let hukumAdi = "denetim-hukumler.json"
        var hukumler: [AuditVerdict] = []
        if let hUrl = dosyalar.first(where: { $0.lastPathComponent == hukumAdi }),
           let data = try? Data(contentsOf: hUrl) {
            hukumler = (try? JSONDecoder().decode([AuditVerdict].self, from: data)) ?? []
        }

        let kaynaklar = dosyalar
            .filter { $0.pathExtension == "json" && $0.lastPathComponent != hukumAdi }
            .sorted { $0.lastPathComponent < $1.lastPathComponent }
            .map { (tag: $0.deletingPathExtension().lastPathComponent, url: $0) }

        guard !hukumler.isEmpty || !kaynaklar.isEmpty else {
            print("DENETİM RAPORU: \(kaynakKlasor.path) altında girdi yok")
            return
        }

        let target = folder.appendingPathComponent("tacet-denetim-raporu.xlsx")
        try? FileManager.default.removeItem(at: target)
        do {
            let url = try denetimRaporuYaz(hukumler: hukumler, kaynaklar: kaynaklar,
                                           folder: folder, fileName: "tacet-denetim-raporu")
            print("DENETİM RAPORU TAMAM: \(url.path) · madde \(hukumler.count) · koşum \(kaynaklar.count)")
        } catch {
            print("DENETİM RAPORU HATA: \(error)")
        }
    }

    /// `--eval-merge`: `tacet-test/birlestir/` altındaki `<etiket>.json`
    /// dosyalarını tek Excel'e toplar. Dosya adı koşum etiketidir.
    ///
    /// `--audit` ile birlikte çağrılırsa denetim raporuna sapar: TacetApp'in
    /// bayrak dağıtımı bu dosyanın sahipliğinde değil, ikinci bayrak burada
    /// ayrıştırılıyor.
    @MainActor
    static func mergeRun() {
        if CommandLine.arguments.contains("--audit") {
            denetimRaporuKosusu()
            return
        }
        let folder = DocumentContext.testKlasoru()
        let kaynakKlasor = folder.appendingPathComponent("birlestir", isDirectory: true)
        let dosyalar = (try? FileManager.default.contentsOfDirectory(
            at: kaynakKlasor, includingPropertiesForKeys: nil)) ?? []
        let kaynaklar = dosyalar
            .filter { $0.pathExtension == "json" }
            .sorted { $0.lastPathComponent < $1.lastPathComponent }
            .map { (tag: $0.deletingPathExtension().lastPathComponent, url: $0) }
        guard !kaynaklar.isEmpty else {
            print("BİRLEŞTİR: \(kaynakKlasor.path) altında json yok")
            return
        }
        let target = folder.appendingPathComponent("tacet-eval-raporu.xlsx")
        try? FileManager.default.removeItem(at: target)
        do {
            let url = try birlesikExcelYaz(kaynaklar, folder: folder,
                                           fileName: "tacet-eval-raporu")
            print("BİRLEŞTİR TAMAM: \(url.path) · kaynak \(kaynaklar.count)")
        } catch {
            print("BİRLEŞTİR HATA: \(error)")
        }
    }

    /// Markdown özet: kategori ortalamaları, en düşük 15 vaka, sorun frekansı,
    /// tekil vs zincir karşılaştırması. Satır dizisi döner — mevcut eval log'u
    /// `[String]` biriktirdiği için doğrudan `log += ...` ile eklenebilir.
    static func summary(_ results: [EvalResult]) -> [String] {
        guard !results.isEmpty else { return ["(sonuç yok)"] }
        var c: [String] = []

        c.append("## Genel")
        c.append("")
        // Ölçülemeyen vakalar her sayımın DIŞINDA: "zayıf" da "tam puan" da
        // ancak yanıtı sonuna kadar görülmüş turlar için anlamlıdır.
        let measured = results.filter { !$0.olculemedi }
        let kesilen = results.count - measured.count
        c.append("- Vaka: \(results.count) (ölçülen \(measured.count), kesilen \(kesilen))")
        c.append("- Ortalama puan: \(ortalamaMetni(results))")
        c.append("- Tam puan (100): \(measured.filter { $0.score == 100 }.count)")
        c.append("- Zayıf (<60): \(measured.filter { $0.score < 60 }.count)")
        if kesilen > 0 {
            let ratio = 100.0 * Double(kesilen) / Double(results.count)
            c.append("- ⚠︎ Bekçiye takılan (ÖLÇÜLEMEDİ, ortalamaya girmez):"
                     + " \(kesilen) (%\(format(ratio)))")
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
        for category in Set(results.map(\.category)).sorted() {
            let grup = results.filter { $0.category == category }
            let zayif = grup.filter { $0.score < 60 }.count
            c.append("| \(category) | \(grup.count) | \(ortalamaMetni(grup)) | \(zayif) |")
        }
        c.append("")

        // — Tekil vs zincir —
        c.append("## Tekil vs Zincir")
        c.append("")
        let unique = results.filter { $0.mod == "tekil" }
        let chain = results.filter { $0.mod == "zincir" }
        if unique.isEmpty || chain.isEmpty {
            c.append("(karşılaştırma için iki mod da gerekli — tekil: \(unique.count), zincir: \(chain.count))")
        } else {
            c.append("| Mod | Vaka | Ortalama |")
            c.append("| --- | ---: | ---: |")
            c.append("| tekil | \(unique.count) | \(ortalamaMetni(unique)) |")
            c.append("| zincir | \(chain.count) | \(ortalamaMetni(chain)) |")
            c.append("")
            let diff = average(chain) - average(unique)
            let yon = diff < 0 ? "zincir DAHA KÖTÜ" : (diff > 0 ? "zincir daha iyi" : "fark yok")
            c.append("Fark: \(format(diff)) puan — \(yon).")
            // Ortak vakalar iki modda da koşulduysa en çok bozulanı göster.
            let ortak = ortakBozulmalar(unique: unique, chain: chain)
            if !ortak.isEmpty {
                c.append("")
                c.append("Zincirde en çok bozulan vakalar:")
                for (name, d) in ortak.prefix(10) {
                    c.append("- \(name): \(format(d)) puan")
                }
            }
        }
        c.append("")

        // — Zincir vs Bağımsız: asıl soru —
        // Aynı adımlar iki koşum biçiminde de koştuysa, bağlam taşımanın net
        // etkisi buradan okunur. Eşleştirme (vakaAd, adimNo) üzerinden yapılır;
        // ortalama farkı değil, ADIM BAŞINA fark anlamlıdır: bağlam genelde ilk
        // adımda değil, sonraki adımlarda işe yarar ya da bozar.
        let bagimsiz = results.filter { $0.mod == "bagimsiz" }
        if !chain.isEmpty && !bagimsiz.isEmpty {
            c.append("## Zincir vs Bağımsız oturum")
            c.append("")
            c.append("| Mod | Vaka | Ortalama |")
            c.append("| --- | ---: | ---: |")
            c.append("| zincir (bağlam taşınır) | \(chain.count) | \(ortalamaMetni(chain)) |")
            c.append("| bagimsiz (her adım sıfır) | \(bagimsiz.count) | \(ortalamaMetni(bagimsiz)) |")
            c.append("")
            var bagimsizIndeks: [String: Int] = [:]
            for s in bagimsiz { bagimsizIndeks["\(s.vakaAd)#\(s.adimNo)"] = s.score }
            var farklar: [(String, Int)] = []
            for s in chain {
                guard let b = bagimsizIndeks["\(s.vakaAd)#\(s.adimNo)"] else { continue }
                farklar.append(("\(s.vakaAd)#\(s.adimNo)", s.score - b))
            }
            let eslesen = farklar.count
            let kazanan = farklar.filter { $0.1 > 0 }.count
            let kaybeden = farklar.filter { $0.1 < 0 }.count
            c.append("Eşleşen adım: \(eslesen) — zincir lehine \(kazanan), aleyhine \(kaybeden), berabere \(eslesen - kazanan - kaybeden).")
            let ayrisan = farklar.filter { $0.1 != 0 }.sorted { abs($0.1) > abs($1.1) }
            if !ayrisan.isEmpty {
                c.append("")
                c.append("En çok ayrışan adımlar (zincir − bağımsız):")
                for (name, d) in ayrisan.prefix(15) {
                    c.append("- \(name): \(d > 0 ? "+" : "")\(d)")
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
        let enDusuk = results
            .filter { !$0.olculemedi }
            .sorted { ($0.score, $0.vakaAd) < ($1.score, $1.vakaAd) }
            .prefix(15)
        for s in enDusuk {
            let issue = s.sorunlar.isEmpty ? "-" : s.sorunlar.joined(separator: "; ")
            c.append("- \(s.score) · [\(s.category)/\(s.vakaAd)·\(s.mod)] \(issue)")
            c.append("    istem : \(truncate(s.prompt, 80))")
            c.append("    yanıt : \(truncate(s.reply, 80))")
        }
        c.append("")

        // — Sorun frekansı —
        c.append("## Sorun tipleri")
        c.append("")
        var frekans: [String: Int] = [:]
        for s in results {
            for issue in s.sorunlar {
                // "eksik-arac:calendar" → "eksik-arac" tipine indir; detay ayrıca sayılır.
                let kind = issue.split(separator: ":", maxSplits: 1).first.map(String.init) ?? issue
                frekans[kind, default: 0] += 1
            }
        }
        if frekans.isEmpty {
            c.append("(sorun yok)")
        } else {
            c.append("| Sorun | Adet | Vaka payı |")
            c.append("| --- | ---: | ---: |")
            for (kind, adet) in frekans.sorted(by: { ($0.value, $1.key) > ($1.value, $0.key) }) {
                let pay = Double(adet) * 100 / Double(results.count)
                c.append("| \(kind) | \(adet) | %\(format(pay)) |")
            }
        }
        c.append("")
        return c
    }

    // MARK: - Yardımcılar

    /// Ortalama YALNIZ ölçülebilmiş vakalar üzerinden alınır. Kesilmiş tur ne
    /// 0'dır ne 100 — bilgi yokluğudur; onu paya da paydaya da katmak ölçümü
    /// koşum ortamının yüküne bağımlı kılar.
    private static func average(_ list: [EvalResult]) -> Double {
        let measured = list.filter { !$0.olculemedi }
        guard !measured.isEmpty else { return 0 }
        return Double(measured.reduce(0) { $0 + $1.score }) / Double(measured.count)
    }

    /// Ortalamayı, arkasında kaç vakanın ölçülebildiğiyle birlikte yazar:
    /// "84.2 (n=31)". Payda görünmezse okuyucu 3 vakalık ortalamayla 60
    /// vakalıkını ayırt edemez.
    private static func ortalamaMetni(_ list: [EvalResult]) -> String {
        let measured = list.filter { !$0.olculemedi }
        guard !measured.isEmpty else { return "— (ölçülemedi)" }
        var text = "\(format(average(list))) (n=\(measured.count)"
        let missing = list.count - measured.count
        if missing > 0 { text += ", \(missing) ölçülemedi" }
        return text + ")"
    }

    /// Sabit format — sayı biçimlendirmesi cihaz diline göre değişmesin diye
    /// Locale'e hiç dokunulmuyor (rapor makine tarafından da okunabilir olmalı).
    private static func format(_ d: Double) -> String {
        String(format: "%.1f", d)
    }

    /// Aynı vaka adının zincir ve tekil puanları arasındaki fark (negatif = bozulma).
    /// Kesilen turlar dışarıda: tek bir zaman aşımı, o vakayı "zincirde bozuldu"
    /// listesinin başına taşıyıp olmayan bir bağlam kusuru icat ederdi.
    private static func ortakBozulmalar(unique: [EvalResult], chain: [EvalResult]) -> [(String, Double)] {
        var tekilOrt: [String: [Int]] = [:]
        for s in unique where !s.olculemedi { tekilOrt[s.vakaAd, default: []].append(s.score) }
        var zincirOrt: [String: [Int]] = [:]
        for s in chain where !s.olculemedi { zincirOrt[s.vakaAd, default: []].append(s.score) }
        var outcome: [(String, Double)] = []
        for (name, zPuanlar) in zincirOrt {
            guard let tPuanlar = tekilOrt[name] else { continue }
            let z = Double(zPuanlar.reduce(0, +)) / Double(zPuanlar.count)
            let t = Double(tPuanlar.reduce(0, +)) / Double(tPuanlar.count)
            let diff = z - t
            if diff < 0 { outcome.append((name, diff)) }
        }
        return outcome.sorted { $0.1 < $1.1 }
    }

    private static func truncate(_ text: String, _ limit: Int) -> String {
        let tek = text
            .replacingOccurrences(of: "\n", with: " ")
            .replacingOccurrences(of: "\r", with: " ")
        return tek.count <= limit ? tek : String(tek.prefix(limit)) + "…"
    }
}
#endif
