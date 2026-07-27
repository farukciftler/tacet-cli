//
//  NewConnection.swift
//  Tacet
//
//  The add-server form — mcp-connection-spec §3.1.
//
//  "Test connection" is a MANDATORY step: the user sees what the server can do BEFORE
//  adding it. If the connection cannot be made, the cause is written in plain language.
//  The device-data default is "never". "always allow" can be selected, but the moment it
//  is, the warning modal appears — see DeviceDataPicker.
//

import SwiftUI

struct NewConnection: View {
    let service: any ConnectionProbe

    @Environment(\.dismiss) private var close

    @State private var name = ""
    @State private var rawURL = ""
    @State private var key = ""
    @State private var deviceData: DeviceDataSetting = .never

    @State private var probing = false
    /// The tool list returned by a successful probe. Shown before adding.
    @State private var tools: [ToolSummary]?
    /// The plain-language counterpart of the cause when the probe fails.
    @State private var errorText: String?
    @State private var addFailed: String?

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: Spacing.s5) {
                    nameSection
                    urlSection
                    keySection
                    deviceDataSection
                    probeSection
                    if let addFailed { warning(addFailed) }
                }
                .padding(.horizontal, Spacing.s5)
                .padding(.vertical, Spacing.s4)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .background(Palette.background)
            .navigationTitle("Add server")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Cancel") { close() }
                        .font(Typography.chip())
                        .foregroundStyle(probing ? Palette.muted : Palette.grey)
                        .disabled(probing)
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Add") { add() }
                        .font(Typography.chip())
                        .foregroundStyle(canAdd ? Palette.ink : Palette.muted)
                        .disabled(!canAdd)
                        .accessibilityHint(canAdd
                                           ? Text("Saves the server and closes this page.")
                                           : Text("You need to test the connection first."))
                }
            }
        }
        .interactiveDismissDisabled(probing)
    }

    // MARK: - State

    private var cleanName: String { name.trimmingCharacters(in: .whitespacesAndNewlines) }
    private var cleanURL: String { rawURL.trimmingCharacters(in: .whitespacesAndNewlines) }

    private var canProbe: Bool {
        !probing && !cleanName.isEmpty && ConnectionURLCheck.isAcceptable(cleanURL)
    }

    /// Adding only unlocks after a SUCCESSFUL probe (§3.1).
    private var canAdd: Bool { !probing && tools != nil && !cleanName.isEmpty }

    // MARK: - Sections

    private var nameSection: some View {
        section("NAME") {
            TextField("home server", text: $name)
                .font(Typography.user())
                .foregroundStyle(Palette.ink)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .padding(.vertical, Spacing.s3)
                .padding(.horizontal, Spacing.s4)
                .hairlineFrame()
                .accessibilityLabel(Text("Server name"))
                .accessibilityHint(Text("This name appears on chips."))
                .onChange(of: name) { _, _ in invalidateProbe() }
        }
    }

    private var urlSection: some View {
        section("URL") {
            VStack(alignment: .leading, spacing: Spacing.s2) {
                TextField("https://…", text: $rawURL)
                    .font(Typography.user())
                    .foregroundStyle(Palette.ink)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .keyboardType(.URL)
                    .padding(.vertical, Spacing.s3)
                    .padding(.horizontal, Spacing.s4)
                    .hairlineFrame()
                    .accessibilityLabel(Text("Server address"))
                    .onChange(of: rawURL) { _, _ in invalidateProbe() }

                Text(urlNote)
                    .font(Typography.chip())
                    .foregroundStyle(Palette.grey)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    /// The address rule: `https://`. Plain `http://` only for local network addresses.
    private var urlNote: LocalizedStringKey {
        if cleanURL.isEmpty || ConnectionURLCheck.isAcceptable(cleanURL) {
            return "A Streamable HTTP address. Plain http is accepted only for local network addresses."
        }
        return "This address isn’t accepted: use https; plain http only works on a local network."
    }

    private var keySection: some View {
        section("ACCESS KEY") {
            VStack(alignment: .leading, spacing: Spacing.s2) {
                SecureField("optional", text: $key)
                    .font(Typography.user())
                    .foregroundStyle(Palette.ink)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .padding(.vertical, Spacing.s3)
                    .padding(.horizontal, Spacing.s4)
                    .hairlineFrame()
                    .accessibilityLabel(Text("Access key, optional"))
                    .onChange(of: key) { _, _ in invalidateProbe() }

                Text("Stored in the Keychain and never shown again.")
                    .font(Typography.chip())
                    .foregroundStyle(Palette.grey)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private var deviceDataSection: some View {
        section("DEVICE DATA") {
            DeviceDataPicker(selection: $deviceData)
        }
    }

    private var probeSection: some View {
        section("CONNECTION") {
            VStack(alignment: .leading, spacing: Spacing.s3) {
                Button { probe() } label: {
                    HStack(spacing: Spacing.s2) {
                        if probing {
                            ProgressView()
                                .controlSize(.small)
                                .tint(Palette.grey)
                        }
                        Text(probing ? "Testing…" : "Test connection")
                        Spacer(minLength: 0)
                    }
                    .font(Typography.user())
                    .foregroundStyle(canProbe ? Palette.ink : Palette.muted)
                    .padding(.vertical, Spacing.s3)
                    .padding(.horizontal, Spacing.s4)
                    .frame(minHeight: Spacing.touchTarget)
                    .hairlineFrame()
                    .contentShape(RoundedRectangle(cornerRadius: Spacing.s4, style: .continuous))
                }
                .buttonStyle(.plain)
                .disabled(!canProbe)
                .accessibilityHint(Text("Connects to the server and fetches its tool list."))

                if let errorText { warning(errorText) }
                if let tools { toolList(tools) }
            }
        }
    }

    private func toolList(_ list: [ToolSummary]) -> some View {
        VStack(alignment: .leading, spacing: Spacing.s3) {
            Text(list.isEmpty
                 ? "Connected; the server didn’t report any tools."
                 : "Connected. This server’s tools:")
                .font(Typography.chip())
                .foregroundStyle(Palette.grey)
                .fixedSize(horizontal: false, vertical: true)

            ForEach(list) { tool in
                ToolSummaryRow(tool: tool)
            }
        }
    }

    // MARK: - Pieces

    private func section<Content: View>(_ title: LocalizedStringKey,
                                        @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: Spacing.s3) {
            Text(title)
                .font(Typography.tag())
                .tracking(1.2)
                .foregroundStyle(Palette.muted)
                .accessibilityAddTraits(.isHeader)
            content()
        }
    }

    /// A failure is written in plain language; it is never swallowed silently.
    private func warning(_ text: String) -> some View {
        Text(text)
            .font(Typography.chip())
            .foregroundStyle(Palette.grey)
            .fixedSize(horizontal: false, vertical: true)
            .accessibilityLabel(Text(verbatim: String(localized: "Error: \(text)")))
            .accessibilityAddTraits(.isSummaryElement)
    }

    // MARK: - Actions

    /// When the form changes, the old probe result is invalid: a record must not be
    /// created with another server's tool list.
    private func invalidateProbe() {
        tools = nil
        errorText = nil
        addFailed = nil
    }

    private func probe() {
        probing = true
        errorText = nil
        tools = nil
        addFailed = nil
        let serverName = cleanName
        let address = cleanURL
        let secret = key.isEmpty ? nil : key
        Task { @MainActor in
            let outcome = await service.probe(name: serverName, rawURL: address, key: secret)
            probing = false
            switch outcome {
            case .succeeded(let list): tools = list
            case .failed(let cause): errorText = cause
            }
        }
    }

    private func add() {
        guard let list = tools else { return }
        do {
            _ = try service.add(name: cleanName,
                                rawURL: cleanURL,
                                key: key.isEmpty ? nil : key,
                                deviceData: deviceData,
                                tools: list)
            close()
        } catch {
            // The page stays open; the user sees what happened.
            addFailed = String(localized: "Couldn’t save the connection: \(error.localizedDescription)")
        }
    }
}

// MARK: - Shared pieces

/// A single row in the tool list. An unsupported tool is not swallowed silently (§5.2).
struct ToolSummaryRow: View {
    let tool: ToolSummary

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.s1) {
            HStack(spacing: Spacing.s2) {
                Text(tool.name)
                    .font(Typography.user())
                    .foregroundStyle(tool.isUnsupported ? Palette.grey : Palette.ink)
                    .lineLimit(1)
                if tool.isUnsupported {
                    Text("not supported")
                        .font(Typography.chip())
                        .foregroundStyle(Palette.muted)
                }
            }
            if !tool.summary.isEmpty {
                Text(tool.summary)
                    .font(Typography.chip())
                    .foregroundStyle(Palette.grey)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.vertical, Spacing.s3)
        .padding(.horizontal, Spacing.s4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(
            RoundedRectangle(cornerRadius: Spacing.s4, style: .continuous)
                .stroke(Palette.divider, lineWidth: Spacing.hairline)
        )
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(verbatim: tool.name))
        .accessibilityValue(tool.isUnsupported
                            ? Text(verbatim: String(localized: "Not supported. \(tool.summary)"))
                            : Text(verbatim: tool.summary))
    }
}

/// The address rule in one place: `https://` everywhere, plain `http://` only for local
/// network addresses (§3.1). The service layer uses this too.
enum ConnectionURLCheck {
    static func isAcceptable(_ raw: String) -> Bool {
        guard let url = URL(string: raw.trimmingCharacters(in: .whitespacesAndNewlines)),
              let scheme = url.scheme?.lowercased(),
              let host = url.host()?.lowercased(), !host.isEmpty else { return false }
        if scheme == "https" { return true }
        guard scheme == "http" else { return false }
        return isLocalNetwork(host)
    }

    /// Local network: localhost, .local (Bonjour), private IPv4 blocks, IPv6 loopback.
    static func isLocalNetwork(_ host: String) -> Bool {
        if host == "localhost" || host == "127.0.0.1" || host == "::1" { return true }
        if host.hasSuffix(".local") { return true }

        let parts = host.split(separator: ".").compactMap { Int($0) }
        guard parts.count == 4, parts.allSatisfy({ (0...255).contains($0) }) else { return false }
        switch (parts[0], parts[1]) {
        case (10, _): return true
        case (192, 168): return true
        case (172, 16...31): return true
        case (169, 254): return true   // link-local
        default: return false
        }
    }
}

private extension View {
    /// The shared hairline frame of the connection setup rows.
    /// NOT named `frame()`: SwiftUI's own `frame(width:height:alignment:)` is callable
    /// with zero arguments, so a bare `frame()` would be ambiguous at every call site.
    func hairlineFrame() -> some View {
        overlay(
            RoundedRectangle(cornerRadius: Spacing.s4, style: .continuous)
                .stroke(Palette.divider, lineWidth: Spacing.hairline)
        )
    }
}
