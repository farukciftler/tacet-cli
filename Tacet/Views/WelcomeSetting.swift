//
//  WelcomeSetting.swift
//  Tacet
//
//  The persistence of the welcome (onboarding) flow and the single channel that
//  carries a job from the welcome screen into the chat. The key names live in ONE
//  place: a UserDefaults key typed out by hand in two different files silently
//  becomes two separate flags the moment one of the spellings changes.
//
//  Note: the plan put this file under `Services/`; in that phase there was only
//  permission to create new files under `Views/`, so it was moved here. The
//  content and the key names are as in the plan.
//

import Foundation
import Observation

enum WelcomeSetting {
    /// The user saw the REAL welcome screen — it never opens by itself again.
    static let completedKey = "tacet.welcome.completed"
    /// The day it was shown half-way because of a temporary block. It is retried the
    /// next day.
    static let lastDayKey = "tacet.welcome.lastDay"
    /// Has the tool-trace hint been seen (§4). Device-lifetime, it does not enter the
    /// SwiftData schema.
    static let chipHintKey = "tacet.hint.toolChip"

    /// Should the welcome screen be shown at launch.
    static var showAtLaunch: Bool {
        if UserDefaults.standard.bool(forKey: completedKey) { return false }
        return UserDefaults.standard.string(forKey: lastDayKey) != dayKey(Date())
    }

    /// Called when the sheet closes. `persistent` is true only if the user saw the REAL
    /// welcome screen (the model is ready) or the block is PERMANENT (the device is not
    /// suitable).
    static func markShown(persistent: Bool) {
        UserDefaults.standard.set(dayKey(Date()), forKey: lastDayKey)
        if persistent { UserDefaults.standard.set(true, forKey: completedKey) }
    }

    /// "Show again" from Settings: the tool-trace hint is set up again too.
    static func reset() {
        UserDefaults.standard.removeObject(forKey: completedKey)
        UserDefaults.standard.removeObject(forKey: lastDayKey)
        UserDefaults.standard.set(false, forKey: chipHintKey)
    }

    /// The day identity. No DateFormatter is set up: only equality is compared, the
    /// format does not need to be readable.
    private static func dayKey(_ t: Date) -> String {
        let c = Calendar.current.dateComponents([.year, .month, .day], from: t)
        return "\(c.year ?? 0)-\(c.month ?? 0)-\(c.day ?? 0)"
    }
}

// MARK: - Welcome → chat bridge

/// The single channel that carries the job picked in the welcome screen to the empty state.
///
/// Why a bridge: the one that will send the job is `ChatView`; `ContentView` cannot reach
/// its input field directly, and in this phase that file belongs to another agent. The
/// bridge reuses the path the empty state's example chips ALREADY use (writing into the
/// input field) — that is, no new presentation channel is opened.
@MainActor
@Observable
final class WelcomeBridge {
    static let shared = WelcomeBridge()
    private init() {}

    /// The job text waiting to be picked up by the empty state.
    private(set) var pendingPrompt: String?

    func release(_ text: String) {
        let clean = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !clean.isEmpty else { return }
        pendingPrompt = clean
    }

    /// Single shot: the job that is read drops at the same moment, it is never delivered
    /// a second time.
    func consume() -> String? {
        defer { pendingPrompt = nil }
        return pendingPrompt
    }
}
