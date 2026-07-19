import SwiftUI

// Ilk acilis ekrani. Ortada sakin bir cumle, kisa aciklama
// ve dokununca giris alanina yazilan uc ornek istem.
struct BosDurum: View {
    // Cip secildiginde ilgili metni giris alanina yazar.
    let ornekSecildi: (String) -> Void

    // Ornek istemler — kullanıcının diline yerelleştirilir (String Catalog).
    // Dokununca giriş alanına yazılan metin de o dilde olur → model o dilde yanıtlar.
    private let ornekler = [
        String(localized: "Yarın neler var?"),
        String(localized: "Beni 18.00'de ara demek için hatırlat"),
        String(localized: "Geçen haftaki toplantı notumu bul"),
    ]

    var body: some View {
        VStack(spacing: Olcek.s4) {
            SirrMark(boyut: 44)
                .padding(.bottom, Olcek.s1)

            Text("Sorduğun burada kalır.")
                .font(Yazi.ketum())
                .foregroundStyle(Renk.murekkep)
                .multilineTextAlignment(.center)

            Text("sirr tamamen bu cihazda çalışır. Takvimine bakabilir, hatırlatıcı kurabilir, notlarında arayabilir.")
                .font(Yazi.cip())
                .foregroundStyle(Renk.gri)
                .multilineTextAlignment(.center)

            // Ornek istem cipleri, dar ekranda alt satira sarar.
            VStack(spacing: Olcek.s2) {
                ForEach(ornekler, id: \.self) { metin in
                    Cip(metin: metin) { ornekSecildi(metin) }
                }
            }
            .padding(.top, Olcek.s2)
        }
        .padding(.horizontal, Olcek.s5)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

// Tek satirlik pill cip. 1px cizgi kenar, gri metin.
private struct Cip: View {
    let metin: String
    let dokun: () -> Void

    var body: some View {
        Button(action: dokun) {
            Text(metin)
                .font(Yazi.cip())
                .foregroundStyle(Renk.gri)
                .padding(.horizontal, Olcek.s3)
                .padding(.vertical, Olcek.s2)
                .overlay(
                    RoundedRectangle(cornerRadius: Olcek.cipKose, style: .continuous)
                        .stroke(Renk.cizgi, lineWidth: Olcek.hairline)
                )
        }
        .buttonStyle(.plain)
    }
}

#Preview {
    BosDurum(ornekSecildi: { _ in })
        .background(Renk.zemin)
}
