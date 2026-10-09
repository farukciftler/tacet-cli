//! `write_code` — code PRODUCTION as a pipeline: write -> verify -> run ->
//! deliver.
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
//! THE VERIFICATION FOLDER IS HIDDEN AND FRESH (`.tacet-sandbox-<nonce>/`,
//! made for the call and deleted after it): the by-products of the execution
//! step (files the script opens, `__pycache__`) must not leak into the user's
//! directory. The ONLY thing delivered is the verified source file.
//!
//! NOR IS EVERY NAME DELIVERED. Some file names are executed by other programs
//! without the user ever running them — `conftest.py` by pytest, `*.config.js`
//! by eslint/jest/vite, `setup.py` by pip — OUTSIDE the sandbox and with the
//! user's privileges. Verification proves the script runs; it does not make it
//! safe to leave where a tool will run it unasked (audit M3). See
//! `auto_executed_name`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tacet_kernel::{
    ArgSchema, Field, Tool, ToolContext, ToolError, ToolFuture, ToolOutcome, ToolResult, ToolState,
    TraceUpdate, boxed,
};

use crate::run_code::{
    CODE_CAP, CodeOutcome, CodeState, Interpreter, MAX_ATTEMPTS, MODEL_OUTPUT_CAP, NetworkShield,
    error_text, language_from_source, looks_like_python, run_program, truncate,
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
        WriteCodeTool {
            shield,
            interpreters,
            state,
            schema_cache: Mutex::new(None),
        }
    }

    /// The target language. The priority is the same as the measurement in
    /// `run_code`: THE SOURCE is evidence and overrides the declaration; if the
    /// source is silent the file EXTENSION speaks (the model writing ".py" is a
    /// declaration too, but it comes after the source); failing that the `language`
    /// field; failing everything, the first interpreter in preference order.
    fn resolve_interpreter(
        &self,
        file_name: &str,
        requested: Option<&str>,
        code: &str,
    ) -> &Interpreter {
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
    let last = file_name.rsplit(['/', '\\']).next().unwrap_or("").trim();
    let mut body: String = last
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    // Leading dots make it a hidden file; a "delivery" that does not show up in
    // the user's `ls` is not a delivery.
    body = body
        .trim_start_matches('.')
        .trim_end_matches('.')
        .to_string();
    // Known code extensions are stripped from the stem (case insensitive) — the
    // extension is the tool's choice, not the model's.
    let small = body.to_ascii_lowercase();
    for ext in [".py", ".js", ".mjs"] {
        if small.ends_with(ext) {
            body.truncate(body.len() - ext.len());
            break;
        }
    }
    if body.is_empty() {
        "script".to_string()
    } else {
        body
    }
}

/// Stems that another program EXECUTES on its own when it finds them in a
/// directory, compared case-insensitively (APFS and NTFS are case-insensitive
/// by default, so `ConfTest.py` IS `conftest.py` to pytest there).
///
/// The audit's list (M3) plus the neighbours that follow the same rule:
/// pytest's `conftest`; `setup` (pip/setuptools); `sitecustomize` /
/// `usercustomize` (imported by every python start that has the directory on
/// its path); `__init__` / `__main__` (they turn the folder into something
/// python imports or runs); the task runners nox, fabric, invoke (`tasks`),
/// doit, paver, gulp, grunt and jake; tox 4's `toxfile` (imported from the
/// tox root as an inline plugin on a bare `tox`) and danger-js's `dangerfile`
/// (run by default) — both found missing by the review of 10 Oct 2026, the
/// first in noxfile's own family; gatsby's four lifecycle files. Test files
/// are a family, not a list — see `test_runner_collects`. `makefile`, `envrc`,
/// `package` and `pyproject` cannot be produced with a `.py`/`.js` extension in
/// a way that make, direnv, npm or pip would read, and are refused anyway: the
/// name alone is what a user's eye trusts in a listing.
const AUTO_EXECUTED_STEMS: &[(&str, &str)] = &[
    ("conftest", "pytest"),
    ("setup", "pip and setuptools"),
    ("sitecustomize", "every python start"),
    ("usercustomize", "every python start"),
    ("__init__", "python's import system"),
    ("__main__", "python's import system"),
    ("noxfile", "nox"),
    ("toxfile", "tox"),
    ("dangerfile", "danger"),
    ("fabfile", "fabric"),
    ("tasks", "invoke"),
    ("dodo", "doit"),
    ("pavement", "paver"),
    ("gulpfile", "gulp"),
    ("gruntfile", "grunt"),
    ("jakefile", "jake"),
    ("gatsby-config", "gatsby"),
    ("gatsby-node", "gatsby"),
    ("gatsby-browser", "gatsby"),
    ("gatsby-ssr", "gatsby"),
    ("makefile", "make"),
    ("gnumakefile", "make"),
    ("envrc", "direnv"),
    ("package", "npm"),
    ("pyproject", "pip"),
];

/// Why `stem` must not be delivered, or `None` if it may.
///
/// A DOT LEFT IN THE STEM IS REFUSED OUTRIGHT, not checked against a list. The
/// family it closes is open-ended — `eslint.config`, `jest.config`,
/// `vite.config`, `karma.conf`, `jest.setup`, `*.test`, `*.spec`, and the next
/// tool's tomorrow — and every member of it is a dotted stem. `safe_stem` has
/// already removed the one dot a legitimate name carries (the extension the
/// model added itself), so what is left is a second one.
///
/// `extension` is the one the tool will deliver under (`interpreter.extension`):
/// a standard-library name is only refused for `py`, see `PYTHON_STDLIB`.
fn auto_executed_name(stem: &str, extension: &str) -> Option<String> {
    if stem.contains('.') {
        return Some(format!(
            "'{stem}' contains a dot: names such as x.config.js, x.conf.js or x.test.js are \
             loaded automatically by build and test tools"
        ));
    }
    let lower = fold_case(stem);
    if let Some(runner) = test_runner_collects(&lower) {
        return Some(format!(
            "'{stem}' is a test file name: {runner} imports and runs every file named like it"
        ));
    }
    if let Some((_, runner)) = AUTO_EXECUTED_STEMS.iter().find(|(name, _)| *name == lower) {
        return Some(format!(
            "'{stem}' is a file name that {runner} runs automatically"
        ));
    }
    if extension == "py"
        && PYTHON_STDLIB
            .split_ascii_whitespace()
            .any(|module| fold_case(module) == lower)
    {
        return Some(format!(
            "'{stem}' is the name of a python standard-library module: every python script \
             in the folder that runs `import {stem}` imports and runs this file instead"
        ));
    }
    None
}

/// The top-level names of python's standard library, which a `<name>.py` in
/// the working folder SHADOWS: python puts the script's own directory first
/// on `sys.path`, so the user's next `python3 their_script.py` there that says
/// `import json` runs the delivered `json.py` — outside the sandbox, with the
/// user's privileges. The same class as `sitecustomize`, and wider: nothing
/// has to be installed, only imported. MEASURED with python 3.12 on macOS: a
/// folder holding `json.py` (printing a marker) and `other.py` (`import json`)
/// printed the marker when `other.py` ran. `JSON.py` and `\u{17F}tring.py` did
/// NOT shadow (importlib matches the directory listing case-sensitively unless
/// `PYTHONCASEOK` is set), and neither did `sys.py` (a built-in module is found
/// before the path is searched) — the folded comparison over-refuses those,
/// which costs the model one rename and covers a `PYTHONCASEOK` user.
///
/// A SNAPSHOT, NOT A DEPENDENCY: `sys.stdlib_module_names` of python 3.12,
/// plus the five modules 3.12 removed (`distutils`, `imp`, `asynchat`,
/// `asyncore`, `smtpd` — an older python on the user's PATH still imports
/// them) and the two top-level packages 3.14 added (`annotationlib`,
/// `compression`). A module a later python adds is a gap until it is added
/// here. `.js` names are not checked: node resolves a bare `require('json')`
/// to `node_modules` or a built-in, never to `./json.js`.
const PYTHON_STDLIB: &str = "\
    __future__ _abc _aix_support _ast _asyncio _bisect _blake2 _bz2 _codecs _codecs_cn \
    _codecs_hk _codecs_iso2022 _codecs_jp _codecs_kr _codecs_tw _collections \
    _collections_abc _compat_pickle _compression _contextvars _crypt _csv _ctypes _curses \
    _curses_panel _datetime _dbm _decimal _elementtree _frozen_importlib \
    _frozen_importlib_external _functools _gdbm _hashlib _heapq _imp _io _json _locale \
    _lsprof _lzma _markupbase _md5 _msi _multibytecodec _multiprocessing _opcode _operator \
    _osx_support _overlapped _pickle _posixshmem _posixsubprocess _py_abc _pydatetime \
    _pydecimal _pyio _pylong _queue _random _scproxy _sha1 _sha2 _sha3 _signal \
    _sitebuiltins _socket _sqlite3 _sre _ssl _stat _statistics _string _strptime _struct \
    _symtable _thread _threading_local _tkinter _tokenize _tracemalloc _typing _uuid \
    _warnings _weakref _weakrefset _winapi _wmi _zoneinfo abc aifc annotationlib \
    antigravity argparse array ast asynchat asyncio asyncore atexit audioop base64 bdb \
    binascii bisect builtins bz2 cProfile calendar cgi cgitb chunk cmath cmd code codecs \
    codeop collections colorsys compileall compression concurrent configparser contextlib \
    contextvars copy copyreg crypt csv ctypes curses dataclasses datetime dbm decimal \
    difflib dis distutils doctest email encodings ensurepip enum errno faulthandler fcntl \
    filecmp fileinput fnmatch fractions ftplib functools gc genericpath getopt getpass \
    gettext glob graphlib grp gzip hashlib heapq hmac html http idlelib imaplib imghdr imp \
    importlib inspect io ipaddress itertools json keyword lib2to3 linecache locale logging \
    lzma mailbox mailcap marshal math mimetypes mmap modulefinder msilib msvcrt \
    multiprocessing netrc nis nntplib nt ntpath nturl2path numbers opcode operator optparse \
    os ossaudiodev pathlib pdb pickle pickletools pipes pkgutil platform plistlib poplib \
    posix posixpath pprint profile pstats pty pwd py_compile pyclbr pydoc pydoc_data \
    pyexpat queue quopri random re readline reprlib resource rlcompleter runpy sched \
    secrets select selectors shelve shlex shutil signal site smtpd smtplib sndhdr socket \
    socketserver spwd sqlite3 sre_compile sre_constants sre_parse ssl stat statistics \
    string stringprep struct subprocess sunau symtable sys sysconfig syslog tabnanny \
    tarfile telnetlib tempfile termios textwrap this threading time timeit tkinter token \
    tokenize tomllib trace traceback tracemalloc tty turtle turtledemo types typing \
    unicodedata unittest urllib uu uuid venv warnings wave weakref webbrowser winreg \
    winsound wsgiref xdrlib xml xmlrpc zipapp zipfile zipimport zlib zoneinfo";

/// The name a case-insensitive file system would LOOK UP for `stem`.
///
/// `to_lowercase()` WAS NOT ENOUGH, MEASURED on this Mac's APFS: a file created
/// as `confte\u{17F}t.py` (long s), `confte\u{FB06}.py` (st ligature),
/// `\u{17F}etup.py`, `ma\u{212A}efile.py` (Kelvin sign), `pa\u{DF}e.py` and
/// `con\u{FB01}g.py` each made `isfile("<ascii name>.py")` true, and the
/// review measured `python3 setup.py` executing a folder's `\u{17F}etup.py`
/// and pytest's `spec_from_file_location("conftest", "conftest.py")` executing
/// `confte\u{17F}t.py`. APFS compares with Unicode full case folding, and the
/// lower-case of `\u{17F}` is `\u{17F}` itself, so the list above never matched.
///
/// LOWER, UPPER, LOWER reproduces full case folding for every alphanumeric
/// code point that folds into ASCII (checked over the whole range against
/// Python's `str.casefold`: ß, ẞ, ſ, the Kelvin sign and the seven Latin
/// ligatures) without a Unicode-table dependency. It folds ONE letter more than
/// APFS does — the Turkish dotless ı becomes `i`, which APFS keeps distinct
/// (`ınit.py` did not answer for `init.py`) — and that only ever over-refuses a
/// name one letter away from a refused one.
///
/// WHY THE STEM KEEPS ITS NON-ASCII LETTERS rather than `safe_stem` turning
/// them into `_`: the delivered file is named for the user, and `asal_sayılar`
/// arriving as `asal_say_lar` would be the tool damaging a Turkish user's
/// ordinary name to close a hole that this comparison closes on its own.
fn fold_case(stem: &str) -> String {
    stem.to_lowercase().to_uppercase().to_lowercase()
}

/// The test runner that would collect — and so EXECUTE — a file with this
/// (lower-case, dot-free) stem when the user types its bare command in the
/// folder, or `None`.
///
/// A RECORDED DECISION, NOT AN ACCIDENT: test files are refused. The first
/// version of the M3 fix closed `conftest.py` and let `test_ideas.py` through
/// — and had a test asserting `test_ideas` was an ordinary name — although a
/// bare `pytest` imports every `test_*.py` and `*_test.py` in the folder, which
/// runs their top level outside the sandbox with the user's privileges: the
/// same path as `conftest.py`, by the very tool the audit named. The patterns
/// are each runner's DEFAULT collection glob, as documented:
///   * pytest `python_files`: `test_*.py`, `*_test.py`;
///   * `python -m unittest` discovery: `test*.py` — any stem STARTING with
///     `test`, which is why `testimonials` is refused too; that is the
///     runner's rule, not ours, and the model is told which runner and can
///     pick another name without losing an attempt;
///   * `node --test`: `test.js`, `test-*.js`, `*-test.js`, `*_test.js`
///     (and `*.test.js`, which the dotted-stem rule already refuses);
///   * jest `testMatch`: `?(*.)+(spec|test).js` — a bare `spec` or `test`.
///
/// What is NOT refused, and why: a name that merely contains the word
/// (`latest`, `contest`, `attestation`) matches none of these globs.
fn test_runner_collects(lower: &str) -> Option<&'static str> {
    if lower.starts_with("test_") || lower.ends_with("_test") {
        Some("pytest")
    } else if lower.starts_with("test") {
        Some("python -m unittest")
    } else if lower.ends_with("-test") {
        Some("node --test")
    } else if lower == "spec" {
        Some("jest")
    } else {
        None
    }
}

/// Produces a NON-COLLIDING target path in the working directory: `name.py`, `name-2.py`, ...
///
/// Overwriting an existing file would be wrong in both directions: the user's
/// file silently disappears, or the new delivery lands on top of the previous
/// one and "which version did I run" is left unanswered.
/// `symlink_metadata` AND NOT `exists()`: `exists()` follows links, so a DANGLING
/// link planted in the working folder (`prime_numbers.py -> ~/.zshrc`, target not
/// yet created) reports "free", the loop hands the link itself to `fs::write`,
/// and the verified script is delivered OUTSIDE the sandbox under the user's
/// full privileges. `symlink_metadata` sees the link itself.
fn empty_target(folder: &Path, body: &str, extension: &str) -> ToolResult<PathBuf> {
    let first = folder.join(format!("{body}.{extension}"));
    if first.symlink_metadata().is_err() {
        return Ok(first);
    }
    for n in 2..=MAX_NAME_ATTEMPTS {
        let candidate = folder.join(format!("{body}-{n}.{extension}"));
        if candidate.symlink_metadata().is_err() {
            return Ok(candidate);
        }
    }
    Err(ToolError::InvalidArgument(format!(
        "no free name found for {body}.{extension}"
    )))
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
        // The sandbox sentence says what the shield GUARANTEES on both
        // platforms — no network, writes confined — and no more: "NO access to
        // files outside the working folder" was false under bwrap, where the
        // rest of the system outside home is readable (audit L20), and implied
        // the script could see the working folder, which since M5 it cannot.
        // No "but": see the routing note on run_code's descriptions.
        "Writes a code file for the user, VERIFIES it and saves it into the working folder. \
         Call this when the user wants a script or program AS A FILE they can keep and run \
         later ('write me a python script that...', 'save the script to a file'). Pass \
         the code as `lines`: an array where each element is ONE line of code - the tool \
         joins them, so never write \\n yourself. Indentation is just leading spaces inside \
         the line. The code is first syntax-checked, then RUN in an isolated sandbox with NO \
         network that can write nowhere except its own temporary folder; the file is saved ONLY if \
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
        let schema = ArgSchema::object(fields).description("Write, verify and save a code file");
        *self.schema_cache.lock().expect("schema lock") = Some(schema.clone());
        schema
    }

    /// AN EXTERNAL TOOL: it runs model output (the verification step is a run)
    /// and leaves a file on disk.
    fn taints_session(&self) -> bool {
        true
    }

    fn run<'a>(&'a self, args: serde_json::Value, ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move {
            if let Err(h) = self.schema().validate(&args) {
                return ToolOutcome::failed(&h);
            }

            // A NAME THAT ANOTHER TOOL WOULD RUN UNASKED IS REFUSED BEFORE ANYTHING
            // ELSE (audit M3): the verified file would otherwise sit in the
            // user's directory waiting for pytest, eslint or pip to execute it
            // outside the sandbox. It is checked BEFORE the attempt counter: the
            // engine is never reached, so the refusal must not cost the model one
            // of its two real runs — it only has to pick another name.
            //
            // The extension is resolved the way `execute` will resolve it — the
            // source, then the name, then `language` — because whether a
            // standard-library name shadows anything depends on it being `.py`.
            if let Some(name) = args.get("file_name").and_then(|v| v.as_str())
                && let Some(why) = {
                    let code = args
                        .get("lines")
                        .and_then(|v| v.as_array())
                        .map(|s| join_lines(s))
                        .unwrap_or_default();
                    let language = args.get("language").and_then(|v| v.as_str());
                    let extension = self.resolve_interpreter(name, language, &code).extension;
                    auto_executed_name(&safe_stem(name), extension)
                }
            {
                return ToolOutcome::new(
                    "Unsafe file name",
                    ToolState::Failed("auto-executed file name".into()),
                    format!(
                        "error: {why}, outside the sandbox. Call write_code again with a plain \
                         descriptive file_name such as 'prime_numbers'."
                    ),
                );
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
    fn execute(
        &self,
        args: &serde_json::Value,
        ctx: &mut ToolContext,
        attempt: usize,
    ) -> ToolOutcome {
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
        let Some(file_name) = args
            .get("file_name")
            .and_then(|v| v.as_str())
            .filter(|a| !a.trim().is_empty())
        else {
            return ToolOutcome::failed(&ToolError::MissingField("file_name".into()));
        };

        let interpreter = self.resolve_interpreter(
            file_name,
            args.get("language").and_then(|v| v.as_str()),
            code,
        );

        // The sandbox root — the resolved-path requirement is the same as in run_code.
        let sandbox = match ctx
            .resolve_path(".")
            .and_then(|k| k.canonicalize().map_err(ToolError::Io))
        {
            Ok(k) => k,
            Err(h) => return ToolOutcome::failed(&h),
        };

        // Verification happens in a FRESH, HIDDEN folder made for this call and
        // deleted after it: by-products must not leak into the user's
        // directory. It used to be `<cwd>/code` — the user's own `code/`
        // directory if they had one, followed if it was a link (audit M5; the
        // full account is at `run_code::run`).
        let verification =
            match crate::sandbox_path::fresh_private_dir(&sandbox, &crate::run_code::nonce()) {
                Ok(v) => v,
                Err(h) => return ToolOutcome::failed(&h),
            };
        // THE NAME IS UNPREDICTABLE AND THE WRITE NEVER FOLLOWS A LINK.
        // `verify-{pid}-{code.len()}` was fully determined: the pid is fixed for
        // the session and the length is the model's to choose, so a first call
        // could plant a symlink at the SECOND call's exact name and the parent
        // process — unshielded, with the user's privileges — wrote straight
        // through it. Two attempts per turn is all that attack needs. See
        // `run_code::create_script` for the full account.
        let script = verification.join(format!(
            "verify-{}.{}",
            crate::run_code::nonce(),
            interpreter.extension
        ));
        let outcome = match crate::run_code::create_script(&script, code.as_bytes()) {
            Ok(()) => self.validate_and_deliver(
                code,
                file_name,
                interpreter,
                &sandbox,
                &script,
                last_attempt,
            ),
            Err(h) => ToolOutcome::failed(&ToolError::Io(h)),
        };
        // The verification folder is deleted ON EVERY PATH (the same rationale as run_code).
        std::fs::remove_dir_all(&verification).ok();
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
        match run_program(
            &self.shield,
            verification,
            &interpreter.path,
            &args,
            SYNTAX_CHECK_TIMEOUT,
        ) {
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
                    format!(
                        "error (syntax): the code does not parse — fix it, do not change \
                             the logic.\n{message}"
                    )
                };
                return ToolOutcome::new(
                    if last_attempt {
                        "Code could not be verified"
                    } else {
                        "Syntax error · retrying"
                    },
                    ToolState::Failed("syntax".into()),
                    error_text(&cause, last_attempt),
                )
                .raw_output(message);
            }
            Ok(CodeOutcome::Timeout) => {
                return ToolOutcome::new(
                    if last_attempt {
                        "Code could not be verified"
                    } else {
                        "Error · retrying"
                    },
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
                    if last_attempt {
                        "Code could not be verified"
                    } else {
                        "Runtime error · retrying"
                    },
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
                    if last_attempt {
                        "Code could not be verified"
                    } else {
                        "Error · retrying"
                    },
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

        let name = target
            .file_name()
            .map(|a| a.to_string_lossy().into_owned())
            .unwrap_or(body);
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
    use tacet_kernel::{InMemoryDataStore, SilentReporter};

    fn hold<F: std::future::Future>(gelecek: F) -> F::Output {
        use std::pin::pin;
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        static VTABLE: RawWakerVTable = RawWakerVTable::new(
            |_| RawWaker::new(std::ptr::null(), &VTABLE),
            |_| {},
            |_| {},
            |_| {},
        );
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

    /// If there is no discovery the real-process tests are skipped — not silently.
    ///
    /// AND NOT AT ALL under `TACET_SANDBOX_MUST_RUN`: there the skip becomes a
    /// panic carrying `diagnose()`. The gate is defined once, in
    /// `run_code::sandbox_must_run`, because the tests it silences are skipped
    /// by the same absent shield and must be re-enabled by the same switch:
    /// 8 here and 16 in run_code.rs on Linux. Those two figures are DERIVED from
    /// the source and checked against this sentence by
    /// `the_skip_count_the_documents_quote_is_the_count_in_the_source`; written
    /// out by hand they were "eight and sixteen", and sixteen was wrong. The syntax-check path
    /// these tests drive
    /// (`node --check`,
    /// `python3 -m py_compile` under the shield) has never run on Linux
    /// anywhere; the CI job that sets the variable is its first execution.
    fn tool_or_skip() -> Option<WriteCodeTool> {
        match RunCodeTool::discover() {
            Some(a) => Some(WriteCodeTool::new(
                a.shield().clone(),
                a.interpreters().to_vec(),
                a.turn_state(),
            )),
            None => {
                let why = RunCodeTool::diagnose();
                assert!(
                    !crate::run_code::sandbox_must_run(),
                    "{} is set, so this test may not skip — \
                     the sandbox was expected to work here: {why}",
                    crate::run_code::SANDBOX_MUST_RUN
                );
                eprintln!("write_code was not discovered, the process tests were skipped: {why}");
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
                json!([
                    "def total(n):",
                    "    return sum(range(1, n + 1))",
                    "",
                    "print(total(10))"
                ]),
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
        // Non-ASCII letters stay: the user's own name is not mangled (the
        // auto-executed check folds case instead, see `fold_case`).
        assert_eq!(safe_stem("asal_say\u{0131}lar"), "asal_say\u{0131}lar");
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

    /// 1c) THE AUTO-EXECUTED NAMES (audit M3): each one is refused, in any
    ///     case, with or without the extension the model may add, and an
    ///     ordinary name — including one that merely CONTAINS a listed word —
    ///     still passes.
    #[test]
    fn names_another_tool_would_run_are_refused() {
        for name in [
            "conftest",
            "conftest.py",
            "ConfTest",
            "setup.py",
            "sitecustomize",
            "__init__",
            "__main__.py",
            "noxfile",
            "toxfile",
            "toxfile.py",
            "ToxFile",
            "dangerfile",
            "dangerfile.js",
            "Dangerfile",
            "gulpfile.js",
            "Gruntfile",
            "gatsby-config",
            "Makefile",
            ".envrc",
            "package",
            "pyproject",
            "eslint.config",
            "eslint.config.js",
            "jest.config.mjs",
            "vite.config",
            "karma.conf",
            "jest.setup",
            "utils.test",
            // Letters APFS case-folds INTO ASCII (see `fold_case`): each of
            // these answers a lookup for the plain name on this Mac.
            "confte\u{017F}t",
            "confte\u{FB06}",
            "confte\u{017F}t.py",
            "\u{017F}etup",
            "\u{017F}etup.py",
            "ma\u{212A}efile",
            "te\u{FB06}_ideas",
            // Invoke's task file, in the fabfile/noxfile family.
            "tasks",
            "Tasks.py",
            // Test files: pytest's two globs, unittest's `test*`, node --test's
            // and jest's bare names (see `test_runner_collects`).
            "test_ideas",
            "Test_Ideas.py",
            "parser_test",
            "PARSER_TEST.py",
            "testimonials",
            "test",
            "test-utils",
            "utils-test.js",
            "spec",
            // Standard-library names: the folder's next `import json` runs
            // the delivered file instead (see `PYTHON_STDLIB`).
            "json",
            "json.py",
            "random",
            "typing",
            "csv",
            "logging",
            "string",
            "datetime",
            "turtle",
            "cProfile",
            "distutils",
            "annotationlib",
        ] {
            assert!(
                auto_executed_name(&safe_stem(name), "py").is_some(),
                "{name:?} would be delivered"
            );
        }
        for name in [
            "prime_numbers",
            "prime.py",
            "my_setup_notes",
            "configure_grid",
            "package_size",
            // They contain the word, and match no runner's glob.
            "latest",
            "contest",
            "attestation",
            "task_list",
            "my_tests_notes",
            "tox_results",
            "danger_zone",
            // A Turkish name keeps its letters and is still delivered.
            "asal_say\u{0131}lar",
            "\u{00E7}arp\u{0131}m_tablosu",
            // A standard-library name with something around it imports nothing.
            "json_report",
            "my_random",
            "typing_speed",
        ] {
            assert_eq!(
                auto_executed_name(&safe_stem(name), "py"),
                None,
                "{name:?} is an ordinary name"
            );
        }
        // A standard-library name shadows nothing as a `.js` file: node never
        // resolves a bare `require('json')` to `./json.js`.
        for name in ["json", "random", "string"] {
            assert_eq!(
                auto_executed_name(&safe_stem(name), "js"),
                None,
                "{name:?}.js shadows no python module"
            );
        }
    }

    /// The shield-free stand-in: `/bin/echo` "verifies" every script by
    /// printing the command line it was given, so these tests reach the
    /// delivery path — and can read where the verification ran — on any unix,
    /// with no sandbox installed.
    #[cfg(unix)]
    fn echo_tool() -> WriteCodeTool {
        WriteCodeTool::new(
            NetworkShield::SandboxExec {
                tool: PathBuf::from("/bin/echo"),
            },
            vec![Interpreter {
                key: "python",
                path: PathBuf::from("/usr/bin/true"),
                extension: "py",
            }],
            Arc::new(CodeState::new()),
        )
    }

    /// 1d) THE REFUSAL HAPPENS BEFORE ANYTHING RUNS AND COSTS NO ATTEMPT (M3).
    ///     No file of that name appears, nothing is executed, and the model
    ///     still has both of its runs for the corrected name.
    #[cfg(unix)]
    #[test]
    fn an_auto_executed_name_is_refused_before_anything_runs() {
        let tool = echo_tool();
        let root = temp_dir("auto-exec");
        let mut ctx = context(&root);
        for name in [
            "conftest",
            "eslint.config",
            "setup",
            "test_ideas",
            "tasks",
            "json",
        ] {
            let outcome =
                hold(tool.run(json!({"file_name": name, "lines": ["print(1)"]}), &mut ctx));
            assert!(
                matches!(outcome.state, ToolState::Failed(_)),
                "{name}: {}",
                outcome.to_model
            );
            assert!(
                outcome.to_model.contains("runs automatically")
                    || outcome.to_model.contains("loaded automatically")
                    || outcome.to_model.contains("imports and runs"),
                "{}",
                outcome.to_model
            );
        }
        let left: Vec<_> = std::fs::read_dir(&root)
            .expect("list")
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert!(left.is_empty(), "something was written: {left:?}");
        assert_eq!(
            tool.state.next_attempt(),
            1,
            "a refused name consumed one of the two real runs"
        );
        assert!(
            !ctx.session_tainted(),
            "nothing ran, nothing may be tainted"
        );

        // The corrected name goes through.
        tool.state.new_turn();
        let ok = hold(tool.run(
            json!({"file_name": "prime_numbers", "lines": ["print(1)"]}),
            &mut ctx,
        ));
        assert_eq!(ok.state, ToolState::Written, "{}", ok.to_model);
        assert!(root.join("prime_numbers.py").is_file());
        std::fs::remove_dir_all(&root).ok();
    }

    /// 1d') THE STANDARD-LIBRARY REFUSAL FOLLOWS THE DELIVERED EXTENSION. The
    ///      check runs before `execute` picks the interpreter, so the call site
    ///      resolves it itself; were it to assume `.py`, a JavaScript `json.js`
    ///      — which shadows nothing — would be refused for a python reason.
    #[cfg(unix)]
    #[test]
    fn a_stdlib_name_is_refused_only_for_python() {
        let tool = WriteCodeTool::new(
            NetworkShield::SandboxExec {
                tool: PathBuf::from("/bin/echo"),
            },
            vec![Interpreter {
                key: "js",
                path: PathBuf::from("/usr/bin/true"),
                extension: "js",
            }],
            Arc::new(CodeState::new()),
        );
        let root = temp_dir("stdlib-js");
        let mut ctx = context(&root);
        let outcome = hold(tool.run(
            json!({"file_name": "json", "lines": ["console.log(1)"]}),
            &mut ctx,
        ));
        assert_eq!(outcome.state, ToolState::Written, "{}", outcome.to_model);
        assert!(root.join("json.js").is_file(), "json.js was not delivered");
        std::fs::remove_dir_all(&root).ok();
    }

    /// 1e) VERIFICATION RUNS IN A FRESH FOLDER, NOT IN THE USER'S `code/` (M5).
    ///
    /// It used to be `<cwd>/code`: a user's real `code/` directory was the
    /// verification sandbox, and a `code` that was a link sent the script into
    /// the link's target. Here `code` IS such a link, to a directory outside.
    #[cfg(unix)]
    #[test]
    fn verification_runs_in_a_fresh_folder_not_in_code() {
        let tool = echo_tool();
        let root = temp_dir("fresh-verify");
        let outside = temp_dir("fresh-verify-target");
        std::os::unix::fs::symlink(&outside, root.join("code")).expect("link");
        let mut ctx = context(&root);

        let outcome = hold(tool.run(
            json!({"file_name": "total", "lines": ["print(55)"]}),
            &mut ctx,
        ));
        assert_eq!(outcome.state, ToolState::Written, "{}", outcome.to_model);
        let echoed = outcome.raw_output.clone().unwrap_or_default();
        let sandbox = echoed
            .split_whitespace()
            .find_map(|w| w.strip_prefix("SBOX="))
            .map(PathBuf::from)
            .unwrap_or_else(|| panic!("no SBOX= in {echoed}"));
        assert_ne!(sandbox, root.join("code"), "verification ran in code/");
        assert_eq!(sandbox.parent(), Some(root.as_path()));
        assert!(
            sandbox
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(crate::sandbox_path::SANDBOX_PREFIX)),
            "{}",
            sandbox.display()
        );
        assert!(!sandbox.exists(), "the verification folder was left behind");
        assert_eq!(
            std::fs::read_dir(&outside).expect("list").count(),
            0,
            "something was written through the code/ link"
        );
        assert!(root.join("total.py").is_file(), "the delivery is missing");
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
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

        let outcome = hold(tool.run(json!({"file_name": "total", "lines": lines}), &mut ctx));
        assert_eq!(outcome.state, ToolState::Written, "{}", outcome.to_model);
        assert!(
            outcome.to_model.contains("55"),
            "no evidence: {}",
            outcome.to_model
        );
        assert!(
            outcome.chip_text.contains("Code verified"),
            "{}",
            outcome.chip_text
        );

        let target = root.join(format!("total.{extension}"));
        assert!(
            target.is_file(),
            "the file was not delivered: {}",
            target.display()
        );
        let content = std::fs::read_to_string(&target).expect("must be readable");
        assert_eq!(
            content, joined,
            "the lines must be joined with \\n and delivered"
        );
        assert!(
            !content.contains("\\n"),
            "an escape sequence leaked into the raw text"
        );
        assert!(
            ctx.session_tainted(),
            "an external tool ran, the session must be tainted"
        );
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

        let outcome = hold(tool.run(json!({"file_name": "broken", "lines": lines}), &mut ctx));
        assert!(
            matches!(outcome.state, ToolState::Failed(_)),
            "{}",
            outcome.chip_text
        );
        assert!(
            outcome.to_model.contains("error (syntax)"),
            "{}",
            outcome.to_model
        );
        let remainder: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(|g| g.ok())
            .filter(|g| g.file_name().to_string_lossy().starts_with("broken"))
            .collect();
        assert!(
            remainder.is_empty(),
            "broken code was delivered: {remainder:?}"
        );
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

        let outcome = hold(tool.run(json!({"file_name": "crashing", "lines": lines}), &mut ctx));
        assert!(
            matches!(outcome.state, ToolState::Failed(_)),
            "{}",
            outcome.chip_text
        );
        assert!(
            outcome.to_model.contains("error (runtime)"),
            "{}",
            outcome.to_model
        );
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
        let outcome = hold(tool.run(json!({"file_name": "total", "lines": lines}), &mut ctx));
        assert_eq!(outcome.state, ToolState::Written, "{}", outcome.to_model);

        assert_eq!(
            std::fs::read_to_string(&old).unwrap(),
            "THE USER'S FILE",
            "the user's file was overwritten"
        );
        assert!(
            root.join(format!("total-2.{extension}")).is_file(),
            "the suffixed name was not produced"
        );
        assert!(
            outcome.chip_text.contains(&format!("total-2.{extension}")),
            "{}",
            outcome.chip_text
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// 6b) A DANGLING SYMLINK IS NOT A FREE NAME EITHER.
    ///
    /// The sibling of the create_document hole: `empty_target` used `exists()`,
    /// which FOLLOWS links, so a dangling `total.py -> <outside>` reported "free"
    /// and the verified script was delivered OUTSIDE the sandbox with the user's
    /// full privileges — while the chip named the in-sandbox file.
    #[cfg(unix)]
    #[test]
    fn a_dangling_symlink_does_not_deliver_the_file_outside() {
        let Some(tool) = tool_or_skip() else { return };
        let root = temp_dir("dangling");
        let (lines, extension, _) = example_code(&tool);
        let victim = std::env::temp_dir().join(format!(
            "tacet-write-victim-{}-{extension}",
            std::process::id()
        ));
        std::fs::remove_file(&victim).ok();
        std::os::unix::fs::symlink(&victim, root.join(format!("total.{extension}")))
            .expect("plant the link");

        tool.state.new_turn();
        let mut ctx = context(&root);
        let outcome = hold(tool.run(json!({"file_name": "total", "lines": lines}), &mut ctx));
        assert_eq!(outcome.state, ToolState::Written, "{}", outcome.to_model);

        assert!(
            !victim.exists(),
            "the file was delivered OUTSIDE the sandbox: {}",
            victim.display()
        );
        assert!(
            root.join(format!("total-2.{extension}")).is_file(),
            "the loop did not rotate past the link"
        );
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

        let first = hold(tool.run(json!({"file_name": "a", "lines": lines.clone()}), &mut ctx));
        assert_eq!(first.state, ToolState::Written);
        let exit_code = match run_tool.interpreters()[0].key {
            "python" => "print(2)",
            _ => "console.log(2)",
        };
        let second_pass = hold(run_tool.run(json!({"code": exit_code}), &mut ctx));
        assert_eq!(
            second_pass.state,
            ToolState::Read,
            "{}",
            second_pass.to_model
        );

        let third = hold(tool.run(json!({"file_name": "b", "lines": lines}), &mut ctx));
        assert!(
            third.to_model.starts_with("error_final"),
            "{}",
            third.to_model
        );
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
        let outcome = hold(tool.run(json!({"file_name": "writing", "lines": lines}), &mut ctx));
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
        assert!(
            schema
                .validate(&json!({"file_name": "a", "lines": ["x"]}))
                .is_ok()
        );
        assert!(schema.validate(&json!({"lines": ["x"]})).is_err());
        assert!(schema.validate(&json!({"file_name": "a"})).is_err());
        // An empty array and text-instead-of-array: both FAIL in the schema.
        assert!(
            schema
                .validate(&json!({"file_name": "a", "lines": []}))
                .is_err()
        );
        assert!(
            schema
                .validate(&json!({"file_name": "a", "lines": "def f():"}))
                .is_err()
        );
    }

    /// 10) THE EXTENSION PICKS THE LANGUAGE: while the source is silent, a ".py" name routes to python.
    #[test]
    fn the_extension_picks_the_language_when_the_source_is_silent() {
        let Some(tool) = tool_or_skip() else { return };
        if tool.interpreters.len() < 2 {
            return;
        }
        // The source is valid in both languages — the guess stays silent, the extension speaks.
        assert_eq!(
            tool.resolve_interpreter("script.py", None, "1 + 2").key,
            "python"
        );
        assert_eq!(
            tool.resolve_interpreter("script.js", None, "1 + 2").key,
            "js"
        );
        // If the source SPEAKS it overrides the extension (evidence > declaration).
        assert_eq!(
            tool.resolve_interpreter("script.js", None, "print(len([1]))")
                .key,
            "python"
        );
    }
}
