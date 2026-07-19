//
//  SeyirSeridi.swift
//  ketum
//
//  Canlı seyir şeridi (seyir-spec §3.1). Yanıt üretilirken yanıt balonu
//  hizasında, sola dayalı TEK satır: spinner + o anki adım metni; üstünde
//  soluk bir "n. adım" sayacı.
//
//  Dramatize yoktur: satır metni uydurma bir durum fiili değil, kodda gerçekten
//  olan deterministik olaydır (§2.1). Şerit modelden habersizdir.
//

import SwiftUI

struct SeyirSeridi: View {
    /// Turun şu ana kadarki adımları (`SeyirKaydedici.adimlar`).
    let adimlar: [SeyirAdimi]
    /// Araç adımlarının metnini okumak için canlı izler.
    var izler: [AracIzi] = []

    /// Dokununca açılan zaman çizgisi — süren işi bekletmez, yerinde açılır.
    @State private var cizgiAcik = false
    @Environment(\.accessibilityReduceMotion) private var hareketiAzalt

    private var sonAdim: SeyirAdimi? { adimlar.last }

    /// Canlı satırın metni. Araç adımında iz metni; iz henüz gelmediyse adım
    /// boş kalır ve satır yalnızca spinner gösterir.
    private var canliMetin: String {
        guard let sonAdim else { return "" }
        return SeyirMetni.satir(sonAdim, izler: izler)
    }

    var body: some View {
        if adimlar.isEmpty {
            EmptyView()
        } else {
            VStack(alignment: .leading, spacing: Olcek.s1) {
                if adimlar.count > 1 {
                    Text(String(localized: "\(adimlar.count). adım"))
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.soluk)
                        .accessibilityHidden(true)
                }

                Button {
                    if hareketiAzalt {
                        cizgiAcik.toggle()
                    } else {
                        withAnimation(.easeInOut(duration: 0.18)) { cizgiAcik.toggle() }
                    }
                } label: {
                    HStack(spacing: Olcek.s2) {
                        ProgressView()
                            .controlSize(.small)
                            .frame(width: 13, height: 13)
                            .accessibilityHidden(true)
                        Text(canliMetin)
                            .font(Yazi.cip())
                            .foregroundStyle(Renk.gri)
                            .lineLimit(1)
                            .truncationMode(.tail)
                            // Satır değişimi kaymayla değil, sakin geçişle olur.
                            .contentTransition(.opacity)
                        Spacer(minLength: 0)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)

                if cizgiAcik {
                    // Süren işin geçmiş adımları — bekletmeden görülebilir.
                    SeyirCizgisi(adimlar: adimlar, izler: izler, baslangictaAcik: true)
                        .transition(hareketiAzalt
                                    ? .opacity
                                    : .opacity.combined(with: .move(edge: .top)))
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .animation(hareketiAzalt ? nil : .easeInOut(duration: 0.18), value: canliMetin)
            // Adım değişimi VoiceOver'a duyurulur.
            .accessibilityElement(children: .contain)
            .accessibilityLabel(Text(canliMetin))
            .accessibilityValue(Text(String(localized: "\(adimlar.count). adım")))
            .accessibilityHint(Text("Adımları görmek için dokun"))
            .accessibilityAddTraits(.updatesFrequently)
        }
    }
}

#Preview("Canlı şerit") {
    let izID = UUID()
    let adimlar: [SeyirAdimi] = [
        SeyirAdimi(tur: .yonlendirme, metin: "yönlendirildi · takvim profili",
                   baslangic: Date(), bitis: Date().addingTimeInterval(0.2)),
        SeyirAdimi(tur: .arac, aracIziID: izID)
    ]
    let izler = [AracIzi(id: izID, ikon: "calendar",
                         metin: "takvim okunuyor · bugün", durum: .calisiyor)]

    return VStack(alignment: .leading, spacing: Olcek.cipYanitAraligi) {
        SeyirSeridi(adimlar: adimlar, izler: izler)
    }
    .padding(Olcek.s5)
    .frame(maxWidth: .infinity, alignment: .leading)
    .background(Renk.zemin)
}
