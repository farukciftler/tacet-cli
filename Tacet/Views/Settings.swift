//
//  Settings.swift
//  Tacet
//
//  The settings page — language preferences, clearing the chat history and
//  permissions. Three sections, tag headings between them; no decoration.
//

import SwiftUI

struct Settings: View {
    var clearHistory: () -> Void
    /// The context needed to delete generated documents. If not given, the documents
    /// section is hidden.
    var documentContext: DocumentContext?
    /// Opens the connections board through the root presentation channel (mcp §3.1).
    var openConnections: () -> Void
    /// Reopens the welcome screen through the root presentation channel.
    var openWelcome: () -> Void

    @Environment(\.dismiss) private var close

    private let language = LanguagePreference.shared
    @State private var capabilitiesOpen = false
    @State private var clearApproval = false
    @State private var documentDeleteApproval = false
    /// The total size of Documents/Tacet — measured at launch and after deleting.
    @State private var documentSize: Int64 = 0

    // MARK: - Web search state (web-search-spec §3.1)

    /// The raw text in the field. This is NOT the SAVED address: the address is written to
    /// `WebSearchSetting` only after a successful test — that is the "mandatory test".
    @State private var searchURL: String = WebSearchSetting.rawRoot
    /// The raw on/off flag. Because `WebSearchSetting.isActive` returns false while the
    /// address is invalid, the key itself is read here.
    @AppStorage(WebSearchSetting.activeKey) private var searchOn = false
    @State private var search: SearchAttempt = .pending
    /// The running test — cancelled when the page closes.
    @State private var searchTask: Task<Void, Never>?
    /// The saved address reflected into the view (we cannot observe a static setting).
    @State private var savedAddress: String = WebSearchSetting.rawRoot

    // MARK: - Shortcuts state (intent-plan §4.2)

    /// The gate for giving files to Shortcuts. Default OFF; this is the only place to turn
    /// it on.
    @AppStorage(ShortcutSetting.exportKey) private var exportOn = false
    /// The record of the last file given. Because UserDefaults cannot be observed, it is
    /// read by hand at launch and after turning it off.
    @State private var lastGiven: (name: String, date: Date)?

    /// The state of the "Test the server" step. On failure the cause is carried in plain
    /// language; no error is swallowed silently.
    enum SearchAttempt: Equatable {
        case pending
        case testing
        /// The number of results the sample query returned.
        case succeeded(Int)
        case failed(String)

        /// Only a failed attempt is written with `Palette.error`.
        var isError: Bool {
            if case .failed = self { return true }
            return false
        }
    }

    init(clearHistory: @escaping () -> Void,
         documentContext: DocumentContext? = nil,
         openConnections: @escaping () -> Void = {},
         openWelcome: @escaping () -> Void = {}) {
        self.clearHistory = clearHistory
        self.documentContext = documentContext
        self.openConnections = openConnections
        self.openWelcome = openWelcome
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: Spacing.s5) {
                    gettingStartedSection
                    languageSection
                    dataSection
                    if documentContext != nil { documentsSection }
                    webSearchSection
                    connectionsSection
                    shortcutsSection
                    permissionsSection
                    footnote
                }
                .padding(.horizontal, Spacing.s5)
                .padding(.vertical, Spacing.s4)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .background(Palette.background)
            .sheet(isPresented: $capabilitiesOpen) { Capabilities() }
            .task {
                measureDocumentSize()
                lastGiven = ShortcutSetting.lastGiven
            }
            .onDisappear {
                // Work that does not depend on the view's lifetime ends with the page.
                searchTask?.cancel()
                searchTask = nil
            }
            .navigationTitle("Settings")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Close") { close() }
                        .font(Typography.chip())
                        .foregroundStyle(Palette.grey)
                }
            }
        }
    }

    // MARK: - Getting started

    /// The welcome screen and the capability catalogue. The section heading is
    /// deliberately "GETTING STARTED": the product name is never written in capitals.
    private var gettingStartedSection: some View {
        section("GETTING STARTED") {
            VStack(alignment: .leading, spacing: Spacing.s3) {
                VStack(spacing: 0) {
                    navigationRow("What Tacet can do",
                                  hint: String(localized: "Opens the full list of what Tacet can do.")) {
                        capabilitiesOpen = true
                    }

                    separator

                    navigationRow("Show the welcome again",
                                  hint: String(localized: "Brings back the first-launch welcome screen and the tool-trace hint.")) {
                        WelcomeSetting.reset()
                        openWelcome()
                    }
                }
                .hairlineFrame()

                Text("Brings back the first-launch welcome screen and the tool-trace hint.")
                    .font(Typography.chip())
                    .foregroundStyle(Palette.muted)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    /// A navigation row with a chevron — exactly the same shape as the `dataSection` button.
    private func navigationRow(_ title: LocalizedStringKey,
                               hint: String,
                               action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: Spacing.s2) {
                Text(title)
                    .font(Typography.user())
                    .foregroundStyle(Palette.ink)
                    .multilineTextAlignment(.leading)
                Spacer(minLength: 0)
                Image(systemName: "chevron.right")
                    .font(Typography.iconSmall())
                    .foregroundStyle(Palette.muted)
                    .accessibilityHidden(true)
            }
            .padding(.vertical, Spacing.s3)
            .padding(.horizontal, Spacing.s4)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text(title))
        .accessibilityHint(Text(hint))
    }

    // MARK: - Language

    private var languageSection: some View {
        section("LANGUAGE") {
            VStack(spacing: 0) {
                row(title: "Reply language",
                    subtitle: language.replyLanguage.isEmpty
                        ? String(localized: "matches the language you write in")
                        : nil,
                    selection: name(language.replyLanguage, automatic: String(localized: "Automatic"))) {
                    Button { language.replyLanguage = "" } label: { Text("Automatic") }
                    ForEach(LanguagePreference.choices, id: \.code) { choice in
                        Button { language.replyLanguage = choice.code } label: { Text(choice.name) }
                    }
                }

                separator

                row(title: "Interface language",
                    subtitle: language.uiRestartPending
                        ? String(localized: "The new language appears after you quit and reopen Tacet.")
                        : nil,
                    selection: name(language.uiLanguage, automatic: String(localized: "Device language"))) {
                    Button { language.uiLanguage = "" } label: { Text("Device language") }
                    ForEach(LanguagePreference.choices, id: \.code) { choice in
                        Button { language.uiLanguage = choice.code } label: { Text(choice.name) }
                    }
                }
            }
            .hairlineFrame()
        }
    }

    private func name(_ code: String, automatic: String) -> String {
        guard !code.isEmpty else { return automatic }
        return LanguagePreference.choices.first { $0.code == code }?.name ?? code
    }

    private func row<Choices: View>(title: LocalizedStringKey,
                                    subtitle: String?,
                                    selection: String,
                                    @ViewBuilder choices: () -> Choices) -> some View {
        VStack(alignment: .leading, spacing: Spacing.s1) {
            Menu {
                choices()
            } label: {
                HStack(spacing: Spacing.s2) {
                    Text(title)
                        .font(Typography.user())
                        .foregroundStyle(Palette.ink)
                    Spacer(minLength: 0)
                    Text(selection)
                        .font(Typography.chip())
                        .foregroundStyle(Palette.grey)
                        .lineLimit(1)
                    Image(systemName: "chevron.up.chevron.down")
                        .font(Typography.iconSmall())
                        .foregroundStyle(Palette.muted)
                        .accessibilityHidden(true)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            // Read as a single selection control, not as three separate pieces
            // ("Reply language", "Automatic", chevron).
            .accessibilityElement(children: .combine)
            .accessibilityLabel(Text(title))
            .accessibilityValue(Text(selection))
            .accessibilityHint(Text("Double-tap to open the options."))

            if let subtitle {
                Text(subtitle)
                    .font(Typography.chip())
                    .foregroundStyle(Palette.muted)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.vertical, Spacing.s3)
        .padding(.horizontal, Spacing.s4)
    }

    // MARK: - Data

    private var dataSection: some View {
        section("DATA") {
            Button { clearApproval = true } label: {
                HStack(spacing: Spacing.s2) {
                    Text("Clear all chat history")
                        .font(Typography.user())
                        .foregroundStyle(Palette.ink)
                        .multilineTextAlignment(.leading)
                    Spacer(minLength: 0)
                    Image(systemName: "chevron.right")
                        .font(Typography.iconSmall())
                        .foregroundStyle(Palette.muted)
                        .accessibilityHidden(true)
                }
                .padding(.vertical, Spacing.s3)
                .padding(.horizontal, Spacing.s4)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(Text("Clear all chat history"))
            .accessibilityHint(Text("You’ll be asked to confirm. Conversations and messages are deleted; memory is kept."))
            .hairlineFrame()
            .confirmationDialog("Clear all chat history?",
                                isPresented: $clearApproval,
                                titleVisibility: .visible) {
                Button("Clear", role: .destructive) { clearHistory() }
                Button("Cancel", role: .cancel) { }
            } message: {
                // Memory is NOT DELETED here (memory-spec §7): chats and memory are
                // separate decisions. Not saying so would be a silent lie.
                Text("Chats and the messages in them are deleted. Generated documents and memory stay — use the Memory board to clear memory. This can’t be undone.")
            }
        }
    }

    // MARK: - Generated documents

    /// The files Tacet produced and the ones the user attached to a chat live under
    /// Documents/Tacet. Deleting a chat does not delete them; personal data is cleared here.
    private var documentsSection: some View {
        section("DOCUMENTS") {
            VStack(alignment: .leading, spacing: Spacing.s3) {
                VStack(spacing: 0) {
                    HStack(spacing: Spacing.s2) {
                        Text("Space used")
                            .font(Typography.user())
                            .foregroundStyle(Palette.ink)
                        Spacer(minLength: 0)
                        Text(sizeText)
                            .font(Typography.chip())
                            .foregroundStyle(Palette.grey)
                            .monospacedDigit()
                    }
                    .padding(.vertical, Spacing.s3)
                    .padding(.horizontal, Spacing.s4)
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel(Text("Space used by generated documents"))
                    .accessibilityValue(Text(sizeText))

                    separator

                    Button { documentDeleteApproval = true } label: {
                        HStack(spacing: Spacing.s2) {
                            Text("Delete generated documents")
                                .font(Typography.user())
                                .foregroundStyle(hasDocuments ? Palette.ink : Palette.muted)
                                .multilineTextAlignment(.leading)
                            Spacer(minLength: 0)
                            Image(systemName: "chevron.right")
                                .font(Typography.iconSmall())
                                .foregroundStyle(Palette.muted)
                                .accessibilityHidden(true)
                        }
                        .padding(.vertical, Spacing.s3)
                        .padding(.horizontal, Spacing.s4)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .disabled(!hasDocuments)
                    .accessibilityLabel(Text("Delete generated documents"))
                    .accessibilityHint(Text("You’ll be asked to confirm. Every Tacet document on this device is deleted."))
                }
                .hairlineFrame()

                Text("Files that are created or attached to a conversation are kept in the Documents folder on this device.")
                    .font(Typography.chip())
                    .foregroundStyle(Palette.muted)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .confirmationDialog("Delete generated documents?",
                                isPresented: $documentDeleteApproval,
                                titleVisibility: .visible) {
                Button("Delete", role: .destructive) { deleteDocuments() }
                Button("Cancel", role: .cancel) { }
            } message: {
                Text("Every file Tacet produced and every file you attached to a conversation is deleted from the device. This can’t be undone.")
            }
        }
    }

    private var hasDocuments: Bool { documentSize > 0 }

    private var sizeText: String {
        documentSize > 0
            ? documentSize.formatted(.byteCount(style: .file))
            : String(localized: "No documents")
    }

    @MainActor
    private func measureDocumentSize() {
        guard documentContext != nil else { return }
        documentSize = DocumentContext.outputSize()
    }

    @MainActor
    private func deleteDocuments() {
        documentContext?.deleteOutputs()
        measureDocumentSize()
    }

    // MARK: - Web search (web-search-spec §3.1, §4)

    /// The user's own SearXNG server. The app ships no preset address; an address is not
    /// saved until it has been TESTED, and search is not enabled until it is saved.
    private var webSearchSection: some View {
        section("WEB SEARCH") {
            VStack(alignment: .leading, spacing: Spacing.s3) {
                VStack(spacing: 0) {
                    addressField
                    separator
                    testRow
                    separator
                    toggleRow
                    if savedIsValid {
                        separator
                        forgetServerRow
                    }
                }
                .hairlineFrame()

                if let result = searchStateText {
                    Text(result)
                        .font(Typography.chip())
                        .foregroundStyle(search.isError ? Palette.error : Palette.grey)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityLabel(Text(result))
                }

                // The empty-state text is written verbatim in spec §4.
                if !savedIsValid {
                    Text("No search server. Connect your own SearXNG server and Tacet can search the web. Until you do, web search stays off.")
                        .font(Typography.chip())
                        .foregroundStyle(Palette.muted)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }

    private var addressField: some View {
        VStack(alignment: .leading, spacing: Spacing.s1) {
            Text("Server address")
                .font(Typography.user())
                .foregroundStyle(Palette.ink)
            TextField("https://example.com/searxng/", text: $searchURL)
                .font(Typography.chip())
                .foregroundStyle(Palette.ink)
                .textFieldStyle(.plain)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .keyboardType(.URL)
                .submitLabel(.done)
                // When the address changes, the previous test's result is invalid.
                .onChange(of: searchURL) { _, new in
                    // After a successful test the address is normalised and written back;
                    // it is compared so that change does not wipe the result.
                    guard new.trimmingCharacters(in: .whitespacesAndNewlines) != savedAddress else { return }
                    searchTask?.cancel()
                    search = .pending
                }
                .accessibilityLabel(Text("Search server address"))
                .accessibilityHint(Text("https addresses only. Plain http is accepted only on a local network."))
        }
        .padding(.vertical, Spacing.s3)
        .padding(.horizontal, Spacing.s4)
    }

    private var testRow: some View {
        Button { testServer() } label: {
            HStack(spacing: Spacing.s2) {
                Text("Test the server")
                    .font(Typography.user())
                    .foregroundStyle(canTest ? Palette.ink : Palette.muted)
                Spacer(minLength: 0)
                Text(testLabel)
                    .font(Typography.chip())
                    .foregroundStyle(Palette.grey)
            }
            .padding(.vertical, Spacing.s3)
            .padding(.horizontal, Spacing.s4)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!canTest)
        .accessibilityLabel(Text("Test the server"))
        .accessibilityValue(Text(testLabel))
        .accessibilityHint(Text("A single sample query is sent to the server."))
    }

    private var toggleRow: some View {
        Toggle(isOn: $searchOn) {
            Text("Turn on search")
                .font(Typography.user())
                .foregroundStyle(savedIsValid ? Palette.ink : Palette.muted)
        }
        // No accent colour: the switch is in the ink tone too.
        .tint(Palette.ink)
        .disabled(!savedIsValid)
        .padding(.vertical, Spacing.s3)
        .padding(.horizontal, Spacing.s4)
        .accessibilityHint(Text(savedIsValid
                                ? String(localized: "When this is off, Tacet never uses search.")
                                : String(localized: "A server address has to be tested first.")))
    }

    private var forgetServerRow: some View {
        Button { forgetServer() } label: {
            HStack(spacing: Spacing.s2) {
                Text("Remove the server")
                    .font(Typography.user())
                    .foregroundStyle(Palette.ink)
                Spacer(minLength: 0)
                Image(systemName: "chevron.right")
                    .font(Typography.iconSmall())
                    .foregroundStyle(Palette.muted)
                    .accessibilityHidden(true)
            }
            .padding(.vertical, Spacing.s3)
            .padding(.horizontal, Spacing.s4)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text("Remove the server"))
        .accessibilityHint(Text("The address is deleted and search is turned off."))
    }

    /// Is the saved (tested) address valid — the on/off switch depends on this.
    private var savedIsValid: Bool { WebSearchSetting.validate(savedAddress) != nil }

    private var canTest: Bool {
        if case .testing = search { return false }
        return !searchURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var testLabel: String {
        switch search {
        case .testing: return String(localized: "testing")
        case .succeeded: return String(localized: "works")
        case .failed: return String(localized: "failed")
        case .pending: return savedIsValid
            && savedAddress == searchURL.trimmingCharacters(in: .whitespacesAndNewlines)
            ? String(localized: "saved")
            : ""
        }
    }

    /// The full sentence of the test result. On failure the cause is written in plain
    /// language.
    private var searchStateText: String? {
        switch search {
        case .pending, .testing:
            return nil
        case .succeeded(let count):
            return String(localized: "search works — the sample query returned \(count) results. The address has been saved.")
        case .failed(let cause):
            return cause
        }
    }

    /// `GET {url}/search?q=test&format=json`. On success the address IS SAVED.
    private func testServer() {
        let raw = searchURL.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let root = WebSearchSetting.validate(raw) else {
            search = .failed(String(localized: "The address couldn’t be read. It must start with https://; plain http is accepted only for addresses on your own local network."))
            return
        }
        searchTask?.cancel()
        search = .testing
        searchTask = Task { @MainActor in
            do {
                let (results, _) = try await WebSearchClient.search("test", root: root)
                guard !Task.isCancelled else { return }
                // The address only becomes persistent once it has been seen to work.
                WebSearchSetting.rawRoot = raw
                savedAddress = raw
                searchURL = raw
                search = .succeeded(results.count)
            } catch {
                guard !Task.isCancelled else { return }
                search = .failed(Self.searchErrorSentence(error))
            }
        }
    }

    /// Clears the address and the on/off flag; the network surface closes again.
    private func forgetServer() {
        searchTask?.cancel()
        searchTask = nil
        searchOn = false
        WebSearchSetting.rawRoot = ""
        savedAddress = ""
        searchURL = ""
        search = .pending
    }

    /// Error → user sentence. The JSON trap is specific to SearXNG and is said explicitly.
    static func searchErrorSentence(_ error: Error) -> String {
        // An unexpected error type is not swallowed silently either, it comes out as a
        // plain sentence.
        guard let known = error as? WebSearchError else {
            return String(localized: "The test couldn’t be completed: \(error.localizedDescription)")
        }
        switch known {
        case .formatNotUnderstood:
            return String(localized: "The server responded but didn’t return JSON. In the server’s settings.yml, formats: json must be enabled.")
        case .serverError(let code):
            return String(localized: "The server returned \(code). The address may be right, but search may not be exposed at this path.")
        case .unreachable:
            return String(localized: "The server couldn’t be reached. The address wasn’t found, or the response didn’t arrive in time.")
        case .noServer:
            return String(localized: "The address couldn’t be read.")
        }
    }

    // MARK: - Connections (mcp §3.1)

    private var connectionsSection: some View {
        section("CONNECTIONS") {
            VStack(alignment: .leading, spacing: Spacing.s3) {
                Button { openConnections() } label: {
                    HStack(spacing: Spacing.s2) {
                        Text("Connections")
                            .font(Typography.user())
                            .foregroundStyle(Palette.ink)
                        Spacer(minLength: 0)
                        Image(systemName: "chevron.right")
                            .font(Typography.iconSmall())
                            .foregroundStyle(Palette.muted)
                            .accessibilityHidden(true)
                    }
                    .padding(.vertical, Spacing.s3)
                    .padding(.horizontal, Spacing.s4)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(Text("Connections"))
                .accessibilityHint(Text("Add and manage your own servers."))
                .hairlineFrame()

                Text("Connect your own MCP servers and Tacet can use their tools. Until you do, nothing goes to any server.")
                    .font(Typography.chip())
                    .foregroundStyle(Palette.muted)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    // MARK: - Shortcuts (intent-plan §4.2)

    /// `ProvideDocumentIntent` is the only surface that gets a REAL file out of the app.
    /// Consent is taken once, here; the export itself shows up here in retrospect.
    private var shortcutsSection: some View {
        section("SHORTCUTS") {
            VStack(alignment: .leading, spacing: Spacing.s3) {
                VStack(spacing: 0) {
                    Toggle(isOn: Binding(
                        get: { exportOn },
                        set: { new in
                            exportOn = new
                            // Turning it off clears the record too: a "last given" row
                            // sitting there while the gate is closed does nothing but
                            // remind you of a permission that is no longer valid.
                            if !new {
                                ShortcutSetting.deleteRecord()
                                lastGiven = nil
                            }
                        })) {
                        Text("Give files to Shortcuts")
                            .font(Typography.user())
                            .foregroundStyle(Palette.ink)
                    }
                    // No accent colour: the switch is in the ink tone too.
                    .tint(Palette.ink)
                    .padding(.vertical, Spacing.s3)
                    .padding(.horizontal, Spacing.s4)
                    .accessibilityHint(Text("When off, Tacet gives no file to Shortcuts."))

                    separator

                    HStack(spacing: Spacing.s2) {
                        Text("Last given")
                            .font(Typography.user())
                            .foregroundStyle(lastGiven == nil ? Palette.muted : Palette.ink)
                        Spacer(minLength: Spacing.s3)
                        Text(lastGivenText)
                            .font(Typography.chip())
                            .foregroundStyle(Palette.grey)
                            .multilineTextAlignment(.trailing)
                    }
                    .padding(.vertical, Spacing.s3)
                    .padding(.horizontal, Spacing.s4)
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel(Text("Last file given to Shortcuts"))
                    .accessibilityValue(Text(lastGivenText))
                }
                .hairlineFrame()

                Text("When off, the files Tacet creates never leave through Shortcuts. If you turn it on, an automation can carry that file to email, the cloud or another app — your call.")
                    .font(Typography.chip())
                    .foregroundStyle(Palette.muted)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private var lastGivenText: String {
        guard let lastGiven else { return String(localized: "Never given") }
        let date = lastGiven.date.formatted(date: .abbreviated, time: .shortened)
        return "\(lastGiven.name) · \(date)"
    }

    // MARK: - Permissions

    private var permissionsSection: some View {
        section("PERMISSIONS") {
            VStack(alignment: .leading, spacing: Spacing.s3) {
                PermissionSection()
                    .hairlineFrame()

                Text(PermissionSection.description)
                    .font(Typography.chip())
                    .foregroundStyle(Palette.muted)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    // MARK: - Pieces

    private func section<Content: View>(_ title: LocalizedStringKey,
                                        @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: Spacing.s3) {
            Text(title)
                .font(Typography.tag())
                .textCase(.uppercase)
                .tracking(1.2)
                .foregroundStyle(Palette.muted)
            content()
        }
    }

    private var separator: some View {
        Rectangle()
            .fill(Palette.divider)
            .frame(height: Spacing.hairline)
    }

    private var footnote: some View {
        Text("Everything stays on this device.")
            .font(Typography.chip())
            .foregroundStyle(Palette.muted)
            .padding(.top, Spacing.s2)
    }
}

private extension View {
    /// The shared hairline frame of the settings cards.
    /// NOT named `frame()`: SwiftUI's own `frame(width:height:alignment:)` is callable
    /// with zero arguments, so a bare `frame()` would be ambiguous at every call site.
    func hairlineFrame() -> some View {
        overlay(
            RoundedRectangle(cornerRadius: Spacing.s4, style: .continuous)
                .stroke(Palette.divider, lineWidth: Spacing.hairline)
        )
    }
}
