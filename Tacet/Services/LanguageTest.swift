//
//  LanguageTest.swift
//  Tacet
//
//  The multilingual evaluation (--language). It runs the core prompts in 9
//  languages (tr + en/zh/ja/es/de/fr/ko/pt) against the real model: the correct
//  tool choice (via the language-neutral chip icon) and whether the reply is in
//  that language are measured. The outcome is written to a file.
//
//  THE PROMPTS BELOW ARE TEST FIXTURES, NOT UI TEXT: each one must stay in the
//  language of its own row — that is precisely what is being measured.
//

#if DEBUG
import Foundation
import NaturalLanguage

// MARK: - The language anchor (P1-9)

/// Determines whether the reply is REALLY in the user's language.
///
/// This criterion used to be "look at it by eye": the code counted only the tool
/// choice (the chip icon prefix), and the reply text was clipped to 55 characters and
/// printed into the log. A language drift — especially drifting into the language of
/// the tool output — was reflected in no number at all, so a regression could pass
/// silently.
///
/// `NLLanguageRecognizer` is unreliable on SHORT text ("Merhaba" on its own can very
/// well come back as "id"). Hence two guards: (a) the candidate languages are
/// constrained — it cannot go outside the nine languages we run in the test, (b) a
/// detection below the confidence threshold IS NOT COUNTED AS A FAILURE, it returns
/// `nil`. Reporting what we could not measure as a defect is worse than not measuring.
enum LanguageAnchor {
    /// The languages we run in the test — the recogniser cannot go outside them.
    static let candidates: [NLLanguage] = [
        .turkish, .english, .simplifiedChinese, .japanese, .spanish,
        .german, .french, .korean, .portuguese
    ]

    /// The language code mapping (so the report line shows "tr"/"zh").
    static let codes: [NLLanguage: String] = [
        .turkish: "tr", .english: "en", .simplifiedChinese: "zh",
        .traditionalChinese: "zh", .japanese: "ja", .spanish: "es",
        .german: "de", .french: "fr", .korean: "ko", .portuguese: "pt"
    ]

    /// The confidence floor. 0.50 is not arbitrary: with the candidate list constrained
    /// to nine, the expectation of a random guess is ~0.11; 0.50 is more than four times
    /// that, yet not so high that it eliminates genuine one-sentence replies.
    static let confidenceFloor = 0.50

    /// The dominant language of the text — `nil` if it cannot be determined (too short /
    /// low confidence). `nil` does NOT mean "wrong language", it means "not measurable".
    static func language(_ text: String) -> String? {
        let clean = text.trimmingCharacters(in: .whitespacesAndNewlines)
        // Digits and punctuation carry no language on their own; a letter count is the
        // baseline requirement.
        guard clean.filter({ $0.isLetter }).count >= 8 else { return nil }
        let recognizer = NLLanguageRecognizer()
        recognizer.languageConstraints = candidates
        recognizer.processString(clean)
        let hypotheses = recognizer.languageHypotheses(withMaximum: 3)
        guard let (best, confidence) = hypotheses.max(by: { $0.value < $1.value }),
              confidence >= confidenceFloor else { return nil }
        return codes[best]
    }

    /// Is there a drift from the expected language? Three-valued: `.correct` /
    /// `.drifted(found)` / `.notMeasured`. The report line shows all three separately.
    enum Outcome: Equatable {
        case correct(String)
        case drifted(expected: String, found: String)
        case notMeasured

        var mark: String {
            switch self {
            case .correct(let code): return "lang:\(code) ✓"
            case .drifted(let expected, let found): return "lang:\(found) ✗ (expected \(expected))"
            case .notMeasured: return "lang:? ⊘"
            }
        }
    }

    static func audit(_ text: String, expected: String) -> Outcome {
        guard let found = language(text) else { return .notMeasured }
        return found == expected ? .correct(found)
                                 : .drifted(expected: expected, found: found)
    }
}

@MainActor
enum LanguageTest {
    struct V { let name: String; let prompt: String; let icon: String }  // icon "" = no tool expected

    static func languages() -> [(String, [V])] {
        [
            ("tr", [
                V(name: "greet",   prompt: "Merhaba", icon: ""),
                V(name: "calc",    prompt: "125 çarpı 8 kaç eder?", icon: "function"),
                V(name: "time",    prompt: "Saat kaç?", icon: ""),
                V(name: "cal",     prompt: "Yarın takvimimde ne var?", icon: "calendar"),
                V(name: "remind",  prompt: "Beni 18:00'de aramam için hatırlat", icon: "bell"),
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
        let outcomeURL = DocumentContext.testFolder().appendingPathComponent("language-outcome.txt")
        guard service.state.isReady else {
            try? "MODEL NOT READY".write(to: outcomeURL, atomically: true, encoding: .utf8)
            return
        }

        var log: [String] = ["=== TACET MULTILINGUAL TEST ===", ""]
        var passed = 0, total = 0
        // The second axis (P1-9): is the reply really in the user's language?
        // `languageNotMeasured` is counted separately — mixing it into the denominator
        // would make the language score swing with the number of short replies.
        var languagePassed = 0, languageMeasured = 0, languageNotMeasured = 0

        for (language, cases) in languages() {
            log.append("──── \(language.uppercased()) ────")
            for v in cases {
                service.resetChat()
                let (text, traces) = await service.respond(v.prompt) { _ in }
                let icons = traces.map(\.icon)
                let toolOk = v.icon.isEmpty
                    ? true   // in tool-less cases we do not force a tool choice; we watch the reply language
                    : icons.contains { $0.hasPrefix(v.icon) }
                total += 1
                if toolOk { passed += 1 }
                let languageOutcome = LanguageAnchor.audit(text, expected: language)
                switch languageOutcome {
                case .correct:     languageMeasured += 1; languagePassed += 1
                case .drifted:     languageMeasured += 1
                case .notMeasured: languageNotMeasured += 1
                }
                let short = text.replacingOccurrences(of: "\n", with: " ").prefix(55)
                log.append("\(toolOk ? "✓" : "✗") [\(v.name)] tool:\(icons) · "
                           + "\(languageOutcome.mark) · \"\(short)\"")
                let running = (["=== running: tool \(passed)/\(total) · language \(languagePassed)/\(languageMeasured) ==="]
                           + log.dropFirst()).joined(separator: "\n")
                try? running.write(to: outcomeURL, atomically: true, encoding: .utf8)
            }
            log.append("")
        }
        log[0] = "=== TACET MULTILINGUAL TEST: tool choice \(passed)/\(total)"
            + " · reply language \(languagePassed)/\(languageMeasured)"
            + (languageNotMeasured > 0 ? " (\(languageNotMeasured) not measured)" : "") + " ==="
        try? log.joined(separator: "\n").write(to: outcomeURL, atomically: true, encoding: .utf8)
    }
}
#endif
