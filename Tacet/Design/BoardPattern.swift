//
//  BoardPattern.swift
//  Tacet
//
//  Patterns that were repeated verbatim across the boards (memory, skills) and
//  the root view. The warning sheet, the list row background, the card frame and
//  the SwiftData write path — copied up to five times — now sit in one place:
//  when the text or the behaviour changes, one file changes.
//
//  NOTHING here MAKES a new visual decision — the values come from the
//  `Spacing`/`Palette` tokens and are identical to how they looked where they
//  were moved from.
//

import SwiftUI
import SwiftData

// MARK: - Warning

/// The single user-visible form of a write/run failure.
///
/// If the text is not `nil` the sheet opens; when it closes the text is reset
/// (the `set` end of the binding). The reset comes from two places — the button
/// and the system's own dismissal — because on swipe-to-dismiss the button's
/// action does not run.
private struct IssueBanner: ViewModifier {
    @Binding var text: String?

    func body(content: Content) -> some View {
        content.alert(Text("Something went wrong"), isPresented: Binding(
            get: { text != nil },
            set: { if !$0 { text = nil } }
        )) {
            Button(role: .cancel) { text = nil } label: { Text("OK") }
        } message: {
            if let text { Text(text) }
        }
    }
}

extension View {
    /// Shows the error text in the one standard warning sheet. While `nil`, nothing happens.
    func issueBanner(_ text: Binding<String?>) -> some View {
        modifier(IssueBanner(text: text))
    }
}

// MARK: - List row

extension View {
    /// The shared background of every board row: no system separators, no system insets.
    func boardRow() -> some View {
        listRowBackground(Palette.background)
            .listRowSeparator(.hidden)
            .listRowInsets(EdgeInsets(top: Spacing.s1, leading: Spacing.s5,
                                      bottom: Spacing.s1, trailing: Spacing.s5))
    }

    /// Card frame: hairline, continuous corner. `dashed` is used only on "new …"
    /// rows — there the frame describes something that does not exist yet.
    func cardFrame(dashed: Bool = false) -> some View {
        overlay(
            RoundedRectangle(cornerRadius: Spacing.s4, style: .continuous)
                .stroke(Palette.divider, style: dashed
                        ? StrokeStyle(lineWidth: Spacing.hairline, dash: [4, 4])
                        : StrokeStyle(lineWidth: Spacing.hairline))
        )
    }
}

// MARK: - Saving

extension ModelContext {
    /// The shared write path of the boards. The error is not swallowed: the user
    /// sees that nothing was saved.
    ///
    /// `rollback` is passed on delete paths — if the write never reached disk and
    /// the context is not rolled back, the list would show the row as deleted and
    /// hide the truth. On insert/update no rollback happens: keeping what the user
    /// typed on screen is preferable.
    ///
    /// Not touching a deleted model stays the CALLER's responsibility (the
    /// `!isDeleted` guards): this place does not know which object is being written.
    @MainActor
    func boardSave(_ cause: String, rollback shouldRollback: Bool = false, warning: Binding<String?>) {
        do {
            try save()
        } catch {
            if shouldRollback { rollback() }
            warning.wrappedValue = "\(cause): \(error.localizedDescription)"
        }
    }
}
