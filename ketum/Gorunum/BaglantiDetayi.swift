//
//  BaglantiDetayi.swift
//  ketum
//
//  Tek bağlantının detayı — mcp-baglanti-spec §3.5.
//
//  Araç listesi (desteklenmeyenler etiketiyle), cihaz verisi ayarı,
//  "Bağlantıyı dene", sil. Silme onaylıdır ve sonucunu söyler. Silme işini
//  pano yapar: burada yalnızca istenir, böylece silinmiş model okunmaz.
//

import SwiftUI
import SwiftData

struct BaglantiDetayi: View {
    let baglanti: Baglanti
    let servis: any BaglantiDeneyici
    let silmeIstegi: () -> Void

    @Environment(\.modelContext) private var kayit

    @State private var deneniyor = false
    @State private var denemeNotu: String?
    @State private var silmeSorusu = false
    @State private var uyariMetni: String?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Olcek.s5) {
                baslik
                cihazVerisiBolumu
                aracBolumu
                denemeBolumu
            }
            .padding(.horizontal, Olcek.s5)
            .padding(.vertical, Olcek.s4)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(Renk.zemin)
        .navigationTitle("Bağlantılar")
        .navigationBarTitleDisplayMode(.inline)
        .safeAreaInset(edge: .bottom) { silSatiri }
        .confirmationDialog("Bu bağlantıyı sil?", isPresented: $silmeSorusu, titleVisibility: .visible) {
            Button("Sil", role: .destructive) { silmeIstegi() }
            Button("Vazgeç", role: .cancel) {}
        } message: {
            Text("\(baglanti.ad) silinecek. Anahtarı Keychain'den kaldırılır; geçmiş sohbetlerdeki izler silinmez.")
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
            Text(baglanti.ad)
                .font(Yazi.marka())
                .foregroundStyle(Renk.murekkep)
            Text(baglanti.urlHam)
                .font(Yazi.cip())
                .foregroundStyle(Renk.gri)
                .fixedSize(horizontal: false, vertical: true)
            if baglanti.anahtarRefi != nil {
                Text("Erişim anahtarı Keychain'de saklı.")
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.soluk)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(Text(verbatim: "\(baglanti.ad). \(baglanti.urlHam)"))
        .accessibilityAddTraits(.isHeader)
    }

    private var cihazVerisiBolumu: some View {
        bolum("CİHAZ VERİSİ") {
            VStack(alignment: .leading, spacing: Olcek.s2) {
                Picker("Cihaz verisi", selection: Binding(
                    get: { baglanti.cihazVerisi },
                    set: { ayarla($0) }
                )) {
                    Text("hiçbir zaman").tag(CihazVerisiAyari.hicbirZaman)
                    Text("her seferinde sor").tag(CihazVerisiAyari.herSeferindeSor)
                }
                .pickerStyle(.segmented)
                .disabled(baglanti.isDeleted)
                .accessibilityLabel(Text("Cihaz verisi paylaşımı"))

                Text(baglanti.cihazVerisi == .hicbirZaman
                     ? "Cihazından okunan veri bu sunucuya hiç gönderilmez."
                     : "Cihazından okunan veri gönderilmeden önce her seferinde sana gösterilir.")
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.gri)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private var aracBolumu: some View {
        bolum("ARAÇLAR") {
            VStack(alignment: .leading, spacing: Olcek.s3) {
                if baglanti.aracOzetleri.isEmpty {
                    Text("Bu sunucudan araç alınmadı.")
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.gri)
                } else {
                    ForEach(baglanti.aracOzetleri) { arac in
                        AracOzetiSatiri(arac: arac)
                    }
                }
            }
        }
    }

    private var denemeBolumu: some View {
        VStack(alignment: .leading, spacing: Olcek.s3) {
            Button { dene() } label: {
                HStack(spacing: Olcek.s2) {
                    if deneniyor {
                        ProgressView()
                            .controlSize(.small)
                            .tint(Renk.gri)
                    }
                    Text(deneniyor ? "Deneniyor…" : "Bağlantıyı dene")
                    Spacer(minLength: 0)
                }
                .font(Yazi.kullanici())
                .foregroundStyle(deneniyor ? Renk.soluk : Renk.murekkep)
                .padding(.vertical, Olcek.s3)
                .padding(.horizontal, Olcek.s4)
                .frame(minHeight: Olcek.dokunmaHedefi)
                .overlay(
                    RoundedRectangle(cornerRadius: Olcek.s4, style: .continuous)
                        .stroke(Renk.cizgi, lineWidth: Olcek.hairline)
                )
                .contentShape(RoundedRectangle(cornerRadius: Olcek.s4, style: .continuous))
            }
            .buttonStyle(.plain)
            .disabled(deneniyor || baglanti.isDeleted)
            .accessibilityHint(Text("Sunucuya bağlanır ve araç listesini tazeler."))

            if let denemeNotu {
                Text(denemeNotu)
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.gri)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityAddTraits(.isSummaryElement)
            }
        }
    }

    private var silSatiri: some View {
        Button("Bağlantıyı sil") { silmeSorusu = true }
            .font(Yazi.cip())
            .foregroundStyle(Renk.gri)
            .accessibilityHint(Text("Onay sorulur. Anahtar Keychain'den kaldırılır."))
            .padding(.horizontal, Olcek.s5)
            .padding(.top, Olcek.s3)
            .padding(.bottom, Olcek.s2)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Renk.zemin)
    }

    private func bolum<Icerik: View>(_ baslik: LocalizedStringKey,
                                     @ViewBuilder icerik: () -> Icerik) -> some View {
        VStack(alignment: .leading, spacing: Olcek.s3) {
            Text(baslik)
                .font(Yazi.etiket())
                .tracking(1.2)
                .foregroundStyle(Renk.soluk)
                .accessibilityAddTraits(.isHeader)
            icerik()
        }
    }

    // MARK: - Eylemler

    private func ayarla(_ yeni: CihazVerisiAyari) {
        guard !baglanti.isDeleted else { return }
        let eski = baglanti.cihazVerisi
        baglanti.cihazVerisi = yeni
        do {
            try kayit.save()
        } catch {
            // Diske yazılamadıysa ayar ekranda değişmiş görünüp uygulama
            // kapanınca eskiye dönerdi; geri alıp söylüyoruz.
            baglanti.cihazVerisi = eski
            kayit.rollback()
            uyariMetni = String(localized: "Ayar kaydedilemedi: \(error.localizedDescription)")
        }
    }

    private func dene() {
        deneniyor = true
        denemeNotu = nil
        Task { @MainActor in
            let sonuc = await servis.dene(baglanti)
            deneniyor = false
            // Görünüm ömrüne bağlı olmayan iş: model silinmişse yazma.
            guard !baglanti.isDeleted else { return }
            switch sonuc {
            case .basarili(let liste):
                let calisan = liste.filter { !$0.desteklenmiyor }.count
                denemeNotu = String(localized: "Bağlantı kuruldu. \(calisan) araç kullanılabilir.")
            case .basarisiz(let neden):
                denemeNotu = neden
            }
        }
    }
}
