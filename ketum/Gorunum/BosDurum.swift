import SwiftUI

// Ilk acilis ekrani. Ortada sakin bir cumle, kisa aciklama
// ve dokununca giris alanina yazilan uc ornek istem.
struct BosDurum: View {
    // Cip secildiginde ilgili metni giris alanina yazar.
    let ornekSecildi: (String) -> Void

    // Ornek istemler — kullanıcının diline yerelleştirilir (String Catalog).
    // Dokununca giriş alanına yazılan metin de o dilde olur → model o dilde yanıtlar.
    // Örnekler yalnızca "soru sorulur" demiyor; ürünün görünmez yetenekleri
    // (nöbet, belge, hatırlatıcı, not arama) de buradan keşfediliyor.
    private let ornekler = [
        String(localized: "Yarın neler var?"),
        String(localized: "Beni 18.00'de ara demek için hatırlat"),
        String(localized: "Geçen haftaki toplantı notumu bul"),
        String(localized: "Her sabah 8'de günümü özetle"),
        String(localized: "Bu haftanın notlarından bir belge çıkar"),
    ]

    var body: some View {
        // Örnekler çoğaldı ve büyük punto ayarında ekrana sığmayabilir; kaydırma
        // güvencesi kırpılmayı önler, sığdığında kaydırma hissedilmez.
        ScrollView {
            icerik
                .padding(.horizontal, Olcek.s5)
                .padding(.vertical, Olcek.s5)
                .frame(maxWidth: .infinity)
                .containerRelativeFrame(.vertical, alignment: .center)
        }
        .scrollBounceBehavior(.basedOnSize)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var icerik: some View {
        VStack(spacing: Olcek.s4) {
            SirrMark(boyut: 44)
                .padding(.bottom, Olcek.s1)

            Text("Sorduğun burada kalır.")
                .font(Yazi.ketum())
                .foregroundStyle(Renk.murekkep)
                .multilineTextAlignment(.center)

            // Ürünün ana vaadi: ilk ekranda, dokunmadan, açık sözle.
            Text("sirr tamamen bu cihazda çalışır. Sorduğun, takvimin, notların cihazdan çıkmaz; internete gitmez.")
                .font(Yazi.cip())
                .foregroundStyle(Renk.gri)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)

            // Ornek istem cipleri, dar ekranda alt satira sarar.
            VStack(spacing: Olcek.s2) {
                ForEach(ornekler, id: \.self) { metin in
                    Cip(metin: metin) { ornekSecildi(metin) }
                }
            }
            .padding(.top, Olcek.s2)

            // Görünmeyen iki yetenek sözle duyurulur; ağır bir tanıtım akışı yok.
            Text("Düzenli işleri nöbete bağlayabilir, kendi yönergelerini beceri olarak kaydedebilirsin. İkisi de soldaki listeden yönetilir.")
                .font(Yazi.cip())
                .foregroundStyle(Renk.soluk)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.top, Olcek.s2)
        }
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
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.horizontal, Olcek.s3)
                .padding(.vertical, Olcek.s2)
                .overlay(
                    RoundedRectangle(cornerRadius: Olcek.cipKose, style: .continuous)
                        .stroke(Renk.cizgi, lineWidth: Olcek.hairline)
                )
        }
        .buttonStyle(.plain)
        .accessibilityHint(Text("Bu örneği giriş alanına yazar"))
    }
}

#Preview {
    BosDurum(ornekSecildi: { _ in })
        .background(Renk.zemin)
}
