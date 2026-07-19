//
//  SohbetTablo.swift
//  ketum
//
//  Sohbet içinde tablo gösterimi (Gemini/Claude gibi). sirr yanıtındaki markdown
//  tablo, metnin arasında sade bir tablo olarak render edilir; altında "Excel indir"
//  düğmesiyle aynı tablo .xlsx olarak üretilip önizlenir/paylaşılır.
//

import SwiftUI

struct SohbetTablo: View {
    let tablo: Tablo
    /// Excel indirme isteği — üst görünüm dosyayı üretip önizler.
    var indir: (Tablo) -> Void = { _ in }

    var body: some View {
        VStack(alignment: .leading, spacing: Olcek.s2) {
            Grid(alignment: .leading, horizontalSpacing: Olcek.hairline, verticalSpacing: Olcek.hairline) {
                // Başlık satırı.
                GridRow {
                    ForEach(Array(tablo.basliklar.enumerated()), id: \.offset) { _, b in
                        hucre(b, baslikMi: true)
                    }
                }
                // Veri satırları.
                ForEach(Array(tablo.satirlar.enumerated()), id: \.offset) { _, satir in
                    GridRow {
                        ForEach(Array(tablo.basliklar.enumerated()), id: \.offset) { i, _ in
                            hucre(i < satir.hucreler.count ? satir.hucreler[i] : "", baslikMi: false)
                        }
                    }
                }
            }
            .background(Renk.cizgi)   // hücreler arası hairline ızgara
            .clipShape(RoundedRectangle(cornerRadius: 10))
            .overlay(
                RoundedRectangle(cornerRadius: 10)
                    .stroke(Renk.cizgi, lineWidth: Olcek.hairline)
            )

            // Excel indirme düğmesi.
            Button {
                indir(tablo)
            } label: {
                HStack(spacing: Olcek.s1) {
                    Image(systemName: "arrow.down.circle")
                    Text("Excel indir")
                }
                .font(Yazi.cip())
                .foregroundStyle(Renk.gri)
                .padding(.horizontal, Olcek.s3)
                .padding(.vertical, Olcek.s2)
                .overlay(Capsule().stroke(Renk.cizgi, lineWidth: Olcek.hairline))
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Tabloyu Excel olarak indir")
        }
    }

    private func hucre(_ metin: String, baslikMi: Bool) -> some View {
        Text(metin)
            .font(baslikMi ? Yazi.cip().weight(.medium) : Yazi.cip())
            .foregroundStyle(baslikMi ? Renk.murekkep : Renk.gri)
            .padding(.horizontal, Olcek.s3)
            .padding(.vertical, Olcek.s2)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(baslikMi ? Renk.dolgu : Renk.zemin)
    }
}
