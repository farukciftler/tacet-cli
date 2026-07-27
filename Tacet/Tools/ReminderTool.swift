//
//  ReminderTool.swift
//  Tacet
//
//  The reminder tool (spec §7.3). Reading + writing on top of EventKit Reminders.
//  The model supplies type-safe arguments, not free text; Swift parses the time.
//  No network — only the local EKEventStore.
//

import Foundation
import FoundationModels
import EventKit

// MARK: - Language-independent time resolution

/// The shared time resolver of the tools.
///
/// The product speaks 9 languages; if date parsing only knows Turkish, the other languages
/// get a SILENT DATA ERROR (the event is set at the wrong time). That is why resolution is
/// layered and language-independent:
///   1. Strict ISO 8601 (the format we expect from the model)
///   2. Language-neutral fixed patterns (en_US_POSIX)
///   3. Turkish shorthand (the native language — a fast path, not the only path)
///   4. Date/time styles through Locale.current (whatever the device language is)
///   5. NSDataDetector — a system component, on-device, no network; the privacy promise holds
///
/// If none of them match, `nil`. The caller NEVER falls back silently to "now"; it returns an error.
enum TimeResolver {
    /// The resolved instant + whether the text carried an explicit clock time.
    struct Resolution {
        var date: Date
        /// If false, only the day was resolved (the clock time is the default/start of day).
        var hasClock: Bool
    }

    /// Turns text into a date. nil if it cannot be resolved.
    static func resolve(_ raw: String?) -> Resolution? {
        guard let firstText = raw?.trimmingCharacters(in: .whitespacesAndNewlines),
              !firstText.isEmpty else { return nil }
        let text = forceLocalClock(firstText)

        // 1) Strict ISO 8601 — since the timezone suffix was dropped above, in practice this
        //    step only catches leftover shapes; it is deliberately kept as the head of the chain.
        if let date = try? Date(text, strategy: .iso8601) {
            return Resolution(date: date, hasClock: true)
        }

        // 2) Language-neutral fixed patterns. The ones with a clock time are tried first.
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = Calendar.current.timeZone
        for pattern in patternsWithClock {
            formatter.dateFormat = pattern
            if let date = formatter.date(from: text) { return Resolution(date: date, hasClock: true) }
        }
        for pattern in patternsWithoutClock {
            formatter.dateFormat = pattern
            if let date = formatter.date(from: text) { return Resolution(date: date, hasClock: false) }
        }

        // 3) Turkish shorthand — "bugün 18:00", "yarın", "öbür gün 9".
        //    The literals stay Turkish: they are USER INPUT TOKENS, not app text — the same
        //    category as the nine languages in `newYearNames` below. English relative wording
        //    is already covered by step 6 (the system data detector); Turkish is not, which is
        //    why this step exists at all. Deleting it would silently misdate Turkish input.
        if let resolution = turkishShorthand(text) { return resolution }

        // 4) Named recurring days ("yılbaşı", "new year"). The CODE resolves these, NOT the
        //    model: this is a calendar computation and the model's idea of the year is
        //    unreliable. MEASURED BUG: on 26 July 2026, asked "how many days until new year",
        //    the model resolved the target to 2025-01-01 and produced "-571 days" and (on a
        //    second pass at the same question) a made-up "39 days".
        if let resolution = namedDay(text) { return resolution }

        // 5) The device language's own date formats ("7/20/26, 6:00 PM", "20.07.2026" …).
        if let resolution = localFormat(text) { return resolution }

        // 6) Last resort: the system's data detector. It is local and uses no network.
        if let resolution = detector(text) { return resolution }

        return nil
    }

    private static let patternsWithClock = [
        // Milliseconds are tried first: DateFormatter will not match with characters left
        // over, so without the "…:00.500" pattern that shape would fall out of the chain.
        "yyyy-MM-dd'T'HH:mm:ss.SSS", "yyyy-MM-dd HH:mm:ss.SSS",
        "yyyy-MM-dd'T'HH:mm:ss", "yyyy-MM-dd'T'HH:mm",
        "yyyy-MM-dd HH:mm:ss", "yyyy-MM-dd HH:mm",
        "yyyy/MM/dd HH:mm", "dd.MM.yyyy HH:mm", "dd/MM/yyyy HH:mm",
    ]
    private static let patternsWithoutClock = [
        "yyyy-MM-dd", "yyyy/MM/dd", "dd.MM.yyyy", "dd/MM/yyyy",
    ]

    /// DROPS the timezone suffix (`Z` or `±hh:mm`) from an ISO timestamp.
    ///
    /// MEASURED SILENT DATA ERROR: small models append `Z` reflexively as soon as they hear
    /// "ISO 8601". Because `Date(_:strategy:.iso8601)` reads that as UTC, a request for
    /// "at 13:00" in Istanbul was written into the calendar as 16:00. The model does not mean
    /// real UTC — it means the time the user SAID. So the suffix is dropped and the remainder
    /// is parsed as LOCAL time.
    ///
    /// The match is deliberately NARROW: only a complete ISO date-time stamp from start to
    /// end. A loose suffix pattern is NOT USED, so that in free text ("between 12:30-14:00")
    /// the trailing `-14:00` is not mistaken for a timezone and trimmed away.
    private static func forceLocalClock(_ text: String) -> String {
        let whole = #"^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}(:\d{2})?(\.\d+)?([Zz]|[+-]\d{2}:?\d{2})$"#
        guard text.range(of: whole, options: .regularExpression) != nil,
              let suffix = text.range(of: #"([Zz]|[+-]\d{2}:?\d{2})$"#,
                                      options: .regularExpression) else { return text }
        return String(text[..<suffix.lowerBound])
    }

    /// Is there an explicit clock trace in the text ("18:00", "6 pm", "18.30")?
    /// Used to tell whether a clock time was actually given in the local-format and
    /// detector results.
    static func clockTrace(_ text: String) -> Bool {
        let lower = text.lowercased()
        if lower.range(of: #"\d{1,2}\s*[:.]\s*\d{2}"#, options: .regularExpression) != nil { return true }
        if lower.range(of: #"\d\s*(am|pm|öö|ös)"#, options: .regularExpression) != nil { return true }
        return false
    }

    /// Resolves named recurring days to their first occurrence AFTER TODAY.
    ///
    /// Only the "new year" family, which is fixed to 1 January, is handled. New years tied to
    /// a lunar calendar (설날, 春节) are DELIBERATELY LEFT OUT: their dates change every year
    /// and resolving them wrong would be a silent data error — failing and asking is more honest.
    ///
    /// If today is 1 January it returns today (difference 0 = "today"), not the following year.
    private static func namedDay(_ text: String) -> Resolution? {
        // ı/İ are mapped by hand FIRST: the dotless ı is not an "accented i" but a separate
        // base letter — `.diacriticInsensitive` does not convert it, and "yılbaşı" would
        // become "yılbası" and not match the list. İ is mapped first too, because
        // `lowercased()` turns it into a composed dotted i and breaks the match.
        let plain = text
            .replacingOccurrences(of: "ı", with: "i")
            .replacingOccurrences(of: "İ", with: "i")
            .lowercased()
            .folding(options: .diacriticInsensitive, locale: Locale(identifier: "en_US_POSIX"))
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard newYearNames.contains(where: { plain.contains($0) }) else { return nil }

        let calendar = Calendar.current
        let today = calendar.startOfDay(for: Date())
        var components = calendar.dateComponents([.year], from: today)
        components.month = 1
        components.day = 1
        guard let thisYear = calendar.date(from: components) else { return nil }
        if thisYear >= today { return Resolution(date: thisYear, hasClock: false) }
        guard let next = calendar.date(byAdding: .year, value: 1, to: thisYear) else { return nil }
        return Resolution(date: next, hasClock: false)
    }

    /// Written in their post-accent-folding shapes ("yılbaşı" → "yilbasi").
    private static let newYearNames = [
        "yilbasi", "yeni yil",                       // tr
        "new year", "new years", "new year's",       // en
        "neujahr", "silvester",                      // de
        "nouvel an", "jour de l'an",                 // fr
        "ano nuevo", "nochevieja",                   // es
        "ano novo", "reveillon",                     // pt
        "capodanno",                                 // it
        "元日", "元旦",                                // ja / zh
        "신정",                                       // ko (solar calendar; 설날 is lunar and is NOT included)
    ]

    /// Turkish relative day + optional clock time.
    /// The matched strings are user input, not UI text — see the note at step 3.
    private static func turkishShorthand(_ raw: String) -> Resolution? {
        let text = raw.lowercased()
        let calendar = Calendar.current

        var dayOffset = 0
        var daySpecified = false
        if text.contains("öbür gün") || text.contains("obur gun") {
            dayOffset = 2; daySpecified = true
        } else if text.contains("yarın") || text.contains("yarin") {
            dayOffset = 1; daySpecified = true
        } else if text.contains("bugün") || text.contains("bugun") {
            dayOffset = 0; daySpecified = true
        }
        guard daySpecified else { return nil }

        // The clock time: "18:00" / "18.00", or — since the day is specified — safely a bare
        // single number ("yarın 9").
        var hour: Int?
        var minute = 0
        if let range = text.range(of: #"(\d{1,2})[:.](\d{2})"#, options: .regularExpression) {
            let parts = text[range].replacingOccurrences(of: ".", with: ":").split(separator: ":")
            hour = Int(parts[0]); minute = Int(parts[1]) ?? 0
        } else if let range = text.range(of: #"(?<!\d)(\d{1,2})(?!\d)"#, options: .regularExpression) {
            hour = Int(text[range])
        }

        guard let targetDay = calendar.date(byAdding: .day, value: dayOffset, to: Date()) else { return nil }
        let startOfDay = calendar.startOfDay(for: targetDay)
        if let h = hour, (0...23).contains(h) {
            let date = calendar.date(bySettingHour: h, minute: minute, second: 0, of: startOfDay) ?? startOfDay
            return Resolution(date: date, hasClock: true)
        }
        return Resolution(date: startOfDay, hasClock: false)
    }

    /// The device language's own short/medium/long date-time formats.
    private static func localFormat(_ text: String) -> Resolution? {
        let styles: [(DateFormatter.Style, DateFormatter.Style)] = [
            (.short, .short), (.medium, .short), (.long, .short), (.full, .short),
            (.short, .none), (.medium, .none), (.long, .none), (.full, .none),
        ]
        let formatter = DateFormatter()
        formatter.locale = Locale.current
        formatter.timeZone = Calendar.current.timeZone
        for (dateStyle, timeStyle) in styles {
            formatter.dateStyle = dateStyle
            formatter.timeStyle = timeStyle
            if let date = formatter.date(from: text) {
                return Resolution(date: date, hasClock: timeStyle != .none)
            }
        }
        return nil
    }

    /// NSDataDetector: resolves free expressions like "next friday at 6", "明日 18時",
    /// "mañana a las 9" with the system's own language data. Entirely local.
    private static func detector(_ text: String) -> Resolution? {
        guard let dataDetector = try? NSDataDetector(types: NSTextCheckingResult.CheckingType.date.rawValue)
        else { return nil }
        let range = NSRange(text.startIndex..., in: text)
        guard let match = dataDetector.firstMatch(in: text, options: [], range: range),
              let date = match.date else { return nil }
        // When no clock time is given the detector invents a default one; if there is no clock
        // trace in the text, count it as "no clock time" and leave it to the caller.
        return Resolution(date: date, hasClock: clockTrace(text))
    }
}

// MARK: - Shared date formats

/// The `DateFormatter` setups shared by the Calendar and Reminder tools.
/// All of them use `Locale.current`: if the user works in Japanese, a Turkish month name must
/// not show up in Excel's "Date" column.
///
/// Deliberately `var`, not `let` — it is built fresh on every call. A single cached instance
/// would bring the risk of getting stuck in the OLD language after iOS's in-app language
/// change; these paths run about once a minute, so the setup cost is irrelevant.
enum TimeFormat {
    /// "14:30" — clock time only.
    static var clock: DateFormatter {
        let f = DateFormatter()
        f.locale = Locale.current
        f.dateFormat = "HH:mm"
        return f
    }

    /// "20 Jul 14:30" — day + clock time, in the device language's ordering.
    static var dayClock: DateFormatter {
        let f = DateFormatter()
        f.locale = Locale.current
        f.setLocalizedDateFormatFromTemplate("d MMM HH:mm")
        return f
    }

    /// "20 Jul 2026" — day only.
    static var day: DateFormatter {
        let f = DateFormatter()
        f.locale = Locale.current
        f.setLocalizedDateFormatFromTemplate("d MMM yyyy")
        return f
    }

    /// A medium date + short clock time using the system styles ("20 Jul 2026 14:30").
    static var mediumDateClock: DateFormatter {
        let f = DateFormatter()
        f.locale = Locale.current
        f.dateStyle = .medium
        f.timeStyle = .short
        return f
    }
}

// MARK: - Tool

struct ReminderTool: TacetTool {
    let name = "reminder"
    let description = """
    Creates a reminder (a to-do / task) or lists pending reminders. Call this whenever the \
    user asks to be reminded of something, in any language (e.g. 'remind me to call at 6pm', \
    'remind me to buy milk tomorrow'), or asks what is on their to-do list ('what are my \
    pending reminders'). action="create" to create, "list" to list. For a fixed appointment use \
    the calendar tool instead.
    """

    weak var reporter: (any ToolReporter)?
    /// The bulk-data transport channel — the listed reminders are stored here and only a ref goes to the model.
    weak var dataStore: DataStore?

    /// The action is NOT free text (P0-4). `contains("oku") || contains("liste")` returned
    /// false for the English "list" and the flow fell SILENTLY into the creation branch: while
    /// the user was asking for their list, an attempt was made to create a titleless reminder.
    /// An enum makes this value impossible to produce.
    @Generable
    enum Action: String, Equatable, CaseIterable {
        case create
        case list
    }

    @Generable
    struct Arguments {
        @Guide(description: "The operation to perform: create a reminder, or list the pending ones.")
        var action: Action
        @Guide(description: "Title of the reminder for 'create'; short and action-oriented, e.g. 'Call Ali', 'Buy milk'.")
        var title: String?
        @Guide(description: "When to be reminded. ISO 8601 in LOCAL time, no timezone suffix: write \"2026-07-20T18:00\", never \"2026-07-20T18:00Z\" or \"+03:00\". Resolve relative wording ('tomorrow', 'tonight') yourself; call the time tool first if you need today's date. Leave empty if no time was asked for.")
        var time: String?
    }

    func call(arguments: Arguments) async -> String {
        // Exhaustive switch: no fuzzy matching, no silent wrong branch.
        let icon: String
        let runningText: String
        switch arguments.action {
        case .list:
            icon = "checklist"
            runningText = Self.checkingReminders
        case .create:
            icon = "bell"
            runningText = L10n.settingReminder
        }
        let rawInput = [arguments.action.rawValue, arguments.title, arguments.time]
            .compactMap { $0 }
            .joined(separator: " · ")

        return await runWithChip(icon: icon, runningText: runningText, rawInput: rawInput) {
            let store = EKEventStore()

            // Authorization through a single gate (PermissionGate): denied/restricted is
            // permanent — asking again on every call is pointless.
            let permission = try await PermissionGate.reminders(store)
            if let cause = permission.toModel {
                return ToolOutcome(chipText: L10n.reminderPermission,
                                   state: .permissionRequired,
                                   toModel: cause)
            }

            // Permission is past; real reminder access happens here. If the result is `.readOk`
            // or `.written` the session is tainted (mcp §5.6); the path that returns `.failed`
            // because the time could not be resolved does not taint it.
            switch arguments.action {
            case .list:
                let listed = await Self.list(store: store, dataStore: dataStore)
                return await taintIfSucceeded(listed)
            case .create:
                let created = try Self.create(store: store, arguments: arguments)
                return await taintIfSucceeded(created)
            }
        }
    }

    // MARK: Creation

    private static func create(store: EKEventStore, arguments: Arguments) throws -> ToolOutcome {
        let title = arguments.title?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !title.isEmpty else {
            return ToolOutcome(chipText: titleMissing,
                               state: .failed(titleMissing),
                               toModel: "error: missing_title. Call the tool again with a short title in \"title\".")
        }

        // If a time was given it MUST resolve. Silently creating a timeless reminder out of an
        // unresolvable one means telling the user "it's set" and then never reminding them.
        var components: DateComponents?
        if let raw = arguments.time?.trimmingCharacters(in: .whitespacesAndNewlines), !raw.isEmpty {
            guard let resolution = TimeResolver.resolve(raw) else {
                return ToolOutcome(
                    chipText: timeNotUnderstood,
                    state: .failed(timeNotUnderstood),
                    toModel: "error: unparsable_time \"\(raw)\". Nothing was created. "
                        + "Call the tool again with \"time\" as an ISO 8601 timestamp, e.g. 2026-07-20T18:00."
                )
            }
            let calendar = Calendar.current
            components = resolution.hasClock
                ? calendar.dateComponents([.year, .month, .day, .hour, .minute], from: resolution.date)
                : calendar.dateComponents([.year, .month, .day], from: resolution.date)
        }

        guard let calendarList = store.defaultCalendarForNewReminders() else {
            throw ReminderError.noCalendar
        }

        let reminder = EKReminder(eventStore: store)
        reminder.title = title
        reminder.calendar = calendarList
        reminder.dueDateComponents = components

        try store.save(reminder, commit: true)

        let clockText = clockText(components)
        let raw = title + (arguments.time.map { " — \($0)" } ?? " — no time")
        return ToolOutcome(chipText: L10n.reminderSet(clock: clockText),
                           state: .written,
                           toModel: "reminder_created",
                           rawOutput: raw)
    }

    // MARK: Reading

    /// The flat, Sendable counterpart of EKReminder (EventKit objects cannot cross the actor boundary).
    private struct Pending: Sendable {
        var title: String
        var time: Date?
        var list: String
    }

    /// Lists the pending (incomplete) reminders.
    /// The context budget (spec §7.2): only the count + the first few titles go to the model,
    /// the full list is put into the DataStore and a ref goes back to the model.
    private static func list(store: EKEventStore, dataStore: DataStore?) async -> ToolOutcome {
        let predicate = store.predicateForIncompleteReminders(
            withDueDateStarting: nil, ending: nil, calendars: nil)
        // EKReminder is not Sendable — let only flat data cross the continuation boundary.
        let calendar = Calendar.current
        let reminders: [Pending] = await withCheckedContinuation { continuation in
            store.fetchReminders(matching: predicate) { result in
                let flat = (result ?? []).map { r in
                    Pending(title: r.title ?? "-",
                            time: r.dueDateComponents.flatMap { calendar.date(from: $0) },
                            list: r.calendar?.title ?? "")
                }
                continuation.resume(returning: flat)
            }
        }

        if reminders.isEmpty {
            return ToolOutcome(chipText: remindersReadEmpty,
                               state: .readOk,
                               toModel: "no_pending_reminders",
                               rawOutput: String(localized: "No pending reminders."))
        }

        // The nearest in time first; the timeless ones last.
        let ordered = reminders.sorted { ($0.time ?? .distantFuture) < ($1.time ?? .distantFuture) }

        // The format follows the device language — no hard-coded tr_TR (shared: TimeFormat).
        let fullFormat = TimeFormat.mediumDateClock

        func timeText(_ r: Pending) -> String {
            guard let t = r.time else { return "" }
            return fullFormat.string(from: t)
        }

        // Only a short summary of the first ~10 goes to the model.
        let preview = Array(ordered.prefix(10))
        let summary = preview
            .map { r -> String in
                let t = timeText(r)
                return t.isEmpty ? r.title : "\(r.title) (\(t))"
            }
            .joined(separator: "; ")
        let raw = preview
            .map { r -> String in
                let t = timeText(r)
                return t.isEmpty ? "• \(r.title)" : "• \(r.title) — \(t)"
            }
            .joined(separator: "\n")

        let result = ToolOutcome(chipText: remindersRead(ordered.count),
                                 state: .readOk,
                                 toModel: "\(ordered.count) pending: \(summary)",
                                 rawOutput: raw)

        // The bulk-data channel: the full list into the store, only a ref to the model.
        if ordered.count > 1, let dataStore {
            let rows = ordered.map { r in
                Row(cells: [r.title, timeText(r), r.list])
            }
            let table = Table(headers: ["Title", "Time", "List"], rows: rows)
            let ref = dataStore.put(table, tag: "reminder")
            return ToolOutcome(chipText: result.chipText,
                               state: result.state,
                               toModel: result.toModel
                                 + " (all \(ordered.count) records ready, data_ref=\(ref))",
                               rawOutput: result.rawOutput)
        }
        return result
    }

    // MARK: - Texts
    // Note: in this phase L10n.swift belongs to another agent; the new keys are defined here
    // with String(localized:) — they enter the String Catalog automatically.

    static var checkingReminders: String { String(localized: "Checking reminders…") }
    static func remindersRead(_ n: Int) -> String {
        String(localized: "Reminders read · \(n) pending")
    }
    static var remindersReadEmpty: String { String(localized: "Reminders read · empty") }
    static var timeNotUnderstood: String { String(localized: "Couldn’t understand the time") }
    static var titleMissing: String { String(localized: "Title missing") }

    enum ReminderError: LocalizedError, ToolErrorCode {
        case noCalendar
        var errorDescription: String? { String(localized: "No reminder list found") }
        /// The model has exactly one useful reaction: telling the user that there is no list.
        /// The cause is spelled out so it does not repeat the same call.
        var errorCode: String { "no_reminder_list_on_device" }
    }

    /// Returns the clock time inside dueDateComponents as "HH.mm"; nil if there is no time.
    static func clockText(_ component: DateComponents?) -> String? {
        guard let hour = component?.hour else { return nil }
        let minute = component?.minute ?? 0
        return String(format: "%02d.%02d", hour, minute)
    }
}
