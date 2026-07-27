//
//  PermissionGate.swift
//  Tacet
//
//  THE single place for the Calendar / Reminders / Contacts permission flow (spec §7.3).
//  The ~15 lines that were copy-pasted into three tools moved here; the copies drifting
//  apart (one accepting `.limited` while another does not) is no longer possible.
//
//  The rule did not change: `denied`/`restricted` are PERMANENT — asking again on every
//  call is pointless, so a permission chip is returned directly. The system prompt is only
//  shown in the `notDetermined` case.
//

import Foundation
import EventKit
import Contacts

enum PermissionGate {
    /// The permission outcome. A permanent denial is kept SEPARATE from a denial at the
    /// prompt: only on a permanent denial can the model say "turn it on in Settings"; on
    /// the first one the prompt will appear again.
    enum Outcome {
        case granted
        /// denied / restricted — the user or device management turned it off permanently.
        case persistentDenial
        /// The system prompt was shown and the user did not grant it.
        case promptDenial

        /// The English constant returned to the model (the model context is English; the
        /// localized chip text must not leak to the model). None for `granted` — the flow continues.
        var toModel: String? {
            switch self {
            case .granted:          nil
            case .persistentDenial: "permission_denied (user must grant access in Settings)"
            case .promptDenial:     "permission_required (user can grant access in Settings)"
            }
        }
    }

    /// What is going to be done in the calendar? The breadth of access narrows accordingly.
    enum CalendarScope {
        /// Reading events — full access is required.
        case read
        /// Only adding events — iOS 17's write-only permission is enough.
        case write
    }

    /// Calendar permission.
    ///
    /// Asking a user who only ADDS an event for permission to READ the whole calendar is
    /// needlessly broad; `requestWriteOnlyAccessToEvents()` exists for exactly this split.
    /// If reading is requested later the state becomes `.writeOnly` and the reading branch
    /// shows the upgrade prompt itself.
    static func calendar(_ store: EKEventStore, scope: CalendarScope) async throws -> Outcome {
        let state = EKEventStore.authorizationStatus(for: .event)
        if state == .denied || state == .restricted { return .persistentDenial }
        if state == .fullAccess { return .granted }

        if scope == .write {
            // The narrow permission is enough for writing; do not ask again.
            if state == .writeOnly { return .granted }
            if state == .notDetermined {
                let granted = try await store.requestWriteOnlyAccessToEvents()
                return granted ? .granted : .promptDenial
            }
        }

        // The reading branch, and a read request arriving on top of `.writeOnly`: full access.
        let granted = try await store.requestFullAccessToEvents()
        return granted ? .granted : .promptDenial
    }

    /// Reminders permission. EventKit has NO write-only EQUIVALENT for reminders (the API
    /// only offers it for events), hence the single branch.
    static func reminders(_ store: EKEventStore) async throws -> Outcome {
        let state = EKEventStore.authorizationStatus(for: .reminder)
        if state == .denied || state == .restricted { return .persistentDenial }
        if state == .fullAccess { return .granted }
        let granted = try await store.requestFullAccessToReminders()
        return granted ? .granted : .promptDenial
    }

    /// Contacts permission. `.limited` (iOS 18 partial address book) COUNTS as access — the
    /// user has shared the contacts they picked, and the tool searches within those.
    static func contacts(_ store: CNContactStore) async throws -> Outcome {
        let state = CNContactStore.authorizationStatus(for: .contacts)
        if state == .denied || state == .restricted { return .persistentDenial }
        if state == .notDetermined {
            let granted = try await store.requestAccess(for: .contacts)
            return granted ? .granted : .promptDenial
        }
        return .granted
    }
}
