//
//  DeviceDataPicker.swift
//  Tacet
//
//  The connection screens' shared "device data" picker — mcp-connection-spec §3.1.
//
//  There are three modes. The first two go in the restrictive direction and are applied
//  silently. The third (`always`) closes the approval gate: THE MOMENT IT IS SELECTED a
//  warning modal appears, and if the user backs out the selection stays at its old value —
//  the setting having silently changed once the modal closed would be exactly the gate
//  losing its meaning.
//
//  While it is selected, a persistent status row stays on screen: when the user comes back
//  to the screen days later they must be able to see, without a modal, that this
//  connection does not ask.
//

import SwiftUI

struct DeviceDataPicker: View {
    @Binding var selection: DeviceDataSetting
    var closed: Bool = false

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// Is approval for closing the gate being awaited.
    @State private var approvalRequested = false
    /// Used to redraw the segment back to the source after a rejected selection.
    /// When the binding's `set` does not change the value, SwiftUI may not invalidate the
    /// view on its own; changing the identity makes it redraw for certain.
    @State private var refresh = 0

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.s2) {
            Picker("Device data", selection: Binding(
                get: { selection },
                set: { requested($0) }
            )) {
                Text("never")
                    .tag(DeviceDataSetting.never)
                    .accessibilityLabel(Text("Never send"))
                Text("ask every time")
                    .tag(DeviceDataSetting.askEveryTime)
                    .accessibilityLabel(Text("Ask every time"))
                Text("always")
                    .tag(DeviceDataSetting.always)
                    .accessibilityLabel(Text("Always allow"))
            }
            .pickerStyle(.segmented)
            .id(refresh)
            .disabled(closed)
            .accessibilityLabel(Text("Device data sharing"))
            .accessibilityHint(Text("Controls whether data such as your calendar, contacts and documents is sent to this server. If Always allow is selected, you aren’t asked to confirm."))

            Text(description)
                .font(Typography.chip())
                .foregroundStyle(Palette.grey)
                .fixedSize(horizontal: false, vertical: true)

            if selection.skipsGate { statusRow }
        }
        .alert(Text("Turn on sending without asking"), isPresented: $approvalRequested) {
            Button(role: .cancel) { approvalRequested = false } label: {
                Text("Cancel")
            }
            .accessibilityHint(Text("The setting stays as it is."))

            Button { approve() } label: {
                Text("Always allow")
            }
            .accessibilityHint(Text("From now on, data is sent to this connection without asking."))
        } message: {
            Text(modalText)
        }
    }

    // MARK: - Pieces

    /// The calm status row that stays on screen while it is selected. No dot, no badge:
    /// the state is said in words.
    private var statusRow: some View {
        VStack(alignment: .leading, spacing: Spacing.s1) {
            Text("Data is sent to this connection without asking for confirmation.")
                .font(Typography.chip())
                .foregroundStyle(Palette.error)
            Text("You can still see everything that was sent by tapping the chip in the conversation. You can undo this setting at any time.")
                .font(Typography.chip())
                .foregroundStyle(Palette.grey)
        }
        .fixedSize(horizontal: false, vertical: true)
        .padding(.vertical, Spacing.s3)
        .padding(.horizontal, Spacing.s4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(
            RoundedRectangle(cornerRadius: Spacing.s4, style: .continuous)
                .stroke(Palette.divider, lineWidth: Spacing.hairline)
        )
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isSummaryElement)
    }

    private var description: LocalizedStringKey {
        switch selection {
        case .never:
            return "Data read from your device is never sent to this server."
        case .askEveryTime:
            return "Data read from your device is shown to you every time before it is sent."
        case .always:
            return "Data read from your device can be sent to this server without asking you."
        }
    }

    /// It says concretely what is being given up; without scaring, but without softening.
    private var modalText: LocalizedStringKey {
        """
        Tacet will no longer ask you before sending data to this connection.

        If your calendar, contacts, notes or documents were used in the same conversation, content from those tools can end up inside an argument you never see and go to the server.

        If the server is compromised or acts in bad faith, the text it returns can steer the model into sending more data. The confirmation step was exactly what stopped that.

        You can still see everything that was sent by tapping the chip in the conversation: what goes away is only the up-front confirmation, not the transparency.

        You can undo this setting at any time.
        """
    }

    // MARK: - Actions

    /// A move in the restrictive direction asks for no warning; a move that closes the
    /// gate does.
    private func requested(_ new: DeviceDataSetting) {
        guard new != selection else { return }
        guard new.skipsGate else {
            selection = new
            return
        }
        // We are NOT writing the value now: the setting must not change before it is approved.
        approvalRequested = true
        var transaction = Transaction()
        transaction.disablesAnimations = reduceMotion
        withTransaction(transaction) { refresh &+= 1 }
    }

    private func approve() {
        approvalRequested = false
        var transaction = Transaction()
        transaction.disablesAnimations = reduceMotion
        withTransaction(transaction) {
            selection = .always
            refresh &+= 1
        }
    }
}
