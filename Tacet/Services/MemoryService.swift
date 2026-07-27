//
//  MemoryService.swift
//  Tacet
//
//  The WRITE path of the memory layer (memory-spec §4). Extraction NEVER runs
//  INSIDE a chat turn: adding a "is there anything to remember" task to the main
//  session breaks tool behaviour (the same regression that was measured in the
//  skill layer). That is why a SEPARATE, short-lived LanguageModelSession is used.
//
//  Triggers: chat switch / new chat (the `resetChat` moment) and the app going to
//  the background (`scenePhase != .active`).
//
//  Extraction is done ONLY from user messages (spec §2.2): if it were extracted
//  from the model's reply, the model would "learn" what it made up itself.
//
//  The model's output is NOT TRUSTED — the §4.3 filters are applied here, in code.
//

import Foundation
import FoundationModels
import SwiftData

// MARK: - Schema (spec §4.2)

@Generable
struct ExtractedNote {
    // THESE @Guide STRINGS GO TO THE MODEL as prompt text inside the structured
    // output schema. They must be English, and the kind values must be the raw
    // values of `MemoryKind` — a value outside that set is dropped by filter 2.
    @Guide(description: "identity | preference | relation | fact")
    var kind: String
    @Guide(description: "One short sentence, in the user's own wording. Do not infer.")
    var text: String
    @Guide(description: "2-4 keywords this note is relevant to.")
    var keys: [String]
}

@Generable
struct ExtractionOutcome {
    @Guide(description: "At most 2 notes. Leave EMPTY if there is no durable information.")
    var notes: [ExtractedNote]
}

// MARK: - Service

@MainActor
@Observable
final class MemoryService {
    /// The most user text handed to the model in one prompt. The extraction session
    /// shares the same 4096 token window; an overflow would produce a silent failure.
    private static let promptLimit = 1800

    /// The most notes saved in a single call — the code counterpart of the schema's
    /// "at most 2" rule (anything above the number is dropped if the model exceeds it).
    private static let perCallCap = 2

    private let models = SystemLanguageModel.default

    /// Let at most ONE session open across back-to-back triggers. For the battery and
    /// the model queue.
    private var running = false

    /// The per-chat "last processed message" caret — the same message is never
    /// processed twice.
    ///
    /// Spec §4.1 describes the caret on `Chat`; here it was kept in UserDefaults
    /// (rationale in the report): `Chat` belongs to another agent in this phase and
    /// adding a field to the model would cross the file boundary. The behaviour is the
    /// same, and the caret persists until the app is deleted.
    private static let caretKey = "memory.carets"
    /// The most chats kept in the dictionary — so the carets of deleted chats do not
    /// pile up here (the oldest records drop out by date).
    private static let caretCap = 100

    // MARK: - Trigger

    /// A fire-and-forget trigger: the view layer calls it on `scenePhase` / chat switch.
    func trigger(chat: Chat?, record: ModelContext) {
        guard let chat else { return }
        Task { await extract(chat: chat, record: record) }
    }

    /// Extracts notes from a chat's unprocessed user messages.
    ///
    /// ONE call is made per chat: the unprocessed messages are joined and given in a
    /// single prompt (a call per message is bad both for the battery and for quality).
    /// If the model is not `.available` it is SILENTLY skipped; the caret is not
    /// advanced, and the next trigger resumes where it left off.
    func extract(chat: Chat, record: ModelContext) async {
        guard !running else { return }
        guard !chat.isDeleted, chat.modelContext != nil else { return }
        guard case .available = models.availability else { return }

        // The model objects are touched BEFORE any await: after a suspension point the
        // chat may have been deleted, and touching a deleted record is fatal.
        let chatID = chat.id
        let caret = Self.caret(chatID)
        let newMessages = chat.orderedMessages.filter {
            $0.role == .user && $0.createdAt > caret
        }
        guard !newMessages.isEmpty else { return }
        let lastDate = newMessages.last!.createdAt
        let body = Self.promptBody(newMessages.map(\.content))
        guard !body.isEmpty else {
            Self.writeCaret(chatID, lastDate)
            return
        }

        // If the cap is full do not go to the model at all — the result would have been
        // dropped entirely anyway (spec §3).
        // THE CARET IS DELIBERATELY NOT ADVANCED: the cap opens up when the user deletes
        // a note, and the messages skipped at that moment must still be processable.
        // Advancing the caret would burn those messages PERMANENTLY — the cost of
        // fetching again (a single SwiftData query, no trip to the model) is trivial
        // next to that loss.
        let available = (try? record.fetch(FetchDescriptor<MemoryNote>())) ?? []
        guard !MemoryStore.isFull(available.count) else { return }
        let existingTexts = Set(available.map(\.normalizedText))

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
            // Overflow / guardrail / cancellation: the caret IS NOT ADVANCED, it is
            // retried on the next trigger.
            //
            // No error is SHOWN to the user (memory is a silent layer, spec §4), but
            // `try?` was burying three separate faults — context overflow, guardrail
            // refusal, persistent model error — in a single silence: a continuously
            // failing extraction and an extraction that never triggered looked the same
            // from outside. The diagnostic channel gives that distinction back.
            Self.recordFailure(error, chatID: chatID, messageCount: newMessages.count)
            return
        }

        // Before writing, is the context still valid — the chat may have been deleted
        // during the suspension.
        guard !chat.isDeleted, chat.modelContext != nil else { return }

        let accepted = Self.filter(outcome.notes,
                                   existingTexts: existingTexts,
                                   savedCount: available.count)
        for draft in accepted {
            let note = MemoryNote(text: draft.text,
                                  kind: draft.kind,
                                  rawKeys: draft.keys.joined(separator: ", "),
                                  sourceChatID: chatID)
            record.insert(note)
        }

        if accepted.isEmpty {
            // Producing no note is a successful processing too; the caret advances so
            // the same messages do not go to the model again on every trigger.
            Self.writeCaret(chatID, lastDate)
            return
        }

        do {
            try record.save()
            Self.writeCaret(chatID, lastDate)
            MemoryStore.reload((try? record.fetch(FetchDescriptor<MemoryNote>())) ?? [])
        } catch {
            // A note that cannot be written to disk must not stay in memory; the caret
            // must not advance either.
            record.rollback()
        }
    }

    // MARK: - Diagnostics

    /// The only diagnostic channel of extraction. In release it IS SILENT: a memory
    /// failure does not interrupt the user's work and nothing surfaces in the UI. The
    /// message CONTENT is not written — the memory layer carries the user's own
    /// sentences and they must not spill into a log; only how many messages could not
    /// be processed, and the error, are recorded.
    private static func recordFailure(_ error: Error, chatID: UUID, messageCount: Int) {
        #if DEBUG
        print("[memory] extraction dropped — chat \(chatID.uuidString.prefix(8)), "
              + "\(messageCount) messages could not be processed: \(error)")
        #endif
    }

    // MARK: - Filters (spec §4.3 — the model's output is not trusted)

    /// An accepted draft. The model's output is not written straight into the model.
    struct NoteDraft: Equatable {
        var text: String
        var kind: MemoryKind
        var keys: [String]
    }

    /// The §4.3 filters, in order: empty/short/long text, invalid kind, no keys,
    /// deduplication, cap. If any of them drops it, the note is not saved.
    ///
    /// The model IS NOT GIVEN the task of "merge two notes" — on this model that loses
    /// data.
    static func filter(_ raw: [ExtractedNote],
                       existingTexts: Set<String>,
                       savedCount: Int) -> [NoteDraft] {
        var seen = existingTexts
        var number = savedCount
        var accepted: [NoteDraft] = []

        for candidate in raw {
            guard accepted.count < perCallCap else { break }
            // 5. Drop if the cap is full.
            guard number < MemoryNote.totalCap else { break }

            // 1. Text: empty / shorter than 10 characters / longer than 160 → drop.
            let text = candidate.text.trimmingCharacters(in: .whitespacesAndNewlines)
            guard text.count >= 10, text.count <= MemoryNote.textLimit else { continue }

            // 2. The kind is not one of the four values → drop (IT IS NOT FALLEN BACK to
            //    a default: if the model is inventing the kind, the note itself is
            //    suspect too).
            guard let kind = MemoryKind(rawValue: candidate.kind.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()) else { continue }

            // 2b. Text in a question / imperative / request mood → drop. The system
            //     prompt already forbids this, but a model of this size copies prompt
            //     lines verbatim ("Bugünün tarihi ne.", "Kişilerimden 10 kişi getir").
            //     The mood is checked in code — that is exactly the rationale of §4.3.
            guard !isInvalidMood(text) else { continue }

            // 3. Keys empty → drop.
            let keys = candidate.keys
                .map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
                .filter { !$0.isEmpty && !$0.contains(",") }
                .prefix(MemoryNote.keyLimit)
                .map { $0 }
            guard !keys.isEmpty else { continue }

            // 2c. Schema echo → drop. MEASURED: at this size the model sometimes takes
            //     the NAME of the @Generable type for the note text; a "fact" reading
            //     "Ben Ayiklama Sonucu" appeared on the board.
            guard !isSchemaEcho(text) else { continue }

            // 4. Deduplication: drop if the normalised text already exists (within the
            //    same call too).
            //    `lowercased()` ALONE IS NOT ENOUGH: "İstanbul'da yaşıyorum" and
            //    "Istanbul'da yaşıyorum" produced different keys and both were saved
            //    (İ folds to a combining dotted i, while I folds to a plain i). Without
            //    mapping Turkish ı/İ by hand, deduplication does not work in that
            //    language.
            let normal = Self.dedupKey(text)
            guard !seen.contains(normal) else { continue }

            seen.insert(normal)
            number += 1
            accepted.append(NoteDraft(text: text, kind: kind, keys: keys))
        }
        return accepted
    }

    /// The deduplication key: ı/İ are mapped by hand, then diacritics are folded and
    /// punctuation is dropped. `existingTexts` must be built with this too.
    /// `nonisolated`: a pure text transform that reads no state. So that
    /// `MemoryNote.normalizedText` (a nonisolated computed property) can call it.
    nonisolated static func dedupKey(_ text: String) -> String {
        text
            .replacingOccurrences(of: "ı", with: "i")
            .replacingOccurrences(of: "İ", with: "i")
            .lowercased()
            .folding(options: .diacriticInsensitive, locale: Locale(identifier: "en_US_POSIX"))
            .filter { $0.isLetter || $0.isNumber || $0 == " " }
            .trimmingCharacters(in: .whitespaces)
    }

    /// Did the model mistake the type/field name of the generation schema for the note
    /// text?
    static func isSchemaEcho(_ text: String) -> Bool {
        let n = dedupKey(text).replacingOccurrences(of: " ", with: "")
        return schemaNames.contains(where: { n.contains($0) })
    }

    /// The names must track the CURRENT type names. When the types were renamed this
    /// list was left behind and the filter went dead — the build stayed green.
    private static let schemaNames: [String] = [
        "extractionoutcome", "extractednote", "notedraft", "memorynote",
        "generable", "guide", "stringunit",
    ]

    // MARK: - Mood filter

    /// Eliminates text that ends with a question mark / carries a question word or a
    /// question suffix / is in an imperative-request mood. A durable fact sentence
    /// carries none of these.
    ///
    /// THE LEXICONS BELOW ARE TURKISH-LANGUAGE DATA, not code identifiers: they detect
    /// the mood of the user's own Turkish sentence and must stay in Turkish, exactly
    /// like a stopword list. Translating them would silently disable the filter for
    /// Turkish input — the language this filter was written for.
    ///
    /// Matching is WORD based, not substring based: the substring "ara" occurs in
    /// "araba" and "ver" in "server" — a substring search would drop correct notes.
    /// Because Turkish is agglutinative, verb stems are looked up as a PREFIX
    /// ("getir" → "getirir", "getirebilir misin").
    ///
    /// It leans to the false-negative side: where it is undecided the note drops,
    /// the same direction as the spec's "when in doubt, extract nothing" stance.
    static func isInvalidMood(_ text: String) -> Bool {
        if text.contains("?") { return true }

        let words = text
            .lowercased(with: Locale(identifier: "tr_TR"))
            .split(whereSeparator: { !$0.isLetter && !$0.isNumber })
            .map(String.init)

        for term in words {
            // Exact match: question words and the separately written question suffix.
            if questionWords.contains(term) { return true }
            // Prefix: imperative / request stems (at least 5 letters — short stems land
            // at the start of innocent words).
            if taskStems.contains(where: { term.hasPrefix($0) }) { return true }
            // EXACT match: stems that are short but, written on their own, are nothing
            // other than an imperative. The prefix search could not catch these ("ekle"
            // is 4 letters) and commands like "Yarın 14.00'te … etkinlik ekle." were
            // being saved on the board as facts. Because a WHOLE word is checked rather
            // than a substring, innocent words like "araba"/"eklem" are unaffected.
            if shortImperatives.contains(term) { return true }
        }
        // In Turkish an imperative sentence ENDS with the verb. If the last word is a
        // short stem that could also be a noun depending on context ("ara", "yaz",
        // "aç"), standing at the end of the sentence makes it an imperative: "beni ara"
        // is a command, "öğle arası" is not.
        if let last = words.last, imperativeWhenFinal.contains(last) { return true }
        return false
    }

    /// Turkish stems that, written on their own, have no reading other than an imperative.
    private static let shortImperatives: Set<String> = [
        "ekle", "sil", "kur", "sula", "kaydet", "oluştur", "hatırlat", "planla",
    ]

    /// Short Turkish stems that can also be nouns depending on context: counted as an
    /// imperative ONLY when they are the last word of the sentence.
    private static let imperativeWhenFinal: Set<String> = [
        "ara", "yaz", "aç", "bul", "ver", "yap", "oku", "sor", "çıkar", "getir",
    ]

    /// Question words + the question suffix written SEPARATELY in Turkish
    /// ("erişebiliyor musun").
    private static let questionWords: Set<String> = [
        "ne", "neden", "niye", "niçin", "nasıl", "kaç", "hangi", "hangisi",
        "nerede", "nereye", "nereden", "neresi", "kim", "kime", "kimin", "kimler",
        "mi", "mı", "mu", "mü",
        "misin", "mısın", "musun", "müsün",
        "miyim", "mıyım", "muyum", "müyüm",
        "miyiz", "mıyız", "muyuz", "müyüz",
        "midir", "mıdır", "mudur", "müdür",
        "what", "when", "where", "who", "why", "how", "which", "whose",
    ]

    /// Imperative / request stems. Only the sufficiently long and distinctive ones:
    /// short stems like "yap", "aç", "bul", "ver" were left out DELIBERATELY because
    /// they occur at the start of innocent words.
    private static let taskStems: [String] = [
        "getir", "göster", "listele", "gönder", "hesapla", "özetle",
        "çalıştır", "kontrol", "araştır", "açıkla", "hatırlat", "oluştur",
        "istiyorum", "isterim", "istiyoruz", "lütfen",
        "please", "show", "list", "find", "tell", "give", "search", "explain",
    ]

    // MARK: - Prompt body

    /// Joins the unprocessed user messages into a single body; for the budget the LAST
    /// messages are kept (the freshest information is the most valuable).
    static func promptBody(_ texts: [String]) -> String {
        var lines: [String] = []
        var length = 0
        for text in texts.reversed() {
            let clean = text.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !clean.isEmpty else { continue }
            let line = "- \(clean)"
            guard length + line.count + 1 <= promptLimit else { break }
            lines.append(line)
            length += line.count + 1
        }
        return lines.reversed().joined(separator: "\n")
    }

    // MARK: - Caret

    private static func caret(_ chatID: UUID) -> Date {
        let dictionary = UserDefaults.standard.dictionary(forKey: caretKey) as? [String: Double] ?? [:]
        guard let ts = dictionary[chatID.uuidString] else { return .distantPast }
        return Date(timeIntervalSinceReferenceDate: ts)
    }

    private static func writeCaret(_ chatID: UUID, _ date: Date) {
        var dictionary = UserDefaults.standard.dictionary(forKey: caretKey) as? [String: Double] ?? [:]
        dictionary[chatID.uuidString] = date.timeIntervalSinceReferenceDate
        if dictionary.count > caretCap {
            // The carets of deleted chats must not pile up forever: the oldest drop out.
            let kept = dictionary.sorted { $0.value > $1.value }.prefix(caretCap)
            dictionary = Dictionary(uniqueKeysWithValues: kept.map { ($0.key, $0.value) })
        }
        UserDefaults.standard.set(dictionary, forKey: caretKey)
    }

    /// Drops the caret of a SINGLE chat — called when that chat is deleted.
    ///
    /// The single-chat delete path was touching the caret dictionary NOT AT ALL: the
    /// deleted chat's UUID stayed in `UserDefaults` and the dictionary grew in one
    /// direction only. The cap (`caretCap`) did not turn this into a crash, but
    /// identities that no longer live were pushing the carets of living chats down past
    /// the cap.
    static func deleteCaret(chatID: UUID) {
        var dictionary = UserDefaults.standard.dictionary(forKey: caretKey) as? [String: Double] ?? [:]
        guard dictionary.removeValue(forKey: chatID.uuidString) != nil else { return }
        UserDefaults.standard.set(dictionary, forKey: caretKey)
    }

    /// Resets the carets when ALL history is cleared. The single-chat delete path DOES
    /// NOT CALL this — its door is `deleteCaret(chatID:)`.
    /// It DOES NOT TOUCH the notes themselves — deleting memory is the board's job (spec §7).
    static func resetCarets() {
        UserDefaults.standard.removeObject(forKey: caretKey)
    }
}
