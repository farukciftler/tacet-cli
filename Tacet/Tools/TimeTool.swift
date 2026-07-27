//
//  TimeTool.swift
//  Tacet
//
//  A helper tool (spec §7.3). Gives the current date/time. Foundation, no network.
//  Spec note: no chip is shown for this tool — it is not noteworthy. It still carries
//  a reporter (the contract), but it drops no chip while running.
//

import Foundation
import FoundationModels

struct TimeTool: TacetTool {
    let name = "time"
    // "diff" is named as a SEPARATE action. Measured bug: with NO tool able to do calendar
    // arithmetic, the model did not leave the question unanswered — it made the answer up
    // (it said "6 days" between 19 July and 2 December). Saying "don't compute it yourself"
    // without handing over the capability produces confident fabrication. CalcTool cannot
    // solve this: it only accepts digits and operators, it knows nothing about calendars
    // (leap years, month lengths). The difference is computed here, with Calendar.
    let description = "Gives the current date/time/day of week, and counts days between today and another date. Call this whenever the user asks for the current time/date OR asks how many days/weeks until (or since) a date — in any language. NEVER compute a date difference yourself: calendar arithmetic needs leap years and month lengths, so it must be calculated here. NEVER resolve a named or relative date ('new year', 'next Friday') into a calendar date yourself — pass the user's own words and let this tool resolve them; your idea of the current year is not reliable. The reply reports 'direction=future' (say 'remaining'), 'past' (say 'passed') or 'today'; use that word, never state a negative number of days."

    weak var reporter: (any ToolReporter)?

    /// The kind is NOT free text (P0-4): listing five free-form values in the @Guide did not
    /// stop the model from inventing a sixth; EVERY value other than `kind.lowercased() == "fark"`
    /// silently fell through to "all" — a model that wrote "difference" got the clock/date
    /// instead of the day difference.
    @Generable
    enum Kind: String, Equatable, CaseIterable {
        case clock
        case date
        case weekday
        case all
        case diff
    }

    @Generable
    struct Arguments {
        @Guide(description: "What to return: 'clock' (time of day), 'date', 'weekday' (day of week), 'all', or 'diff' (days until/since the date given in 'target'). If unsure use 'all'.")
        var kind: Kind
        @Guide(description: "Only for kind='diff': the other date, copied from the user's own words (e.g. 'December 2 2026', '2026-12-02', 'next Friday', 'new year'). Copy it VERBATIM — never substitute a calendar date of your own and never add a year the user did not write. Named and relative dates are resolved here, in code. Leave empty otherwise.")
        var target: String
    }

    func call(arguments: Arguments) async throws -> String {
        // "what time is it" stays CHIPLESS (spec: not noteworthy, it clutters the flow).
        // "diff" DOES DROP A CHIP: it produces a number, and that number rests on a PARSED
        // input — the date may have been read wrong. The chip detail shows
        // "from=… to=… days=…", so the user catches a bad parse. Hiding a number the user
        // needs to verify would pierce the timeline's "Tacet does not hide what it did" principle.
        guard arguments.kind == .diff else {
            return Self.now(kind: arguments.kind)
        }
        let rawTarget = arguments.target
        return await runWithChip(icon: "calendar",
                                 runningText: L10n.countingDays,
                                 rawInput: rawTarget) {
            let output = Self.diff(rawTarget: rawTarget)
            guard !output.hasPrefix("error:") else {
                return ToolOutcome(
                    chipText: L10n.dateNotUnderstood,
                    state: .failed(L10n.dateNotUnderstood),
                    toModel: output,
                    rawOutput: output
                )
            }
            return ToolOutcome(
                chipText: L10n.daysCounted,
                state: .readOk,
                toModel: output,
                rawOutput: output
            )
        }
    }

    /// The WHOLE number of days between today and the target date. Both dates are reduced to
    /// the start of the day; otherwise a difference in hours shifts the count by one day.
    /// It returns a negative number for a past date — so the model can say "passed".
    static func diff(rawTarget: String) -> String {
        guard let resolution = TimeResolver.resolve(rawTarget) else {
            // NO silent fallback to today: an unresolvable date comes back as an error,
            // otherwise the model sees "0 days" and takes that for the answer.
            return "error: could not parse the date \"\(rawTarget)\". Ask the user to restate it."
        }
        let calendar = Calendar.current
        let today = calendar.startOfDay(for: Date())
        let target = calendar.startOfDay(for: resolution.date)
        guard let days = calendar.dateComponents([.day], from: today, to: target).day else {
            return "error: date difference could not be computed."
        }

        let format = DateFormatter()
        format.locale = Locale(identifier: "en_US_POSIX")
        format.dateFormat = "yyyy-MM-dd"
        // Language-neutral output; the model translates it into the user's language. The sign
        // is kept deliberately (negative = past) — it stops the model from inventing a direction.
        //
        // `direction` is given ALONGSIDE the sign, not instead of it: MEASURED BUG — with only
        // the sign present, the model dropped the number "-571" into the sentence as-is and
        // said "there are -571 days left until new year". Spelling the direction out in a word
        // means the choice between "remaining" and "passed" is not left to the model's inference.
        let direction = days > 0 ? "future" : (days < 0 ? "past" : "today")
        return "from=\(format.string(from: today)) to=\(format.string(from: target)) days=\(days) direction=\(direction)"
    }

    static func now(kind: Kind) -> String {
        let now = Date()
        // Language-neutral output: ISO date + 24h clock + English day name. The model
        // translates it into the user's language (multilingual — it never parrots the
        // date/time string verbatim).
        let tf = DateFormatter()
        tf.locale = Locale(identifier: "en_US_POSIX")
        tf.dateFormat = "HH:mm"
        let clock = tf.string(from: now)
        tf.dateFormat = "yyyy-MM-dd"
        let date = tf.string(from: now)
        tf.dateFormat = "EEEE"
        let weekday = tf.string(from: now)

        switch kind {
        case .clock:   return "time=\(clock)"
        case .date:    return "date=\(date)"
        case .weekday: return "weekday=\(weekday)"
        // `.diff` never reaches here (the caller splits it off) but it is written out
        // explicitly because the switch must be exhaustive: we leave no silent `default` branch.
        case .all, .diff: return "time=\(clock) date=\(date) weekday=\(weekday)"
        }
    }
}
