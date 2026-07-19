//
//  Ayarlar.swift
//  ketum
//
//  Ayarlar sayfası — dil tercihleri, sohbet geçmişinin temizlenmesi ve
//  izinler. Üç bölüm, aralarında etiket başlıklar; süs yok.
//

import SwiftUI

struct Ayarlar: View {
    var gecmisiTemizle: () -> Void

    @Environment(\.dismiss) private var kapat

    private let dil = DilTercihi.paylasilan
    @State private var temizlemeOnayi = false

    init(gecmisiTemizle: @escaping () -> Void) {
        self.gecmisiTemizle = gecmisiTemizle
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: Olcek.s5) {
                    dilBolumu
                    verilerBolumu
                    izinlerBolumu
                    dipNot
                }
                .padding(.horizontal, Olcek.s5)
                .padding(.vertical, Olcek.s4)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .background(Renk.zemin)
            .navigationTitle("Ayarlar")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Kapat") { kapat() }
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.gri)
                }
            }
        }
    }

    // MARK: - Dil

    private var dilBolumu: some View {
        bolum("DİL") {
            VStack(spacing: 0) {
                satir(baslik: "Yanıt dili",
                      altyazi: dil.yanitDili.isEmpty
                          ? String(localized: "yazdığın dile göre")
                          : nil,
                      secim: adi(dil.yanitDili, otomatik: String(localized: "Otomatik"))) {
                    Button { dil.yanitDili = "" } label: { Text("Otomatik") }
                    ForEach(DilTercihi.secenekler, id: \.kod) { secenek in
                        Button { dil.yanitDili = secenek.kod } label: { Text(secenek.ad) }
                    }
                }

                ayrac

                satir(baslik: "Arayüz dili",
                      altyazi: dil.arayuzYenidenBaslatmaBekliyor
                          ? String(localized: "Yeni dil, sirr'ı kapatıp yeniden açtığında görünür.")
                          : nil,
                      secim: adi(dil.arayuzDili, otomatik: String(localized: "Cihaz dili"))) {
                    Button { dil.arayuzDili = "" } label: { Text("Cihaz dili") }
                    ForEach(DilTercihi.secenekler, id: \.kod) { secenek in
                        Button { dil.arayuzDili = secenek.kod } label: { Text(secenek.ad) }
                    }
                }
            }
            .cerceve()
        }
    }

    private func adi(_ kod: String, otomatik: String) -> String {
        guard !kod.isEmpty else { return otomatik }
        return DilTercihi.secenekler.first { $0.kod == kod }?.ad ?? kod
    }

    private func satir<Secenekler: View>(baslik: LocalizedStringKey,
                                         altyazi: String?,
                                         secim: String,
                                         @ViewBuilder secenekler: () -> Secenekler) -> some View {
        VStack(alignment: .leading, spacing: Olcek.s1) {
            Menu {
                secenekler()
            } label: {
                HStack(spacing: Olcek.s2) {
                    Text(baslik)
                        .font(Yazi.kullanici())
                        .foregroundStyle(Renk.murekkep)
                    Spacer(minLength: 0)
                    Text(secim)
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.gri)
                        .lineLimit(1)
                    Image(systemName: "chevron.up.chevron.down")
                        .font(.system(size: 11))
                        .foregroundStyle(Renk.soluk)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            if let altyazi {
                Text(altyazi)
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.soluk)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.vertical, Olcek.s3)
        .padding(.horizontal, Olcek.s4)
    }

    // MARK: - Veriler

    private var verilerBolumu: some View {
        bolum("VERİLER") {
            Button { temizlemeOnayi = true } label: {
                HStack(spacing: Olcek.s2) {
                    Text("Tüm sohbet geçmişini temizle")
                        .font(Yazi.kullanici())
                        .foregroundStyle(Renk.murekkep)
                        .multilineTextAlignment(.leading)
                    Spacer(minLength: 0)
                    Image(systemName: "chevron.right")
                        .font(.system(size: 12))
                        .foregroundStyle(Renk.soluk)
                }
                .padding(.vertical, Olcek.s3)
                .padding(.horizontal, Olcek.s4)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .cerceve()
            .confirmationDialog("Tüm sohbet geçmişi temizlensin mi?",
                                isPresented: $temizlemeOnayi,
                                titleVisibility: .visible) {
                Button("Temizle", role: .destructive) { gecmisiTemizle() }
                Button("Vazgeç", role: .cancel) { }
            } message: {
                Text("Sohbetler ve içindeki mesajlar silinir. Nöbetler ve üretilmiş belgeler kalır. Bu işlem geri alınamaz.")
            }
        }
    }

    // MARK: - İzinler

    private var izinlerBolumu: some View {
        bolum("İZİNLER") {
            VStack(alignment: .leading, spacing: Olcek.s3) {
                IzinBolumu()
                    .cerceve()

                Text(IzinBolumu.aciklama)
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.soluk)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    // MARK: - Parçalar

    private func bolum<Icerik: View>(_ baslik: LocalizedStringKey,
                                     @ViewBuilder icerik: () -> Icerik) -> some View {
        VStack(alignment: .leading, spacing: Olcek.s3) {
            Text(baslik)
                .font(Yazi.etiket())
                .textCase(.uppercase)
                .tracking(1.2)
                .foregroundStyle(Renk.soluk)
            icerik()
        }
    }

    private var ayrac: some View {
        Rectangle()
            .fill(Renk.cizgi)
            .frame(height: Olcek.hairline)
    }

    private var dipNot: some View {
        Text("Her şey bu cihazda kalır.")
            .font(Yazi.cip())
            .foregroundStyle(Renk.soluk)
            .padding(.top, Olcek.s2)
    }
}

private extension View {
    /// Ayarlar kartlarının ortak hairline çerçevesi.
    func cerceve() -> some View {
        overlay(
            RoundedRectangle(cornerRadius: Olcek.s4, style: .continuous)
                .stroke(Renk.cizgi, lineWidth: Olcek.hairline)
        )
    }
}
