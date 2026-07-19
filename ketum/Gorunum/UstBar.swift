import SwiftUI

// Üst bar: solda geçmiş + işaret + serif "sirr", sağda yeni sohbet.
// Marka kimliği: durum renkle (yeşil nokta) değil, yalnızca işaret ve sözle anlatılır.
// Cihaz göstergesi yoktur; gizlilik açıklaması markaya dokununca açılır.
struct UstBar: View {
    let durum: ModelServisi.Durum
    /// Durumun tam cümlesi — ModelServisi.engelMesaji. Üst bar ile sohbete düşen
    /// mesaj TEK kaynaktan gelsin diye dışarıdan verilir; UstBar kendi dizgisini
    /// gömerse ("birazdan" ↔ "birkaç dakika") iki metin çelişir.
    var aciklama: String? = nil
    var gecmisAc: (() -> Void)? = nil
    var yeniSohbet: (() -> Void)? = nil
    @State private var sheetAcik = false

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: Olcek.s3) {
                if let gecmisAc {
                    Button(action: gecmisAc) {
                        Image(systemName: "list.bullet")
                            .font(.system(size: 17, weight: .regular))
                            .foregroundStyle(Renk.gri)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(Text("Sohbet geçmişi"))
                }

                Button {
                    sheetAcik = true
                } label: {
                    HStack(spacing: Olcek.s2) {
                        SirrMark(boyut: 20)
                        Text(verbatim: "sirr")
                            .font(Yazi.marka())
                            .foregroundStyle(Renk.murekkep)
                        // Dokunulabilir olduğunun tek ipucu: rozet değil, sessiz bir im.
                        Image(systemName: "chevron.down")
                            .font(.system(size: 10, weight: .regular))
                            .foregroundStyle(Renk.soluk)
                    }
                }
                .buttonStyle(.plain)
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(Text("sirr hakkında"))
                .accessibilityHint(Text("Cihazda çalışma açıklamasını açar"))

                Spacer()

                if let yeniSohbet {
                    Button(action: yeniSohbet) {
                        Image(systemName: "square.and.pencil")
                            .font(.system(size: 17, weight: .regular))
                            .foregroundStyle(Renk.gri)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(Text("Yeni sohbet"))
                }
            }
            .padding(.horizontal, Olcek.s5)
            .padding(.vertical, Olcek.s3)

            durumSatiri

            Rectangle()
                .fill(Renk.cizgi)
                .frame(height: Olcek.hairline)
        }
        .background(Renk.zemin)
        .sheet(isPresented: $sheetAcik) {
            GizlilikAcikamasi()
        }
    }

    // Model hazır değilse kullanıcı bunu yazmadan ÖNCE öğrenmeli.
    // Marka: renkli nokta / rozet yok — durum yalnızca söz ve sessiz bir imle anlatılır.
    // Hazır durumda hiçbir şey gösterilmez.
    @ViewBuilder
    private var durumSatiri: some View {
        switch durum {
        case .hazir:
            EmptyView()
        case .hazirlaniyor:
            durumMetni("ellipsis", Text(verbatim: aciklama ?? Yerel.modelHazirlaniyor))
        case .kullanilamaz:
            durumMetni("exclamationmark.circle",
                       Text(verbatim: aciklama ?? Yerel.cihazUygunDegil))
        }
    }

    private func durumMetni(_ im: String, _ metin: Text) -> some View {
        HStack(spacing: Olcek.s2) {
            Image(systemName: im)
                .font(Yazi.cip())
                .foregroundStyle(Renk.soluk)
                .accessibilityHidden(true)
            metin
                .font(Yazi.cip())
                .foregroundStyle(Renk.gri)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, Olcek.s5)
        .padding(.bottom, Olcek.s3)
        .accessibilityElement(children: .combine)
    }
}

// Cihazda çalışmayı anlatan sade açıklama (markaya dokununca).
private struct GizlilikAcikamasi: View {
    @Environment(\.dismiss) private var kapat

    var body: some View {
        VStack(alignment: .leading, spacing: Olcek.s4) {
            HStack {
                HStack(spacing: Olcek.s2) {
                    SirrMark(boyut: 20)
                    Text(verbatim: "sirr").font(Yazi.marka()).foregroundStyle(Renk.murekkep)
                }
                Spacer()
                Button { kapat() } label: { Text("Kapat") }
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.gri)
                    .buttonStyle(.plain)
            }

            Text("sirr tamamen bu cihazda çalışır. Sorduğun, takvimin, notların cihazdan çıkmaz. İnternete çıkmaz.")
                .font(Yazi.ketum())
                .foregroundStyle(Renk.gri)
                .fixedSize(horizontal: false, vertical: true)

            Spacer()
        }
        .padding(Olcek.s5)
        .background(Renk.zemin)
        .presentationDetents([.medium])
    }
}

#Preview {
    VStack(spacing: 0) {
        UstBar(durum: .hazir, gecmisAc: {}, yeniSohbet: {})
        UstBar(durum: .hazirlaniyor, aciklama: Yerel.modelHazirlaniyor, gecmisAc: {}, yeniSohbet: {})
        UstBar(durum: .kullanilamaz(""), aciklama: Yerel.appleIntelligenceKapali,
               gecmisAc: {}, yeniSohbet: {})
        Spacer()
    }
}
