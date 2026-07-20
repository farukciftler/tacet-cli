//
//  MCPAraci.swift
//  ketum
//
//  Uzak MCP aracı ↔ FoundationModels `Tool` köprüsü (mcp-baglanti-spec §5.2).
//
//  Derleme zamanı tip YOK, çalışma anı şema VAR: `Arguments = GeneratedContent`
//  ve `parameters` sunucudan gelen JSON Şema'dan çalışma anında kurulur.
//  Constrained decoding sayesinde model şemaya aykırı argüman ÜRETEMEZ.
//
//  Sıra bozulmaz: ÖNCE onay kapısı (AracYurutucu, kodda — modelde değil),
//  SONRA ağ çağrısı. Bu dosyada ağ kodu yoktur; çağrı `MCPCagirici`ye
//  (BaglantiServisi) devredilir, §5.5 sonuç işleme orada uygulanır.
//
//  Çip metni bağlantı adıyla BAŞLAR (§3.2): kullanıcı "bu iş cihaz dışında
//  oldu"yu çipten okur.
//

import Foundation
import FoundationModels

// MARK: - Servis sözleşmeleri

/// Onay kapısı — `AracYurutucu` uygular (mcp §3.3). Araç yürütücünün tamamını
/// değil yalnızca bu tek yeteneği görür.
@MainActor
protocol OnayKapisi: AnyObject, Sendable {
    /// Oturumda kişisel veri aracı en az bir kez başarıyla çalıştı mı (mcp §5.6).
    /// Araç bunu `cihazVerisi == .hicbirZaman` kararını verebilmek için okur:
    /// o ayarda kirli oturumda çağrı HİÇ yapılmaz, onay bile sorulmaz (§3.1).
    var oturumKirli: Bool { get }
    /// `false` = gönderme. Kirli olmayan oturumda sormadan `true` döner —
    /// `zorunlu` true ise oturum temiz olsa da sorulur (yıkıcı uzak araç).
    func onayIste(kaynak: String, aracAdi: String, icerik: String, zorunlu: Bool) async -> Bool
}

extension AracYurutucu: OnayKapisi {}

/// Kirli oturum işaretleyici (mcp §5.6). Kişisel veri araçları (Takvim, Kişi,
/// Arama/Spotlight, Belge*, Hatırlatıcı) İLK BAŞARILI çağrılarından sonra
/// bunu çağırır; kapı böylece kodda kalır, modelde değil.
///
/// Ayrı bir protokol: `AracRaporlayici` çip sözleşmesidir, kirlilik değil.
/// Araç somut `AracYurutucu`ya bağlanmasın diye raporlayıcı bu protokole
/// köprülenir (bkz. `KetumAraci.kirletEgerBasarili`).
@MainActor
protocol KirlilikBildirici: AnyObject, Sendable {
    func kirlet()
}

extension AracYurutucu: KirlilikBildirici {}

extension KetumAraci {
    /// Kişisel veri aracının sonucunu kirlilik bayrağına bağlar (mcp §5.6).
    ///
    /// Yalnızca GERÇEKTEN veriye dokunulan sonuçlar (`.okundu` / `.yazildi`)
    /// kirletir. İzin reddi (`.izinGerekli`) ve hata (`.basarisiz`) kirletmez:
    /// spec "ilk BAŞARILI çağrı" der; erişilemeyen veri oturumu kirletemez.
    /// Sonucu değiştirmeden geri verir, çağrı yerinde zincirlenebilsin diye.
    func kirletEgerBasarili(_ sonuc: AracSonucu) async -> AracSonucu {
        switch sonuc.durum {
        case .okundu, .yazildi:
            await (raporlayici as? any KirlilikBildirici)?.kirlet()
        default:
            break
        }
        return sonuc
    }
}

/// Sunucudan `tools/list` ile gelen ham araç tanımı.
struct MCPAracTanimi: Hashable, Sendable {
    /// Uzak araç adı (MCP `tools/list` içindeki ad).
    var ad: String
    /// Sunucunun yazdığı ham açıklama — 4096 pencereye ham girmez, §5.3'te özetlenir.
    var aciklama: String
    /// Ham JSON Şema (`inputSchema`) — UTF-8 JSON nesnesi.
    var girdiSemasiJSON: Data
    /// Sunucunun `annotations.readOnlyHint`i; bildirmediyse nil.
    var saltOkumaIpucu: Bool?
    /// Sunucunun `annotations.destructiveHint`i; bildirmediyse nil.
    var yikiciIpucu: Bool?

    init(ad: String, aciklama: String = "", girdiSemasiJSON: Data = Data(),
         saltOkumaIpucu: Bool? = nil, yikiciIpucu: Bool? = nil) {
        self.ad = ad
        self.aciklama = aciklama
        self.girdiSemasiJSON = girdiSemasiJSON
        self.saltOkumaIpucu = saltOkumaIpucu
        self.yikiciIpucu = yikiciIpucu
    }
}

/// Uzak aracın yan etki sınıfı (mcp §3.3 genişletmesi).
///
/// Onay kapısı bugüne dek YALNIZCA "cihaz verisi dışarı sızmasın" kapısıydı:
/// oturumda kişisel veri aracı kullanılmadıysa her uzak çağrı sorgusuz geçiyordu.
/// Ama uzak tarafta yan etki bırakan bir araç (`dosya_sil`, `komut_calistir`,
/// `eposta_gonder`) için sorulması gereken soru "cihazdan ne çıkıyor" değil
/// "kullanıcının sunucusunda ne DEĞİŞİYOR"dur — ve bu, oturumun temiz olmasıyla
/// hiç ilgisiz. Bu tip, o ikinci soruyu kodda görünür kılar.
enum YanEtkiSinifi: Sendable {
    /// Sunucu salt-okuma dedi ya da ad/özet hiçbir yıkıcılık sinyali taşımıyor.
    case saltOkuma
    /// Sunucu yıkıcı dedi ya da ad/özet yıkıcı bir eylem sinyali taşıyor.
    case yikici

    var onayZorunluMu: Bool { self == .yikici }

    /// Yıkıcılık sinyali taşıyan eylem kökleri — Türkçe ve İngilizce.
    ///
    /// Sözlük SEZGİSELDİR ve tam olduğu iddia edilmiyor: sunucu keyfî adlar
    /// verebilir. Bu yüzden tek savunma değil, `annotations` ipuçlarının
    /// ÜSTÜNE binen ikinci kattır.
    ///
    /// YALNIZCA ADA bakılır, açıklamaya DEĞİL. İlk sürüm açıklamayı da
    /// tarıyordu ve bu ölçülebilir bir arıza üretti: salt-okuma `ag_durumu`
    /// aracının sunucu açıklamasında "command" geçtiği için araç yıkıcı
    /// sayıldı, her çağrıda onay istedi ve eval koşusunda 250 sn'lik zaman
    /// aşımlarına düştü. Açıklama sunucunun yazdığı serbest metindir; yıkıcı
    /// EYLEM adı ile o eylemden BAHSEDEN cümleyi ayırt edemez. Ad ise
    /// sözleşmedir. Yanlış pozitif burada ucuz değil — gereksiz onay kapı
    /// yorgunluğu üretir ve kullanıcı hepsini körlemesine onaylamaya başlar.
    private static let yikiciKokler = [
        "sil", "delete", "remove", "drop", "destroy", "purge", "wipe",
        "yaz", "write", "olustur", "create", "kaydet", "save",
        "degistir", "degisiklik", "modify", "update", "patch", "edit", "rename",
        "tasi", "kopyala", "move", "copy", "upload", "put",
        "calistir", "exec", "execute", "run", "shell", "command", "komut",
        "gonder", "send", "post", "eposta", "email", "mail", "notify",
        "yonet", "manage", "restart", "stop", "start", "kill", "deploy",
        "install", "kur", "reboot", "shutdown", "grant", "revoke", "chmod"
    ]

    /// Ad + özetten sınıf çıkarır. Sıra önemli: sunucunun AÇIK beyanı önce.
    static func sinifla(ad: String, ozet: String,
                        saltOkumaIpucu: Bool?, yikiciIpucu: Bool?) -> YanEtkiSinifi {
        // Sunucu "yıkıcı" dediyse tartışma yok.
        if yikiciIpucu == true { return .yikici }
        // Sunucu "salt okuma" dediyse ona güveniriz — ama yalnızca yıkıcı
        // olduğunu ayrıca söylemediyse (yukarıdaki dal zaten kazandı).
        if saltOkumaIpucu == true { return .saltOkuma }

        // Sunucu susuyor: ADA bak. `ozet` bilerek kullanılmıyor (yukarıdaki not).
        _ = ozet
        let havuz = ad.lowercased()
            .folding(options: .diacriticInsensitive, locale: Locale(identifier: "tr"))
        return yikiciKokler.contains(where: havuz.contains) ? .yikici : .saltOkuma
    }
}

/// Uzak çağrının §5.5 (4096 bypass) uygulanmış sonucu.
struct MCPSonucu: Sendable {
    /// Çip metninin `·` sonrası parçası — "git pull tamam". Bağlantı adını
    /// araç ekler, servis eklemez.
    var cipDetayi: String
    /// Modele dönecek kısa metin: özet + gerekiyorsa `kaynakRef` (§5.5).
    var modeleDonen: String
    /// Çip detayında gösterilecek ham çıktının TAMAMI (şeffaflık ikinci katman).
    var hamCikti: String?

    init(cipDetayi: String, modeleDonen: String, hamCikti: String? = nil) {
        self.cipDetayi = cipDetayi
        self.modeleDonen = modeleDonen
        self.hamCikti = hamCikti
    }
}

/// Uygulamadaki TEK ağ yolu buradan geçer (§2.1): araç ağ API'sine dokunmaz,
/// yalnızca bu sözleşmeyi çağırır. `BaglantiServisi` uygular.
@MainActor
protocol MCPCagirici: AnyObject, Sendable {
    /// Uzak aracı çağırır ve §5.5 sonuç işlemesini uygulamış sonucu döndürür.
    /// Zaman aşımı, iptal ve yetki hataları `Error` olarak fırlatılır; çip
    /// metnine dönecek düz dilli neden `localizedDescription`dan alınır.
    func cagir(baglantiID: UUID, aracAdi: String, argumanlarJSON: String) async throws -> MCPSonucu
}

// MARK: - Araç

/// Tek bir uzak araç. Bağlantıdaki her desteklenen araç için bir örnek üretilir.
struct MCPAraci: KetumAraci {
    let name: String
    let description: String
    /// Çalışma anında sunucu şemasından kurulan argüman şeması.
    let parameters: GenerationSchema

    /// Derleme zamanı tip yok: model ne üretirse şemaya uygun `GeneratedContent`
    /// olarak gelir.
    typealias Arguments = GeneratedContent

    /// Çip metninin başına gelen ad — "ev sunucusu".
    let baglantiAdi: String
    let baglantiID: UUID
    /// Uzak araç adı; `name` çakışmayı önlemek için önek almış olabilir.
    let uzakAd: String
    /// Ekleme anında önbelleklenen sunucu özeti (§5.3). `description` bunun
    /// sunucu adıyla süslenmiş hâli; ham özet AYRICA saklanır çünkü yuva
    /// alaka sıralaması (`AracAlaka`) süsü değil METNİ okumalı.
    let ozet: String
    /// Bağlantının cihaz verisi ayarı (§3.1). Kirli oturumda davranışı belirler:
    /// `.hicbirZaman` (varsayılan) hiç çağırmaz, `.herSeferindeSor` onay sorar.
    /// Kirli OLMAYAN oturumda ikisi de sorgusuz geçer (§2.4 "onay nadirse okunur").
    let cihazVerisi: CihazVerisiAyari
    /// Uzak tarafta yan etki bırakıyor mu (§3.3). `.yikici` ise oturum temiz
    /// olsa DA onay sorulur; bu kapı `cihazVerisi` ayarından bağımsızdır.
    let yanEtki: YanEtkiSinifi

    let cagirici: any MCPCagirici
    weak var kapi: (any OnayKapisi)?
    weak var raporlayici: (any AracRaporlayici)?

    init(baglantiID: UUID,
         baglantiAdi: String,
         uzakAd: String,
         ozet: String,
         parameters: GenerationSchema,
         cagirici: any MCPCagirici,
         cihazVerisi: CihazVerisiAyari = .hicbirZaman,
         yanEtki: YanEtkiSinifi = .saltOkuma,
         kapi: (any OnayKapisi)? = nil,
         raporlayici: (any AracRaporlayici)? = nil,
         cozulmusAd: String? = nil) {
        self.yanEtki = yanEtki
        self.baglantiID = baglantiID
        self.baglantiAdi = baglantiAdi
        self.uzakAd = uzakAd
        self.ozet = ozet
        // Varsayılan en kısıtlı seçenek: araç üretilirken ayar UNUTULURSA
        // davranış "gönderme" tarafına düşer, sessizce sızma tarafına değil.
        self.cihazVerisi = cihazVerisi
        // Ad koleksiyon düzeyinde çözülmüş olabilir (§P2-9): iki sunucuda aynı
        // uzak ad varsa `adlariCoz` ikisine farklı `name` verir ve buraya
        // çözülmüşünü geçirir. Geçirilmediyse tek-ad davranışı korunur.
        self.name = cozulmusAd.map(Self.gecerliAd) ?? Self.gecerliAd(uzakAd)
        // Modele giden tanım §5.3'te sıkıştırılmış özettir; buraya ham
        // açıklama koymak 4096 pencereyi tek araçla doldurabilir.
        self.description = Self.tanim(ozet: ozet, sunucu: baglantiAdi)
        self.parameters = parameters
        self.cagirici = cagirici
        self.kapi = kapi
        self.raporlayici = raporlayici
    }

    func call(arguments: GeneratedContent) async -> String {
        let argumanlar = Self.okunurJSON(arguments)

        // ÖNCE KAPI. Kirli oturumda kullanıcı gönderilecek içeriğin aynısını
        // görmeden hiçbir şey çıkmaz; kapı kodda, modelde değil (§2.2).
        if let kapi {
            // Cihaz verisi ayarı kapının ÖNÜNDE okunur (§3.1). "hiçbir zaman"
            // kirli oturumda soru bile sormaz: kullanıcı kararını bağlantıyı
            // eklerken bir kez verdi, her çağrıda yeniden sorulmaz.
            let kirli = await kapi.oturumKirli
            if cihazVerisi == .hicbirZaman, kirli {
                await gonderilmediCipi(argumanlar: argumanlar)
                // Modele dönen sözleşme onay reddiyle AYNI: model iki yolu
                // ayırt edemez, dolayısıyla ayara göre ısrar stratejisi geliştiremez.
                return "kullanıcı bu veriyi paylaşmayı reddetti"
            }

            // "Her zaman izin ver": kapı atlanır. Kullanıcı bu kararı bağlantı
            // ayarında bir kez, uyarı modalını okuyarak verdi.
            //
            // GİZLEME YOK (§2.2): kapı atlansa da giden içerik çipin ham
            // girdisinde DURUR ve kirli oturumda gönderildiyse bunun onay
            // sorulmadan olduğu ayrıca yazılır. Kullanıcı sonradan "ne çıktı"
            // sorusunu yanıtlayabilmeli; atlanan şey ONAY, ŞEFFAFLIK DEĞİL.
            // Yıkıcı araç: oturum temiz olsa da, kullanıcı "her zaman izin ver"
            // demiş olsa DA sorulur. O ayar "cihazımdaki veriyi sormadan
            // gönderebilirsin" demektir; "sunucumda sormadan iş yapabilirsin"
            // demek değildir. İki karar ayrı, kapı da ayrı.
            if yanEtki.onayZorunluMu {
                let onay = await kapi.onayIste(kaynak: baglantiAdi,
                                               aracAdi: uzakAd,
                                               icerik: argumanlar,
                                               zorunlu: true)
                guard onay else { return "kullanıcı bu veriyi paylaşmayı reddetti" }
                return await uzagaCagir(argumanlar: argumanlar, gonderilen: argumanlar)
            }

            if cihazVerisi.kapiyiAtlarMi {
                let not = kirli
                    ? String(localized: "onay sorulmadan gönderildi · bağlantı ayarı: her zaman izin ver")
                    : String(localized: "gönderildi · oturumda kişisel veri aracı kullanılmamıştı")
                return await uzagaCagir(argumanlar: "\(not)\n\n\(argumanlar)",
                                        gonderilen: argumanlar)
            }

            let onay = await kapi.onayIste(kaynak: baglantiAdi,
                                           aracAdi: uzakAd,
                                           icerik: argumanlar,
                                           zorunlu: false)
            guard onay else {
                // Ret bir hata değil kısıttır: çipi AracYurutucu "gönderilmedi"
                // durumunda bıraktı, burada ikinci çip düşürülmez.
                return "kullanıcı bu veriyi paylaşmayı reddetti"
            }
        }

        return await uzagaCagir(argumanlar: argumanlar, gonderilen: argumanlar)
    }

    /// Uzak çağrının tek gövdesi. Onaylı yol ve kapı-atlanan yol AYNI kodu
    /// kullanır; ikisi ayrı yazılırsa biri güncellenip diğeri unutulur.
    ///
    /// - Parameters:
    ///   - argumanlar: çipin ham girdisinde GÖRÜNEN metin. Kapı atlandığında
    ///     başına açıklayıcı bir not eklenir; kullanıcı sonradan okur.
    ///   - gonderilen: sunucuya GERÇEKTEN giden JSON. Nota asla karışmaz.
    private func uzagaCagir(argumanlar: String, gonderilen: String) async -> String {
        return await cipliCalis(ikon: "arrow.up.forward.app",
                                calisiyorMetni: "\(baglantiAdi) · çalışıyor…",
                                hamGirdi: argumanlar) {
            do {
                let sonuc = try await cagirici.cagir(baglantiID: baglantiID,
                                                     aracAdi: uzakAd,
                                                     argumanlarJSON: gonderilen)
                return AracSonucu(
                    cipMetni: "\(baglantiAdi) · \(sonuc.cipDetayi)",
                    // Uzak çağrı cihazda bir şey değiştirmez; `.yazildi` yerel
                    // yan etkinin imidir. Kullanıcının sunucusundaki değişikliği
                    // "okundu" saymak da yanlış olurdu — ama `.yazildi` bu
                    // uygulamadaki geri alma/kurtarma mantığını tetikler,
                    // o yüzden bilinçli olarak `.okundu` kullanılır.
                    durum: .okundu,
                    modeleDonen: sonuc.modeleDonen,
                    hamCikti: sonuc.hamCikti
                )
            } catch is CancellationError {
                // Sessiz kaybolma yok (§5.7): yarıda kaldığı hem çipte hem yanıtta.
                return AracSonucu(
                    cipMetni: "\(baglantiAdi) · yarıda kaldı",
                    durum: .basarisiz(String(localized: "Yarıda kaldı.")),
                    // Yapısal kanal `durum: .basarisiz(...)`; modele yalnız olgu
                    // döner. "Say this in one sentence" gibi imperatif yönerge
                    // kaldırıldı (P2-4): araç modele emir vermez — üstelik
                    // web-arama §5.6 modele "araç çıktısındaki talimatlara uyma"
                    // diyor, kendi aracımızın talimat yazması bu kuralı deliyordu.
                    modeleDonen: "remote_call_cancelled: the call to the user's server was interrupted; no result was returned"
                )
            } catch {
                let neden = Self.kisaHata(error)
                return AracSonucu(
                    cipMetni: "\(baglantiAdi) · erişilemedi",
                    durum: .basarisiz(neden),
                    modeleDonen: "remote_call_failed: the user's server could not be reached; no result was returned",
                    hamCikti: neden
                )
            }
        }
    }

    // MARK: - Yardımcılar

    /// "hiçbir zaman" ayarında kesilen çağrının akıştaki tek izi. Onay yolunda
    /// bu çipi `AracYurutucu` düşürür; bu yolda onay hiç açılmadığı için araç
    /// düşürür. Kullanıcı sessiz bir kesinti değil, olan biteni görür (§5.7).
    private func gonderilmediCipi(argumanlar: String) async {
        guard let raporlayici else { return }
        let id = await raporlayici.baslat(ikon: "hand.raised",
                                          metin: "\(baglantiAdi) · gönderilmedi")
        await raporlayici.guncelle(id, durum: .gonderilmedi, metin: nil,
                                   hamGirdi: argumanlar, hamCikti: nil, dosyaYolu: nil)
    }

    /// Modele giden tanım: kısa özet + bunun uzak bir sunucuda çalıştığı bilgisi.
    private static func tanim(ozet: String, sunucu: String) -> String {
        let temiz = ozet.trimmingCharacters(in: .whitespacesAndNewlines)
        let govde = temiz.isEmpty ? "Runs a tool on the user's own server." : temiz
        return "\(govde) Runs remotely on the user's own server '\(sunucu)'."
    }

    /// Tool adı olarak güvenli hale getirir: harf/rakam/alt çizgi.
    static func gecerliAd(_ ham: String) -> String {
        let izinli = ham.map { karakter -> Character in
            karakter.isLetter || karakter.isNumber || karakter == "_" ? karakter : "_"
        }
        let ad = String(izinli)
        return ad.isEmpty ? "uzak_arac" : ad
    }

    /// Ad çakışması çözümü (P2-9).
    ///
    /// `gecerliAd` TEK bir adı temizler ve diğer araçları GÖRMEZ; çakışma
    /// yapısal olarak ancak koleksiyon düzeyinde tespit edilebilir. İki
    /// sunucuda aynı `uzakAd` varsa (ya da iki farklı ad aynı geçerli ada
    /// indirgeniyorsa — "dosya-oku" ve "dosya oku" ikisi de `dosya_oku`)
    /// modele iki araç AYNI adla gider ve biri diğerini sessizce gölgeler:
    /// model hangisini çağırdığını bilmez, sunucu seçimi rastgeleleşir.
    ///
    /// Çözüm sırası önemli: ÖNCE sunucu öneki denenir (okunur ve modele
    /// bilgi taşır — `evsunucu_dosya_oku`), yalnız o da çakışırsa sayı eklenir.
    /// Sıra korunur; ilk gelen adını değiştirmeden tutar, çünkü onun adı
    /// zaten kullanıcı ayarlarında/özetlerde geçiyor olabilir.
    ///
    /// - Parameter girdiler: (uzakAd, sunucuAdi) çiftleri, sunucu sırasıyla.
    /// - Returns: girdilerle AYNI sırada, ikişer ikişer FARKLI, geçerli adlar.
    static func adlariCoz(_ girdiler: [(uzakAd: String, sunucu: String)]) -> [String] {
        var kullanilan = Set<String>()
        var sonuc: [String] = []
        for girdi in girdiler {
            let temel = gecerliAd(girdi.uzakAd)
            var aday = temel
            if kullanilan.contains(aday) {
                let onek = gecerliAd(girdi.sunucu).lowercased()
                if !onek.isEmpty, onek != "uzak_arac" { aday = "\(onek)_\(temel)" }
            }
            var sayi = 2
            while kullanilan.contains(aday) {
                aday = "\(temel)_\(sayi)"
                sayi += 1
            }
            kullanilan.insert(aday)
            sonuc.append(gecerliAd(aday))
        }
        return sonuc
    }

    /// Onay sayfasında ve çip detayında gösterilecek argüman metni.
    /// Kategori özeti DEĞİL: gönderilecek içeriğin aynısı (§3.3).
    static func okunurJSON(_ icerik: GeneratedContent) -> String {
        let ham = icerik.jsonString
        guard let veri = ham.data(using: .utf8),
              let nesne = try? JSONSerialization.jsonObject(with: veri),
              let guzel = try? JSONSerialization.data(withJSONObject: nesne,
                                                      options: [.prettyPrinted, .sortedKeys]),
              let metin = String(data: guzel, encoding: .utf8) else {
            return ham
        }
        return metin
    }
}

// MARK: - Araç yuvası alaka sıralaması (P1-6)

/// 4096 pencerede oturuma en fazla 6 uzak araç giriyor. Eski doldurma
/// `Array(mcpAraclari.prefix(tavan))` idi: yuvalar SUNUCUNUN döndürdüğü
/// sırayla, yani KÖR dolduruluyordu. 20 araçlı bir sunucuda kullanıcı "issue
/// aç" dediğinde `issue_olustur` 14. sıradaysa masaya hiç gelmiyor ve model
/// "böyle bir şey yapamam" diyor — araç VAR, yuva yok.
///
/// Buradaki puanlama BİLEREK aptal: kelime eşleşmesi + son kullanım. Cihaz-üstü
/// bir gömme modeli çalıştırmak turun kendisinden pahalı olurdu ve yanlış
/// sıralamanın maliyeti zaten düşük (model yine de yanlış aracı çağırmaz;
/// yalnız doğru araç masada olmayabilir). Ölçülebilir ve saf olması, akıllı
/// olmasından değerli.
enum AracAlaka {

    /// Türkçe ekleri ve aksanı düşüren kaba normalleştirme.
    static func kokler(_ metin: String) -> [String] {
        metin.lowercased()
            .folding(options: .diacriticInsensitive, locale: Locale(identifier: "tr"))
            .split(whereSeparator: { !$0.isLetter && !$0.isNumber })
            .map(String.init)
            .filter { $0.count >= 3 }
    }

    /// Tek aracın alaka puanı. Yüksek = masaya daha çok yakışır.
    ///
    /// - Ad eşleşmesi (+10) özetten (+4) ağır: sunucu özeti serbest metindir
    ///   ve "bu araç issue AÇMAZ" cümlesi de "issue" içerir; ad sözleşmedir.
    /// - Önek eşleşmesi (+6) "issue" ↔ "issues" / "olustur" ↔ "olusturma"
    ///   farkını kapatır; tam eşleşmeyle çift sayılmaz.
    /// - Son kullanım küçük bir taban (+3/+1): kullanıcının o turda hiç
    ///   sözünü etmediği bir aracı kelime eşleşmesinin ÖNÜNE geçirmemeli.
    static func puan(ad: String, ozet: String, soruKokleri: [String],
                     sonKullanim: Date?, simdi: Date = Date()) -> Int {
        let adKok = ad.lowercased()
            .folding(options: .diacriticInsensitive, locale: Locale(identifier: "tr"))
        let ozetKok = ozet.lowercased()
            .folding(options: .diacriticInsensitive, locale: Locale(identifier: "tr"))
        var toplam = 0
        for kok in Set(soruKokleri) {
            if adKok.contains(kok) {
                toplam += 10
            } else if kok.count >= 4, adKok.contains(String(kok.prefix(kok.count - 1))) {
                toplam += 6
            }
            if ozetKok.contains(kok) { toplam += 4 }
        }
        if let sonKullanim {
            toplam += simdi.timeIntervalSince(sonKullanim) < 3600 ? 3 : 1
        }
        return toplam
    }

    /// Alakaya göre sırala. **Kararlı**: eşit puanlı araçlar sunucu sırasını
    /// korur, yani soru hiçbir sinyal taşımadığında davranış eski kör
    /// prefix'in AYNISIDIR — bu bir gerileme güvencesidir, süs değil.
    static func sirala<T>(_ ogeler: [T],
                          soru: String,
                          sonKullanim: [String: Date] = [:],
                          simdi: Date = Date(),
                          ad: (T) -> String,
                          ozet: (T) -> String) -> [T] {
        let kokler = kokler(soru)
        guard !kokler.isEmpty || !sonKullanim.isEmpty else { return ogeler }
        return ogeler.enumerated()
            .map { (sira: $0.offset, oge: $0.element,
                    puan: puan(ad: ad($0.element), ozet: ozet($0.element),
                               soruKokleri: kokler,
                               sonKullanim: sonKullanim[ad($0.element)],
                               simdi: simdi)) }
            .sorted { ($0.puan, -$0.sira) > ($1.puan, -$1.sira) }
            .map(\.oge)
    }
}

// MARK: - Şema çevirisi (§5.2)

/// Şema düzleştirilemediğinde neden. Kullanıcıya "desteklenmiyor" olarak
/// listelenir — sessizce yutulmaz.
enum SemaHatasi: LocalizedError, Equatable {
    case cokDerin
    case cokGenis
    case duzlesmiyor(String)
    case bozukSema

    var errorDescription: String? {
        switch self {
        case .cokDerin:
            return String(localized: "Şeması fazla iç içe.")
        case .cokGenis:
            return String(localized: "Şeması fazla geniş (çok fazla alan).")
        case .duzlesmiyor(let alan):
            return String(localized: "Şu alan sadeleştirilemedi: \(alan)")
        case .bozukSema:
            return String(localized: "Sunucu okunabilir bir şema vermedi.")
        }
    }
}

/// MCP `inputSchema` (JSON Şema) → `GenerationSchema` çevirisi.
///
/// Aşırı iç içe / `anyOf` yoğun şemalar düzleştirilir; düzleşmiyorsa araç
/// ATLANIR ve bağlantı detayında "desteklenmiyor" diye listelenir (§5.2).
enum MCPSemaCevirici {
    /// Nesne iç içeliği için üst sınır. 3B modelin derin ağaçları doğru
    /// dolduramadığı yerde araç atlamak, yanlış argüman üretmekten iyidir.
    static let derinlikSiniri = 4

    /// Şema GENİŞLİĞİ tavanı (P1-6). Derinlik sınırı tek başına yetmiyordu:
    /// 200 alanlı DÜZ bir nesne derinlik 1'dir ve eski kod onu koşulsuz
    /// çeviriyordu. 4096 token'lık pencerede tek araç şeması yüzlerce alan
    /// adı + açıklama taşıyabilir — bu bir token bombasıdır ve diğer araçları
    /// (ve konuşma geçmişini) pencereden atar.
    ///
    /// 48 sayısı: gerçek MCP sunucularında gördüğümüz en geniş araç ~20 düğüm;
    /// tavan onun iki katının biraz üstünde, yani meşru hiçbir aracı elemez
    /// ama patolojik şemayı keser. Aşan araç ATLANIR ve bağlantı detayında
    /// "desteklenmiyor" diye listelenir — sessizce kırpılmaz, çünkü yarısı
    /// kesilmiş bir şema modele yalan söyler (zorunlu alan görünmez olur).
    static let dugumButcesi = 48

    /// Alan (property) açıklaması karakter tavanı (P2-9). Araç DÜZEYİ tanım
    /// zaten `MCPAraci.tanim`de sıkıştırılıyordu, ALAN düzeyi ham geçiyordu:
    /// tek bir 5000 karakterlik `description` pencereyi tek başına yiyebilir.
    static let aciklamaTavani = 160

    /// Tek aracın şemasını çevirir. Fırlatırsa araç desteklenmiyordur.
    static func cevir(tanim: MCPAracTanimi) throws -> GenerationSchema {
        let nesne = try sozluk(tanim.girdiSemasiJSON)
        var sayac = 0
        let kok = try dugum(ad: tanim.ad, sema: nesne, derinlik: 0, sayac: &sayac)
        return try GenerationSchema(root: kok, dependencies: [])
    }

    /// Kaç düğüm üretileceğini ÇEVİRMEDEN sayar (ölçüm/iddia için).
    /// Bütçeyi aşan şemada `cevir` fırlatır; bu fonksiyon fırlatmaz, sayıyı verir.
    static func dugumSayisi(_ ham: [String: Any]) -> Int {
        var toplam = 1
        if let alanlar = ham["properties"] as? [String: Any] {
            for (_, alt) in alanlar {
                toplam += dugumSayisi((alt as? [String: Any]) ?? [:])
            }
        }
        if let oge = ham["items"] as? [String: Any] {
            toplam += dugumSayisi(oge)
        }
        return toplam
    }

    /// Açıklamayı tavana kırpar. Boş girdi nil döner; BOŞ OLMAYAN girdi asla
    /// boş dönmez (kırpma bilgiyi azaltır, yok etmez).
    static func kirpAciklama(_ ham: String?) -> String? {
        guard let ham else { return nil }
        let temiz = ham
            .replacingOccurrences(of: "\n", with: " ")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard !temiz.isEmpty else { return nil }
        guard temiz.count > aciklamaTavani else { return temiz }
        var kesik = String(temiz.prefix(aciklamaTavani))
        // Sözcük ortasında kesmemeye çalış — ama yarıdan fazlasını atmak
        // pahasına değil, yoksa tek uzun sözcüklü açıklama boşa düşerdi.
        if let bosluk = kesik.lastIndex(of: " "),
           kesik.distance(from: kesik.startIndex, to: bosluk) >= aciklamaTavani / 2 {
            kesik = String(kesik[kesik.startIndex..<bosluk])
        }
        return kesik + "…"
    }

    /// Bir bağlantının araç listesini ayıklar: çevrilenler ve atlananlar.
    /// Atlananlar `AracOzeti.desteklenmiyor` ile listelenir.
    static func ayikla(_ tanimlar: [MCPAracTanimi])
        -> (kabul: [(tanim: MCPAracTanimi, sema: GenerationSchema)],
            atlanan: [(tanim: MCPAracTanimi, neden: String)]) {
        var kabul: [(tanim: MCPAracTanimi, sema: GenerationSchema)] = []
        var atlanan: [(tanim: MCPAracTanimi, neden: String)] = []
        for tanim in tanimlar {
            do {
                kabul.append((tanim, try cevir(tanim: tanim)))
            } catch {
                let neden = (error as? LocalizedError)?.errorDescription
                    ?? String(localized: "Şeması desteklenmiyor.")
                atlanan.append((tanim, neden))
            }
        }
        return (kabul, atlanan)
    }

    // MARK: - Özyineleme

    private static func sozluk(_ veri: Data) throws -> [String: Any] {
        guard !veri.isEmpty,
              let nesne = try? JSONSerialization.jsonObject(with: veri) as? [String: Any] else {
            throw SemaHatasi.bozukSema
        }
        return nesne
    }

    private static func dugum(ad: String,
                              sema ham: [String: Any],
                              derinlik: Int,
                              sayac: inout Int) throws -> DynamicGenerationSchema {
        guard derinlik <= derinlikSiniri else { throw SemaHatasi.cokDerin }
        // Genişlik bütçesi (P1-6): sayaç TÜM ağaç boyunca ortaktır, yani
        // 200 alanlı düz nesne de, 50 alanlı iki kat da aynı tavana çarpar.
        sayac += 1
        guard sayac <= dugumButcesi else { throw SemaHatasi.cokGenis }

        // anyOf/oneOf: önce düzleştirmeyi dener, olmuyorsa aracı atlatır.
        let sema = try birlesimiDuzlestir(ad: ad, sema: ham)
        let aciklama = kirpAciklama(sema["description"] as? String)

        switch try tur(sema) {
        case "object":
            let alanlar = sema["properties"] as? [String: Any] ?? [:]
            let zorunlu = Set(sema["required"] as? [String] ?? [])
            // Alan sayısı tek başına da bütçeyi aşıyorsa özyinelemeye hiç
            // girmeden kes: 5000 alanlı şemada 48 kez döngüye girmenin anlamı
            // olsa da, hata mesajının nedeni ("çok geniş") burada netleşir.
            guard sayac + alanlar.count <= dugumButcesi else { throw SemaHatasi.cokGenis }
            // Anahtar sırası deterministik olsun: aynı sunucu her açılışta
            // aynı şemayı üretsin.
            let ozellikler = try alanlar.keys.sorted().map { anahtar -> DynamicGenerationSchema.Property in
                guard let alt = alanlar[anahtar] as? [String: Any] else {
                    throw SemaHatasi.duzlesmiyor(anahtar)
                }
                let altSema = try dugum(ad: "\(ad)_\(anahtar)",
                                        sema: alt,
                                        derinlik: derinlik + 1,
                                        sayac: &sayac)
                return DynamicGenerationSchema.Property(
                    name: anahtar,
                    description: kirpAciklama(alt["description"] as? String),
                    schema: altSema,
                    isOptional: !zorunlu.contains(anahtar)
                )
            }
            return DynamicGenerationSchema(name: ad, description: aciklama, properties: ozellikler)

        case "array":
            guard let oge = sema["items"] as? [String: Any] else {
                throw SemaHatasi.duzlesmiyor(ad)
            }
            let ogeSema = try dugum(ad: "\(ad)_oge", sema: oge,
                                    derinlik: derinlik + 1, sayac: &sayac)
            return DynamicGenerationSchema(arrayOf: ogeSema)

        case "string":
            // Sabit seçenek listesi doğrudan şemaya girer: model listenin
            // dışına çıkamaz.
            if let secenekler = sema["enum"] as? [Any] {
                let metinler = secenekler.compactMap { $0 as? String }
                guard !metinler.isEmpty, metinler.count == secenekler.count else {
                    throw SemaHatasi.duzlesmiyor(ad)
                }
                return DynamicGenerationSchema(name: ad, description: aciklama, anyOf: metinler)
            }
            return DynamicGenerationSchema(type: String.self)

        case "integer":
            return DynamicGenerationSchema(type: Int.self)
        case "number":
            return DynamicGenerationSchema(type: Double.self)
        case "boolean":
            return DynamicGenerationSchema(type: Bool.self)
        default:
            throw SemaHatasi.duzlesmiyor(ad)
        }
    }

    /// `type` alanı — dizi biçiminde geldiyse ("string"/"null" gibi) null
    /// atılır ve tek tür kalırsa o kullanılır.
    private static func tur(_ sema: [String: Any]) throws -> String {
        if let tek = sema["type"] as? String { return tek }
        if let coklu = sema["type"] as? [String] {
            let bos = coklu.filter { $0 != "null" }
            if bos.count == 1 { return bos[0] }
            throw SemaHatasi.duzlesmiyor(coklu.joined(separator: "/"))
        }
        // Tür yazılmamış ama alanlar verilmişse nesnedir; hiçbir ipucu yoksa
        // uydurmayız.
        if sema["properties"] != nil { return "object" }
        if sema["items"] != nil { return "array" }
        if sema["enum"] != nil { return "string" }
        throw SemaHatasi.bozukSema
    }

    /// `anyOf`/`oneOf` düzleştirme: nullable sarmalı ve tek türlü birleşimler
    /// açılır. Gerçekten ayrışan birleşimler düzleşmez — araç atlanır.
    private static func birlesimiDuzlestir(ad: String,
                                           sema: [String: Any]) throws -> [String: Any] {
        let anahtar = sema["anyOf"] != nil ? "anyOf" : (sema["oneOf"] != nil ? "oneOf" : nil)
        guard let anahtar, let dallar = sema[anahtar] as? [[String: Any]] else { return sema }

        // "null" dalları kısıt değil, isteğe bağlılık bilgisidir; zorunluluk
        // zaten `required` listesinden geliyor.
        let anlamli = dallar.filter { ($0["type"] as? String) != "null" }
        guard var tek = anlamli.first else { throw SemaHatasi.duzlesmiyor(ad) }

        if anlamli.count > 1 {
            // Hepsi aynı ilkel türse birleşim bilgi taşımıyordur, tek dala iner.
            let turler = Set(anlamli.compactMap { $0["type"] as? String })
            guard turler.count == 1, let t = turler.first,
                  ["string", "integer", "number", "boolean"].contains(t) else {
                throw SemaHatasi.duzlesmiyor(ad)
            }
        }

        // Sarmalın kendi açıklaması varsa korunur.
        if tek["description"] == nil, let aciklama = sema["description"] {
            tek["description"] = aciklama
        }
        return tek
    }
}
