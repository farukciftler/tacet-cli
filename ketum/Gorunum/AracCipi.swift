import SwiftUI

// Araç çipi — sistemin imzası (spec §4.4).
// sirr'in dünyaya dokunuşunu tek satırda, sakin bir çip olarak gösterir.
struct AracCipi: View {
    let iz: AracIzi
    /// Canlı turda yürütücü — yalnızca "onay bekleniyor" çipinin kararı için.
    /// Geçmiş mesajlarda nil'dir; o zaman bekleyen bir karar da yoktur ve çip
    /// normal detay sayfasını açar.
    var yurutucu: AracYurutucu? = nil

    // Detay sayfası (ham girdi/çıktı) açık mı.
    @State private var detayAcik = false
    // Dosya önizlemesi (QuickLook) açık mı.
    @State private var onizlemeAcik = false
    // İzin gerektiren çipte açılan "ne yapmalıyım" sayfası.
    @State private var izinAcik = false
    // Paylaşım onayı sayfası açık mı (mcp §3.3).
    @State private var onayAcik = false

    /// Bu çip, şu an kullanıcı kararı bekleyen isteğin çipi mi. Yürütücüde
    /// bekleyen istek yoksa (ya da başka bir çipe aitse) onay sayfası açılmaz —
    /// karar verilmiş eski bir çip ikinci kez sormaz.
    private var bekleyenOnay: AracYurutucu.OnayIstegi? {
        guard iz.durum == .onayBekleniyor,
              let istek = yurutucu?.bekleyenOnay,
              istek.izID == iz.id else { return nil }
        return istek
    }

    @Environment(\.accessibilityReduceMotion) private var hareketiAzalt

    // Bu çip önizlenebilir bir dosya ürettiyse URL'i.
    private var dosyaURL: URL? {
        guard let yol = iz.dosyaYolu, FileManager.default.fileExists(atPath: yol) else { return nil }
        return URL(fileURLWithPath: yol)
    }

    var body: some View {
        Button {
            // İzin gerekiyorsa çip çıkmaz sokak olmamalı: ham girdi/çıktı yerine
            // doğrudan "iOS Ayarlar'a git" yolunu sunan sayfa açılır.
            if iz.durum == .izinGerekli {
                izinAcik = true
            } else if bekleyenOnay != nil {
                // Karar bekleyen çip: dokunma doğrudan onay sayfasını açar.
                onayAcik = true
            } else if dosyaURL != nil {
                onizlemeAcik = true
            } else {
                detayAcik = true
            }
        } label: {
            cipGovdesi
        }
        .buttonStyle(.plain)
        .accessibilityLabel(iz.seslendirme)
        .accessibilityHint(ipucu)
        .sheet(isPresented: $izinAcik) {
            IzinYonlendirme(baslik: iz.metin)
        }
        .sheet(isPresented: $detayAcik) {
            AracCipiDetay(iz: iz)
        }
        .sheet(isPresented: $onizlemeAcik) {
            if let url = dosyaURL {
                BelgeOnizlemeSheet(url: url)
            }
        }
        .sheet(isPresented: $onayAcik) {
            if let istek = bekleyenOnay {
                OnaySayfasi(kaynak: istek.kaynak,
                            aracAdi: istek.aracAdi,
                            icerik: istek.icerik) { kabul in
                    yurutucu?.onayKarariVer(kabul)
                }
            }
        }
    }

    // Dokunmanın ne yapacağını önceden söyleyen ipucu.
    private var ipucu: Text {
        if iz.durum == .izinGerekli {
            return Text("İzni açmak için dokun")
        }
        if bekleyenOnay != nil {
            return Text("Gönderileni görmek için dokun")
        }
        return dosyaURL != nil ? Text("Dosyayı önizlemek için dokun") : Text("Ayrıntı için dokun")
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
        case .onayBekleniyor:
            // Bekleme dürüstçe görünür: spinner değil, kullanıcıyı bekleyen bir el.
            simge("hand.raised")
        case .gonderilmedi:
            // Ret bir hata değil kısıttır — uyarı imi kullanılmaz, dramatize yok.
            simge("nosign")
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
        // Onay bekleyen ve gönderilmeyen çip gridir: üstü çizili değil, kırmızı
        // değil. Sessizce durur, kullanıcının kararını taşır.
        case .onayBekleniyor, .gonderilmedi:
            return Renk.gri
        case .yazildi, .basarisiz:
            return Renk.murekkep
        }
    }
}

// İzin gerekiyor: tek yol iOS Ayarlar. Sayfa kısa, tek düğmeli ve dürüst;
// izni uygulama kendi kendine açamaz, kararı kullanıcı verir.
private struct IzinYonlendirme: View {
    let baslik: String
    @Environment(\.dismiss) private var kapat

    var body: some View {
        VStack(alignment: .leading, spacing: Olcek.s4) {
            HStack {
                Text("İzin gerekiyor")
                    .font(Yazi.marka())
                    .foregroundStyle(Renk.murekkep)
                Spacer()
                Button { kapat() } label: { Text("Kapat") }
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.gri)
                    .buttonStyle(.plain)
            }

            Text(baslik)
                .font(Yazi.kullanici())
                .foregroundStyle(Renk.murekkep)
                .fixedSize(horizontal: false, vertical: true)

            Text("Bu adımı sürdürmek için iOS Ayarlar'dan sirr'e izin vermen gerekiyor. Veri yine cihazdan çıkmaz.")
                .font(Yazi.cip())
                .foregroundStyle(Renk.gri)
                .fixedSize(horizontal: false, vertical: true)

            Button {
                if let url = URL(string: UIApplication.openSettingsURLString) {
                    UIApplication.shared.open(url)
                }
            } label: {
                HStack(spacing: Olcek.s2) {
                    Text("iOS Ayarlar'ı aç")
                        .font(Yazi.kullanici())
                        .foregroundStyle(Renk.murekkep)
                    Image(systemName: "arrow.up.forward")
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.soluk)
                        .accessibilityHidden(true)
                }
                .padding(.horizontal, Olcek.s4)
                .padding(.vertical, Olcek.s3)
                .overlay(
                    RoundedRectangle(cornerRadius: Olcek.cipKose, style: .continuous)
                        .stroke(Renk.cizgi, lineWidth: Olcek.hairline)
                )
                .contentShape(RoundedRectangle(cornerRadius: Olcek.cipKose, style: .continuous))
            }
            .buttonStyle(.plain)
            .accessibilityElement(children: .combine)

            Spacer()
        }
        .padding(Olcek.s5)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Renk.zemin)
        .presentationDetents([.medium])
    }
}

// Detay: ham girdi ve çıktı, saydamlığın ikinci katmanı.
// Dosya-içi değil çünkü Seyir zaman çizgisi de AYNI sheet'i açar — araç adımının
// detayı için ikinci bir yüzey yazılmaz (seyir-spec §2.4).
struct AracCipiDetay: View {
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

        AracCipi(iz: AracIzi(
            id: UUID(), ikon: "hand.raised", metin: "ev sunucusu · onay bekleniyor",
            durum: .onayBekleniyor, hamGirdi: "sorgu: yarınki nöbet", hamCikti: nil))

        AracCipi(iz: AracIzi(
            id: UUID(), ikon: "hand.raised", metin: "ev sunucusu · gönderilmedi",
            durum: .gonderilmedi, hamGirdi: "sorgu: yarınki nöbet", hamCikti: nil))
    }
    .padding(Olcek.s5)
    .frame(maxWidth: .infinity, alignment: .leading)
    .background(Renk.zemin)
}
