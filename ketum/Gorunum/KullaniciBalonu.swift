import SwiftUI

// Kullanıcı mesaj balonu — spec §4.2.
// Sağa hizalı, dolgu zeminli, sağ altta kısa köşe (kuyruk).
struct KullaniciBalonu: View {
    let metin: String

    var body: some View {
        HStack {
            // Sol boşluk balonu sağa iter.
            Spacer(minLength: Olcek.s4)

            Text(metin)
                .font(Yazi.kullanici())
                .foregroundStyle(Renk.murekkep)
                .padding(.horizontal, Olcek.s3)
                .padding(.vertical, Olcek.s2)
                .background(
                    UnevenRoundedRectangle(
                        topLeadingRadius: 18,
                        bottomLeadingRadius: 18,
                        bottomTrailingRadius: 5,
                        topTrailingRadius: 18
                    )
                    .fill(Renk.dolgu)
                )
        }
        .frame(maxWidth: .infinity, alignment: .trailing)
    }
}

#Preview {
    VStack(spacing: Olcek.mesajAraligi) {
        KullaniciBalonu(metin: "Selam, bugün hava nasıl?")
        KullaniciBalonu(metin: "Bana yarın için kısa bir hatırlatma oluşturur musun? Saat 9'da toplantı var.")
    }
    .padding(Olcek.s5)
    .background(Renk.zemin)
}
