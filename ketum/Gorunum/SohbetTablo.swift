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
                    ForEach(Array(tablo.basliklar.enumerated()), id: \.offset) { i, b in
                        hucre(b, sutun: b, satirNo: nil, sutunNo: i, baslikMi: true)
                    }
                }
                // Veri satırları.
                ForEach(Array(tablo.satirlar.enumerated()), id: \.offset) { s, satir in
                    GridRow {
                        ForEach(Array(tablo.basliklar.enumerated()), id: \.offset) { i, b in
                            hucre(i < satir.hucreler.count ? satir.hucreler[i] : "",
                                  sutun: b, satirNo: s + 1, sutunNo: i, baslikMi: false)
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
            // Ekran okuyucuda tablonun sınırı ve ölçüsü kaybolmasın.
            .accessibilityElement(children: .contain)
            .accessibilityLabel(Text("Tablo, \(tablo.basliklar.count) sütun, \(tablo.satirlar.count) satır"))

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

    /// Hücre görsel olarak yalnızca değerdir; ekran okuyucuda ise sütun başlığı
    /// ve konum bağlamıyla birlikte okunur — yoksa tablo yapısı tamamen kaybolur.
    private func hucre(_ metin: String,
                       sutun: String,
                       satirNo: Int?,
                       sutunNo: Int,
                       baslikMi: Bool) -> some View {
        Text(metin)
            .font(baslikMi ? Yazi.cip().weight(.medium) : Yazi.cip())
            .foregroundStyle(baslikMi ? Renk.murekkep : Renk.gri)
            .padding(.horizontal, Olcek.s3)
            .padding(.vertical, Olcek.s2)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(baslikMi ? Renk.dolgu : Renk.zemin)
            .accessibilityLabel(hucreEtiketi(metin, sutun: sutun, satirNo: satirNo,
                                             sutunNo: sutunNo, baslikMi: baslikMi))
    }

    private func hucreEtiketi(_ metin: String,
                              sutun: String,
                              satirNo: Int?,
                              sutunNo: Int,
                              baslikMi: Bool) -> Text {
        let sutunAdi = sutun.isEmpty ? String(localized: "\(sutunNo + 1). sütun") : sutun
        guard let satirNo else {
            return Text("Sütun başlığı: \(sutunAdi)")
        }
        let deger = metin.isEmpty ? String(localized: "boş") : metin
        return Text("\(satirNo). satır, \(sutunAdi): \(deger)")
    }
}
