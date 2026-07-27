//
//  TacetShortcuts.swift
//  Tacet
//
//  The App Shortcut provider (intent-plan §3). WITHOUT THIS FILE THE FEATURE IS DEAD: the
//  intents compile, the metadata is produced, but Siri recognizes no phrase, Spotlight shows
//  no action, and the build still stays green.
//
//  What it provides (on its own, with no extra setup):
//   · the actions appearing in Spotlight when the app or an action name is typed
//   · a "Tacet" tile in the Shortcuts gallery
//   · Siri recognizing the phrases below without any setup
//   · being listed under Settings > Siri > Shortcuts
//
//  The phrases are written ONLY IN THE SOURCE LANGUAGE (en); the phrases for the other eight
//  languages come from `AppShortcuts.xcstrings`. English phrases used to be added into the
//  array by hand — that trick saves only two languages and leaves the remaining seven without
//  phrases.
//
//  EVERY PHRASE LITERAL BELOW IS A KEY INTO `AppShortcuts.xcstrings`. The 13 literals here and
//  the 13 keys in that catalog were moved from Turkish to English IN THE SAME STEP and must
//  stay byte-identical — including the typographic apostrophe in "${applicationName}’s last
//  document". Edit one side alone and the phrase silently loses all eight localizations; the
//  build stays green, so nothing tells you.
//
//  A PHRASE KEY ALSO EMBEDS THE PARAMETER NAME: the two `${format}` keys were keyed on the
//  Turkish parameter name `bicim` before it was renamed, which is why re-keying the catalog
//  was not optional. Renaming an intent parameter re-keys its phrases too.
//
//  THE TYPE NAME IS A PERMANENT IDENTITY: the provider's name also enters the compiled
//  metadata; because this provider carried the previous brand name, the migration was completed
//  before release. The old name lives only in the git history and is not written here.
//
//  There are FOUR shortcuts; the system limit is 10. `OpenChatIntent` is DELIBERATELY left out:
//  its parameter is an `AppEntity`, and if it were embedded in a phrase the chat titles would be
//  carried onto the Siri surface (§2.1, the second of the three bans).
//
//  NOT DONE: chat indexing via `CSSearchableItem` and donation via `IntentDonationManager` (§7).
//  The reasons in order: copying the same data into another system database where the
//  `.completeUnlessOpen` protection DOES NOT apply; and the fact that `SearchNotesTool`
//  (search_notes) searches exactly in Spotlight — if Tacet indexed its own chats it would serve
//  its own old answers back to the model as "the user's notes".
//

import AppIntents

nonisolated struct TacetShortcuts: AppShortcutsProvider {

    /// The system offers no neutral grey; this is the chromatically dullest option. The closest
    /// compromise to the "NO accent colour" rule — not exact, a deliberate deviation.
    static var shortcutTileColor: ShortcutTileColor { .grayBlue }

    static var appShortcuts: [AppShortcut] {
        // 1 — Ask. HISTORICAL NOTE, kept because it explains the shape of this list: while the
        // source language was Turkish there was a fourth, apostrophe-free fallback phrase,
        // because Siri matching gets weaker on the Turkish apostrophe. English needs no such
        // fallback, so the slot is spent on a plain synonym instead.
        AppShortcut(
            intent: AskTacetIntent(),
            phrases: [
                "Ask \(.applicationName)",
                "Ask \(.applicationName) something",
                "Ask \(.applicationName) a question",
                "Talk to \(.applicationName)",
            ],
            shortTitle: "Ask",
            systemImageName: "bubble.left")

        // 2 — Document. The format is the only parameter embedded in the phrase (AppEnum).
        AppShortcut(
            intent: GenerateDocumentIntent(),
            phrases: [
                "Create a \(\.$format) with \(.applicationName)",
                "Make a \(\.$format) in \(.applicationName)",
                "Create a document with \(.applicationName)",
            ],
            shortTitle: "Document",
            systemImageName: "doc.badge.plus")

        // 3 — Note. The verb "remind" is DELIBERATELY not used: it clashes with `ReminderTool`
        // semantics and the user would think a reminder had been set.
        AppShortcut(
            intent: AddNoteIntent(),
            phrases: [
                "Add a note to \(.applicationName)",
                "Have \(.applicationName) remember this",
                "Take a note in \(.applicationName)",
            ],
            shortTitle: "Note",
            systemImageName: "text.badge.plus")

        // 4 — Last document. The only intent that takes data out of the app; its gate is in
        // Settings.
        AppShortcut(
            intent: ProvideDocumentIntent(),
            phrases: [
                "\(.applicationName)’s last document",
                "Get the last file from \(.applicationName)",
                "Get the file \(.applicationName) made",
            ],
            shortTitle: "Last document",
            systemImageName: "arrow.up.doc")
    }
}
