//
//  SkillBoard.swift
//  Tacet
//
//  The skill board — guides the user writes themselves (the equivalent of SKILL.md).
//  A skill is attached to a chat once, when its trigger words appear in a message. The
//  body is hard-limited: the window is 4096 tokens, every character is budget.
//

import SwiftUI
import SwiftData

struct SkillBoard: View {
    @Query(sort: \UserSkill.createdAt, order: .reverse)
    private var skills: [UserSkill]

    @Environment(\.modelContext) private var record
    @Environment(\.dismiss) private var close

    @State private var editing: UserSkill?
    @State private var newOpen = false
    /// The user-visible counterpart of a write failure.
    @State private var warningText: String?

    var body: some View {
        NavigationStack {
            list
                .background(Palette.background)
                .navigationTitle("Skills")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) {
                        Button("Close") { close() }
                            .font(Typography.chip())
                            .foregroundStyle(Palette.grey)
                    }
                }
                .sheet(isPresented: $newOpen) {
                    SkillEditor(skill: nil, save: add)
                }
                .sheet(item: $editing) { skill in
                    SkillEditor(skill: skill, save: update)
                }
                .issueBanner($warningText)
        }
    }

    // MARK: - List

    private var list: some View {
        List {
            Section {
                if skills.isEmpty {
                    emptyState
                } else {
                    Text("Tacet reads this guide whenever your trigger word appears in a message.")
                        .font(Typography.chip())
                        .foregroundStyle(Palette.grey)
                        .padding(.bottom, Spacing.s1)
                        .boardRow()
                }
            }

            ForEach(skills) { skill in
                Button { editing = skill } label: { row(skill) }
                    .buttonStyle(.plain)
                    .boardRow()
            }
            .onDelete { indices in
                // Collect the objects first: deleting by index on a live @Query produces a
                // shift.
                for skill in indices.map({ skills[$0] }) { delete(skill) }
            }

            Section {
                newRow.boardRow()
            }

            Section {
                Text("Tacet’s built-in skills")
                    .font(Typography.tag())
                    .foregroundStyle(Palette.muted)
                    .textCase(.uppercase)
                    .accessibilityAddTraits(.isHeader)
                    .boardRow()
                ForEach(SkillStore.package, id: \.name) { skill in
                    VStack(alignment: .leading, spacing: Spacing.s1) {
                        Text(skill.name)
                            .font(Typography.user())
                            .foregroundStyle(Palette.grey)
                        Text(skill.triggers.prefix(5).joined(separator: " · "))
                            .font(Typography.chip())
                            .foregroundStyle(Palette.muted)
                            .lineLimit(1)
                    }
                    .padding(.vertical, Spacing.s1)
                    // The "·" separators must not be read one by one; it is heard as a
                    // single line.
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(Text(verbatim: skill.name))
                    .accessibilityValue(Text("Triggers: \(skill.triggers.prefix(5).joined(separator: ", "))"))
                    .boardRow()
                }
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
    }

    private func row(_ skill: UserSkill) -> some View {
        VStack(alignment: .leading, spacing: Spacing.s1) {
            HStack(spacing: Spacing.s2) {
                Text(skill.name)
                    .font(Typography.user())
                    .foregroundStyle(Palette.ink)
                    .lineLimit(1)
                if !skill.isActive {
                    Text("off")
                        .font(Typography.chip())
                        .foregroundStyle(Palette.muted)
                }
            }
            Text(skill.summary)
                .font(Typography.chip())
                .foregroundStyle(Palette.grey)
                .lineLimit(1)
            Text(skill.body)
                .font(Typography.chip())
                .foregroundStyle(Palette.muted)
                .lineLimit(2)
        }
        .padding(.vertical, Spacing.s3)
        .padding(.horizontal, Spacing.s4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
        .cardFrame()
        // Not three separate texts but a single skill row. The "off" badge turns into a
        // state.
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(verbatim: "\(skill.name). \(skill.summary)"))
        .accessibilityValue(skill.isActive ? Text("On") : Text("Off"))
        .accessibilityHint(Text("Double-tap to edit."))
        .accessibilityAddTraits(.isButton)
    }

    private var newRow: some View {
        Button { newOpen = true } label: {
            HStack(spacing: Spacing.s2) {
                Image(systemName: "plus")
                    .accessibilityHidden(true)
                Text("New skill")
                Spacer(minLength: 0)
            }
            .font(Typography.user())
            .foregroundStyle(Palette.grey)
            .padding(.vertical, Spacing.s3)
            .padding(.horizontal, Spacing.s4)
            .cardFrame(dashed: true)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text("New skill"))
        .accessibilityHint(Text("Double-tap to write your own guide."))
    }

    private var emptyState: some View {
        VStack(alignment: .leading, spacing: Spacing.s2) {
            Text("No skills yet.")
                .font(Typography.tacet())
                .foregroundStyle(Palette.ink)
            Text("Teach Tacet your own rule: write when it should kick in and what it should do.")
                .font(Typography.chip())
                .foregroundStyle(Palette.grey)
        }
        .padding(.vertical, Spacing.s4)
        .boardRow()
    }

    // MARK: - Saving

    private func add(_ draft: SkillDraft) {
        let new = UserSkill(name: draft.name, rawTriggers: draft.triggers, body: draft.body)
        new.isActive = draft.isActive
        record.insert(new)
        save(String(localized: "Couldn’t save the skill"))
    }

    private func update(_ draft: SkillDraft) {
        guard let skill = editing, !skill.isDeleted else { return }
        skill.name = draft.name
        skill.rawTriggers = draft.triggers
        skill.body = draft.body
        skill.isActive = draft.isActive
        save(String(localized: "Couldn’t save the skill"))
    }

    private func delete(_ skill: UserSkill) {
        guard !skill.isDeleted else { return }
        record.delete(skill)
        // If the delete cannot be written it is rolled back: let the list show the truth,
        // so the user does not believe it was deleted and miss that the skill is still
        // active.
        save(String(localized: "Couldn’t delete the skill"), rollback: true)
    }

    /// Writes to disk and refreshes the store — the model reads the new state on the next
    /// message. The write error is no longer swallowed: the user sees that the skill was
    /// not saved.
    private func save(_ cause: String, rollback: Bool = false) {
        record.boardSave(cause, rollback: rollback, warning: $warningText)
        // Even on an error, sync the store with the context's current state — the model
        // and the screen must not diverge.
        SkillStore.reloadUser(
            (try? record.fetch(FetchDescriptor<UserSkill>())) ?? []
        )
    }
}

/// A plain value returned from the editor to the board — so it can be saved without
/// touching a deleted model.
struct SkillDraft {
    var name: String
    var triggers: String
    var body: String
    var isActive: Bool
}

// MARK: - Editor

private struct SkillEditor: View {
    let skill: UserSkill?
    let save: (SkillDraft) -> Void

    @Environment(\.dismiss) private var close
    @State private var name = ""
    @State private var triggers = ""
    /// NOT named `body`: inside a View that name is taken by `var body: some View`.
    @State private var guideBody = ""
    @State private var isActive = true

    private var remaining: Int { UserSkill.bodyLimit - guideBody.count }
    private var isValid: Bool {
        !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !triggers.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !guideBody.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && remaining >= 0
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Bill tracking", text: $name)
                        .font(Typography.user())
                        // The placeholder is an example text; it must not be read as the label.
                        .accessibilityLabel(Text("Skill name"))
                } header: {
                    Text("Name")
                }

                Section {
                    TextField("bill, expense, spending", text: $triggers, axis: .vertical)
                        .font(Typography.user())
                        .lineLimit(1...3)
                        .accessibilityLabel(Text("Trigger words"))
                        .accessibilityHint(Text("Separate with commas."))
                } header: {
                    Text("Triggers")
                } footer: {
                    Text("Separate with commas. The skill kicks in when one of these words appears in your message.")
                }

                Section {
                    TextEditor(text: $guideBody)
                        .font(Typography.user())
                        .frame(minHeight: 160)
                        .scrollContentBackground(.hidden)
                        .accessibilityLabel(Text("Guide text"))
                } header: {
                    HStack {
                        Text("Guide")
                        Spacer()
                        Text("\(max(remaining, 0))")
                            .foregroundStyle(remaining < 0 ? Palette.error : Palette.muted)
                            .monospacedDigit()
                            // It must not be read as a bare number.
                            .accessibilityLabel(Text("Characters left"))
                            .accessibilityValue(Text(verbatim: "\(max(remaining, 0))"))
                    }
                } footer: {
                    Text("Keep it short and imperative — \(UserSkill.bodyLimit) characters max. Example: “When bills come up, search my notes with the search tool first, then pass the amount to the calculator tool.”")
                }

                Section {
                    Toggle("On", isOn: $isActive)
                        .font(Typography.user())
                }
            }
            .scrollContentBackground(.hidden)
            .background(Palette.background)
            .navigationTitle(skill == nil ? "New skill" : "Skill")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Cancel") { close() }
                        .font(Typography.chip())
                        .foregroundStyle(Palette.grey)
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Save") {
                        save(SkillDraft(
                            name: name.trimmingCharacters(in: .whitespacesAndNewlines),
                            triggers: triggers,
                            body: guideBody.trimmingCharacters(in: .whitespacesAndNewlines),
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
                guard let skill, name.isEmpty else { return }
                name = skill.name
                triggers = skill.rawTriggers
                guideBody = skill.body
                isActive = skill.isActive
            }
            // Apply the limit while typing — the user must not silently exceed it and lose
            // text.
            .onChange(of: guideBody) { _, new in
                if new.count > UserSkill.bodyLimit {
                    guideBody = String(new.prefix(UserSkill.bodyLimit))
                }
            }
        }
    }
}
