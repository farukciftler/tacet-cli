import SwiftUI

// The centred separator that marks a day change in the chat.
// In the form "TODAY", "YESTERDAY" or "12 JULY".
struct DateSeparator: View {
    let date: Date

    var body: some View {
        Text(tag)
            .font(Typography.tag())
            .tracking(1.4)
            .foregroundStyle(Palette.grey)
            .frame(maxWidth: .infinity, alignment: .center)
            .padding(.vertical, Spacing.s2)
    }

    // Relative days first; otherwise "d MMMM" in uppercase. Localised per device language.
    private var tag: String {
        if Calendar.current.isDateInToday(date) {
            return L10n.today
        }
        if Calendar.current.isDateInYesterday(date) {
            return L10n.yesterdayUpper
        }
        return DateSeparator.formatter.string(from: date)
            .uppercased(with: Locale.current)
    }

    // The month name is formatted per device language.
    private static let formatter: DateFormatter = {
        let df = DateFormatter()
        df.locale = Locale.current
        df.dateFormat = "d MMMM"
        return df
    }()
}

#Preview {
    VStack(spacing: Spacing.s4) {
        DateSeparator(date: Date())
        DateSeparator(date: Calendar.current.date(byAdding: .day, value: -1, to: Date())!)
        DateSeparator(date: Calendar.current.date(byAdding: .day, value: -8, to: Date())!)
    }
    .background(Palette.background)
}
