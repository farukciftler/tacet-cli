//! `write_code` — code PRODUCTION in the spirit of Claude Code: write ->
//! verify -> run -> deliver.
//!
//! `run_code` is a calculator: it runs the script, returns THE OUTPUT and
//! deletes the file. When the user says "write me that script" what they want
//! is not the output but THE FILE — and that the file runs must be a FACT, not
//! a CLAIM. That is exactly this tool's contract: the file is written into the
//! working directory ONLY if it passed both verification stages.
//!
//! THE WORKFLOW (every stage is visible to the user as a chip):
//!
//!   1. SYNTAX — `node --check` / `python3 -m py_compile`, under the shield.
//!      The code is validated formally without running at all; the error line goes to the model.
//!   2. EXECUTION — the script runs in the sandbox's HIDDEN verification folder
//!      (network cut, home directory closed, timeout mandatory — the same
//!      shield and the same runner as `run_code`).
//!   3. DELIVERY — if both stages passed, the file is written into the working
//!      directory and the chip says "verified". If they did not, the file is
//!      NEVER CREATED: leaving broken code in the user's directory is the same
//!      class of lie as leaving an empty file and saying "created" (the bypass lesson of the document layer).
//!
//! THE ATTEMPT BUDGET IS SHARED WITH `run_code` (the same `CodeState`): the
//! budget is not "per tool" but "per model-code run in the turn". Whichever
//! gate the model comes through there are at most two real runs in a turn, and
//! the third is rejected without ever seeing the engine (code-spec §5.4).
//!
//! THE VERIFICATION FOLDER IS HIDDEN (`sandbox/code/`): the by-products of the
//! execution step (files the script opens, `__pycache__`) must not leak into
//! the user's directory. The ONLY thing delivered is the verified source file.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tacet_core::{
    Field, Tool, ToolContext, ToolState, ToolFuture, ToolError, ToolResult, ToolOutcome, ArgSchema,
    TraceUpdate, boxed,
};

use crate::run_code::{
    NetworkShield, MAX_ATTEMPTS, CODE_CAP, CodeState, CodeOutcome, MODEL_OUTPUT_CAP, Interpreter,
    error_text, language_from_source, truncate, run_program, looks_like_python,
};

/// The timeout of the execution verification. The same base as `run_code`'s
/// default; the model CANNOT CHOOSE a duration here — a script to be delivered
/// is already bound by the "short and self-contained" contract.
const EXECUTION_TIMEOUT: Duration = Duration::from_secs(10);
/// A syntax check is cheaper than an execution, but there is also a cold python start.
const SYNTAX_CHECK_TIMEOUT: Duration = Duration::from_secs(10);
/// The most suffixes to try on a name collision. Beyond that it is not
/// `ToolError::OutOfSpace` but a signal in the same spirit: "something is wrong here".
const MAX_NAME_ATTEMPTS: u32 = 100;
/// The most lines a script may have. In a 4096-token window a longer script
/// cannot be produced anyway; the limit lives in the schema so the grammar ENFORCES it.
const MAX_LINES: usize = 200;

/// `write_code` — verifies the code and delivers it as a file in the working directory.
pub struct WriteCodeTool {
    shield: NetworkShield,
    interpreters: Vec<Interpreter>,
    /// The SHARED counter (see the file header).
    state: Arc<CodeState>,
    schema_cache: Mutex<Option<ArgSchema>>,
}

impl WriteCodeTool {
    /// Built from `run_code`'s measured discovery — the shield validation is
    /// not repeated. If `run_code` is not in the catalog this tool is not
    /// either: code that cannot be verified is not delivered, so without the
    /// execution infrastructure the tool itself is meaningless.
    pub fn new(
        shield: NetworkShield,
        interpreters: Vec<Interpreter>,
        state: Arc<CodeState>,
    ) -> WriteCodeTool {
        WriteCodeTool { shield, interpreters, state, schema_cache: Mutex::new(None) }
    }

    /// The target language. The priority is the same as the measurement in
    /// `run_code`: THE SOURCE is evidence and overrides the declaration; if the
    /// source is silent the file EXTENSION speaks (the model writing ".py" is a
    /// declaration too, but it comes after the source); failing that the `language`
    /// field; failing everything, the first interpreter in preference order.
    fn resolve_interpreter(&self, file_name: &str, requested: Option<&str>, code: &str) -> &Interpreter {
        let find = |key: &str| self.interpreters.iter().find(|y| y.key == key);
        if let Some(y) = language_from_source(code).and_then(find) {
            return y;
        }
        if let Some(y) = language_from_extension(file_name).and_then(find) {
            return y;
        }
        if let Some(y) = requested.and_then(find) {
            return y;
        }
        &self.interpreters[0]
    }
}

/// Turns the array of lines into a script.
///
/// A NON-text element is SKIPPED rather than raising an error: the schema
/// already rejects it, and it can only get here through a grammar-less path
/// (eval, a hand-written call); in that case putting the rest through
/// verification loses less than throwing the whole script away for one odd
/// element — code that cannot be verified is NOT DELIVERED anyway, so there is
/// no "silently wrong file" risk.
/// If the model still put a real line break INSIDE an element it is left
/// alone: after joining it lands in the same place and the script is still correct.
fn join_lines(lines: &[serde_json::Value]) -> String {
    lines
        .iter()
        .filter_map(|v| v.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The language guess from the extension in the file name.
fn language_from_extension(file_name: &str) -> Option<&'static str> {
    let small = file_name.trim().to_ascii_lowercase();
    if small.ends_with(".py") {
        Some("python")
    } else if small.ends_with(".js") || small.ends_with(".mjs") {
        Some("js")
    } else {
        None
    }
}

/// Reduces the file name to a single safe STEM: path components are dropped,
/// the hidden-file dot is stripped and a known code extension is removed from
/// the stem (the double-extension lesson: the same failure as
/// `daily_menu.xlsx.xlsx` in `create_document` would be `script.py.py` here).
fn safe_stem(file_name: &str) -> String {
    // The last path component: "..", "/" and "\" never make it into the name.
    let last = file_name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .trim();
    let mut body: String = last
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }
        })
        .collect();
    // Leading dots make it a hidden file; a "delivery" that does not show up in
    // the user's `ls` is not a delivery.
    body = body.trim_start_matches('.').trim_end_matches('.').to_string();
    // Known code extensions are stripped from the stem (case insensitive) — the
    // extension is the tool's choice, not the model's.
    let small = body.to_ascii_lowercase();
    for ext in [".py", ".js", ".mjs"] {
        if small.ends_with(ext) {
            body.truncate(body.len() - ext.len());
            break;
        }
    }
    if body.is_empty() { "script".to_string() } else { body }
}

/// Produces a NON-COLLIDING target path in the working directory: `name.py`, `name-2.py`, ...
///
/// Overwriting an existing file would be wrong in both directions: the user's
/// file silently disappears, or the new delivery lands on top of the previous
/// one and "which version did I run" is left unanswered.
fn empty_target(folder: &Path, body: &str, extension: &str) -> ToolResult<PathBuf> {
    let first = folder.join(format!("{body}.{extension}"));
    if !first.exists() {
        return Ok(first);
    }
    for n in 2..=MAX_NAME_ATTEMPTS {
        let candidate = folder.join(format!("{body}-{n}.{extension}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(ToolError::InvalidArgument(format!("no free name found for {body}.{extension}")))
}

/// The command line of the syntax check. It DOES NOT RUN the script, it only parses it.
fn syntax_check_args(interpreter: &Interpreter, script: &Path) -> Vec<OsString> {
    match interpreter.key {
        "python" => vec!["-m".into(), "py_compile".into(), script.into()],
        _ => vec!["--check".into(), script.into()],
    }
}

impl Tool for WriteCodeTool {
    fn name(&self) -> &str {
        "write_code"
    }

    fn description(&self) -> &str {
        // "verified by actually running it" is not a promise but a fact: the
        // delivery path cannot write a file without going through the
        // verification steps (the single write point is inside `run`). The stdin
        // warning comes from measurement: the sandbox runs with stdin closed, so
        // a script waiting on `input()` fails verification.
        // The "ONE LINE per element" sentence comes FROM MEASUREMENT and should
        // be read together with the schema (see `schema`): a single escaped string was breaking the model.
        "Writes a code file for the user, VERIFIES it and saves it into the working folder. \
         Call this when the user wants a script or program AS A FILE they can keep and run \
         later ('write me a python script that...', 'save the script to a file'). Pass \
         the code as `lines`: an array where each element is ONE line of code - the tool \
         joins them, so never write \\n yourself. Indentation is just leading spaces inside \
         the line. The code is first syntax-checked, then RUN in an isolated sandbox with NO \
         network and NO access to files outside the working folder; the file is saved ONLY if \
         it runs without errors. Write a COMPLETE, SELF-CONTAINED script: never read user \
         input (stdin is closed), use example values instead, and end with a small demo print \
         so the run PROVES the code works. If you only need to READ a result and no file was \
         asked for, use run_code instead. If the tool returns an error, fix the code and \
         call it ONCE more."
    }

    fn schema(&self) -> ArgSchema {
        if let Some(s) = self.schema_cache.lock().expect("schema lock").clone() {
            return s;
        }
        // THE CODE IS ASKED FOR AS AN ARRAY OF LINES, NOT AS A SINGLE ESCAPED STRING.
        //
        // MEASURED (Qwen3-8B, 26 Jul 2026), and the finding is clear: the model
        // writes the same script flawlessly twice IN THE THINKING BLOCK (raw
        // text, real line breaks), then corrupts the same script while pouring
        // it into `"code": "...\n    if n..."` form — `prime_numbers` ->
        // `primeumbers`, indentation collapse, `if n` dropped. Same model, same
        // content, two channels: the difference is THE CHANNEL.
        // Two alternatives were measured and eliminated: (1) the repeat penalty
        // was suspected, an exemption was added in the structural region — 141
        // tokens were produced with no penalty and the corruption CONTINUED;
        // (2) the grammar mask was suspected, a "mask intervention" counter was
        // added — ZERO interventions came out inside the code string. So the
        // culprit is the model's load of producing an escape sequence (`\n`) and
        // counting indentation AFTER that escape. The array removes that load
        // entirely: indentation is now just the spaces right after a quote mark, with no escape sequence.
        // THIS CHANNEL DEPENDS ON THE STRUCTURAL-REGION PENALTY EXEMPTION: an
        // array of lines is repetitive by nature (`", "` separators, the same
        // indentation pattern). Without the exemption the penalty would suppress exactly that repetition.
        //
        // `run_code` was DELIBERATELY not changed: there the script is a
        // one-line computation (`print(sum(range(10)))`) and works on the 4B in
        // measurement; changing the channel would be a regression risk with no measured gain.
        let mut fields = vec![
            Field::new(
                "file_name",
                ArgSchema::text().description(
                    "File name without extension (the tool picks the extension), e.g. 'prime_numbers'.",
                ),
            )
            .required(),
            Field::new(
                "lines",
                ArgSchema::array(ArgSchema::text())
                    .length(Some(1), Some(MAX_LINES))
                    .description(
                        "The script, ONE LINE PER ELEMENT. No \\n escapes; indentation is \
                         leading spaces. Self-contained, no stdin, last line prints a demo.",
                    ),
            )
            .required(),
        ];
        // `language` only if there is a real choice (the same decision as in run_code).
        if self.interpreters.len() > 1 {
            let choices: Vec<&str> = self.interpreters.iter().map(|y| y.key).collect();
            let default = self.interpreters[0].key;
            fields.push(Field::new(
                "language",
                ArgSchema::choice(choices)
                    .description(format!("Script language (default '{default}').")),
            ));
        }
        let schema =
            ArgSchema::object(fields).description("Write, verify and save a code file");
        *self.schema_cache.lock().expect("schema lock") = Some(schema.clone());
        schema
    }

    /// AN EXTERNAL TOOL: it runs model output (the verification step is a run)
    /// and leaves a file on disk.
    fn taints_session(&self) -> bool {
        true
    }

    fn run<'a>(
        &'a self,
        args: serde_json::Value,
        ctx: &'a mut ToolContext,
    ) -> ToolFuture<'a> {
        boxed(async move {
            if let Err(h) = self.schema().validate(&args) {
                return ToolOutcome::failed(&h);
            }

            // The counter is incremented BEFORE THE DENIAL DECISION — exactly as in run_code.
            let attempt = self.state.next_attempt();
            if attempt > MAX_ATTEMPTS {
                return ToolOutcome::new(
                    "Code writing limit",
                    ToolState::Failed("two attempts exhausted".into()),
                    "error_final: give the user a short honest answer, do NOT retry",
                );
            }

            let trace = ctx.start_chip("code", "Writing code…");
            let outcome = self.execute(&args, ctx, attempt);

            // The raw input going to the chip is the JOINED script, not the JSON
            // array: the user (and the diagnostic dump) wants to see what was
            // run, not its representation on the wire.
            let raw_input = args
                .get("lines")
                .and_then(|v| v.as_array())
                .map(|s| join_lines(s))
                .unwrap_or_default();
            ctx.update_chip(
                trace,
                TraceUpdate::state(outcome.state.clone())
                    .text(outcome.chip_text.clone())
                    .raw_input(raw_input)
                    .raw_output(outcome.raw_output.clone().unwrap_or_default()),
            );
            if !matches!(outcome.state, ToolState::Failed(_)) {
                ctx.taint();
            }
            outcome
        })
    }
}

impl WriteCodeTool {
    fn execute(&self, args: &serde_json::Value, ctx: &mut ToolContext, attempt: usize) -> ToolOutcome {
        let last_attempt = attempt >= MAX_ATTEMPTS;

        // JOIN THE LINES. The model does not write `\n`; joining is the tool's
        // job (rationale in `schema`). An empty array and a whitespace-only array are NOT code.
        let Some(lines) = args.get("lines").and_then(|v| v.as_array()) else {
            return ToolOutcome::failed(&ToolError::MissingField("lines".into()));
        };
        let code_body = join_lines(lines);
        if code_body.trim().is_empty() {
            return ToolOutcome::failed(&ToolError::MissingField("lines".into()));
        }
        if code_body.len() > CODE_CAP {
            return ToolOutcome::failed(&ToolError::InvalidArgument("the code is too long".into()));
        }
        let code = code_body.as_str();
        let Some(file_name) =
            args.get("file_name").and_then(|v| v.as_str()).filter(|a| !a.trim().is_empty())
        else {
            return ToolOutcome::failed(&ToolError::MissingField("file_name".into()));
        };

        let interpreter =
            self.resolve_interpreter(file_name, args.get("language").and_then(|v| v.as_str()), code);

        // The sandbox root — the resolved-path requirement is the same as in run_code.
        let sandbox = match ctx.resolve_path(".").and_then(|k| k.canonicalize().map_err(ToolError::Io))
        {
            Ok(k) => k,
            Err(h) => return ToolOutcome::failed(&h),
        };

        // Verification happens in a HIDDEN folder: by-products must not leak into the user's directory.
        let verification = sandbox.join("code");
        if let Err(h) = std::fs::create_dir_all(&verification) {
            return ToolOutcome::failed(&ToolError::Io(h));
        }
        let script = verification.join(format!(
            "verify-{}-{}.{}",
            std::process::id(),
            code.len(),
            interpreter.extension
        ));
        if let Err(h) = std::fs::write(&script, code.as_bytes()) {
            return ToolOutcome::failed(&ToolError::Io(h));
        }
        let outcome = self.validate_and_deliver(code, file_name, interpreter, &sandbox, &script, last_attempt);
        // The verification script is deleted ON EVERY PATH (the same rationale as run_code).
        std::fs::remove_file(&script).ok();
        outcome
    }

    /// Two-stage verification + delivery. The SINGLE write point is at the end
    /// of this function: none of its error branches can leave a file behind.
    fn validate_and_deliver(
        &self,
        code: &str,
        file_name: &str,
        interpreter: &Interpreter,
        sandbox: &Path,
        script: &Path,
        last_attempt: bool,
    ) -> ToolOutcome {
        let verification = script.parent().unwrap_or(sandbox);

        // STAGE 1 — syntax.
        let args = syntax_check_args(interpreter, script);
        match run_program(&self.shield, verification, &interpreter.path, &args, SYNTAX_CHECK_TIMEOUT) {
            Ok(CodeOutcome::Succeeded { .. }) => {}
            Ok(CodeOutcome::Error(message)) => {
                // The language diagnosis comes FIRST (the measurement in
                // run_code): putting Python through the JS check makes the model apply the wrong repair.
                let cause = if interpreter.key == "js" && looks_like_python(code) {
                    format!(
                        "error (syntax): this script is Python, but it would be saved and run \
                         as JavaScript. Rewrite the SAME logic in JavaScript, or if python is \
                         available pass language:\"python\".\n{message}"
                    )
                } else {
                    format!("error (syntax): the code does not parse — fix it, do not change \
                             the logic.\n{message}")
                };
                return ToolOutcome::new(
                    if last_attempt { "Code could not be verified" } else { "Syntax error · retrying" },
                    ToolState::Failed("syntax".into()),
                    error_text(&cause, last_attempt),
                )
                .raw_output(message);
            }
            Ok(CodeOutcome::Timeout) => {
                return ToolOutcome::new(
                    if last_attempt { "Code could not be verified" } else { "Error · retrying" },
                    ToolState::Failed("syntax check timeout".into()),
                    error_text("error (syntax): the syntax check timed out", last_attempt),
                )
                .raw_output("syntax check timeout");
            }
            Err(h) => return ToolOutcome::failed(&h),
        }

        // STAGE 2 — execution. Output IS NOT REQUIRED: what is delivered is the
        // file, the output is only evidence. But it must finish without an error.
        let run = run_program(
            &self.shield,
            verification,
            &interpreter.path,
            &[script.into()],
            EXECUTION_TIMEOUT,
        );
        let (output, ms) = match run {
            Ok(CodeOutcome::Succeeded { output, ms, .. }) => (output, ms),
            Ok(CodeOutcome::Error(message)) => {
                return ToolOutcome::new(
                    if last_attempt { "Code could not be verified" } else { "Runtime error · retrying" },
                    ToolState::Failed("runtime error".into()),
                    error_text(
                        &format!(
                            "error (runtime): the code parses but fails when run. Remember: \
                             stdin is closed, use example values instead of input().\n{message}"
                        ),
                        last_attempt,
                    ),
                )
                .raw_output(message);
            }
            Ok(CodeOutcome::Timeout) => {
                return ToolOutcome::new(
                    if last_attempt { "Code could not be verified" } else { "Error · retrying" },
                    ToolState::Failed("timed out".into()),
                    error_text(
                        "error (runtime): the script did not finish in time — it probably has \
                         an endless loop or waits for input; bound every loop, never read stdin",
                        last_attempt,
                    ),
                )
                .raw_output("timeout");
            }
            Err(h) => return ToolOutcome::failed(&h),
        };

        // STAGE 3 — delivery. The verified SOURCE is written (not the script
        // itself, the same content): the file name is the name the user asked
        // for, the permissions are 0600 (the document layer's rule — privacy on device starts with the file permission).
        let body = safe_stem(file_name);
        let target = match empty_target(sandbox, &body, interpreter.extension) {
            Ok(h) => h,
            Err(h) => return ToolOutcome::failed(&h),
        };
        if let Err(h) = std::fs::write(&target, code.as_bytes()) {
            return ToolOutcome::failed(&ToolError::Io(h));
        }
        // PLATFORM RECORD — THE 0600 STAMP EXISTS ONLY ON UNIX (the same
        // as `create_document::finish_up` and `tacet_memory::store::narrow_permissions`
        // rationale as written at the top of the document layer): on Windows
        // `set_permissions` only flips the read-only flag, it does not restrict
        // access; faking it would produce the illusion of "stamped" without
        // providing privacy. Touching the ACL on Windows would need a new
        // dependency; the gap is left DELIBERATELY and IS NOT MEASURED ON THIS MACHINE.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).ok();
        }

        let name = target.file_name().map(|a| a.to_string_lossy().into_owned()).unwrap_or(body);
        let short = truncate(&output, MODEL_OUTPUT_CAP);
        let output_note = if output.trim().is_empty() {
            "(the run printed nothing — mention the file, do not invent output)".to_string()
        } else if short.len() < output.len() {
            format!("output:\n{short}\n(output truncated)")
        } else {
            format!("output:\n{short}")
        };
        ToolOutcome::written(
            format!("Code verified · {name} · {ms} ms"),
            format!(
                "ok: {name} written to the working folder and verified by running it ({ms} ms).\n\
                 {output_note}\n\
                 Tell the user the file name and what it does; show the demo output if any."
            ),
        )
        .raw_output(output)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_code::RunCodeTool;
    use serde_json::json;
    use tacet_core::{InMemoryDataStore, SilentReporter};

    fn hold<F: std::future::Future>(gelecek: F) -> F::Output {
        use std::pin::pin;
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        static VTABLE: RawWakerVTable =
            RawWakerVTable::new(|_| RawWaker::new(std::ptr::null(), &VTABLE), |_| {}, |_| {}, |_| {});
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
        let mut cx = Context::from_waker(&waker);
        let mut gelecek = pin!(gelecek);
        loop {
            if let Poll::Ready(output) = gelecek.as_mut().poll(&mut cx) {
                return output;
            }
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "tacet-writecode-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).expect("temp dir");
        path.canonicalize().expect("resolved path")
    }

    fn context(root: &Path) -> ToolContext {
        ToolContext::new(
            Arc::new(InMemoryDataStore::new()),
            root,
            Arc::new(SilentReporter),
        )
    }

    /// If there is no discovery (CI/Linux) the real-process tests are skipped — not silently.
    fn tool_or_skip() -> Option<WriteCodeTool> {
        match RunCodeTool::discover() {
            Some(a) => Some(WriteCodeTool::new(
                a.shield().clone(),
                a.interpreters().to_vec(),
                a.turn_state(),
            )),
            None => {
                eprintln!(
                    "write_code was not discovered, the process tests were skipped: {}",
                    RunCodeTool::diagnose()
                );
                None
            }
        }
    }

    /// The example script AS AN ARRAY OF LINES — the same format as the
    /// production path. A multi-line one was chosen deliberately: a one-line
    /// example would never exercise the channel's real job (carrying an indented body without escapes).
    fn example_code(tool: &WriteCodeTool) -> (serde_json::Value, &'static str, &'static str) {
        match tool.interpreters[0].key {
            "python" => (
                json!(["def total(n):", "    return sum(range(1, n + 1))", "", "print(total(10))"]),
                "py",
                "def total(n):\n    return sum(range(1, n + 1))\n\nprint(total(10))",
            ),
            _ => (
                json!([
                    "function total(n) {",
                    "  let t = 0;",
                    "  for (let i = 1; i <= n; i++) t += i;",
                    "  return t;",
                    "}",
                    "console.log(total(10));"
                ]),
                "js",
                "function total(n) {\n  let t = 0;\n  for (let i = 1; i <= n; i++) t += i;\n  return t;\n}\nconsole.log(total(10));",
            ),
        }
    }

    // --- Pure tests ---

    /// 1) Name hygiene: path escape, hidden file, double extension, empty name.
    #[test]
    fn the_safe_stem_cuts_path_escapes_and_a_double_extension() {
        assert_eq!(safe_stem("asal_sayilar"), "asal_sayilar");
        assert_eq!(safe_stem("prime.py"), "prime");
        assert_eq!(safe_stem("Prime.PY"), "Prime");
        assert_eq!(safe_stem("script.js"), "script");
        assert_eq!(safe_stem("../../etc/passwd"), "passwd");
        assert_eq!(safe_stem("/tmp/escape"), "escape");
        assert_eq!(safe_stem("a\\b\\c"), "c");
        assert_eq!(safe_stem(".hidden"), "hidden");
        assert_eq!(safe_stem("prime numbers"), "prime_numbers");
        assert_eq!(safe_stem(""), "script");
        assert_eq!(safe_stem("...."), "script");
    }

    /// 1b) JOINING: the lines join with `\n`, indentation is preserved, a
    ///     non-text element is skipped. The channel's whole promise is in this function.
    #[test]
    fn the_lines_join_without_escapes() {
        let s = join_lines(&[
            json!("def f(n):"),
            json!("    return n + 1"),
            json!(""),
            json!("print(f(1))"),
        ]);
        assert_eq!(s, "def f(n):\n    return n + 1\n\nprint(f(1))");
        // The indentation stands as real spaces; there is no escape sequence anywhere.
        assert!(s.contains("\n    return"));
        assert!(!s.contains("\\n"), "an escape sequence was produced");

        // A non-text element is SKIPPED (it can arrive through a grammar-less path).
        assert_eq!(join_lines(&[json!("a"), json!(7), json!("b")]), "a\nb");
        assert_eq!(join_lines(&[]), "");
        // If an element CONTAINS a real line break it is left alone.
        assert_eq!(join_lines(&[json!("a\nb"), json!("c")]), "a\nb\nc");
    }

    /// 2) The language guess from the extension.
    #[test]
    fn language_from_extension_senses_correctly() {
        assert_eq!(language_from_extension("a.py"), Some("python"));
        assert_eq!(language_from_extension("a.JS"), Some("js"));
        assert_eq!(language_from_extension("a.mjs"), Some("js"));
        assert_eq!(language_from_extension("a.txt"), None);
        assert_eq!(language_from_extension("script"), None);
    }

    // --- Tests that run a real process ---

    /// 3) THE HAPPY PATH: valid code is verified, the file is written into the
    ///    working directory, the state is Written (the world changed -> retry
    ///    forbidden), and the output goes back to the model as evidence.
    #[test]
    fn valid_code_is_validated_and_the_file_is_delivered() {
        let Some(tool) = tool_or_skip() else { return };
        tool.state.new_turn();
        let root = temp_dir("happy");
        let mut ctx = context(&root);
        let (lines, extension, joined) = example_code(&tool);

        let outcome =
            hold(tool.run(json!({"file_name": "total", "lines": lines}), &mut ctx));
        assert_eq!(outcome.state, ToolState::Written, "{}", outcome.to_model);
        assert!(outcome.to_model.contains("55"), "no evidence: {}", outcome.to_model);
        assert!(outcome.chip_text.contains("Code verified"), "{}", outcome.chip_text);

        let target = root.join(format!("total.{extension}"));
        assert!(target.is_file(), "the file was not delivered: {}", target.display());
        let content = std::fs::read_to_string(&target).expect("must be readable");
        assert_eq!(content, joined, "the lines must be joined with \\n and delivered");
        assert!(!content.contains("\\n"), "an escape sequence leaked into the raw text");
        assert!(ctx.session_tainted(), "an external tool ran, the session must be tainted");
        std::fs::remove_dir_all(&root).ok();
    }

    /// 4) A SYNTAX ERROR: the file is NEVER CREATED, the error is named "syntax".
    #[test]
    fn on_a_syntax_error_the_file_is_not_delivered() {
        let Some(tool) = tool_or_skip() else { return };
        tool.state.new_turn();
        let root = temp_dir("syntax");
        let mut ctx = context(&root);
        let lines = match tool.interpreters[0].key {
            "python" => json!(["def f(:", "  pass"]),
            _ => json!(["function f( {", "console.log(1)"]),
        };

        let outcome =
            hold(tool.run(json!({"file_name": "broken", "lines": lines}), &mut ctx));
        assert!(matches!(outcome.state, ToolState::Failed(_)), "{}", outcome.chip_text);
        assert!(outcome.to_model.contains("error (syntax)"), "{}", outcome.to_model);
        let remainder: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(|g| g.ok())
            .filter(|g| g.file_name().to_string_lossy().starts_with("broken"))
            .collect();
        assert!(remainder.is_empty(), "broken code was delivered: {remainder:?}");
        std::fs::remove_dir_all(&root).ok();
    }

    /// 5) A RUNTIME ERROR: the syntax passes, the run fails -> the file is again NOT CREATED.
    #[test]
    fn on_a_runtime_error_the_file_is_not_delivered() {
        let Some(tool) = tool_or_skip() else { return };
        tool.state.new_turn();
        let root = temp_dir("run");
        let mut ctx = context(&root);
        let lines = match tool.interpreters[0].key {
            "python" => json!(["raise RuntimeError('on purpose')"]),
            _ => json!(["throw new Error('on purpose')"]),
        };

        let outcome =
            hold(tool.run(json!({"file_name": "crashing", "lines": lines}), &mut ctx));
        assert!(matches!(outcome.state, ToolState::Failed(_)), "{}", outcome.chip_text);
        assert!(outcome.to_model.contains("error (runtime)"), "{}", outcome.to_model);
        assert!(!root.join("crashing.py").exists() && !root.join("crashing.js").exists());
        std::fs::remove_dir_all(&root).ok();
    }

    /// 6) A NAME COLLISION: an existing file IS NOT OVERWRITTEN, a `-2` suffix is produced.
    #[test]
    fn on_a_name_collision_nothing_is_overwritten() {
        let Some(tool) = tool_or_skip() else { return };
        let root = temp_dir("collision");
        let (lines, extension, _) = example_code(&tool);
        let old = root.join(format!("total.{extension}"));
        std::fs::write(&old, "THE USER'S FILE").unwrap();

        tool.state.new_turn();
        let mut ctx = context(&root);
        let outcome =
            hold(tool.run(json!({"file_name": "total", "lines": lines}), &mut ctx));
        assert_eq!(outcome.state, ToolState::Written, "{}", outcome.to_model);

        assert_eq!(
            std::fs::read_to_string(&old).unwrap(),
            "THE USER'S FILE",
            "the user's file was overwritten"
        );
        assert!(root.join(format!("total-2.{extension}")).is_file(), "the suffixed name was not produced");
        assert!(outcome.chip_text.contains(&format!("total-2.{extension}")), "{}", outcome.chip_text);
        std::fs::remove_dir_all(&root).ok();
    }

    /// 7) THE SHARED BUDGET: write_code + run_code consume the same counter;
    ///    whichever tool the third run reaches, it does not see the engine.
    #[test]
    fn the_attempt_budget_is_shared_with_run_code() {
        let Some(run_tool) = RunCodeTool::discover() else {
            eprintln!("no discovery, skipped");
            return;
        };
        let tool = WriteCodeTool::new(
            run_tool.shield().clone(),
            run_tool.interpreters().to_vec(),
            run_tool.turn_state(),
        );
        tool.state.new_turn();
        let root = temp_dir("budget");
        let mut ctx = context(&root);
        let (lines, _, _) = example_code(&tool);

        let first =
            hold(tool.run(json!({"file_name": "a", "lines": lines.clone()}), &mut ctx));
        assert_eq!(first.state, ToolState::Written);
        let exit_code = match run_tool.interpreters()[0].key {
            "python" => "print(2)",
            _ => "console.log(2)",
        };
        let second_pass = hold(run_tool.run(json!({"code": exit_code}), &mut ctx));
        assert_eq!(second_pass.state, ToolState::Read, "{}", second_pass.to_model);

        let third =
            hold(tool.run(json!({"file_name": "b", "lines": lines}), &mut ctx));
        assert!(third.to_model.starts_with("error_final"), "{}", third.to_model);
        assert!(!root.join("b.py").exists() && !root.join("b.js").exists());
        std::fs::remove_dir_all(&root).ok();
    }

    /// 8) THE VERIFICATION BY-PRODUCTS do not leak into the user's directory:
    ///    the run happens in the hidden `code/` folder, and only the delivery stands in the working directory.
    #[test]
    fn the_validation_by_products_do_not_leak_into_the_working_directory() {
        let Some(tool) = tool_or_skip() else { return };
        tool.state.new_turn();
        let root = temp_dir("leak");
        let mut ctx = context(&root);
        // The script writes a file where it sits — legitimate work, but NOT A DELIVERY.
        let lines = match tool.interpreters[0].key {
            "python" => json!(["open('by_product.txt','w').write('x')", "print('ok')"]),
            _ => json!([
                "require('fs').writeFileSync('by_product.txt','x');",
                "console.log('ok')"
            ]),
        };
        let outcome =
            hold(tool.run(json!({"file_name": "writing", "lines": lines}), &mut ctx));
        assert_eq!(outcome.state, ToolState::Written, "{}", outcome.to_model);
        assert!(
            !root.join("by_product.txt").exists(),
            "the by-product leaked into the working directory"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// 9) THE SCHEMA CONTRACT: two required fields; `language` only with multiple interpreters.
    #[test]
    fn the_schema_contract_is_binding() {
        let Some(tool) = tool_or_skip() else { return };
        let schema = tool.schema();
        let js = schema.json_schema();
        assert_eq!(js["required"], json!(["file_name", "lines"]));
        assert_eq!(js["properties"]["lines"]["type"], json!("array"));
        assert_eq!(js["properties"]["lines"]["items"]["type"], json!("string"));
        let has_language = js["properties"].get("language").is_some();
        assert_eq!(has_language, tool.interpreters.len() > 1);
        assert!(schema.validate(&json!({"file_name": "a", "lines": ["x"]})).is_ok());
        assert!(schema.validate(&json!({"lines": ["x"]})).is_err());
        assert!(schema.validate(&json!({"file_name": "a"})).is_err());
        // An empty array and text-instead-of-array: both FAIL in the schema.
        assert!(schema.validate(&json!({"file_name": "a", "lines": []})).is_err());
        assert!(schema.validate(&json!({"file_name": "a", "lines": "def f():"})).is_err());
    }

    /// 10) THE EXTENSION PICKS THE LANGUAGE: while the source is silent, a ".py" name routes to python.
    #[test]
    fn the_extension_picks_the_language_when_the_source_is_silent() {
        let Some(tool) = tool_or_skip() else { return };
        if tool.interpreters.len() < 2 {
            return;
        }
        // The source is valid in both languages — the guess stays silent, the extension speaks.
        assert_eq!(tool.resolve_interpreter("script.py", None, "1 + 2").key, "python");
        assert_eq!(tool.resolve_interpreter("script.js", None, "1 + 2").key, "js");
        // If the source SPEAKS it overrides the extension (evidence > declaration).
        assert_eq!(tool.resolve_interpreter("script.js", None, "print(len([1]))").key, "python");
    }
}
