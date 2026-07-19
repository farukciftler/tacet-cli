//
//  NobetDetayi.swift
//  ketum
//
//  Bir nöbetin detayı ve çalışma günlüğü. Günlük dürüsttür: atlanan ya da
//  başarısız çalışmalar da silinmez, olduğu gibi yazılır. Durum renkle değil
//  sözle ve işaretle anlatılır. Silme, panonun işidir: burada yalnızca istenir.
//

import SwiftUI
import SwiftData

struct NobetDetayi: View {
    let nobet: Nobet
    let servis: NobetServisi
    /// Silme isteği panoya devredilir; detay silinmiş modeli okumadan kapanır.
    let silmeIstegi: () -> Void

    @Environment(\.modelContext) private var kayit
    @State private var silmeSorusu = false
    /// Yazma hatasının kullanıcıya görünen karşılığı.
    @State private var uyariMetni: String?

    // En yeni üstte, makul bir sayıyla sınırlı.
    private var kayitlar: [NobetKaydi] {
        Array(nobet.kayitlar.sorted { $0.tarih > $1.tarih }.prefix(20))
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Olcek.s5) {
                baslik
                gunluk
            }
            .padding(.horizontal, Olcek.s5)
            .padding(.vertical, Olcek.s4)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(Renk.zemin)
        .navigationTitle("Nöbetler")
        .navigationBarTitleDisplayMode(.inline)
        .safeAreaInset(edge: .bottom) { eylemler }
        .confirmationDialog("Bu nöbeti sil?", isPresented: $silmeSorusu, titleVisibility: .visible) {
            Button("Sil", role: .destructive) { silmeIstegi() }
            Button("Vazgeç", role: .cancel) {}
        } message: {
            Text("Çalışma günlüğü de silinir.")
        }
        .alert(Text("Bir sorun oldu"), isPresented: Binding(
            get: { uyariMetni != nil },
            set: { if !$0 { uyariMetni = nil } }
        )) {
            Button(role: .cancel) { uyariMetni = nil } label: { Text("Tamam") }
        } message: {
            if let uyariMetni { Text(uyariMetni) }
        }
    }

    // MARK: - Bölümler

    private var baslik: some View {
        VStack(alignment: .leading, spacing: Olcek.s2) {
            Text(nobet.ad)
                .font(Yazi.marka())
                .foregroundStyle(Renk.murekkep)
            Text(nobet.ozet)
                .font(Yazi.cip())
                .foregroundStyle(Renk.gri)
            if !nobet.aktif {
                Text("Duraklatıldı — yeni çalışma yapılmıyor.")
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.soluk)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(Text(verbatim: "\(nobet.ad). \(nobet.ozet)"))
        .accessibilityValue(nobet.aktif ? Text("Çalışıyor") : Text("Duraklatıldı"))
        .accessibilityAddTraits(.isHeader)
    }

    private var gunluk: some View {
        VStack(alignment: .leading, spacing: Olcek.s4) {
            Text("SON ÇALIŞMALAR")
                .font(Yazi.etiket())
                .tracking(1.2)
                .foregroundStyle(Renk.soluk)
                .accessibilityAddTraits(.isHeader)

            if kayitlar.isEmpty {
                Text("Henüz çalışma yok.")
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.gri)
            } else {
                // Girişler sol dikey çizgiyle birbirine bağlanır.
                VStack(alignment: .leading, spacing: Olcek.s4) {
                    ForEach(kayitlar) { k in
                        giris(k)
                    }
                }
                .padding(.leading, Olcek.s4)
                .overlay(alignment: .leading) {
                    Rectangle()
                        .fill(Renk.cizgi)
                        .frame(width: Olcek.hairline)
                }
                .padding(.leading, Olcek.s2)
            }
        }
    }

    // Tek günlük girişi: tarih + bağlam, altında kaydın çipleri.
    private func giris(_ k: NobetKaydi) -> some View {
        VStack(alignment: .leading, spacing: Olcek.s2) {
            HStack(spacing: Olcek.s2) {
                // Sonuç renkle değil imle ayrışır.
                // Sonuç imi görsel; karşılığı aşağıda satırın etiketinde söze dökülür.
                if k.basarili {
                    OnayImi()
                        .accessibilityHidden(true)
                } else {
                    Image(systemName: "exclamationmark.triangle")
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.murekkep)
                        .accessibilityHidden(true)
                }
                Text(NobetBicim.tarihSaat(k.tarih).localizedCapitalized)
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.murekkep)
                if !k.baglam.isEmpty {
                    Text("· \(k.baglam)")
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.soluk)
                }
            }

            if !k.ozet.isEmpty {
                Text(k.ozet)
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.gri)
                    .fixedSize(horizontal: false, vertical: true)
            }

            if !k.cipler.isEmpty {
                SarmalDizi(yatay: Olcek.s2, dikey: Olcek.s2) {
                    ForEach(k.cipler, id: \.self) { metin in
                        GunlukCipi(metin: metin)
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        // Günlük girişi tek parça okunur: sonuç, zaman, bağlam, özet, çipler.
        // children: .ignore, çünkü sonuç imi görsel — etiket elde kurulur.
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(verbatim: girisEtiketi(k)))
    }

    /// Günlük girişinin sesli karşılığı. Renk ve im yerine düz cümle.
    private func girisEtiketi(_ k: NobetKaydi) -> String {
        var parcalar = [
            k.basarili ? String(localized: "Başarılı") : String(localized: "Başarısız"),
            NobetBicim.tarihSaat(k.tarih).localizedCapitalized
        ]
        if !k.baglam.isEmpty { parcalar.append(k.baglam) }
        if !k.ozet.isEmpty { parcalar.append(k.ozet) }
        if !k.cipler.isEmpty { parcalar.append(k.cipler.joined(separator: ", ")) }
        return parcalar.joined(separator: ". ")
    }

    private var eylemler: some View {
        VStack(spacing: Olcek.s3) {
            HStack {
                Text(nobet.aktif ? "Nöbeti duraklat" : "Nöbeti sürdür")
                    .font(Yazi.kullanici())
                    .foregroundStyle(Renk.murekkep)
                Spacer(minLength: Olcek.s4)
                Toggle("", isOn: Binding(get: { nobet.aktif }, set: { _ in cevir() }))
                    .labelsHidden()
                    .tint(Renk.murekkep)
            }
            // labelsHidden anahtarı sessiz bırakıyordu; satır tek denetim olarak okunur.
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(Text("Nöbet"))
            .accessibilityValue(nobet.aktif ? Text("Çalışıyor") : Text("Duraklatıldı"))
            .accessibilityHint(nobet.aktif
                               ? Text("Duraklatmak için çift dokun.")
                               : Text("Sürdürmek için çift dokun."))
            .accessibilityAddTraits(.isButton)
            .accessibilityAction { cevir() }
            .padding(.vertical, Olcek.s3)
            .padding(.horizontal, Olcek.s4)
            .overlay(
                RoundedRectangle(cornerRadius: Olcek.s4, style: .continuous)
                    .stroke(Renk.cizgi, lineWidth: Olcek.hairline)
            )

            Button("Nöbeti sil") { silmeSorusu = true }
                .font(Yazi.cip())
                .foregroundStyle(Renk.gri)
                .accessibilityHint(Text("Onay sorulur. Çalışma günlüğü de silinir."))
        }
        .padding(.horizontal, Olcek.s5)
        .padding(.top, Olcek.s3)
        .padding(.bottom, Olcek.s2)
        .background(Renk.zemin)
    }

    // MARK: - Eylemler

    private func cevir() {
        nobet.aktif.toggle()
        bildirimiEsle()
        do {
            try kayit.save()
        } catch {
            // Diske yazılamadı: anahtar açık görünüp uygulama kapanınca eski haline
            // dönerdi. Geri alıp bildirimi gerçek duruma göre yeniden kuruyoruz.
            kayit.rollback()
            bildirimiEsle()
            uyariMetni = String(localized: "Nöbet durumu kaydedilemedi: \(error.localizedDescription)")
        }
    }

    /// Bildirim planını nöbetin güncel `aktif` durumuna eşitler.
    private func bildirimiEsle() {
        guard !nobet.isDeleted else { return }
        if nobet.aktif {
            servis.bildirimPlanla(nobet)
        } else {
            servis.bildirimIptal(nobet)
        }
    }
}

// Günlükteki küçük çip — sohbetteki araç çipiyle aynı dil: hairline çerçeve, gri metin.
private struct GunlukCipi: View {
    let metin: String

    var body: some View {
        Text(metin)
            .font(Yazi.cip())
            .foregroundStyle(Renk.gri)
            .padding(.horizontal, Olcek.s3)
            .padding(.vertical, Olcek.s2)
            .overlay(
                RoundedRectangle(cornerRadius: Olcek.cipKose, style: .continuous)
                    .stroke(Renk.cizgi, lineWidth: Olcek.hairline)
            )
    }
}

/// Çipleri yatay dizip satır dolunca alta saran basit yerleşim.
struct SarmalDizi: Layout {
    var yatay: CGFloat
    var dikey: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let en = proposal.width ?? .infinity
        let satirlar = diz(en: en, subviews: subviews)
        let yukseklik = satirlar.last.map { $0.y + $0.yukseklik } ?? 0
        return CGSize(width: proposal.width ?? satirlar.map(\.genislik).max() ?? 0,
                      height: yukseklik)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize,
                       subviews: Subviews, cache: inout ()) {
        for satir in diz(en: bounds.width, subviews: subviews) {
            var x = bounds.minX
            for i in satir.araligi {
                let olcu = subviews[i].sizeThatFits(.unspecified)
                subviews[i].place(at: CGPoint(x: x, y: bounds.minY + satir.y),
                                  proposal: ProposedViewSize(olcu))
                x += olcu.width + yatay
            }
        }
    }

    private struct Satir {
        var araligi: Range<Int>
        var y: CGFloat
        var genislik: CGFloat
        var yukseklik: CGFloat
    }

    private func diz(en: CGFloat, subviews: Subviews) -> [Satir] {
        var satirlar: [Satir] = []
        var bas = 0, x: CGFloat = 0, y: CGFloat = 0, satirYukseklik: CGFloat = 0

        for i in subviews.indices {
            let olcu = subviews[i].sizeThatFits(.unspecified)
            if x > 0, x + olcu.width > en {
                satirlar.append(Satir(araligi: bas..<i, y: y, genislik: x - yatay, yukseklik: satirYukseklik))
                y += satirYukseklik + dikey
                bas = i; x = 0; satirYukseklik = 0
            }
            x += olcu.width + yatay
            satirYukseklik = max(satirYukseklik, olcu.height)
        }
        if bas < subviews.count {
            satirlar.append(Satir(araligi: bas..<subviews.count, y: y,
                                  genislik: x - yatay, yukseklik: satirYukseklik))
        }
        return satirlar
    }
}
