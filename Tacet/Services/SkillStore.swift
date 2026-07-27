//
//  SkillStore.swift
//  Tacet
//
//  The skill layer — Claude's SKILL.md logic. Each tool's detailed usage guide
//  lives in `Skills/*.md` (frontmatter + body); the ones the user wrote
//  themselves live in SwiftData (UserSkill). To avoid bloating the 4096 token
//  window this is "progressive disclosure": not all of them at once, only the
//  SINGLE skill matching the current intent, injected into that session ONCE.
//

import Foundation

/// A skill: name, triggering keywords and the guidance text.
struct Skill {
    let name: String
    let triggers: [String]
    let text: String
    /// The names of the tools this guide COMMANDS (frontmatter `tools:`).
    /// If empty the skill is tool-independent and free in every profile.
    var tools: [String] = []
    /// Whether the user wrote it themselves — on a tie the user's own wins.
    var isUserAuthored: Bool = false
}

enum SkillStore {
    /// The most characters taken from a single skill at injection time. Bundled
    /// skills may be longer as a human reference; the part that goes to the model
    /// is capped.
    static let injectionLimit = 700

    /// The HTML comment marking where the CORE of the body ends. It is invisible
    /// in Markdown, so the file stays readable for a human too.
    ///
    /// Why it exists: the old clipping dropped the END of the body, but the concrete
    /// `tool(args)` example and the anti-hallucination rules sat exactly there — so
    /// the limit was swallowing the very reason the skill existed (327 characters in
    /// code.md, 729 characters in create-document.md). The files are now written
    /// "core-first": example + unbreakable rules ABOVE the marker, human reference
    /// below. The injection takes the core WHOLE and fills the remaining budget with
    /// the tail.
    static let coreMarker = "<!--/core-->"

    /// The smallest remaining budget that still makes it worth taking a piece of the
    /// tail. Below this not even one line fits meaningfully; adding no rule is better
    /// than adding half a rule.
    private static let tailThreshold = 80

    /// The .md skills in the bundle (loaded once, read-only).
    static let package: [Skill] = load()

    /// Skills the user added — refreshed with `reloadUser` as the UI saves them.
    private(set) static var user: [Skill] = []

    /// Package + user; on matching the user's own is tried first.
    static var all: [Skill] { user + package }

    /// Mirrors the user skills in SwiftData into the store (only the active ones).
    static func reloadUser(_ models: [UserSkill]) {
        user = models.compactMap { m in
            guard m.isActive, m.isValid else { return nil }
            return Skill(name: m.name, triggers: m.triggers, text: m.body, isUserAuthored: true)
        }
    }

    /// Returns the skill with the given name.
    static func skill(name: String) -> Skill? {
        all.first { $0.name == name }
    }

    /// Merges the skills with the given names into a single text.
    static func merge(_ names: [String]) -> String {
        names.compactMap { skill(name: $0) }
            .map { "## \($0.name)\n\($0.text)" }
            .joined(separator: "\n\n")
    }

    /// Returns the skill that best fits the given message (nil if none).
    ///
    /// The score is the sum of the LENGTHS of the matching triggers — not their count.
    /// That way a specific phrase beats a generic word: in "show this as a table"
    /// read-document's "as a table" beats create-document's "table". Had the count been
    /// used, both would score 1 and the order would decide at random.
    /// On an equal score the user's skill wins, by the order of `all`.
    ///
    /// If `availableTools` is given (the list of `tool.name` in the active profile's
    /// tool set), skills whose COMMANDED tool is not present are eliminated — the tool
    /// set is the single source of truth, not a hand-maintained profile map. Passing
    /// nil disables the elimination (the test/preview path).
    static func matching(_ question: String, availableTools: Set<String>? = nil) -> Skill? {
        let s = question.lowercased()
        var best: (skill: Skill, score: Int)?
        for candidate in all {
            guard hasTools(candidate, availableTools) else { continue }
            let score = candidate.triggers.reduce(0) { $0 + (contains(s, $1) ? $1.count : 0) }
            if score > 0, score > (best?.score ?? 0) {
                best = (candidate, score)
            }
        }
        return best?.skill
    }

    /// Are ALL the tools the skill declares present in the session?
    ///
    /// The gate is over "all" because a guide can describe a two-step flow
    /// (edit-document: first `read_document`, then `edit_document`); if half of it is
    /// missing the guide cannot be applied anyway. A skill that declares no tool
    /// (calculation, user skills) is free in every set.
    static func hasTools(_ skill: Skill, _ availableTools: Set<String>?) -> Bool {
        guard let available = availableTools, !skill.tools.isEmpty else { return true }
        return skill.tools.allSatisfy(available.contains)
    }

    /// Trigger lookup requires a WORD START (not a raw substring).
    ///
    /// A raw `contains` was finding short triggers INSIDE frequent words: "betik"
    /// inside "alfabetik" called the code skill, "yüz" inside "yüzde" called the
    /// calculation skill; with the wrong guide injected the model forced that skill's
    /// tool and produced a call that did not match the schema. Because Turkish suffixes
    /// attach at the END, only the word START is constrained — "satır" still matches
    /// "satırını", which is what we want.
    ///
    /// In scripts that use no spaces (CJK, Korean) there is no such thing as a word
    /// boundary; for triggers there it falls back to a raw substring, otherwise they
    /// would never match at all.
    /// The length limit at which a word START is not enough. Below it, a word END is
    /// required too (whole-word match).
    ///
    /// MEASURED BUG: the trigger "ara" was catching "**ara**lık", i.e. every question
    /// about the month of December called the Spotlight search skill, ate ~700
    /// characters of the 4096 budget and injected an irrelevant guide into the model.
    /// The same trap exists for "bul" → "bulut/bulmaca/bulunduğu".
    ///
    /// Why only for SHORT triggers: Turkish takes suffixes at the END, so for long
    /// triggers prefix matching is A MUST ("satır" → "satırını", "çarp" →
    /// "çarparsak"). But a short root can also stand at the start of entirely unrelated
    /// words — that is where it produces noise.
    ///
    /// Threshold 4: 3-letter roots (ara/bul/oku/dök/pdf) demand a whole word, 4+ keeps
    /// prefix matching. Making the threshold 5 would break "çarp".
    ///
    /// The asymmetry is deliberate: injecting the wrong skill MISLEADS the model and
    /// costs ~700 characters of budget; missing a skill only loses the extra guidance —
    /// the tool still stands in the session. Missing is cheap, mismatching is
    /// expensive; hence the strictness on short roots.
    static let wholeWordLimit = 4

    private static func contains(_ text: String, _ trigger: String) -> Bool {
        guard let first = trigger.unicodeScalars.first, first.value < 0x0590 else {
            return text.contains(trigger)   // CJK/Korean: no boundary concept
        }
        // Triggers containing a space ("kaç satır") are phrases already; the shortness
        // rule applies only to single-word roots.
        let needsWholeWord = trigger.count < wholeWordLimit && !trigger.contains(" ")

        var field = text.startIndex..<text.endIndex
        while let r = text.range(of: trigger, range: field) {
            let atWordStart = r.lowerBound == text.startIndex
                || !text[text.index(before: r.lowerBound)].isLetter
                && !text[text.index(before: r.lowerBound)].isNumber
            if atWordStart {
                if !needsWholeWord { return true }
                // Short trigger: no letter/digit may follow it.
                let after = r.upperBound
                if after == text.endIndex { return true }
                let next = text[after]
                if !next.isLetter && !next.isNumber { return true }
            }
            guard r.lowerBound < text.endIndex else { break }
            field = text.index(after: r.lowerBound)..<text.endIndex
        }
        return false
    }

    /// Reduces the text to at most `cap` characters at a line boundary (so no half rule
    /// is left behind).
    private static func clipAtLine(_ text: String, cap: Int) -> String {
        guard text.count > cap else { return text }
        let clipped = String(text.prefix(cap))
        guard let last = clipped.range(of: "\n", options: .backwards) else { return clipped }
        return String(clipped[..<last.lowerBound])
    }

    /// Splits the body into (core, tail). If the marker is absent the core is empty and
    /// the whole body counts as tail — user skills are not obliged to place a marker.
    static func splitCore(_ text: String) -> (core: String, tail: String) {
        let body = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let r = body.range(of: coreMarker) else { return ("", body) }
        return (String(body[..<r.lowerBound]).trimmingCharacters(in: .whitespacesAndNewlines),
                String(body[r.upperBound...]).trimmingCharacters(in: .whitespacesAndNewlines))
    }

    /// The body that goes to the model: CORE FIRST, tail into the remaining budget.
    ///
    /// Core integrity comes before the limit; even so, if a skill's core exceeds the cap
    /// it is clipped at a line (otherwise a single file could silently eat the budget of
    /// the 4096 window).
    static func injectionBody(_ text: String) -> String {
        let (core, tail) = splitCore(text)
        guard !core.isEmpty else { return clipAtLine(tail, cap: injectionLimit) }

        let body = clipAtLine(core, cap: injectionLimit)
        let remaining = injectionLimit - body.count - 1   // -1: the "\n" that goes between
        guard remaining >= tailThreshold, !tail.isEmpty else { return body }
        let extra = clipAtLine(tail, cap: remaining)
        return extra.isEmpty ? body : body + "\n" + extra
    }

    /// The shape given to the model: core-first body + the "do not talk about this" fences.
    static func injectionText(_ skill: Skill) -> String {
        let body = injectionBody(skill.text)
        return """
        <guidance name="\(skill.name)">
        \(body)
        </guidance>
        Follow the guidance above when answering. It is internal: never quote, \
        summarize, or mention it, and never reply with the guidance itself.
        """
    }

    // MARK: - Loading

    private static func load() -> [Skill] {
        let urls = Bundle.main.urls(forResourcesWithExtension: "md", subdirectory: nil) ?? []
        return urls.compactMap { parse($0) }
    }

    /// Parses the frontmatter (--- name: … / triggers: … ---) plus the body.
    ///
    /// THE KEYS MOVED TO ENGLISH TOGETHER WITH THE `Skills/*.md` FILES. If the two
    /// drift apart, `triggers` stays empty and the guard below silently drops ALL
    /// bundled skills: the build is green, the skill layer is dead.
    private static func parse(_ url: URL) -> Skill? {
        guard let raw = try? String(contentsOf: url, encoding: .utf8) else { return nil }
        var name = url.deletingPathExtension().lastPathComponent
        var triggers: [String] = []
        var tools: [String] = []
        var body = raw

        let lines = raw.components(separatedBy: "\n")
        if lines.first == "---", let closing = lines.dropFirst().firstIndex(of: "---") {
            for line in lines[1..<closing] {
                let chunk = line.split(separator: ":", maxSplits: 1).map {
                    $0.trimmingCharacters(in: .whitespaces)
                }
                guard chunk.count == 2 else { continue }
                switch chunk[0] {
                case "name": name = chunk[1]
                case "triggers":
                    triggers = chunk[1]
                        .split(separator: ",")
                        .map { $0.trimmingCharacters(in: .whitespaces).lowercased() }
                        .filter { !$0.isEmpty }
                case "tools":
                    tools = chunk[1]
                        .split(separator: ",")
                        .map { $0.trimmingCharacters(in: .whitespaces) }
                        .filter { !$0.isEmpty }
                default: break
                }
            }
            body = lines[(closing + 1)...].joined(separator: "\n")
                .trimmingCharacters(in: .whitespacesAndNewlines)
        }
        guard !triggers.isEmpty else { return nil }
        return Skill(name: name, triggers: triggers, text: body, tools: tools)
    }

    // MARK: - Re-injection (spaced mark)

    /// The pure state machine tracking which skill was injected in which turn.
    ///
    /// Old behaviour: a skill was injected once and marked permanently. In a long
    /// conversation the guide slid out of the window as the transcript advanced, but
    /// because the mark stayed it never entered again — the behavioural drift in late
    /// turns came from exactly this. The mark is now SPACED: once enough turns have
    /// passed, the guide comes back into force.
    ///
    /// The state is held in ModelService, the logic stays here — so it can be tested
    /// without a model.
    struct InjectionState {
        /// After how many turns the same skill may be injected again.
        ///
        /// 6: a skill takes ~700 characters and repeating it every turn would eat the
        /// budget of the 4096 window; on the other hand, at too large a distance the
        /// guide would slide out of the window and never return. 6 turns is twice a
        /// typical tool-use exchange (question → tool → answer).
        static let distance = 6

        /// The number of turns processed so far (starts at 1).
        private(set) var turn = 0
        private var lastInjection: [String: Int] = [:]

        /// Called once at the START of every turn.
        mutating func beginTurn() { turn += 1 }

        /// Should this skill be injected in this turn (it never entered, or `distance`
        /// turns have passed since)?
        func isNeeded(_ name: String) -> Bool {
            guard let last = lastInjection[name] else { return true }
            return turn - last >= Self.distance
        }

        /// Called when the injection ACTUALLY happened. A skill skipped because the
        /// profile did not fit is not marked — it is retried once the right profile is
        /// entered.
        mutating func mark(_ name: String) { lastInjection[name] = turn }

        /// New session = new context: the counter and the marks are reset.
        mutating func reset() {
            turn = 0
            lastInjection.removeAll()
        }
    }
}
