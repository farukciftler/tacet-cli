//
//  DilTesti.swift
//  ketum
//
//  Çok dilli değerlendirme (--dil). 9 dilde (tr + en/zh/ja/es/de/fr/ko/pt) çekirdek
//  istemleri gerçek model üzerinde koşar: doğru araç seçimi (dil-nötr çip ikonuyla)
//  ve yanıtın o dilde olup olmadığı (log'dan gözle) ölçülür. Sonuç dosyaya yazılır.
//

#if DEBUG
import Foundation
import NaturalLanguage

// MARK: - Dil çapası (P1-9)

/// Yanıtın GERÇEKTEN kullanıcının dilinde olup olmadığını saptar.
///
/// Eskiden bu ölçüt "gözle bak"tı: kod yalnız araç seçimini (çip ikon önekini)
/// sayıyor, yanıt metni log'a 55 karakter kırpılıp basılıyordu. Dil kayması —
/// özellikle araç çıktısının diline sürüklenme — hiçbir sayıya yansımıyordu,
/// dolayısıyla gerileme sessizce geçebilirdi.
///
/// `NLLanguageRecognizer` KISA metinde güvenilmezdir ("Merhaba" tek başına
/// pekâlâ "id" dönebilir). Bu yüzden iki koruma var: (a) aday diller
/// kısıtlanır — testte koştuğumuz dokuz dil dışına çıkılmaz, (b) güven eşiği
/// altındaki saptama BAŞARISIZLIK SAYILMAZ, `nil` döner. Ölçemediğimiz şeyi
/// kusur diye raporlamak, ölçmemekten daha kötüdür.
enum DilCapasi {
    /// Testte koştuğumuz diller — tanıyıcı bunların dışına çıkamaz.
    static let adaylar: [NLLanguage] = [
        .turkish, .english, .simplifiedChinese, .japanese, .spanish,
        .german, .french, .korean, .portuguese
    ]

    /// Dil kodu eşlemesi (rapor satırında "tr"/"zh" gibi görünsün).
    static let kodlar: [NLLanguage: String] = [
        .turkish: "tr", .english: "en", .simplifiedChinese: "zh",
        .traditionalChinese: "zh", .japanese: "ja", .spanish: "es",
        .german: "de", .french: "fr", .korean: "ko", .portuguese: "pt"
    ]

    /// Güven tabanı. 0.50 keyfi değil: aday listesi dokuza kısıtlıyken rastgele
    /// tahminin beklentisi ~0.11; 0.50 onun dört katından fazlası ama tek
    /// cümlelik gerçek yanıtları da eleyecek kadar yüksek değil.
    static let guvenTabani = 0.50

    /// Metnin baskın dili — saptanamazsa (çok kısa / güven düşük) `nil`.
    /// `nil` "dil yanlış" DEĞİL, "ölçülemedi" demektir.
    static func dil(_ metin: String) -> String? {
        let temiz = metin.trimmingCharacters(in: .whitespacesAndNewlines)
        // Rakam ve noktalama tek başına dil taşımaz; harf sayısı taban şart.
        guard temiz.filter({ $0.isLetter }).count >= 8 else { return nil }
        let tanici = NLLanguageRecognizer()
        tanici.languageConstraints = adaylar
        tanici.processString(temiz)
        let olasiliklar = tanici.languageHypotheses(withMaximum: 3)
        guard let (en, guven) = olasiliklar.max(by: { $0.value < $1.value }),
              guven >= guvenTabani else { return nil }
        return kodlar[en]
    }

    /// Beklenen dilden sapma var mı. Üç değerli: `.dogru` / `.sapti(bulunan)`
    /// / `.olculemedi`. Rapor satırı üçünü de ayrı gösterir.
    enum Sonuc: Equatable {
        case dogru(String)
        case sapti(beklenen: String, bulunan: String)
        case olculemedi

        var isareti: String {
            switch self {
            case .dogru(let d): return "dil:\(d) ✓"
            case .sapti(let b, let g): return "dil:\(g) ✗ (beklenen \(b))"
            case .olculemedi: return "dil:? ⊘"
            }
        }
    }

    static func denetle(_ metin: String, beklenen: String) -> Sonuc {
        guard let bulunan = dil(metin) else { return .olculemedi }
        return bulunan == beklenen ? .dogru(bulunan)
                                   : .sapti(beklenen: beklenen, bulunan: bulunan)
    }
}

@MainActor
enum DilTesti {
    struct V { let ad: String; let istem: String; let ikon: String }  // ikon "" = araç beklenmiyor

    static func diller() -> [(String, [V])] {
        [
            ("tr", [
                V(ad: "selam",   istem: "Merhaba", ikon: ""),
                V(ad: "hesap",   istem: "125 çarpı 8 kaç eder?", ikon: "function"),
                V(ad: "zaman",   istem: "Saat kaç?", ikon: ""),
                V(ad: "takvim",  istem: "Yarın takvimimde ne var?", ikon: "calendar"),
                V(ad: "hatirlat",istem: "Beni 18:00'de aramam için hatırlat", ikon: "bell"),
                V(ad: "excel",   istem: "Haftalık yemek listesi için excel yap", ikon: "tablecells"),
            ]),
            ("en", [
                V(ad: "greet",   istem: "Hello", ikon: ""),
                V(ad: "calc",    istem: "What is 125 times 8?", ikon: "function"),
                V(ad: "time",    istem: "What time is it?", ikon: ""),
                V(ad: "cal",     istem: "What's on my calendar tomorrow?", ikon: "calendar"),
                V(ad: "remind",  istem: "Remind me to call at 6pm", ikon: "bell"),
                V(ad: "excel",   istem: "Make an excel of my weekly meal plan", ikon: "tablecells"),
            ]),
            ("zh", [
                V(ad: "greet",   istem: "你好", ikon: ""),
                V(ad: "calc",    istem: "125乘以8等于多少？", ikon: "function"),
                V(ad: "time",    istem: "现在几点？", ikon: ""),
                V(ad: "cal",     istem: "我明天的日历上有什么？", ikon: "calendar"),
                V(ad: "remind",  istem: "提醒我下午6点打电话", ikon: "bell"),
                V(ad: "excel",   istem: "帮我做一个每周膳食计划的excel表格", ikon: "tablecells"),
            ]),
            ("ja", [
                V(ad: "greet",   istem: "こんにちは", ikon: ""),
                V(ad: "calc",    istem: "125かける8は？", ikon: "function"),
                V(ad: "time",    istem: "今何時？", ikon: ""),
                V(ad: "cal",     istem: "明日のカレンダーの予定は？", ikon: "calendar"),
                V(ad: "remind",  istem: "18時に電話するようリマインドして", ikon: "bell"),
                V(ad: "excel",   istem: "週間献立のエクセルを作って", ikon: "tablecells"),
            ]),
            ("es", [
                V(ad: "greet",   istem: "Hola", ikon: ""),
                V(ad: "calc",    istem: "¿Cuánto es 125 por 8?", ikon: "function"),
                V(ad: "time",    istem: "¿Qué hora es?", ikon: ""),
                V(ad: "cal",     istem: "¿Qué tengo en mi calendario mañana?", ikon: "calendar"),
                V(ad: "remind",  istem: "Recuérdame llamar a las 6 de la tarde", ikon: "bell"),
                V(ad: "excel",   istem: "Haz un excel con mi plan de comidas semanal", ikon: "tablecells"),
            ]),
            ("de", [
                V(ad: "greet",   istem: "Hallo", ikon: ""),
                V(ad: "calc",    istem: "Was ist 125 mal 8?", ikon: "function"),
                V(ad: "time",    istem: "Wie spät ist es?", ikon: ""),
                V(ad: "cal",     istem: "Was steht morgen in meinem Kalender?", ikon: "calendar"),
                V(ad: "remind",  istem: "Erinnere mich, um 18 Uhr anzurufen", ikon: "bell"),
                V(ad: "excel",   istem: "Erstelle eine Excel-Tabelle für meinen Wochenessensplan", ikon: "tablecells"),
            ]),
            ("fr", [
                V(ad: "greet",   istem: "Bonjour", ikon: ""),
                V(ad: "calc",    istem: "Combien font 125 fois 8 ?", ikon: "function"),
                V(ad: "time",    istem: "Quelle heure est-il ?", ikon: ""),
                V(ad: "cal",     istem: "Qu'y a-t-il dans mon agenda demain ?", ikon: "calendar"),
                V(ad: "remind",  istem: "Rappelle-moi d'appeler à 18h", ikon: "bell"),
                V(ad: "excel",   istem: "Fais un excel de mon plan de repas hebdomadaire", ikon: "tablecells"),
            ]),
            ("ko", [
                V(ad: "greet",   istem: "안녕하세요", ikon: ""),
                V(ad: "calc",    istem: "125 곱하기 8은?", ikon: "function"),
                V(ad: "time",    istem: "지금 몇 시야?", ikon: ""),
                V(ad: "cal",     istem: "내일 내 캘린더에 뭐 있어?", ikon: "calendar"),
                V(ad: "remind",  istem: "오후 6시에 전화하라고 알림 설정해줘", ikon: "bell"),
                V(ad: "excel",   istem: "주간 식단표 엑셀 만들어줘", ikon: "tablecells"),
            ]),
            ("pt", [
                V(ad: "greet",   istem: "Olá", ikon: ""),
                V(ad: "calc",    istem: "Quanto é 125 vezes 8?", ikon: "function"),
                V(ad: "time",    istem: "Que horas são?", ikon: ""),
                V(ad: "cal",     istem: "O que tenho na minha agenda amanhã?", ikon: "calendar"),
                V(ad: "remind",  istem: "Me lembre de ligar às 18h", ikon: "bell"),
                V(ad: "excel",   istem: "Faça um excel do meu plano de refeições semanal", ikon: "tablecells"),
            ]),
        ]
    }

    static func calistir() { Task { await kosu() } }

    static func kosu() async {
        let servis = ModelServisi()
        let sonucURL = BelgeBaglami.testKlasoru().appendingPathComponent("dil-sonuc.txt")
        guard servis.durum.hazirMi else {
            try? "MODEL HAZIR DEĞİL".write(to: sonucURL, atomically: true, encoding: .utf8)
            return
        }

        var log: [String] = ["=== KETUM ÇOK DİLLİ TEST ===", ""]
        var gecen = 0, toplam = 0
        // İkinci eksen (P1-9): yanıt gerçekten kullanıcının dilinde mi.
        // `dilOlculemedi` ayrı sayılır — payda ile karıştırılırsa dil skoru
        // kısa yanıtların sayısına göre oynar.
        var dilGecen = 0, dilOlculen = 0, dilOlculemedi = 0

        for (dil, vakalar) in diller() {
            log.append("──── \(dil.uppercased()) ────")
            for v in vakalar {
                servis.sohbetiSifirla()
                let (metin, izler) = await servis.yanitla(v.istem) { _ in }
                let ikonlar = izler.map(\.ikon)
                let toolOk = v.ikon.isEmpty
                    ? true   // araçsız vakalarda araç seçimini zorlamıyoruz; yanıt dilini gözlüyoruz
                    : ikonlar.contains { $0.hasPrefix(v.ikon) }
                toplam += 1
                if toolOk { gecen += 1 }
                let dilSonuc = DilCapasi.denetle(metin, beklenen: dil)
                switch dilSonuc {
                case .dogru:      dilOlculen += 1; dilGecen += 1
                case .sapti:      dilOlculen += 1
                case .olculemedi: dilOlculemedi += 1
                }
                let kisa = metin.replacingOccurrences(of: "\n", with: " ").prefix(55)
                log.append("\(toolOk ? "✓" : "✗") [\(v.ad)] araç:\(ikonlar) · "
                           + "\(dilSonuc.isareti) · \"\(kisa)\"")
                let ara = (["=== çalışıyor: araç \(gecen)/\(toplam) · dil \(dilGecen)/\(dilOlculen) ==="]
                           + log.dropFirst()).joined(separator: "\n")
                try? ara.write(to: sonucURL, atomically: true, encoding: .utf8)
            }
            log.append("")
        }
        log[0] = "=== KETUM ÇOK DİLLİ TEST: araç seçimi \(gecen)/\(toplam)"
            + " · yanıt dili \(dilGecen)/\(dilOlculen)"
            + (dilOlculemedi > 0 ? " (\(dilOlculemedi) ölçülemedi)" : "") + " ==="
        try? log.joined(separator: "\n").write(to: sonucURL, atomically: true, encoding: .utf8)
    }
}
#endif
