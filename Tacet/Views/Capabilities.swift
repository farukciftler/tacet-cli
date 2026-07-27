//
//  Capabilities.swift
//  Tacet
//
//  The "what can it do" catalogue. NOT PART OF the welcome flow: it is not a
//  mandatory tour, it is a list the user opens themselves. At the end there is an
//  honest "what it can't do" section — a limit that is never spoken about turns
//  into disappointment the moment it is hit.
//

import SwiftUI

struct Capabilities: View {
    /// If nil, the rows are read-only (opened from Welcome and from Settings).
    /// If set, the example chips are tappable and the prompt is written into the
    /// input field.
    var exampleSelected: ((String) -> Void)? = nil

    @Environment(\.dismiss) private var close

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: Spacing.s5) {
                    Text("Everything below happens on your device. Web search and connections come into play only if you enable them, with the approval you see.")
                        .font(Typography.chip())
                        .foregroundStyle(Palette.grey)
                        .fixedSize(horizontal: false, vertical: true)

                    onDevice
                    documents
                    mathTimeCode
                    memoryAndSkills
                    ifYouTurnItOn
                    whatItCannotDo
                }
                .padding(.horizontal, Spacing.s5)
                .padding(.vertical, Spacing.s4)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .background(Palette.background)
            .navigationTitle("What Tacet can do")
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

    // MARK: - Sections

    private var onDevice: some View {
        section("ON YOUR DEVICE") {
            row(Text("Calendar"), Text("Reads your events, adds new ones."),
                example: String(localized: "What's on tomorrow?"))
            row(Text("Reminders"), Text("Sets reminders, lists the pending ones."),
                example: String(localized: "Remind me to make a call at 6:00 PM"))
            row(Text("Contacts"), Text("Finds numbers and email addresses in your contacts."),
                example: String(localized: "What’s Anna’s number?"))
            row(Text("Notes and files"), Text("Searches your notes and files on the device."),
                example: String(localized: "Find last week's meeting note"))
            row(Text("Write by voice"),
                Text("Tap the microphone in the input field; what you say turns into text on this device. The audio goes to no server."))
            note(Text("The first four and voice writing need permission. We don’t take it up front; the tool asks the moment you want it."))
        }
    }

    private var documents: some View {
        section("DOCUMENTS") {
            row(Text("Creates documents"),
                Text("Excel, Word, PDF, Markdown, plain text and a single-page website."),
                example: String(localized: "Turn this week’s notes into a document"))
            row(Text("Reads what you attach"),
                Text("Attach a document with the paper clip in the input field; Tacet summarizes it, answers questions from it, turns it into a table."))
            row(Text("Edits what it created"),
                Text("Adds rows, deletes them, changes the title; writes a new version."))
        }
    }

    private var mathTimeCode: some View {
        section("MATH, TIME, CODE") {
            row(Text("Math"), Text("Doesn’t do arithmetic in its head; it uses a tool."),
                // "%18" is not written out: in String Catalog extraction "%1" looks like a
                // format specifier. Writing the rate in words removes the problem.
                example: String(localized: "What is 1,499 plus 18 percent VAT?"))
            row(Text("Time"), Text("Tells today’s date, counts the days between two dates."),
                example: String(localized: "How many days until New Year?"))
            row(Text("Code"), Text("Runs short JavaScript in a closed box with no network and no file system."),
                example: String(localized: "List the prime numbers from 1 to 100"))
        }
    }

    /// This was the plan's "SCHEDULED JOBS" section; since the scheduled agent was cut
    /// from the product in this round, that row is gone and the heading was named after
    /// the two remaining capabilities.
    private var memoryAndSkills: some View {
        section("MEMORY AND SKILLS") {
            row(Text("Memory"),
                Text("Pulls lasting notes out of your chats and remembers them in later ones. In the Memory board you see them all and delete whichever you want."))
            row(Text("Skill"),
                Text("You write your own instruction; when your trigger word appears in a message, Tacet reads it and behaves accordingly."))
        }
    }

    private var ifYouTurnItOn: some View {
        section("IF YOU TURN IT ON") {
            row(Text("Web search"),
                Text("Connect your own search server and Tacet searches the web. Until you do, web search stays off. Settings > Web search."))
            row(Text("Connections"),
                Text("If you connect your own MCP servers, Tacet uses their tools. Before anything leaves, it shows you what’s leaving and asks for your approval. Settings > Connections."))
        }
    }

    private var whatItCannotDo: some View {
        section("WHAT IT CAN’T DO") {
            note(Text("Its general world knowledge is weak. When unsure, it doesn’t make things up — it says it doesn’t know."))
            note(Text("In long conversations it may lose the thread; starting a new chat usually fixes it."))
            note(Text("It doesn’t generate images, doesn’t answer out loud, and doesn’t send email or messages."))
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
                .accessibilityAddTraits(.isHeader)
            content()
        }
    }

    /// Title + description are ONE VoiceOver element; the example chip stays a separate
    /// button (different behaviour, different hint).
    @ViewBuilder
    private func row(_ title: Text, _ description: Text, example: String? = nil) -> some View {
        VStack(alignment: .leading, spacing: Spacing.s2) {
            VStack(alignment: .leading, spacing: Spacing.s1) {
                title
                    .font(Typography.user())
                    .foregroundStyle(Palette.ink)
                description
                    .font(Typography.chip())
                    .foregroundStyle(Palette.grey)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityElement(children: .combine)

            if let example {
                if let exampleSelected {
                    SampleChip(text: example) {
                        // Close first, then write: if the input field's focus is set up
                        // while the sheet is closing, the keyboard does not open.
                        close()
                        exampleSelected(example)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                } else {
                    // A dead end must not be a button: an example that cannot be selected
                    // is plain text.
                    Text(example)
                        .font(Typography.chip())
                        .foregroundStyle(Palette.muted)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func note(_ text: Text) -> some View {
        text
            .font(Typography.chip())
            .foregroundStyle(Palette.muted)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

#Preview("read-only") {
    Capabilities()
}

#Preview("example selectable") {
    Capabilities(exampleSelected: { _ in })
}
