//
//  Welcome.swift
//  Tacet
//
//  The first-launch welcome. No slides, no paging: a single screen sets the frame
//  and immediately has you pick a task. Product principle #2 — "don't tell, show":
//  most of what there is to explain the product explains by itself, and the only
//  action here is picking a task.
//
//  While the model is unreachable (downloading / Apple Intelligence off / device not
//  eligible) the task chips are NOT SHOWN: in that state the reply would be dropped
//  without being saved, and the user who tapped a chip would see nothing happen.
//

import SwiftUI
import UIKit

struct Welcome: View {
    let state: ModelService.State
    let block: ModelService.Block?
    /// The picked task — the root view carries it into the chat.
    var jobSelected: (String) -> Void = { _ in }

    @Environment(\.dismiss) private var close
    @State private var capabilitiesOpen = false
    /// The task to be reported on close. It is reported AFTER the sheet has closed;
    /// otherwise the input field's focus is lost inside the sheet's dismissal animation.
    @State private var selectedJob: String?
    @ScaledMetric(relativeTo: .body) private var touchTarget = Spacing.touchTarget

    /// The tasks in the welcome screen were deliberately picked from strings that are
    /// ALREADY translated: an untranslated prompt sends a Turkish sentence to a
    /// non-Turkish user and the model answers in that language — the language breaks in
    /// the first second of onboarding.
    private let jobs = [
        String(localized: "What's on tomorrow?"),
        String(localized: "Remind me to make a call at 6:00 PM"),
        String(localized: "Turn this week’s notes into a document"),
    ]

    // MARK: - State discrimination

    /// `Block` is not Equatable — pattern matching is used, not comparison.
    private var isPreparing: Bool {
        guard let block else { return false }
        if case .preparing = block { return true }
        return false
    }

    private var isClosed: Bool {
        guard let block else { return false }
        if case .closed = block { return true }
        return false
    }

    /// If the block is unknown (nil), the same assumption as `ModelService.blockMessage`:
    /// the device is not eligible.
    private var isDeviceIneligible: Bool {
        guard let block else { return true }
        if case .device = block { return true }
        return false
    }

    /// Should the flag be written persistently: the real welcome was seen, or the block
    /// is permanent.
    private var isPersistent: Bool { state.isReady || isDeviceIneligible }

    var body: some View {
        ScrollView {
            VStack(spacing: Spacing.s4) {
                TacetMark(size: 44)
                    .padding(.bottom, Spacing.s1)

                if state.isReady {
                    readyContent
                } else if isPreparing {
                    preparingContent
                } else if isClosed {
                    closedContent
                } else {
                    deviceContent
                }

                textButton(Text("What can Tacet do?"), color: Palette.muted) {
                    capabilitiesOpen = true
                }
                .padding(.top, Spacing.s2)
            }
            .padding(.horizontal, Spacing.s5)
            .padding(.vertical, Spacing.s5)
            .frame(maxWidth: .infinity)
        }
        .scrollBounceBehavior(.basedOnSize)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Palette.background)
        .sheet(isPresented: $capabilitiesOpen) {
            // The catalogue opened from the welcome screen is READ-ONLY: picking an
            // example from here would open a third presentation layer and a second
            // "pick a task" channel.
            Capabilities()
        }
        .onDisappear {
            // Swiping to dismiss also counts as "shown" — pressing the button is not
            // required.
            WelcomeSetting.markShown(persistent: isPersistent)
            if let selectedJob { jobSelected(selectedJob) }
        }
    }

    // MARK: - Model ready (§2.1)

    private var readyContent: some View {
        VStack(spacing: Spacing.s4) {
            title(Text("Instead of explaining, let’s just do something."))

            bodyText(Text("Tacet's core runs on this device. Web search and connections engage only if you turn them on, and only in plain sight."))
            bodyText(Text("The model is small and runs on your device: its general world knowledge is weak. When it doesn’t know, it doesn’t make things up — it says so."))
            bodyText(Text("We’re not asking for any permissions now. When calendar, reminders or contacts are needed, Tacet will ask right then."))

            Text("PICK A TASK")
                .font(Typography.tag())
                .textCase(.uppercase)
                .tracking(1.2)
                .foregroundStyle(Palette.muted)
                .padding(.top, Spacing.s2)
                .accessibilityAddTraits(.isHeader)

            VStack(spacing: Spacing.s2) {
                ForEach(jobs, id: \.self) { text in
                    SampleChip(text: text) {
                        selectedJob = text
                        close()
                    }
                }
            }

            bodyText(Text("The task you pick lands in the input field; it runs as soon as you send it. Tacet leaves every tool it touches on screen — you don’t have to read what it did, you see it."),
                     color: Palette.muted)

            textButton(Text("I’ll write my own"), color: Palette.grey) { close() }
                .padding(.top, Spacing.s2)
        }
    }

    // MARK: - Model downloading (§2.2)

    private var preparingContent: some View {
        VStack(spacing: Spacing.s4) {
            title(Text("Tacet’s core is still downloading."))
            bodyText(Text("The system is downloading the on-device model. When it’s done, Tacet works on its own; you don’t need to restart the app."))
            bodyText(Text("What you write meanwhile isn’t lost; you can ask once it’s ready."), color: Palette.muted)
            okButton
        }
    }

    // MARK: - Apple Intelligence off (§2.3)

    private var closedContent: some View {
        VStack(spacing: Spacing.s4) {
            title(Text("Apple Intelligence is off."))
            bodyText(Text("Tacet works only with the on-device model; while that’s off it can’t answer. Just turn it on in iOS Settings > Apple Intelligence & Siri — then come back here."))

            // `openSettingsURLString` opens the app's OWN settings page, not the Apple
            // Intelligence page. The text describes the path; the same limit as the
            // permission redirect in ToolChip.
            framedButton(Text("Open iOS Settings"), glyph: "arrow.up.forward") {
                if let url = URL(string: UIApplication.openSettingsURLString) {
                    UIApplication.shared.open(url)
                }
            }

            textButton(Text("OK"), color: Palette.grey) { close() }
        }
    }

    // MARK: - Device not eligible (§2.4)

    private var deviceContent: some View {
        VStack(spacing: Spacing.s4) {
            title(Text("This device can’t run Tacet."))
            // Palette.error IS NOT USED: this is not an error, it is a permanent fact.
            bodyText(Text("Tacet works only with the on-device Apple Intelligence model, and this device doesn’t have it. There’s no cloud fallback; if there were, your question would leave the device."))
            bodyText(Text("On an iPhone that can run Apple Intelligence, Tacet works just as it is. Your chats, documents and settings stay here."),
                     color: Palette.muted)
            okButton
        }
    }

    private var okButton: some View {
        framedButton(Text("OK"), glyph: nil) { close() }
    }

    // MARK: - Pieces

    private func title(_ text: Text) -> some View {
        text
            .font(Typography.tacet())
            .foregroundStyle(Palette.ink)
            .multilineTextAlignment(.center)
            .fixedSize(horizontal: false, vertical: true)
            .accessibilityAddTraits(.isHeader)
    }

    /// The body sentences ARE NOT MERGED: three separate sentences are three separate
    /// VoiceOver elements, and turning them into one long label creates a block that
    /// cannot be skipped.
    private func bodyText(_ text: Text, color: Color = Palette.grey) -> some View {
        text
            .font(Typography.chip())
            .foregroundStyle(color)
            .multilineTextAlignment(.center)
            .fixedSize(horizontal: false, vertical: true)
    }

    private func textButton(_ label: Text, color: Color, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            label
                .font(Typography.chip())
                .foregroundStyle(color)
                .frame(minWidth: touchTarget, minHeight: touchTarget)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private func framedButton(_ label: Text, glyph: String?, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: Spacing.s2) {
                label
                    .font(Typography.user())
                    .foregroundStyle(Palette.ink)
                if let glyph {
                    Image(systemName: glyph)
                        .font(Typography.chip())
                        .foregroundStyle(Palette.muted)
                        .accessibilityHidden(true)
                }
            }
            .padding(.horizontal, Spacing.s4)
            .padding(.vertical, Spacing.s3)
            .frame(minHeight: touchTarget)
            .overlay(
                RoundedRectangle(cornerRadius: Spacing.chipCorner, style: .continuous)
                    .stroke(Palette.divider, lineWidth: Spacing.hairline)
            )
            .contentShape(RoundedRectangle(cornerRadius: Spacing.chipCorner, style: .continuous))
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .combine)
    }
}

#Preview("ready") {
    Welcome(state: .ready, block: nil)
}

#Preview("device not eligible") {
    Welcome(state: .unavailable(""), block: .device)
}
