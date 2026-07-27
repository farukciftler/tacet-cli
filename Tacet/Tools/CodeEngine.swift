//
//  CodeEngine.swift
//  Tacet
//
//  The JavaScriptCore sandbox (code-spec §5.3). Runs the small script the model wrote and
//  captures its output. The sandbox is absolute: NO bridge carrying files, network or device
//  data is ever handed to the JSContext. `print`/`console` are provided by a pure-JS prefix;
//  the only native bridge is the `__g` guard, and it answers exactly one question with a Bool —
//  "is the time/memory budget spent?" (see the Guard note).
//
//  MEASURED RATIONALE (every mechanism in this file is the answer to a measurement):
//
//  1. A JSC context arrives with its OWN `console` object (log/warn/error/…) and that object
//     writes to the system log. Unless it is captured, `console.log('x')` runs without error
//     but the output comes back EMPTY — the model sees "ok" and makes the result up; on top of
//     that user data leaks into the device log. That is why `console` is OVERWRITTEN in the
//     prefix with pure JS and bound to `print`.
//  2. `String(object)` → "[object Object]". Objects/arrays the model printed came back
//     useless; `__s` converts them to JSON.
//  3. Endless loops: JSC has NO cooperative cancellation, so an outside watcher thread can do
//     nothing but abandon — and an abandoned thread eats a core FOREVER. The fix: a pure-JS
//     counter `__c()` is injected into loop conditions, the counter polls the native guard
//     every 256 iterations and throws a JS exception if time or memory is spent. That makes
//     the cancellation REAL; the thread finishes.
//  4. Memory: `while(true){a.push(new Array(1e5))}` reached a ~2 GB resident / ~12 GB peak
//     footprint in measurement before the 3 s outer timeout expired — on iOS that means
//     jetsam, i.e. THE DEATH OF THE APP. An outside watcher cannot catch up with it. That is
//     why the guard also measures the GROWTH of the footprint alongside the time, and stops
//     the script once the cap is exceeded.
//
//  Every call gets a fresh JSVirtualMachine + JSContext (no leaks accumulate). The 3 s outer
//  timeout is kept: it is the last seatbelt if the guard was overwritten or the injection was
//  skipped. Output is truncated at 10,000 characters; the truncation happens INSIDE JS so that
//  a huge output is never bridged to the Swift side at all.
//

import Foundation
import JavaScriptCore

/// The outcome of a code run (code-spec §5.2/§5.3).
enum CodeOutcome {
    /// The script finished without error — the captured output + the elapsed time (milliseconds).
    /// If `output` is EMPTY the script printed nothing: the caller must not present that as a
    /// success (this is the very hole through which the model fabricated results).
    case succeeded(output: String, ms: Int)
    /// The script threw — type + message + line + the TEXT of the offending line.
    case error(String)
    /// Time ran out; the script was stopped by the guard or the context was abandoned.
    case timeout
    /// The memory cap was exceeded; the script was stopped (the app was protected).
    case memoryLimit
}

/// The JSC sandbox engine. It holds no state — every call is independent.
enum CodeEngine {
    /// The output cap: anything above it is truncated (code-spec §5.3).
    static let outputCap = 10_000
    /// The outer timeout (seconds) — the seatbelt that kicks in if the guard does not work.
    static let timeoutDuration: TimeInterval = 3
    /// The inner time cap the guard enforces. It must be SHORTER than the outer one so that
    /// cooperative stopping wins (so the thread really finishes).
    /// 2.7 s was chosen by measurement: SelfTest expects the endless loop to take ≥2.5 s (had
    /// we set 2.5 the margin would be 1.5 ms), and 300 ms is still left to the outer 3 s —
    /// unwinding takes ~1 ms in measurement, so the cooperative path always wins.
    static let guardDuration: TimeInterval = 2.7
    /// The footprint cap the script may add (bytes). If exceeded it is stopped.
    /// 256 MB: above a heavy but legitimate computation, below the jetsam threshold.
    static let memoryCap: UInt64 = 256 << 20
    /// The abandoned-thread cap. Thanks to the guard this path is not taken in practice; in
    /// the overwritten-guard scenario the accumulation is still bounded.
    static let abandonCap = 3

    /// The signature of the exceptions the guard throws — it is TOLD APART from the error text.
    private static let durationSignature = "__tacet_duration"
    private static let memorySignature = "__tacet_memory"

    /// The pure-JS prefix. It is NOT a native bridge (the single exception is `__g`, below).
    /// It is evaluated BEFORE the user code and as a SEPARATE script — that way the user
    /// code's line numbers start at 1.
    private static var prefix: String {
        """
        var __out=[],__len=0,__lim=\(outputCap),__trunc=false,__i=0;
        function __s(v){
          var t=typeof v;
          if(t==='string')return v;
          if(v===null)return 'null';
          if(v===undefined)return 'undefined';
          if(t==='number'||t==='boolean'||t==='bigint'||t==='symbol')return String(v);
          if(t==='function')return '[Function'+(v.name?': '+v.name:'')+']';
          if(v instanceof Error)return v.name+': '+v.message;
          if(v instanceof Date)return isNaN(v)?'Invalid Date':v.toISOString();
          try{var j=JSON.stringify(v);return j===undefined?String(v):j}catch(e){return String(v)}
        }
        function __write(a){
          if(__len>=__lim){__trunc=true;return}
          var s='';
          for(var i=0;i<a.length;i++){if(i)s+=' ';s+=__s(a[i])}
          if(s.length>__lim-__len){s=s.slice(0,__lim-__len);__trunc=true}
          __len+=s.length+1;__out.push(s)
        }
        function print(){__write(arguments)}
        (function(){
          var n=function(){};
          globalThis.console={log:print,info:print,warn:print,error:print,debug:print,
            trace:print,dir:print,dirxml:print,table:print,group:print,groupCollapsed:print,
            groupEnd:n,time:n,timeEnd:n,timeLog:n,count:n,countReset:n,clear:n,
            assert:function(c){if(!c){var a=[].slice.call(arguments,1);a.unshift('Assertion failed:');__write(a)}}};
        })();
        """
    }

    /// Runs the script in the sandbox. It evaluates on a separate Thread and waits on a
    /// semaphore; the guard enforces time/memory from the inside, the outer watcher is only a
    /// seatbelt.
    static func run(_ code: String) async -> CodeOutcome {
        guard AbandonCounter.shared.value < abandonCap else {
            return .error("engine unavailable: too many runs had to be abandoned this session")
        }
        return await withCheckedContinuation { continuation in
            let semaphore = DispatchSemaphore(value: 0)
            let box = OutcomeBox()
            let worker = Thread {
                if box.write(evaluate(code)) { AbandonCounter.shared.decrement() }
                semaphore.signal()
            }
            worker.name = "tacet.codeengine"
            // .utility: if it is abandoned, do not let it spin in the same priority band as the UI.
            worker.qualityOfService = .utility
            worker.start()
            DispatchQueue.global(qos: .utility).async {
                if semaphore.wait(timeout: .now() + timeoutDuration) == .timedOut {
                    if !box.abandon() { AbandonCounter.shared.increment() }
                    continuation.resume(returning: .timeout)
                } else {
                    continuation.resume(returning: box.read() ?? .error("no result"))
                }
            }
        }
    }

    /// The real evaluation — runs concurrently, on the worker thread.
    private static func evaluate(_ code: String) -> CodeOutcome {
        let vm = JSVirtualMachine()
        guard let context = JSContext(virtualMachine: vm) else {
            return .error("engine unavailable")
        }

        // MARK: The guard — the only native bridge.
        // The sandbox rationale: this block sees NO FILES, NO NETWORK and NO DEVICE DATA; it
        // takes no input and returns no data. The one thing it does is throw a JS exception if
        // the time/memory cap is exceeded. It adds no capability beyond what `Date.now()`
        // already gives — the sandbox surface does not widen, it NARROWS (the endless loop,
        // which was an escape route, is closed).
        let end = DispatchTime.now().uptimeNanoseconds
            &+ UInt64(guardDuration * 1_000_000_000)
        let startFootprint = footprint()
        let guardBlock: @convention(block) () -> Bool = {
            guard let active = JSContext.current() else { return true }
            if DispatchTime.now().uptimeNanoseconds > end {
                active.exception = JSValue(newErrorFromMessage: durationSignature, in: active)
                return true
            }
            let now = footprint()
            if now > startFootprint, now &- startFootprint > memoryCap {
                active.exception = JSValue(newErrorFromMessage: memorySignature, in: active)
            }
            return true
        }
        context.setObject(guardBlock, forKeyedSubscript: "__g" as NSString)

        // Exception capture: the first error + its line/column are kept.
        var errorMessage: String?
        var errorLine = 0
        context.exceptionHandler = { _, exception in
            guard errorMessage == nil else { return }
            errorMessage = firstLine(exception?.toString() ?? "unknown error")
            errorLine = Int(exception?.objectForKeyedSubscript("line")?.toInt32() ?? 0)
        }

        context.evaluateScript(prefix)
        // The guard calls are injected into loop conditions. If the injection cannot be done
        // safely (ambiguous syntax) the code runs AS IS — the outer timeout still protects us.
        let runnable = GuardInjection.apply(code)
        let start = DispatchTime.now()
        var lastValue = context.evaluateScript(runnable)

        // TOP-LEVEL `return` RESCUE (measured: the most frequent error in the code category).
        // The small model often writes the script as if it were a function body and hands the
        // result back with `return`; in global scope that is a SyntaxError and the turn fell
        // through to "I couldn't do it". The code is wrapped in an IIFE and tried ONE more time.
        //
        // Why we do not wrap unconditionally: wrapping swallows the last-expression value
        // (`6*7` → undefined), whereas the last value of a print-less script is deliberately
        // counted as output. So it engages ONLY on THIS syntax error; the successful path and
        // the other error paths stay bit-for-bit identical.
        if let raw = errorMessage, Self.isTopLevelReturn(raw) {
            errorMessage = nil
            errorLine = 0
            // A syntax error ran nothing, so there are no side effects; the output buffer is
            // still cleared so no partial output leaks.
            context.evaluateScript("__out.length=0;__trunc=false")
            lastValue = context.evaluateScript("(function(){\n\(runnable)\n})()")
        }
        let ms = Int((DispatchTime.now().uptimeNanoseconds &- start.uptimeNanoseconds) / 1_000_000)

        // The output is read EVEN IF there was an error: showing the model "how far it got"
        // markedly improves the accuracy of the second attempt.
        var output = collectOutput(context)

        if let raw = errorMessage {
            if raw.contains(durationSignature) { return .timeout }
            if raw.contains(memorySignature) { return .memoryLimit }
            return .error(errorReport(raw, line: errorLine, source: code, partialOutput: output))
        }

        // If print was never called, the value of the last expression counts as the output
        // (SelfTest: `print(6*7)` → "42"; `6*7` must give "42" too).
        if output.isEmpty, let lastValue, !lastValue.isUndefined, !lastValue.isNull {
            let shorten = context.evaluateScript(
                "(function(v){try{return __s(v).slice(0,\(outputCap + 1))}catch(e){return ''}})")
            output = shorten?.call(withArguments: [lastValue])?.toString() ?? ""
            if output.count > outputCap {
                output = String(output.prefix(outputCap)) + "\n" + L10n.codeOutputTruncated
            }
        }
        return .succeeded(output: output, ms: ms)
    }

    /// Is the error text the "return in global scope" syntax error?
    ///
    /// JSC reports it as "SyntaxError: Return statements are only valid inside functions."
    /// The `syntaxerror` CONDITION IS MANDATORY: without it a RUNTIME error such as
    /// `TypeError: x.return is not a function` also contains "return"+"function", so the whole
    /// script ran a second time inside an IIFE (side effects repeated), the partial output was
    /// wiped and the real error report was lost. A syntax error runs nothing — retrying is
    /// safe only there.
    private static func isTopLevelReturn(_ raw: String) -> Bool {
        let h = raw.lowercased()
        guard h.contains("syntaxerror") else { return false }
        return h.contains("return") && h.contains("function")
    }

    /// Bridges the contents of `__out`. The truncation happens INSIDE JS: data above the cap
    /// is never copied to the Swift side (otherwise `print('x'.repeat(5e8))` would push
    /// hundreds of MB across the bridge BEFORE the truncation — jetsam/OOM).
    private static func collectOutput(_ context: JSContext) -> String {
        let raw = context.evaluateScript(
            "(function(){try{return __out.join('\\n').slice(0,\(outputCap + 1))}catch(e){return ''}})()")
        var output = raw?.toString() ?? ""
        // If the JSValue is nil/undefined, toString() gives "undefined" — that does not count as output.
        if output == "undefined" { output = "" }
        let truncated = context.evaluateScript("__trunc===true")?.toBool() ?? false
        if output.count > outputCap {
            output = String(output.prefix(outputCap))
        }
        if truncated, !output.isEmpty {
            output += "\n" + L10n.codeOutputTruncated
        }
        return output
    }

    /// The error report that goes to the model. MEASURED LESSON: "SyntaxError" on its own says
    /// nothing to a 3B model; once the TEXT of the offending line is given, the fix locks onto
    /// the target. The report has three parts: type+message, the line number plus the line
    /// itself, and the output printed before the error.
    private static func errorReport(_ message: String, line: Int,
                                    source: String, partialOutput: String) -> String {
        var parts = [message]
        let lines = source.components(separatedBy: "\n")
        if line > 0, line <= lines.count {
            let text = lines[line - 1].trimmingCharacters(in: .whitespaces)
            if text.isEmpty {
                parts.append("at line \(line)")
            } else {
                parts.append("at line \(line): \(String(text.prefix(160)))")
            }
        }
        if !partialOutput.isEmpty {
            parts.append("output before the error:\n\(String(partialOutput.prefix(300)))")
        }
        return parts.joined(separator: "\n")
    }

    /// The process footprint (the metric jetsam looks at). If it cannot be read it returns 0
    /// and the memory guard silently switches off — the time guard keeps working.
    private static func footprint() -> UInt64 {
        var info = task_vm_info_data_t()
        var count = mach_msg_type_number_t(
            MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<integer_t>.size)
        let result = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
            }
        }
        return result == KERN_SUCCESS ? UInt64(info.phys_footprint) : 0
    }

    /// Only the first line of the error text goes to the model (code-spec §5.2).
    private static func firstLine(_ text: String) -> String {
        text.split(separator: "\n", maxSplits: 1,
                   omittingEmptySubsequences: false).first.map(String.init) ?? text
    }
}

// MARK: - Guard injection

/// Places a pure-JS guard call (`__c()`) into loop conditions.
///
/// WHY IT IS NEEDED: there is no public way to stop a running script in JSC from the outside
/// (`JSContextGroupSetExecutionTimeLimit` is private API and carries App Store risk — it was
/// deliberately NOT USED). An endless loop burns a core forever even if the outer watcher
/// reports a timeout. The `__c()` injected into the condition runs on every iteration and
/// makes the cancellation REAL.
///
/// WHY THE CONDITION AND NOT THE BODY: body injection breaks brace-less loops
/// (`while(x) y();`) and shifts line numbers. Entering the condition with a comma/`&&` is
/// always valid syntax and STAYS ON ONE LINE — the error line numbers are not disturbed.
///
/// SAFETY: `for`/`while` words inside strings, template strings, regular expressions and
/// comments are FILTERED OUT by a lexer. If the lexer ends in a strange state (unterminated
/// string/comment) the injection is skipped entirely — better to trust the outer timeout than
/// to break working code.
enum GuardInjection {
    /// The counter that runs on every iteration. MEASURED: wrapping this in a JS function
    /// (`__c()`) turned a tight 3e7-iteration loop from 222 ms into 886 ms; the inline
    /// expression stays at 556 ms (35% cheaper). The native guard is polled once every 256
    /// iterations — the bridge crossing is not on the hot path.
    private static let counter = "((++__i&255)||__g())"

    static func apply(_ code: String) -> String {
        let k = Array(code)
        guard let mask = codeMask(k) else { return code }

        // The injection points are applied back to front so the indices do not shift.
        var additions: [(index: Int, text: String)] = []
        var i = 0
        while i < k.count {
            guard mask[i], let length = keyword(k, i, mask) else { i += 1; continue }
            // Find the first '(' after the keyword.
            var j = i + length
            while j < k.count, mask[j], k[j] == " " || k[j] == "\n" || k[j] == "\t" || k[j] == "\r" { j += 1 }
            guard j < k.count, mask[j], k[j] == "(" else { i += length; continue }
            guard let closing = matchingParen(k, j, mask) else { i += length; continue }

            let word = String(k[i..<(i + length)])
            if word == "while" {
                // while (CONDITION) → while (COUNTER, (CONDITION))
                additions.append((j + 1, "\(counter),("))
                additions.append((closing, ")"))
            } else {
                // for: only the classic three-part form. for-of/for-in have no condition;
                // they are LEFT ALONE (finite iterators).
                guard let semicolons = topLevelSemicolons(k, j, closing, mask),
                      semicolons.count == 2 else { i += length; continue }
                let conditionStart = semicolons[0] + 1
                let conditionEnd = semicolons[1]
                let empty = (conditionStart..<conditionEnd).allSatisfy { k[$0] == " " || k[$0] == "\t" }
                if empty {
                    // for(;;) → for(;COUNTER,true;) — a truth value is needed in place of the condition.
                    additions.append((conditionStart, "\(counter),true"))
                } else {
                    additions.append((conditionStart, "\(counter),("))
                    additions.append((conditionEnd, ")"))
                }
            }
            i = closing + 1
        }
        guard !additions.isEmpty else { return code }

        var result = k
        for addition in additions.sorted(by: { $0.index > $1.index }) {
            result.insert(contentsOf: Array(addition.text), at: addition.index)
        }
        return String(result)
    }

    /// If there is a real `while`/`for` keyword at position `i`, returns its length.
    /// It is filtered out if the preceding character is part of an identifier (`.for`, `xfor`).
    private static func keyword(_ k: [Character], _ i: Int, _ mask: [Bool]) -> Int? {
        for word in ["while", "for"] {
            let n = word.count
            guard i + n <= k.count, String(k[i..<(i + n)]) == word else { continue }
            if i > 0, isIdentifierPart(k[i - 1]) || k[i - 1] == "." { return nil }
            if i + n < k.count, isIdentifierPart(k[i + n]) { return nil }
            return n
        }
        return nil
    }

    private static func isIdentifierPart(_ c: Character) -> Bool {
        c.isLetter || c.isNumber || c == "_" || c == "$"
    }

    /// Returns the index of the ')' matching the '(' at position `opening`.
    private static func matchingParen(_ k: [Character], _ opening: Int, _ mask: [Bool]) -> Int? {
        var depth = 0
        var i = opening
        while i < k.count {
            if mask[i] {
                if k[i] == "(" { depth += 1 }
                else if k[i] == ")" {
                    depth -= 1
                    if depth == 0 { return i }
                }
            }
            i += 1
        }
        return nil
    }

    /// The indices of the TOP-LEVEL semicolons (not inside parens/brackets/braces) within `for(...)`.
    private static func topLevelSemicolons(_ k: [Character], _ opening: Int,
                                           _ closing: Int, _ mask: [Bool]) -> [Int]? {
        var depth = 0
        var found: [Int] = []
        for i in (opening + 1)..<closing where mask[i] {
            switch k[i] {
            case "(", "[", "{": depth += 1
            case ")", "]", "}": depth -= 1
            case ";" where depth == 0:
                found.append(i)
                if found.count > 2 { return nil }
            default: break
            }
        }
        return found
    }

    /// A per-character "is this CODE?" mask. Anything inside a string/template/comment/regex
    /// becomes `false`. If the lexer ends in an unexpected state it returns `nil` and the
    /// caller skips the injection entirely.
    private static func codeMask(_ k: [Character]) -> [Bool]? {
        var mask = [Bool](repeating: true, count: k.count)
        // Template string `${}` can nest; the depth is tracked with a stack.
        var templateStack: [Int] = []
        var braceDepth = 0
        var i = 0
        // The last meaningful code character, used to tell a regex from a division.
        var lastMeaningful: Character?

        while i < k.count {
            let c = k[i]
            if c == "'" || c == "\"" {
                let quote = c
                mask[i] = false
                i += 1
                while i < k.count {
                    mask[i] = false
                    if k[i] == "\\" { if i + 1 < k.count { mask[i + 1] = false }; i += 2; continue }
                    if k[i] == quote { i += 1; break }
                    if k[i] == "\n" { return nil } // unterminated string
                    i += 1
                }
                lastMeaningful = "\""
                continue
            }
            if c == "`" {
                mask[i] = false
                i += 1
                var closed = false
                while i < k.count {
                    if k[i] == "\\" { mask[i] = false; if i + 1 < k.count { mask[i + 1] = false }; i += 2; continue }
                    if k[i] == "`" { mask[i] = false; i += 1; closed = true; break }
                    if k[i] == "$", i + 1 < k.count, k[i + 1] == "{" {
                        // The inside of `${` is CODE; normal lexing until the brace closes.
                        mask[i] = false; mask[i + 1] = true
                        templateStack.append(braceDepth)
                        braceDepth += 1
                        i += 2
                        // Break out so the inner code is handled by the normal loop.
                        break
                    }
                    mask[i] = false
                    i += 1
                }
                if closed || i >= k.count {
                    if !closed { return nil }
                    lastMeaningful = "\""
                }
                continue
            }
            if c == "/", i + 1 < k.count, k[i + 1] == "/" {
                while i < k.count, k[i] != "\n" { mask[i] = false; i += 1 }
                continue
            }
            if c == "/", i + 1 < k.count, k[i + 1] == "*" {
                mask[i] = false; mask[i + 1] = false
                i += 2
                var closed = false
                while i + 1 < k.count {
                    if k[i] == "*", k[i + 1] == "/" { mask[i] = false; mask[i + 1] = false; i += 2; closed = true; break }
                    mask[i] = false; i += 1
                }
                if !closed { return nil }
                continue
            }
            if c == "/", regexStart(lastMeaningful) {
                mask[i] = false
                i += 1
                while i < k.count {
                    mask[i] = false
                    if k[i] == "\\" { if i + 1 < k.count { mask[i + 1] = false }; i += 2; continue }
                    if k[i] == "[" { // a character class: '/' may appear unescaped
                        while i < k.count, k[i] != "]" {
                            mask[i] = false
                            if k[i] == "\\" { if i + 1 < k.count { mask[i + 1] = false }; i += 1 }
                            i += 1
                        }
                        if i < k.count { mask[i] = false }
                        i += 1
                        continue
                    }
                    if k[i] == "/" { i += 1; break }
                    if k[i] == "\n" { return nil }
                    i += 1
                }
                while i < k.count, k[i].isLetter { mask[i] = false; i += 1 } // flags
                lastMeaningful = ")"
                continue
            }
            if c == "{" { braceDepth += 1 }
            if c == "}" {
                braceDepth -= 1
                if let expected = templateStack.last, braceDepth == expected {
                    // `${...}` closed; we go back to the rest of the template string.
                    templateStack.removeLast()
                    mask[i] = false
                    i += 1
                    var closed = false
                    while i < k.count {
                        if k[i] == "\\" { mask[i] = false; if i + 1 < k.count { mask[i + 1] = false }; i += 2; continue }
                        if k[i] == "`" { mask[i] = false; i += 1; closed = true; break }
                        if k[i] == "$", i + 1 < k.count, k[i + 1] == "{" {
                            mask[i] = false; mask[i + 1] = true
                            templateStack.append(braceDepth)
                            braceDepth += 1
                            i += 2
                            closed = true
                            break
                        }
                        mask[i] = false
                        i += 1
                    }
                    if !closed { return nil }
                    lastMeaningful = "\""
                    continue
                }
            }
            if !c.isWhitespace { lastMeaningful = c }
            i += 1
        }
        guard templateStack.isEmpty else { return nil }
        return mask
    }

    /// Is the `/` a regular expression or a division? The standard heuristic: if the
    /// meaningful character before it is the end of a VALUE it is a division, otherwise a regex.
    private static func regexStart(_ lastMeaningful: Character?) -> Bool {
        guard let s = lastMeaningful else { return true }
        if s.isLetter || s.isNumber || s == "_" || s == "$" { return false }
        if s == ")" || s == "]" { return false }
        return true
    }
}

// MARK: - Cross-thread transport

/// A locked box that carries the outcome between two threads. On a timeout the worker may
/// write the outcome late — thanks to the lock that write is harmless, and it serializes the
/// abandon/finish race: whoever arrives second sees the first.
private final class OutcomeBox: @unchecked Sendable {
    private let lock = NSLock()
    private var outcome: CodeOutcome?
    private var abandoned = false

    /// Writes the outcome; returns true if the thread had been abandoned before this moment
    /// (it must then be subtracted from the abandon counter — it did come back after all).
    func write(_ s: CodeOutcome) -> Bool {
        lock.lock(); defer { lock.unlock() }
        outcome = s
        return abandoned
    }

    /// Marks the abandonment; returns true if the outcome had been written before this moment
    /// (the thread already finished, so it does NOT COUNT as abandoned).
    func abandon() -> Bool {
        lock.lock(); defer { lock.unlock() }
        abandoned = true
        return outcome != nil
    }

    func read() -> CodeOutcome? {
        lock.lock(); defer { lock.unlock() }
        return outcome
    }
}

/// The abandoned-thread counter — a single process-wide instance, locked.
/// It goes up at the moment of the timeout and down when an abandoned job finishes late.
private final class AbandonCounter: @unchecked Sendable {
    static let shared = AbandonCounter()
    private let lock = NSLock()
    private var count = 0

    var value: Int {
        lock.lock(); defer { lock.unlock() }
        return count
    }
    func increment() { lock.lock(); count += 1; lock.unlock() }
    func decrement() { lock.lock(); count -= 1; lock.unlock() }
}
