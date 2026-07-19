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
                let kisa = metin.replacingOccurrences(of: "\n", with: " ").prefix(55)
                log.append("\(toolOk ? "✓" : "✗") [\(v.ad)] araç:\(ikonlar) · \"\(kisa)\"")
                let ara = (["=== çalışıyor: \(gecen)/\(toplam) ==="] + log.dropFirst()).joined(separator: "\n")
                try? ara.write(to: sonucURL, atomically: true, encoding: .utf8)
            }
            log.append("")
        }
        log[0] = "=== KETUM ÇOK DİLLİ TEST: araç seçimi \(gecen)/\(toplam) ==="
        try? log.joined(separator: "\n").write(to: sonucURL, atomically: true, encoding: .utf8)
    }
}
#endif
