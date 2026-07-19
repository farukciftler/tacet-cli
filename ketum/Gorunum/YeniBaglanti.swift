//
//  YeniBaglanti.swift
//  ketum
//
//  Sunucu ekleme formu — mcp-baglanti-spec §3.1.
//
//  "Bağlantıyı dene" ZORUNLU adımdır: kullanıcı sunucunun ne yapabildiğini
//  eklemeden ÖNCE görür. Bağlantı kurulamazsa neden düz dille yazılır.
//  Cihaz verisi varsayılanı "hiçbir zaman"dır. "her zaman izin ver" seçilebilir
//  ama seçildiği anda uyarı modalına düşer — bkz. CihazVerisiSecici.
//

import SwiftUI

struct YeniBaglanti: View {
    let servis: any BaglantiDeneyici

    @Environment(\.dismiss) private var kapat

    @State private var ad = ""
    @State private var urlHam = ""
    @State private var anahtar = ""
    @State private var cihazVerisi: CihazVerisiAyari = .hicbirZaman

    @State private var deneniyor = false
    /// Başarılı denemeden dönen araç listesi. Eklemeden önce gösterilir.
    @State private var araclar: [AracOzeti]?
    /// Deneme başarısızsa nedenin düz dilli karşılığı.
    @State private var hataMetni: String?
    @State private var eklenemedi: String?

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: Olcek.s5) {
                    adBolumu
                    urlBolumu
                    anahtarBolumu
                    cihazVerisiBolumu
                    denemeBolumu
                    if let eklenemedi { uyari(eklenemedi) }
                }
                .padding(.horizontal, Olcek.s5)
                .padding(.vertical, Olcek.s4)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .background(Renk.zemin)
            .navigationTitle("Sunucu ekle")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Vazgeç") { kapat() }
                        .font(Yazi.cip())
                        .foregroundStyle(deneniyor ? Renk.soluk : Renk.gri)
                        .disabled(deneniyor)
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Ekle") { ekle() }
                        .font(Yazi.cip())
                        .foregroundStyle(eklenebilir ? Renk.murekkep : Renk.soluk)
                        .disabled(!eklenebilir)
                        .accessibilityHint(eklenebilir
                                           ? Text("Sunucuyu kaydeder ve bu sayfayı kapatır.")
                                           : Text("Önce bağlantıyı denemen gerekiyor."))
                }
            }
        }
        .interactiveDismissDisabled(deneniyor)
    }

    // MARK: - Durum

    private var temizAd: String { ad.trimmingCharacters(in: .whitespacesAndNewlines) }
    private var temizURL: String { urlHam.trimmingCharacters(in: .whitespacesAndNewlines) }

    private var denenebilir: Bool {
        !deneniyor && !temizAd.isEmpty && BaglantiURLDenetimi.kabulEdilirMi(temizURL)
    }

    /// Ekleme yalnızca BAŞARILI denemeden sonra açılır (§3.1).
    private var eklenebilir: Bool { !deneniyor && araclar != nil && !temizAd.isEmpty }

    // MARK: - Bölümler

    private var adBolumu: some View {
        bolum("AD") {
            TextField("ev sunucusu", text: $ad)
                .font(Yazi.kullanici())
                .foregroundStyle(Renk.murekkep)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .padding(.vertical, Olcek.s3)
                .padding(.horizontal, Olcek.s4)
                .cerceve()
                .accessibilityLabel(Text("Sunucunun adı"))
                .accessibilityHint(Text("Çiplerde bu ad görünür."))
                .onChange(of: ad) { _, _ in denemeyiGecersizKil() }
        }
    }

    private var urlBolumu: some View {
        bolum("URL") {
            VStack(alignment: .leading, spacing: Olcek.s2) {
                TextField("https://…", text: $urlHam)
                    .font(Yazi.kullanici())
                    .foregroundStyle(Renk.murekkep)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .keyboardType(.URL)
                    .padding(.vertical, Olcek.s3)
                    .padding(.horizontal, Olcek.s4)
                    .cerceve()
                    .accessibilityLabel(Text("Sunucu adresi"))
                    .onChange(of: urlHam) { _, _ in denemeyiGecersizKil() }

                Text(urlNotu)
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.gri)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    /// Adres kuralı: `https://`. Düz `http://` yalnızca yerel ağ adreslerinde.
    private var urlNotu: LocalizedStringKey {
        if temizURL.isEmpty || BaglantiURLDenetimi.kabulEdilirMi(temizURL) {
            return "Streamable HTTP adresi. Düz http yalnızca yerel ağ adreslerinde kabul edilir."
        }
        return "Bu adres kabul edilmiyor: https kullan; düz http yalnızca yerel ağda geçerli."
    }

    private var anahtarBolumu: some View {
        bolum("ERİŞİM ANAHTARI") {
            VStack(alignment: .leading, spacing: Olcek.s2) {
                SecureField("isteğe bağlı", text: $anahtar)
                    .font(Yazi.kullanici())
                    .foregroundStyle(Renk.murekkep)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .padding(.vertical, Olcek.s3)
                    .padding(.horizontal, Olcek.s4)
                    .cerceve()
                    .accessibilityLabel(Text("Erişim anahtarı, isteğe bağlı"))
                    .onChange(of: anahtar) { _, _ in denemeyiGecersizKil() }

                Text("Keychain'de saklanır ve bir daha gösterilmez.")
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.gri)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private var cihazVerisiBolumu: some View {
        bolum("CİHAZ VERİSİ") {
            CihazVerisiSecici(secim: $cihazVerisi)
        }
    }

    private var denemeBolumu: some View {
        bolum("BAĞLANTI") {
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
                    .foregroundStyle(denenebilir ? Renk.murekkep : Renk.soluk)
                    .padding(.vertical, Olcek.s3)
                    .padding(.horizontal, Olcek.s4)
                    .frame(minHeight: Olcek.dokunmaHedefi)
                    .cerceve()
                    .contentShape(RoundedRectangle(cornerRadius: Olcek.s4, style: .continuous))
                }
                .buttonStyle(.plain)
                .disabled(!denenebilir)
                .accessibilityHint(Text("Sunucuya bağlanır ve araç listesini getirir."))

                if let hataMetni { uyari(hataMetni) }
                if let araclar { aracListesi(araclar) }
            }
        }
    }

    private func aracListesi(_ liste: [AracOzeti]) -> some View {
        VStack(alignment: .leading, spacing: Olcek.s3) {
            Text(liste.isEmpty
                 ? "Bağlantı kuruldu; sunucu hiç araç bildirmedi."
                 : "Bağlantı kuruldu. Bu sunucunun araçları:")
                .font(Yazi.cip())
                .foregroundStyle(Renk.gri)
                .fixedSize(horizontal: false, vertical: true)

            ForEach(liste) { arac in
                AracOzetiSatiri(arac: arac)
            }
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
                .accessibilityAddTraits(.isHeader)
            icerik()
        }
    }

    /// Başarısızlık düz dille yazılır; sessizce yutulmaz.
    private func uyari(_ metin: String) -> some View {
        Text(metin)
            .font(Yazi.cip())
            .foregroundStyle(Renk.gri)
            .fixedSize(horizontal: false, vertical: true)
            .accessibilityLabel(Text(verbatim: String(localized: "Hata: \(metin)")))
            .accessibilityAddTraits(.isSummaryElement)
    }

    // MARK: - Eylemler

    /// Form değişince eski deneme sonucu geçersizdir: başka bir sunucunun
    /// araç listesiyle kayıt açılmasın.
    private func denemeyiGecersizKil() {
        araclar = nil
        hataMetni = nil
        eklenemedi = nil
    }

    private func dene() {
        deneniyor = true
        hataMetni = nil
        araclar = nil
        eklenemedi = nil
        let isim = temizAd
        let adres = temizURL
        let sir = anahtar.isEmpty ? nil : anahtar
        Task { @MainActor in
            let sonuc = await servis.dene(ad: isim, urlHam: adres, anahtar: sir)
            deneniyor = false
            switch sonuc {
            case .basarili(let liste): araclar = liste
            case .basarisiz(let neden): hataMetni = neden
            }
        }
    }

    private func ekle() {
        guard let liste = araclar else { return }
        do {
            _ = try servis.ekle(ad: temizAd,
                                urlHam: temizURL,
                                anahtar: anahtar.isEmpty ? nil : anahtar,
                                cihazVerisi: cihazVerisi,
                                araclar: liste)
            kapat()
        } catch {
            // Sayfa açık kalır; kullanıcı ne olduğunu görür.
            eklenemedi = String(localized: "Bağlantı kaydedilemedi: \(error.localizedDescription)")
        }
    }
}

// MARK: - Ortak parçalar

/// Araç listesindeki tek satır. Desteklenmeyen araç sessizce yutulmaz (§5.2).
struct AracOzetiSatiri: View {
    let arac: AracOzeti

    var body: some View {
        VStack(alignment: .leading, spacing: Olcek.s1) {
            HStack(spacing: Olcek.s2) {
                Text(arac.ad)
                    .font(Yazi.kullanici())
                    .foregroundStyle(arac.desteklenmiyor ? Renk.gri : Renk.murekkep)
                    .lineLimit(1)
                if arac.desteklenmiyor {
                    Text("desteklenmiyor")
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.soluk)
                }
            }
            if !arac.ozet.isEmpty {
                Text(arac.ozet)
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.gri)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.vertical, Olcek.s3)
        .padding(.horizontal, Olcek.s4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(
            RoundedRectangle(cornerRadius: Olcek.s4, style: .continuous)
                .stroke(Renk.cizgi, lineWidth: Olcek.hairline)
        )
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(verbatim: arac.ad))
        .accessibilityValue(arac.desteklenmiyor
                            ? Text(verbatim: String(localized: "Desteklenmiyor. \(arac.ozet)"))
                            : Text(verbatim: arac.ozet))
    }
}

/// Adres kuralı tek yerde: `https://` her yerde, düz `http://` yalnızca
/// yerel ağ adreslerinde (§3.1). Servis katmanı da bunu kullanır.
enum BaglantiURLDenetimi {
    static func kabulEdilirMi(_ ham: String) -> Bool {
        guard let url = URL(string: ham.trimmingCharacters(in: .whitespacesAndNewlines)),
              let sema = url.scheme?.lowercased(),
              let sunucu = url.host()?.lowercased(), !sunucu.isEmpty else { return false }
        if sema == "https" { return true }
        guard sema == "http" else { return false }
        return yerelAgMi(sunucu)
    }

    /// Yerel ağ: localhost, .local (Bonjour), özel IPv4 blokları, IPv6 loopback.
    static func yerelAgMi(_ sunucu: String) -> Bool {
        if sunucu == "localhost" || sunucu == "127.0.0.1" || sunucu == "::1" { return true }
        if sunucu.hasSuffix(".local") { return true }

        let parcalar = sunucu.split(separator: ".").compactMap { Int($0) }
        guard parcalar.count == 4, parcalar.allSatisfy({ (0...255).contains($0) }) else { return false }
        switch (parcalar[0], parcalar[1]) {
        case (10, _): return true
        case (192, 168): return true
        case (172, 16...31): return true
        case (169, 254): return true   // link-local
        default: return false
        }
    }
}

private extension View {
    /// Bağlantı kurulum satırlarının ortak hairline çerçevesi.
    func cerceve() -> some View {
        overlay(
            RoundedRectangle(cornerRadius: Olcek.s4, style: .continuous)
                .stroke(Renk.cizgi, lineWidth: Olcek.hairline)
        )
    }
}
