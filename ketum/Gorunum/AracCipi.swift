import SwiftUI

// Araç çipi — sistemin imzası (spec §4.4).
// sirr'in dünyaya dokunuşunu tek satırda, sakin bir çip olarak gösterir.
struct AracCipi: View {
    let iz: AracIzi

    // Detay sayfası (ham girdi/çıktı) açık mı.
    @State private var detayAcik = false
    // Dosya önizlemesi (QuickLook) açık mı.
    @State private var onizlemeAcik = false

    @Environment(\.accessibilityReduceMotion) private var hareketiAzalt

    // Bu çip önizlenebilir bir dosya ürettiyse URL'i.
    private var dosyaURL: URL? {
        guard let yol = iz.dosyaYolu, FileManager.default.fileExists(atPath: yol) else { return nil }
        return URL(fileURLWithPath: yol)
    }

    var body: some View {
        Button {
            if dosyaURL != nil { onizlemeAcik = true } else { detayAcik = true }
        } label: {
            cipGovdesi
        }
        .buttonStyle(.plain)
        .accessibilityLabel(iz.seslendirme)
        .accessibilityHint(dosyaURL != nil ? "Dosyayı önizlemek için dokun" : "Ayrıntı için dokun")
        .sheet(isPresented: $detayAcik) {
            AracCipiDetay(iz: iz)
        }
        .sheet(isPresented: $onizlemeAcik) {
            if let url = dosyaURL {
                BelgeOnizlemeSheet(url: url)
            }
        }
    }

    // Çipin kendisi: pill çerçeve, ikon + metin, sola hizalı.
    private var cipGovdesi: some View {
        HStack(spacing: Olcek.s2) {
            onEleman
            Text(iz.metin)
                // Yazma eylemi renkle değil AĞIRLIKLA ayrışır (marka: renk durum anlatmaz).
                .font(iz.durum == .yazildi ? Yazi.cip().weight(.medium) : Yazi.cip())
                .foregroundStyle(renk)
            // Önizlenebilir dosya çipinde küçük bir işaret.
            if dosyaURL != nil {
                Image(systemName: "eye")
                    .font(.system(size: 11))
                    .foregroundStyle(renk)
            }
        }
        .padding(.horizontal, Olcek.s3)
        .padding(.vertical, Olcek.s2)
        .overlay(
            RoundedRectangle(cornerRadius: Olcek.cipKose)
                .stroke(Renk.cizgi, lineWidth: Olcek.hairline)
        )
        .contentShape(RoundedRectangle(cornerRadius: Olcek.cipKose))
    }

    // Duruma göre öndeki eleman: spinner, simge ya da onay.
    @ViewBuilder
    private var onEleman: some View {
        switch iz.durum {
        case .calisiyor:
            ProgressView()
                .controlSize(.small)
                .frame(width: 13, height: 13)
        case .okundu, .izinGerekli:
            simge(iz.ikon)
        case .yazildi:
            simge("checkmark")
        case .basarisiz:
            simge("exclamationmark.triangle")
        }
    }

    // İkon: outline SF Symbol, üstteki metinle aynı renk, ~13pt.
    private func simge(_ ad: String) -> some View {
        Image(systemName: ad)
            .font(.system(size: 13))
            .foregroundStyle(renk)
    }

    // Metnin (ve ikonun) rengi. Marka: renk durum anlatmaz — yazma mürekkep + onay
    // imiyle, hata mürekkep + uyarı imiyle ayrışır. Yeşil/kırmızı kullanılmaz.
    private var renk: Color {
        switch iz.durum {
        case .calisiyor, .okundu, .izinGerekli:
            return Renk.gri
        case .yazildi, .basarisiz:
            return Renk.murekkep
        }
    }
}

// Detay: ham girdi ve çıktı, saydamlığın ikinci katmanı.
private struct AracCipiDetay: View {
    let iz: AracIzi
    @Environment(\.dismiss) private var kapat

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: Olcek.s4) {
                    bolum("Girdi", iz.hamGirdi)
                    bolum("Çıktı", iz.hamCikti)
                }
                .padding(.horizontal, Olcek.s5)
                .padding(.vertical, Olcek.s4)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .background(Renk.zemin)
            .navigationTitle(iz.metin)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Kapat") { kapat() }
                }
            }
        }
    }

    // Tek bir ham blok: başlık + monospace içerik.
    @ViewBuilder
    private func bolum(_ baslik: String, _ icerik: String?) -> some View {
        VStack(alignment: .leading, spacing: Olcek.s2) {
            Text(baslik)
                .font(Yazi.etiket())
                .foregroundStyle(Renk.soluk)
            Text(icerik ?? "—")
                .font(.system(.footnote, design: .monospaced))
                .foregroundStyle(Renk.murekkep)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(Olcek.s3)
                .background(Renk.dolgu)
                .clipShape(RoundedRectangle(cornerRadius: Olcek.cipKose))
        }
    }
}

#Preview {
    VStack(alignment: .leading, spacing: Olcek.cipYanitAraligi) {
        AracCipi(iz: AracIzi(
            id: UUID(), ikon: "magnifyingglass", metin: "takvim okunuyor",
            durum: .calisiyor, hamGirdi: "bugün", hamCikti: nil))

        AracCipi(iz: AracIzi(
            id: UUID(), ikon: "calendar", metin: "3 etkinlik okundu",
            durum: .okundu, hamGirdi: "range: bugün",
            hamCikti: "09:00 toplantı\n13:00 öğle\n18:00 spor"))

        AracCipi(iz: AracIzi(
            id: UUID(), ikon: "calendar.badge.plus", metin: "etkinlik eklendi",
            durum: .yazildi, hamGirdi: "başlık: Diş hekimi\ntarih: yarın 10:00",
            hamCikti: "id: E-4821"))

        AracCipi(iz: AracIzi(
            id: UUID(), ikon: "lock", metin: "konum için izin ver",
            durum: .izinGerekli, hamGirdi: nil, hamCikti: nil))

        AracCipi(iz: AracIzi(
            id: UUID(), ikon: "exclamationmark.triangle", metin: "ağ yok",
            durum: .basarisiz("bağlantı yok"), hamGirdi: "istek", hamCikti: nil))
    }
    .padding(Olcek.s5)
    .frame(maxWidth: .infinity, alignment: .leading)
    .background(Renk.zemin)
}
