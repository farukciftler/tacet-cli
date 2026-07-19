//
//  DosyaKarti.swift
//  ketum
//
//  Üretilen dosyanın sunumu — seyir-spec §9.2.
//
//  Kart bir balon DEĞİL, çip ailesindendir: hairline çerçeve, Olcek.cipKose
//  köşe, Renk.zemin. Yanıt gövdesinin altında, balonla aynı hizada durur.
//  Tek doğruluk kaynağı yine AracIzi'dir; kart yeni model alanı istemez,
//  yalnızca iz.dosyaYolu'ndan beslenir.
//
//  Dosya zaten cihazdadır — kart indirmeyi ima eden hiçbir söz kullanmaz.
//

import SwiftUI

struct DosyaKarti: View {
    /// Dosyanın diskteki yolu (`AracIzi.dosyaYolu`).
    let yol: String

    /// İz üzerinden kurulum — KetumYaniti'nin kullanacağı yol.
    /// Dosya yolu olmayan izler kart olarak çizilemez; çağıran taraf ayıklar.
    init?(iz: AracIzi) {
        guard let yol = iz.dosyaYolu, !yol.isEmpty else { return nil }
        self.yol = yol
    }

    /// Doğrudan yol ile kurulum (önizleme ve testler için).
    init(yol: String) {
        self.yol = yol
    }

    @State private var onizlemeAcik = false

    private var url: URL { URL(fileURLWithPath: yol) }
    private var ad: String { url.lastPathComponent }
    private var uzanti: String { url.pathExtension }

    /// Dosya hâlâ diskte mi. Silinmişse kart kalır, eylemler düşer (§9.2).
    private var cihazdaMi: Bool { FileManager.default.fileExists(atPath: yol) }

    /// "Hesap tablosu · XLSX". UTType çözemediğinde etiket zaten uzantının
    /// kendisidir; o zaman iki kez yazılmaz.
    private var turSatiri: String {
        let etiket = DosyaIkonu.turEtiketi(uzanti: uzanti)
        let buyuk = uzanti.uppercased()
        if etiket.isEmpty { return buyuk }
        if etiket == buyuk { return buyuk }
        return "\(etiket) · \(buyuk)"
    }

    var body: some View {
        HStack(spacing: Olcek.s3) {
            DosyaIkonu.ikon(uzanti: uzanti)
                .resizable()
                .scaledToFit()
                .frame(width: 24, height: 24)
                .foregroundStyle(cihazdaMi ? Renk.murekkep : Renk.soluk)
                .accessibilityHidden(true)

            VStack(alignment: .leading, spacing: 2) {
                Text(ad)
                    .font(Yazi.kullanici())
                    .foregroundStyle(cihazdaMi ? Renk.murekkep : Renk.gri)
                    .lineLimit(1)
                    .truncationMode(.middle)

                if cihazdaMi {
                    Text(turSatiri)
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.soluk)
                        .lineLimit(1)
                } else {
                    // Sessiz kaybolma yok: durum yazıyla söylenir.
                    Text("dosya artık cihazda değil")
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.soluk)
                        .lineLimit(1)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityElement(children: .combine)
            .accessibilityLabel(seslendirme)

            if cihazdaMi {
                eylemler
            }
        }
        .padding(.horizontal, Olcek.s3)
        .padding(.vertical, Olcek.s3)
        .background(Renk.zemin)
        .overlay(
            RoundedRectangle(cornerRadius: Olcek.cipKose, style: .continuous)
                .stroke(Renk.cizgi, lineWidth: Olcek.hairline)
        )
        .clipShape(RoundedRectangle(cornerRadius: Olcek.cipKose, style: .continuous))
        .frame(maxWidth: .infinity, alignment: .leading)
        .sheet(isPresented: $onizlemeAcik) {
            BelgeOnizlemeSheet(url: url)
        }
    }

    // Sağdaki iki eylem: aç (mevcut QuickLook sayfası) ve paylaş.
    private var eylemler: some View {
        HStack(spacing: Olcek.s2) {
            Button {
                onizlemeAcik = true
            } label: {
                Text("Aç")
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.murekkep)
                    .padding(.horizontal, Olcek.s3)
                    .padding(.vertical, Olcek.s1 + 2)
                    .overlay(
                        RoundedRectangle(cornerRadius: Olcek.cipKose, style: .continuous)
                            .stroke(Renk.cizgi, lineWidth: Olcek.hairline)
                    )
                    .contentShape(RoundedRectangle(cornerRadius: Olcek.cipKose,
                                                   style: .continuous))
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Aç")
            .accessibilityHint("Dosyayı önizler")

            ShareLink(item: url) {
                Image(systemName: "square.and.arrow.up")
                    .font(Yazi.ikon())
                    .foregroundStyle(Renk.gri)
                    .frame(width: 28, height: 28)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Paylaş")
        }
    }

    // VoiceOver: tek cümle. Dosyanın nerede olduğu hakkında abartı yok.
    private var seslendirme: Text {
        cihazdaMi
            ? Text("Dosya: \(ad), \(turSatiri)")
            : Text("Dosya: \(ad), dosya artık cihazda değil")
    }
}

#Preview {
    VStack(alignment: .leading, spacing: Olcek.cipYanitAraligi) {
        DosyaKarti(yol: "/tmp/Yıldızlar keşif soruları.xlsx")
        DosyaKarti(yol: "/tmp/ozet.pdf")
        DosyaKarti(yol: "/tmp/silinmis-cok-uzun-bir-dosya-adi-ornegi.docx")
    }
    .padding(Olcek.s5)
    .background(Renk.zemin)
}
