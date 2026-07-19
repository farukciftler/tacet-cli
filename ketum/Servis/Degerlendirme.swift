//
//  Degerlendirme.swift
//  ketum
//
//  Otomatik değerlendirme (eval) — gerçek on-device model üzerinde tüm beceriler
//  ve normal sohbet için test vakaları koşar; doğru araç seçimi, yanıt kalitesi
//  ve hata olup olmadığını ölçer. "--test" argümanıyla açılır (yalnızca DEBUG).
//  Sonuçları Caches/sirr-test/test-sonuc.txt'e artımlı yazar — çıktılar gerçek
//  takvim/kişi verisi içerdiği için kullanıcının belge klasöründen ayrı tutulur.
//

#if DEBUG
import Foundation
import SwiftData

struct TestVaka {
    let ad: String
    let istem: String
    var ikonlar: [String] = []      // beklenen çip ikon önekleri (hepsi bulunmalı)
    var cipYok = false              // normal sohbet: hiç araç çağrılmamalı
    var ekliBelge = false           // önce test belgesi ekle (oku/düzenle vakaları)
    var yanitIcermeli: String? = nil
    var yanitIcermemeli: String? = nil   // uydurma tespiti (ör. "Paris" dememeli)
}

@MainActor
enum Degerlendirme {
    static func calistir() {
        Task { await kosu() }
    }

    static func vakalar() -> [TestVaka] {
        [
            // — Normal sohbet (araç yok) —
            TestVaka(ad: "selam", istem: "Merhaba", cipYok: true),
            TestVaka(ad: "nasilsin", istem: "Nasılsın?", cipYok: true),
            TestVaka(ad: "kimsin", istem: "Sen kimsin?", cipYok: true),
            // Hava/dünya bilgisi: araç çağırsa da çağırmasa da, cevabı UYDURMAMALI (sınırını söylemeli).
            TestVaka(ad: "hava", istem: "Bugün hava nasıl olacak?", yanitIcermemeli: "derece"),
            TestVaka(ad: "dunya-bilgi", istem: "Fransa'nın başkenti neresi?", yanitIcermemeli: "Paris"),

            // — Hesap —
            TestVaka(ad: "hesap-carpma", istem: "125 çarpı 8 kaç eder?", ikonlar: ["function"]),
            TestVaka(ad: "hesap-toplam", istem: "Üç ürün aldım, her biri 45 lira, toplam ne kadar?", ikonlar: ["function"]),
            TestVaka(ad: "hesap-yuzde", istem: "250 liranın yüzde 20 indirimlisi kaç lira?", ikonlar: ["function"]),

            // — Zaman (çip yok; yanıt bir saat/gün içermeli) —
            TestVaka(ad: "zaman-saat", istem: "Saat kaç?", yanitIcermeli: ":"),
            TestVaka(ad: "zaman-gun", istem: "Bugün günlerden ne?", yanitIcermeli: suGun()),

            // — Takvim —
            TestVaka(ad: "takvim-oku", istem: "Yarın neler var?", ikonlar: ["calendar"]),
            TestVaka(ad: "takvim-hafta", istem: "Bu hafta programım ne?", ikonlar: ["calendar"]),
            TestVaka(ad: "takvim-ekle", istem: "Cuma saat 14:00'te toplantı ekle", ikonlar: ["calendar"]),

            // — Hatırlatıcı —
            TestVaka(ad: "hatirlatici-1", istem: "Beni 18:00'de aramam için hatırlat", ikonlar: ["bell"]),
            TestVaka(ad: "hatirlatici-2", istem: "Yarın ekmek almayı hatırlat", ikonlar: ["bell"]),

            // — Kişi —
            TestVaka(ad: "kisi-numara", istem: "Ahmet'in telefon numarası ne?", ikonlar: ["person"]),
            TestVaka(ad: "kisi-mail", istem: "Mehmet'in e-posta adresini bul", ikonlar: ["person"]),

            // — Arama —
            TestVaka(ad: "arama-not", istem: "Notlarımda toplantı ile ilgili ne var?", ikonlar: ["magnifyingglass"]),
            TestVaka(ad: "arama-bul", istem: "Geçen haftaki alışveriş notumu bul", ikonlar: ["magnifyingglass"]),

            // — Belge oluşturma —
            TestVaka(ad: "belge-excel", istem: "Haftalık yemek listesi için bir excel yap", ikonlar: ["tablecells"]),
            TestVaka(ad: "belge-pdf", istem: "Kısa bir tanıtım metnini pdf yap", ikonlar: ["doc"]),
            TestVaka(ad: "belge-word", istem: "Alışveriş listemi word belgesi olarak oluştur", ikonlar: ["doc"]),

            // — Belge okuma/düzenleme (ekli belge ile) —
            TestVaka(ad: "belge-oku", istem: "Bu belgede ne var, özetle", ikonlar: ["tablecells"], ekliBelge: true),
            TestVaka(ad: "belge-duzenle", istem: "Bu tabloya yeni bir satır ekle: Cumartesi, Pizza", ikonlar: ["tablecells"], ekliBelge: true),

            // — Zincir: cihaz verisi → dosya (bağlam bütçesi) —
            TestVaka(ad: "zincir-takvim-excel", istem: "Bu haftaki etkinliklerimi excel'e dök", ikonlar: ["calendar", "tablecells"]),
        ]
    }

    static func kosu() async {
        let servis = ModelServisi()
        let klasor = BelgeBaglami.testKlasoru()
        let sonucURL = klasor.appendingPathComponent("test-sonuc.txt")

        // Model hazır değilse çık.
        guard servis.durum.hazirMi else {
            try? "MODEL HAZIR DEĞİL: \(servis.durum.etiket)".write(to: sonucURL, atomically: true, encoding: .utf8)
            return
        }

        // Oku/düzenle vakaları için test xlsx üret.
        let testBelge = try? ExcelMotor().yaz(
            dosyaAdi: "test-girdi", baslik: "Test",
            govde: nil,
            tablo: Tablo(basliklar: ["Gün", "Yemek"],
                         satirlar: [Satir(hucreler: ["Pazartesi", "Mercimek"]),
                                    Satir(hucreler: ["Salı", "Tavuk"])]),
            klasor: klasor)

        let hepsi = vakalar()
        var log: [String] = ["=== KETUM EVAL — \(hepsi.count) vaka ===", ""]
        var gecen = 0

        for (i, v) in hepsi.enumerated() {
            servis.sohbetiSifirla()
            if v.ekliBelge, let testBelge { servis.belgeBaglami.belgeEkle(url: testBelge) }

            let (metin, izler) = await servis.yanitla(v.istem) { _ in }
            let ikonlar = izler.map(\.ikon)
            var sorunlar: [String] = []

            // Hata yanıtı?
            let hataIzleri = ["yapamadım", "hazır değil", "sorun oldu"]
            if hataIzleri.contains(where: { metin.localizedCaseInsensitiveContains($0) }) {
                sorunlar.append("hata-yaniti")
            }
            // Meta sızıntısı?
            if metin.localizedCaseInsensitiveContains("önizle") || metin.localizedCaseInsensitiveContains("paylaşabilir") {
                sorunlar.append("meta-sizinti")
            }
            // Başarısız çip?
            if izler.contains(where: { if case .basarisiz = $0.durum { return true }; return false }) {
                sorunlar.append("basarisiz-cip")
            }
            // Beklenen araçlar?
            for beklenen in v.ikonlar where !ikonlar.contains(where: { $0.hasPrefix(beklenen) }) {
                sorunlar.append("eksik-arac:\(beklenen)")
            }
            // Normal sohbette araç olmamalı.
            if v.cipYok && !izler.isEmpty {
                sorunlar.append("beklenmeyen-arac:\(ikonlar)")
            }
            // Yanıt beklenen metni içermeli?
            if let ic = v.yanitIcermeli, !metin.localizedCaseInsensitiveContains(ic) {
                sorunlar.append("yanit-icermiyor:\(ic)")
            }
            if let ic = v.yanitIcermemeli, metin.localizedCaseInsensitiveContains(ic) {
                sorunlar.append("uydurma:\(ic)")
            }

            let ok = sorunlar.isEmpty
            if ok { gecen += 1 }
            let kisaYanit = metin.replacingOccurrences(of: "\n", with: " ").prefix(70)
            log.append("\(ok ? "✓" : "✗") [\(v.ad)] '\(v.istem)'")
            log.append("    çip:\(ikonlar) yanıt:\"\(kisaYanit)\"")
            if !ok { log.append("    ⚠︎ \(sorunlar.joined(separator: "; "))") }
            log.append("")

            // Artımlı yaz (koşu yarıda kalırsa da sonuç görünür).
            let ara = (["=== \(gecen)/\(i + 1) GEÇTİ (devam ediyor) ==="] + log.dropFirst()).joined(separator: "\n")
            try? ara.write(to: sonucURL, atomically: true, encoding: .utf8)
        }

        var sayac = Sayac(gecen: gecen, toplam: hepsi.count)

        // — Spec vakaları: hafıza / seyir / web araması —
        // Her blok kendi başlığını ve vaka başına BEKLENEN/GERÇEK satırını yazar;
        // "gözle bak" satırı yoktur. Bloklar birbirini etkilemesin diye her biri
        // kendi kurulumunu yapıp geri alır.
        for blok in 0..<3 {
            let sonuc: Sayac
            switch blok {
            case 0: sonuc = await hafizaKosusu(servis)
            case 1: sonuc = await seyirKosusu(servis)
            default: sonuc = await webKosusu(servis)
            }
            sayac.birlestir(sonuc)
            log += sonuc.satirlar
            try? (["=== \(sayac.gecen)/\(sayac.toplam) GEÇTİ (devam ediyor) ==="] + log.dropFirst())
                .joined(separator: "\n")
                .write(to: sonucURL, atomically: true, encoding: .utf8)
        }

        log[0] = sayac.baslik
        try? log.joined(separator: "\n").write(to: sonucURL, atomically: true, encoding: .utf8)
        // NSLog yok: eval yanıtları gerçek takvim/kişi verisi içeriyor, sistem log'una gitmemeli.
        print("EVAL bitti: \(sayac.gecen)/\(sayac.toplam)")
    }

    // MARK: - Sayaç

    /// Blok sonuçlarının toplandığı kap.
    ///
    /// `kacirma` ve `atlanan` GEÇME sayılır ama ayrı raporlanır: hafıza kabul
    /// ölçütü (spec §8) yanlış pozitifi öncelikli sayar — bir doğru notu
    /// kaçırmak başarısızlık değil, kalite uyarısıdır. Ağ gerektiren vakalar da
    /// sunucu tanımsızken düşmez, atlanır.
    struct Sayac {
        var gecen = 0
        var toplam = 0
        var kacirma = 0
        var atlanan = 0
        var satirlar: [String] = []

        mutating func birlestir(_ o: Sayac) {
            gecen += o.gecen; toplam += o.toplam
            kacirma += o.kacirma; atlanan += o.atlanan
        }

        /// Vaka sonucunu kaydeder ve tek satırlık raporunu üretir.
        mutating func yaz(_ ad: String, beklenen: String, gercek: String, sorunlar: [String]) {
            toplam += 1
            let ok = sorunlar.isEmpty
            if ok { gecen += 1 }
            satirlar.append("\(ok ? "✓" : "✗") [\(ad)]")
            satirlar.append("    beklenen: \(beklenen)")
            satirlar.append("    gerçek  : \(gercek)")
            if !ok { satirlar.append("    ⚠︎ \(sorunlar.joined(separator: "; "))") }
            satirlar.append("")
        }

        /// Kalite uyarısı: vaka geçer ama not düşülür (yanlış pozitif DEĞİL).
        mutating func uyar(_ metin: String) {
            kacirma += 1
            satirlar.append("    ↪ kaçırma (kabul ölçütü: yanlış pozitiften iyidir): \(metin)")
        }

        /// Koşulamayan vaka — çökme yerine atlama (ağ/sunucu yokluğu).
        mutating func atla(_ ad: String, _ neden: String) {
            atlanan += 1
            satirlar.append("⊘ [\(ad)] ATLANDI — \(neden)")
            satirlar.append("")
        }

        mutating func baslikEkle(_ metin: String) {
            satirlar.append("— \(metin) —")
            satirlar.append("")
        }

        var baslik: String {
            var s = "=== KETUM EVAL: \(gecen)/\(toplam) GEÇTİ"
            if kacirma > 0 { s += " · \(kacirma) kaçırma" }
            if atlanan > 0 { s += " · \(atlanan) atlandı" }
            return s + " ==="
        }
    }

    // MARK: - Hafıza (hafiza-spec §8)

    /// Ayıklama vakası: mesaj → beklenen not sayısı.
    private struct HafizaVaka {
        let ad: String
        let mesaj: String
        /// Beklenen not sayısı. `0` ise ÇIKAN her not yanlış pozitiftir (fail).
        let beklenen: Int
        /// Not metninde geçmemesi gereken parçalar (geçici ayrıntının kaçması).
        var yasakli: [String] = []
    }

    private static func hafizaVakalari() -> [HafizaVaka] {
        [
            // Açık, kalıcı olgu → 1 not.
            HafizaVaka(ad: "hafiza-vejetaryen", mesaj: "Ben vejetaryenim.", beklenen: 1),
            // Geçici gözlem → not YOK.
            HafizaVaka(ad: "hafiza-gecici", mesaj: "Bugün hava çok güzel.", beklenen: 0),
            // Örtük çıkarsama (§4.4) v1 hedefi DEĞİL → not YOK.
            // "eşim" → "evli" çıkarımı yapılırsa bu bir YANLIŞ POZİTİFTİR.
            HafizaVaka(ad: "hafiza-ortuk", mesaj: "Eşim yarın geliyor.", beklenen: 0),
            // Karışık mesaj: yalnız kalıcı olgu ("öğretmenim") seçilmeli;
            // geçici ayrıntı ("yarın", "toplantı", "başım ağrıyor") sızmamalı.
            HafizaVaka(ad: "hafiza-karisik",
                       mesaj: "Öğretmenim. Yarın saat 10'da veli toplantım var, bugün de başım ağrıyor.",
                       beklenen: 1,
                       yasakli: ["toplant", "başım", "yarın"])
        ]
    }

    /// Cihazda ayıklama ölçümü. Her vaka KENDİ bellek-içi deposunda koşar;
    /// gerçek hafıza deposuna dokunulmaz (blok sonunda üretim notları geri konur).
    private static func hafizaKosusu(_ servis: ModelServisi) async -> Sayac {
        var sayac = Sayac()
        sayac.baslikEkle("HAFIZA (hafiza-spec §8) — yanlış pozitif önceliklidir")

        // Ayıklama `HafizaDeposu.yenile` çağırır; üretim deposunu geri koyabilmek
        // için önce anlık görüntü alınır. Kutular blok boyunca canlı tutulur:
        // konteyner ölürse depoda geçersiz nesne kalırdı.
        let uretimNotlari = HafizaDeposu.notlar
        var kutular: [ModelContainer] = []
        let servisHafiza = HafizaServisi()

        for vaka in hafizaVakalari() {
            let sema = Schema(versionedSchema: SemaV1.self)
            let yapilandirma = ModelConfiguration(schema: sema, isStoredInMemoryOnly: true)
            guard let kutu = try? ModelContainer(for: sema, configurations: yapilandirma) else {
                sayac.atla(vaka.ad, "bellek-içi depo kurulamadı")
                continue
            }
            kutular.append(kutu)
            let kayit = kutu.mainContext

            let sohbet = Sohbet()
            kayit.insert(sohbet)
            let mesaj = Mesaj(rol: .sen, icerik: vaka.mesaj)
            mesaj.sohbet = sohbet
            kayit.insert(mesaj)
            try? kayit.save()

            await servisHafiza.ayikla(sohbet: sohbet, kayit: kayit)

            let notlar = ((try? kayit.fetch(FetchDescriptor<HafizaNotu>())) ?? [])
                .filter { !$0.isDeleted }
            let metinler = notlar.map(\.metin)
            let gercek = notlar.isEmpty ? "0 not" : "\(notlar.count) not \(metinler)"

            var sorunlar: [String] = []
            if vaka.beklenen == 0 {
                // Yanlış pozitif — kabul ölçütünün ÖNCELİKLİ ihlali.
                if !notlar.isEmpty { sorunlar.append("yanlış-pozitif:\(metinler)") }
            } else {
                if notlar.count > vaka.beklenen {
                    sorunlar.append("fazla-not:\(notlar.count)>\(vaka.beklenen)")
                }
                for kotu in vaka.yasakli
                where metinler.contains(where: { $0.localizedCaseInsensitiveContains(kotu) }) {
                    sorunlar.append("gecici-ayrinti-sizdi:\(kotu)")
                }
            }

            sayac.yaz(vaka.ad,
                      beklenen: "\(vaka.beklenen) not — '\(vaka.mesaj)'",
                      gercek: gercek,
                      sorunlar: sorunlar)

            // Kaçırma: beklenen vardı, hiç not çıkmadı. Kabul ölçütü (§8) bunu
            // başarısızlık SAYMAZ — yalnız kalite sinyali olarak düşülür.
            if vaka.beklenen > 0 && notlar.isEmpty && sorunlar.isEmpty {
                sayac.uyar("'\(vaka.mesaj)' için not çıkmadı")
                sayac.satirlar.append("")
            }
        }

        // Enjeksiyon sızıntısı (§5.1 çiti): model notu KULLANABİLİR ama ANMAMALI.
        // Not bellek-içi bir bağlama TAKILIR: bağlamsız @Model nesnesini depoya
        // koymak SwiftData'da tanımsız davranıştır ve `kutular` sayesinde canlı kalır.
        let citNotu = HafizaNotu(metin: "Kullanıcı vejetaryendir.",
                                 tur: .tercih,
                                 anahtarlarHam: "yemek, öğle yemeği, akşam yemeği")
        let citSema = Schema(versionedSchema: SemaV1.self)
        if let citKutu = try? ModelContainer(
            for: citSema,
            configurations: ModelConfiguration(schema: citSema, isStoredInMemoryOnly: true)) {
            kutular.append(citKutu)
            citKutu.mainContext.insert(citNotu)
            try? citKutu.mainContext.save()
        }
        HafizaDeposu.yenile([citNotu])
        let soru = "Akşam yemeği için ne önerirsin?"
        if HafizaDeposu.eslesen(soru: soru).isEmpty {
            sayac.atla("hafiza-sizinti", "not eşleşmedi, enjeksiyon hiç olmadı")
        } else {
            servis.sohbetiSifirla()
            let (metin, _) = await servis.yanitla(soru) { _ in }
            let sizintilar = ["hatırladığıma göre", "hatırlıyorum", "hafızam", "notuma göre",
                              "kayıtlarıma göre", "<memory>", "bana söylediğin"]
            let bulunan = sizintilar.filter { metin.localizedCaseInsensitiveContains($0) }
            sayac.yaz("hafiza-sizinti",
                      beklenen: "yanıt notu ANMAZ (sızıntı kalıbı yok)",
                      gercek: bulunan.isEmpty
                        ? "sızıntı yok — \"\(kisalt(metin))\""
                        : "sızıntı \(bulunan) — \"\(kisalt(metin))\"",
                      sorunlar: bulunan.isEmpty ? [] : ["hafiza-sizintisi:\(bulunan)"])
        }

        // Üretim deposunu geri koy — eval, kullanıcının hafızasını değiştirmez.
        HafizaDeposu.yenile(uretimNotlari)
        kutular.removeAll()
        return sayac
    }

    // MARK: - Seyir (seyir-spec §6)

    private static func seyirKosusu(_ servis: ModelServisi) async -> Sayac {
        var sayac = Sayac()
        sayac.baslikEkle("SEYİR (seyir-spec §6) — salt gözlemci")

        // 1) Araçlı turda adım SIRASI: yönlendirme → … → araç → … → yazım.
        servis.sohbetiSifirla()
        let (metin, _) = await servis.yanitla("125 çarpı 8 kaç eder?") { _ in }
        let turler = servis.seyir.adimlar.map(\.tur)
        let dizi = turler.map(\.rawValue).joined(separator: " → ")

        var sorunlar: [String] = []
        if turler.first != .yonlendirme { sorunlar.append("ilk-adim-yonlendirme-degil:\(turler.first?.rawValue ?? "yok")") }
        guard let aracYeri = turler.firstIndex(of: .arac) else {
            sorunlar.append("arac-adimi-yok")
            sayac.yaz("seyir-sira",
                      beklenen: "yonlendirme → arac → yazim",
                      gercek: dizi.isEmpty ? "adım yok" : dizi,
                      sorunlar: sorunlar)
            return sayac
        }
        if let yazimYeri = turler.firstIndex(of: .yazim) {
            if yazimYeri < aracYeri { sorunlar.append("yazim-aractan-once:\(yazimYeri)<\(aracYeri)") }
        } else {
            sorunlar.append("yazim-adimi-yok")
        }
        if let yonYeri = turler.firstIndex(of: .yonlendirme), yonYeri > aracYeri {
            sorunlar.append("yonlendirme-aractan-sonra")
        }
        sayac.yaz("seyir-sira",
                  beklenen: "yonlendirme → arac → yazim (bu sırayla)",
                  gercek: dizi,
                  sorunlar: sorunlar)

        // 2) Salt gözlemci ölçütü: adım metinleri modele HİÇ girmez, dolayısıyla
        //    model çıktısında da görünemez. (Seyir kapatılamadığı için "kapalı vs
        //    açık" farkı ölçülemez — spec §6 zaten ölçülmemesini söyler; ölçülebilir
        //    karşılığı budur: gözlemci metinlerinin yanıta sızmaması.)
        let adimMetinleri = servis.seyir.adimlar.map(\.metin).filter { !$0.isEmpty }
        let sizan = adimMetinleri.filter { metin.localizedCaseInsensitiveContains($0) }
        sayac.yaz("seyir-gozlemci",
                  beklenen: "adım metinleri (\(adimMetinleri)) yanıtta GEÇMEZ",
                  gercek: sizan.isEmpty ? "geçmiyor" : "geçti: \(sizan)",
                  sorunlar: sizan.isEmpty ? [] : ["gozlemci-sizintisi:\(sizan)"])

        // 3) İptal turunda `kesinti` adımı.
        servis.sohbetiSifirla()
        let gorev = Task { _ = await servis.yanitla("Bana uzun bir masal anlat.") { _ in } }
        try? await Task.sleep(for: .milliseconds(1500))
        if servis.uretiyor {
            servis.durdur()
            _ = await gorev.value
            let sonTur = servis.seyir.adimlar.last?.tur
            sayac.yaz("seyir-kesinti",
                      beklenen: "son adım tur = kesinti",
                      gercek: "son adım tur = \(sonTur?.rawValue ?? "yok") · tüm dizi: "
                        + servis.seyir.adimlar.map(\.tur.rawValue).joined(separator: " → "),
                      sorunlar: sonTur == .kesinti ? [] : ["kesinti-adimi-yok"])
        } else {
            _ = await gorev.value
            sayac.atla("seyir-kesinti", "tur 1,5 sn dolmadan bitti, kesecek üretim kalmadı")
        }

        return sayac
    }

    // MARK: - Web araması (web-arama-spec §6)

    private static func webKosusu(_ servis: ModelServisi) async -> Sayac {
        var sayac = Sayac()
        sayac.baslikEkle("WEB ARAMASI (web-arama-spec §6) — yanlış araç seçimi önceliklidir")

        let sunucuVar = WebAramaAyari.aktifMi

        // 1) Sunucu KAPALIYKEN "hava nasıl" → araçsız, dürüst yanıt.
        //    Sunucu tanımlıysa bayrak geçici indirilir, sonra geri konur.
        let onceki = UserDefaults.standard.bool(forKey: WebAramaAyari.aktifAnahtar)
        WebAramaAyari.aktifMi = false
        servis.sohbetiSifirla()
        let (kapaliMetin, kapaliIzler) = await servis.yanitla("Bugün hava nasıl?") { _ in }
        let kapaliIkonlar = kapaliIzler.map(\.ikon)
        var kapaliSorunlar: [String] = []
        if kapaliIkonlar.contains("globe") { kapaliSorunlar.append("kapaliyken-web-arama-cagrildi") }
        if kapaliMetin.localizedCaseInsensitiveContains("derece") {
            kapaliSorunlar.append("uydurma:derece")
        }
        sayac.yaz("web-kapali-durustluk",
                  beklenen: "globe çipi YOK ve yanıtta 'derece' YOK (sınırını söyler)",
                  gercek: "çip:\(kapaliIkonlar) yanıt:\"\(kisalt(kapaliMetin))\"",
                  sorunlar: kapaliSorunlar)
        WebAramaAyari.aktifMi = onceki

        // 2) KARIŞMAMA: kişisel arama web'e gitmemeli. Bu vaka ağ GEREKTİRMEZ
        //    ama anlamlı olması için aracın masada olması gerekir.
        if sunucuVar {
            servis.sohbetiSifirla()
            let (_, izler) = await servis.yanitla("Notlarımda toplantı ile ilgili ne var?") { _ in }
            let ikonlar = izler.map(\.ikon)
            var s: [String] = []
            if ikonlar.contains("globe") { s.append("yanlis-arac:web_arama-kisisel-aramada") }
            if !ikonlar.contains(where: { $0.hasPrefix("magnifyingglass") }) {
                s.append("eksik-arac:magnifyingglass")
            }
            sayac.yaz("web-karismama",
                      beklenen: "magnifyingglass (Spotlight) çağrılır, globe ÇAĞRILMAZ",
                      gercek: "çip:\(ikonlar)",
                      sorunlar: s)
        } else {
            sayac.atla("web-karismama", "arama sunucusu tanımsız/kapalı")
        }

        // 3) "hava nasıl" → web_arama çağrısı ve sorgunun makul olması.
        //    Sorgu kalitesi İKİNCİL: eksikse kaçırma düşülür, vaka düşmez.
        if sunucuVar {
            servis.sohbetiSifirla()
            let (_, izler) = await servis.yanitla("Bugün hava nasıl?") { _ in }
            let ikonlar = izler.map(\.ikon)
            let sorgu = izler.first { $0.ikon == "globe" }?.hamGirdi ?? ""
            let cagrildi = ikonlar.contains("globe")
            sayac.yaz("web-hava",
                      beklenen: "globe çipi (web_arama) çağrılır",
                      gercek: "çip:\(ikonlar) sorgu:\"\(sorgu)\"",
                      sorunlar: cagrildi ? [] : ["eksik-arac:globe"])
            if cagrildi {
                let makul = ["hava", "weather", "durum", "sıcak"]
                if !makul.contains(where: { sorgu.localizedCaseInsensitiveContains($0) }) {
                    sayac.uyar("sorgu kalitesi ikincil — \"\(sorgu)\" hava anahtarı içermiyor")
                    sayac.satirlar.append("")
                }
            }
        } else {
            sayac.atla("web-hava", "arama sunucusu tanımsız/kapalı")
        }

        // 4) Sonuç dönmeyince UYDURMAMA: karşılığı olmayan bir sorgu.
        if sunucuVar {
            servis.sohbetiSifirla()
            let (metin, _) = await servis.yanitla("Zrqxvlon Pflumtek 9182 nedir?") { _ in }
            let durustluk = ["bulamadım", "bulunamadı", "sonuç yok", "bilgi yok",
                             "emin değilim", "bilmiyorum", "ulaşamadım", "rastlamadım"]
            let durust = durustluk.contains { metin.localizedCaseInsensitiveContains($0) }
            sayac.yaz("web-uydurmama",
                      beklenen: "sonuç yokken 'bulamadım' türü dürüst yanıt",
                      gercek: "\"\(kisalt(metin))\"",
                      sorunlar: durust ? [] : ["uydurma-suphesi:dürüstlük-kalıbı-yok"])
        } else {
            sayac.atla("web-uydurmama", "arama sunucusu tanımsız/kapalı")
        }

        return sayac
    }

    /// Rapor satırı için yanıt kırpma (mevcut 70 karakterlik desen).
    private static func kisalt(_ metin: String) -> String {
        String(metin.replacingOccurrences(of: "\n", with: " ").prefix(70))
    }

    // Yardımcılar — beklenen zaman metni.
    private static func suSaat() -> String {
        let f = DateFormatter(); f.locale = Locale(identifier: "tr_TR"); f.dateFormat = "HH:"
        return f.string(from: Date())   // "14:" — saat kısmı yanıtta geçmeli
    }
    private static func suGun() -> String {
        let f = DateFormatter(); f.locale = Locale(identifier: "tr_TR"); f.dateFormat = "EEEE"
        return f.string(from: Date())
    }
}
#endif
