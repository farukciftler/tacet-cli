import SwiftUI

// Sohbette gün geçişini gösteren ortalı ayraç.
// "BUGÜN", "DÜN" ya da "12 TEMMUZ" biçiminde.
struct TarihAyraci: View {
    let tarih: Date

    var body: some View {
        Text(etiket)
            .font(Yazi.etiket())
            .tracking(1.4)
            .foregroundStyle(Renk.gri)
            .frame(maxWidth: .infinity, alignment: .center)
            .padding(.vertical, Olcek.s2)
    }

    // Göreli günler önce; değilse "d MMMM" büyük harf. Cihaz diline göre yerelleşir.
    private var etiket: String {
        if Calendar.current.isDateInToday(tarih) {
            return Yerel.bugun
        }
        if Calendar.current.isDateInYesterday(tarih) {
            return Yerel.dunUst
        }
        return TarihAyraci.bicimlendirici.string(from: tarih)
            .uppercased(with: Locale.current)
    }

    // Ay adı cihaz diline göre biçimlenir.
    private static let bicimlendirici: DateFormatter = {
        let df = DateFormatter()
        df.locale = Locale.current
        df.dateFormat = "d MMMM"
        return df
    }()
}

#Preview {
    VStack(spacing: Olcek.s4) {
        TarihAyraci(tarih: Date())
        TarihAyraci(tarih: Calendar.current.date(byAdding: .day, value: -1, to: Date())!)
        TarihAyraci(tarih: Calendar.current.date(byAdding: .day, value: -8, to: Date())!)
    }
    .background(Renk.zemin)
}
