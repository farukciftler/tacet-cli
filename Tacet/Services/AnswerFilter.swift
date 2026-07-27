//
//  AnswerFilter.swift
//  Tacet
//
//  The DETERMINISTIC layer that moves web search from being a "list of links" to
//  something that "brings the answer". Everything here IS A PURE FUNCTION: no
//  network, no model, tested with fixtures. The only piece that goes to the network
//  stays in that file as `WebSearchClient.fetchPage` (the network monopoly rule,
//  which SelfTest verifies with a static scan).
//
//  WHY CODE, WHY NOT A MODEL
//  ------------------------
//  In this project, every time the relevance/sufficiency judgement was left to the
//  model, the model made things up: 20°C and 24°C for the same weather question, 6
//  days for a date difference. The model produces a value it does not have "because
//  it looks plausible". That is why the question "did the requested information
//  arrive?" is answered here WITH REGEX: the SHAPE of the information is derived from
//  the query, that shape is looked for in the page text, and if it is not found often
//  enough NO content is given to the model at all — it honestly returns "I could not
//  find it".
//
//  INJECTION
//  ---------
//  Page text is untrusted external content. ONLY pattern matches and the narrow
//  context of each (≤120 characters, a single line) pass the filter; the raw page text
//  never goes to the model. Markdown link syntax and code fences are stripped from the
//  context — no surface is left for the model to copy a link or an instruction off the
//  page. The raw full text goes to the `DataStore` channel and the user sees it in the
//  chip detail.

import Foundation

// MARK: - The sought shape

/// The SHAPE of the information the query is after. Derived from the query IN CODE;
/// the model is never asked.
nonisolated enum SoughtShape: String, CaseIterable, Equatable, Sendable {
    /// A clock time / timetable: "07:30", "19.45".
    case clock
    /// Temperature + weather-condition text: "24°", "-3 derece", "parçalı bulutlu".
    case temperature
    /// An exchange/gold rate: "47,1329", "41,25 TL". SEPARATE from `price` — on rate
    /// pages the value arrives bare (no symbol) and the decimals go up to 4.
    case rate
    /// Money/price: "41,25 TL", "$1.200". A symbol or a unit is MANDATORY.
    case price
    /// A date: "12.08.2026", "3 Ağustos".
    case date
    /// A match score: "2-1".
    case score
    /// A distance/duration: "12 km", "45 dakika".
    case distance
    /// The shape could not be determined — a free-text question. The loop DOES NOT RUN
    /// and the existing behaviour (the blurb list) is preserved.
    case none

    /// The DETERMINISTIC order used in shape detection and tie-breaking.
    /// `allCases` order is not trusted: it changes silently when a new case is added.
    /// Narrow patterns (rate, score) come BEFORE the wide ones (price, clock).
    static let ordered: [SoughtShape] = [.rate, .score, .clock, .temperature, .price, .distance, .date]

    /// Is the value time-dependent — i.e. would stale data mislead the user?
    /// A ferry timetable, an exchange rate, the weather and a score belong to today; the
    /// date of an event or the distance between two cities do not. The freshness warning
    /// is given ONLY for these shapes; adding a warning to every answer turns the warning
    /// into noise.
    var isTimeDependent: Bool {
        switch self {
        case .clock, .temperature, .rate, .price, .score: return true
        case .date, .distance, .none: return false
        }
    }

    /// The English name that goes to the model (the output contract is English).
    var englishName: String {
        switch self {
        case .clock: return "clock times"
        case .temperature: return "temperatures and weather conditions"
        case .rate: return "exchange rates"
        case .price: return "prices"
        case .date: return "dates"
        case .score: return "match scores"
        case .distance: return "distances or durations"
        case .none: return "-"
        }
    }

    /// The user-visible (chip) name.
    var localName: String {
        switch self {
        case .clock: return String(localized: "time")
        case .temperature: return String(localized: "weather")
        case .rate: return String(localized: "rate")
        case .price: return String(localized: "price")
        case .date: return String(localized: "date")
        case .score: return String(localized: "score")
        case .distance: return String(localized: "distance")
        case .none: return ""
        }
    }
}

/// Whether the fetched page belongs to TODAY.
///
/// WHY IT EXISTS: the most insidious bug measured came out of this layer. Prayer times
/// came back as 03:49 / 05:23 / 05:04 in three separate attempts; all three had been
/// read from a REAL source and at least two were the winter timetable. The filter was
/// asking "is there a clock pattern", not "does this page belong to TODAY". Wrong data
/// relayed correctly is more insidious than invention: because a source is shown, the
/// user does not question it.
///
/// The value is still given — it is just not PRESENTED silently as current.
nonisolated enum Freshness: Equatable, Sendable {
    /// Today's date is written on the page the value came from. High confidence.
    case verified
    /// The page was downloaded but today's date does not appear. The value is given, and
    /// so is a warning.
    case notVerified
    /// No page was downloaded at all (the values came from search blurbs). Blurb text is
    /// undated; it DOES NOT COUNT as verified.
    case unknown

    /// The warning that goes to the model. If `nil` no warning is added.
    var modelWarning: String? {
        switch self {
        case .verified:
            return nil
        case .notVerified:
            return "WARNING: today's date does not appear on the pages these values "
                + "came from, so they may be out of date (for example a winter "
                + "timetable or yesterday's rate). Give the values, but also tell the "
                + "user plainly, in their own language, that you could not confirm "
                + "they are current and that they should check the source."
        case .unknown:
            return "WARNING: these values come from search-result summaries, which "
                + "carry no date. Give them, but also tell the user plainly, in their "
                + "own language, that you could not confirm they are up to date."
        }
    }
}

/// A single match: the value found + a narrow context from the line it was found on +
/// the source domain.
nonisolated struct Match: Equatable, Sendable {
    /// The raw value matching the pattern ("07:30").
    var value: String
    /// The context clipped from the line the value was found on (≤ `contextCap`).
    var context: String
    /// The domain the value came from ("sehirhatlari.istanbul"). NOT the full URL.
    var source: String
    /// Does the page the value came from belong to today? `.unknown` for matches coming
    /// from search blurbs — blurb text is undated.
    var freshness: Freshness = .unknown
}

// MARK: - The filter

/// `nonisolated` IS DELIBERATE (the same rationale as `MCPClient.JSONValue`): the
/// project builds with `SWIFT_DEFAULT_ACTOR_ISOLATION = MainActor`, so every type whose
/// isolation is not stated binds to the UI queue. Everything in this file is a pure
/// function — no network, no model, no state — and it does the most expensive work in
/// the app: HTML→text conversion, entity resolution and a line-by-line regex scan over
/// a 400 KB page. While they ran on the main actor the chip animation and the stream
/// froze for the duration of the search round. Leaving isolation makes it possible to
/// move the work to the global executor (see `WebSearchClient.fetchPage`).
///
/// THE ONE EXCEPTION IS `table`: `Table`/`Row` are @Generable types bound to MainActor;
/// that member is explicitly marked `@MainActor` and its callers do not change.
nonisolated enum AnswerFilter {

    // MARK: Hard limits
    // No magic number is embedded; they are all named here and SelfTest looks at them.

    /// If this many DISTINCT matches are found the loop stops; the answer is sufficient.
    static let sufficiencyThreshold = 3
    /// At most this many pages are downloaded SUCCESSFULLY (per search round). The number
    /// of rounds is 1: rewriting the query and searching again does not exist in v1.
    /// A dead/unreachable page does not spend this counter — see `candidateCap`.
    static let pageCap = 6
    /// The maximum number of candidates tried in order. It must be larger than `pageCap`:
    /// dead pages or pages carrying no data can sit at the top of the search results and
    /// the value we want can be further down. The time budget is the real brake.
    static let candidateCap = 10
    /// The maximum bytes downloaded per page. Beyond it the download is cut off.
    static let pageByteCap = 400 * 1024
    /// The timeout for fetching a single page (s).
    ///
    /// It is kept BELOW the total budget and that is critical: with a 10 s page timeout
    /// against a 15 s budget, a single hanging page ate two thirds of the budget and the
    /// next healthy page never got its turn. A page that does not respond in 5 s is
    /// probably useless anyway; dropping it and moving on pays better.
    static let pageTimeout: TimeInterval = 5
    /// The total search + fetch budget (s). Beyond it no new page is fetched.
    ///
    /// THIS IS THE REAL BRAKE — not the page count. The page cap is kept high and time is
    /// what is watched: on fast servers many pages are visited, on slow ones few. The user
    /// sees that they will wait 15 s from the domain names streaming through the chip.
    static let totalBudget: TimeInterval = 15
    /// The cap on the number of matches that go to the model.
    static let matchCap = 25
    /// The context character cap per match.
    static let contextCap = 120
    /// The cap on the TOTAL filtered text that goes to the model (the 4096 budget).
    static let modelTextCap = 1200
    /// With this many regular matches a `Table` is produced in code.
    static let tableThreshold = 6

    // MARK: - Shape detection (pure)

    /// Derives the sought shape from the query. A keyword count; the shape with the most
    /// signal wins, ties broken by the `SoughtShape.ordered` order (clock first).
    static func findShape(_ query: String) -> SoughtShape {
        let text = simplify(query)
        guard !text.isEmpty else { return .none }

        var scores: [(SoughtShape, Int)] = []
        for shape in SoughtShape.ordered {
            let number = hints(shape).reduce(0) { total, hint in
                total + (containsWord(text, hint) ? 1 : 0)
            }
            if number > 0 { scores.append((shape, number)) }
        }
        guard let best = scores.max(by: { $0.1 < $1.1 }) else { return .none }
        // On a tie the `SoughtShape.ordered` order wins (so that it is deterministic).
        // The order runs from the narrow pattern to the wide one: "dolar kuru" gives both
        // a `rate` and a `price` signal, and `rate` must win.
        let highest = best.1
        for shape in SoughtShape.ordered
        where scores.contains(where: { $0.0 == shape && $0.1 == highest }) {
            return shape
        }
        return .none
    }

    /// The query hints belonging to a shape. Turkish + English; all of them are written
    /// simplified (unaccented, lowercase) because `simplify` delivers the input that way.
    ///
    /// THESE ARE USER-INPUT MATCH DATA, not identifiers: they must stay in their own
    /// languages, exactly like a keyword list.
    static func hints(_ shape: SoughtShape) -> [String] {
        switch shape {
        case .clock:
            // "vakit/vakitleri/imsak/iftar/ezan" were added AFTER MEASUREMENT: the query
            // "istanbul namaz vakitleri" fitted no shape at all (`findShape` → .none) and
            // the loop returned a link list without ever running. That was exactly the
            // clock question the user asked most often.
            return ["saat", "saatleri", "saatler", "sefer", "seferleri", "tarife",
                    "tarifesi", "kacta", "kalkis", "varis", "vapur", "feribot",
                    "otobus", "tren", "metro", "ucus", "seans", "acilis", "kapanis",
                    "vakit", "vakti", "vakitleri", "namaz", "imsak", "iftar",
                    "sahur", "ezan", "gunes", "ogle", "ikindi", "aksam", "yatsi",
                    "schedule", "timetable", "departure", "departures", "arrival"]
        case .temperature:
            return ["hava", "havalar", "sicaklik", "sicak", "soguk", "derece",
                    "hava durumu", "yagmur", "yagis", "kar", "ruzgar", "nem",
                    "weather", "temperature", "forecast", "rain", "snow"]
        case .rate:
            // A rate is a SEPARATE shape from `price`: on rate pages the value arrives
            // BARE ("47,1329") and the decimals go up to 4. Because the `price` pattern
            // requires a symbol/unit, it caught none of those values.
            return ["kur", "kuru", "kurlari", "doviz", "dolar", "euro", "avro",
                    "sterlin", "pound", "altin", "gram altin", "ceyrek", "ons",
                    "usd", "eur", "gbp", "try", "parite", "serbest piyasa",
                    "exchange", "rate"]
        case .price:
            return ["fiyat", "fiyati", "fiyatlari", "ucret", "ucreti", "lira",
                    "kac para", "kac tl", "borsa", "zam", "indirim", "maliyet",
                    "price", "cost", "fee"]
        case .date:
            return ["tarih", "tarihi", "tarihleri", "ne zaman", "hangi gun",
                    "hangi tarih", "son basvuru", "takvim", "date", "when",
                    "deadline"]
        case .score:
            return ["skor", "skoru", "mac", "maci", "maclari", "sonuc", "sonucu",
                    "kac kac", "gol", "derbi", "score", "result", "match"]
        case .distance:
            return ["mesafe", "mesafesi", "kac km", "kac kilometre", "kac saat surer",
                    "ne kadar surer", "kac dakika", "sure", "suresi", "uzaklik",
                    "distance", "how far", "how long"]
        case .none:
            return []
        }
    }

    /// The regex pattern of the shape. The bounds are kept narrow to reduce false
    /// positives (e.g. 00–23 / 00–59 for a clock; otherwise "3.14" is taken for a time).
    static func pattern(_ shape: SoughtShape) -> String? {
        switch shape {
        case .clock:
            // Two separate branches, because the dot separator is dangerous:
            // - With a `:` separator the hour may be a single digit (7:30).
            // - With a `.` separator the hour MUST BE TWO DIGITS (08.30). Otherwise
            //   decimals such as "3.14" (pi) or "1.50" (a price) were taken for a time — a
            //   measured false positive. Real timetables write zero-padded anyway.
            // The look-ahead ACCEPTS a sentence-ending dot as in "21:45." but rejects a
            // date chain such as "12.08.2026".
            return "(?<![\\d.:])(?:([01][0-9]|2[0-3])\\.[0-5][0-9]"
                + "|([01]?[0-9]|2[0-3]):[0-5][0-9])(?![0-9]|[.:][0-9])"
        case .temperature:
            // Two branches: the DEGREE value and the CONDITION TEXT. The condition text was
            // added after measurement — expressions like "parçalı bulutlu" are half of a
            // weather answer, and carrying the degree alone gave a bare, incomplete answer
            // such as "24°".
            return "(-?[0-9]{1,2}\\s*(°[CcFf]?|derece|degrees?))"
                + "|(\\b(güneşli|gunesli|parçalı bulutlu|parcali bulutlu|çok bulutlu|"
                + "cok bulutlu|bulutlu|açık|acik|yağmurlu|yagmurlu|sağanak|saganak|"
                + "kar yağışlı|kar yagisli|karlı|karli|sisli|puslu|rüzgarlı|ruzgarli|"
                + "gök gürültülü|gok gurultulu|hafif yağmur|hafif yagmur|"
                + "sunny|cloudy|partly cloudy|overcast|rainy|showers|snow|foggy|clear)\\b)"
        case .rate:
            // A BARE Turkish decimal number: "47,1329", "1.234,56", "41,25".
            // The decimal COMMA is mandatory — so that whole numbers (a year, a count, an
            // index) do not match. 2–6 decimal places: rate pages write 4.
            // NO symbol is looked for; instead a currency hint is looked for AT LINE LEVEL
            // (`lineHints`), because in rate tables the unit is in the column header, not
            // inside the cell.
            // IT IS NOT A PERCENTAGE: rate pages write the daily change next to the value
            // ("47,1588  %0,14"). Measured — those percentages were going to the model as
            // the rate value and left the door open to an answer like "the dollar is 0.14".
            // A two-directional percentage look-around eliminates them.
            // The percent sign eliminates ONLY when it is adjacent to its own number: "%0,14"
            // (leading) in Turkish and "0,14%" (trailing, NO SPACE) in English. A "%" with a
            // space before it belongs to the next number — on the line "47,1588 %0,14",
            // 47,1588 is the real rate and must not be eliminated (measured).
            return "(?<![0-9.,%])(?<!% )[0-9]{1,3}(?:\\.[0-9]{3})*,[0-9]{2,6}(?![0-9])(?!%)"
                + "|(?<![0-9.,%])(?<!% )[0-9]{1,5}\\.[0-9]{2,6}(?![0-9.,])(?!%)"
        case .price:
            return "((?:[₺$€£]|\\bUSD\\b|\\bEUR\\b|\\bTRY\\b)\\s*[0-9]{1,3}(?:[.,][0-9]{3})*(?:[.,][0-9]{1,2})?)"
                + "|([0-9]{1,3}(?:[.,][0-9]{3})*(?:[.,][0-9]{1,2})?\\s*(?:[₺$€£]|TL|USD|EUR|TRY|lira|dolar|euro))"
        case .score:
            // "2-1", "3 - 0". So that it is not confused with a time or a date, the
            // separator is a hyphen only and both sides are AT MOST TWO DIGITS; a year range
            // ("2024-2026") is eliminated by the look-around.
            return "(?<![0-9.,:/-])[0-9]{1,2}\\s?[-–]\\s?[0-9]{1,2}(?![0-9.,:/-])"
        case .distance:
            return "[0-9]{1,4}(?:[.,][0-9]{1,2})?\\s*"
                + "(km\\b|kilometre|metre\\b|\\bm\\b|mil\\b|saat\\b|dakika|dk\\b|"
                + "minutes?\\b|hours?\\b|miles?\\b)"
        case .date:
            return "([0-3]?[0-9][./-][01]?[0-9](?:[./-][0-9]{2,4})?)"
                + "|([0-3]?[0-9]\\s+(?:ocak|şubat|subat|mart|nisan|mayıs|mayis|haziran|temmuz|"
                + "ağustos|agustos|eylül|eylul|ekim|kasım|kasim|aralık|aralik|"
                + "january|february|march|april|may|june|july|august|september|october|november|december))"
        case .none:
            return nil
        }
    }

    /// A LINE-LEVEL HINT: at least one of these words must occur on the line the match is
    /// counted on. An empty list = no condition.
    ///
    /// WHY: the `rate` and `score` patterns are FAR TOO WIDE on their own. "47,1329" may
    /// be a rate but it may equally be a product weight; "2-1" may be a score but it may
    /// equally be a page number. Requiring the CONTEXT of the line produces fewer false
    /// positives than narrowing the pattern — and in this project a false positive is more
    /// expensive than an empty result: because the user sees a source, they do not
    /// question the wrong value.
    static func lineHints(_ shape: SoughtShape) -> [String] {
        switch shape {
        case .rate:
            return ["usd", "eur", "gbp", "try", "dolar", "euro", "avro", "sterlin",
                    "lira", "tl", "altin", "gram", "ceyrek", "ons", "kur", "doviz",
                    "alis", "satis", "parite", "₺", "$", "€", "£"]
        case .score:
            return ["skor", "mac", "sonuc", "gol", "devre", "ms", "ft", "score",
                    "match", "result", "half"]
        case .clock, .temperature, .price, .date, .distance, .none:
            return []
        }
    }

    /// Does the line satisfy the shape's hint condition?
    static func lineQualifies(_ line: String, shape: SoughtShape) -> Bool {
        let hints = lineHints(shape)
        guard !hints.isEmpty else { return true }
        let plain = simplify(line)
        return hints.contains { hint in
            // Punctuation/symbol hints do not respect word boundaries.
            hint.unicodeScalars.allSatisfy { CharacterSet.letters.contains($0) }
                ? containsWord(plain, hint)
                : plain.contains(hint)
        }
    }

    // MARK: - Turkish number format

    /// "47,1329" → 47.1329, "1.234,56" → 1234.56, "1,234.56" → 1234.56.
    ///
    /// In Turkish content the decimal separator is the COMMA and that distinction is
    /// critical: "1.234" is one thousand two hundred and thirty-four in Turkish and one
    /// point two three four in English. A rate resolved wrongly is a rate relayed wrongly.
    ///
    /// The rule: if both separators are present, the LAST one is the decimal. If only a
    /// comma is present it is the decimal (the Turkish default). If only a dot is present
    /// and exactly three digits follow it, it is a thousands separator, otherwise a decimal.
    static func resolveNumber(_ raw: String) -> Double? {
        let digits = raw.filter { $0.isNumber || $0 == "." || $0 == "," || $0 == "-" }
        guard !digits.isEmpty else { return nil }

        let lastDot = digits.lastIndex(of: ".")
        let lastComma = digits.lastIndex(of: ",")

        var decimalSeparator: Character?
        switch (lastDot, lastComma) {
        case let (dot?, comma?):
            decimalSeparator = dot > comma ? "." : ","
        case (nil, .some):
            decimalSeparator = ","
        case let (dot?, nil):
            let after = digits[digits.index(after: dot)...]
            // Exactly three digits AND no other dot means a thousands separator.
            decimalSeparator = (after.count == 3 && after.allSatisfy(\.isNumber)) ? nil : "."
        case (nil, nil):
            decimalSeparator = nil
        }

        var whole = ""
        var fraction = ""
        var sawDecimal = false
        for char in digits {
            if char == "-", whole.isEmpty, !sawDecimal {
                whole.append(char)
            } else if char.isNumber {
                if sawDecimal { fraction.append(char) } else { whole.append(char) }
            } else if let decimalSeparator, char == decimalSeparator, !sawDecimal {
                sawDecimal = true
            }
        }
        guard !whole.isEmpty, whole != "-" else { return nil }
        return Double(fraction.isEmpty ? whole : "\(whole).\(fraction)")
    }

    // MARK: - Freshness (does today's date appear on the page)

    /// Turkish month names (unaccented — compared against `simplify` output).
    static let monthsTR = ["ocak", "subat", "mart", "nisan", "mayis", "haziran",
                          "temmuz", "agustos", "eylul", "ekim", "kasim", "aralik"]
    /// English month names.
    static let monthsEN = ["january", "february", "march", "april", "may", "june",
                          "july", "august", "september", "october", "november", "december"]

    /// The WRITTEN FORMS of the given day that are looked for on the page.
    ///
    /// All of them CONTAIN THE YEAR and that is deliberate. A year-less form such as
    /// "20 temmuz" also occurs on last year's page; counting that as "current" is exactly
    /// the bug we are trying to avoid. Not accepting the year-less form produces more
    /// "not verified" — which is better than presenting something wrong as current.
    static func dayFormats(_ date: Date, calendar: Calendar = Calendar(identifier: .gregorian)) -> [String] {
        let chunk = calendar.dateComponents([.year, .month, .day], from: date)
        guard let year = chunk.year, let month = chunk.month, let day = chunk.day,
              (1...12).contains(month) else { return [] }

        let g = String(day), gg = String(format: "%02d", day)
        let a = String(month), aa = String(format: "%02d", month)
        let y = String(year), yy = String(format: "%02d", year % 100)

        var formats = [
            "\(gg).\(aa).\(y)", "\(g).\(a).\(y)",
            "\(gg)/\(aa)/\(y)", "\(g)/\(a)/\(y)",
            "\(gg)-\(aa)-\(y)", "\(g)-\(a)-\(y)",
            "\(y)-\(aa)-\(gg)", "\(y)/\(aa)/\(gg)",
            "\(gg).\(aa).\(yy)",
        ]
        for names in [monthsTR, monthsEN] {
            let name = names[month - 1]
            formats += ["\(g) \(name) \(y)", "\(gg) \(name) \(y)",
                         "\(name) \(g), \(y)", "\(name) \(g) \(y)"]
        }
        return formats
    }

    /// Does today's date occur in the page text? The comparison goes through `simplify`:
    /// "20 Temmuz 2026" and "20 temmuz 2026" are the same.
    static func todayAppears(_ text: String,
                                 today: Date,
                                 calendar: Calendar = Calendar(identifier: .gregorian)) -> Bool {
        let plain = simplify(text)
        guard !plain.isEmpty else { return false }
        return dayFormats(today, calendar: calendar).contains { plain.contains($0) }
    }

    /// The page freshness. For shapes that are NOT time-dependent (a distance, the date of
    /// an event) looking for a date is meaningless — straight to `.verified`.
    static func pageFreshness(_ text: String, shape: SoughtShape, today: Date) -> Freshness {
        guard shape.isTimeDependent else { return .verified }
        return todayAppears(text, today: today) ? .verified : .notVerified
    }

    /// The OVERALL freshness of the match set: the WORST match decides.
    /// One page showing today does not absolve a stale value coming from another.
    static func overallFreshness(_ matches: [Match]) -> Freshness {
        if matches.contains(where: { $0.freshness == .notVerified }) { return .notVerified }
        if matches.contains(where: { $0.freshness == .unknown }) { return .unknown }
        return matches.isEmpty ? .unknown : .verified
    }

    /// IF THERE ARE VERIFIED VALUES, DROP THE UNVERIFIED ONES.
    ///
    /// If we gathered enough values from a page carrying today's date, mixing values from
    /// undated sources into the same list pollutes the answer: the user sees 03:49
    /// (winter) next to 05:23 (summer) and cannot tell which belongs to today. A mixed set
    /// is not given while a clean one exists.
    static func preferFresh(_ matches: [Match]) -> [Match] {
        let verifiedOnes = matches.filter { $0.freshness == .verified }
        return verifiedOnes.count >= sufficiencyThreshold ? verifiedOnes : matches
    }

    // MARK: - The second-round query (deterministic narrowing)

    /// If the first round did not find the shape, the query is narrowed IN CODE. The model
    /// DOES NOT rewrite the query: having the model write queries repeatedly produced
    /// irrelevant queries in this project and ate the budget. The narrowing is fixed and
    /// predictable.
    ///
    /// The added terms do two jobs: a shape-specific word (timetable/rate/degree) steers
    /// towards the right page type, and today's date pulls the CURRENT page forward —
    /// pushing back on the stale-data problem from the search side as well.
    ///
    /// It does not narrow an already narrowed query (it returns `nil`) — there is no point
    /// running the same search twice and burning the budget.
    static func narrowedQuery(_ query: String,
                                 shape: SoughtShape,
                                 today: Date,
                                 calendar: Calendar = Calendar(identifier: .gregorian)) -> String? {
        let plain = simplify(query)
        guard !plain.isEmpty else { return nil }

        var extras: [String] = []
        switch shape {
        case .clock: extras = ["saat", "tarife"]
        case .rate: extras = ["kur", "alis satis"]
        case .price: extras = ["fiyat"]
        case .temperature: extras = ["hava durumu", "derece"]
        case .score: extras = ["mac sonucu"]
        case .distance: extras = ["mesafe"]
        case .date: extras = ["tarih"]
        case .none: return nil
        }
        // Do not add extras that already occur in the query.
        let new = extras.filter { !plain.contains($0) }

        // For time-dependent shapes today's date is added too (dd.mm.yyyy).
        let chunk = calendar.dateComponents([.year, .month, .day], from: today)
        var dateExtra = ""
        if shape.isTimeDependent, let year = chunk.year, let month = chunk.month, let day = chunk.day {
            let candidate = String(format: "%02d.%02d.%d", day, month, year)
            if !plain.contains(candidate) { dateExtra = candidate }
        }

        guard !new.isEmpty || !dateExtra.isEmpty else { return nil }
        return ([query.trimmingCharacters(in: .whitespacesAndNewlines)] + new
                + (dateExtra.isEmpty ? [] : [dateExtra])).joined(separator: " ")
    }

    // MARK: - Match scanning (pure)

    /// Finds the values in the text matching the shape, together with their line contexts.
    /// If the same value occurs several times it is counted ONCE: the threshold counts
    /// "distinct matches", and the same time repeated five times on a page does not enrich
    /// the answer.
    static func match(_ text: String,
                         shape: SoughtShape,
                         source: String,
                         freshness: Freshness = .unknown) -> [Match] {
        guard let phrase = pattern(shape),
              let engine = try? NSRegularExpression(pattern: phrase, options: [.caseInsensitive])
        else { return [] }

        var outcome: [Match] = []
        var seen = Set<String>()

        for line in text.components(separatedBy: .newlines) {
            let clean = line.trimmingCharacters(in: .whitespaces)
            guard !clean.isEmpty else { continue }
            // The line-level hint condition (rate/score): with no context the number does
            // not count.
            guard lineQualifies(clean, shape: shape) else { continue }
            let ns = clean as NSString
            let found = engine.matches(in: clean, options: [],
                                           range: NSRange(location: 0, length: ns.length))
            for b in found {
                let value = ns.substring(with: b.range).trimmingCharacters(in: .whitespaces)
                guard valueIsPlausible(value, shape: shape) else { continue }
                let key = normalizeValue(value, shape: shape)
                guard !key.isEmpty, !seen.contains(key) else { continue }
                seen.insert(key)
                outcome.append(Match(value: value,
                                     context: extractContext(ns, range: b.range),
                                     source: source,
                                     freshness: freshness))
                if outcome.count >= matchCap { return outcome }
            }
        }
        return outcome
    }

    /// Scans the downloaded page IN A SINGLE CALL: first the freshness stamp, then the
    /// matches carrying that stamp. The two can also be called separately; the reason they
    /// stand together is that the call is made OFF the main actor — for text of hundreds of
    /// thousands of characters one executor hop is enough instead of two.
    static func scanPage(_ text: String,
                            shape: SoughtShape,
                            source: String,
                            today: Date) -> (freshness: Freshness, matches: [Match]) {
        let freshness = pageFreshness(text, shape: shape, today: today)
        return (freshness, match(text, shape: shape, source: source, freshness: freshness))
    }

    /// Turns the value into a deduplication key: "19.45" and "19:45" are the same time,
    /// "47,1329 TL" and "47,1329" are the same rate.
    static func normalizeValue(_ value: String, shape: SoughtShape) -> String {
        var d = simplify(value).replacingOccurrences(of: " ", with: "")
        switch shape {
        case .clock:
            d = d.replacingOccurrences(of: ".", with: ":")
        case .rate, .price, .distance:
            // Reduce to the numeric value: a unit/symbol difference must not count the same
            // value twice.
            if let number = resolveNumber(d) { d = String(number) }
        case .temperature, .date, .score, .none:
            break
        }
        return d
    }

    /// THE VALUE SANITY FILTER. Not every number matching the regex is a plausible answer.
    /// Out-of-range values are eliminated silently — they never reach the model.
    static func valueIsPlausible(_ value: String, shape: SoughtShape) -> Bool {
        switch shape {
        case .rate:
            // The range for rates and gram gold is wide but not infinite. Values close to 0
            // or in the millions are not a rate but some other number (e.g. a view count or
            // a product code).
            guard let number = resolveNumber(value) else { return false }
            return number > 0.0001 && number < 1_000_000
        case .temperature:
            // If it is the degree branch, check the range; if it is the condition text,
            // anything goes.
            guard value.rangeOfCharacter(from: .decimalDigits) != nil else { return true }
            guard let number = resolveNumber(value) else { return true }
            return number >= -60 && number <= 60
        case .score:
            // "12-14" may be a score but "2024-2026" is not; the pattern is already limited
            // to two digits, and here both sides are checked for a plausible goal count.
            let sides = value.split(whereSeparator: { !$0.isNumber })
            guard sides.count == 2, let a = Int(sides[0]), let b = Int(sides[1])
            else { return false }
            return a <= 30 && b <= 30
        case .clock, .price, .date, .distance, .none:
            return true
        }
    }

    /// Extracts a narrow context from the line the match was found on: at most `contextCap`
    /// characters with the match in the middle. Markdown link syntax and code fences are
    /// stripped (the model is prevented from copying a link off the page).
    static func extractContext(_ line: NSString, range: NSRange) -> String {
        let cap = contextCap
        guard line.length > cap else { return cleanContext(line as String) }

        let middle = range.location + range.length / 2
        var start = max(0, middle - cap / 2)
        if start + cap > line.length { start = line.length - cap }
        let chunk = line.substring(with: NSRange(location: start, length: cap))
        return cleanContext(chunk)
    }

    /// Neutralises the context: markdown link/fence characters go, whitespace collapses.
    /// Page text is untrusted; the surface that reaches the model is as narrow as possible.
    static func cleanContext(_ raw: String) -> String {
        var m = raw
        for char in ["[", "]", "(", ")", "`", "|", "*", "_", "<", ">"] {
            m = m.replacingOccurrences(of: char, with: " ")
        }
        return collapseSpaces(m).trimmingCharacters(in: .whitespaces)
    }

    // MARK: - The text that goes to the model (the 4096 budget)

    /// The filtered text returned to the model. THE RAW PAGE TEXT DOES NOT GO — only the
    /// matches and their narrow contexts. HARD-cut at `modelTextCap` characters in total.
    static func modelText(query: String,
                            shape: SoughtShape,
                            matches: [Match],
                            freshness: Freshness? = nil) -> String {
        guard !matches.isEmpty else { return notFoundText }
        // If the freshness was not given explicitly it is computed from the matches — so
        // that a caller who forgets does not silently get "current" assumed; the worst case
        // wins.
        let computed = freshness ?? overallFreshness(matches)

        // GROUP BY SOURCE. Every line used to carry its own source and the model printed it
        // faithfully: the same domain was repeated on all 22 of 22 times. The source is now
        // written per GROUP rather than per line, and named once more collectively at the
        // END of the list.
        //
        // The context is filtered too: in a timetable cell the context is most often the
        // value ITSELF, producing meaningless lines like "07:25 — 07:25". The context is
        // written only if it says more than the value does.
        // Values sharing THE SAME LINE carry that line ONCE.
        //
        // Measured bug: prayer times arrive on a single line ("İmsak 03:49 Güneş 05:41 Öğle
        // …"). Because every match carried its own context, the model saw the same line six
        // times, found it redundant and SUMMARISED it, ending with a "…" — the answer came
        // out truncated. Yet that line is itself the complete answer; it must be given once,
        // as it is.
        var groups: [(source: String, items: [String])] = []
        var seenContext = Set<String>()
        for e in matches.prefix(matchCap) {
            let plain = e.context.trimmingCharacters(in: .whitespacesAndNewlines)
            // If the context says no more than the value (a bare timetable cell) write only
            // the value; if it does, write the whole line, but only once.
            let same = plain.isEmpty || plain == e.value
                || plain.replacingOccurrences(of: e.value, with: "")
                       .trimmingCharacters(in: .whitespacesAndNewlines).count < 3
            let item: String
            if same {
                item = e.value
            } else {
                let key = "\(e.source)|\(plain)"
                if seenContext.contains(key) { continue }
                seenContext.insert(key)
                item = plain
            }
            if let i = groups.firstIndex(where: { $0.source == e.source }) {
                groups[i].items.append(item)
            } else {
                groups.append((e.source, [item]))
            }
        }
        guard !groups.isEmpty else { return notFoundText }

        let sources = groups.map(\.source).joined(separator: ", ")
        let start = "verbatim \(shape.englishName) extracted from web pages for "
            + "\"\(WebSearchClient.truncate(query, limit: 60))\". "
            + "These are the only values found; do not add, adjust or invent any others.\n"
        // The freshness warning comes BEFORE THE VALUES. Placed at the end, the 3B model
        // skipped the warning sitting behind the long-output rule.
        let warning = computed.modelWarning.map { "\($0)\n" } ?? ""
        let last = "\nSOURCES: \(sources)\n"
            + "Give ALL of the values above — every single one. Never abbreviate the list, "
            + "never end it with \"…\", \"etc.\" or \"and so on\", and never say the rest is "
            + "available elsewhere: this is the complete set and the user sees no other copy "
            + "of it. Do NOT repeat the source on every line — name the source(s) once, at "
            + "the end. Write domain names as plain text only; never build a markdown link "
            + "and never write a full URL."

        var body = ""
        for g in groups {
            let line = groups.count > 1
                ? "\(g.source): \(g.items.joined(separator: ", "))\n"
                : "\(g.items.joined(separator: ", "))\n"
            if start.count + warning.count + body.count + line.count + last.count > modelTextCap { break }
            body += line
        }
        guard !body.isEmpty else { return notFoundText }
        return start + warning + body + last
    }

    /// The shape was looked for but stayed below the threshold: NO CONTENT is given to the
    /// model. This constant is deliberately separate from "no_results" — there were pages,
    /// there was no value.
    static let notFoundText = "answer_not_found: the pages did not contain the "
        + "requested values. Tell the user plainly, in their own language, that you "
        + "could not find this information. Do not guess and do not state any value."

    // MARK: - Table (the DataStore channel)

    /// If the matches are regular (≥ `tableThreshold`) a table is produced in code. The
    /// model does not write the table itself; `ChatTable` already draws it.
    /// `@MainActor`: the `Table`/`Row` @Generable types stand in the default isolation
    /// (MainActor) and belong to another file. The explicit marker leaves this member's
    /// call contract unchanged while `AnswerFilter` becomes nonisolated.
    @MainActor
    static func table(_ matches: [Match], shape: SoughtShape) -> Table? {
        guard matches.count >= tableThreshold else { return nil }
        let headers = [shape.localName.isEmpty ? String(localized: "Value") : shape.localName,
                         String(localized: "Context"),
                         String(localized: "Source")]
        return Table(headers: headers,
                     rows: matches.map { Row(cells: [$0.value, $0.context, $0.source]) })
    }

    // MARK: - HTML → text (pure, zero dependencies)

    /// Blocks that carry no content into the text. Once the opening tag is seen everything
    /// up to its closing tag is skipped; if the closing tag never arrives, skipping runs to
    /// the end of the text (producing "silently half text" beats producing garbage on
    /// broken HTML).
    static let skippedBlocks: Set<String> = [
        "script", "style", "nav", "footer", "header", "aside", "form",
        "noscript", "svg", "head", "iframe", "template", "button", "select",
    ]

    /// Block tags that produce a line break.
    static let lineBreaking: Set<String> = [
        "p", "br", "div", "tr", "li", "h1", "h2", "h3", "h4", "h5", "h6",
        "td", "th", "table", "section", "article", "ul", "ol", "dd", "dt", "hr",
    ]

    /// Pure-Swift HTML→text extraction. A hand-written scanner, not a pile of regexes,
    /// because on broken/unclosed tags the regex approach either blows up or swallows the
    /// whole body. NO NEW DEPENDENCY IS ADDED (the project has zero dependencies).
    static func toText(_ html: String) -> String {
        var output = ""
        output.reserveCapacity(html.count / 4)

        var i = html.startIndex
        // The name of the skipped block (while non-nil everything is swallowed until
        // `</name>` is seen).
        var skipping: String?

        while i < html.endIndex {
            guard html[i] == "<" else {
                if skipping == nil { output.append(html[i]) }
                i = html.index(after: i)
                continue
            }
            // Find the end of the tag; if there is none (broken HTML) finish here.
            guard let closing = html[i...].firstIndex(of: ">") else { break }
            let body = String(html[html.index(after: i)..<closing])
            let isClosing = body.hasPrefix("/")
            let name = tagName(body)

            if let openTag = skipping {
                if isClosing && name == openTag { skipping = nil }
            } else if !isClosing, skippedBlocks.contains(name),
                      !body.hasSuffix("/") {
                skipping = name
            } else if lineBreaking.contains(name) {
                output.append("\n")
            }

            i = html.index(after: closing)
        }

        return simplifyLines(resolveEntities(output))
    }

    /// `<div class="x">` → "div", `</TR>` → "tr".
    static func tagName(_ body: String) -> String {
        var s = Substring(body)
        if s.hasPrefix("/") { s = s.dropFirst() }
        if s.hasPrefix("!") { return "" }
        let name = s.prefix { !$0.isWhitespace && $0 != "/" && $0 != ">" }
        return name.lowercased()
    }

    /// Resolves HTML entities: the common named ones + numeric (`&#160;`, `&#x27;`).
    static func resolveEntities(_ text: String) -> String {
        guard text.contains("&") else { return text }
        var m = text
        let named: [String: String] = [
            "&nbsp;": " ", "&amp;": "&", "&lt;": "<", "&gt;": ">", "&quot;": "\"",
            "&apos;": "'", "&#39;": "'", "&hellip;": "…", "&ndash;": "–",
            "&mdash;": "—", "&deg;": "°", "&euro;": "€", "&pound;": "£",
            "&laquo;": "«", "&raquo;": "»", "&rsquo;": "'", "&lsquo;": "'",
            "&ldquo;": "\"", "&rdquo;": "\"", "&middot;": "·", "&bull;": "•",
        ]
        for (k, v) in named { m = m.replacingOccurrences(of: k, with: v) }

        // Numeric entities. Because `&amp;` has already been resolved, what remains here is
        // genuine.
        guard m.contains("&#"),
              let engine = try? NSRegularExpression(pattern: "&#(x?)([0-9A-Fa-f]{1,6});",
                                                   options: [])
        else { return m }
        let ns = m as NSString
        var outcome = ""
        var last = 0
        for b in engine.matches(in: m, options: [], range: NSRange(location: 0, length: ns.length)) {
            outcome += ns.substring(with: NSRange(location: last, length: b.range.location - last))
            let isHex = !ns.substring(with: b.range(at: 1)).isEmpty
            let digits = ns.substring(with: b.range(at: 2))
            if let code = UInt32(digits, radix: isHex ? 16 : 10),
               let scalar = Unicode.Scalar(code) {
                outcome.append(Character(scalar))
            }
            last = b.range.location + b.range.length
        }
        outcome += ns.substring(from: last)
        return outcome
    }

    // MARK: - Text simplification

    /// Collapses multiple spaces into one (line breaks are not preserved — a single-line
    /// job).
    static func collapseSpaces(_ text: String) -> String {
        var outcome = ""
        var previousWasSpace = false
        for k in text {
            let isSpace = k.isWhitespace || k.isNewline
            if isSpace {
                if !previousWasSpace { outcome.append(" ") }
            } else {
                outcome.append(k)
            }
            previousWasSpace = isSpace
        }
        return outcome
    }

    /// Simplifies line by line: the inside of every line collapses to single spaces and
    /// empty lines drop. The line structure IS PRESERVED — context extraction feeds off the
    /// line.
    static func simplifyLines(_ text: String) -> String {
        text.components(separatedBy: .newlines)
            .map { collapseSpaces($0).trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }
            .joined(separator: "\n")
    }

    // MARK: - Query simplification

    /// Lowercasing + diacritic folding. A keyword match must not distinguish "kaçta" from
    /// "kacta".
    static func simplify(_ text: String) -> String {
        let small = text.lowercased()
        let unaccented = small.folding(options: [.diacriticInsensitive], locale: Locale(identifier: "en_US"))
        return collapseSpaces(unaccented).trimmingCharacters(in: .whitespaces)
    }

    /// Containment that respects word boundaries: the "hava" query must not catch
    /// "havaalanı", while multi-word hints ("hava durumu") must still work.
    static func containsWord(_ text: String, _ hint: String) -> Bool {
        if hint.contains(" ") { return text.contains(hint) }
        let words = text.split(whereSeparator: { !$0.isLetter && !$0.isNumber })
        return words.contains { $0 == Substring(hint) }
    }

    // MARK: - Page selection

    /// Ranks the pages to fetch: first the number of matches fitting the shape, then domain
    /// authority (official/institutional domains first). The model does not choose — the
    /// code does.
    static func candidatesToFetch(_ results: [WebResult], shape: SoughtShape) -> [WebResult] {
        let scored = results
            .filter { !$0.fullAddress.isEmpty }
            .enumerated()
            .map { (order, s) -> (WebResult, Int, Int) in
                let text = "\(s.title)\n\(s.summary)"
                let matching = match(text, shape: shape, source: s.domain).count
                return (s, rankScore(domain: s.domain, shape: shape,
                                         blurbMatches: matching), order)
            }
            .sorted {
                if $0.1 != $1.1 { return $0.1 > $1.1 }
                // On an equal score SearXNG's own order is preserved (a stable sort).
                return $0.2 < $1.2
            }
        // The CANDIDATE count is SEPARATE from the cap on pages to fetch. The candidate
        // list used to be clipped to `pageCap` here; because a dead page or one returning
        // 403 occupied its place in the list, the next healthy page never got its turn.
        // Measured case: the 1st result returned HTTP 500 and the page carrying the data was
        // 3rd and never entered the candidate list. How many pages are DOWNLOADED is bounded
        // by the calling loop (the successful-fetch count + the time budget).
        return scored.prefix(candidateCap).map(\.0)
    }

    /// THE RANK SCORE: the blurb match and the authority are weighed TOGETHER.
    ///
    /// The ranking used to look at the match count first and at the authority only on a
    /// tie. The measured result: a content farm whose blurb happened to contain two numbers
    /// beat the official site that actually owns the data. The reverse is dangerous too —
    /// an official site can return HTTP 500 (measured), so authority must not decide ON ITS
    /// OWN either. The two are summed.
    ///
    /// The match coefficient is 2: one blurb match is worth slightly more than one
    /// authority tier, but cannot topple an official source (6 points) on its own.
    static func rankScore(domain: String, shape: SoughtShape, blurbMatches: Int) -> Int {
        authority(domain) + shapeAuthority(domain, shape: shape) + blurbMatches * 2
    }

    /// Domain authority — coarse but deterministic. It is tuned for Turkey: official
    /// institutions and the sites that actually OWN the data go to the front, social media
    /// and app stores to the back.
    static func authority(_ domain: String) -> Int {
        let a = domain.lowercased()
        let root = a.hasPrefix("www.") ? String(a.dropFirst(4)) : a

        // The institutions that are the PRIMARY source of the data. These were seen in
        // search results during measurement and added by hand; it is an observation list,
        // not a guess.
        let primary = [
            "tcmb.gov.tr", "mgm.gov.tr", "diyanet.gov.tr", "namazvakitleri.diyanet.gov.tr",
            "resmigazete.gov.tr", "tuik.gov.tr", "sgk.gov.tr", "turkiye.gov.tr",
            "osym.gov.tr", "meb.gov.tr", "saglik.gov.tr", "epdk.gov.tr",
            "sehirhatlari.istanbul", "ido.com.tr", "iett.istanbul", "ibb.istanbul",
            "akom.ibb.istanbul", "tcddtasimacilik.gov.tr", "tcdd.gov.tr",
            "borsaistanbul.com", "darphane.gov.tr",
        ]
        if primary.contains(root) || primary.contains(where: { root.hasSuffix(".\($0)") }) {
            return 6
        }

        // Sites that carry no data and are most often a login wall. In measurement
        // instagram.com and play.google.com entered the top five of the candidate list and
        // ate the page-fetch budget; these are pushed to the back.
        let low = ["instagram.com", "facebook.com", "x.com", "twitter.com",
                     "pinterest.com", "youtube.com", "tiktok.com", "play.google.com",
                     "apps.apple.com", "linkedin.com", "reddit.com"]
        if low.contains(root) || low.contains(where: { root.hasSuffix(".\($0)") }) {
            return -4
        }

        if a.hasSuffix(".gov.tr") || a.hasSuffix(".gov") || a.hasSuffix(".bel.tr") { return 4 }
        if a.hasSuffix(".edu.tr") || a.hasSuffix(".edu") { return 2 }
        if a.hasSuffix(".org.tr") || a.hasSuffix(".org") { return 1 }
        if a.hasSuffix(".com.tr") || a.hasSuffix(".istanbul") { return 1 }
        return 0
    }

    /// SHAPE-SPECIFIC authority: asking the right question of the right institution. For a
    /// rate TCMB, for the weather MGM, for a ferry Şehir Hatları own the data; the general
    /// authority list cannot see that pairing.
    static func shapeAuthority(_ domain: String, shape: SoughtShape) -> Int {
        let a = domain.lowercased()
        let specialists: [String]
        switch shape {
        case .rate, .price:
            specialists = ["tcmb.gov.tr", "doviz.com", "investing.com", "bloomberght.com",
                        "bigpara.hurriyet.com.tr", "altinkaynak.com", "borsaistanbul.com"]
        case .temperature:
            specialists = ["mgm.gov.tr", "accuweather.com", "weather.com", "havadurumu15gunluk.net"]
        case .clock:
            specialists = ["sehirhatlari.istanbul", "ido.com.tr", "iett.istanbul",
                        "tcddtasimacilik.gov.tr", "namazvakitleri.diyanet.gov.tr",
                        "diyanet.gov.tr", "turktakvim.com", "namazvakti.com"]
        case .score:
            specialists = ["mackolik.com", "tff.org", "sporx.com", "flashscore.com"]
        case .date, .distance, .none:
            specialists = []
        }
        return specialists.contains(where: { a == $0 || a.hasSuffix(".\($0)") }) ? 3 : 0
    }
}
