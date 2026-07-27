//
//  LanguagePreference.swift
//  Tacet
//
//  Language preferences (reply language + interface language). The two are
//  separate: the user can keep the interface in Turkish and ask for replies in
//  English. The preferences are persisted in UserDefaults; because this is a
//  service it uses UserDefaults directly, not @AppStorage.
//

import Foundation
import Observation

@MainActor
@Observable
final class LanguagePreference {

    static let shared = LanguagePreference()

    // RENAMED WITH THE ENGLISH MIGRATION. These are PERSISTED KEYS, so a rename
    // drops the language choice of every existing install back to "automatic".
    // That is accepted here because the loss is one visible, one-time re-pick in
    // Settings — unlike the Keychain service string (Keychain.swift), which is
    // frozen because the secret behind it can never be shown or retyped again.
    private static let replyKey = "tacet.replyLanguage"
    private static let uiKey = "tacet.uiLanguage"

    /// "" = automatic (detect the language that was written). Otherwise a BCP47
    /// code, e.g. "en".
    var replyLanguage: String {
        didSet {
            guard replyLanguage != oldValue else { return }
            UserDefaults.standard.set(replyLanguage, forKey: Self.replyKey)
        }
    }

    /// "" = device language. Otherwise a BCP47 code.
    var uiLanguage: String {
        didSet {
            guard uiLanguage != oldValue else { return }
            UserDefaults.standard.set(uiLanguage, forKey: Self.uiKey)
            applyUiLanguage()
        }
    }

    /// The interface language that is ACTUALLY in effect as the process starts.
    /// The bundle's localizations are resolved once; the language we see on
    /// screen is this value, and it does not change even when the selection does.
    private let initialUiLanguage: String

    /// So that we can say on screen, in one plain sentence, that a restart is
    /// needed after the interface language changed — we do not silently promise a
    /// half-applied language.
    /// Not a persisted flag but a derived value: if the user goes back to the
    /// first language no pending change is left and the note disappears by itself.
    var uiRestartPending: Bool {
        uiLanguage != initialUiLanguage
    }

    /// The 9 supported languages; each name written in its own language.
    static let choices: [(code: String, name: String)] = [
        ("tr", "Türkçe"),
        ("en", "English"),
        ("de", "Deutsch"),
        ("es", "Español"),
        ("fr", "Français"),
        ("ja", "日本語"),
        ("ko", "한국어"),
        ("pt-BR", "Português (Brasil)"),
        ("zh-Hans", "简体中文"),
    ]

    /// That Locale if an interface language is selected, otherwise nil (the
    /// device language applies).
    var uiLocale: Locale? {
        uiLanguage.isEmpty ? nil : Locale(identifier: uiLanguage)
    }

    private init() {
        let d = UserDefaults.standard
        replyLanguage = d.string(forKey: Self.replyKey) ?? ""
        let registeredUi = d.string(forKey: Self.uiKey) ?? ""
        uiLanguage = registeredUi
        initialUiLanguage = registeredUi
        applyUiLanguage()
    }

    /// AppleLanguages decides which .lproj the bundle picks; this key is what
    /// makes the interface open in the selected language on the next launch.
    private func applyUiLanguage() {
        let d = UserDefaults.standard
        if uiLanguage.isEmpty {
            d.removeObject(forKey: "AppleLanguages")
        } else {
            d.set([uiLanguage], forKey: "AppleLanguages")
        }
    }
}
