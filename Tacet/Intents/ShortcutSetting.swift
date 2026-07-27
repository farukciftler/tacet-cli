//
//  ShortcutSetting.swift
//  Tacet
//
//  The gate for giving files to Shortcuts (intent-plan §4.2). The pattern is exactly the same
//  as `WebSearchSetting`: OFF BY DEFAULT, a single flag in UserDefaults, and the view layer
//  binds to the same key with @AppStorage.
//
//  The reason: `ProvideDocumentIntent` is the only surface that takes a real file OUTSIDE the
//  app. An automation built in Shortcuts can carry that file to email, to the cloud, to the
//  internet. Tacet does not make this decision on the user's behalf — but it does not say "ask
//  every time" either (§4.3): consent is taken ONCE, inside the app, and what went out stays
//  visible in Settings after the fact.
//
//  NO new SwiftData field (rule 6): the export record is a single line in UserDefaults, it
//  requires no schema migration.
//

import Foundation

nonisolated enum ShortcutSetting {

    // MARK: - UserDefaults keys
    // View layer: @AppStorage(ShortcutSetting.exportKey)
    //
    // THE KEY STRINGS ARE UNCHANGED AND MUST STAY THAT WAY. They are on-disk identities: if
    // they were renamed along with the property names, `UserDefaults.bool` would fall back to
    // `false` on an existing install and a user who had turned export on would silently find
    // it off again. The Swift names are English, the persisted keys are the old ones.

    static let exportKey = "kisayol.disaVerim"
    static let lastGivenKey = "kisayol.sonVerilen"

    // MARK: - The gate

    /// Can files be given to Shortcuts? The default is `false`; if the key was never written,
    /// `UserDefaults.bool` already returns `false`.
    static var exportEnabled: Bool {
        get { UserDefaults.standard.bool(forKey: exportKey) }
        set { UserDefaults.standard.set(newValue, forKey: exportKey) }
    }

    // MARK: - The export record

    /// A single line: "<ISO date>|<file name>". The separator is THE FIRST "|"; since "|" does
    /// not occur in an ISO date, having one in the file name does not corrupt the record.
    static func recordExport(name: String, date: Date) {
        let iso = ISO8601DateFormatter().string(from: date)
        UserDefaults.standard.set("\(iso)|\(name)", forKey: lastGivenKey)
    }

    /// The last export, read by the Settings screen. `nil` if nothing was ever given.
    static var lastGiven: (name: String, date: Date)? {
        guard let raw = UserDefaults.standard.string(forKey: lastGivenKey),
              let separator = raw.firstIndex(of: "|") else { return nil }
        let isoText = String(raw[raw.startIndex..<separator])
        let name = String(raw[raw.index(after: separator)...])
        guard !name.isEmpty, let date = ISO8601DateFormatter().date(from: isoText) else { return nil }
        return (name, date)
    }

    /// Deleting the record — Settings can call this if the user turns export off.
    static func deleteRecord() {
        UserDefaults.standard.removeObject(forKey: lastGivenKey)
    }
}
