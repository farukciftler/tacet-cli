//
//  RunCodeTool.swift
//  Tacet
//
//  The code-running tool (code-spec §5). The model writes a small script, CodeEngine runs it
//  in the sandbox, and only verified output is presented. Write → run → verify → present;
//  the model does not claim the result, the tool runs it.
//
//  The attempt counter (code-spec §5.4): at most 2 real runs per turn. If the second attempt
//  also fails, `error_final` is returned to the model right then; a third call is rejected
//  WITHOUT running the engine at all (a seatbelt) — the model is not expected to do the
//  counting, the refusal lives in the tool. The counter lives in `CodeState`; the integrator
//  wires it to ToolExecutor's turn hook (it resets per turn).
//
//  THERE IS NO LANGUAGE FIELD in the contract; the only engine is JS — even if "python" is
//  asked for it is solved with JS, the language is an implementation detail (§5.1).
//

import Foundation
import FoundationModels

/// The per-turn attempt counter. The integrator wires `newTurn()` to ToolExecutor's turn hook
/// (`turnHook`) — so the reset happens where code-spec §5.4 says it should, inside
/// ToolExecutor.newTurn and from a SINGLE point.
@MainActor
final class CodeState {
    /// The number of real runs in this turn.
    var attempt = 0
    /// A new turn — the counter is reset (code-spec §5.4).
    func newTurn() { attempt = 0 }
}

struct RunCodeTool: TacetTool {
    let name = "run_code"
    let description = "Runs a short JavaScript in an isolated sandbox with NO network, NO filesystem and NO access to this device or any server. Call this for any calculation or transformation too complex for the calculate tool (loops, dates, text processing, simulations). Date, JSON, Math, RegExp and Intl are available. Never call it to inspect servers, containers, ports, processes, disks, networks or the operating system — it cannot see them, and guessing their state is worse than saying you cannot check. The script MUST print its result with print(...) or console.log(...); a script that prints nothing returns an error. If the tool returns an error, fix the code and call it ONCE more."

    weak var reporter: (any ToolReporter)?
    /// The attempt counter — a weak reference following the reporter pattern, no retain cycle.
    weak var state: CodeState?

    @Generable
    struct Arguments {
        @Guide(description: "The script. Keep it minimal; print the final result.")
        var code: String
        // THE `language` FIELD WAS REMOVED (P2-3). It was a dead field: its only valid value
        // was "js", it was read in no branch at all, yet it consumed a decode slot the small
        // model had to fill on EVERY call. There is already a single engine: JS (§5.1) — the
        // language is an implementation detail, not part of the contract.
    }

    /// The cap on the output returned to the model — the full output goes to the chip (code-spec §5.2).
    private static let modelOutputCap = 500
    /// The cap on the error text. The error report now also carries the source line and the
    /// partial output; it is truncated here so it does not eat the 4096 budget (code-spec §7).
    private static let modelErrorCap = 400

    /// Does the script look like Python? MEASUREMENT FINDING: the small model writes Python in
    /// most code cases (`for i in range(2,101):`, `print(i, end=' ')`), the engine is JS so it
    /// gets a `SyntaxError` and loses the turn. The raw JS parser message ("Unexpected
    /// identifier 'i'") is NOT ENOUGH to fix it: telling a model that believes it wrote Python
    /// "expected '('" leads it to the wrong repair. So the tool states the cause — so the
    /// model's ONE remaining attempt is not wasted.
    ///
    /// Pure and static: testable without the model.
    static func isPython(_ code: String) -> Bool {
        // Each of these tokens is either a syntax error or undefined in JS; there is no risk
        // of mistaking a valid JS script for Python.
        // `print(` is DELIBERATELY absent: the engine recognizes `print` in JS too (code-spec §5.2).
        let pythonSpecific = ["range(", "def ", "elif ", "end=", " True", " False", " None"]
        if pythonSpecific.contains(where: code.contains) { return true }
        // A BLOCK HEADER ENDING IN A COLON. In JS a `for`/`if`/`while` header opens with a
        // paren and the body arrives in braces; it does NOT END with a colon. The
        // `for i from 0 to 19: print(...)` seen in measurement lands exactly here — no
        // `range(`, no ` in `, but not JS either.
        // If there is a `{`, leave it alone: `for (const k in o) { ... }` is valid JS, and
        // object literals carry colons too.
        guard !code.contains("{") else { return false }
        let blockStarts = ["for ", "if ", "while ", "else", "try", "except"]
        return code.contains(":") && blockStarts.contains(where: code.contains)
    }

    /// Builds the error return in a single place: on the last attempt `error_final` is
    /// appended (code-spec §5.4 step 3) — but the CAUSE is stated too, because the honest
    /// short answer the model gives the user has to contain the cause.
    private static func errorText(_ cause: String, lastAttempt: Bool) -> String {
        let short = String(cause.prefix(modelErrorCap))
        guard lastAttempt else { return short }
        return "\(short)\nerror_final: give the user a short honest answer, do NOT retry"
    }

    func call(arguments: Arguments) async -> String {
        await runWithChip(icon: "curlybraces",
                          runningText: L10n.runningCode,
                          rawInput: arguments.code) {
            // The counter lives on the MainActor; it is incremented BEFORE the refusal decision.
            let attemptNo = await MainActor.run { [state] () -> Int in
                // Fail-closed (code-spec §5.4 — "the refusal lives in the TOOL"): if the state
                // was not wired, the cap is not ignored, the call falls into refusal. Silent
                // unlimited running is the worst failure of a forgotten wiring; a loud refusal
                // makes the integration bug visible immediately.
                guard let state else { return 3 }
                state.attempt += 1
                return state.attempt
            }
            // The 3rd and later calls never see the engine (code-spec §5.4): a loop does not
            // rescue what it could not fix, it just eats the window.
            guard attemptNo <= 2 else {
                return ToolOutcome(
                    chipText: L10n.codeRetryLimit,
                    state: .failed(L10n.codeTwoAttempts),
                    toModel: "error_final: give the user a short honest answer, do NOT retry"
                )
            }

            // v1: a single engine, JS (§5.1).
            let outcome = await CodeEngine.run(arguments.code)
            switch outcome {
            case .succeeded(let output, _) where output.isEmpty:
                // A SUCCESS WITHOUT OUTPUT IS NOT A SUCCESS. This used to return "ok (0 ms)\n":
                // the model saw "it ran" and MADE UP the result — in measurement every script
                // using `console.log` landed exactly here. It now counts as an error and the
                // model is steered towards printing (code-spec §2.1 "no claims").
                return ToolOutcome(
                    chipText: attemptNo == 1 ? L10n.codeRetrying
                                             : L10n.codeCouldNotRun,
                    state: .failed("no output"),
                    toModel: Self.errorText(
                        "error: the script ran but printed nothing. Add print(...) for the value you need",
                        lastAttempt: attemptNo >= 2),
                    rawOutput: "no output"
                )
            case .succeeded(let output, let ms):
                let short = String(output.prefix(Self.modelOutputCap))
                return ToolOutcome(
                    chipText: L10n.codeRun(ms),
                    state: .readOk,
                    toModel: "ok (\(ms) ms)\n\(short)",
                    rawOutput: output
                )
            case .error(let message):
                // Failure is not hidden: on the 1st attempt the chip says it is retrying, on
                // the 2nd it falls honestly (code-spec §6). The model also gets `error_final`
                // on the 2nd attempt (code-spec §5.4 step 3) — so that "call it ONCE more" in
                // the description does not push the model into a pointless 3rd call; the 3rd
                // call refusal is only a seatbelt.
                let firstAttempt = attemptNo == 1
                // THE LANGUAGE DIAGNOSIS COMES FIRST. If the script is Python, the raw JS
                // parser message ("Unexpected identifier 'i'. Expected '('") drags the model
                // into the wrong repair: it adds parentheses, does not change the language, the
                // second attempt fails too and the turn is lost. Stating the cause plainly
                // makes the one remaining attempt useful.
                let cause = Self.isPython(arguments.code)
                    ? "error: this script is Python, but the sandbox runs JavaScript ONLY. "
                      + "Rewrite the SAME logic in JavaScript: for (let i = 0; i < n; i++) {...}, "
                      + "console.log(x). Do not use range(), def, or indent blocks.\n\(message)"
                    : "error: \(message)"
                return ToolOutcome(
                    chipText: firstAttempt ? L10n.codeRetrying
                                           : L10n.codeCouldNotRun,
                    state: .failed(message),
                    // The engine now gives type+message+line no+THE TEXT OF THE OFFENDING LINE
                    // and the output printed before the error; all of it goes to the model
                    // (measured: "SyntaxError" alone is not enough for a fix).
                    toModel: Self.errorText(cause, lastAttempt: !firstAttempt),
                    rawOutput: message
                )
            case .timeout:
                // A timeout is an error too: on the 2nd attempt it also falls to error_final.
                return ToolOutcome(
                    chipText: L10n.codeTimedOut,
                    state: .failed(L10n.codeTimeoutCause),
                    toModel: Self.errorText(
                        "error: the script did not finish in time — it probably has an endless loop; bound every loop",
                        lastAttempt: attemptNo >= 2),
                    rawOutput: L10n.codeTimeoutCause
                )
            case .memoryLimit:
                // The memory cap: the script was stopped, the app was protected. Here the model
                // should shrink the loop — this is stated SEPARATELY from the time problem.
                return ToolOutcome(
                    chipText: L10n.codeCouldNotRun,
                    state: .failed("memory limit"),
                    toModel: Self.errorText(
                        "error: the script used too much memory — do not build huge arrays or strings; compute the result incrementally",
                        lastAttempt: attemptNo >= 2),
                    rawOutput: "memory limit"
                )
            }
        }
    }
}
