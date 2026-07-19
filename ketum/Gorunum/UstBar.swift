import SwiftUI

// Üst bar: solda geçmiş + işaret + serif "sirr", sağda yeni sohbet.
// Marka kimliği: durum renkle (yeşil nokta) değil, yalnızca işaret ve sözle anlatılır.
// Cihaz göstergesi yoktur; gizlilik açıklaması markaya dokununca açılır.
struct UstBar: View {
    let durum: ModelServisi.Durum
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
                    }
                }
                .buttonStyle(.plain)
                .accessibilityLabel(Text("sirr"))

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

            Rectangle()
                .fill(Renk.cizgi)
                .frame(height: Olcek.hairline)
        }
        .background(Renk.zemin)
        .sheet(isPresented: $sheetAcik) {
            GizlilikAcikamasi()
        }
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
        Spacer()
    }
}
