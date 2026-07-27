import SwiftUI

// The input field: a pill-shaped text box and a circular send button.
// Spec §4.5. While the text is empty the button is inert but its appearance does not change.
struct InputField: View {
    @Binding var text: String
    let send: () -> Void
    /// Attaching a document (for reading/editing). If nil, the attach button is not shown.
    var add: (() -> Void)? = nil
    /// Is a reply being produced right now — the send button turns into a stop button.
    var isProducing: Bool = false
    /// Cancels production (ModelService.stop()).
    var stop: () -> Void = {}

    /// The haptic trigger: it increments on every send/stop.
    @State private var tapCounter = 0

    /// In-app dictation (on-device). The audio session is only open while listening.
    @State private var voice = VoiceInput()
    /// The text standing in the field when dictation starts — the recognised speech is
    /// appended AFTER it, what the user typed is not erased.
    @State private var beforeDictation = ""

    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.openURL) private var openLink

    // The touch target and the icon boxes sit next to text: they grow with Dynamic Type
    // (Theme.swift §Spacing). A hard-coded 44 is not written, the token is read scaled.
    @ScaledMetric(relativeTo: .callout) private var touchTarget: CGFloat = Spacing.touchTarget
    @ScaledMetric(relativeTo: .callout) private var attachBox: CGFloat = 28
    @ScaledMetric(relativeTo: .callout) private var sendBox: CGFloat = 32

    // If the text is only whitespace, nothing is sent.
    private var isEmpty: Bool {
        text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var body: some View {
        // .bottom so that when the field grows to several lines the buttons stay aligned
        // with the LAST line.
        HStack(alignment: .bottom, spacing: Spacing.s2) {
            if let add {
                Button(action: add) {
                    Image(systemName: "paperclip")
                        .font(Typography.user())
                        .foregroundStyle(Palette.grey)
                        .frame(width: attachBox, height: attachBox)
                        // The visual box stays 28pt; only the touch area is opened to 44.
                        .frame(minWidth: touchTarget, minHeight: touchTarget)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(Text("Add document"))
            }

            TextField("", text: $text, prompt: placeholder, axis: .vertical)
                .lineLimit(1...5)
                .font(Typography.user())
                .foregroundStyle(Palette.ink)
                .textFieldStyle(.plain)
                .submitLabel(.send)
                .onSubmit { triggerSend() }
                // In a vertical-axis field "return" does not fire onSubmit, it inserts a
                // line break. To keep the old behaviour (return = send) we undo the
                // trailing line break and send; line breaks inside the text (from pasting)
                // are not disturbed.
                //
                // The SINGLE-CHARACTER growth condition: without it, a PASTE ending in a
                // line break (the most common form of multi-line copying) would be sent
                // without the user pressing anything.
                .onChange(of: text) { old, new in
                    guard new.count == old.count + 1, new.hasSuffix("\n") else { return }
                    text = String(new.dropLast())
                    triggerSend()
                }
                .padding(.vertical, Spacing.s3)

            microphoneButton
            sendButton
        }
        // The recognised speech lands in the field live; sending is left to the user.
        .onChange(of: voice.transcribed) { _, new in
            text = merge(beforeDictation, new)
        }
        // The microphone must not stay open when going to the background (privacy + battery).
        .onChange(of: scenePhase) { _, new in
            guard new != .active, voice.isRunning else { return }
            Task { await voice.stop() }
        }
        .onDisappear {
            guard voice.isRunning else { return }
            Task { await voice.stop() }
        }
        .alert(blockTitle, isPresented: blockShowing) {
            if settingsNeeded, let link = VoiceInput.settingsLink {
                Button { openLink(link) } label: {
                    Text("Open Settings")
                }
            }
            Button(role: .cancel) { voice.block = nil } label: {
                Text("OK")
            }
        } message: {
            blockText
        }
        .sensoryFeedback(.impact(weight: .light), trigger: tapCounter)
        // The buttons' 44pt touch box already carries the visual inner padding; when the
        // old s1/s2 margins were stacked on top of it, the pill puffed up.
        .padding(.leading, add == nil ? Spacing.s4 : Spacing.s1)
        .overlay(
            RoundedRectangle(cornerRadius: Spacing.inputCorner)
                .stroke(Palette.divider, lineWidth: Spacing.hairline)
        )
        .padding(.horizontal, Spacing.s5)
    }

    // The placeholder text in the muted colour from the contract.
    private var placeholder: Text {
        Text("Ask Tacet").foregroundStyle(Palette.muted)
    }

    // The microphone button: the same quiet language as the paper clip — no fill, no
    // frame, grey. While listening, the only difference is that the icon fills in and
    // darkens to ink; the system's own faint pulse rides on top. NO accent colour, no
    // shadow, no glow.
    private var microphoneButton: some View {
        Button {
            tapCounter += 1
            if voice.isRunning {
                Task { await voice.stop() }
            } else {
                beforeDictation = text
                Task { await voice.start() }
            }
        } label: {
            Image(systemName: voice.isRunning ? "mic.fill" : "mic")
                .font(Typography.user())
                .foregroundStyle(voice.isRunning ? Palette.ink : Palette.grey)
                .symbolEffect(.pulse, isActive: voice.isRunning)
                .frame(width: attachBox, height: attachBox)
                // The visual box stays 28pt; only the touch area is opened to 44.
                .frame(minWidth: touchTarget, minHeight: touchTarget)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text("Write by voice"))
        .accessibilityValue(microphoneState)
    }

    private var microphoneState: Text {
        switch voice.state {
        case .idle:      Text("Off")
        case .preparing: Text("Getting ready")
        case .listening: Text("Listening")
        }
    }

    /// The dictated text is appended after what the user typed earlier.
    private func merge(_ previous: String, _ new: String) -> String {
        guard !previous.isEmpty else { return new }
        guard !new.isEmpty else { return previous }
        return previous.hasSuffix(" ") ? previous + new : previous + " " + new
    }

    // MARK: - Block notice

    private var blockShowing: Binding<Bool> {
        Binding(get: { voice.block != nil }, set: { if !$0 { voice.block = nil } })
    }

    private var settingsNeeded: Bool {
        voice.block == .microphonePermission || voice.block == .speechPermission
    }

    private var blockTitle: Text {
        switch voice.block {
        case .microphonePermission: Text("Microphone access is off")
        case .speechPermission:     Text("Speech recognition access is off")
        case .languageMissing:      Text("No on-device dictation for this language")
        default:                    Text("Couldn’t start dictation")
        }
    }

    private var blockText: Text {
        switch voice.block {
        case .microphonePermission:
            Text("To write by voice you need to turn on microphone access in Settings.")
        case .speechPermission:
            Text("For your voice to become text you need to turn on speech recognition in Settings.")
        case .languageMissing:
            Text("There’s no on-device recognition model for the selected language on this device. So your voice never leaves the device, dictation stays off.")
        default:
            Text("The microphone wasn’t available just now. Try again in a moment.")
        }
    }

    // The circular send button: ink fill, a white up arrow, ~32pt.
    // While empty the appearance stays THE SAME but the action is not fired — there is no
    // faded/disabled button (spec §4.5). While production is running the same circle turns
    // into a STOP button, so that the only way out of a long reply is not closing the app.
    private var sendButton: some View {
        Button {
            tapCounter += 1
            if isProducing {
                stop()
            } else {
                guard !isEmpty else { return }
                send()
            }
        } label: {
            Image(systemName: isProducing ? "stop.fill" : "arrow.up")
                .font((isProducing ? Typography.chip() : Typography.icon()).weight(.medium))
                // The glyph is the opposite of the fill (ink): the background colour. In
                // dark mode ink is light, so a white glyph became invisible; background =
                // dark, and the contrast is preserved.
                .foregroundStyle(Palette.background)
                .frame(width: sendBox, height: sendBox)
                .background(Palette.ink, in: Circle())
                // The visual circle stays 32pt; the touch area is opened to 44.
                .frame(minWidth: touchTarget, minHeight: touchTarget)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(isProducing ? Text("Stop") : Text("Send"))
    }

    /// One door, so that the keyboard "return" path also goes through the haptic.
    private func triggerSend() {
        guard !isProducing, !isEmpty else { return }
        tapCounter += 1
        send()
    }
}

#Preview {
    struct Preview: View {
        @State private var empty = ""
        @State private var full = "Show tomorrow's meetings"
        var body: some View {
            VStack(spacing: Spacing.messageGap) {
                InputField(text: $empty, send: {})
                InputField(text: $full, send: {})
                InputField(text: $full, send: {}, isProducing: true)
            }
            .padding(.vertical, Spacing.s4)
        }
    }
    return Preview()
}
