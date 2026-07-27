//
//  ChatList.swift
//  Tacet
//
//  Chat history (spec §4.7). Reaching old chats, a new chat, deleting.
//  A plain list: title + last-line preview + date. No decoration.
//

import SwiftUI
import SwiftData

struct ChatList: View {
    let chats: [Chat]
    let activeID: UUID?
    let select: (Chat) -> Void
    let delete: (Chat) -> Void
    let new: () -> Void
    var openSkills: () -> Void = {}
    var openMemory: () -> Void = {}
    var openSettings: () -> Void = {}

    @Environment(\.dismiss) private var close

    @State private var search = ""
    /// The ids of the chats matching the search. `nil` = no filter (the search is empty
    /// or the scan has not run yet). IDs are held, NOT Chat objects: a deleted chat never
    /// stays hanging in the list, the displayed array is always derived from the live
    /// `chats`.
    @State private var matching: Set<UUID>?
    /// The chat waiting for delete confirmation. Nothing is deleted before confirmation.
    @State private var toDelete: Chat?
    /// The dot and the icon column grow with the text; alignment must not break under
    /// Dynamic Type.
    @ScaledMetric(relativeTo: .callout) private var dotSize = Spacing.dot
    @ScaledMetric(relativeTo: .callout) private var iconColumn = Spacing.iconColumn

    /// The trigger for the full-text scan. It re-filters when the term or the list
    /// length changes; not on every body draw.
    private struct FilterKey: Equatable {
        let term: String
        let count: Int
    }

    /// The array drawn on screen. The heavy work (searching message contents) is NOT
    /// DONE here; only the ready-made id set is consulted.
    private var shown: [Chat] {
        guard let matching else { return chats }
        return chats.filter { matching.contains($0.id) }
    }

    var body: some View {
        // A single read: the same filtering must not be repeated three times in the body.
        let list = shown
        return NavigationStack {
            List {
                // While a search is running the menu rows must not clog the path.
                if search.isEmpty {
                    menuRow(icon: "wand.and.stars", title: "Skills", action: openSkills)
                    menuRow(icon: "text.book.closed", title: "Memory", action: openMemory)
                    menuRow(icon: "gearshape", title: "Settings", action: openSettings)
                }

                if !search.isEmpty && list.isEmpty {
                    Text("No matching conversations.")
                        .font(Typography.chip())
                        .foregroundStyle(Palette.grey)
                        .listRowBackground(Palette.background)
                }

                ForEach(list) { chat in
                    Button {
                        select(chat)
                    } label: {
                        row(chat)
                    }
                    .buttonStyle(.plain)
                    .listRowBackground(Palette.background)
                }
                .onDelete { indices in
                    // Take the object first: coming back to a live array by index at the
                    // moment of deletion produces a shift.
                    toDelete = indices.map { list[$0] }.first
                }
            }
            .listStyle(.plain)
            .background(Palette.background)
            .searchable(text: $search, prompt: Text("Search conversations"))
            // The full-text scan runs once when typing stops, not per keystroke: a new
            // keystroke cancels the task, the sleep throws, and the scan never starts.
            .task(id: FilterKey(term: search, count: chats.count)) {
                let term = search.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !term.isEmpty else { matching = nil; return }
                do { try await Task.sleep(for: .milliseconds(200)) } catch { return }
                guard !Task.isCancelled else { return }
                matching = Set(chats.lazy.filter { chat in
                    if chat.title.localizedCaseInsensitiveContains(term) { return true }
                    return chat.messages.contains { $0.content.localizedCaseInsensitiveContains(term) }
                }.map(\.id))
            }
            .confirmationDialog("Delete this conversation?",
                                isPresented: Binding(get: { toDelete != nil },
                                                     set: { if !$0 { toDelete = nil } }),
                                titleVisibility: .visible,
                                presenting: toDelete) { chat in
                Button("Delete", role: .destructive) {
                    // Hand it out BEFORE deleting; afterwards no field of the object is
                    // touched.
                    toDelete = nil
                    delete(chat)
                }
                Button("Cancel", role: .cancel) { toDelete = nil }
            } message: { chat in
                Text("“\(Self.displayName(chat))” and the messages in it are deleted. This can’t be undone.")
            }
            .navigationTitle("Conversations")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Close") { close() }
                        .font(Typography.chip())
                        .foregroundStyle(Palette.grey)
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button(action: new) {
                        Image(systemName: "square.and.pencil")
                            .foregroundStyle(Palette.ink)
                    }
                    .accessibilityLabel("New chat")
                }
            }
        }
    }

    /// The three navigation rows at the top of the list — one body, one accessibility button.
    private func menuRow(icon: String,
                         title: LocalizedStringKey,
                         action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: Spacing.s3) {
                Image(systemName: icon)
                    .font(Typography.icon())
                    .foregroundStyle(Palette.grey)
                    .frame(width: iconColumn)
                Text(title)
                    .font(Typography.user())
                    .foregroundStyle(Palette.ink)
                Spacer()
                Image(systemName: "chevron.right")
                    .font(Typography.iconSmall())
                    .foregroundStyle(Palette.muted)
            }
            .padding(.vertical, Spacing.s1)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .listRowBackground(Palette.background)
        // The icon and the chevron are decoration; the only thing read out is the row's name.
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(title))
        .accessibilityAddTraits(.isButton)
    }

    private func row(_ chat: Chat) -> some View {
        let isActive = chat.id == activeID
        let name = Self.displayName(chat)
        let when = Self.date(chat.updatedAt)

        return HStack(spacing: Spacing.s3) {
            // A small ink dot for the active chat (brand: not colour, weight/mark).
            // A visual indicator; its counterpart is put into words below with
            // accessibilityValue.
            Circle()
                .fill(isActive ? Palette.ink : Color.clear)
                .frame(width: dotSize, height: dotSize)

            VStack(alignment: .leading, spacing: Spacing.s1) {
                Text(name)
                    .font(Typography.user())
                    .foregroundStyle(Palette.ink)
                    .lineLimit(1)
                Text(chat.preview)
                    .font(Typography.chip())
                    .foregroundStyle(Palette.grey)
                    .lineLimit(1)
            }
            Spacer()
            Text(when)
                .font(Typography.chip())
                .foregroundStyle(Palette.muted)
        }
        .padding(.vertical, Spacing.s1)
        .contentShape(Rectangle())
        // Read as a single chat row, not four separate pieces. The active dot must not
        // stay purely visual, so the state is also put into words.
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(verbatim: "\(name). \(chat.preview)"))
        .accessibilityValue(isActive
                            ? Text("Open conversation, \(when)")
                            : Text(verbatim: when))
        .accessibilityHint(Text("Double-tap to open this conversation."))
        .accessibilityAddTraits(isActive ? [.isButton, .isSelected] : [.isButton])
    }

    /// The chat name shown in the list. An `isEmpty` check alone was not enough: Chat's
    /// default title was a raw Turkish "Yeni sohbet" literal, i.e. not empty — so it
    /// showed up in Turkish even in the English UI. If the title is still automatic (the
    /// user or the first message has not set it), we show the localised counterpart.
    private static func displayName(_ chat: Chat) -> String {
        let title = chat.title.trimmingCharacters(in: .whitespacesAndNewlines)
        if title.isEmpty || chat.titleIsAutomatic { return String(localized: "New chat") }
        return title
    }

    /// Creating a `DateFormatter` per row was expensive; a fixed "HH:mm" pattern also
    /// ignored the user's 12/24-hour preference. `FormatStyle` solves both.
    private static func date(_ d: Date) -> String {
        let c = Calendar.current
        if c.isDateInToday(d) { return d.formatted(.dateTime.hour().minute()) }
        if c.isDateInYesterday(d) { return L10n.yesterday }
        return d.formatted(.dateTime.day().month(.abbreviated))
    }
}
