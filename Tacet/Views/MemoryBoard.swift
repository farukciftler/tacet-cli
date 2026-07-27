//
//  MemoryBoard.swift
//  Tacet
//
//  The memory board (memory-spec §6.1) — the same skeleton as SkillBoard.
//
//  The layer's first principle becomes visible here: "no silent learning". Every
//  extracted note sits in this list, can be edited and can be deleted. There is NO
//  "I took a note" chip inside the chat — the chip language belongs to tool calls and
//  extraction is not part of the chat turn (spec §6.2).
//

import SwiftUI
import SwiftData

struct MemoryBoard: View {
    @Query(sort: \MemoryNote.createdAt, order: .reverse)
    private var notes: [MemoryNote]

    /// So the source chat's date can be shown. If the chat was deleted the source row is
    /// hidden and the note stays (spec §7).
    @Query private var chats: [Chat]

    @Environment(\.modelContext) private var record
    @Environment(\.dismiss) private var close

    @State private var editing: MemoryNote?
    @State private var deleteApproval = false
    /// The user-visible counterpart of a write failure.
    @State private var warningText: String?

    var body: some View {
        NavigationStack {
            list
                .background(Palette.background)
                .navigationTitle("Memory")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) {
                        Button("Close") { close() }
                            .font(Typography.chip())
                            .foregroundStyle(Palette.grey)
                    }
                }
                .sheet(item: $editing) { note in
                    MemoryEditor(note: note, save: update)
                }
                .confirmationDialog("Delete everything in memory?",
                                    isPresented: $deleteApproval,
                                    titleVisibility: .visible) {
                    Button("Delete", role: .destructive) { deleteAll() }
                    Button("Cancel", role: .cancel) { }
                } message: {
                    Text("All learned notes are deleted. Conversations are kept. This can’t be undone.")
                }
                .issueBanner($warningText)
        }
    }

    // MARK: - List

    private var list: some View {
        // The source dates are put into a dictionary ONCE: scanning all the chats with
        // `first(where:)` on every note row meant note-count × chat-count work.
        // The date is held, not the Chat object — a deleted model is never touched later.
        let sources = Dictionary(chats.map { ($0.id, $0.createdAt) },
                                 uniquingKeysWith: { first, _ in first })
        return List {
            Section {
                if notes.isEmpty {
                    emptyState
                } else {
                    Text("These notes never leave your device. Tacet looks at them when they’re relevant.")
                        .font(Typography.chip())
                        .foregroundStyle(Palette.grey)
                        .padding(.bottom, Spacing.s1)
                        .boardRow()
                    if notes.count >= MemoryNote.totalCap {
                        Text("Memory is full — no new notes are being added. Delete a few to make room.")
                            .font(Typography.chip())
                            .foregroundStyle(Palette.error)
                            .padding(.bottom, Spacing.s1)
                            .boardRow()
                    }
                }
            }

            ForEach(notes) { note in
                Button { editing = note } label: { row(note, sources: sources) }
                    .buttonStyle(.plain)
                    .boardRow()
            }
            .onDelete { indices in
                // Collect the objects first: deleting by index on a live @Query produces a
                // shift.
                for note in indices.map({ notes[$0] }) { delete(note) }
            }

            if !notes.isEmpty {
                Section {
                    deleteAllRow.boardRow()
                }
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
    }

    private func row(_ note: MemoryNote, sources: [UUID: Date]) -> some View {
        VStack(alignment: .leading, spacing: Spacing.s1) {
            Text(note.text)
                .font(Typography.user())
                .foregroundStyle(Palette.ink)
                .multilineTextAlignment(.leading)
                .lineLimit(3)
            HStack(spacing: Spacing.s2) {
                Text(Self.kindLabel(note.kind))
                    .font(Typography.chip())
                    .foregroundStyle(Palette.grey)
                if let source = Self.sourceText(note, sources: sources) {
                    Text(source)
                        .font(Typography.chip())
                        .foregroundStyle(Palette.muted)
                        .lineLimit(1)
                }
                if !note.isActive {
                    Text("off")
                        .font(Typography.chip())
                        .foregroundStyle(Palette.muted)
                }
            }
        }
        .padding(.vertical, Spacing.s3)
        .padding(.horizontal, Spacing.s4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
        .cardFrame()
        // Not three separate texts but a single note row. The "off" badge turns into a
        // state.
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(verbatim: "\(note.text). \(Self.kindLabel(note.kind))"))
        .accessibilityValue(note.isActive ? Text("On") : Text("Off"))
        .accessibilityHint(Text("Double-tap to edit."))
        .accessibilityAddTraits(.isButton)
    }

    private var deleteAllRow: some View {
        Button(role: .destructive) { deleteApproval = true } label: {
            HStack(spacing: Spacing.s2) {
                Text("Delete all")
                Spacer(minLength: 0)
            }
            .font(Typography.user())
            .foregroundStyle(Palette.error)
            .padding(.vertical, Spacing.s3)
            .padding(.horizontal, Spacing.s4)
            .contentShape(Rectangle())
            .cardFrame()
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text("Delete everything in memory"))
        .accessibilityHint(Text("You’ll be asked to confirm. All notes are deleted."))
    }

    private var emptyState: some View {
        VStack(alignment: .leading, spacing: Spacing.s2) {
            Text("Tacet hasn’t learned anything yet.")
                .font(Typography.tacet())
                .foregroundStyle(Palette.ink)
            Text("As you mention things about yourself in conversations, they show up here — and stay only here.")
                .font(Typography.chip())
                .foregroundStyle(Palette.grey)
        }
        .padding(.vertical, Spacing.s4)
        .boardRow()
    }

    // MARK: - Source

    /// "from the 12 Jul conversation" — nil if the source chat was deleted (the row is
    /// hidden).
    private static func sourceText(_ note: MemoryNote, sources: [UUID: Date]) -> String? {
        guard let id = note.sourceChatID, let date = sources[id] else { return nil }
        return String(localized: "from the \(date.formatted(.dateTime.day().month(.abbreviated))) conversation")
    }

    static func kindLabel(_ kind: MemoryKind) -> String {
        switch kind {
        case .identity:   String(localized: "identity")
        case .preference: String(localized: "preference")
        case .relation:   String(localized: "relationship")
        case .fact:       String(localized: "fact")
        }
    }

    // MARK: - Saving

    private func update(_ draft: MemoryDraft) {
        guard let note = editing, !note.isDeleted else { return }
        note.text = draft.text
        note.rawKeys = draft.keys
        note.isActive = draft.isActive
        save(String(localized: "Couldn’t save the note"))
    }

    private func delete(_ note: MemoryNote) {
        guard !note.isDeleted else { return }
        record.delete(note)
        // If the delete cannot be written it is rolled back: let the list show the truth,
        // so the user does not believe they deleted it and miss that the note is still
        // being injected.
        save(String(localized: "Couldn’t delete the note"), rollback: true)
    }

    private func deleteAll() {
        // Collect the objects BEFORE deleting; touching a deleted record afterwards is
        // fatal.
        let all = notes
        guard !all.isEmpty else { return }
        for note in all where !note.isDeleted { record.delete(note) }
        save(String(localized: "Couldn’t delete the notes"), rollback: true)
    }

    /// Writes to disk and refreshes the store — the model reads the new state on the next
    /// message. The write error is not swallowed: the user sees that the note was not
    /// saved.
    private func save(_ cause: String, rollback: Bool = false) {
        record.boardSave(cause, rollback: rollback, warning: $warningText)
        // Even on an error, sync the store with the context's current state — the model
        // and the screen must not diverge.
        MemoryStore.reload((try? record.fetch(FetchDescriptor<MemoryNote>())) ?? [])
    }
}

/// A plain value returned from the editor to the board — so it can be saved without
/// touching a deleted model.
struct MemoryDraft {
    var text: String
    var keys: String
    var isActive: Bool
}

// MARK: - Editor

private struct MemoryEditor: View {
    let note: MemoryNote
    let save: (MemoryDraft) -> Void

    @Environment(\.dismiss) private var close
    @State private var text = ""
    @State private var keys = ""
    @State private var isActive = true
    @State private var loaded = false

    private var remaining: Int { MemoryNote.textLimit - text.count }
    private var isValid: Bool {
        !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !keys.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && remaining >= 0
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextEditor(text: $text)
                        .font(Typography.user())
                        .frame(minHeight: 90)
                        .scrollContentBackground(.hidden)
                        .accessibilityLabel(Text("Note text"))
                } header: {
                    HStack {
                        Text("Note")
                        Spacer()
                        Text("\(max(remaining, 0))")
                            .foregroundStyle(remaining < 0 ? Palette.error : Palette.muted)
                            .monospacedDigit()
                            // It must not be read as a bare number.
                            .accessibilityLabel(Text("Characters left"))
                            .accessibilityValue(Text(verbatim: "\(max(remaining, 0))"))
                    }
                } footer: {
                    Text("One sentence, up to \(MemoryNote.textLimit) characters. You can rewrite it in your own words.")
                }

                Section {
                    TextField("food, restaurant, evening", text: $keys, axis: .vertical)
                        .font(Typography.user())
                        .lineLimit(1...3)
                        .accessibilityLabel(Text("Keywords"))
                        .accessibilityHint(Text("Separate with commas."))
                } header: {
                    Text("Match words")
                } footer: {
                    Text("Separate with commas. When one of these words appears in a message, the note is given to Tacet.")
                }

                Section {
                    Toggle("On", isOn: $isActive)
                        .font(Typography.user())
                } footer: {
                    Text("A note that’s off is kept but never given to Tacet.")
                }
            }
            .scrollContentBackground(.hidden)
            .background(Palette.background)
            .navigationTitle("Note")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Cancel") { close() }
                        .font(Typography.chip())
                        .foregroundStyle(Palette.grey)
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Save") {
                        save(MemoryDraft(
                            text: text.trimmingCharacters(in: .whitespacesAndNewlines),
                            keys: keys,
                            isActive: isActive
                        ))
                        close()
                    }
                    .font(Typography.chip())
                    .foregroundStyle(isValid ? Palette.ink : Palette.muted)
                    .disabled(!isValid)
                }
            }
            .onAppear {
                guard !loaded, !note.isDeleted else { return }
                loaded = true
                text = note.text
                keys = note.rawKeys
                isActive = note.isActive
            }
            // Apply the limit while typing — the user must not silently exceed it and lose
            // text.
            .onChange(of: text) { _, new in
                if new.count > MemoryNote.textLimit {
                    text = String(new.prefix(MemoryNote.textLimit))
                }
            }
        }
    }
}
