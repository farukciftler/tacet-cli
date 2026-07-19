import SwiftUI

// Giriş alanı: pill biçimli metin kutusu ve dairesel gönder düğmesi.
// Spec §4.5. Metin boşken düğme pasiftir ama görünümü değişmez.
struct GirisAlani: View {
    @Binding var metin: String
    let gonder: () -> Void
    /// Belge ekleme (okuma/düzenleme için). nil ise ek düğmesi gösterilmez.
    var ekle: (() -> Void)? = nil
    /// Şu an yanıt üretiliyor mu — gönder düğmesi durdur düğmesine dönüşür.
    var uretiyor: Bool = false
    /// Üretimi iptal eder (ModelServisi.durdur()).
    var durdur: () -> Void = {}

    /// Haptik tetikleyicisi: her gönderim/durdurmada artar.
    @State private var dokunusSayaci = 0

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
                .onSubmit { gonderTetikle() }

            gonderDugmesi
        }
        .sensoryFeedback(.impact(weight: .light), trigger: dokunusSayaci)
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
    // Üretim sürerken aynı daire DURDUR düğmesine döner; uzun yanıtta tek çıkış
    // yolu uygulamayı kapatmak olmasın diye.
    private var gonderDugmesi: some View {
        Button {
            dokunusSayaci += 1
            if uretiyor {
                durdur()
            } else {
                guard !bosMu else { return }
                gonder()
            }
        } label: {
            Image(systemName: uretiyor ? "stop.fill" : "arrow.up")
                .font(.system(size: uretiyor ? 13 : 15, weight: .medium))
                // Simge, dolgunun (murekkep) zıttı: zemin rengi. Karanlık modda murekkep
                // açık olduğundan beyaz simge görünmez oluyordu; zemin = koyu, kontrast korunur.
                .foregroundStyle(Renk.zemin)
                .frame(width: 32, height: 32)
                .background(Renk.murekkep, in: Circle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(uretiyor ? Text("Durdur") : Text("Gönder"))
    }

    /// Klavye "return" yolu da haptikten geçsin diye tek kapı.
    private func gonderTetikle() {
        guard !uretiyor, !bosMu else { return }
        dokunusSayaci += 1
        gonder()
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
                GirisAlani(metin: $dolu, gonder: {}, uretiyor: true)
            }
            .padding(.vertical, Olcek.s4)
        }
    }
    return Onizleme()
}
