//
//  EvalZincir.swift
//  Tacet
//
//  "Zincir oturum" eval altyapısı.
//
//  Ayrık (tekil) koşum her vakayı TEMİZ bir oturumda çalıştırır ve tam da bu
//  yüzden bir hata sınıfını hiç göremez: oturum İÇİNDE biriken durumun
//  bozulmasını. Bağlam bütçesinin dolup özetlemenin tetiklenmesi, profil/imza
//  değişince oturumun yeniden kurulup önceki turun bilgisini düşürmesi, önceki
//  turda üretilen belgeye ("bunu pdf yap") atıf, `VeriDeposu` ref'lerinin LRU
//  tavanına takılıp turlar arası kaybolması, hafıza çıkarımının biriken
//  mesajlardan tetiklenmesi, araç bütçesinin (kod-spec §5.4) tur sınırında
//  sıfırlanmaması, `akanMetin`in turlar arası yarışı — hepsi YALNIZCA arka
//  arkaya gelen turlarda oluşur.
//
//  Bu dosya iki şey sunar:
//   1. `ZincirVaka` / `ZincirTur` — vaka yazarlarının kullanacağı veri tipleri.
//      Beklenti kelime dağarcığı `TestVaka` ile BİREBİR aynıdır; yeni bir
//      puanlama dili YOKTUR, puanlama `EvalPuan.puanla` ile yapılır.
//   2. `Degerlendirme.zincirKosusu` — vaka başına TEK oturum kuran, turları
//      sırayla koşan, her turu ayrı puanlayan ve zincire özgü ölçümleri
//      kaydeden yürütücü.
//
//  Puanlama semantiği DEĞİŞMEZ: geçme puanı `EvalKapisi.gecmePuani` (80),
//  koşum eşiği `EvalKapisi.esik` (0.75), uydurma dedektörü aynı.
//

#if DEBUG
import Foundation
import SwiftData

// MARK: - Vaka tipleri

/// Zincirin TEK turu. Alan adları ve anlamları `TestVaka` ile birebir aynı
/// tutuldu: vaka yazan ajan iki tip arasında zihinsel çeviri yapmak zorunda
/// kalmasın ve puanlayıcıya (`EvalPuan.puanla`) ikinci bir yol açılmasın.
struct ChainKind {
    /// Bu turda kullanıcının yazdığı istem.
    let prompt: String
    /// Beklenen çip ikon ÖNEKLERİ — hepsi düşmeli. (`doc` öneki
    /// `doc.text.image`i de eşler; biçim ayrımı isteyen vaka tam adı yazmalı.)
    var ikonlar: [String] = []
    /// Bu turda hiç araç çağrılmamalı (netleştirme / sohbet turları).
    var cipYok = false
    /// Yanıtta GEÇMESİ gereken parça.
    var yanitIcermeli: String? = nil
    /// Yanıtta GEÇMEMESİ gereken parça — uydurma tespiti (en ağır ceza).
    var yanitIcermemeli: String? = nil
    /// Araç ARGÜMANINDA geçmesi gereken parçalar. Çip ikonu doğru aracın
    /// çağrıldığını söyler, argümanın doğru olduğunu SÖYLEMEZ.
    var girdiIcermeli: [String] = []
    /// Araç ÇIKTISINDA geçmesi gereken parçalar. Sayıyı aracın söylemesi
    /// gerekir; modelin yanıtında yazması aracın doğru çalıştığının kanıtı değil.
    var ciktiIcermeli: [String] = []
}

/// Tek oturumda arka arkaya koşulacak senaryo.
///
/// DİKKAT: turlar arasında `sohbetiSifirla()` ÇAĞRILMAZ — ölçülen şey tam da
/// oturumda biriken durumdur. Zincirin turları bölünemez; shard'lama zinciri
/// TEK ELEMAN olarak dağıtır (bkz. `EvalRapor.shardSec`).
struct ChainCase {
    /// Rapor satırlarında görünen ad. Kategori ile karışmasın diye `zincir-`
    /// önekiyle başlaması beklenir (zorunlu değil, ama rapor okunurluğu için).
    let name: String
    /// Bu zincirin NEYİ ölçtüğü — tek cümle. Rapora girmez, kodu okuyan içindir.
    let description: String
    /// Sıralı turlar.
    let turlar: [ChainKind]
    /// Zincir başında test belgesi eklensin mi (oku/düzenle senaryoları).
    var attachedDocument = false
    /// Kontrol koşumu ("bagimsiz": her tur öncesi sıfırlama) da yapılsın mı.
    ///
    /// VARSAYILAN AÇIK, çünkü zincir puanı TEK BAŞINA yorumlanamaz: bir turun
    /// düşük puanı bağlam taşımanın kusuru mu, yoksa o istemin zaten zor
    /// olması mı? Kontrol koşumu bu ikisini ayırır (`EvalRapor.ozet`in
    /// "Zincir vs Bağımsız" bölümü bu çiftten hesaplanır). Yalnızca kontrolü
    /// ANLAMSIZ olan zincirlerde kapatın — ör. tur N'in istemi tur N-1'in
    /// çıktısına dilbilgisel olarak bağımlıysa ("onu 10:00'a al") bağımsız
    /// koşum bir şey ölçmez, yalnız süre yakar.
    var karsilastir = true
    /// Hafıza çıkarımı KAÇINCI turdan SONRA tetiklensin (1 tabanlı); nil = hiç.
    ///
    /// Üretimde ayıklama `sohbetiSifirla()` anında çalışır, yani tek oturum
    /// içinde HİÇ çalışmaz. Zincir bunu taklit eder: belirtilen turdan sonra
    /// o ana kadarki mesajlar bellek-içi bir `Sohbet`e yazılır, `HafizaServisi`
    /// çalıştırılır ve çıkan notlar `HafizaDeposu`ya konur — böylece SONRAKİ
    /// turlar notu görebilir. Üretim deposu zincir sonunda geri yüklenir.
    var hafizaAyiklaTuru: Int? = nil
}

extension ChainCase {
    /// Eski `EvalVakalari.EvalZincir` korpusunu yeni tipe taşır.
    ///
    /// Korpus SİLİNMİYOR: 16 zincir zaten ölçülmüş veridir ve tek yürütücüye
    /// geçmenin bedeli, onları elle kopyalamak değil, bir dönüştürücü olmalı.
    /// `beklenenler` adım sayısından kısaysa kalan turlar çip beklentisiz sayılır
    /// (eski koşucunun davranışının aynısı).
    init(eski: EvalCases.EvalChain) {
        self.name = eski.name
        self.description = eski.description
        self.turlar = eski.steps.enumerated().map { i, prompt in
            ChainKind(prompt: prompt,
                      ikonlar: i < eski.beklenenler.count ? eski.beklenenler[i] : [])
        }
        // Eski koşucu ekli belgeyi ADA BAKARAK açıyordu; davranışı korumak için
        // aynı kural burada tek yerde duruyor.
        self.attachedDocument = eski.name.contains("belge-oku")
        self.karsilastir = true
        self.hafizaAyiklaTuru = nil
    }
}

// MARK: - Zincire özgü ölçümler

/// Bir zincir turunun, PUANDAN bağımsız ölçümleri.
///
/// Bunlar 1. turda değişen mekanizmaların (özetleme önbelleği, oturum
/// bölünmesi, `VeriDeposu` LRU tavanı, kod deneme sayacı, akış performansı)
/// gerileme sinyalleridir. Puana GİRMEZLER — ölçülmeyen bir mekanizmanın
/// bozulduğu ancak aylar sonra fark edilir, ama ölçümü puana bağlamak da
/// eşiği sessizce kaydırmak olurdu.
struct ChainMeasurement {
    var turNo: Int
    /// Turda hiç araç çağrıldı mı, kaç tane.
    var aracSayisi: Int
    /// Seyirden okunan profil etiketi ("gündelik profil", "belge profili" …).
    var profile: String
    /// Profil bir ÖNCEKİ tura göre değişti — oturum yeniden kuruldu demektir
    /// (`ModelServisi.yanitSonucu` profil değişiminde `oturumKur` çağırır).
    var oturumYenidenKuruldu: Bool
    /// Aynı tur içinde İKİNCİ bir yönlendirme adımı — tur-içi profil kurtarma
    /// (P1-2) çalıştı, yani oturum tur ortasında bir kez daha kuruldu.
    var turIciKurtarma: Bool
    /// Özetleme ŞÜPHESİ. Kesin sinyal değil (bkz. dosya sonundaki not):
    /// araç çağrılmamışken ilk token `ozetlemeSuphesiEsigiMs`i aşarsa,
    /// aradaki süre üretim değil hazırlıktır ve tek uzun hazırlık adımı
    /// `ozetle()`nin LLM çağrısıdır.
    var ozetlemeSuphesi: Bool
    /// İlk akış parçasının geldiği an (ms). Akış hiç parça üretmediyse nil.
    var ilkTokenMs: Int?
    var sureMs: Int
    /// Tur sonunda `calisilabilirBelge` — "bunu pdf yap" atfının yaşayıp
    /// yaşamadığı buradan okunur.
    var belgeAdi: String?
    /// Tur sonunda `VeriDeposu` ref sayısı — yeni LRU tavanı (24 kayıt)
    /// zincirin ortasında ref düşürüyorsa burada görünür.
    var refSayisi: Int
    /// Turdaki gerçek kod çalıştırma sayısı (kod-spec §5.4 tavanı 2).
    /// Tur sınırında sıfırlanmıyorsa ikinci kod turu sessizce reddedilir.
    var kodDenemesi: Int
    /// Hafıza ayıklaması bu turdan sonra koştuysa çıkan not sayısı.
    var memoryNote: Int?
    var zamanAsimi: Bool

    /// Tek satırlık makine-okunur özet.
    ///
    /// `EvalSonuc`ta zincir ölçümleri için ALAN YOK ve o dosya bu turun
    /// sahipliğinde değil; ölçüm bu yüzden `hamCiktilar`a PUANLAMADAN SONRA
    /// eklenir (puana etki edemez) ve ham JSON'da bu önekle aranabilir.
    var line: String {
        var p = ["ZINCIR-OLCUM",
                 "tur=\(turNo)",
                 "arac=\(aracSayisi)",
                 "profil=\(profile.isEmpty ? "?" : profile)",
                 "oturum-kuruldu=\(oturumYenidenKuruldu ? 1 : 0)",
                 "tur-ici-kurtarma=\(turIciKurtarma ? 1 : 0)",
                 "ozetleme-suphesi=\(ozetlemeSuphesi ? 1 : 0)",
                 "ilk-token-ms=\(ilkTokenMs.map(String.init) ?? "-")",
                 "sure-ms=\(sureMs)",
                 "belge=\(belgeAdi ?? "-")",
                 "ref=\(refSayisi)",
                 "kod-deneme=\(kodDenemesi)"]
        if let memoryNote { p.append("hafiza-notu=\(memoryNote)") }
        if zamanAsimi { p.append("takildi=1") }
        return p.joined(separator: "|")
    }
}

// MARK: - Yürütücü

@MainActor
extension Evaluation {

    /// Araç çağrılmamış bir turda ilk tokenin bu kadar gecikmesi, üretimden
    /// ÖNCE uzun bir hazırlık adımı çalıştığını söyler. Tek uzun hazırlık
    /// adımı `ozetle()`nin ayrı LLM çağrısıdır (kısa geçmişte o çağrı hiç
    /// yapılmaz, ham kuyruk taşınır — yani eşiği aşan tur zaten uzun oturum
    /// olan turdur). 2.5 sn, tek süreçlik koşumda ölçülen araçsız tur ilk-token
    /// gecikmesinin (birkaç yüz ms) kat kat üstünde.
    static var ozetlemeSuphesiEsigiMs: Int { 2500 }

    /// Bir zincir vakasını TEK oturumda, turları SIRAYLA koşar.
    ///
    /// - Bir tur düşse (zaman aşımı, boş yanıt, eksik araç) bile KALAN TURLAR
    ///   koşulur: zincirin nerede koptuğu ancak kopuştan sonrası da ölçülürse
    ///   görülür. Düşen tur `olculemedi` ile işaretlenir ve ortalamaya girmez.
    /// - Her tur `adimBitti` ile ANINDA çağırana geri verilir; çağıran diske
    ///   yazar. Koşu yarıda kalsa da eldeki turlar rapordadır.
    /// - `mod` doğrudan `EvalSonuc.mod`a yazılır ve "zincir" | "bagimsiz"
    ///   dışında bir değer ALMAMALIDIR: `EvalRapor.ozet` karşılaştırmayı bu iki
    ///   dizgeyle yapıyor.
    static func zincirKosusu(_ service: ModelService,
                             testCase: ChainCase,
                             mod: String,
                             testBelge: URL?,
                             adimBitti: (EvalResult, [String]) async -> Void) async {

        // — Hafıza kurulumu (yalnız gerçek zincir koşumunda anlamlı) —
        // Ayıklama bellek-içi bir depoda çalışır; üretim notları zincir sonunda
        // geri konur. Konteyner zincir boyunca CANLI tutulur: bağlamı ölen bir
        // @Model nesnesini depoda bırakmak SwiftData'da tanımsız davranıştır.
        let hafizaIster = testCase.hafizaAyiklaTuru != nil && mod == "zincir"
        let uretimNotlari = MemoryStore.notes
        var kutular: [ModelContainer] = []
        var record: ModelContext?
        var chat: Chat?
        if hafizaIster {
            let schema = Schema(versionedSchema: SchemaV1.self)
            let config = ModelConfiguration(schema: schema, isStoredInMemoryOnly: true)
            if let kutu = try? ModelContainer(for: schema, configurations: config) {
                kutular.append(kutu)
                let k = kutu.mainContext
                let s = Chat()
                k.insert(s)
                try? k.save()
                record = k
                chat = s
            }
        }

        /// Oturumu (yeniden) kurar — zincirde yalnız başta, bağımsızda her turda.
        func oturumuKur() {
            service.resetChat()
            if testCase.attachedDocument, let testBelge {
                service.documentContext.addDocument(url: testBelge)
            }
        }

        if mod != "bagimsiz" { oturumuKur() }

        var oncekiProfil = ""
        for (i, t) in testCase.turlar.enumerated() {
            let turNo = i + 1
            if mod == "bagimsiz" {
                oturumuKur()
                // Bağımsız koşumda "önceki tur" diye bir şey yok; profil
                // değişimi ölçümü anlamsızlaşmasın diye referans sıfırlanır.
                oncekiProfil = ""
            }

            let kind = await turKos(service, t.prompt)

            // — Seyirden okunan oturum sinyalleri —
            // `seyir.adimlar` bir sonraki tura kadar duruyor; tur bittikten
            // hemen sonra okunması güvenli. Profil adı `yönlendirildi · X`
            // biçiminde geliyor.
            let yonlendirmeler = service.timeline.steps
                .filter { $0.kind == .routing }
                .map(\.text)
            let profile = yonlendirmeler.last.map(Self.profilEtiketi) ?? ""
            let yenidenKuruldu = !oncekiProfil.isEmpty && !profile.isEmpty && profile != oncekiProfil
            if !profile.isEmpty { oncekiProfil = profile }

            let ilkToken = kind.ilkTokenMs
            let ozetlemeSuphesi = kind.traces.isEmpty
                && (ilkToken ?? 0) >= ozetlemeSuphesiEsigiMs

            var measure = ChainMeasurement(
                turNo: turNo,
                aracSayisi: kind.traces.count,
                profile: profile,
                oturumYenidenKuruldu: yenidenKuruldu,
                turIciKurtarma: yonlendirmeler.count > 1,
                ozetlemeSuphesi: ozetlemeSuphesi,
                ilkTokenMs: ilkToken,
                sureMs: kind.sureMs,
                belgeAdi: service.documentContext.runnableDocument?.name,
                refSayisi: service.dataStore.refAnahtarlari.count,
                kodDenemesi: service.codeState.attempt,
                memoryNote: nil,
                zamanAsimi: kind.zamanAsimi)

            // — Hafıza: mesajları biriktir, istenen turdan sonra ayıkla —
            if let record, let chat {
                let soruMesaji = Message(role: .user, content: t.prompt)
                soruMesaji.chat = chat
                record.insert(soruMesaji)
                if !kind.text.isEmpty {
                    let yanitMesaji = Message(role: .tacet, content: kind.text)
                    yanitMesaji.chat = chat
                    record.insert(yanitMesaji)
                }
                try? record.save()
                if testCase.hafizaAyiklaTuru == turNo {
                    await MemoryService().extract(chat: chat, record: record)
                    let notlar = ((try? record.fetch(FetchDescriptor<MemoryNote>())) ?? [])
                        .filter { !$0.isDeleted }
                    measure.memoryNote = notlar.count
                    // Sonraki turlar notu GÖRSÜN: üretimde enjeksiyon deposu
                    // budur, taklidi eksik kalırsa "not çıktı ama uygulanmadı"
                    // hatası hiç ölçülemez.
                    MemoryStore.reload(notlar)
                }
            }

            // — Puanlama: mevcut mekanizma, yeni dil YOK —
            var s = EvalResult(vakaAd: testCase.name,
                              category: "zincir",
                              mod: mod,
                              adimNo: turNo,
                              prompt: t.prompt,
                              beklenenCipler: t.ikonlar,
                              gercekCipler: kind.traces.map(\.icon),
                              reply: kind.text,
                              sureMs: kind.sureMs,
                              hamGirdiler: kind.traces.compactMap(\.rawInput),
                              hamCiktilar: kind.traces.compactMap(\.rawOutput),
                              errorClass: kind.errorClass == .none ? nil : kind.errorClass.rawValue)
            s = EvalScore.score(s,
                                cipYok: t.cipYok,
                                yanitIcermeli: t.yanitIcermeli,
                                yanitIcermemeli: t.yanitIcermemeli,
                                basarisizCipVar: kind.basarisizCip,
                                girdiIcermeli: t.girdiIcermeli,
                                ciktiIcermeli: t.ciktiIcermeli)
            if kind.zamanAsimi { s.sorunlar.append("zaman-asimi"); s.olculemedi = true }

            // Ölçüm satırı PUANLAMADAN SONRA ekleniyor: `hamCiktilar` puanlayıcının
            // `ciktiIcermeli` havuzudur, önce eklenirse ölçüm metni bir beklentiyi
            // yanlışlıkla karşılayabilirdi.
            s.hamCiktilar = (s.hamCiktilar ?? []) + [measure.line]

            var kayitlar = lines(s)
            kayitlar.insert("    ⧉ \(measure.line)", at: max(0, kayitlar.count - 1))
            await adimBitti(s, kayitlar)
            try? await Task.sleep(for: nefes)
        }

        // — Üretim hafızasını geri koy: eval kullanıcının notlarını değiştirmez —
        if hafizaIster { MemoryStore.reload(uretimNotlari) }
        kutular.removeAll()
    }

    /// "yönlendirildi · belge profili" → "belge profili".
    /// Ayraç yoksa metnin kendisi döner (yerelleştirme değişirse ölçüm sessizce
    /// boşalmasın diye).
    static func profilEtiketi(_ text: String) -> String {
        guard let separator = text.range(of: "·") else {
            return text.trimmingCharacters(in: .whitespaces)
        }
        return String(text[separator.upperBound...]).trimmingCharacters(in: .whitespaces)
    }
}

// MARK: - ÖLÇÜMÜN SINIRLARI (bilerek yazıldı)
//
// `ozetlemeSuphesi` bir ŞÜPHEDİR, olgu değil. Kesin ölçüm `ModelServisi`nin
// içinden gelmeli: `oturumKur` ve `ozetle` çağrı sayaçları (`private(set) var`)
// açılırsa bu alan tahminden ölçüme döner. O dosya bu turun sahipliğinde
// olmadığı için sözleşme ucu olarak raporlandı.
//
// Aynı biçimde `oturumYenidenKuruldu` YALNIZCA profil değişimini görür:
// araç imzası değişimi, dil değişimi ve bütçe aşımı da oturumu yeniden kurar
// ama seyire yeni bir `yonlendirme` adımı DÜŞÜRMEZ. Yani bu alan yanlış
// pozitif üretmez, yanlış NEGATİF üretir — eksik saydığı bilinerek okunmalı.
#endif
