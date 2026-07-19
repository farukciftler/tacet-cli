//
//  HafizaServisi.swift
//  ketum
//
//  Hafıza katmanının YAZMA yolu (hafiza-spec §4). Ayıklama sohbet turunun
//  İÇİNDE ASLA çalışmaz: ana oturuma "hatırlanacak bir şey var mı" görevi
//  eklemek araç davranışını bozar (beceri katmanında ölçülen regresyonun
//  aynısı). Bu yüzden AYRI ve kısa ömürlü bir LanguageModelSession kullanılır.
//
//  Tetikler: sohbet değişimi / yeni sohbet (`sohbetiSifirla` anı) ve
//  uygulamanın arka plana geçmesi (`scenePhase != .active`).
//
//  Ayıklama YALNIZCA kullanıcı mesajlarından yapılır (spec §2.2): model
//  yanıtından ayıklanırsa model kendi uydurduğunu "öğrenir".
//
//  Modelin çıktısına GÜVENİLMEZ — §4.3 filtreleri burada, kodda uygulanır.
//

import Foundation
import FoundationModels
import SwiftData

// MARK: - Şema (spec §4.2)

@Generable
struct AyiklananNot {
    @Guide(description: "kimlik | tercih | iliski | olgu")
    var tur: String
    @Guide(description: "Tek kısa cümle, kullanıcının kendi ifadesinden. Çıkarsama yapma.")
    var metin: String
    @Guide(description: "Bu notun ilgili olduğu 2-4 anahtar kelime.")
    var anahtarlar: [String]
}

@Generable
struct AyiklamaSonucu {
    @Guide(description: "En fazla 2 not. Kalıcı bilgi yoksa BOŞ bırak.")
    var notlar: [AyiklananNot]
}

// MARK: - Servis

@MainActor
@Observable
final class HafizaServisi {
    /// Bir istemde modele verilecek en fazla kullanıcı metni. Ayıklama oturumu
    /// da 4096 token penceresini paylaşır; taşma sessiz başarısızlık üretirdi.
    private static let istemSiniri = 1800

    /// Tek çağrıda kaydedilecek en fazla not — şemadaki "en fazla 2" kuralının
    /// koddaki karşılığı (model sayıyı aşarsa fazlası düşer).
    private static let cagriBasiTavan = 2

    private let model = SystemLanguageModel.default

    /// Art arda tetiklerde en fazla BİR oturum açılsın (NobetServisi/ContentView'daki
    /// `tazeleniyor` koruma deseni). Pil ve model kuyruğu için.
    private var calisiyor = false

    /// Sohbet başına "son işlenen mesaj" imleci — aynı mesaj iki kez işlenmez.
    ///
    /// Spec §4.1 imleci `Sohbet` üzerinde tarif eder; burada UserDefaults'ta
    /// tutuldu (gerekçe raporda): `Sohbet` bu fazda başka bir ajana ait ve
    /// modele alan eklemek dosya sınırını aşardı. Davranış aynıdır, imleç
    /// uygulama silinene kadar kalıcıdır.
    private static let imlecAnahtari = "hafiza.imlecler"
    /// Sözlükte tutulacak en fazla sohbet — silinen sohbetlerin imleci burada
    /// birikmesin (eski kayıtlar tarihe göre düşer).
    private static let imlecTavani = 100

    // MARK: - Tetik

    /// Ateşle-ve-unut tetik: görünüm katmanı `scenePhase` / sohbet değişiminde çağırır.
    func tetikle(sohbet: Sohbet?, kayit: ModelContext) {
        guard let sohbet else { return }
        Task { await ayikla(sohbet: sohbet, kayit: kayit) }
    }

    /// Bir sohbetin işlenmemiş kullanıcı mesajlarından not ayıklar.
    ///
    /// Sohbet başına TEK çağrı yapılır: işlenmemiş mesajlar birleştirilip tek
    /// istemde verilir (mesaj başına çağrı hem pil hem kalite açısından kötüdür).
    /// Model `.available` değilse SESSİZCE atlanır; imleç ilerletilmez, bir
    /// sonraki tetikte kaldığı yerden denenir.
    func ayikla(sohbet: Sohbet, kayit: ModelContext) async {
        guard !calisiyor else { return }
        guard !sohbet.isDeleted, sohbet.modelContext != nil else { return }
        guard case .available = model.availability else { return }

        // Model nesnelerine await'ten ÖNCE dokunulur: askıya alma noktasından
        // sonra sohbet silinmiş olabilir ve silinmiş kayda erişmek ölümcüldür.
        let sohbetID = sohbet.id
        let imlec = Self.imlec(sohbetID)
        let yeniMesajlar = sohbet.siraliMesajlar.filter {
            $0.rol == .sen && $0.olusturulma > imlec
        }
        guard !yeniMesajlar.isEmpty else { return }
        let sonTarih = yeniMesajlar.last!.olusturulma
        let govde = Self.istemGovdesi(yeniMesajlar.map(\.icerik))
        guard !govde.isEmpty else {
            Self.imlecYaz(sohbetID, sonTarih)
            return
        }

        // Tavan doluysa modele hiç gitme — sonuç zaten tamamen düşerdi (spec §3).
        let mevcut = (try? kayit.fetch(FetchDescriptor<HafizaNotu>())) ?? []
        guard !HafizaDeposu.doluMu(mevcut.count) else { return }
        let varolanMetinler = Set(mevcut.map(\.normalMetin))

        calisiyor = true
        defer { calisiyor = false }

        let oturum = LanguageModelSession {
            """
            You extract durable facts from a user's own messages for a personal \
            memory store. Extract only durable facts the user states about \
            themselves: identity, stable preferences, relationships, or lasting \
            circumstances. Ignore questions, tasks, transient details, moods, \
            and anything the user asks you to do. Do not infer or generalise — \
            use only what is explicitly stated. When in doubt, extract nothing.
            """
        }

        let istem = """
        User messages:
        \(govde)

        Extract at most 2 durable facts. If there are none, return an empty list.
        """

        guard let sonuc = try? await oturum.respond(to: istem, generating: AyiklamaSonucu.self).content else {
            // Taşma / guardrail / iptal: imleç İLERLETİLMEZ, sonraki tetikte tekrar denenir.
            return
        }

        // Yazmadan önce bağlam hâlâ geçerli mi — askı sırasında sohbet silinmiş olabilir.
        guard !sohbet.isDeleted, sohbet.modelContext != nil else { return }

        let kabul = Self.suz(sonuc.notlar,
                             varolanMetinler: varolanMetinler,
                             kayitliSayi: mevcut.count)
        for taslak in kabul {
            let not = HafizaNotu(metin: taslak.metin,
                                 tur: taslak.tur,
                                 anahtarlarHam: taslak.anahtarlar.joined(separator: ", "),
                                 kaynakSohbetID: sohbetID)
            kayit.insert(not)
        }

        if kabul.isEmpty {
            // Not çıkmaması da başarılı bir işlemedir; imleç ilerler ki aynı
            // mesajlar her tetikte yeniden modele gitmesin.
            Self.imlecYaz(sohbetID, sonTarih)
            return
        }

        do {
            try kayit.save()
            Self.imlecYaz(sohbetID, sonTarih)
            HafizaDeposu.yenile((try? kayit.fetch(FetchDescriptor<HafizaNotu>())) ?? [])
        } catch {
            // Diske yazılamayan not bellekte durmasın; imleç de ilerlemesin.
            kayit.rollback()
        }
    }

    // MARK: - Filtreler (spec §4.3 — model çıktısına güvenilmez)

    /// Kabul edilen taslak. Model çıktısı doğrudan modele yazılmaz.
    struct NotTaslagi: Equatable {
        var metin: String
        var tur: HafizaTuru
        var anahtarlar: [String]
    }

    /// §4.3 filtreleri sırayla: boş/kısa/uzun metin, geçersiz tür, anahtarsız,
    /// tekilleştirme, tavan. Herhangi biri düşürürse not kaydedilmez.
    ///
    /// Modele "iki notu birleştir" görevi VERİLMEZ — bu modelde veri kaybettirir.
    static func suz(_ ham: [AyiklananNot],
                    varolanMetinler: Set<String>,
                    kayitliSayi: Int) -> [NotTaslagi] {
        var gorulen = varolanMetinler
        var sayi = kayitliSayi
        var kabul: [NotTaslagi] = []

        for aday in ham {
            guard kabul.count < cagriBasiTavan else { break }
            // 5. Tavan dolduysa düş.
            guard sayi < HafizaNotu.toplamTavan else { break }

            // 1. Metin: boş / 10 karakterden kısa / 160'tan uzun → düş.
            let metin = aday.metin.trimmingCharacters(in: .whitespacesAndNewlines)
            guard metin.count >= 10, metin.count <= HafizaNotu.metinSiniri else { continue }

            // 2. Tür dört değerden biri değil → düş (varsayılana DÜŞÜRÜLMEZ:
            //    model türü uyduruyorsa notun kendisi de şüphelidir).
            guard let tur = HafizaTuru(rawValue: aday.tur.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()) else { continue }

            // 3. Anahtarlar boş → düş.
            let anahtarlar = aday.anahtarlar
                .map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
                .filter { !$0.isEmpty && !$0.contains(",") }
                .prefix(HafizaNotu.anahtarSiniri)
                .map { $0 }
            guard !anahtarlar.isEmpty else { continue }

            // 4. Tekilleştirme: normalize metin zaten varsa düş (aynı çağrı içinde de).
            let normal = metin.lowercased()
            guard !gorulen.contains(normal) else { continue }

            gorulen.insert(normal)
            sayi += 1
            kabul.append(NotTaslagi(metin: metin, tur: tur, anahtarlar: anahtarlar))
        }
        return kabul
    }

    // MARK: - İstem gövdesi

    /// İşlenmemiş kullanıcı mesajlarını tek gövdede birleştirir; bütçe için
    /// SON mesajlar korunur (en taze bilgi en değerlisidir).
    static func istemGovdesi(_ metinler: [String]) -> String {
        var satirlar: [String] = []
        var uzunluk = 0
        for metin in metinler.reversed() {
            let temiz = metin.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !temiz.isEmpty else { continue }
            let satir = "- \(temiz)"
            guard uzunluk + satir.count + 1 <= istemSiniri else { break }
            satirlar.append(satir)
            uzunluk += satir.count + 1
        }
        return satirlar.reversed().joined(separator: "\n")
    }

    // MARK: - İmleç

    private static func imlec(_ sohbetID: UUID) -> Date {
        let sozluk = UserDefaults.standard.dictionary(forKey: imlecAnahtari) as? [String: Double] ?? [:]
        guard let ts = sozluk[sohbetID.uuidString] else { return .distantPast }
        return Date(timeIntervalSinceReferenceDate: ts)
    }

    private static func imlecYaz(_ sohbetID: UUID, _ tarih: Date) {
        var sozluk = UserDefaults.standard.dictionary(forKey: imlecAnahtari) as? [String: Double] ?? [:]
        sozluk[sohbetID.uuidString] = tarih.timeIntervalSinceReferenceDate
        if sozluk.count > imlecTavani {
            // Silinen sohbetlerin imleci sonsuza kadar birikmesin: en eskiler düşer.
            let korunan = sozluk.sorted { $0.value > $1.value }.prefix(imlecTavani)
            sozluk = Dictionary(uniqueKeysWithValues: korunan.map { ($0.key, $0.value) })
        }
        UserDefaults.standard.set(sozluk, forKey: imlecAnahtari)
    }

    /// Sohbet silindiğinde / geçmiş temizlendiğinde imleçleri sıfırlar.
    /// Notların kendisine DOKUNMAZ — hafızayı silmek panonun işidir (spec §7).
    static func imlecleriSifirla() {
        UserDefaults.standard.removeObject(forKey: imlecAnahtari)
    }
}
