import Foundation
import EventKit
import FoundationModels

// The calendar tool: reading and adding events through EventKit.
// Reading drops a grey chip (.readOk), adding a green one (.written).
struct CalendarTool: TacetTool {
    let name = "calendar"
    let description = """
    Reads calendar events and adds new ones. Call this whenever the user asks about \
    their schedule ("what's on tomorrow", "my week", "am I free Friday") or asks to add \
    an event ("add a meeting Friday at 2pm"), in any language. action="read" to read, "add" to add.
    ONLY for real calendar events. If the user is adding or deleting a ROW, COLUMN or \
    SECTION of a document, table or list, use the document tools instead — a weekday name \
    on its own ("Wednesday - Pizza") is NOT a calendar request.
    """
    weak var reporter: (any ToolReporter)?
    /// The bulk-data transport channel — every event read is stored here and only a ref goes to the model.
    weak var dataStore: DataStore?

    /// The action is NOT free text any more (P0-4). MEASURED FAILURE: a model writing in
    /// English produced "add", `action.lowercased().contains("ekle")` returned false and the
    /// flow fell SILENTLY into the reading branch — the user thought "I added it" and found
    /// nothing in the calendar. With an enum this value becomes impossible to produce.
    @Generable
    enum Action: String, Equatable, CaseIterable {
        case read
        case add
    }

    @Generable struct Arguments {
        @Guide(description: "The operation to perform: read the calendar, or add an event.")
        var action: Action
        // The ISO instruction lives in ONE place: it used to be repeated word for word in the
        // start/end @Guides (a waste of tokens, audit §5.2).
        @Guide(description: "Start of the range for 'read', or the event time for 'add'. ISO 8601 in LOCAL time, no timezone suffix: write \"2026-07-20T13:00\", never \"2026-07-20T13:00Z\" or \"+03:00\". Resolve relative wording ('tomorrow', 'next Friday') yourself; call the time tool first if you need today's date. Required for 'add'.")
        var start: String?
        @Guide(description: "End of the range or of the event, same local ISO format, no timezone suffix. Leave empty if unknown.")
        var end: String?
        @Guide(description: "Event title for \"add\", e.g. \"Dentist\".")
        var title: String?
    }

    func call(arguments: Arguments) async -> String {
        // An exhaustive switch instead of a fuzzy `.contains`: if a new action is added the
        // compiler points here, and no branch is left silently falling back to reading.
        let icon: String
        let runningText: String
        switch arguments.action {
        case .add:
            icon = "calendar.badge.plus"
            runningText = L10n.addingEvent
        case .read:
            icon = "calendar"
            runningText = L10n.checkingCalendar
        }
        let rawInput = [arguments.action.rawValue, arguments.start, arguments.end, arguments.title]
            .compactMap { $0 }
            .joined(separator: " · ")

        return await runWithChip(icon: icon, runningText: runningText, rawInput: rawInput) {
            let store = EKEventStore()

            // Authorization through a single gate (PermissionGate). The scope narrows with the
            // action: a user who only adds an event is not asked for permission to read the
            // WHOLE calendar — iOS 17's write-only permission exists for exactly this split.
            let scope: PermissionGate.CalendarScope = arguments.action == .add ? .write : .read
            let permission = try await PermissionGate.calendar(store, scope: scope)
            if let cause = permission.toModel {
                return ToolOutcome(chipText: L10n.calendarPermission,
                                   state: .permissionRequired,
                                   toModel: cause)
            }

            // Everything past this point is REAL calendar access; we tie its result to the
            // taint flag (mcp §5.6). A permission denial already returned above — data that
            // could not be reached does not taint the session.
            switch arguments.action {
            case .add:
                let added = try Self.add(store: store, arguments: arguments)
                return await taintIfSucceeded(added)
            case .read:
                let (raw, table) = Self.read(store: store, arguments: arguments)
                let result = await taintIfSucceeded(raw)
                // The bulk-data channel: put every record into the store and return only a ref
                // to the model. That way, for jobs like "dump the calendar into excel", the
                // data never enters the context window.
                if let table, table.rows.count > 1, let dataStore = dataStore {
                    let ref = dataStore.put(table, tag: "calendar")
                    return ToolOutcome(
                        chipText: result.chipText,
                        state: result.state,
                        // Data/facts only; do not write imperative instructions (the model parrots them).
                        toModel: result.toModel + " (all \(table.rows.count) records ready, data_ref=\(ref))",
                        rawOutput: result.rawOutput)
                }
                return result
            }
        }
    }

    // Reading: summarizes the events in the range (for the model) and returns the full table (for the store).
    private static func read(store: EKEventStore, arguments: Arguments) -> (ToolOutcome, Table?) {
        // If a time was given but cannot be resolved, return an error rather than reading the
        // WRONG range and saying "your calendar is empty"; the model retries with ISO.
        let start: Date
        switch resolveRequired(arguments.start) {
        case .error(let s): return (s, nil)
        case .ok(let t): start = t
        case .missing: start = Calendar.current.startOfDay(for: Date())
        }

        let end: Date
        switch resolveRequired(arguments.end) {
        case .error(let s): return (s, nil)
        case .ok(let t): end = t
        case .missing: end = Calendar.current.date(byAdding: .day, value: 7, to: start)!
        }

        let predicate = store.predicateForEvents(withStart: start, end: end, calendars: nil)
        let all = store.events(matching: predicate).sorted { $0.startDate < $1.startDate }

        // The formats follow the device language — if the user works in Japanese they must not
        // see a Turkish month name in Excel's "Date" column (shared: TimeFormat).
        let clockFormat = TimeFormat.clock
        let fullFormat = TimeFormat.dayClock
        let dayFormat = TimeFormat.day

        if all.isEmpty {
            return (ToolOutcome(chipText: L10n.calendarReadEmpty,
                                state: .readOk,
                                toModel: "no_events_in_range",
                                rawOutput: String(localized: "No events found.")), nil)
        }

        // Only a short summary of the first ~10 goes to the model (the context budget — spec §7.2).
        let preview = Array(all.prefix(10))
        let summary = preview
            .map { "\(clockFormat.string(from: $0.startDate)) \($0.title ?? "Event")" }
            .joined(separator: "; ")
        let raw = preview
            .map { "\(fullFormat.string(from: $0.startDate)) — \($0.title ?? "Event")" }
            .joined(separator: "\n")

        // The FULL table that goes into the store (every record, structured columns).
        let rows = all.map { e -> Row in
            Row(cells: [
                dayFormat.string(from: e.startDate),
                clockFormat.string(from: e.startDate),
                clockFormat.string(from: e.endDate),
                e.title ?? "Event",
                e.location ?? "",
            ])
        }
        let table = Table(headers: ["Date", "Start", "End", "Title", "Location"],
                          rows: rows)

        return (ToolOutcome(chipText: L10n.calendarRead(all.count),
                            state: .readOk,
                            toModel: summary,
                            rawOutput: raw), table)
    }

    // Adding: creates a new event and saves it.
    private static func add(store: EKEventStore, arguments: Arguments) throws -> ToolOutcome {
        // If the time cannot be resolved, DO NOT ADD THE EVENT AT THE CURRENT MOMENT. Better
        // that the model retries with ISO than that the user sees "set" and finds the event at
        // the wrong time in the calendar.
        guard let resolution = TimeResolver.resolve(arguments.start) else {
            return ToolOutcome(
                chipText: timeNotUnderstood,
                state: .failed(timeNotUnderstood),
                toModel: "error: unparsable_or_missing_start_time"
                    + (arguments.start.map { " \"\($0)\"" } ?? "")
                    + ". Nothing was created. Call the tool again with \"start\" as an "
                    + "ISO 8601 timestamp, e.g. 2026-07-20T13:00."
            )
        }
        let start = resolution.date

        let end: Date
        switch resolveRequired(arguments.end) {
        case .error(let s): return s
        case .ok(let t): end = t
        case .missing: end = Calendar.current.date(byAdding: .hour, value: 1, to: start)!
        }
        let title = arguments.title?.isEmpty == false ? arguments.title! : String(localized: "Event")

        let event = EKEvent(eventStore: store)
        event.title = title
        event.startDate = start
        event.endDate = end
        event.calendar = store.defaultCalendarForNewEvents

        try store.save(event, span: .thisEvent)

        let fullFormat = TimeFormat.dayClock

        return ToolOutcome(chipText: L10n.eventAdded,
                           state: .written,
                           toModel: "event_added",
                           rawOutput: "\(title) — \(fullFormat.string(from: start))")
    }

    // Time parsing lives in the shared `TimeResolver` (ReminderTool.swift) —
    // language-independent: ISO 8601 → fixed patterns → Turkish shorthand →
    // Locale.current → NSDataDetector. All on-device, no network.

    /// For fields that may be left empty but MUST resolve if they are given.
    private enum RequiredOutcome {
        case missing                // the field is empty — the caller may use its default
        case ok(Date)
        case error(ToolOutcome)
    }

    private static func resolveRequired(_ text: String?) -> RequiredOutcome {
        guard let raw = text?.trimmingCharacters(in: .whitespacesAndNewlines), !raw.isEmpty else {
            return .missing
        }
        guard let resolution = TimeResolver.resolve(raw) else {
            return .error(ToolOutcome(
                chipText: timeNotUnderstood,
                state: .failed(timeNotUnderstood),
                toModel: "error: unparsable_time \"\(raw)\". Nothing was read or created. "
                    + "Call the tool again with ISO 8601 timestamps, e.g. 2026-07-20T13:00."
            ))
        }
        return .ok(resolution.date)
    }

    static var timeNotUnderstood: String { String(localized: "Couldn’t understand the time") }
}
