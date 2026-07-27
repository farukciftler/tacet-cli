//
//  PermissionSection.swift
//  Tacet
//
//  The permissions section inside Settings. It only shows: the tools ask for the
//  permission the moment the user actually wants something — a settings screen opening
//  a permission dialog would be asking for something the user did not want.
//

import SwiftUI
import EventKit
import Contacts
import AVFoundation
import Speech

struct PermissionSection: View {
    @Environment(\.scenePhase) private var scenePhase

    @State private var calendar: PermissionState = .unknown
    @State private var reminder: PermissionState = .unknown
    @State private var contacts: PermissionState = .unknown
    /// The two permissions for writing by voice. There is NO notifications row: after the
    /// duty feature was removed the app schedules no notifications and asks for no
    /// permission — a row permanently showing "Not asked yet" would give the user false
    /// information.
    @State private var microphone: PermissionState = .unknown
    @State private var speech: PermissionState = .unknown

    init() {}

    /// The honest description sitting under the card; the title and the frame are
    /// Settings' job.
    static let description: LocalizedStringKey =
        "Permissions are used only the moment you ask for something; what is read does not go out without your approval. Change them in iOS Settings."

    var body: some View {
        VStack(spacing: 0) {
            row(name: String(localized: "Calendar"), state: calendar)
            separator
            row(name: String(localized: "Reminders"), state: reminder)
            separator
            row(name: String(localized: "Contacts"), state: contacts)
            separator
            row(name: String(localized: "Microphone"), state: microphone)
            separator
            row(name: String(localized: "Speech recognition"), state: speech)
            separator
            settingsRow
        }
        .task { await refresh() }
        .onChange(of: scenePhase) { _, new in
            // The screen must not stay stale when the user comes back from iOS Settings.
            if new == .active { Task { await refresh() } }
        }
    }

    // MARK: - Rows

    private var separator: some View {
        Rectangle()
            .fill(Palette.divider)
            .frame(height: Spacing.hairline)
    }

    private func row(name: String, state: PermissionState) -> some View {
        HStack(spacing: Spacing.s2) {
            Text(name)
                .font(Typography.user())
                .foregroundStyle(Palette.ink)
            Spacer(minLength: Spacing.s3)
            // The glyph is for the eye only; the word next to it already says the state.
            Image(systemName: state.glyph)
                .font(Typography.chip())
                .foregroundStyle(Palette.muted)
                .accessibilityHidden(true)
            Text(state.name)
                .font(Typography.chip())
                .foregroundStyle(Palette.grey)
        }
        .padding(.vertical, Spacing.s3)
        .padding(.horizontal, Spacing.s4)
        // "Calendar, Not granted" is read as one piece; it is not navigated as three
        // separate elements.
        .accessibilityElement(children: .combine)
    }

    private var settingsRow: some View {
        Button {
            if let url = URL(string: UIApplication.openSettingsURLString) {
                UIApplication.shared.open(url)
            }
        } label: {
            HStack(spacing: Spacing.s2) {
                Text("Open iOS Settings")
                    .font(Typography.user())
                    .foregroundStyle(Palette.ink)
                Spacer(minLength: Spacing.s3)
                Image(systemName: "arrow.up.forward")
                    .font(Typography.chip())
                    .foregroundStyle(Palette.muted)
                    .accessibilityHidden(true)
            }
            .padding(.vertical, Spacing.s3)
            .padding(.horizontal, Spacing.s4)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .combine)
        .accessibilityHint(Text("Leaves the app and opens iOS Settings"))
    }

    // MARK: - Reading

    private func refresh() async {
        calendar = PermissionState(EKEventStore.authorizationStatus(for: .event))
        reminder = PermissionState(EKEventStore.authorizationStatus(for: .reminder))
        contacts = PermissionState(CNContactStore.authorizationStatus(for: .contacts))
        microphone = PermissionState(AVAudioApplication.shared.recordPermission)
        speech = PermissionState(SFSpeechRecognizer.authorizationStatus())
    }
}

// MARK: - State

/// Reduces four different system enums to a single state told in one word.
private enum PermissionState {
    case granted, notGranted, notAsked, restricted, unknown

    var name: String {
        switch self {
        case .granted:    String(localized: "Granted")
        case .notGranted: String(localized: "Not granted")
        case .notAsked:   String(localized: "Not asked yet")
        case .restricted: String(localized: "Restricted")
        case .unknown:    String(localized: "Reading…")
        }
    }

    /// A colourless glyph — the state is told in words, the glyph is for the eye only.
    var glyph: String {
        switch self {
        case .granted:    "checkmark"
        case .notGranted: "xmark"
        case .notAsked:   "minus"
        case .restricted: "lock"
        case .unknown:    "ellipsis"
        }
    }

    init(_ state: EKAuthorizationStatus) {
        switch state {
        case .fullAccess:    self = .granted
        case .writeOnly:     self = .restricted
        case .notDetermined: self = .notAsked
        case .restricted:    self = .restricted
        case .denied:        self = .notGranted
        @unknown default:    self = .unknown
        }
    }

    init(_ state: CNAuthorizationStatus) {
        switch state {
        case .authorized:    self = .granted
        case .limited:       self = .restricted
        case .notDetermined: self = .notAsked
        case .restricted:    self = .restricted
        case .denied:        self = .notGranted
        @unknown default:    self = .unknown
        }
    }

    init(_ state: AVAudioApplication.recordPermission) {
        switch state {
        case .granted:      self = .granted
        case .denied:       self = .notGranted
        case .undetermined: self = .notAsked
        @unknown default:   self = .unknown
        }
    }

    init(_ state: SFSpeechRecognizerAuthorizationStatus) {
        switch state {
        case .authorized:    self = .granted
        case .denied:        self = .notGranted
        case .notDetermined: self = .notAsked
        case .restricted:    self = .restricted
        @unknown default:    self = .unknown
        }
    }
}
