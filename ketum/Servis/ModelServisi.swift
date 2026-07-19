//
//  ModelServisi.swift
//  ketum
//
//  Model katmanı (spec §7.1, §7.2). Yalnızca on-device SystemLanguageModel;
//  Private Cloud Compute kullanılmaz. Availability bir feature flag gibi ele
//  alınır. Bağlam penceresi (4096) aktif yönetilir; taşma sessizce kurtarılır.
//

import Foundation
import Observation
import FoundationModels
import NaturalLanguage

@MainActor
@Observable
final class ModelServisi {

    /// Oturumun durumu. Ekranda renkli gösterge yoktur; durum yalnızca sözle anlatılır.
    enum Durum: Equatable {
        case hazir                    // sessiz — anlatılacak bir şey yok
        case hazirlaniyor             // gri · "hazırlanıyor…"
        case kullanilamaz(String)     // gri · neden

        var etiket: String {
            switch self {
            case .hazir: return String(localized: "Cihazında")
            case .hazirlaniyor: return String(localized: "hazırlanıyor…")
            case .kullanilamaz(let neden):
                return neden.isEmpty ? String(localized: "bu cihazda kullanılamıyor") : neden
            }
        }
        var hazirMi: Bool { self == .hazir }
    }

    private(set) var durum: Durum = .hazirlaniyor
    /// Araç çipi tek doğruluk kaynağı — tools buraya rapor eder, UI buradan okur.
    let yurutucu = AracYurutucu()
    /// Sohbete paylaşılan/üretilen belgeler — belge araçları buraya erişir, UI önizler.
    let belgeBaglami = BelgeBaglami()
    /// Büyük veri taşıma kanalı (spec §7.3.2) — toplu veri modelden geçmeden araçlar arası taşınır.
    let veriDeposu = VeriDeposu()
    /// Nöbet (zamanlanmış ajan) kurma bağlamı — NobetAraci buraya erişir.
    let nobetBaglami = NobetBaglami()

    /// Araç profili (spec §7.3.1): 4096 pencerede oturuma en fazla 6–8 araç verilir.
    enum Profil { case gundelik, belge }
    private var aktifProfil: Profil = .gundelik
    /// Oturuma gömülü dil adı (yanıt-dili çapası). Kullanıcının dili saptanınca güncellenir.
    private var aktifDil: String = ""
    /// Mevcut oturumun kurulduğu dil — gereksiz yeniden kurmayı önler.
    private var oturumDili: String = ""
    /// Aktif dil kullanıcının açık seçiminden mi geliyor (saptamadan değil) — talimat metnini değiştirir.
    private var dilSecildi: Bool = false
    /// Oturumun kurulduğu andaki seçim durumu — talimat metni buna göre yazıldığı için izlenir.
    private var oturumDilSecildi: Bool = false


    private let model = SystemLanguageModel.default
    private var oturum: LanguageModelSession?

    /// Bağlam bütçesi eşiği: contextSize'ın %80'i (araştırma raporu §5.2).
    private let esikOran = 0.80

    init() { availabilityKontrol() }

    // MARK: - Availability

    func availabilityKontrol() {
        switch model.availability {
        case .available:
            durum = .hazir
            oturumKur(profil: .gundelik)
        case .unavailable(let neden):
            switch neden {
            case .deviceNotEligible:
                durum = .kullanilamaz(String(localized: "bu cihazda kullanılamıyor"))
            case .appleIntelligenceNotEnabled:
                durum = .kullanilamaz(String(localized: "Apple Intelligence kapalı"))
            case .modelNotReady:
                durum = .hazirlaniyor
            @unknown default:
                durum = .kullanilamaz(String(localized: "bu cihazda kullanılamıyor"))
            }
        }
    }

    // MARK: - Oturum ve profiller

    /// Gündelik profil (spec §8, v1): Takvim, Hatırlatıcı, Kişi, Arama, Hesap, Zaman.
    private func gundelikAraclar() -> [any Tool] {
        var takvim = TakvimAraci();          takvim.raporlayici = yurutucu; takvim.veriDeposu = veriDeposu
        var hatirlatici = HatirlaticiAraci(); hatirlatici.raporlayici = yurutucu
        var kisi = KisiAraci();              kisi.raporlayici = yurutucu
        var arama = AramaAraci();            arama.raporlayici = yurutucu
        var hesap = HesapAraci();            hesap.raporlayici = yurutucu
        var zaman = ZamanAraci();            zaman.raporlayici = yurutucu
        var nobet = NobetAraci();            nobet.raporlayici = yurutucu; nobet.baglam = nobetBaglami
        return [takvim, hatirlatici, kisi, arama, hesap, zaman, nobet]
    }

    /// Belge/üretim profili (spec §7.3.1): Oluştur, Oku, Düzenle + veri kaynağı ve yardımcılar.
    private func belgeAraclar() -> [any Tool] {
        var olustur = BelgeOlusturAraci(); olustur.raporlayici = yurutucu; olustur.baglam = belgeBaglami; olustur.veriDeposu = veriDeposu
        var oku = BelgeOkuAraci();         oku.raporlayici = yurutucu;      oku.baglam = belgeBaglami
        var duzenle = BelgeDuzenleAraci(); duzenle.raporlayici = yurutucu;  duzenle.baglam = belgeBaglami
        var takvim = TakvimAraci();        takvim.raporlayici = yurutucu;   takvim.veriDeposu = veriDeposu
        var kisi = KisiAraci();            kisi.raporlayici = yurutucu
        var hesap = HesapAraci();          hesap.raporlayici = yurutucu
        var zaman = ZamanAraci();          zaman.raporlayici = yurutucu
        return [olustur, oku, duzenle, takvim, kisi, hesap, zaman]
    }

    private func araclariYap(_ profil: Profil) -> [any Tool] {
        switch profil {
        case .gundelik: return gundelikAraclar()
        case .belge:    return belgeAraclar()
        }
    }

    // MARK: - Beceriler (progressive disclosure)

    /// Beceri kılavuzları oturum TALİMATINA gömülmez: ölçümler, küçük on-device
    /// modelin fazla sabit talimatla araçları çağırmak yerine "anlatmaya" başladığını
    /// gösterdi. Bunun yerine Claude'un SKILL.md mantığı uygulanır — yalnızca o
    /// mesaja uyan TEK beceri, o oturuma BİR KEZ, o turun istemine iliştirilir.
    /// Böylece hem sabit talimat kısa kalır hem rehberlik gerektiği anda gelir.
    private var enjekteBeceriler: Set<String> = []

    /// Soruya beceri eşleşirse kılavuzu bu turun istemine iliştirir; yoksa soruyu aynen döner.
    private func beceriliIstem(_ soru: String) -> String {
        guard let beceri = BeceriDeposu.eslesen(soru),
              !enjekteBeceriler.contains(beceri.ad) else { return soru }
        enjekteBeceriler.insert(beceri.ad)
        return BeceriDeposu.enjeksiyonMetni(beceri) + "\n\n" + soru
    }

    private func oturumKur(profil: Profil, devam ozet: String? = nil) {
        aktifProfil = profil
        // Yeni oturum = yeni bağlam: enjekte edilmiş beceriler artık transcript'te yok.
        enjekteBeceriler.removeAll()
        let dil = aktifDil
        let secildi = dilSecildi
        let temel = LanguageModelSession(tools: araclariYap(profil)) {
            Yonlendirici.talimatlar
            if !dil.isEmpty {
                // Yanıt-dili çapası: adlandırılmış dil direktifi (sızıntıyı azaltır).
                if secildi {
                    "\n\nThe user has chosen \(dil) as the reply language. Reply ONLY in \(dil), never in another language, even if the user writes in a different language."
                } else {
                    "\n\nThe user is writing in \(dil). Reply ONLY in \(dil), never in another language."
                }
            }
            if let ozet {
                "\n\nÖnceki konuşmanın özeti: \(ozet)"
            }
        }
        oturum = temel
        oturumDili = dil
        oturumDilSecildi = secildi
        // Prewarm: executor'ı ısıt, ilk-token gecikmesini düşür (rapor §5.1).
        temel.prewarm()
    }

    /// Kullanıcı mesajının dilini cihaz-üstü saptar (NaturalLanguage). Kısa/belirsizde nil.
    private func algilananDil(_ metin: String) -> String? {
        let t = metin.trimmingCharacters(in: .whitespacesAndNewlines)
        guard t.count >= 2 else { return nil }
        let tanıyıcı = NLLanguageRecognizer()
        tanıyıcı.processString(t)
        guard let dil = tanıyıcı.dominantLanguage else { return nil }
        let olasilik = tanıyıcı.languageHypotheses(withMaximum: 1)[dil] ?? 0
        guard olasilik >= 0.5 else { return nil }
        return Self.dilAdlari[dil.rawValue]
    }

    private static let dilAdlari: [String: String] = [
        "tr": "Turkish", "en": "English", "zh-Hans": "Chinese", "zh-Hant": "Chinese",
        "zh": "Chinese", "ja": "Japanese", "es": "Spanish", "de": "German",
        "fr": "French", "ko": "Korean", "pt": "Portuguese", "pt-BR": "Portuguese",
        "it": "Italian",
    ]

    /// Kullanıcı bir yanıt dili seçtiyse onun İngilizce adı; "otomatik" ise nil.
    /// Seçim varken dil saptama devre dışıdır — tercih her turda kazanır.
    private var secilenDilAdi: String? {
        let kod = DilTercihi.paylasilan.yanitDili
        guard !kod.isEmpty else { return nil }
        return Self.dilAdlari[kod]
    }

    /// Sohbet yüzeyi görünür olunca çağrılır — kullanıcı yazmadan modeli ısıtır.
    func hazirla() { oturum?.prewarm() }

    /// Yeni sohbete geçişte model bağlamını, veri deposunu ve ekli belgeyi sıfırlar.
    func sohbetiSifirla() {
        veriDeposu.temizle()
        belgeBaglami.belgeKaldir()
        belgeBaglami.uretimiUnut()
        belgeBaglami.onizlenecek = nil
        yurutucu.yeniTur()
        // Sıfırlama saptanan dili unutur ama kullanıcının açık seçimini EZMEZ.
        aktifDil = secilenDilAdi ?? ""
        dilSecildi = secilenDilAdi != nil
        enjekteBeceriler.removeAll()
        // Tembel: oturumu şimdi kurma. İlk mesajda dil saptanıp tek seferde kurulur
        // (yeni sohbet başına çift kurulumu önler).
        oturum = nil
        oturumDili = ""
        oturumDilSecildi = false
    }

    /// Niyet sınıflandırması (spec §7.3.1). Gereksiz oturum yeniden kurmayı önlemek için
    /// mevcut profil isteği karşılayabiliyorsa DEĞİŞTİRMEZ — yalnızca o profilde olmayan
    /// bir araç gerektiğinde geçiş yapar. İki profil de Takvim/Kişi/Hesap/Zaman'ı paylaşır.
    private func niyetProfili(_ soru: String, mevcut: Profil) -> Profil {
        // Ekli belge ya da az önce üretilmiş dosya varsa devam isteği belge profilinde
        // kalmalı — "onu tablo olarak göster" gibi cümlelerde biçim adı geçmeyebilir.
        if belgeBaglami.calisilabilirBelge != nil { return .belge }
        let s = soru.lowercased()
        // Gündelik profile ÖZGÜ araçlar (Hatırlatıcı, Arama) — 8 dilde tetikleyiciler.
        if Self.gundelikIzleri.contains(where: s.contains) { return .gundelik }
        // Güçlü belge/biçim sinyalleri (biçim adları dil-nötr; ad-fiiller 8 dilde).
        if Self.belgeIzleri.contains(where: s.contains) { return .belge }
        // Aksi halde mevcut profili koru (paylaşılan araçlar her iki profilde çalışır).
        return mevcut
    }

    /// Hatırlatıcı/arama niyeti (gündelik profil) — tr/en/zh/ja/es/de/fr/ko/pt.
    private static let gundelikIzleri = [
        "hatırlat", "hatirlat", "anımsat", "notlarım", "notlarda",          // tr
        "remind", "reminder", "my note", "notes", "search my",              // en
        "提醒", "备忘", "笔记", "搜索",                                          // zh
        "リマインド", "思い出させ", "メモ", "検索",                                // ja
        "recuérda", "recordar", "recordatorio", "mis notas", "buscar",      // es
        "erinner", "notiz", "suche", "meine noti",                          // de
        "rappelle", "rappel", "mes notes", "cherche",                       // fr
        "알림", "리마인더", "메모", "검색",                                       // ko
        "lembre", "lembrete", "minhas notas", "procur",                     // pt
    ]

    /// Belge/dosya niyeti (belge profil) — biçim adları + 8 dilde ad-fiiller.
    private static let belgeIzleri = [
        "excel", "xlsx", "pdf", "word", "docx", "markdown", ".md",          // dil-nötr
        "belge", "dosya", "tablo", "çizelge", "rapor", "döküm", "dök",      // tr
        "document", "file", "spreadsheet", "table", "report", "export",     // en
        "文档", "文件", "表格", "报告", "列表",                                   // zh
        "ドキュメント", "ファイル", "表", "レポート", "リスト",                     // ja
        "documento", "archivo", "tabla", "informe", "hoja de",              // es
        "dokument", "datei", "tabelle", "bericht", "tabellen",              // de
        "fichier", "tableau", "rapport", "feuille",                         // fr
        "문서", "파일", "표", "보고서", "목록",                                   // ko
        "arquivo", "tabela", "relatório", "planilha",                       // pt
    ]

    // MARK: - Yanıt (streaming)

    /// Kullanıcı sorusuna akışlı yanıt üretir. `akis` her kısmi metinle çağrılır.
    /// Dönüş: (son metin, bu turun araç çipleri). Hata kullanıcıya sızmadan kurtarılır.
    func yanitla(_ soru: String, akis: @escaping (String) -> Void) async -> (metin: String, izler: [AracIzi]) {
        guard durum.hazirMi else {
            return (Yerel.modelHazirDegil, [])
        }
        // isResponding kilidi (rapor §5.1): model yanıtlarken paralel istek açma.
        if oturum?.isResponding == true {
            return (Yerel.oncekiBitiyor, [])
        }
        yurutucu.yeniTur()

        // Profil + dil yönlendirmesi: oturum yoksa ya da profil/dil değişince tek seferde kur.
        let istenen = niyetProfili(soru, mevcut: aktifProfil)
        // Tercih varsa saptama çalışmaz; tercih değişince aktifDil de değişir ve
        // aşağıdaki `aktifDil != oturumDili` koşulu oturumu yeni dille yeniden kurar.
        if let secilen = secilenDilAdi {
            aktifDil = secilen
            dilSecildi = true
        } else {
            // Açık seçimden Otomatik'e dönüşte zorlanmış dil takılı kalmasın:
            // saptama temiz sayfadan başlasın, aksi halde eşiği geçemeyen kısa
            // girdilerde yanıt eski seçilen dilde gelmeye devam ederdi.
            if dilSecildi { aktifDil = "" }
            dilSecildi = false
            aktifDil = algilananDil(soru) ?? aktifDil
        }
        if oturum == nil || istenen != aktifProfil || aktifDil != oturumDili || dilSecildi != oturumDilSecildi {
            oturumKur(profil: istenen, devam: await ozetle())
        }
        await butceKontrol()
        guard let oturum else { return (Yerel.modelHazirDegil, []) }

        // Eşleşen beceri kılavuzu bu turun istemine iliştirilir (oturum başına bir kez).
        let istem = beceriliIstem(soru)
        do {
            let sonMetin = try await akisYut(oturum, soru: istem, akis: akis)
            return (sonMetin, yurutucu.izler)
        } catch {
            return await hataKurtar(error, soru: istem, akis: akis)
        }
    }

    /// Hata taksonomisi (rapor §5.5): taşma → görünmez kurtarma; guardrail/dil → retry YOK.
    private func hataKurtar(_ error: Error,
                            soru: String,
                            akis: @escaping (String) -> Void) async -> (metin: String, izler: [AracIzi]) {
        akis("")  // yarım akan metni temizle
        if let g = error as? LanguageModelSession.GenerationError {
            switch g {
            case .guardrailViolation:
                // Kurtarılamaz — pil yakmadan tek cümle (retry yok).
                return (Yerel.sinirDisi, yurutucu.izler)
            case .unsupportedLanguageOrLocale:
                return (Yerel.dilDesteklenmiyor, yurutucu.izler)
            case .exceededContextWindowSize:
                // Kurtarılabilir: özetle, oturumu yeniden kur, bir kez dene.
                yurutucu.yeniTur()
                oturumKur(profil: aktifProfil, devam: await ozetle())
                if let yeni = oturum, let m = try? await akisYut(yeni, soru: soru, akis: akis) {
                    return (m, yurutucu.izler)
                }
                return (Yerel.tekrarDene, yurutucu.izler)
            default:
                break
            }
        }
        // Diğer geçici hatalar: taze oturumla bir kez daha dene.
        yurutucu.yeniTur()
        oturumKur(profil: aktifProfil)
        if let yeni = oturum, let m = try? await akisYut(yeni, soru: soru, akis: akis) {
            return (m, yurutucu.izler)
        }
        return (Yerel.tekrarDene, yurutucu.izler)
    }

    private func akisYut(_ oturum: LanguageModelSession,
                         soru: String,
                         akis: @escaping (String) -> Void) async throws -> String {
        var son = ""
        let stream = oturum.streamResponse(to: soru)
        for try await parca in stream {
            son = parca.content
            akis(son)
        }
        return son
    }

    // MARK: - Bağlam bütçesi (rapor §5.2 — gerçek token ölçümü)

    /// Transcript'in gerçek token sayısını `tokenCount` ile ölçer; contextSize'ın
    /// %80'ini aşınca oturumu özetle yeniden kurar. Tahmin değil, ölçüm.
    private func butceKontrol() async {
        guard let oturum else { return }
        // contextSize ve tokenCount SystemLanguageModel üzerindedir (iOS 26.4+).
        let esik = Int(Double(model.contextSize) * esikOran)
        guard let sayi = try? await model.tokenCount(for: oturum.transcript),
              sayi > esik else { return }
        oturumKur(profil: aktifProfil, devam: await ozetle())
    }

    /// Eski geçmişi tek paragrafa özetletir. Model yoksa boş döner.
    private func ozetle() async -> String? {
        guard let oturum else { return nil }
        let istem = "Bu sohbeti tek kısa paragrafta özetle; sonraki turlarda bağlam olarak kullanılacak."
        return try? await oturum.respond(to: istem).content
    }
}
