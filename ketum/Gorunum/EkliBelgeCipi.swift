import SwiftUI

// Eklenmiş belge çipi — giriş alanının hemen üstünde gösterilir.
// Sakin bir pill: solda biçim ikonu, ortada dosya adı, sağda kaldır düğmesi.
struct EkliBelgeCipi: View {
    let belge: EkliBelge
    let kaldir: () -> Void

    var body: some View {
        HStack(spacing: Olcek.s2) {
            Image(systemName: belge.bicim.ikon)
                .font(.system(size: 13))
                .foregroundStyle(Renk.gri)

            Text(belge.ad)
                .font(Yazi.cip())
                .foregroundStyle(Renk.murekkep)
                .lineLimit(1)

            Button(action: kaldir) {
                Image(systemName: "xmark.circle.fill")
                    .font(.system(size: 15))
                    .foregroundStyle(Renk.gri)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Belgeyi kaldır")
        }
        .padding(.horizontal, Olcek.s3)
        .padding(.vertical, Olcek.s2)
        .overlay(
            RoundedRectangle(cornerRadius: Olcek.cipKose)
                .stroke(Renk.cizgi, lineWidth: Olcek.hairline)
        )
        .contentShape(RoundedRectangle(cornerRadius: Olcek.cipKose))
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityLabel("Eklenen belge: \(belge.ad)")
    }
}

#Preview {
    EkliBelgeCipi(
        belge: EkliBelge(
            id: UUID(),
            url: URL(fileURLWithPath: "/tmp/ozet.pdf"),
            ad: "ozet.pdf",
            bicim: .pdf
        ),
        kaldir: {}
    )
    .padding(Olcek.s5)
    .frame(maxWidth: .infinity, alignment: .leading)
    .background(Renk.zemin)
}
