//
//  CalcTool.swift
//  Tacet
//
//  A helper tool (spec §7.3). Arithmetic is ALWAYS resolved in code, never in the model
//  (spec §2, principle 4). The instructions steer the model to route every calculation here.
//  Pure Swift — evaluation without NSExpression, no network.
//

import Foundation
import FoundationModels

struct CalcTool: TacetTool {
    let name = "calculate"
    let description = "Evaluates arithmetic: addition, subtraction, multiplication, division, percentages, parentheses. ALWAYS call this for ANY numeric calculation, in any language; never compute the result yourself."

    weak var reporter: (any ToolReporter)?

    @Generable
    struct Arguments {
        // MEASURED BUG: the old example was `(1250+890)*1.20` and `%` was NOT COUNTED among
        // the allowed operators (even though the evaluator supported it). With the guide
        // hiding the capability, the model converted the percentage into a multiplier ITSELF:
        // "add 18% VAT to 480" → it wrote `480*1.19` and produced 571.2 (the correct answer
        // is 566.4). Taking exactly that conversion away from the model is why this tool exists.
        @Guide(description: "The arithmetic expression to evaluate; digits and the operators + - * / ( ) % . only. Use '.' as the decimal point and write no thousands separators: '1000+500', not '1,000+500'. '%' is postfix and divides by 100, so '18%' is 0.18. Write percentages literally and NEVER fold one into a multiplier yourself: to add 18% VAT to 480 write '480*(1+18%)' or '480+480*18%' — writing '480*1.18' means you did the conversion, which is exactly the arithmetic this tool exists to take off your hands. E.g. '(1250+890)*(1+20%)'.")
        var expression: String
    }

    func call(arguments: Arguments) async -> String {
        await runWithChip(icon: "function", runningText: L10n.calculating, rawInput: arguments.expression) {
            let result = try Self.evaluate(arguments.expression)
            let text = Self.format(result)
            return ToolOutcome(
                chipText: L10n.calculated,
                state: .readOk,
                toModel: "result=\(text)",
                rawOutput: "\(arguments.expression) = \(text)"
            )
        }
    }

    enum CalcError: LocalizedError, ToolErrorCode {
        case invalid
        var errorDescription: String? { String(localized: "Couldn’t evaluate the expression") }
        var errorCode: String { "invalid_expression" }
    }

    /// Safe arithmetic evaluation. Because NSExpression can throw an UNCATCHABLE ObjC
    /// exception and crash the app, a hand-written recursive-descent resolver is used —
    /// a malformed expression never produces a crash, only `CalcError.invalid`.
    static func evaluate(_ raw: String) throws -> Double {
        let allowed = CharacterSet(charactersIn: "0123456789.+-*/()% ")
        let cleaned = try resolveSeparators(raw)
        guard cleaned.unicodeScalars.allSatisfy({ allowed.contains($0) }),
              !cleaned.trimmingCharacters(in: .whitespaces).isEmpty else {
            throw CalcError.invalid
        }
        var resolver = ArithmeticResolver(cleaned)
        let result = try resolver.resolve()
        guard result.isFinite else { throw CalcError.invalid }
        return result
    }

    // MARK: - Separator resolution

    /// MEASURED FAILURE: the old code turned `,` into `.` unconditionally. When the model
    /// wrote an English thousands separator ("1,000 + 500") the result came out as 501, and
    /// a Turkish "1.250,50" silently became 1,250 — a CONFIDENT WRONG NUMBER right in the
    /// middle of the "arithmetic is always in code" promise. That is why the conversion is
    /// done PER NUMBER CLUSTER, not over the whole expression.
    ///
    /// Cluster rules:
    ///  • If both `,` and `.` are present: the LAST separator is the decimal one, the other
    ///    is the thousands one. If the grouping does not hold `\d{1,3}(thousands\d{3})+decimal\d+`,
    ///    the input is ambiguous.
    ///  • If only `,` is present: if `\d{1,3}(,\d{3})+` holds it is a thousands group (dropped),
    ///    if it is `\d+,\d+` it is a decimal, otherwise ambiguous.
    ///  • If only `.` is present: THE RULE IS THE SAME AS THE COMMA'S — if `\d{1,3}(.\d{3})+`
    ///    holds it is a thousands group (dropped), a single dot is a decimal, otherwise
    ///    ambiguous. The dot-only branch used to count as a decimal unconditionally, and the
    ///    same shape was resolved in OPPOSITE ways with the two separators
    ///    (`1,500` → 1500 but `1.500` → 1.5).
    /// For a cluster that stays ambiguous an error is thrown instead of silently guessing:
    /// saying "couldn't evaluate the expression" beats handing back a wrong number.
    private static func resolveSeparators(_ raw: String) throws -> String {
        var result = ""
        var cluster = ""
        func drainCluster() throws {
            guard !cluster.isEmpty else { return }
            result += try resolveCluster(cluster)
            cluster = ""
        }
        for c in raw {
            // The ASCII-digit condition is deliberate: Arabic-Indic digits would pass
            // `isNumber` but they are already eliminated by the allowed character set.
            if (c.isASCII && c.isNumber) || c == "." || c == "," {
                cluster.append(c)
            } else {
                try drainCluster()
                result.append(c)
            }
        }
        try drainCluster()
        return result
    }

    private static func resolveCluster(_ cluster: String) throws -> String {
        let hasComma = cluster.contains(",")
        let hasDot = cluster.contains(".")
        if !hasComma && !hasDot { return cluster }

        if hasComma && hasDot {
            guard let lastComma = cluster.lastIndex(of: ","),
                  let lastDot = cluster.lastIndex(of: ".") else { throw CalcError.invalid }
            let decimal: Character = lastComma > lastDot ? "," : "."
            let thousands: Character = decimal == "," ? "." : ","
            let t = NSRegularExpression.escapedPattern(for: String(thousands))
            let d = NSRegularExpression.escapedPattern(for: String(decimal))
            guard patternHolds(cluster, "^[0-9]{1,3}(\(t)[0-9]{3})+\(d)[0-9]+$") else {
                throw CalcError.invalid
            }
            return cluster
                .replacingOccurrences(of: String(thousands), with: "")
                .replacingOccurrences(of: String(decimal), with: ".")
        }

        // ONE SEPARATOR, ONE RULE. Deciding the OPPOSITE way depending on the separator was
        // a defect: "1,500" was read as thousands and became 1500, "1.500" was read as a
        // decimal and became 1.5 — one of two inputs with the same shape is off by a factor
        // of 1000 either way, and which one is wrong cannot be read off the input. Now both
        // separators obey the same grouping rule; that rule is also identical to
        // `AnswerFilter.resolveTheNumber`'s (the interface language is Turkish, so "1.234"
        // here is one thousand two hundred thirty-four).
        //
        // A thousands group does NOT START with 0: that is why "0,500"/"0.500" are decimals.
        if hasComma {
            if patternHolds(cluster, "^[1-9][0-9]{0,2}(,[0-9]{3})+$") {
                return cluster.replacingOccurrences(of: ",", with: "")
            }
            guard patternHolds(cluster, "^[0-9]+,[0-9]+$") else { throw CalcError.invalid }
            return cluster.replacingOccurrences(of: ",", with: ".")
        }

        // Dot only — identical to the comma.
        if patternHolds(cluster, "^[1-9][0-9]{0,2}(\\.[0-9]{3})+$") {
            return cluster.replacingOccurrences(of: ".", with: "")
        }
        guard cluster.filter({ $0 == "." }).count == 1 else { throw CalcError.invalid }
        return cluster
    }

    private static func patternHolds(_ text: String, _ pattern: String) -> Bool {
        text.range(of: pattern, options: .regularExpression) != nil
    }

    static func format(_ d: Double) -> String {
        if d == d.rounded() && abs(d) < 1e15 {
            return String(Int(d))
        }
        let nf = NumberFormatter()
        nf.locale = Locale(identifier: "tr_TR")
        nf.maximumFractionDigits = 4
        nf.minimumFractionDigits = 0
        return nf.string(from: NSNumber(value: d)) ?? String(d)
    }
}

/// A safe recursive-descent arithmetic resolver. Grammar:
///   expression = term (('+'|'-') term)*
///   term       = percent (('*'|'/') percent)*
///   percent    = unit ('%')*                 // postfix percent: 20% = 0.20
///   unit       = number | '(' expression ')' | ('+'|'-') unit
/// It throws no ObjC exception; on malformed input it throws CalcTool.CalcError.invalid.
private struct ArithmeticResolver {
    private let k: [Character]
    private var i = 0
    init(_ s: String) { k = Array(s) }

    mutating func resolve() throws -> Double {
        let v = try expression()
        space()
        guard i >= k.count else { throw CalcTool.CalcError.invalid }
        return v
    }

    private mutating func space() { while i < k.count, k[i] == " " { i += 1 } }
    private mutating func peek() -> Character? { space(); return i < k.count ? k[i] : nil }

    private mutating func expression() throws -> Double {
        var v = try term()
        while let c = peek(), c == "+" || c == "-" {
            i += 1
            let t = try term()
            v = (c == "+") ? v + t : v - t
        }
        return v
    }

    private mutating func term() throws -> Double {
        var v = try percent()
        while let c = peek(), c == "*" || c == "/" {
            i += 1
            let t = try percent()
            if c == "*" { v *= t }
            else {
                guard t != 0 else { throw CalcTool.CalcError.invalid }
                v /= t
            }
        }
        return v
    }

    private mutating func percent() throws -> Double {
        var v = try unit()
        while peek() == "%" { i += 1; v /= 100 }
        return v
    }

    private mutating func unit() throws -> Double {
        guard let c = peek() else { throw CalcTool.CalcError.invalid }
        if c == "-" { i += 1; return -(try unit()) }
        if c == "+" { i += 1; return try unit() }
        if c == "(" {
            i += 1
            let v = try expression()
            guard peek() == ")" else { throw CalcTool.CalcError.invalid }
            i += 1
            return v
        }
        return try number()
    }

    private mutating func number() throws -> Double {
        space()
        var j = i
        while j < k.count, k[j].isNumber || k[j] == "." { j += 1 }
        guard j > i, let d = Double(String(k[i..<j])) else {
            throw CalcTool.CalcError.invalid
        }
        i = j
        return d
    }
}
