//
//  YeniNobet.swift
//  ketum
//
//  Kompakt nöbet kurulumu: ad, teslimat saati, içerik anahtarları. Zamanlama
//  bölümü ne olacağını olduğu gibi söyler — arka planda gece üretimi yoktur.
//

import SwiftUI

struct YeniNobet: View {
    let baglam: NobetBaglami

    @Environment(\.dismiss) private var kapat
    @State private var ad = String(localized: "Sabah brifingi")
    @State private var saat = 7
    @State private var takvim = true
    @State private var hatirlatici = true
    @State private var notlar = false
    @State private var kuruluyor = false
    @State private var kurulamadi = false

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: Olcek.s5) {
                    adBolumu
                    zamanlamaBolumu
                    icerikBolumu
                    if kurulamadi { hataSatiri }
                }
                .padding(.horizontal, Olcek.s5)
                .padding(.vertical, Olcek.s4)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .background(Renk.zemin)
            .navigationTitle("Yeni nöbet")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Vazgeç") { kapat() }
                        .font(Yazi.cip())
                        .foregroundStyle(kuruluyor ? Renk.soluk : Renk.gri)
                        .disabled(kuruluyor)
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Kur") { kur() }
                        .font(Yazi.cip())
                        .foregroundStyle(kurulabilir ? Renk.murekkep : Renk.soluk)
                        .disabled(!kurulabilir)
                }
            }
        }
        .interactiveDismissDisabled(kuruluyor)
    }

    private var kurulabilir: Bool {
        !kuruluyor && !ad.trimmingCharacters(in: .whitespaces).isEmpty
    }

    // MARK: - Bölümler

    private var adBolumu: some View {
        bolum("AD") {
            TextField("Sabah brifingi", text: $ad)
                .font(Yazi.kullanici())
                .foregroundStyle(Renk.murekkep)
                .textInputAutocapitalization(.sentences)
                .padding(.vertical, Olcek.s3)
                .padding(.horizontal, Olcek.s4)
                .cerceve()
        }
    }

    private var zamanlamaBolumu: some View {
        bolum("TESLİMAT") {
            VStack(alignment: .leading, spacing: Olcek.s3) {
                HStack {
                    Text("Saat")
                        .font(Yazi.kullanici())
                        .foregroundStyle(Renk.murekkep)
                    Spacer(minLength: Olcek.s4)
                    Picker("Saat", selection: $saat) {
                        ForEach(0..<24, id: \.self) { s in
                            Text(String(format: "%02d.00", s)).tag(s)
                        }
                    }
                    .pickerStyle(.menu)
                    .tint(Renk.gri)
                }
                .padding(.vertical, Olcek.s2)
                .padding(.horizontal, Olcek.s4)
                .cerceve()

                // Dürüstlük: arka planda gece üretimi yok, olduğu gibi yazılır.
                Text("Bu saatte bildirim planlanır; brifing sirr'i açtığında tazelenir.")
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.gri)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private var icerikBolumu: some View {
        bolum("İÇERİK") {
            VStack(spacing: 0) {
                anahtar("Takvim", $takvim)
                ayrac
                anahtar("Hatırlatıcılar", $hatirlatici)
                ayrac
                anahtar("Notlar", $notlar)
            }
            .cerceve()
        }
    }

    // MARK: - Parçalar

    private func bolum<Icerik: View>(_ baslik: LocalizedStringKey,
                                     @ViewBuilder icerik: () -> Icerik) -> some View {
        VStack(alignment: .leading, spacing: Olcek.s3) {
            Text(baslik)
                .font(Yazi.etiket())
                .tracking(1.2)
                .foregroundStyle(Renk.soluk)
            icerik()
        }
    }

    private func anahtar(_ baslik: LocalizedStringKey, _ deger: Binding<Bool>) -> some View {
        Toggle(isOn: deger) {
            Text(baslik)
                .font(Yazi.kullanici())
                .foregroundStyle(Renk.murekkep)
        }
        .tint(Renk.murekkep)
        .padding(.vertical, Olcek.s3)
        .padding(.horizontal, Olcek.s4)
    }

    /// Kurulum tutmadığında sade, tek cümlelik açıklama.
    private var hataSatiri: some View {
        Text("Nöbet kurulamadı. Bir daha dene.")
            .font(Yazi.cip())
            .foregroundStyle(Renk.gri)
            .fixedSize(horizontal: false, vertical: true)
    }

    private var ayrac: some View {
        Rectangle()
            .fill(Renk.cizgi)
            .frame(height: Olcek.hairline)
            .padding(.leading, Olcek.s4)
    }

    // MARK: - Kurulum

    private func kur() {
        kuruluyor = true
        kurulamadi = false
        let isim = ad.trimmingCharacters(in: .whitespaces)
        Task { @MainActor in
            let oldu = await baglam.kur(ad: isim, saat: saat, takvim: takvim,
                                        hatirlatici: hatirlatici, notlar: notlar)
            kuruluyor = false
            // Kurulmadıysa sayfa açık kalır; kullanıcı ne olduğunu görür.
            guard oldu else {
                kurulamadi = true
                return
            }
            kapat()
        }
    }
}

private extension View {
    /// Kurulum satırlarının ortak hairline çerçevesi.
    func cerceve() -> some View {
        overlay(
            RoundedRectangle(cornerRadius: Olcek.s4, style: .continuous)
                .stroke(Renk.cizgi, lineWidth: Olcek.hairline)
        )
    }
}
