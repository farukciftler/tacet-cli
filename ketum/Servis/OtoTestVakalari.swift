//
//  OtoTestVakalari.swift
//  ketum
//
//  Dört spec'in MODEL VE AĞ GEREKTİRMEYEN kabul ölçütleri (hafiza §8,
//  seyir §6, web-arama §6, mcp kapısı). Hepsi saf fonksiyon ya da bellek içi
//  nesne üzerinde çalışır: cihazda model olmasa, ağ kapalı olsa da geçerler.
//
//  Bu dosya "gözle bak" çıktısı ÜRETMEZ. Her satır bir iddiadır; iddia
//  tutmazsa satır "✗ BAŞARISIZ" ile işaretlenir ve sonda toplam hata sayısı
//  raporlanır. OtoTest.calistir() bunu çağırır, `--ototest` argümanıyla açılır.
//

#if DEBUG
import Foundation

// MARK: - Küçük iddia defteri

/// Assert defteri: satırları biriktirir, başarısızlıkları sayar.
struct OtoTestDefteri {
    private(set) var satirlar: [String] = []
    private(set) var hata = 0
    private(set) var iddia = 0

    /// Ham satır (bölüm başlığı vb.).
    mutating func satir(_ metin: String) {
        satirlar.append(metin)
    }

    mutating func baslik(_ metin: String) {
        satirlar.append("--- \(metin) ---")
    }

    mutating func not(_ metin: String) {
        satirlar.append("    \(metin)")
    }

    /// Tek iddia. `kosul` false ise hata sayacı artar ve satır işaretlenir.
    mutating func dogru(_ kosul: Bool, _ ad: String, _ detay: @autoclosure () -> String = "") {
        iddia += 1
        if kosul {
            satirlar.append("  ✓ \(ad)")
        } else {
            hata += 1
            let ek = detay()
            satirlar.append("  ✗ BAŞARISIZ \(ad)\(ek.isEmpty ? "" : " — \(ek)")")
        }
    }

    /// Eşitlik iddiası; eşit değilse iki değeri de yazar.
    mutating func esit<T: Equatable>(_ bulunan: T, _ beklenen: T, _ ad: String) {
        dogru(bulunan == beklenen, ad, "bulunan=\(bulunan) beklenen=\(beklenen)")
    }

    mutating func ekle(_ oteki: OtoTestDefteri) {
        satirlar.append(contentsOf: oteki.satirlar)
        hata += oteki.hata
        iddia += oteki.iddia
    }
}

// MARK: - Vakalar

enum OtoTestVakalari {

    /// Senkron çalışan tüm vakalar. Model ve ağ gerekmez.
    @MainActor
    static func calistir() -> OtoTestDefteri {
        var d = OtoTestDefteri()
        d.satir("=== SPEC VAKALARI (model/ağ gerekmez) ===")
        hafizaFiltreleri(&d)
        hafizaEslesme(&d)
        hafizaEnjeksiyon(&d)
        seyirKaydedici(&d)
        seyirKodlama(&d)
        seyirKatlama(&d)
        dosyaIkonu(&d)
        webAyristirma(&d)
        webButce(&d)
        agTekeli(&d)
        return d
    }

    /// Askıya alma gerektiren vakalar (onay kapısı). OtoTest ayrı bir Task'ta çağırır.
    @MainActor
    static func asenkronCalistir() async -> OtoTestDefteri {
        var d = OtoTestDefteri()
        d.satir("=== ASENKRON VAKALAR ===")
        await onayKapisi(&d)
        return d
    }

    // MARK: - hafiza-spec §8: filtreler (§4.3)

    @MainActor
    private static func hafizaFiltreleri(_ d: inout OtoTestDefteri) {
        d.baslik("HAFIZA · AYIKLAMA FİLTRELERİ (§4.3)")

        func aday(_ tur: String, _ metin: String, _ anahtarlar: [String]) -> AyiklananNot {
            AyiklananNot(tur: tur, metin: metin, anahtarlar: anahtarlar)
        }

        // 1. Kısa metin (10 karakterden az) düşer.
        d.esit(HafizaServisi.suz([aday("olgu", "kısa", ["a"])],
                                 varolanMetinler: [], kayitliSayi: 0).count,
               0, "10 karakterden kısa metin reddedilir")

        // 1b. Sınırı aşan metin (160+) düşer; tam sınırdaki metin geçer.
        let tamSinir = String(repeating: "a", count: HafizaNotu.metinSiniri)
        let asan = String(repeating: "a", count: HafizaNotu.metinSiniri + 1)
        d.esit(HafizaServisi.suz([aday("olgu", asan, ["a"])],
                                 varolanMetinler: [], kayitliSayi: 0).count,
               0, "160 karakteri aşan metin reddedilir")
        d.esit(HafizaServisi.suz([aday("olgu", tamSinir, ["a"])],
                                 varolanMetinler: [], kayitliSayi: 0).count,
               1, "tam 160 karakterlik metin kabul edilir")

        // 2. Geçersiz tür düşer — varsayılana DÜŞÜRÜLMEZ.
        d.esit(HafizaServisi.suz([aday("şarkıcı", "Kullanıcı İzmir'de yaşıyor.", ["izmir"])],
                                 varolanMetinler: [], kayitliSayi: 0).count,
               0, "geçersiz tür reddedilir (olgu'ya düşürülmez)")
        // Türde büyük harf / boşluk toleransı olmalı.
        d.esit(HafizaServisi.suz([aday(" Tercih ", "Kullanıcı sabah kahve içer.", ["kahve"])],
                                 varolanMetinler: [], kayitliSayi: 0).first?.tur,
               .tercih, "tür kırpılıp küçültülerek okunur")

        // 3. Anahtarsız not düşer (boş dizi ve yalnızca boşluktan oluşan anahtar).
        d.esit(HafizaServisi.suz([aday("olgu", "Kullanıcı İzmir'de yaşıyor.", [])],
                                 varolanMetinler: [], kayitliSayi: 0).count,
               0, "anahtarsız not reddedilir")
        d.esit(HafizaServisi.suz([aday("olgu", "Kullanıcı İzmir'de yaşıyor.", ["  ", ""])],
                                 varolanMetinler: [], kayitliSayi: 0).count,
               0, "yalnızca boşluk olan anahtarlar reddedilir")

        // 4. Tekilleştirme: kayıtlı metinle aynı olan düşer (büyük/küçük harf duyarsız).
        let mevcutMetin = "kullanıcı i̇zmir'de yaşıyor."
        d.esit(HafizaServisi.suz([aday("olgu", "Kullanıcı İzmir'de yaşıyor.", ["izmir"])],
                                 varolanMetinler: [mevcutMetin.lowercased()],
                                 kayitliSayi: 1).count,
               0, "kayıtlı notun tekrarı reddedilir")
        // 4b. Aynı çağrı içindeki tekrar da düşer.
        let ayni = HafizaServisi.suz([aday("olgu", "Kullanıcı kedi besliyor.", ["kedi"]),
                                      aday("olgu", "kullanıcı kedi besliyor.", ["kedi"])],
                                     varolanMetinler: [], kayitliSayi: 0)
        d.esit(ayni.count, 1, "aynı çağrıdaki tekrar tekilleştirilir")

        // 5. Tavan: 50 kayıt varken hiçbir not kabul edilmez.
        d.esit(HafizaServisi.suz([aday("olgu", "Kullanıcı kedi besliyor.", ["kedi"])],
                                 varolanMetinler: [],
                                 kayitliSayi: HafizaNotu.toplamTavan).count,
               0, "50 tavanı doluyken not kabul edilmez")
        // 5b. Tavanın bir altında yalnız bir not sığar.
        d.esit(HafizaServisi.suz([aday("olgu", "Kullanıcı kedi besliyor.", ["kedi"]),
                                  aday("olgu", "Kullanıcı köpek besliyor.", ["köpek"])],
                                 varolanMetinler: [],
                                 kayitliSayi: HafizaNotu.toplamTavan - 1).count,
               1, "tavana bir kala tek not sığar")

        // Çağrı başına 2 not tavanı (şemadaki "en fazla 2"nin koddaki karşılığı).
        let ucAday = [aday("olgu", "Kullanıcı kedi besliyor.", ["kedi"]),
                      aday("olgu", "Kullanıcı köpek besliyor.", ["köpek"]),
                      aday("olgu", "Kullanıcı kuş besliyor.", ["kuş"])]
        d.esit(HafizaServisi.suz(ucAday, varolanMetinler: [], kayitliSayi: 0).count,
               2, "çağrı başına en fazla 2 not")

        // Anahtar sayısı üst sınırı zorlanır.
        let cokAnahtar = (1...20).map { "anahtar\($0)" }
        d.esit(HafizaServisi.suz([aday("olgu", "Kullanıcı kedi besliyor.", cokAnahtar)],
                                 varolanMetinler: [], kayitliSayi: 0).first?.anahtarlar.count,
               HafizaNotu.anahtarSiniri, "anahtar sayısı 8'de kesilir")

        // İstem gövdesi: son mesajlar korunur, bütçe aşılmaz.
        let uzunMesajlar = (1...50).map { "mesaj \($0) " + String(repeating: "x", count: 100) }
        let govde = HafizaServisi.istemGovdesi(uzunMesajlar)
        d.dogru(govde.count <= 1800, "istem gövdesi 1800 karakteri aşmaz", "\(govde.count)")
        d.dogru(govde.contains("mesaj 50"), "istem gövdesinde SON mesaj korunur")
        d.esit(HafizaServisi.istemGovdesi(["  ", ""]), "", "boş mesajlardan boş gövde çıkar")
    }

    // MARK: - hafiza-spec §8: eşleşme (§5)

    @MainActor
    private static func hafizaEslesme(_ d: inout OtoTestDefteri) {
        d.baslik("HAFIZA · EŞLEŞME (§5)")

        // Notlar BİR ModelContext'e EKLENMEZ: eşleşme ve enjeksiyon saf
        // fonksiyonlardır, kalıcılığa ihtiyaç duymazlar. Test uygulamanın
        // gerçek mağazasına da ayrı bir kaba da hiçbir şey yazmaz.

        func not(_ metin: String, _ anahtarlar: String, aktif: Bool = true,
                 yas: TimeInterval = 0) -> HafizaNotu {
            let n = HafizaNotu(metin: metin, tur: .olgu, anahtarlarHam: anahtarlar)
            n.aktif = aktif
            n.olusturulma = Date().addingTimeInterval(-yas)
            return n
        }

        // Özgüllük: puan anahtarların UZUNLUK TOPLAMI — özgül ifade genel kelimeyi yener.
        let ozgul = not("Kullanıcı akşam yemeğini geç yer.", "akşam yemeği", yas: 100)
        let genel = not("Kullanıcı yemek konusunda seçicidir.", "yemek", yas: 50)
        HafizaDeposu.yenile([ozgul, genel])
        let sonuc = HafizaDeposu.eslesen(soru: "akşam yemeği için yemek önerir misin")
        d.esit(sonuc.count, 2, "iki not da eşleşir")
        d.esit(sonuc.first?.id, ozgul.id, "özgül ifade genel kelimeyi yener")

        // Hiç eşleşme yoksa boş dizi.
        d.esit(HafizaDeposu.eslesen(soru: "bugün hava nasıl").count, 0,
               "eşleşme yoksa boş dizi döner")

        // Kapalı not enjeksiyona hiç girmez.
        let kapali = not("Kullanıcı vejetaryen beslenir.", "yemek, beslenme", aktif: false)
        HafizaDeposu.yenile([ozgul, genel, kapali])
        d.dogru(!HafizaDeposu.eslesen(soru: "yemek önerir misin").contains { $0.id == kapali.id },
                "kapalı not eşleşmeden düşer")

        // Geçersiz not (anahtarsız) depoya alınmaz.
        let anahtarsiz = not("Kullanıcı bir şey söyledi burada.", "")
        HafizaDeposu.yenile([ozgul, anahtarsiz])
        d.esit(HafizaDeposu.notlar.count, 1, "geçersiz not depoya alınmaz")

        // 3 not tavanı: 5 eşleşen nottan yalnızca 3'ü döner.
        let besli = (1...5).map { i in
            not("Kullanıcı hakkında \(i) numaralı olgu buraya yazıldı.", "ortak", yas: TimeInterval(i))
        }
        HafizaDeposu.yenile(besli)
        let tavan = HafizaDeposu.eslesen(soru: "ortak bir soru")
        d.esit(tavan.count, HafizaDeposu.enFazlaNot, "en fazla 3 not döner")
        // Eşit puanda YENİ not kazanır (yaş küçük olan en yeni).
        d.esit(tavan.first?.id, besli[0].id, "eşit puanda en yeni not öne geçer")

        HafizaDeposu.yenile([])
    }

    // MARK: - hafiza-spec §8: enjeksiyon bütçesi (§5.1)

    @MainActor
    private static func hafizaEnjeksiyon(_ d: inout OtoTestDefteri) {
        d.baslik("HAFIZA · ENJEKSİYON BÜTÇESİ (§5.1)")

        // En kötü durum: sınır uzunluğunda üç not.
        let uzunlar: [HafizaNotu] = (1...3).map { i in
            let govde = "Not \(i): " + String(repeating: "ç", count: HafizaNotu.metinSiniri - 8)
            let n = HafizaNotu(metin: String(govde.prefix(HafizaNotu.metinSiniri)),
                               tur: .olgu, anahtarlarHam: "ortak")
            return n
        }
        let metin = HafizaDeposu.enjeksiyonMetni(uzunlar)
        d.dogru(metin.count <= HafizaDeposu.enjeksiyonSiniri,
                "hafıza enjeksiyonu 600 karakteri aşmaz (çit dahil)", "\(metin.count)")
        d.dogru(metin.contains("<memory>") && metin.contains("</memory>"),
                "enjeksiyon <memory> bloğuyla çitlenir")

        // Sığmayan not KESİLMEZ, ELENİR: her satır tam nottur.
        let satirlar = metin
            .split(separator: "\n")
            .filter { $0.hasPrefix("- ") }
            .map { String($0.dropFirst(2)) }
        d.dogru(!satirlar.isEmpty, "en az bir not sığar")
        let hepsiTam = satirlar.allSatisfy { satir in
            uzunlar.contains { $0.metin == satir }
        }
        d.dogru(hepsiTam, "sığmayan not kesilmez, tamamen elenir")
        d.dogru(satirlar.count < uzunlar.count,
                "bütçeye sığmayan not enjeksiyona alınmaz", "\(satirlar.count)/3")

        // Boş liste hiçbir şey eklemez (çit tek başına gitmez).
        d.esit(HafizaDeposu.enjeksiyonMetni([]), "", "not yoksa enjeksiyon boştur")

        // EN KÖTÜ DURUM TOPLAMI: beceri (700 + çit) + hafıza (600) aynı tura düşebilir.
        let beceriEnKotu = BeceriDeposu.paket
            .map { BeceriDeposu.enjeksiyonMetni($0).count }
            .max() ?? 0
        let toplam = beceriEnKotu + metin.count
        d.dogru(toplam <= 1600,
                "beceri + hafıza en kötü toplamı ~1500 karakter tavanında",
                "beceri=\(beceriEnKotu) hafıza=\(metin.count) toplam=\(toplam)")
    }

    // MARK: - seyir-spec §6: kaydedici

    @MainActor
    private static func seyirKaydedici(_ d: inout OtoTestDefteri) {
        d.baslik("SEYİR · KAYDEDİCİ (§5.2)")

        let k = SeyirKaydedici()
        k.basla(tur: .yonlendirme, metin: "yönlendirildi · takvim profili")
        k.basla(tur: .zenginlestirme, metin: "beceri eklendi · takvim")
        d.esit(k.adimlar.count, 2, "ardışık iki adım kaydedildi")
        d.dogru(k.adimlar[0].bittiMi, "yeni adım açılınca önceki KAPANIR")
        d.dogru(!k.adimlar[1].bittiMi, "son adım açık kalır")

        // Araç adımı: metin izden okunur, adımda BOŞ durur.
        let izID = UUID()
        k.basla(tur: .arac, metin: "bu metin yok sayılmalı")
        k.aracBagla(izID: izID)
        d.esit(k.adimlar.count, 3, "araç adımı açık adıma bağlanır, yeni adım açmaz")
        d.esit(k.adimlar[2].aracIziID, izID, "araç adımı ize bağlandı")
        d.esit(k.adimlar[2].metin, "", "araç adımının metni boştur (tek doğruluk kaynağı AracIzi)")

        // Bağlı adım varken ikinci bağlama YENİ adım açar.
        k.aracBagla(izID: UUID())
        d.esit(k.adimlar.count, 4, "ikinci araç için yeni adım açılır")

        k.basla(tur: .yazim, metin: "yazıyor")
        k.bitir()
        d.dogru(!k.suruyorMu, "bitir() kaydediciyi kapatır")
        d.dogru(k.adimlar.allSatisfy { $0.bittiMi }, "bitir() sonrası açık adım kalmaz")
        d.dogru(k.adimlar.allSatisfy { ($0.sure ?? 0) >= 0 }, "hiçbir süre negatif değil")

        // Kapandıktan sonra yazma yok.
        let sayi = k.adimlar.count
        k.basla(tur: .yazim, metin: "geç kalan")
        d.esit(k.adimlar.count, sayi, "kapalı kaydediciye adım eklenmez")

        // kes(): açık adım varken son adım kesinti olur.
        let k2 = SeyirKaydedici()
        k2.basla(tur: .yonlendirme, metin: "yönlendirildi · gündelik profil")
        k2.kes()
        d.esit(k2.adimlar.last?.tur, .kesinti, "kes() sona kesinti adımı ekler")
        d.dogru(k2.adimlar.allSatisfy { $0.bittiMi },
                "kes() sonrası bitis == nil kalan adım YOKTUR")
        d.dogru(!k2.suruyorMu, "kes() kaydediciyi kapatır")

        // Hiç adım yokken kes(): yine de kesinti kaydı düşer (sessiz kaybolma yok).
        let k3 = SeyirKaydedici()
        k3.kes()
        d.esit(k3.adimlar.count, 1, "boş turda da kesinti kaydı düşer")

        // Süre asla negatif olamaz (saat geri alınsa bile).
        let an = Date()
        let tersAdim = SeyirAdimi(tur: .yazim, metin: "x",
                                  baslangic: an, bitis: an.addingTimeInterval(-5))
        d.esit(tersAdim.sure, 0, "ters saatte süre sıfıra kırpılır")
        d.esit(SeyirAdimi(tur: .yazim, metin: "x").sure, nil, "süren adımda süre nil")

        // sifirla() yeni tur için temizler.
        k2.sifirla()
        d.dogru(k2.adimlar.isEmpty && k2.suruyorMu, "sifirla() yeni tura hazırlar")
    }

    // MARK: - seyir-spec §6: kodlama / kalıcılık

    @MainActor
    private static func seyirKodlama(_ d: inout OtoTestDefteri) {
        d.baslik("SEYİR · KODLAMA (§5.1)")

        let izID = UUID()
        let adimlar: [SeyirAdimi] = [
            SeyirAdimi(tur: .yonlendirme, metin: "yönlendirildi · takvim profili",
                       baslangic: Date(), bitis: Date().addingTimeInterval(0.2)),
            SeyirAdimi(tur: .arac, aracIziID: izID,
                       baslangic: Date(), bitis: Date().addingTimeInterval(1.1)),
            SeyirAdimi(tur: .yazim, metin: "yazıldı",
                       baslangic: Date(), bitis: Date().addingTimeInterval(3))
        ]

        let mesaj = Mesaj(rol: .ketum, icerik: "yanıt", adimlar: adimlar)
        let geri = mesaj.adimlar
        d.esit(geri.count, 3, "adımlar mesaja yazılıp geri okundu")
        d.esit(geri.map(\.id), adimlar.map(\.id), "adım kimlikleri korunur")
        d.esit(geri.map(\.tur), adimlar.map(\.tur), "adım türleri korunur")
        d.esit(geri[1].aracIziID, izID, "araç izi bağı korunur")
        d.dogru(geri.allSatisfy { $0.bittiMi }, "bitiş tarihleri korunur")

        // Eski mesaj (adimlarVeri == nil) BOŞ LİSTE döner — geriye dönük dolgu yok.
        let eski = Mesaj(rol: .ketum, icerik: "eski yanıt")
        d.esit(eski.adimlar.count, 0, "adım verisi olmayan eski mesaj boş liste döner")
        d.dogru(!SeyirKatlama.satirGosterilirMi(eski.adimlar),
                "eski mesajda seyir satırı çizilmez")

        // Boş liste yazmak da "seyir yok" ile aynıdır.
        let bosla = Mesaj(rol: .ketum, icerik: "y", adimlar: [])
        d.esit(bosla.adimlar.count, 0, "boş adım listesi seyirsiz sayılır")

        // Setter yolu da çalışmalı (kaydedici.yaz bunu kullanır).
        let sonradan = Mesaj(rol: .ketum, icerik: "y")
        sonradan.adimlar = adimlar
        d.esit(sonradan.adimlar.count, 3, "adımlar sonradan da yazılabilir")
    }

    // MARK: - seyir-spec §6: katlama kuralı (saf fonksiyon)

    @MainActor
    private static func seyirKatlama(_ d: inout OtoTestDefteri) {
        d.baslik("SEYİR · KATLAMA KURALI (§2.2, §3.2)")

        // Yalnız-yazım turunda satır ÜRETİLMEZ — Seyir susar.
        let yazimTek = [SeyirAdimi(tur: .yazim, metin: "yazıldı")]
        d.dogru(!SeyirKatlama.satirGosterilirMi(yazimTek),
                "araçsız (yalnız yazım) turda katlama satırı çizilmez")
        d.dogru(!SeyirKatlama.satirGosterilirMi([]), "adım yoksa satır çizilmez")
        d.dogru(SeyirKatlama.satirGosterilirMi([
            SeyirAdimi(tur: .yonlendirme, metin: "yönlendirildi · takvim profili"),
            SeyirAdimi(tur: .yazim, metin: "yazıldı")
        ]), "iki adımlı turda satır çizilir")
        // Tek adım yazım DEĞİLSE satır çizilir (kesinti gizlenmez).
        d.dogru(SeyirKatlama.satirGosterilirMi([SeyirAdimi(tur: .kesinti, metin: "yarıda kaldı")]),
                "tek kesinti adımı da gösterilir")

        // Yan etki ve hata izleri katlamanın DIŞINDA kalır.
        let okundu = AracIzi(ikon: "calendar", metin: "takvim okundu", durum: .okundu)
        let yazildi = AracIzi(ikon: "calendar", metin: "etkinlik yazıldı", durum: .yazildi)
        let basarisiz = AracIzi(ikon: "x", metin: "arama başarısız", durum: .basarisiz("ulaşılamadı"))
        let izin = AracIzi(ikon: "lock", metin: "takvim izni gerekli", durum: .izinGerekli)
        let onay = AracIzi(ikon: "hand.raised", metin: "ev · onay bekleniyor", durum: .onayBekleniyor)
        let ret = AracIzi(ikon: "nosign", metin: "ev · gönderilmedi", durum: .gonderilmedi)
        let calisiyor = AracIzi(ikon: "gear", metin: "çalışıyor", durum: .calisiyor)
        let hepsi = [okundu, yazildi, basarisiz, izin, onay, ret, calisiyor]

        d.esit(SeyirKatlama.katlamaIci(hepsi).map(\.id), [okundu.id],
               "yalnızca okuma izi katlanır")
        d.esit(SeyirKatlama.katlamaDisi(hepsi).count, 6,
               "yazildi/basarisiz/izin/onay/ret/çalışıyor katlamanın dışındadır")
        d.dogru(SeyirKatlama.katlamaDisi(hepsi).contains { $0.id == yazildi.id },
                "yazildi izi asla gizlenmez")
        d.dogru(SeyirKatlama.katlamaDisi(hepsi).contains { $0.id == basarisiz.id },
                "basarisiz izi asla gizlenmez")

        // Çip/kart ayrımı (§9.4).
        let dosyali = AracIzi(ikon: "doc", metin: "excel yazıldı", durum: .yazildi,
                              dosyaYolu: "/tmp/x.xlsx")
        let dosyaliHatali = AracIzi(ikon: "doc", metin: "excel başarısız",
                                    durum: .basarisiz("yazılamadı"), dosyaYolu: "/tmp/y.xlsx")
        d.dogru(YanitIzleri.kartlikMi(dosyali), "dosya üreten iz kart olur")
        d.dogru(!YanitIzleri.kartlikMi(dosyaliHatali), "başarısız iz kart olmaz")
        d.esit(YanitIzleri.kartlar([dosyali, dosyaliHatali, okundu]).map(\.id), [dosyali.id],
               "yalnızca başarılı dosya izi kart listesine girer")

        // Eski mesaj (seyirVar == false): çipler bugünkü gibi tümü görünür.
        d.esit(YanitIzleri.cipler(hepsi + [dosyali], seyirVar: false).count, 7,
               "adım verisi yoksa geriye dönük katlama yapılmaz")
        d.esit(YanitIzleri.cipler(hepsi + [dosyali], seyirVar: true).count, 6,
               "seyir varken okuma çipi katlanır, kart çıkarılır")

        // Canlı blok: şerit varken çalışıyor çipi çizilmez.
        d.dogru(!YanitIzleri.canliCipler(hepsi, seritVar: true).contains { $0.id == calisiyor.id },
                "şerit varken 'çalışıyor' çipi ikinci kez çizilmez")
        d.dogru(YanitIzleri.canliCipler(hepsi, seritVar: false).contains { $0.id == calisiyor.id },
                "şerit yokken 'çalışıyor' çipi görünür")

        // Özet metni: başarısız varsa süre değil hata sayısı yazılır.
        let adimlar = [SeyirAdimi(tur: .yonlendirme, metin: "y",
                                  baslangic: Date(), bitis: Date().addingTimeInterval(1)),
                       SeyirAdimi(tur: .arac, aracIziID: basarisiz.id,
                                  baslangic: Date(), bitis: Date().addingTimeInterval(1))]
        d.dogru(SeyirKatlama.ozetMetni(adimlar: adimlar, izler: [basarisiz]).contains("1"),
                "özet metni aşılamayan adım sayısını taşır")
        d.dogru(SeyirKatlama.toplamSure(adimlar) >= 0, "toplam süre negatif olamaz")
        // Süren adım toplama katılmaz (yalan ilerleme yok).
        d.esit(SeyirKatlama.toplamSure([SeyirAdimi(tur: .yazim, metin: "x")]), 0,
               "süren adım toplam süreye katılmaz")

        // Satır metni araç adımında İZDEN okunur.
        let aracAdimi = SeyirAdimi(tur: .arac, aracIziID: okundu.id)
        d.esit(SeyirMetni.satir(aracAdimi, izler: [okundu]), okundu.metin,
               "araç adımının metni AracIzi'den gelir")
    }

    // MARK: - seyir-spec §9.3: dosya ikonu eşlemesi

    @MainActor
    private static func dosyaIkonu(_ d: inout OtoTestDefteri) {
        d.baslik("SEYİR · DOSYA İKONU EŞLEMESİ (§9.3)")

        d.esit(DosyaIkonu.bilinenTipler.count, 20, "set tam 20 tip içerir")
        let kendine = DosyaIkonu.bilinenTipler.allSatisfy { DosyaIkonu.tip(uzanti: $0) == $0 }
        d.dogru(kendine, "20 tipin her biri kendine eşlenir")
        let tekil = Set(DosyaIkonu.bilinenTipler).count == DosyaIkonu.bilinenTipler.count
        d.dogru(tekil, "set içinde yinelenen tip yok")

        // Eş anlamlılar.
        let esler: [(String, String)] = [
            ("jpeg", "jpg"), ("jpe", "jpg"),
            ("markdown", "md"), ("mdown", "md"), ("mkd", "md"),
            ("text", "txt"), ("heif", "heic"), ("tsv", "csv"),
            ("xls", "xlsx"), ("doc", "docx"), ("ppt", "pptx"),
            ("m4v", "mp4"), ("qt", "mov"), ("wave", "wav"),
            ("aac", "m4a"), ("zipx", "zip")
        ]
        for (giren, beklenen) in esler {
            d.esit(DosyaIkonu.tip(uzanti: giren), beklenen, "eş anlamlı \(giren) → \(beklenen)")
        }

        // Büyük/küçük harf ve biçim duyarsızlığı.
        d.esit(DosyaIkonu.tip(uzanti: "JPEG"), "jpg", "büyük harf eş anlamlı çözülür")
        d.esit(DosyaIkonu.tip(uzanti: ".PNG"), "png", "baştaki nokta düşer")
        d.esit(DosyaIkonu.tip(uzanti: "  PdF  "), "pdf", "boşluk kırpılır")
        d.esit(DosyaIkonu.tip(uzanti: "rapor.XLSX"), "xlsx", "tam dosya adından uzantı alınır")
        d.esit(DosyaIkonu.tip(uzanti: "arsiv.tar.GZ"), DosyaIkonu.jenerikTip,
               "çok noktalı bilinmeyen uzantı jeneriğe düşer")

        // Geri düşüş: kart asla ikonsuz çizilmez.
        d.esit(DosyaIkonu.tip(uzanti: "qwerty"), DosyaIkonu.jenerikTip, "bilinmeyen uzantı jeneriğe düşer")
        d.esit(DosyaIkonu.tip(uzanti: ""), DosyaIkonu.jenerikTip, "boş uzantı jeneriğe düşer")
        d.esit(DosyaIkonu.varlikAdi(uzanti: "qwerty"), "dosya-belge", "jenerik varlık adı doğru")
        d.esit(DosyaIkonu.varlikAdi(uzanti: "jpeg"), "dosya-jpg", "eş anlamlı varlık adına yansır")

        // Tür etiketi: boş uzantı boş etiket, bilinen uzantı boş olmayan etiket.
        d.esit(DosyaIkonu.turEtiketi(uzanti: ""), "", "boş uzantıda etiket yok")
        let etiketliler = ["pdf", "xlsx", "png", "qwerty"]
        let hepsiDolu = etiketliler.allSatisfy { !DosyaIkonu.turEtiketi(uzanti: $0).isEmpty }
        d.dogru(hepsiDolu, "her uzantı için tür etiketi üretilir")
        let ilkHarf = DosyaIkonu.turEtiketi(uzanti: "pdf").first
        d.dogru(ilkHarf.map { !$0.isLowercase } ?? false,
                "tür etiketi büyük harfle başlar", DosyaIkonu.turEtiketi(uzanti: "pdf"))
        d.esit(DosyaIkonu.turEtiketi(uzanti: "qwerty"), "QWERTY",
               "sistem çözemezse uzantı büyük harfle yazılır")
    }

    // MARK: - web-arama-spec §6: ayrıştırma

    @MainActor
    private static func webAyristirma(_ d: inout OtoTestDefteri) {
        d.baslik("WEB ARAMA · AYRIŞTIRMA (§5.3)")

        guard let veri = fixtureJSON().data(using: .utf8) else {
            d.dogru(false, "fixture JSON kodlandı")
            return
        }

        do {
            let sonuclar = try WebAramaIstemcisi.ayristir(veri)
            d.esit(sonuclar.count, WebAramaIstemcisi.sonucTavani,
                   "7 sonuçlu yanıt 5 sonuç tavanına kırpılır")
            d.dogru(sonuclar.first?.bilgiKutusuMu == true, "bilgi kutusu ilk sırada")
            d.dogru(sonuclar.dropFirst().allSatisfy { !$0.bilgiKutusuMu },
                    "yalnızca bir bilgi kutusu alınır")
            d.esit(sonuclar.first?.alanAdi, "www.mgm.gov.tr",
                   "bilgi kutusunun adresi alan adına indirgenir")
            d.esit(sonuclar[1].alanAdi, "tr.wikipedia.org",
                   "sonuç adresi alan adına indirgenir (yol ve sorgu düşer)")
            d.dogru(sonuclar[1].tamAdres.contains("/wiki/"),
                    "tam adres sonuçta korunur (çip detayı için)")
            d.dogru(sonuclar.allSatisfy { $0.ozet.count <= WebAramaIstemcisi.ozetTavani + 1 },
                    "her özet 200 karakter tavanında")
            d.dogru(sonuclar.allSatisfy { !$0.ozet.contains("\n") },
                    "özetlerde satır sonu kalmaz")
            // Başlıksız ve adressiz öge atlanır.
            d.dogru(!sonuclar.contains { $0.baslik.isEmpty && $0.tamAdres.isEmpty },
                    "başlıksız ve adressiz öge atlanır")
        } catch {
            d.dogru(false, "geçerli fixture ayrıştırıldı", "\(error)")
        }

        // BOZUK JSON → hata yolu.
        for bozuk in ["<html><body>SearXNG</body></html>", "", "[1,2,3]"] {
            do {
                _ = try WebAramaIstemcisi.ayristir(Data(bozuk.utf8))
                d.dogru(false, "bozuk gövde reddedilir: \(bozuk.prefix(20))")
            } catch let hata as WebAramaHatasi {
                d.esit(hata, .bicimAnlasilmadi, "bozuk gövde bicimAnlasilmadi verir: \(bozuk.prefix(20))")
            } catch {
                d.dogru(false, "bozuk gövdede beklenen hata türü", "\(error)")
            }
        }
        // `results` yoksa bu BOŞ ama geçerli bir yanıttır — hata değil.
        do {
            let bos = try WebAramaIstemcisi.ayristir(Data("{\"query\":\"x\"}".utf8))
            d.esit(bos.count, 0, "sonuçsuz geçerli JSON boş liste döner (hata değil)")
        } catch {
            d.dogru(false, "sonuçsuz JSON hata vermemeli", "\(error)")
        }

        // Kırpma KELİME SINIRINDA olmalı.
        let kelimeler = Array(repeating: "kelime", count: 60).joined(separator: " ")
        let kirpik = WebAramaIstemcisi.kirp(kelimeler)
        d.dogru(kirpik.count <= WebAramaIstemcisi.ozetTavani + 1,
                "kırpılmış özet tavanı aşmaz", "\(kirpik.count)")
        d.dogru(kirpik.hasSuffix("…"), "kırpılan özet üç noktayla biter")
        let parcalar = kirpik.dropLast().split(separator: " ").map(String.init)
        d.dogru(parcalar.allSatisfy { $0 == "kelime" },
                "kırpma kelimeyi ortasından bölmez", parcalar.last ?? "-")
        // Tavanın altındaki metin dokunulmadan döner.
        d.esit(WebAramaIstemcisi.kirp("kısa özet"), "kısa özet", "kısa özet kırpılmaz")
        d.esit(WebAramaIstemcisi.kirp("iki\nsatır"), "iki satır", "satır sonu boşluğa çevrilir")

        // Alan adı indirgeme.
        d.esit(WebAramaIstemcisi.alanAdiCikar("https://www.mgm.gov.tr/tahmin?il=izmir"),
               "www.mgm.gov.tr", "alan adı yol ve sorgudan arındırılır")
        d.esit(WebAramaIstemcisi.alanAdiCikar("bu bir url değil"), "",
               "geçersiz adres boş alan adı verir")
        d.esit(WebAramaIstemcisi.alanAdiCikar(""), "", "boş adres boş alan adı verir")

        // İstek URL'i: boş sorguda istek KURULMAZ.
        let kok = URL(string: "https://ornek.com/searxng/")!
        d.esit(WebAramaIstemcisi.istekURL(kok: kok, sorgu: "   ", dil: "tr"), nil,
               "boş sorguda istek URL'i kurulmaz")
        let istek = WebAramaIstemcisi.istekURL(kok: kok, sorgu: "hava durumu", dil: "tr")
        d.dogru(istek?.absoluteString.contains("format=json") == true,
                "istek json biçimi ister", istek?.absoluteString ?? "-")
        d.dogru(istek?.absoluteString.contains("/search") == true, "istek /search yoluna gider")
        let dilsiz = WebAramaIstemcisi.istekURL(kok: kok, sorgu: "hava", dil: nil)
        d.dogru(dilsiz?.absoluteString.contains("language=") == false,
                "dil bilinmiyorsa language parametresi HİÇ gönderilmez")
    }

    // MARK: - web-arama-spec §6: bütçe (§5.5)

    @MainActor
    private static func webButce(_ d: inout OtoTestDefteri) {
        d.baslik("WEB ARAMA · MODELE DÖNEN BÜTÇE (§5.5)")

        // Sıfır sonuçta sabit işaret.
        d.esit(WebAramaIstemcisi.modeleMetin(sorgu: "x", sonuclar: []), "no_results",
               "sonuç yoksa sabit no_results döner")

        // EN KÖTÜ DURUM: 5 sonuç, her biri uzun başlık + uzun alan adı + tavan özet.
        let enKotu: [WebSonuc] = (1...WebAramaIstemcisi.sonucTavani).map { i in
            WebSonuc(baslik: String(repeating: "b", count: 60) + "\(i)",
                     alanAdi: "www.cok-uzun-bir-alan-adi-ornegi.com.tr",
                     tamAdres: "https://www.cok-uzun-bir-alan-adi-ornegi.com.tr/" + String(repeating: "y", count: 120),
                     ozet: String(repeating: "ö", count: WebAramaIstemcisi.ozetTavani),
                     bilgiKutusuMu: i == 1)
        }
        let metin = WebAramaIstemcisi.modeleMetin(sorgu: String(repeating: "s", count: 40),
                                                 sonuclar: enKotu)
        // Spec §5.5 tavanı ~300 token; ~4 karakter ≈ 1 token kabulüyle 1200 karakter.
        //
        // BİLİNEN AÇIK: `modeleMetin` BAŞLIĞA TAVAN KOYMAZ. Özetler 200'de kesilir
        // (5 × 200 = 1000 karakter ≈ 250 token) ama başlık ham gelir, dolayısıyla
        // uzun başlıklı beş sonuçta bütçe aşılır. Çözüm ayrıştırma tarafında tek
        // satırdır: `baslik` alanı da `kirp(_:sinir:)` ile (ör. 80) kırpılmalı.
        // Bu iddia bilerek spec değerinde bırakıldı — kod düzelene dek KIRMIZI kalır.
        d.dogru(metin.count <= 1200,
                "en kötü modele dönen metin ~300 token (1200 karakter) bütçesinde",
                "\(metin.count) karakter ≈ \(metin.count / 4) token — başlıkta kırpma yok")

        // Özetlerin tek başına payı bütçenin içinde kalmalı (kırpma çalışıyor).
        let sadeceOzet = enKotu.reduce(0) { $0 + $1.ozet.count }
        d.dogru(sadeceOzet <= 1000, "beş özetin toplamı 1000 karakteri aşmaz", "\(sadeceOzet)")
        d.dogru(!metin.contains("https://"),
                "modele TAM URL gitmez (halüsinasyonlu link riski)")
        d.dogru(metin.contains("[infobox]"), "bilgi kutusu modele işaretli gider")

        // Ham çıktı (çip detayı) tam adresi TAŞIR — kullanıcı ne geldiğini görür.
        let ham = WebAramaIstemcisi.hamCiktiMetni(enKotu)
        d.dogru(ham.contains("https://"), "çip detayında tam adres durur")

        // VeriDeposu tablosu üç sütunlu olmalı.
        let tablo = WebAramaIstemcisi.tablo(enKotu)
        d.esit(tablo.basliklar.count, 3, "sonuç tablosu üç sütunlu")
        d.esit(tablo.satirlar.count, 5, "sonuç tablosu tüm sonuçları taşır")
    }

    // MARK: - mcp-spec §5.6 / web-arama §3.3: onay kapısı

    @MainActor
    private static func onayKapisi(_ d: inout OtoTestDefteri) async {
        d.baslik("ONAY KAPISI · KİRLİ OTURUM (mcp §5.6, §3.3)")

        // 1. Temiz oturumda kapı SORMADAN geçer — onay nadirse okunur.
        let temiz = AracYurutucu()
        let gecti = await temiz.onayIste(kaynak: "ev sunucusu", aracAdi: "issue_ac", icerik: "x")
        d.dogru(gecti, "temiz oturumda onay sorulmaz, çağrı geçer")
        d.esit(temiz.izler.count, 0, "temiz oturumda onay çipi düşmez")
        d.esit(temiz.bekleyenOnay, nil, "temiz oturumda bekleyen istek yok")

        // 2. Kirli oturumda çağrı DURDURULUR ve kullanıcı kararı beklenir.
        let y = AracYurutucu()
        y.kirlet()
        d.dogru(y.oturumKirli, "kirlet() bayrağı kaldırır")

        let icerik = "repo: ev/notlar\nbaslik: alışveriş"
        let gorev = Task { @MainActor in
            await y.onayIste(kaynak: "ev sunucusu", aracAdi: "issue_ac", icerik: icerik)
        }
        // Kapı gerçekten askıya alıyor mu — bekleyen istek görünene dek bekle.
        var tur = 0
        while y.bekleyenOnay == nil && tur < 200 {
            await Task.yield()
            tur += 1
        }
        d.dogru(y.bekleyenOnay != nil, "kirli oturumda çağrı askıya alınır (kapı durdurur)")
        d.esit(y.bekleyenOnay?.icerik, icerik,
               "onay sayfasına GÖNDERİLECEK içeriğin aynısı taşınır")
        d.dogru(y.izler.contains { $0.durum == .onayBekleniyor },
                "akışa 'onay bekleniyor' çipi düşer")

        // 3. Kullanıcı reddediyor.
        y.onayKarariVer(false)
        let karar = await gorev.value
        d.dogru(!karar, "ret sonucu false döner (veri gitmez)")
        d.esit(y.bekleyenOnay, nil, "karar sonrası bekleyen istek temizlenir")
        d.dogru(y.izler.contains { $0.durum == .gonderilmedi },
                "reddedilen istek 'gönderilmedi' çipine döner")

        // 4. AYNI kaynak için ikinci çağrı ÖNBELLEKTEN aynı reddi alır — çip düşmez.
        let cipSayisi = y.izler.count
        let ikinci = await y.onayIste(kaynak: "ev sunucusu", aracAdi: "issue_kapat", icerik: "y")
        d.dogru(!ikinci, "aynı kaynağın ikinci isteği önbellekten reddedilir")
        d.esit(y.izler.count, cipSayisi, "ikinci ret için yeni çip üretilmez (ısrar döngüsü yok)")
        d.esit(y.bekleyenOnay, nil, "ikinci istekte kullanıcıya sorulmaz")

        // 5. BAŞKA kaynak reddedilmiş sayılmaz — ret önbelleği kaynak başınadır.
        let gorev2 = Task { @MainActor in
            await y.onayIste(kaynak: "iş sunucusu", aracAdi: "issue_ac", icerik: "z")
        }
        tur = 0
        while y.bekleyenOnay == nil && tur < 200 {
            await Task.yield()
            tur += 1
        }
        d.dogru(y.bekleyenOnay?.kaynak == "iş sunucusu",
                "farklı kaynak için yeniden sorulur")
        // 6. Kabul edilince bekleme çipi akışta iz bırakmaz.
        let bekleyenIzID = y.bekleyenOnay?.izID
        y.onayKarariVer(true)
        let kabul = await gorev2.value
        d.dogru(kabul, "kabul sonucu true döner")
        d.dogru(!y.izler.contains { $0.id == bekleyenIzID },
                "kabul edilen bekleme çipi akıştan kaldırılır")

        // 7. Kirlilik yeniTur() ile TEMİZLENMEZ, yalnız sohbetiSifirla() temizler.
        y.yeniTur()
        d.dogru(y.oturumKirli, "yeniTur() kirliliği taşır (özet kişisel veri taşıyabilir)")
        let uctuncu = await y.onayIste(kaynak: "ev sunucusu", aracAdi: "x", icerik: "q")
        d.dogru(!uctuncu, "ret önbelleği yeniTur() sonrası da geçerlidir")
        y.sohbetiSifirla()
        d.dogru(!y.oturumKirli, "sohbetiSifirla() kirliliği temizler")
        let temizlendi = await y.onayIste(kaynak: "ev sunucusu", aracAdi: "x", icerik: "q")
        d.dogru(temizlendi, "sohbetiSifirla() ret önbelleğini de temizler")

        // 8. İptal askıda continuation bırakmaz.
        let y2 = AracYurutucu()
        y2.kirlet()
        let gorev3 = Task { @MainActor in
            await y2.onayIste(kaynak: "ev sunucusu", aracAdi: "x", icerik: "q")
        }
        tur = 0
        while y2.bekleyenOnay == nil && tur < 200 {
            await Task.yield()
            tur += 1
        }
        y2.yeniTur()   // tur iptali
        let iptalSonucu = await gorev3.value
        d.dogru(!iptalSonucu, "tur iptalinde bekleyen onay reddedilerek çözülür (askıda kalmaz)")
    }

    // MARK: - web-arama-spec §5.5: AĞ TEKELİ (statik tarama)

    /// `Servis/` ve `Araclar/` altında ağ API'sine dokunan dosyalar YALNIZCA
    /// `WebAramaIstemcisi.swift` ve `MCPIstemcisi.swift` olmalıdır. Başka bir
    /// katman ağa çıkıyorsa "cihazdan ne çıkıyor" sorusunun tek yanıtı kalmaz.
    ///
    /// Tarama kaynak ağacında yapılır: `#filePath` derleme anındaki mutlak yolu
    /// taşır, simülatör aynı makinede çalıştığı için dizin okunabilir.
    @MainActor
    private static func agTekeli(_ d: inout OtoTestDefteri) {
        d.baslik("AĞ TEKELİ · STATİK TARAMA (§5.5)")

        let servis = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
        let kok = servis.deletingLastPathComponent()
        let araclar = kok.appendingPathComponent("Araclar", isDirectory: true)
        let izinli: Set<String> = ["WebAramaIstemcisi.swift", "MCPIstemcisi.swift"]
        // Kendi kaynağımızda desen metin olarak geçmesin diye parçalı yazılır.
        let desenler = ["URL" + "Session", "URL" + "Request", "NW" + "Connection", "CF" + "Socket"]

        var taranan = 0
        var ihlaller: [String] = []
        var okunamadi = false

        for klasor in [servis, araclar] {
            guard let icerik = try? FileManager.default.contentsOfDirectory(
                at: klasor, includingPropertiesForKeys: nil) else {
                okunamadi = true
                continue
            }
            for dosya in icerik where dosya.pathExtension == "swift" {
                guard let metin = try? String(contentsOf: dosya, encoding: .utf8) else { continue }
                taranan += 1
                let ad = dosya.lastPathComponent
                guard !izinli.contains(ad) else { continue }
                for desen in desenler where metin.contains(desen) {
                    ihlaller.append("\(ad) → \(desen)")
                }
            }
        }

        if okunamadi || taranan == 0 {
            // Kaynak ağacı okunamıyorsa test SESSİZCE GEÇMEZ: açıkça başarısızdır,
            // yoksa "hiç dosya bulamadım" yeşil rapor üretirdi.
            d.dogru(false, "kaynak ağacı taranabildi",
                    "taranan=\(taranan) yol=\(kok.path)")
            return
        }

        d.dogru(taranan >= 30, "Servis/ + Araclar/ altındaki dosyalar tarandı", "\(taranan) dosya")
        d.dogru(ihlaller.isEmpty,
                "ağ API'si YALNIZCA WebAramaIstemcisi ve MCPIstemcisi içinde",
                ihlaller.joined(separator: ", "))
        // İzinli dosyaların gerçekten var olduğunu da doğrula: yeniden adlandırılırsa
        // yukarıdaki iddia sahte biçimde yeşile döner.
        for ad in izinli {
            let yol = servis.appendingPathComponent(ad)
            d.dogru(FileManager.default.fileExists(atPath: yol.path),
                    "izinli ağ dosyası yerinde: \(ad)")
        }
    }

    // MARK: - Yardımcılar

    /// Gerçek SearXNG yanıtının sadeleştirilmiş kopyası: 1 bilgi kutusu + 7 sonuç,
    /// biri 200 karakteri aşan özetli, biri başlıksız-adressiz (atlanmalı).
    private static func fixtureJSON() -> String {
        let uzunIcerik = Array(repeating: "kelime", count: 60).joined(separator: " ")
        return """
        {
          "query": "izmir hava durumu",
          "number_of_results": 7,
          "infoboxes": [
            {
              "infobox": "İzmir hava durumu",
              "id": "https://www.mgm.gov.tr/tahmin?il=izmir",
              "content": "\(uzunIcerik)",
              "urls": [{"title": "MGM", "url": "https://www.mgm.gov.tr/tahmin?il=izmir"}]
            }
          ],
          "results": [
            {"title": "İzmir", "url": "https://tr.wikipedia.org/wiki/%C4%B0zmir",
             "content": "İzmir, Türkiye'nin batısında yer alan bir şehirdir.\\nİkinci satır."},
            {"title": "Hava Durumu", "url": "https://www.havadurumu15gunluk.net/izmir",
             "content": "\(uzunIcerik)"},
            {"title": "", "url": "", "content": "atlanmalı"},
            {"title": "Üçüncü", "url": "https://ornek1.com/a", "content": "kısa"},
            {"title": "Dördüncü", "url": "https://ornek2.com/b", "content": "kısa"},
            {"title": "Beşinci", "url": "https://ornek3.com/c", "content": "kısa"},
            {"title": "Altıncı", "url": "https://ornek4.com/d", "content": "tavanın dışında"}
          ]
        }
        """
    }
}
#endif
