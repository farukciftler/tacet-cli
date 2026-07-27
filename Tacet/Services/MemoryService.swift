//
//  HafizaServisi.swift
//  Tacet
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
struct ExtractedNote {
    @Guide(description: "kimlik | tercih | iliski | olgu")
    var kind: String
    @Guide(description: "Tek kısa cümle, kullanıcının kendi ifadesinden. Çıkarsama yapma.")
    var text: String
    @Guide(description: "Bu notun ilgili olduğu 2-4 anahtar kelime.")
    var keys: [String]
}

@Generable
struct ExtractionOutcome {
    @Guide(description: "En fazla 2 not. Kalıcı bilgi yoksa BOŞ bırak.")
    var notlar: [ExtractedNote]
}

// MARK: - Servis

@MainActor
@Observable
final class MemoryService {
    /// Bir istemde modele verilecek en fazla kullanıcı metni. Ayıklama oturumu
    /// da 4096 token penceresini paylaşır; taşma sessiz başarısızlık üretirdi.
    private static let istemSiniri = 1800

    /// Tek çağrıda kaydedilecek en fazla not — şemadaki "en fazla 2" kuralının
    /// koddaki karşılığı (model sayıyı aşarsa fazlası düşer).
    private static let cagriBasiTavan = 2

    private let models = SystemLanguageModel.default

    /// Art arda tetiklerde en fazla BİR oturum açılsın. Pil ve model kuyruğu için.
    private var running = false

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
    func trigger(chat: Chat?, record: ModelContext) {
        guard let chat else { return }
        Task { await extract(chat: chat, record: record) }
    }

    /// Bir sohbetin işlenmemiş kullanıcı mesajlarından not ayıklar.
    ///
    /// Sohbet başına TEK çağrı yapılır: işlenmemiş mesajlar birleştirilip tek
    /// istemde verilir (mesaj başına çağrı hem pil hem kalite açısından kötüdür).
    /// Model `.available` değilse SESSİZCE atlanır; imleç ilerletilmez, bir
    /// sonraki tetikte kaldığı yerden denenir.
    func extract(chat: Chat, record: ModelContext) async {
        guard !running else { return }
        guard !chat.isDeleted, chat.modelContext != nil else { return }
        guard case .available = models.availability else { return }

        // Model nesnelerine await'ten ÖNCE dokunulur: askıya alma noktasından
        // sonra sohbet silinmiş olabilir ve silinmiş kayda erişmek ölümcüldür.
        let chatID = chat.id
        let caret = Self.caret(chatID)
        let yeniMesajlar = chat.orderedMessages.filter {
            $0.role == .user && $0.createdAt > caret
        }
        guard !yeniMesajlar.isEmpty else { return }
        let sonTarih = yeniMesajlar.last!.createdAt
        let body = Self.istemGovdesi(yeniMesajlar.map(\.content))
        guard !body.isEmpty else {
            Self.imlecYaz(chatID, sonTarih)
            return
        }

        // Tavan doluysa modele hiç gitme — sonuç zaten tamamen düşerdi (spec §3).
        // İMLEÇ BİLEREK İLERLETİLMEZ: tavan kullanıcı not silince açılır ve o an
        // atlanmış mesajlar hâlâ işlenebilir olmalı. İmleci ilerletmek bu
        // mesajları KALICI olarak yakardı — tekrar fetch etmenin maliyeti (tek
        // SwiftData sorgusu, modele çıkış yok) o kaybın yanında önemsiz.
        let available = (try? record.fetch(FetchDescriptor<MemoryNote>())) ?? []
        guard !MemoryStore.isFull(available.count) else { return }
        let varolanMetinler = Set(available.map(\.normalizedText))

        running = true
        defer { running = false }

        let session = LanguageModelSession {
            """
            You extract durable facts from a user's own messages for a personal \
            memory store. Extract only durable facts the user states about \
            themselves: identity, stable preferences, relationships, or lasting \
            circumstances. Do not infer or generalise — use only what is \
            explicitly stated. When in doubt, extract nothing.

            Most messages contain NOTHING to extract. A message is not a fact \
            just because the user wrote it. Never copy a message that asks a \
            question, gives you an instruction, or requests something — those \
            are never facts, no matter what they are about.

            "What is today's date?" -> nothing
            "Can you reach my server?" -> nothing
            "Get me 10 of my contacts" -> nothing
            "I want today's weather" -> nothing
            "I'm searching the web" -> nothing
            "I live in Istanbul" -> fact: the user lives in Istanbul
            """
        }

        let prompt = """
        User messages:
        \(body)

        Extract at most 2 durable facts. Most message sets yield none — \
        return an empty list unless the user plainly stated something \
        lasting about themselves.
        """

        let outcome: ExtractionOutcome
        do {
            outcome = try await session.respond(to: prompt, generating: ExtractionOutcome.self).content
        } catch {
            // Taşma / guardrail / iptal: imleç İLERLETİLMEZ, sonraki tetikte tekrar denenir.
            //
            // Kullanıcıya hata GÖSTERİLMEZ (hafıza sessiz bir katmandır, spec §4)
            // ama `try?` üç ayrı arızayı — bağlam taşması, guardrail reddi, kalıcı
            // model hatası — tek bir sessizliğe gömüyordu: sürekli başarısız bir
            // ayıklama ile hiç tetiklenmemiş bir ayıklama dışarıdan aynı görünüyor.
            // Teşhis kanalı bu ayrımı geri veriyor.
            Self.arizaKaydet(error, chatID: chatID, mesajSayisi: yeniMesajlar.count)
            return
        }

        // Yazmadan önce bağlam hâlâ geçerli mi — askı sırasında sohbet silinmiş olabilir.
        guard !chat.isDeleted, chat.modelContext != nil else { return }

        let accepted = Self.filter(outcome.notlar,
                             varolanMetinler: varolanMetinler,
                             kayitliSayi: available.count)
        for taslak in accepted {
            let note = MemoryNote(text: taslak.text,
                                 kind: taslak.kind,
                                 rawKeys: taslak.keys.joined(separator: ", "),
                                 sourceChatID: chatID)
            record.insert(note)
        }

        if accepted.isEmpty {
            // Not çıkmaması da başarılı bir işlemedir; imleç ilerler ki aynı
            // mesajlar her tetikte yeniden modele gitmesin.
            Self.imlecYaz(chatID, sonTarih)
            return
        }

        do {
            try record.save()
            Self.imlecYaz(chatID, sonTarih)
            MemoryStore.reload((try? record.fetch(FetchDescriptor<MemoryNote>())) ?? [])
        } catch {
            // Diske yazılamayan not bellekte durmasın; imleç de ilerlemesin.
            record.rollback()
        }
    }

    // MARK: - Teşhis

    /// Ayıklamanın tek teşhis kanalı. Yayında SESSİZDİR: hafıza başarısızlığı
    /// kullanıcının işini bölmez ve arayüze hiçbir şey çıkmaz. Mesaj İÇERİĞİ
    /// yazılmaz — hafıza katmanı kullanıcının kendi cümlelerini taşır, log'a
    /// dökülmemeli; yalnız kaç mesajın işlenemediği ve hata kaydedilir.
    private static func arizaKaydet(_ error: Error, chatID: UUID, mesajSayisi: Int) {
        #if DEBUG
        print("[hafiza] ayıklama düştü — sohbet \(chatID.uuidString.prefix(8)), "
              + "\(mesajSayisi) mesaj işlenemedi: \(error)")
        #endif
    }

    // MARK: - Filtreler (spec §4.3 — model çıktısına güvenilmez)

    /// Kabul edilen taslak. Model çıktısı doğrudan modele yazılmaz.
    struct NoteDraft: Equatable {
        var text: String
        var kind: MemoryKind
        var keys: [String]
    }

    /// §4.3 filtreleri sırayla: boş/kısa/uzun metin, geçersiz tür, anahtarsız,
    /// tekilleştirme, tavan. Herhangi biri düşürürse not kaydedilmez.
    ///
    /// Modele "iki notu birleştir" görevi VERİLMEZ — bu modelde veri kaybettirir.
    static func filter(_ raw: [ExtractedNote],
                    varolanMetinler: Set<String>,
                    kayitliSayi: Int) -> [NoteDraft] {
        var gorulen = varolanMetinler
        var number = kayitliSayi
        var accepted: [NoteDraft] = []

        for candidate in raw {
            guard accepted.count < cagriBasiTavan else { break }
            // 5. Tavan dolduysa düş.
            guard number < MemoryNote.totalCap else { break }

            // 1. Metin: boş / 10 karakterden kısa / 160'tan uzun → düş.
            let text = candidate.text.trimmingCharacters(in: .whitespacesAndNewlines)
            guard text.count >= 10, text.count <= MemoryNote.textLimit else { continue }

            // 2. Tür dört değerden biri değil → düş (varsayılana DÜŞÜRÜLMEZ:
            //    model türü uyduruyorsa notun kendisi de şüphelidir).
            guard let kind = MemoryKind(rawValue: candidate.kind.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()) else { continue }

            // 2b. Soru / emir / talep kipindeki metin → düş. Sistem istemi bunu
            //     zaten yasaklar ama bu boyuttaki model istem satırlarını olduğu
            //     gibi kopyalıyor ("Bugünün tarihi ne.", "Kişilerimden 10 kişi
            //     getir"). Kip koddan bakılır — §4.3'ün gerekçesi tam olarak bu.
            guard !kipiGecersiz(text) else { continue }

            // 3. Anahtarlar boş → düş.
            let keys = candidate.keys
                .map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
                .filter { !$0.isEmpty && !$0.contains(",") }
                .prefix(MemoryNote.keyLimit)
                .map { $0 }
            guard !keys.isEmpty else { continue }

            // 2c. Şema yankısı → düş. ÖLÇÜLEN: model bu boyutta bazen
            //     @Generable tipinin ADINI not metni sanıp yazıyor; panelde
            //     "Ben Ayiklama Sonucu" diye bir "olgu" belirdi.
            guard !semaYankisi(text) else { continue }

            // 4. Tekilleştirme: normalize metin zaten varsa düş (aynı çağrı içinde de).
            //    `lowercased()` TEK BAŞINA YETMEZ: "İstanbul'da yaşıyorum" ve
            //    "Istanbul'da yaşıyorum" farklı anahtar üretip ikisi de kaydediliyordu
            //    (İ birleşik noktalı i'ye düşer, I ise düz i'ye). Türkçe ı/İ elle
            //    eşlenmeden tekilleştirme bu dilde çalışmaz.
            let normal = Self.dedupKey(text)
            guard !gorulen.contains(normal) else { continue }

            gorulen.insert(normal)
            number += 1
            accepted.append(NoteDraft(text: text, kind: kind, keys: keys))
        }
        return accepted
    }

    /// Tekilleştirme anahtarı: ı/İ elle eşlenir, sonra aksanlar katlanır ve
    /// noktalama atılır. `varolanMetinler` de bununla kurulmalıdır.
    /// `nonisolated`: saf metin dönüşümü, durum okumaz. `HafizaNotu.normalMetin`
    /// (nonisolated bir computed property) buradan çağırabilsin diye.
    nonisolated static func dedupKey(_ text: String) -> String {
        text
            .replacingOccurrences(of: "ı", with: "i")
            .replacingOccurrences(of: "İ", with: "i")
            .lowercased()
            .folding(options: .diacriticInsensitive, locale: Locale(identifier: "en_US_POSIX"))
            .filter { $0.isLetter || $0.isNumber || $0 == " " }
            .trimmingCharacters(in: .whitespaces)
    }

    /// Model, üretim şemasının tip/alan adını not metni sanmış mı?
    static func semaYankisi(_ text: String) -> Bool {
        let n = dedupKey(text).replacingOccurrences(of: " ", with: "")
        return semaAdlari.contains(where: { n.contains($0) })
    }

    private static let semaAdlari: [String] = [
        "ayiklamasonucu", "ayiklanannot", "nottaslagi", "hafizanotu",
        "generable", "guide", "stringunit",
    ]

    // MARK: - Kip filtresi

    /// Soru işareti ile biten / soru sözcüğü ya da soru eki taşıyan / emir-talep
    /// kipindeki metinleri eler. Kalıcı olgu cümlesi bunların hiçbirini taşımaz.
    ///
    /// Eşleşme SÖZCÜK bazlıdır, alt dizge bazlı değil: "ara" alt dizgesi
    /// "araba"da, "ver" ise "server"da geçer — alt dizge araması doğru notları
    /// düşürürdü. Türkçe eklemeli olduğu için fiil gövdeleri ÖN EK olarak
    /// aranır ("getir" → "getirir", "getirebilir misin").
    ///
    /// Yanlış negatif tarafına eğimlidir: kararsız kalınan yerde not düşer,
    /// spec'in "when in doubt, extract nothing" duruşuyla aynı yön.
    static func kipiGecersiz(_ text: String) -> Bool {
        if text.contains("?") { return true }

        let sozcukler = text
            .lowercased(with: Locale(identifier: "tr_TR"))
            .split(whereSeparator: { !$0.isLetter && !$0.isNumber })
            .map(String.init)

        for term in sozcukler {
            // Tam eşleşme: soru sözcükleri ve ayrı yazılan soru eki.
            if soruSozcukleri.contains(term) { return true }
            // Ön ek: emir / talep gövdeleri (en az 5 harf — kısa gövdeler
            // masum sözcüklerin başına denk gelir).
            if gorevGovdeleri.contains(where: { term.hasPrefix($0) }) { return true }
            // TAM eşleşme: kısa ama tek başına yazıldığında emirden başka bir
            // şey olmayan gövdeler. Ön ek araması bunları alamıyordu ("ekle"
            // 4 harf) ve panele "Yarın 14.00'te … etkinlik ekle." gibi komutlar
            // olgu diye kaydediliyordu. Alt dizge değil TAM sözcük bakıldığı
            // için "araba"/"eklem" gibi masum sözcükler etkilenmez.
            if kisaEmirler.contains(term) { return true }
        }
        // Türkçede emir cümlesi fiille BİTER. Son sözcük bağlama göre isim de
        // olabilen kısa bir gövdeyse ("ara", "yaz", "aç"), cümle sonunda olması
        // onu emir yapar: "beni ara" komut, "öğle arası" değil.
        if let last = sozcukler.last, sonEkiEmirOlanlar.contains(last) { return true }
        return false
    }

    /// Tek başına yazıldığında Türkçede emirden başka okuması olmayan gövdeler.
    private static let kisaEmirler: Set<String> = [
        "ekle", "sil", "kur", "sula", "kaydet", "oluştur", "hatırlat", "planla",
    ]

    /// Bağlama göre isim de olabilen kısa gövdeler: YALNIZCA cümlenin son
    /// sözcüğüyken emir sayılır.
    private static let sonEkiEmirOlanlar: Set<String> = [
        "ara", "yaz", "aç", "bul", "ver", "yap", "oku", "sor", "çıkar", "getir",
    ]

    /// Soru sözcükleri + Türkçede AYRI yazılan soru eki ("erişebiliyor musun").
    private static let soruSozcukleri: Set<String> = [
        "ne", "neden", "niye", "niçin", "nasıl", "kaç", "hangi", "hangisi",
        "nerede", "nereye", "nereden", "neresi", "kim", "kime", "kimin", "kimler",
        "mi", "mı", "mu", "mü",
        "misin", "mısın", "musun", "müsün",
        "miyim", "mıyım", "muyum", "müyüm",
        "miyiz", "mıyız", "muyuz", "müyüz",
        "midir", "mıdır", "mudur", "müdür",
        "what", "when", "where", "who", "why", "how", "which", "whose",
    ]

    /// Emir / talep gövdeleri. Yalnızca yeterince uzun ve ayırt edici olanlar:
    /// "yap", "aç", "bul", "ver" gibi kısa gövdeler masum sözcüklerin başında
    /// geçtiği için BİLEREK dışarıda bırakıldı.
    private static let gorevGovdeleri: [String] = [
        "getir", "göster", "listele", "gönder", "hesapla", "özetle",
        "çalıştır", "kontrol", "araştır", "açıkla", "hatırlat", "oluştur",
        "istiyorum", "isterim", "istiyoruz", "lütfen",
        "please", "show", "list", "find", "tell", "give", "search", "explain",
    ]

    // MARK: - İstem gövdesi

    /// İşlenmemiş kullanıcı mesajlarını tek gövdede birleştirir; bütçe için
    /// SON mesajlar korunur (en taze bilgi en değerlisidir).
    static func istemGovdesi(_ metinler: [String]) -> String {
        var lines: [String] = []
        var length = 0
        for text in metinler.reversed() {
            let temiz = text.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !temiz.isEmpty else { continue }
            let line = "- \(temiz)"
            guard length + line.count + 1 <= istemSiniri else { break }
            lines.append(line)
            length += line.count + 1
        }
        return lines.reversed().joined(separator: "\n")
    }

    // MARK: - İmleç

    private static func caret(_ chatID: UUID) -> Date {
        let dictionary = UserDefaults.standard.dictionary(forKey: imlecAnahtari) as? [String: Double] ?? [:]
        guard let ts = dictionary[chatID.uuidString] else { return .distantPast }
        return Date(timeIntervalSinceReferenceDate: ts)
    }

    private static func imlecYaz(_ chatID: UUID, _ date: Date) {
        var dictionary = UserDefaults.standard.dictionary(forKey: imlecAnahtari) as? [String: Double] ?? [:]
        dictionary[chatID.uuidString] = date.timeIntervalSinceReferenceDate
        if dictionary.count > imlecTavani {
            // Silinen sohbetlerin imleci sonsuza kadar birikmesin: en eskiler düşer.
            let korunan = dictionary.sorted { $0.value > $1.value }.prefix(imlecTavani)
            dictionary = Dictionary(uniqueKeysWithValues: korunan.map { ($0.key, $0.value) })
        }
        UserDefaults.standard.set(dictionary, forKey: imlecAnahtari)
    }

    /// TEK sohbetin imlecini düşürür — sohbet silindiğinde çağrılır.
    ///
    /// Tek sohbet silme yolu imleç sözlüğüne HİÇ dokunmuyordu: silinen sohbetin
    /// UUID'si `UserDefaults`ta kalıyor, sözlük tek yönlü büyüyordu. Tavan
    /// (`imlecTavani`) bunu bir çöküşe çevirmiyordu ama artık yaşamayan
    /// kimlikler, yaşayan sohbetlerin imleçlerini tavandan aşağı itiyordu.
    static func deleteCaret(chatID: UUID) {
        var dictionary = UserDefaults.standard.dictionary(forKey: imlecAnahtari) as? [String: Double] ?? [:]
        guard dictionary.removeValue(forKey: chatID.uuidString) != nil else { return }
        UserDefaults.standard.set(dictionary, forKey: imlecAnahtari)
    }

    /// TÜM geçmiş temizlendiğinde imleçleri sıfırlar. Tek sohbet silme yolu
    /// bunu ÇAĞIRMAZ — onun kapısı `imlecSil(chatID:)`.
    /// Notların kendisine DOKUNMAZ — hafızayı silmek panonun işidir (spec §7).
    static func resetCarets() {
        UserDefaults.standard.removeObject(forKey: imlecAnahtari)
    }
}
