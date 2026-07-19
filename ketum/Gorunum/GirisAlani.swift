import SwiftUI

// Giriş alanı: pill biçimli metin kutusu ve dairesel gönder düğmesi.
// Spec §4.5. Metin boşken düğme pasiftir ama görünümü değişmez.
struct GirisAlani: View {
    @Binding var metin: String
    let gonder: () -> Void
    /// Belge ekleme (okuma/düzenleme için). nil ise ek düğmesi gösterilmez.
    var ekle: (() -> Void)? = nil

    // Metin sadece boşluksa gönderim yapılmaz.
    private var bosMu: Bool {
        metin.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var body: some View {
        HStack(spacing: Olcek.s2) {
            if let ekle {
                Button(action: ekle) {
                    Image(systemName: "paperclip")
                        .font(.system(size: 16, weight: .regular))
                        .foregroundStyle(Renk.gri)
                        .frame(width: 28, height: 28)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Belge ekle")
            }

            TextField("", text: $metin, prompt: yerTutucu)
                .font(Yazi.kullanici())
                .foregroundStyle(Renk.murekkep)
                .textFieldStyle(.plain)
                .onSubmit(gonder)

            gonderDugmesi
        }
        .padding(.leading, ekle == nil ? Olcek.s4 : Olcek.s2)
        .padding(.trailing, Olcek.s1)
        .padding(.vertical, Olcek.s1)
        .overlay(
            RoundedRectangle(cornerRadius: Olcek.girisKose)
                .stroke(Renk.cizgi, lineWidth: Olcek.hairline)
        )
        .padding(.horizontal, Olcek.s5)
    }

    // Placeholder metni sözleşmedeki soluk renkte.
    private var yerTutucu: Text {
        Text("sirr'e sor").foregroundStyle(Renk.soluk)
    }

    // Dairesel gönder düğmesi: mürekkep dolgu, beyaz yukarı ok, ~32pt.
    // Boşken görünüm AYNI kalır ama eylem tetiklenmez — soluk/disabled düğme yok (spec §4.5).
    private var gonderDugmesi: some View {
        Button {
            guard !bosMu else { return }
            gonder()
        } label: {
            Image(systemName: "arrow.up")
                .font(.system(size: 15, weight: .medium))
                // Ok, dolgunun (murekkep) zıttı: zemin rengi. Karanlık modda murekkep
                // açık olduğundan beyaz ok görünmez oluyordu; zemin = koyu ok, kontrast korunur.
                .foregroundStyle(Renk.zemin)
                .frame(width: 32, height: 32)
                .background(Renk.murekkep, in: Circle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Gönder")
    }
}

#Preview {
    struct Onizleme: View {
        @State private var bos = ""
        @State private var dolu = "Yarınki toplantıları göster"
        var body: some View {
            VStack(spacing: Olcek.mesajAraligi) {
                GirisAlani(metin: $bos, gonder: {})
                GirisAlani(metin: $dolu, gonder: {})
            }
            .padding(.vertical, Olcek.s4)
        }
    }
    return Onizleme()
}
