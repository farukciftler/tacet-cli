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
    /// `false` = gönderme. Kirli olmayan oturumda sormadan `true` döner.
    func onayIste(kaynak: String, aracAdi: String, icerik: String) async -> Bool
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

    init(ad: String, aciklama: String = "", girdiSemasiJSON: Data = Data()) {
        self.ad = ad
        self.aciklama = aciklama
        self.girdiSemasiJSON = girdiSemasiJSON
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
    /// Bağlantının cihaz verisi ayarı (§3.1). Kirli oturumda davranışı belirler:
    /// `.hicbirZaman` (varsayılan) hiç çağırmaz, `.herSeferindeSor` onay sorar.
    /// Kirli OLMAYAN oturumda ikisi de sorgusuz geçer (§2.4 "onay nadirse okunur").
    let cihazVerisi: CihazVerisiAyari

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
         kapi: (any OnayKapisi)? = nil,
         raporlayici: (any AracRaporlayici)? = nil) {
        self.baglantiID = baglantiID
        self.baglantiAdi = baglantiAdi
        self.uzakAd = uzakAd
        // Varsayılan en kısıtlı seçenek: araç üretilirken ayar UNUTULURSA
        // davranış "gönderme" tarafına düşer, sessizce sızma tarafına değil.
        self.cihazVerisi = cihazVerisi
        self.name = Self.gecerliAd(uzakAd)
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
            if cihazVerisi.kapiyiAtlarMi {
                let not = kirli
                    ? String(localized: "onay sorulmadan gönderildi · bağlantı ayarı: her zaman izin ver")
                    : String(localized: "gönderildi · oturumda kişisel veri aracı kullanılmamıştı")
                return await uzagaCagir(argumanlar: "\(not)\n\n\(argumanlar)",
                                        gonderilen: argumanlar)
            }

            let onay = await kapi.onayIste(kaynak: baglantiAdi,
                                           aracAdi: uzakAd,
                                           icerik: argumanlar)
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
                    modeleDonen: "remote_call_cancelled: the call to the user's server was interrupted. Say this in one sentence."
                )
            } catch {
                let neden = Self.kisaHata(error)
                return AracSonucu(
                    cipMetni: "\(baglantiAdi) · erişilemedi",
                    durum: .basarisiz(neden),
                    modeleDonen: "remote_call_failed: the user's server could not be reached. Say this in one sentence; do not invent a result.",
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
    private static func gecerliAd(_ ham: String) -> String {
        let izinli = ham.map { karakter -> Character in
            karakter.isLetter || karakter.isNumber || karakter == "_" ? karakter : "_"
        }
        let ad = String(izinli)
        return ad.isEmpty ? "uzak_arac" : ad
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

// MARK: - Şema çevirisi (§5.2)

/// Şema düzleştirilemediğinde neden. Kullanıcıya "desteklenmiyor" olarak
/// listelenir — sessizce yutulmaz.
enum SemaHatasi: LocalizedError, Equatable {
    case cokDerin
    case duzlesmiyor(String)
    case bozukSema

    var errorDescription: String? {
        switch self {
        case .cokDerin:
            return String(localized: "Şeması fazla iç içe.")
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

    /// Tek aracın şemasını çevirir. Fırlatırsa araç desteklenmiyordur.
    static func cevir(tanim: MCPAracTanimi) throws -> GenerationSchema {
        let nesne = try sozluk(tanim.girdiSemasiJSON)
        let kok = try dugum(ad: tanim.ad, sema: nesne, derinlik: 0)
        return try GenerationSchema(root: kok, dependencies: [])
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
                              derinlik: Int) throws -> DynamicGenerationSchema {
        guard derinlik <= derinlikSiniri else { throw SemaHatasi.cokDerin }

        // anyOf/oneOf: önce düzleştirmeyi dener, olmuyorsa aracı atlatır.
        let sema = try birlesimiDuzlestir(ad: ad, sema: ham)
        let aciklama = sema["description"] as? String

        switch try tur(sema) {
        case "object":
            let alanlar = sema["properties"] as? [String: Any] ?? [:]
            let zorunlu = Set(sema["required"] as? [String] ?? [])
            // Anahtar sırası deterministik olsun: aynı sunucu her açılışta
            // aynı şemayı üretsin.
            let ozellikler = try alanlar.keys.sorted().map { anahtar -> DynamicGenerationSchema.Property in
                guard let alt = alanlar[anahtar] as? [String: Any] else {
                    throw SemaHatasi.duzlesmiyor(anahtar)
                }
                let altSema = try dugum(ad: "\(ad)_\(anahtar)",
                                        sema: alt,
                                        derinlik: derinlik + 1)
                return DynamicGenerationSchema.Property(
                    name: anahtar,
                    description: alt["description"] as? String,
                    schema: altSema,
                    isOptional: !zorunlu.contains(anahtar)
                )
            }
            return DynamicGenerationSchema(name: ad, description: aciklama, properties: ozellikler)

        case "array":
            guard let oge = sema["items"] as? [String: Any] else {
                throw SemaHatasi.duzlesmiyor(ad)
            }
            let ogeSema = try dugum(ad: "\(ad)_oge", sema: oge, derinlik: derinlik + 1)
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
