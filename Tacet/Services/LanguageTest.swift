//
//  DilTesti.swift
//  Tacet
//
//  Çok dilli değerlendirme (--language). 9 dilde (tr + en/zh/ja/es/de/fr/ko/pt) çekirdek
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
enum LanguageAnchor {
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
    static func language(_ text: String) -> String? {
        let temiz = text.trimmingCharacters(in: .whitespacesAndNewlines)
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
    enum Outcome: Equatable {
        case dogru(String)
        case sapti(expected: String, bulunan: String)
        case olculemedi

        var isareti: String {
            switch self {
            case .dogru(let d): return "dil:\(d) ✓"
            case .sapti(let b, let g): return "dil:\(g) ✗ (beklenen \(b))"
            case .olculemedi: return "dil:? ⊘"
            }
        }
    }

    static func audit(_ text: String, expected: String) -> Outcome {
        guard let bulunan = language(text) else { return .olculemedi }
        return bulunan == expected ? .dogru(bulunan)
                                   : .sapti(expected: expected, bulunan: bulunan)
    }
}

@MainActor
enum LanguageTest {
    struct V { let name: String; let prompt: String; let icon: String }  // ikon "" = araç beklenmiyor

    static func languages() -> [(String, [V])] {
        [
            ("tr", [
                V(name: "selam",   prompt: "Merhaba", icon: ""),
                V(name: "hesap",   prompt: "125 çarpı 8 kaç eder?", icon: "function"),
                V(name: "zaman",   prompt: "Saat kaç?", icon: ""),
                V(name: "takvim",  prompt: "Yarın takvimimde ne var?", icon: "calendar"),
                V(name: "hatirlat",prompt: "Beni 18:00'de aramam için hatırlat", icon: "bell"),
                V(name: "excel",   prompt: "Haftalık yemek listesi için excel yap", icon: "tablecells"),
            ]),
            ("en", [
                V(name: "greet",   prompt: "Hello", icon: ""),
                V(name: "calc",    prompt: "What is 125 times 8?", icon: "function"),
                V(name: "time",    prompt: "What time is it?", icon: ""),
                V(name: "cal",     prompt: "What's on my calendar tomorrow?", icon: "calendar"),
                V(name: "remind",  prompt: "Remind me to call at 6pm", icon: "bell"),
                V(name: "excel",   prompt: "Make an excel of my weekly meal plan", icon: "tablecells"),
            ]),
            ("zh", [
                V(name: "greet",   prompt: "你好", icon: ""),
                V(name: "calc",    prompt: "125乘以8等于多少？", icon: "function"),
                V(name: "time",    prompt: "现在几点？", icon: ""),
                V(name: "cal",     prompt: "我明天的日历上有什么？", icon: "calendar"),
                V(name: "remind",  prompt: "提醒我下午6点打电话", icon: "bell"),
                V(name: "excel",   prompt: "帮我做一个每周膳食计划的excel表格", icon: "tablecells"),
            ]),
            ("ja", [
                V(name: "greet",   prompt: "こんにちは", icon: ""),
                V(name: "calc",    prompt: "125かける8は？", icon: "function"),
                V(name: "time",    prompt: "今何時？", icon: ""),
                V(name: "cal",     prompt: "明日のカレンダーの予定は？", icon: "calendar"),
                V(name: "remind",  prompt: "18時に電話するようリマインドして", icon: "bell"),
                V(name: "excel",   prompt: "週間献立のエクセルを作って", icon: "tablecells"),
            ]),
            ("es", [
                V(name: "greet",   prompt: "Hola", icon: ""),
                V(name: "calc",    prompt: "¿Cuánto es 125 por 8?", icon: "function"),
                V(name: "time",    prompt: "¿Qué hora es?", icon: ""),
                V(name: "cal",     prompt: "¿Qué tengo en mi calendario mañana?", icon: "calendar"),
                V(name: "remind",  prompt: "Recuérdame llamar a las 6 de la tarde", icon: "bell"),
                V(name: "excel",   prompt: "Haz un excel con mi plan de comidas semanal", icon: "tablecells"),
            ]),
            ("de", [
                V(name: "greet",   prompt: "Hallo", icon: ""),
                V(name: "calc",    prompt: "Was ist 125 mal 8?", icon: "function"),
                V(name: "time",    prompt: "Wie spät ist es?", icon: ""),
                V(name: "cal",     prompt: "Was steht morgen in meinem Kalender?", icon: "calendar"),
                V(name: "remind",  prompt: "Erinnere mich, um 18 Uhr anzurufen", icon: "bell"),
                V(name: "excel",   prompt: "Erstelle eine Excel-Tabelle für meinen Wochenessensplan", icon: "tablecells"),
            ]),
            ("fr", [
                V(name: "greet",   prompt: "Bonjour", icon: ""),
                V(name: "calc",    prompt: "Combien font 125 fois 8 ?", icon: "function"),
                V(name: "time",    prompt: "Quelle heure est-il ?", icon: ""),
                V(name: "cal",     prompt: "Qu'y a-t-il dans mon agenda demain ?", icon: "calendar"),
                V(name: "remind",  prompt: "Rappelle-moi d'appeler à 18h", icon: "bell"),
                V(name: "excel",   prompt: "Fais un excel de mon plan de repas hebdomadaire", icon: "tablecells"),
            ]),
            ("ko", [
                V(name: "greet",   prompt: "안녕하세요", icon: ""),
                V(name: "calc",    prompt: "125 곱하기 8은?", icon: "function"),
                V(name: "time",    prompt: "지금 몇 시야?", icon: ""),
                V(name: "cal",     prompt: "내일 내 캘린더에 뭐 있어?", icon: "calendar"),
                V(name: "remind",  prompt: "오후 6시에 전화하라고 알림 설정해줘", icon: "bell"),
                V(name: "excel",   prompt: "주간 식단표 엑셀 만들어줘", icon: "tablecells"),
            ]),
            ("pt", [
                V(name: "greet",   prompt: "Olá", icon: ""),
                V(name: "calc",    prompt: "Quanto é 125 vezes 8?", icon: "function"),
                V(name: "time",    prompt: "Que horas são?", icon: ""),
                V(name: "cal",     prompt: "O que tenho na minha agenda amanhã?", icon: "calendar"),
                V(name: "remind",  prompt: "Me lembre de ligar às 18h", icon: "bell"),
                V(name: "excel",   prompt: "Faça um excel do meu plano de refeições semanal", icon: "tablecells"),
            ]),
        ]
    }

    static func run() { Task { await run() } }

    static func run() async {
        let service = ModelService()
        let sonucURL = DocumentContext.testKlasoru().appendingPathComponent("dil-sonuc.txt")
        guard service.state.isReady else {
            try? "MODEL HAZIR DEĞİL".write(to: sonucURL, atomically: true, encoding: .utf8)
            return
        }

        var log: [String] = ["=== TACET ÇOK DİLLİ TEST ===", ""]
        var gecen = 0, total = 0
        // İkinci eksen (P1-9): yanıt gerçekten kullanıcının dilinde mi.
        // `dilOlculemedi` ayrı sayılır — payda ile karıştırılırsa dil skoru
        // kısa yanıtların sayısına göre oynar.
        var dilGecen = 0, dilOlculen = 0, dilOlculemedi = 0

        for (language, vakalar) in languages() {
            log.append("──── \(language.uppercased()) ────")
            for v in vakalar {
                service.resetChat()
                let (text, traces) = await service.yanitla(v.prompt) { _ in }
                let ikonlar = traces.map(\.icon)
                let toolOk = v.icon.isEmpty
                    ? true   // araçsız vakalarda araç seçimini zorlamıyoruz; yanıt dilini gözlüyoruz
                    : ikonlar.contains { $0.hasPrefix(v.icon) }
                total += 1
                if toolOk { gecen += 1 }
                let dilSonuc = LanguageAnchor.audit(text, expected: language)
                switch dilSonuc {
                case .dogru:      dilOlculen += 1; dilGecen += 1
                case .sapti:      dilOlculen += 1
                case .olculemedi: dilOlculemedi += 1
                }
                let short = text.replacingOccurrences(of: "\n", with: " ").prefix(55)
                log.append("\(toolOk ? "✓" : "✗") [\(v.name)] araç:\(ikonlar) · "
                           + "\(dilSonuc.isareti) · \"\(short)\"")
                let search = (["=== çalışıyor: araç \(gecen)/\(total) · dil \(dilGecen)/\(dilOlculen) ==="]
                           + log.dropFirst()).joined(separator: "\n")
                try? search.write(to: sonucURL, atomically: true, encoding: .utf8)
            }
            log.append("")
        }
        log[0] = "=== TACET ÇOK DİLLİ TEST: araç seçimi \(gecen)/\(total)"
            + " · yanıt dili \(dilGecen)/\(dilOlculen)"
            + (dilOlculemedi > 0 ? " (\(dilOlculemedi) ölçülemedi)" : "") + " ==="
        try? log.joined(separator: "\n").write(to: sonucURL, atomically: true, encoding: .utf8)
    }
}
#endif
