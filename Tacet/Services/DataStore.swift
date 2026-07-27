//
//  DataStore.swift
//  Tacet
//
//  The channel for moving large data without exceeding the context window (spec
//  §7.2, §7.3.2 flow 2). The source tool (e.g. Calendar) puts the structured data
//  it gathered here and returns to the model ONLY a short summary + a reference
//  ID. The production tool (create_document) pulls the data out of this store —
//  bulk data never enters the context window. The file size is limited by the
//  device, not by the window.
//

import Foundation
import Observation

@MainActor
@Observable
final class DataStore {
    /// Reference ID → table. Large tables live here, the model never sees them.
    private var store: [String: Table] = [:]
    /// Large content that is NOT a table (the plain body of a document that was read —
    /// P2-6).
    ///
    /// A separate dictionary is kept because squeezing prose into a single-column
    /// `Table` would turn EVERY document produced from that ref (docx/pdf/md) into a
    /// table: a new class of corruption that silently converts text into something else.
    /// The type split makes that impossible from the start.
    private var textStore: [String: String] = [:]
    private var counter = 0

    // MARK: - Caps (LRU)

    /// THE STORE WAS GROWING IN ONE DIRECTION ACROSS A CHAT. The full body of every
    /// document read (hundreds of thousands of characters per document) and every tool
    /// table stayed here, and only `clear` emptied it; in a long chat that is an
    /// accumulation that silently eats the device's memory.
    ///
    /// The cap has TWO AXES: hundreds of small records and two enormous bodies cause the
    /// same trouble; a single axis would miss the second.
    static let recordCap = 24
    static let characterCap = 800_000
    /// The most dropped ref names to remember. A name list (not the content) is kept;
    /// this is what distinguishes "never existed" from "existed and was dropped".
    static let droppedCap = 60

    /// Use order — OLDEST first, FRESHEST last. Dropping eats from the front.
    /// The view layer does not observe the store ledger; it must not produce observation
    /// noise.
    @ObservationIgnored private var order: [String] = []
    /// The character cost per ref — stored so the cap computation does not walk the whole
    /// store on every put.
    @ObservationIgnored private var sizes: [String: Int] = [:]
    @ObservationIgnored private var totalCharacters = 0
    /// The ref names DROPPED because of the cap (oldest first).
    @ObservationIgnored private var dropped: [String] = []

    /// Stores the table and returns a short reference ID. The model carries that ID.
    func put(_ table: Table, tag: String) -> String {
        counter += 1
        let ref = "\(tag)-\(counter)"
        store[ref] = table
        writeLedger(ref, size: Self.measure(table))
        return ref
    }

    /// Stores a plain text body (the same ref space — the IDs never collide).
    func putText(_ text: String, tag: String) -> String {
        counter += 1
        let ref = "\(tag)-\(counter)"
        textStore[ref] = text
        writeLedger(ref, size: text.count)
        return ref
    }

    /// Pulls the full table by reference (the production tool calls it).
    func take(_ ref: String) -> Table? {
        let key = Self.normalize(ref)
        guard let table = store[key] else { return nil }
        refresh(key)
        return table
    }

    /// Pulls the full TEXT body by reference.
    func takeText(_ ref: String) -> String? {
        let key = Self.normalize(ref)
        guard let text = textStore[key] else { return nil }
        refresh(key)
        return text
    }

    // MARK: - Ledger (LRU bookkeeping)

    /// Takes the new record to the END of the order and drops the oldest ones if the cap
    /// is exceeded.
    private func writeLedger(_ ref: String, size: Int) {
        order.removeAll { $0 == ref }
        order.append(ref)
        totalCharacters += size - (sizes[ref] ?? 0)
        sizes[ref] = size
        dropped.removeAll { $0 == ref }
        prune()
    }

    /// A ref that is read counts as the freshest: in a chat producing repeatedly from one
    /// document, pulling that document out from under it merely for being old would make
    /// no sense.
    private func refresh(_ ref: String) {
        guard order.last != ref, order.contains(ref) else { return }
        order.removeAll { $0 == ref }
        order.append(ref)
    }

    /// Drops the OLDEST ref as the cap is exceeded.
    ///
    /// THE NEWEST RECORD NEVER DROPS (the `order.count > 1` condition): if a body that
    /// exceeds the cap on its own were deleted the moment it was put, the source tool
    /// would tell the model "stored it, here is the ref" and the production tool would not
    /// find that ref — exactly the silent data loss the 4096 bypass channel exists to
    /// prevent.
    private func prune() {
        while order.count > 1,
              order.count > Self.recordCap || totalCharacters > Self.characterCap {
            let oldest = order.removeFirst()
            store[oldest] = nil
            textStore[oldest] = nil
            totalCharacters -= sizes.removeValue(forKey: oldest) ?? 0
            dropped.append(oldest)
        }
        if dropped.count > Self.droppedCap {
            dropped.removeFirst(dropped.count - Self.droppedCap)
        }
    }

    /// The character cost of the table. It looks at the REAL load, not the row count:
    /// 500 rows with three columns and 500 rows with thirty columns are not the same thing.
    private static func measure(_ table: Table) -> Int {
        table.headers.reduce(0) { $0 + $1.count }
            + table.rows.reduce(0) { total, line in
                total + line.cells.reduce(0) { $0 + $1.count }
            }
    }

    /// The FACT sentence returned to the model for a ref that cannot be resolved (no
    /// imperative directive — the same stance as P2-4).
    ///
    /// The two cases are SEPARATE sentences and the distinction matters: "never existed"
    /// says the model invented the ref, while "existed but was dropped to make room" says
    /// re-gathering the data is worthwhile. Using the same sentence would make a dropped
    /// ref look like a model hallucination.
    func refState(_ ref: String) -> String {
        let key = Self.normalize(ref)
        let inHand = refKeys
        let list = inHand.isEmpty ? "none" : inHand.joined(separator: ",")
        if dropped.contains(key) {
            return "expired_data_ref: \"\(key)\" existed but was dropped to stay "
                + "within the device memory budget and is no longer available "
                + "(available: \(list))"
        }
        return "unknown_data_ref: \"\(key)\" (available: \(list))"
    }

    /// Does the ref resolve on any channel? (The P0-2 error branch asks this.)
    func resolves(_ ref: String) -> Bool {
        let key = Self.normalize(ref)
        return store[key] != nil || textStore[key] != nil
    }

    /// The model does not hand the reference over bare: because the tools return
    /// "data_ref=calendar-1" to the model, the model copies THE WHOLE KEY-VALUE PAIR into
    /// sourceRef. The old matching only stripped quotes/whitespace, so the store was
    /// missed, and a miss (P0-2) turned into a silent empty document. Stripping the prefix
    /// closes that entire class; a ref that does not match now REALLY does not exist.
    ///
    /// THE PREFIX LIST TRACKS THE SCHEMA FIELD NAME. It was `kaynakref`/`kaynak_ref` while
    /// the argument was Turkish; the field is now `sourceRef`, so those spellings would
    /// never arrive again and the English ones would go unstripped.
    static func normalize(_ raw: String) -> String {
        let junk = CharacterSet(charactersIn: " \t\n\r\"'`()[]{},;")
        var s = raw.trimmingCharacters(in: junk)
        // "data_ref=calendar-1", "sourceRef: calendar-1", "ref = calendar-1" …
        for prefix in ["data_ref", "sourceref", "source_ref", "dataref", "ref"] {
            guard s.lowercased().hasPrefix(prefix) else { continue }
            let remaining = s.dropFirst(prefix.count).drop(while: { $0 == " " })
            // The separator is MANDATORY: without it a legitimate ID like "reference-1"
            // would be clipped.
            guard let first = remaining.first, first == "=" || first == ":" else { continue }
            s = String(remaining.dropFirst()).trimmingCharacters(in: junk)
            break
        }
        return s
    }

    var isEmpty: Bool { store.isEmpty && textStore.isEmpty }

    /// Keys only — to return a FACT ("these are what is in hand") to the model on an
    /// unresolvable-ref error (not an imperative directive, just the list of live
    /// addresses).
    var refKeys: [String] { (Array(store.keys) + Array(textStore.keys)).sorted() }

    /// The references in hand — in the form "ref (label, N rows)".
    ///
    /// A profile change REBUILDS the session and the transcript drops to a summary; during
    /// that the `sourceRef`s the tools returned were falling out of the model's context.
    /// The measured result: after "prayer times" were searched and found, saying "make a
    /// table" switched to the document profile, the model could not see the data it held
    /// and mistook the EXAMPLE in the skill file for real content, producing an unrelated
    /// file. The data was not lost — only its address had been forgotten.
    ///
    /// THIS STRING GOES TO THE MODEL: its wording must stay English, like every other
    /// tool-facing text.
    var references: [String] {
        let tables = store.keys.sorted().compactMap { ref -> String? in
            guard let t = store[ref] else { return nil }
            return "\(ref) (\(t.headers.joined(separator: "/")), \(t.rows.count) rows)"
        }
        let texts = textStore.keys.sorted().compactMap { ref -> String? in
            guard let m = textStore[ref] else { return nil }
            return "\(ref) (text, \(m.count) characters)"
        }
        return tables + texts
    }

    /// Empties the store when switching to a new chat.
    ///
    /// The counter is DELIBERATELY not reset: it keeps increasing monotonically. Had it
    /// been reset, a new chat would produce "calendar-1" again and an old reference left
    /// in the model's context (or in the summary text) would hit a brand new table — the
    /// user would get a document produced from another chat's data. A silent and
    /// hard-to-diagnose data mix-up; keeping the IDs unique is cheap and removes it
    /// entirely.
    func clear() {
        store.removeAll()
        textStore.removeAll()
        order.removeAll()
        sizes.removeAll()
        totalCharacters = 0
        // The dropped-ref list goes too: saying "it existed but was dropped" in a new chat
        // would be wrong, that data never belonged to this chat.
        dropped.removeAll()
    }
}
