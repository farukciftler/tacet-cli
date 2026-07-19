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
    /// Üretilen belgeleri silmek için gerekli bağlam. Verilmezse belge bölümü gizlenir.
    var belgeBaglami: BelgeBaglami?
    /// Bağlantılar panosunu kök sunum kanalından açar (mcp §3.1).
    var baglantilarAc: () -> Void

    @Environment(\.dismiss) private var kapat

    private let dil = DilTercihi.paylasilan
    @State private var temizlemeOnayi = false
    @State private var belgeSilmeOnayi = false
    /// Documents/sirr toplam boyutu — açılışta ve silmeden sonra ölçülür.
    @State private var belgeBoyutu: Int64 = 0

    // MARK: - Web araması durumu (web-arama-spec §3.1)

    /// Alandaki ham metin. KAYDEDİLMİŞ adres bu değil: adres yalnızca başarılı
    /// denemeden sonra `WebAramaAyari`ye yazılır — "zorunlu deneme" budur.
    @State private var aramaURL: String = WebAramaAyari.kokHam
    /// Aç/kapa ham bayrağı. `WebAramaAyari.aktifMi` adres geçersizken false
    /// döndürdüğü için anahtarın kendisi burada okunur.
    @AppStorage(WebAramaAyari.aktifAnahtar) private var aramaAcik = false
    @State private var arama: AramaDenemesi = .bekliyor
    /// Süren deneme — sayfa kapanınca iptal edilir.
    @State private var aramaGorevi: Task<Void, Never>?
    /// Kaydedilmiş adresin görünüme yansıması (statik ayarı gözlemleyemeyiz).
    @State private var kayitliAdres: String = WebAramaAyari.kokHam

    /// "Sunucuyu dene" adımının durumu. Başarısızlıkta neden düz dille taşınır;
    /// sessizce yutulan hata yok.
    enum AramaDenemesi: Equatable {
        case bekliyor
        case deneniyor
        /// Örnek sorgunun döndürdüğü sonuç sayısı.
        case basarili(Int)
        case basarisiz(String)

        /// Yalnızca başarısız deneme `Renk.hata` ile yazılır.
        var hataMi: Bool {
            if case .basarisiz = self { return true }
            return false
        }
    }

    init(gecmisiTemizle: @escaping () -> Void,
         belgeBaglami: BelgeBaglami? = nil,
         baglantilarAc: @escaping () -> Void = {}) {
        self.gecmisiTemizle = gecmisiTemizle
        self.belgeBaglami = belgeBaglami
        self.baglantilarAc = baglantilarAc
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: Olcek.s5) {
                    dilBolumu
                    verilerBolumu
                    if belgeBaglami != nil { belgelerBolumu }
                    webAramasiBolumu
                    baglantilarBolumu
                    izinlerBolumu
                    dipNot
                }
                .padding(.horizontal, Olcek.s5)
                .padding(.vertical, Olcek.s4)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .background(Renk.zemin)
            .task { belgeBoyutunuOlc() }
            .onDisappear {
                // Görünüm ömrüne bağlı olmayan iş sayfayla birlikte biter.
                aramaGorevi?.cancel()
                aramaGorevi = nil
            }
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
                        .font(Yazi.ikonKucuk())
                        .foregroundStyle(Renk.soluk)
                        .accessibilityHidden(true)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            // Üç ayrı parça ("Yanıt dili", "Otomatik", "chevron") olarak değil,
            // tek bir seçim denetimi olarak okunur.
            .accessibilityElement(children: .combine)
            .accessibilityLabel(Text(baslik))
            .accessibilityValue(Text(secim))
            .accessibilityHint(Text("Seçenekleri açmak için çift dokun."))

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
                        .font(Yazi.ikonKucuk())
                        .foregroundStyle(Renk.soluk)
                        .accessibilityHidden(true)
                }
                .padding(.vertical, Olcek.s3)
                .padding(.horizontal, Olcek.s4)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(Text("Tüm sohbet geçmişini temizle"))
            .accessibilityHint(Text("Onay sorulur. Sohbetler ve mesajlar silinir, hafıza kalır."))
            .cerceve()
            .confirmationDialog("Tüm sohbet geçmişi temizlensin mi?",
                                isPresented: $temizlemeOnayi,
                                titleVisibility: .visible) {
                Button("Temizle", role: .destructive) { gecmisiTemizle() }
                Button("Vazgeç", role: .cancel) { }
            } message: {
                // Hafıza burada SİLİNMEZ (hafiza-spec §7): sohbet ve hafıza ayrı
                // kararlardır. Bunu söylememek sessiz bir yalan olurdu.
                Text("Sohbetler ve içindeki mesajlar silinir. Nöbetler, üretilmiş belgeler ve hafıza kalır — hafızayı silmek için Hafıza panosunu kullan. Bu işlem geri alınamaz.")
            }
        }
    }

    // MARK: - Üretilen belgeler

    /// sirr'in ürettiği ve kullanıcının sohbete eklediği dosyalar Documents/sirr
    /// altında durur. Sohbeti silmek onları silmez; kişisel veri burada temizlenir.
    private var belgelerBolumu: some View {
        bolum("BELGELER") {
            VStack(alignment: .leading, spacing: Olcek.s3) {
                VStack(spacing: 0) {
                    HStack(spacing: Olcek.s2) {
                        Text("Kapladığı yer")
                            .font(Yazi.kullanici())
                            .foregroundStyle(Renk.murekkep)
                        Spacer(minLength: 0)
                        Text(boyutMetni)
                            .font(Yazi.cip())
                            .foregroundStyle(Renk.gri)
                            .monospacedDigit()
                    }
                    .padding(.vertical, Olcek.s3)
                    .padding(.horizontal, Olcek.s4)
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel(Text("Üretilen belgelerin kapladığı yer"))
                    .accessibilityValue(Text(boyutMetni))

                    ayrac

                    Button { belgeSilmeOnayi = true } label: {
                        HStack(spacing: Olcek.s2) {
                            Text("Üretilen belgeleri sil")
                                .font(Yazi.kullanici())
                                .foregroundStyle(belgeVarMi ? Renk.murekkep : Renk.soluk)
                                .multilineTextAlignment(.leading)
                            Spacer(minLength: 0)
                            Image(systemName: "chevron.right")
                                .font(Yazi.ikonKucuk())
                                .foregroundStyle(Renk.soluk)
                                .accessibilityHidden(true)
                        }
                        .padding(.vertical, Olcek.s3)
                        .padding(.horizontal, Olcek.s4)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .disabled(!belgeVarMi)
                    .accessibilityLabel(Text("Üretilen belgeleri sil"))
                    .accessibilityHint(Text("Onay sorulur. Cihazdaki tüm sirr belgeleri silinir."))
                }
                .cerceve()

                Text("Oluşturulan ve sohbete eklenen dosyalar cihazda Belgeler klasöründe saklanır.")
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.soluk)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .confirmationDialog("Üretilen belgeler silinsin mi?",
                                isPresented: $belgeSilmeOnayi,
                                titleVisibility: .visible) {
                Button("Sil", role: .destructive) { belgeleriSil() }
                Button("Vazgeç", role: .cancel) { }
            } message: {
                Text("sirr'in ürettiği ve sohbete eklediğin tüm dosyalar cihazdan silinir. Bu işlem geri alınamaz.")
            }
        }
    }

    private var belgeVarMi: Bool { belgeBoyutu > 0 }

    private var boyutMetni: String {
        belgeBoyutu > 0
            ? belgeBoyutu.formatted(.byteCount(style: .file))
            : String(localized: "Belge yok")
    }

    @MainActor
    private func belgeBoyutunuOlc() {
        guard belgeBaglami != nil else { return }
        belgeBoyutu = BelgeBaglami.ciktiBoyutu()
    }

    @MainActor
    private func belgeleriSil() {
        belgeBaglami?.ciktilariSil()
        belgeBoyutunuOlc()
    }

    // MARK: - Web araması (web-arama-spec §3.1, §4)

    /// Kullanıcının kendi SearXNG sunucusu. Uygulama hiçbir adres ön tanımlı
    /// getirmez; adres girilip DENENMEDEN kaydedilmez, kaydedilmeden arama açılmaz.
    private var webAramasiBolumu: some View {
        bolum("WEB ARAMASI") {
            VStack(alignment: .leading, spacing: Olcek.s3) {
                VStack(spacing: 0) {
                    adresAlani
                    ayrac
                    denemeSatiri
                    ayrac
                    acKapaSatiri
                    if kayitliGecerliMi {
                        ayrac
                        sunucuyuUnutSatiri
                    }
                }
                .cerceve()

                if let sonuc = aramaDurumMetni {
                    Text(sonuc)
                        .font(Yazi.cip())
                        .foregroundStyle(arama.hataMi ? Renk.hata : Renk.gri)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityLabel(Text(sonuc))
                }

                // Boş durum metni spec §4'te birebir yazılı.
                if !kayitliGecerliMi {
                    Text("Arama sunucusu yok. Kendi SearXNG sunucunu bağlarsan sirr web'de arayabilir. Bağlanmadıkça sirr internete çıkmaz.")
                        .font(Yazi.cip())
                        .foregroundStyle(Renk.soluk)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }

    private var adresAlani: some View {
        VStack(alignment: .leading, spacing: Olcek.s1) {
            Text("Sunucu adresi")
                .font(Yazi.kullanici())
                .foregroundStyle(Renk.murekkep)
            TextField("https://ornek.com/searxng/", text: $aramaURL)
                .font(Yazi.cip())
                .foregroundStyle(Renk.murekkep)
                .textFieldStyle(.plain)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .keyboardType(.URL)
                .submitLabel(.done)
                // Adres değişince önceki denemenin sonucu geçersizdir.
                .onChange(of: aramaURL) { _, yeni in
                    // Başarılı denemeden sonra adres normalize edilip geri
                    // yazılıyor; o değişimde sonucu silmemek için karşılaştırılır.
                    guard yeni.trimmingCharacters(in: .whitespacesAndNewlines) != kayitliAdres else { return }
                    aramaGorevi?.cancel()
                    arama = .bekliyor
                }
                .accessibilityLabel(Text("Arama sunucusu adresi"))
                .accessibilityHint(Text("Yalnızca https adresi. Düz http yalnızca yerel ağda kabul edilir."))
        }
        .padding(.vertical, Olcek.s3)
        .padding(.horizontal, Olcek.s4)
    }

    private var denemeSatiri: some View {
        Button { sunucuyuDene() } label: {
            HStack(spacing: Olcek.s2) {
                Text("Sunucuyu dene")
                    .font(Yazi.kullanici())
                    .foregroundStyle(denenebilirMi ? Renk.murekkep : Renk.soluk)
                Spacer(minLength: 0)
                Text(denemeYazisi)
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.gri)
            }
            .padding(.vertical, Olcek.s3)
            .padding(.horizontal, Olcek.s4)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!denenebilirMi)
        .accessibilityLabel(Text("Sunucuyu dene"))
        .accessibilityValue(Text(denemeYazisi))
        .accessibilityHint(Text("Sunucuya tek bir örnek sorgu gönderilir."))
    }

    private var acKapaSatiri: some View {
        Toggle(isOn: $aramaAcik) {
            Text("Aramayı aç")
                .font(Yazi.kullanici())
                .foregroundStyle(kayitliGecerliMi ? Renk.murekkep : Renk.soluk)
        }
        // Vurgu rengi yok: anahtar da mürekkep tonunda.
        .tint(Renk.murekkep)
        .disabled(!kayitliGecerliMi)
        .padding(.vertical, Olcek.s3)
        .padding(.horizontal, Olcek.s4)
        .accessibilityHint(Text(kayitliGecerliMi
                                ? String(localized: "Kapalıyken sirr aramayı hiç kullanmaz.")
                                : String(localized: "Önce bir sunucu adresi denenmeli.")))
    }

    private var sunucuyuUnutSatiri: some View {
        Button { sunucuyuUnut() } label: {
            HStack(spacing: Olcek.s2) {
                Text("Sunucuyu kaldır")
                    .font(Yazi.kullanici())
                    .foregroundStyle(Renk.murekkep)
                Spacer(minLength: 0)
                Image(systemName: "chevron.right")
                    .font(Yazi.ikonKucuk())
                    .foregroundStyle(Renk.soluk)
                    .accessibilityHidden(true)
            }
            .padding(.vertical, Olcek.s3)
            .padding(.horizontal, Olcek.s4)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text("Sunucuyu kaldır"))
        .accessibilityHint(Text("Adres silinir ve arama kapanır."))
    }

    /// Kaydedilmiş (denenmiş) adres geçerli mi — aç/kapa buna bağlı.
    private var kayitliGecerliMi: Bool { WebAramaAyari.dogrula(kayitliAdres) != nil }

    private var denenebilirMi: Bool {
        if case .deneniyor = arama { return false }
        return !aramaURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var denemeYazisi: String {
        switch arama {
        case .deneniyor: return String(localized: "deneniyor")
        case .basarili: return String(localized: "çalışıyor")
        case .basarisiz: return String(localized: "başarısız")
        case .bekliyor: return kayitliGecerliMi
            && kayitliAdres == aramaURL.trimmingCharacters(in: .whitespacesAndNewlines)
            ? String(localized: "kayıtlı")
            : ""
        }
    }

    /// Deneme sonucunun tam cümlesi. Başarısızlıkta neden düz dille yazılır.
    private var aramaDurumMetni: String? {
        switch arama {
        case .bekliyor, .deneniyor:
            return nil
        case .basarili(let sayi):
            return String(localized: "arama çalışıyor — örnek sorgu \(sayi) sonuç döndürdü. Adres kaydedildi.")
        case .basarisiz(let neden):
            return neden
        }
    }

    /// `GET {url}/search?q=test&format=json`. Başarıda adres KAYDEDİLİR.
    private func sunucuyuDene() {
        let ham = aramaURL.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let kok = WebAramaAyari.dogrula(ham) else {
            arama = .basarisiz(String(localized: "Adres okunamadı. https:// ile başlamalı; düz http yalnızca kendi yerel ağındaki adreslerde kabul edilir."))
            return
        }
        aramaGorevi?.cancel()
        arama = .deneniyor
        aramaGorevi = Task { @MainActor in
            do {
                let (sonuclar, _) = try await WebAramaIstemcisi.ara("test", kok: kok)
                guard !Task.isCancelled else { return }
                // Adres yalnızca çalıştığı görüldükten sonra kalıcılaşır.
                WebAramaAyari.kokHam = ham
                kayitliAdres = ham
                aramaURL = ham
                arama = .basarili(sonuclar.count)
            } catch {
                guard !Task.isCancelled else { return }
                arama = .basarisiz(Self.aramaHataCumlesi(error))
            }
        }
    }

    /// Adresi ve aç/kapa bayrağını temizler; ağ yüzeyi yeniden kapanır.
    private func sunucuyuUnut() {
        aramaGorevi?.cancel()
        aramaGorevi = nil
        aramaAcik = false
        WebAramaAyari.kokHam = ""
        kayitliAdres = ""
        aramaURL = ""
        arama = .bekliyor
    }

    /// Hata → kullanıcı cümlesi. JSON tuzağı SearXNG'ye özgüdür ve açıkça söylenir.
    static func aramaHataCumlesi(_ hata: Error) -> String {
        // Beklenmedik hata türü de sessizce yutulmaz, düz cümleyle çıkar.
        guard let bilinen = hata as? WebAramaHatasi else {
            return String(localized: "Deneme tamamlanamadı: \(hata.localizedDescription)")
        }
        switch bilinen {
        case .bicimAnlasilmadi:
            return String(localized: "Sunucu yanıt verdi ama JSON döndürmedi. Sunucunun settings.yml dosyasında formats: json açık olmalı.")
        case .sunucuHatasi(let kod):
            return String(localized: "Sunucu \(kod) döndürdü. Adres doğru olsa da arama bu yoldan açılmıyor olabilir.")
        case .ulasilamadi:
            return String(localized: "Sunucuya ulaşılamadı. Adres bulunamadı ya da yanıt zamanında gelmedi.")
        case .sunucuYok:
            return String(localized: "Adres okunamadı.")
        }
    }

    // MARK: - Bağlantılar (mcp §3.1)

    private var baglantilarBolumu: some View {
        bolum("BAĞLANTILAR") {
            VStack(alignment: .leading, spacing: Olcek.s3) {
                Button { baglantilarAc() } label: {
                    HStack(spacing: Olcek.s2) {
                        Text("Bağlantılar")
                            .font(Yazi.kullanici())
                            .foregroundStyle(Renk.murekkep)
                        Spacer(minLength: 0)
                        Image(systemName: "chevron.right")
                            .font(Yazi.ikonKucuk())
                            .foregroundStyle(Renk.soluk)
                            .accessibilityHidden(true)
                    }
                    .padding(.vertical, Olcek.s3)
                    .padding(.horizontal, Olcek.s4)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(Text("Bağlantılar"))
                .accessibilityHint(Text("Kendi sunucularını ekle ve yönet."))
                .cerceve()

                Text("Kendi MCP sunucularını bağlarsan sirr onların araçlarını kullanabilir. Bağlamadıkça hiçbir sunucuya bir şey gitmez.")
                    .font(Yazi.cip())
                    .foregroundStyle(Renk.soluk)
                    .fixedSize(horizontal: false, vertical: true)
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
