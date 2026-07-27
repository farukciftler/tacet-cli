//
//  L10n.swift
//  Tacet
//
//  The single localization point for user-visible text produced in code (tool chips, state
//  and error messages, date separators). Resolved from Localizable.xcstrings according to the
//  device language (9 languages). The tools pull their text from here.
//
//  THE LITERALS BELOW ARE CATALOG KEYS. They and the keys in Localizable.xcstrings moved from
//  Turkish to English in the same step; change one side alone and the string falls back to the
//  key itself in all nine languages while the build stays green.
//

import Foundation

enum L10n {
    // MARK: - Chip "running" texts
    static var checkingCalendar: String { String(localized: "Checking calendar…") }
    static var addingEvent: String { String(localized: "Adding event…") }
    static var settingReminder: String { String(localized: "Setting reminder…") }
    static var searchingContacts: String { String(localized: "Searching contacts…") }
    static var searchingNotes: String { String(localized: "Searching notes…") }
    static var calculating: String { String(localized: "Calculating…") }
    static var lookingForDocument: String { String(localized: "Looking for document…") }
    static func creatingDocument(_ b: String) -> String { String(localized: "Creating \(b)…") }
    static func readingDocument(_ b: String) -> String { String(localized: "Reading \(b)…") }
    static func editingDocument(_ b: String) -> String { String(localized: "Editing \(b)…") }

    // MARK: - Chip "done" texts
    static func calendarRead(_ n: Int) -> String { String(localized: "Calendar read · \(n) events") }
    static var calendarReadEmpty: String { String(localized: "Calendar read · empty") }
    static var eventAdded: String { String(localized: "Event added") }
    static var calendarPermission: String { String(localized: "Calendar permission needed") }
    static func reminderSet(clock: String?) -> String {
        if let clock { return String(localized: "Reminder set · \(clock)") }
        return String(localized: "Reminder set")
    }
    static var reminderPermission: String { String(localized: "Reminder permission needed") }
    static var contactsSearched: String { String(localized: "Contacts searched") }
    static var contactsSearchedNone: String { String(localized: "Contacts searched · no results") }
    static var contactsPermission: String { String(localized: "Contacts permission needed") }
    static func notesSearched(_ n: Int) -> String { String(localized: "Notes searched · \(n) results") }
    static var notesSearchedNone: String { String(localized: "Notes searched · no results") }
    static var calculated: String { String(localized: "Calculated") }
    static var countingDays: String { String(localized: "Counting days…") }
    static var daysCounted: String { String(localized: "Days counted") }
    static var dateNotUnderstood: String { String(localized: "Date not understood") }
    static func documentCreated(_ b: String, _ name: String) -> String { String(localized: "\(b) created · \(name)") }
    static func documentEdited(_ b: String, _ name: String) -> String { String(localized: "\(b) edited · \(name)") }
    static func documentRead(_ b: String, _ name: String) -> String { String(localized: "\(b) read · \(name)") }
    static var noDocumentToEdit: String { String(localized: "No document to edit") }
    static var noSharedDocument: String { String(localized: "No shared document") }

    // MARK: - Model state/error messages
    // Each one says "what happened + what to do"; they are SPLIT by availability cause so they
    // cannot contradict each other (preparing ≠ will never work).
    /// modelNotReady — the model is downloading; this is temporary, it will work shortly.
    static var modelPreparing: String {
        String(localized: "The model is still downloading. Try again in a few minutes.")
    }
    /// appleIntelligenceNotEnabled — the user can fix it, so describe the path.
    static var appleIntelligenceClosed: String {
        String(localized: "Apple Intelligence is off. Turn it on in Settings > Apple Intelligence & Siri, then try again.")
    }
    /// deviceNotEligible — permanent; say it plainly so nobody waits for nothing.
    static var deviceNotEligible: String {
        String(localized: "This device can’t run the on-device model; Tacet can’t produce replies here.")
    }
    /// The error after a side effect has already happened: NO retry was made, because it would
    /// have repeated the side effect.
    static var errorAfterWrite: String {
        String(localized: "I couldn’t finish the reply, but the change I made along the way was saved. I didn’t retry so as not to do the same thing twice — check it and ask again if needed.")
    }
    static var previousFinishing: String { String(localized: "One moment, finishing the previous reply.") }
    static var outOfBounds: String { String(localized: "I can't do that; it's outside my limits.") }
    static var languageUnsupported: String { String(localized: "I don't fully support this language right now.") }
    static var tryAgain: String { String(localized: "I couldn't do that just now. Could you ask again?") }

    // MARK: - Fallback sentences by error class
    // A single "I couldn't do that right now" was covering three separate faults: the user
    // could tell neither what had happened nor whether there was anything they could do about
    // it. Each sentence says WHAT happened but gives NO INTERNAL DETAIL — no error text, line
    // number, tool name or model name appears in any of them.
    /// All that is left is a half-finished tool call: the model never formed the sentence.
    static var answerNotAssembled: String {
        String(localized: "I couldn’t pull the answer together — I’d rather not show you something half-finished. Could you ask again?")
    }
    /// A tool failed during the turn and the model said nothing about it. The chip already
    /// shows WHAT failed; the sentence says we are not inventing anything and leaves the
    /// detail to the chip.
    static var toolFailedReply: String {
        String(localized: "I tried, but this step didn’t work; I’d rather say so than invent the result. Tap the step above to see what happened.")
    }
    /// The context overflowed: there IS something concrete the user can do, so say it.
    static var conversationTooLong: String {
        String(localized: "This conversation has grown too long for me. If you start a new chat and ask again, I can do it.")
    }

    // MARK: - Running code (code-spec §5)
    static var runningCode: String { String(localized: "Running code…") }
    static func codeRun(_ ms: Int) -> String { String(localized: "Code run · \(ms) ms") }
    static var codeRetrying: String { String(localized: "Error · retrying") }
    static var codeCouldNotRun: String { String(localized: "Code couldn’t be run") }
    static var codeTimedOut: String { String(localized: "Code timed out") }
    static var codeTimeoutCause: String { String(localized: "The code didn’t finish within 3 seconds.") }
    static var codeRetryLimit: String { String(localized: "Retry limit reached") }
    static var codeTwoAttempts: String { String(localized: "The code didn’t work in two attempts.") }
    static var codeOutputTruncated: String { String(localized: "… output was trimmed at 10,000 characters.") }

    // MARK: - Page verification (code-spec §4.3)
    static var pageNotVerified: String { String(localized: "The page couldn’t be verified") }
    static var pageDidNotLoad: String { String(localized: "The page didn’t load within 3 seconds") }
    static var pageOutsideRequest: String { String(localized: "The page made a request to an address outside the file") }
    static func pageCouldNotLoad(_ error: String) -> String { String(localized: "The page couldn’t be loaded: \(error)") }
    static func pageScriptError(_ message: String) -> String { String(localized: "Script error on the page: \(message)") }

    // MARK: - Connection (MCP) chips
    // These are user-visible like every other chip; while they were left as bare Turkish
    // literals they were translated in no language at all.
    static func connectionRunning(_ name: String) -> String { String(localized: "\(name) · running…") }
    static func connectionDetailed(_ name: String, _ detail: String) -> String { String(localized: "\(name) · \(detail)") }
    static func connectionInterrupted(_ name: String) -> String { String(localized: "\(name) · stopped partway") }
    static func connectionUnreachable(_ name: String) -> String { String(localized: "\(name) · couldn’t be reached") }
    static func connectionNotSent(_ name: String) -> String { String(localized: "\(name) · not sent") }
    static func connectionAwaitingApproval(_ name: String) -> String { String(localized: "\(name) · waiting for approval") }
    static var interrupted: String { String(localized: "Stopped partway.") }

    // MARK: - Date separators
    static var today: String { String(localized: "TODAY") }
    static var yesterdayUpper: String { String(localized: "YESTERDAY") }
    static var yesterday: String { String(localized: "Yesterday") }
}
