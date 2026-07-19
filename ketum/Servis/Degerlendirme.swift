//
//  Degerlendirme.swift
//  ketum
//
//  Otomatik değerlendirme (eval) — gerçek on-device model üzerinde tüm beceriler
//  ve normal sohbet için test vakaları koşar; doğru araç seçimi, yanıt kalitesi
//  ve hata olup olmadığını ölçer. "--test" argümanıyla açılır (yalnızca DEBUG).
//  Sonuçları Documents/sirr/test-sonuc.txt'e artımlı yazar.
//

#if DEBUG
import Foundation

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
        let klasor = BelgeBaglami.ciktiKlasoru()
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

        log[0] = "=== KETUM EVAL: \(gecen)/\(hepsi.count) GEÇTİ ==="
        try? log.joined(separator: "\n").write(to: sonucURL, atomically: true, encoding: .utf8)
        NSLog("EVAL bitti: %d/%d", gecen, hepsi.count)
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
