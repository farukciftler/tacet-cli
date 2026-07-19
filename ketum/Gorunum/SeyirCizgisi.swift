//
//  SeyirCizgisi.swift
//  ketum
//
//  Katlanmış seyir satırı + katlanır zaman çizgisi (seyir-spec §3.2, §3.3).
//
//  Katlama kuralı (§2.2) burada SAF FONKSİYON olarak yaşar: yan etki ve hata
//  çipleri katlamanın DIŞINDA kalır, yalnızca okuma çipleri içeri girer.
//  Şeffaflık açılır-kapanır bir süs değildir.
//
//  Araç adımının detayı MEVCUT `AracCipiDetay` sheet'idir — ikinci bir detay
//  yüzeyi yazılmaz (§2.4).
//

import SwiftUI

// MARK: - Katlama kuralı (saf fonksiyonlar, OtoTest edilir)

enum SeyirKatlama {

    /// Katlamanın DIŞINDA kalan, her zaman görünen izler.
    /// Kural: yan etki (`yazildi`) ve hata (`basarisiz`) asla gizlenmez.
    /// Ek olarak kullanıcıdan eylem bekleyen ya da hâlâ süren durumlar da
    /// dışarıda tutulur — bunları katlamak çıkmaz sokak yaratırdı
    /// (spec metni yalnız yazildi/basarisiz sayar; gerekçeli genişletme).
    static func katlamaDisi(_ izler: [AracIzi]) -> [AracIzi] {
        izler.filter { !katlanirMi($0.durum) }
    }

    /// Katlamanın İÇİNE giren izler — yalnızca okuma.
    static func katlamaIci(_ izler: [AracIzi]) -> [AracIzi] {
        izler.filter { katlanirMi($0.durum) }
    }

    /// Tek durum kararı: yalnızca `okundu` katlanır.
    static func katlanirMi(_ durum: AracDurumu) -> Bool {
        switch durum {
        case .okundu:
            return true
        case .calisiyor, .yazildi, .izinGerekli, .basarisiz, .onayBekleniyor, .gonderilmedi:
            return false
        }
    }

    /// Katlama satırı çizilsin mi? Adım yoksa ya da tek adım varsa ve o da
    /// yazımsa (araçsız yanıt) Seyir SUSAR — söyleyecek şeyi yoktur (§3.2).
    static func satirGosterilirMi(_ adimlar: [SeyirAdimi]) -> Bool {
        guard !adimlar.isEmpty else { return false }
        if adimlar.count == 1, adimlar[0].tur == .yazim { return false }
        return true
    }

    /// Katlama satırının metni: "seyir · 4 adım · 6 sn" ya da başarısız varsa
    /// "seyir · 4 adım · 1 aşılamadı" (§3.4).
    static func ozetMetni(adimlar: [SeyirAdimi], izler: [AracIzi]) -> String {
        let sayi = adimlar.count
        let asilamadi = izler.filter {
            if case .basarisiz = $0.durum { return true }
            return false
        }.count

        let adimParcasi = String(localized: "\(sayi) adım")
        if asilamadi > 0 {
            let hataParcasi = String(localized: "\(asilamadi) aşılamadı")
            return "\(SeyirKatlama.baslik) · \(adimParcasi) · \(hataParcasi)"
        }
        return "\(SeyirKatlama.baslik) · \(adimParcasi) · \(SeyirSure.metin(toplamSure(adimlar)))"
    }

    /// Kapanmış adımların toplam süresi. Süren adım toplama katılmaz —
    /// tahmin edilmez (§7: yalan ilerleme yok).
    static func toplamSure(_ adimlar: [SeyirAdimi]) -> TimeInterval {
        adimlar.reduce(0) { $0 + ($1.sure ?? 0) }
    }

    static var baslik: String { String(localized: "seyir") }
}

// MARK: - Satır metni (tek doğruluk kaynağı: AracIzi)

enum SeyirMetni {
    /// Bir adımın görünen metni. Araç adımında metin adımdan değil izden okunur.
    static func satir(_ adim: SeyirAdimi, izler: [AracIzi]) -> String {
        if adim.tur == .arac {
            return iz(adim, izler: izler)?.metin ?? adim.metin
        }
        return adim.metin
    }

    /// Adıma bağlı iz.
    static func iz(_ adim: SeyirAdimi, izler: [AracIzi]) -> AracIzi? {
        guard let id = adim.aracIziID else { return nil }
        return izler.first { $0.id == id }
    }
}

// MARK: - Süre biçimi

enum SeyirSure {
    /// "0,2 sn" / "6 sn" — cihaz diline göre ondalık ayraç.
    static func metin(_ saniye: TimeInterval) -> String {
        let f = NumberFormatter()
        f.locale = .current
        f.numberStyle = .decimal
        f.minimumFractionDigits = 0
        f.maximumFractionDigits = saniye < 10 ? 1 : 0
        let sayi = f.string(from: NSNumber(value: max(0, saniye))) ?? "0"
        return String(localized: "\(sayi) sn")
    }
}

// MARK: - Görünüm

/// Katlanır zaman çizgisi. `DisclosureGroup` DEĞİL — açılış animasyonu
/// `hareketiAzalt`a saygılı olsun ve satırlar hairline dilinde kalsın diye
/// elle kurulur. Varsayılan HEP KAPALI; açık/kapalı durumu mesaj başına
/// hatırlanmaz (§3.3).
struct SeyirCizgisi: View {
    let adimlar: [SeyirAdimi]
    var izler: [AracIzi] = []
    /// Dışarıdan açık başlatma (canlı şerit dokunuşu) için.
    var baslangictaAcik: Bool = false

    @State private var acik = false
    @State private var detayIzi: AracIzi?
    @Environment(\.accessibilityReduceMotion) private var hareketiAzalt

    var body: some View {
        if SeyirKatlama.satirGosterilirMi(adimlar) {
            VStack(alignment: .leading, spacing: Olcek.s2) {
                katlamaSatiri
                if acik {
                    zamanCizgisi
                        .transition(hareketiAzalt
                                    ? .opacity
                                    : .opacity.combined(with: .move(edge: .top)))
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .onAppear { acik = baslangictaAcik }
            .sheet(item: $detayIzi) { iz in
                AracCipiDetay(iz: iz)
            }
        }
    }

    // Katlanmış satır: çiplerden bir kademe geride — hairline ÇERÇEVESİZ, soluk.
    private var katlamaSatiri: some View {
        Button {
            if hareketiAzalt {
                acik.toggle()
            } else {
                withAnimation(.easeInOut(duration: 0.18)) { acik.toggle() }
            }
        } label: {
            HStack(spacing: Olcek.s1) {
                Text(SeyirKatlama.ozetMetni(adimlar: adimlar, izler: izler))
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.soluk)
                Image(systemName: acik ? "chevron.down" : "chevron.right")
                    .font(Yazi.ikonKucuk())
                    .foregroundStyle(Renk.soluk)
                    .accessibilityHidden(true)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text(SeyirKatlama.ozetMetni(adimlar: adimlar, izler: izler)))
        .accessibilityHint(acik ? Text("Kapatmak için dokun") : Text("Açmak için dokun"))
    }

    // Açık liste: dikey hairline adımları bağlar; ikon yok, renk yok.
    private var zamanCizgisi: some View {
        VStack(alignment: .leading, spacing: Olcek.s2) {
            ForEach(adimlar) { adim in
                satir(adim)
            }
        }
        .padding(.leading, Olcek.s3)
        .overlay(alignment: .leading) {
            Rectangle()
                .fill(Renk.cizgi)
                .frame(width: Olcek.hairline)
                .accessibilityHidden(true)
        }
        .accessibilityElement(children: .contain)
    }

    @ViewBuilder
    private func satir(_ adim: SeyirAdimi) -> some View {
        let iz = SeyirMetni.iz(adim, izler: izler)
        let metin = SeyirMetni.satir(adim, izler: izler)
        let sure = adim.sure.map { SeyirSure.metin($0) } ?? "—"

        if let iz {
            // Araç adımı: detay MEVCUT AracCipiDetay sheet'idir (§2.4).
            Button { detayIzi = iz } label: {
                satirGovdesi(metin: metin, sure: sure, chevron: true)
            }
            .buttonStyle(.plain)
            .accessibilityLabel(Text("\(metin), \(sure)"))
            .accessibilityHint(Text("Ayrıntı için dokun"))
        } else {
            // Boru hattı adımı: detayı yok, süre zaten satırda.
            satirGovdesi(metin: metin, sure: sure, chevron: false)
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(Text("\(metin), \(sure)"))
        }
    }

    private func satirGovdesi(metin: String, sure: String, chevron: Bool) -> some View {
        HStack(spacing: Olcek.s2) {
            Text(metin)
                .font(Yazi.cip())
                .foregroundStyle(Renk.gri)
                .lineLimit(1)
                .truncationMode(.middle)
            Spacer(minLength: Olcek.s2)
            Text(sure)
                .font(Yazi.cip())
                .foregroundStyle(Renk.soluk)
                .monospacedDigit()
            if chevron {
                Image(systemName: "chevron.right")
                    .font(Yazi.ikonKucuk())
                    .foregroundStyle(Renk.soluk)
                    .accessibilityHidden(true)
            }
        }
        .contentShape(Rectangle())
    }
}

#Preview("Seyir çizgisi") {
    let izID = UUID()
    let adimlar: [SeyirAdimi] = [
        SeyirAdimi(tur: .yonlendirme, metin: "yönlendirildi · takvim profili",
                   baslangic: Date(), bitis: Date().addingTimeInterval(0.2)),
        SeyirAdimi(tur: .zenginlestirme, metin: "beceri eklendi · belge-oku",
                   baslangic: Date(), bitis: Date().addingTimeInterval(0.05)),
        SeyirAdimi(tur: .arac, aracIziID: izID,
                   baslangic: Date(), bitis: Date().addingTimeInterval(1.1)),
        SeyirAdimi(tur: .yazim, metin: "yazıldı",
                   baslangic: Date(), bitis: Date().addingTimeInterval(4.4))
    ]
    let izler = [AracIzi(id: izID, ikon: "calendar", metin: "takvim okundu · 3 etkinlik",
                         durum: .okundu, hamGirdi: "bugün", hamCikti: "09:00 toplantı")]

    return VStack(alignment: .leading, spacing: Olcek.cipYanitAraligi) {
        SeyirCizgisi(adimlar: adimlar, izler: izler)
        KetumYaniti(metin: "Bugün üç toplantın var.")
    }
    .padding(Olcek.s5)
    .frame(maxWidth: .infinity, alignment: .leading)
    .background(Renk.zemin)
}
