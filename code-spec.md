# Tacet — Code Execution and Web Page Layer Specification

**Version:** 0.1 (draft) · **Date:** 19 July 2026 · **Depends on spec:** tacet-spec.md §7 (tool catalog, context budget), skill layer (SkillStore)
**Status:** Implemented — read this spec together with the code as it stands today

---

## 1. Summary

Two new generation capabilities:

- **A. Web page generation** — when you say "build me a site", the model produces content, the app pours it into a single-file HTML page in the Tacet design language, **verifies** it and shows a preview.
- **B. Code execution** — when you say "do that calculation in code", "solve it with python", the model writes a small script, **runs it in a sandbox** on the device, verifies the output and presents only the verified result.

The shared backbone of both is the Claude Code logic:

> **Write → run → verify → present.**
> The model does not claim a result; a tool runs it, verification happens in code, and every stage appears on screen as a tool chip. A result that cannot be verified is not presented — the failure is stated honestly.

---

## 2. Principles

1. **Verification in code, no claims.** The "ran it" chip only lands if the code actually ran. Verification is done by the tool, not the model (timeout, error capture, output check). A continuation of product principle #4.
2. **Stages are visible.** Every attempt is a tool chip: "Code ran ✓" or "Error · retrying". The user sees how many attempts it took to get to the result; failure is not hidden.
3. **The sandbox is absolute.** Executed code cannot reach the file system, the network, or device data. This is not a setting, it is how the engine is constructed (§5.3).
4. **The budget is sacred.** Code generation burns tokens. The model does NOT write the full HTML skeleton (the template is in the app, §4.2); a code attempt is capped at 2 (§5.4).

---

## 3. Why two separate paths

"Build a site" and "compute it in code" are not the same feature:

| | Web page (A) | Code execution (B) |
|---|---|---|
| What the model produces | Content (markdown sections) | A script to run |
| What the engine does | Pour into a template + verify | Run it + capture the output |
| Verification | Does the page load, is the console clean | Did the script finish without error, is there output |
| Output | An .html file + preview | The result in the chat (+ a file if asked) |
| New tool | None — an `.html` format for `create_document` | `run_code` (+ `write_code`, §5.4b) |

If the model wrote the HTML by hand, the 4096 window would run out in a few sections and output quality would be at the mercy of the model's CSS knowledge. Instead the established document-layer pattern is followed: the model produces markdown, the engine pours it into a format (whatever `create_document` → `DocxEngine/PdfEngine` is, `HtmlEngine` is the same).

---

## 4. A — Web page generation

### 4.1 Flow

1. User: "build a site for my coffee shop".
2. The model calls `create_document(format:"html", file_name:"coffee-shop", content:<markdown>)`. The content is ordinary markdown: an `#` heading becomes the hero section, `##` headings become page sections, tables become a price list, `-` lists become feature cards.
3. `HtmlEngine` parses the markdown and pours it into a **template embedded in the app**: single file, self-contained (NO external font/CSS/JS request — the page carries the network promise too), responsive, light/dark aware, in the spirit of Tacet typography.
4. If **verification (§4.3)** passes, the chip "Page created · coffee-shop.html" lands and the preview opens. If it does not pass, the chip drops to the error state and an error is returned to the model.

### 4.2 HtmlEngine

`Tools/HtmlEngine.swift` — conforms to the `DocumentEngine` protocol (`write`/`read`), and `.html` is added to `DocumentFormat` (label "Page", icon NOT `globe` — that suggests the network — but `richtext.page` or `doc.text.image`; "html", "site", "page", "web" in the `userText` mapping).

- The template is a constant inside Swift (it could also be a single .html template file in the bundle); the model never sees the template.
- `read` extracts the plain text back (tags are stripped) — so the "add a section to the site" flow works for free through the `read_document` → `edit_document` chain (the `workableDocument` pattern is already in place).
- A multi-page site is out of scope for v1 (§9).

### 4.3 Verification

With an off-screen `WKWebView` (never shown to the user):

1. The file is loaded with `loadFileURL`; 3 s timeout.
2. A navigation error or failure to load → fail.
3. A JS error hitting the console (a `window.onerror` bridge via `WKUserScript`) → fail.
4. If it passes, the tool result is returned; if it does not, a `ToolOutcome` error state + a short reason to the model.

Because the template is a constant in the app, verification should in practice always pass; its reason for existing is to catch a regression in the **markdown parsing**, not in the template (a broken table, an unescaped `<`). A verification failure is a template bug and must be caught in SelfTest — the user seeing it is the exception.

### 4.4 Preview

After generation, `DocumentContext.outputAdded` → the existing preview channel. QuickLook renders the HTML; if that is not enough, a WKWebView path is added to `DocumentPreview` (for `.html` only, `allowsContentJavaScript` on, network requests rejected via `WKNavigationDelegate` — since the page is self-contained there is no legitimate request).

---

## 5. B — Code execution (`run_code`)

### 5.1 Engine choice: JavaScriptCore first, Python a deliberate second step

| | JavaScriptCore (v1) | Embedded Python (v1.5+) |
|---|---|---|
| Size cost | 0 (embedded in iOS) | ~60-80 MB (Python.xcframework) |
| Setup | `import JavaScriptCore`, done | Python-Apple-support (BeeWare) + stdlib pruning |
| Sandbox | `JSContext` natively knows nothing of files/network | `socket`, `ctypes`, `subprocess` etc. must be stripped from stdlib BY HAND |
| App Store | No issue | 2.5.2 compliant (an embedded interpreter is allowed; downloading remote code is not — there is no network anyway) |
| Model fit | A small model writes JS as well as it writes Python | The word "python" being a product promise |

**Decision:** v1 ships with JavaScriptCore. The `run_code` tool contract is designed language-independent (a `language` parameter, in v1 only `"js"`); when Python is added the tool does not change, an engine is added. If the user says "with python", in v1 the model solves it with JS and does NOT SAY SO — if the result is correct the language is an implementation detail; the skill guide regulates this.

**The concrete path to adding Python (v1.5):** `Python.xcframework` from the BeeWare **Python-Apple-support** package is embedded in the project; `Py_Initialize` happens not at app start but on the first `run_code(language:"python")` call (cold start ~1 s, once). The network/file/process modules of stdlib (`socket`, `ssl`, `ctypes`, `subprocess`, `multiprocessing`, the dangerous edges of `os`) are removed from the bundle or blocked with an import hook. `sys.stdout` is captured; there is NO pip, and NO third-party packages.

### 5.2 Tool contract

```swift
struct RunCodeTool: TacetTool {
    let name = "run_code"
    // description: "Runs a short script in a sandbox and returns its output.
    //  Call this for any calculation or transformation too complex for the
    //  calculate tool (loops, dates, text processing, simulations). Write
    //  minimal code that PRINTS the final result. If the tool returns an
    //  error, fix the code and call it ONCE more."

    @Generable struct Arguments {
        @Guide(description: "The script. Keep it minimal; print the result.")
        var code: String
        @Guide(description: "js")   // in v1.5: "js | python"
        var language: String
    }
}
```

The return value is kept small for the model: `ok (312 ms)\n<first 500 characters of the output>` or `error: <first line + line no>`. The full output goes to the chip via `rawOutput` (visible on tap — the ToolTrace pattern).

### 5.3 Sandbox rules

- NO native bridge is handed to `JSContext` (no `setObject`): files, network and device data are physically unreachable.
- Timeout **3 s**: it runs on a separate thread; when the time is up the context is abandoned (JSC has no cooperative cancellation — the context is thrown away and the result becomes "timeout").
- Memory: a single-use context per `JSVirtualMachine`; a fresh VM per call (no leak accumulation).
- Output cap 10,000 characters; anything over is truncated and the truncation is stated.
- An infinite loop / heavy computation shows up to the user as a "code timed out" chip — no silent freeze.

### 5.4 The verification loop (Claude Code logic, on a small model's budget)

1. The model writes the code and calls `run_code` → **chip 1**: "Running code…" → result.
2. If an error came back the model fixes the code and calls **one more time** → **chip 2**.
3. If the second attempt also fails, the tool returns `error_final: give the user a short honest answer, do NOT retry` to the model. The attempt counter is kept in the tool (reset in `ToolExecutor.newTurn`) — the model is not expected to count, and a third call is refused by the tool.
4. On success: the model presents the output in the user's language. It cannot say "I ran it" WITHOUT stating the result — the result text also sits in the chip, so a contradiction is visible.

Why 2? A measured fact: what the small model fixes on attempt 2 it also fixes on attempt 3; what it cannot fix, a loop does not rescue — it eats the window. (Consistent with the skill-layer regression lesson.)

### 5.4b `write_code` — delivery as a file (Rust port, 26 Jul 2026)

`run_code` is a calculator: it runs the script, returns the **output**, and deletes the file. When the user says "write me that script" what they want is not output but **a file** — and that file working must be a fact, not a claim. `write_code` (`tacet-tools/src/write_code.rs`) closes this gap and is the full form of the backbone in §1:

1. **Syntax** — `node --check` / `python3 -m py_compile`, under the *same* shield as `run_code` (network cut, home directory closed). The code is checked formally without running at all.
2. **Execution** — the script runs in the sandbox's **hidden** `code/` folder; its by-products (files it opens, `__pycache__`) do not leak into the user's directory.
3. **Delivery** — if both stages pass, the file is written to the working directory (0600) and the chip says "Code verified · `name.py` · 30 ms". If they do not pass, **the file is never created**: leaving broken code in the user's directory is the same class of lie as saying "created" and leaving an empty file.

The attempt budget is **shared** with `run_code` (the same `CodeState`): the budget is not per tool but per model-code run within the turn. An existing file is not overwritten; `name-2.py` is produced instead.

**The code channel is an array of lines, not a single escaped string** (`lines: array of text`). Measurement: Qwen3-8B writes the same script flawlessly twice in the thinking block (raw text, real newlines), then corrupts it while pouring it into the `"code": "...\n    if n..."` form — `prime_numbers` → `primeumbers`, indentation collapse, `if n` dropped. Same model, same content, two channels: the difference is the channel. Two alternative hypotheses were **eliminated by measurement**: (a) suspected repetition penalty → an exemption was added in the structural region, 141 tokens were generated penalty-free, the corruption continued; (b) suspected grammar mask → a "mask intervention" counter was added, and it showed **zero** interventions inside the code string. The only remaining culprit: the model producing the `\n` escape and then bearing the load of counting indentation after that escape. An array removes that load entirely.

This channel **depends on the structural-region penalty exemption**: an array of lines is repetitive by nature (`", "` separators, the same indentation pattern) and without the exemption the penalty would suppress exactly that repetition. `run_code` was deliberately left alone — there the script is a one-line calculation and it works in measurement.

**Measured result (M4 Pro/Metal):**

| model | "write a prime number script" | "write a Celsius→Fahrenheit script" |
|---|---|---|
| qwen3-4b (default) | ✅ first attempt, 30 ms verification | ✅ |
| qwen3-8b | ❌ never reaches the tool: thinking fills 1790 tokens and gets cut | ✅ on attempt 2 (attempt 1 had `convert_to_f` ≠ `convert_c_to_f`, the loop fixed it) |

The 8B's failure is a **window budget, not a code ability**: a thinking model + an 11-tool prompt (~2300 tokens) does not fit in a 4096 window. Two changes follow from this: `GENERATION_SHARE` 512 → 1024, and more importantly the generation cap is now **derived from the actual length of the prompt** (`TokenCounter::generation_cap`) — because a fixed share is both the minimum for truncation and the cap for generation, short prompts were leaving ~770 tokens unused. Turning off thinking is not a solution: the `<think>\n\n</think>` anchor dropped tool calling from 7/10 to 2/10 (measured, reverted).

**Diagnostics:** `TACET_TRACE_DUMP=1` dumps to stderr the model's raw generation, the thinking block, the assembled script that goes to the tool, its output, and the token accounting (`structural/penalty-free`, `mask intervention`). Without these counters the three hypotheses above would have stayed guesses — and the first guess turned out wrong.

### 5.5 The boundary with `calculate`

`calculate` stays: four-function arithmetic in a single call, through a parser, with an accuracy independent of the model. `run_code` is the layer above it (loops, dates, text processing, simulation). The skill guides draw the line: "a single arithmetic expression → calculate; multiple steps/loops → run_code". Both are in the everyday profile (tool budget in §7).

---

## 6. How the stages look

NO new UI component is needed — the tool chip chain (`ToolTrace`) is already the stage indicator:

```
[ ⚙ Running code… ]                          ← live chip
[ ✕ Error · retrying ]                       ← attempt 1 failed (error state, grey-error not red)
[ ✓ Code ran · 312 ms ]                      ← attempt 2 passed
Result: 42 days.                              ← model reply, serif
```

For the web page: `[ ✓ Page created · coffee-shop.html ]` + preview. Tapping the chip opens the raw input/output (code + stdout) — the transparency principle in its code form: **the user can see what ran.**

---

## 7. Budget and profile

- `run_code` is added to the **everyday profile** (the 8th tool — the cap is under strain; after measurement, `time` can be dropped from the document profile if needed, or traded off via an intent signal).
- The `.html` format adds no tool; `create_document` is already in the document profile. The "site/html/page" traces are added to `ModelService.documentTraces`.
- Code prompt budget: the skill guide says "minimal code, one screen, no comments"; the tool return is truncated at 500 characters (§5.2). The worst turn (code + error + fixed code + output) is ~1200 tokens — it fits in the window, but if a skill injection lands on the same turn, `budgetCheck` is in play.

---

## 8. Test and measurement

- **SelfTest** (needs no model):
  - HtmlEngine: markdown → HTML round-trip (`read` extracts it back), that the template has NO external URL (a search for `http` must come back empty), that verification fails on broken markdown.
  - CodeEngine: `print(6*7)` → "42"; syntax error → `error:` + line; infinite loop → timeout at 3 s; truncation of output above 10k; a file/network access attempt (`fetch`, `require`) staying undefined.
  - Attempt counter: the third call being refused by the tool.
- **Evaluation** (on device): "sum the primes from 1 to 100" → the correct number; "make a coffee shop site" → a single tool call + .html in the chip; on a failed first attempt, the model fixing it and making the SECOND call; after `error_final`, the model giving an honest short answer and not retrying.
- Acceptance criterion: the rate of presenting a wrong result must be near zero — a result is only presented if it came from the tool; if the model fabricates output and says "I ran it", that is the most severe regression (the Router rule + chip visibility are two locks against it).

---

## 9. Out of scope (v1) and open questions

**Out of scope:** multi-page sites, external assets (downloading images/fonts — contradicts the network promise), pip/third-party packages, long-running jobs (>3 s), plotting (we cannot verify canvas output), an editing session for the user's own code (code is a tool, not an editor).

**Open questions:**
1. Is Python's ~70 MB cost worth the product promise of the word "python", or will nobody ask as long as JS stays quiet? (Measurement: how often the "python" trigger occurs in v1.)
2. `run_code` takes the everyday profile up to 8 tools — the over-cap behavior must be measured on device; if needed, code intent becomes its own profile.
3. Will the page template offer the user a theme (color) choice, or does Tacet stay single-voiced? (Recommendation: single voice — consistent with the brand decision.)

---

## 10. Implementation plan (file map)

| Step | File | Work |
|---|---|---|
| 1 | `Models/DocumentFormat.swift` | `.html` case + label/icon/mapping |
| 2 | `Tools/HtmlEngine.swift` | template + markdown rendering + `read`; registration in `DocumentEngines.engine` |
| 3 | `Services/PageVerifier.swift` | off-screen WKWebView verification (§4.3) |
| 4 | `Tools/CodeEngine.swift` | JSC sandbox: run/timeout/truncation (§5.3) |
| 5 | `Tools/RunCodeTool.swift` | the tool + attempt counter (§5.4); adding it to the everyday profile |
| 6 | `ModelService` | site traces in `documentTraces`; profile measurement |
| 7 | `Skills/code.md`, `Skills/web-page.md` | ACTIVATING the triggers (see below) |
| 8 | `SelfTest` + `Evaluation` | the §8 cases |

**Rollout note — skill files (step 7 is DONE; kept for the reasoning):** `code.md` and `web-page.md` used to sit in the repo with the frontmatter key `draft-triggers:`, so `SkillStore.parse` did NOT load them (a skill with no triggers is dropped). Had they become active before the tools landed, the model would have tried to call a tool that did not exist — which is why step 7 was to turn the key into `triggers:` and had to happen in the SAME build as the tools.

Measured while this spec was being updated: both `Tacet/Skills/code.md` and `Tacet/Skills/web-page.md` now carry `triggers:` (plus a `tools:` tag), so the gate described above is closed and step 7 no longer has any work in it. The constraint is recorded here because it applies again to any future skill file drafted ahead of its tool.
