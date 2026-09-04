//! `checksum` — the SHA-256 fingerprint of a file, and the two questions worth
//! asking about one.
//!
//! WHY THIS TOOL EXISTS: `tacet_kernel::Sha256` is in the kernel because three
//! internal layers need it — the download verifier, the receipt chain, the PKCE
//! challenge — and not one of them is the user. The hash is already written,
//! already proven against the official FIPS 180-4 vectors, and already
//! unreachable to the person whose files it would be about.
//!
//! THREE SHAPES, ONE COMPUTATION. With neither optional argument it reports the
//! digest. With `expected` it answers "is this the file they said it was".
//! With `other` it answers "are these two files the same file". All three are
//! the same 64 hex digits; what changes is the sentence around them.
//!
//! A MISMATCH IS A RESULT, NOT A FAILURE, and that is the one design decision in
//! this file worth arguing about. `ToolOutcome::failed` hands the model
//! `ERROR_MODEL_TEXT` — a fixed "the action could not be completed" — so a model
//! told that would say "I could not check it" when the truth is "I checked it
//! and it does not match". The measurement SUCCEEDED; the answer is negative.
//! It comes back as `read_ok` with `digest_mismatch` in the text.
//!
//! THE BYPASS CHANNEL IS DELIBERATELY NOT USED. The output is 64 hex digits and
//! a byte count whatever the file's size — a constant-size answer. Putting it in
//! the store would make the model resolve a reference to read a single line;
//! `calc.rs` records the same rule ("the channel is used where it pays off").

use std::fs::File;
use std::io::Read;

use serde_json::Value;
use tacet_kernel::hash::hex;
use tacet_kernel::{
    ArgSchema, Field, Sha256, Tool, ToolContext, ToolError, ToolFuture, ToolOutcome, ToolResult,
    TraceUpdate, boxed,
};

/// The read block. 64 KiB is one `Sha256::feed` call's worth of whole blocks and
/// keeps peak memory flat no matter how large the file is — the whole point of
/// streaming rather than `fs::read`.
const CHUNK: usize = 64 * 1024;

/// The upper bound on a file this tool will hash.
///
/// MEASURED ON THIS MACHINE (Apple silicon, `cargo test --release`): the
/// kernel's Sha256 runs at 456.9 MiB/s, so this cap is about 1.1 seconds — and
/// in a `cargo test` debug build the same code runs at 30.9 MiB/s, i.e. 17
/// seconds for the same file. The number is chosen against the first figure and
/// the second is why no test hashes anywhere near it.
///
/// WHY A CAP AT ALL, given that `Sha256::feed`'s own comment names a 2.5 GB
/// file: the hashing happens INSIDE the tool's future, on the tool loop, with no
/// cancellation point in it. An unbounded stream is not a memory risk, it is a
/// user staring at a chip that will not move and cannot be stopped.
/// `read_document` set the same precedent with its 32 MiB `FILE_CAP`; this one
/// is larger because a disk image or an installer is exactly the file somebody
/// wants a checksum for, and 32 MiB would refuse the tool's own use case.
const FILE_CAP: u64 = 512 * 1024 * 1024;

/// The length of a SHA-256 digest written as hex.
const DIGEST_CHARS: usize = 64;

pub struct ChecksumTool {
    /// The size ceiling, as a field ONLY so a test can lower it. A test that had
    /// to write half a gigabyte to prove the cap refuses would not be run, and a
    /// gate nobody measures is a gate nobody keeps. The production path always
    /// gets `FILE_CAP` (`new`).
    max_bytes: u64,
}

impl Default for ChecksumTool {
    fn default() -> Self {
        Self::new()
    }
}

impl ChecksumTool {
    pub fn new() -> Self {
        Self {
            max_bytes: FILE_CAP,
        }
    }

    #[cfg(test)]
    fn with_cap(max_bytes: u64) -> Self {
        Self { max_bytes }
    }
}

impl Tool for ChecksumTool {
    fn name(&self) -> &str {
        "checksum"
    }

    /// KEPT SHORT ON PURPOSE. `router::overlap` — the tie-break that orders every
    /// tool no profile scored — matches word stems over `name + description`, so
    /// each extra sentence here is another chance for this tool to outrank
    /// something on a message that has nothing to do with hashes. The name
    /// already carries the profile hint; the prose only has to say WHEN.
    fn description(&self) -> &str {
        "Computes the SHA-256 checksum of a file on disk. Call this when the user wants a \
         file's hash or fingerprint, wants to know whether a download matches the checksum \
         its publisher gave (pass it as `expected`), or whether two files are byte-for-byte \
         identical (pass the second as `other`). A mismatch is a normal answer, not an error."
    }

    fn schema(&self) -> ArgSchema {
        // THE ONE RULE THIS SCHEMA CANNOT EXPRESS, written down rather than left
        // to be discovered: `expected` and `other` are mutually exclusive, and
        // `ArgSchema` has no "exactly one of these" — the grammar can force a
        // type and a closed set, not a relation between two fields. So that one
        // rule lives in the body and carries its own test
        // (`expected_and_other_together_are_refused`). Everything else IS the
        // shape: there is no field through which a second path, a format switch
        // or an algorithm name could arrive.
        ArgSchema::object(vec![
            Field::new(
                "path",
                ArgSchema::text().description(
                    "Path to the file to fingerprint, relative to the working directory.",
                ),
            )
            .required(),
            Field::new(
                "expected",
                ArgSchema::text().description(
                    "Optional: the 64-character SHA-256 the file should have. \
                     Example: ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
                ),
            ),
            Field::new(
                "other",
                ArgSchema::text().description(
                    "Optional: a second file to compare against, instead of `expected`.",
                ),
            ),
        ])
        .description("SHA-256 fingerprint of a file, or a comparison")
    }

    /// A digest is a LINKABLE FINGERPRINT of the user's own file — it identifies
    /// the file exactly to anyone holding a copy. Same reasoning as
    /// `read_document`: once it is in the window a later web/mcp call could carry
    /// it off the device, so the session is tainted and the next external tool
    /// meets the approval gate.
    fn taints_session(&self) -> bool {
        true
    }

    fn run<'a>(&'a self, args: Value, ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move {
            let trace = ctx.start_chip("hash", "Fingerprinting…");

            let (outcome, tainted) = match self.work(&args, ctx) {
                Ok(o) => (o, true),
                Err(e) => (ToolOutcome::failed(&e), false),
            };

            ctx.update_chip(
                trace,
                TraceUpdate::state(outcome.state.clone())
                    .text(outcome.chip_text.clone())
                    .raw_input(args.to_string())
                    .raw_output(outcome.raw_output.clone().unwrap_or_default()),
            );
            // A refusal read nothing; only a real digest taints.
            if tainted {
                ctx.taint();
            }
            outcome
        })
    }
}

impl ChecksumTool {
    fn work(&self, args: &Value, ctx: &ToolContext) -> ToolResult<ToolOutcome> {
        self.schema().validate(args)?;

        let raw_path = args
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::MissingField("path".into()))?;
        let expected = args
            .get("expected")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let other = args
            .get("other")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());

        if expected.is_some() && other.is_some() {
            return Err(ToolError::InvalidArgument(
                "give either `expected` or `other`, not both".into(),
            ));
        }

        let (digest, size) = self.digest_of(ctx, raw_path)?;
        let name = leaf(raw_path);

        // THE VERDICT, and the three arms are three different sentences to the
        // user for the same computation.
        match (expected, other) {
            (Some(claim), _) => {
                let wanted = parse_digest(claim)?;
                let matched = wanted == digest;
                Ok(verdict(&name, matched, &digest, size, None))
            }
            (_, Some(second_path)) => {
                let (second, _) = self.digest_of(ctx, second_path)?;
                let matched = second == digest;
                Ok(verdict(&name, matched, &digest, size, Some(&second)))
            }
            (None, None) => Ok(ToolOutcome::read_ok(
                format!("{name} · {}", &digest[..12]),
                format!("sha256={digest} bytes={size}"),
            )
            .raw_output(digest)),
        }
    }

    /// Streams the file through the hasher and returns `(hex digest, bytes)`.
    fn digest_of(&self, ctx: &ToolContext, raw: &str) -> ToolResult<(String, u64)> {
        // BOTH path arguments go through the SAME gate, and that is the reason
        // this is a function rather than two call sites: `other` is as much a
        // path from the model as `path` is, and a second gate written a second
        // time is the one that ends up missing a case. `resolve_existing_file`
        // canonicalizes every component, so a link planted inside the sandbox
        // cannot be hashed through — and it also proves the target is a FILE, so
        // a directory is refused rather than read as zero bytes.
        let path = crate::sandbox_path::resolve_existing_file(ctx, raw)?;
        let size = path
            .metadata()
            .map_err(|_| ToolError::FileNotFound(path.clone()))?
            .len();
        if size > self.max_bytes {
            return Err(ToolError::Other(format!(
                "the file is {size} bytes, over the {} byte limit",
                self.max_bytes
            )));
        }

        let mut file = File::open(&path)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; CHUNK];
        let mut read_total = 0u64;
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            read_total += n as u64;
            // THE CAP IS RE-CHECKED WHILE READING, not only from the metadata.
            // A file being appended to between `metadata()` and the last read
            // would otherwise slip past a ceiling that was measured on a
            // different file than the one being hashed.
            if read_total > self.max_bytes {
                return Err(ToolError::Other(format!(
                    "the file grew past the {} byte limit while it was being read",
                    self.max_bytes
                )));
            }
            hasher.feed(&buffer[..n]);
        }
        Ok((hex(&hasher.finish()), read_total))
    }
}

/// The answer when there was something to compare against.
///
/// `read_ok` ON BOTH ARMS. A mismatch is the tool working; see the file header
/// for why `failed` would make the model say the opposite of what happened.
fn verdict(name: &str, matched: bool, digest: &str, size: u64, other: Option<&str>) -> ToolOutcome {
    let chip = if matched {
        format!("{name} · matches")
    } else {
        format!("{name} · DOES NOT MATCH")
    };
    let head = if matched {
        "digest_match"
    } else {
        "digest_mismatch"
    };
    let to_model = match other {
        // Both digests go back only when TWO FILES were compared and they
        // differ: that is the one case where the second number tells the user
        // something they cannot get from the first. It is ~175 characters, still
        // a constant, and still no reason for a store reference.
        Some(second) if !matched => {
            format!("{head} sha256={digest} other_sha256={second} bytes={size}")
        }
        _ => format!("{head} sha256={digest} bytes={size}"),
    };
    ToolOutcome::read_ok(chip, to_model).raw_output(digest.to_string())
}

/// Accepts a digest the user or a publisher wrote down, or refuses it.
///
/// THE SHAPE IS THE CHECK. 64 hex characters is the only thing a SHA-256 is, so
/// a 63-character string, a `sha256:` prefix or an uppercase-with-spaces copy
/// out of a web page is refused rather than compared — and a PREFIX of the right
/// digest is refused too, which is the attack a `starts_with` comparison would
/// walk straight into. Case is folded because `sha256sum` writes lowercase and
/// plenty of release pages write uppercase, and that difference means nothing.
fn parse_digest(raw: &str) -> ToolResult<String> {
    let cleaned = raw.trim();
    if cleaned.chars().count() != DIGEST_CHARS || !cleaned.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ToolError::InvalidArgument(
            "`expected` must be exactly 64 hexadecimal characters".into(),
        ));
    }
    Ok(cleaned.to_ascii_lowercase())
}

/// The file's own name, for the chip. The chip is one line and a full path
/// breaks the layout; the model already knows the path it passed.
fn leaf(raw: &str) -> String {
    raw.rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(raw)
        .to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use tacet_kernel::{InMemoryDataStore, SilentReporter, ToolState};

    /// THE OFFICIAL VECTOR, reused rather than restated: `sha256_hex(b"abc")` is
    /// proven against FIPS 180-4 in `tacet_kernel::hash`'s own tests. If this
    /// tool ever answered something else, the difference would be in the
    /// streaming/reading layer this file adds, which is exactly what wants
    /// measuring.
    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn temp_root(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "tacet-checksum-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).expect("temp dir");
        path.canonicalize().expect("resolved")
    }

    fn isolated() -> std::sync::MutexGuard<'static, ()> {
        let guard = crate::workspace::test_lock();
        crate::workspace::clear_roots();
        guard
    }

    fn context(root: &Path) -> ToolContext {
        ToolContext::new(
            Arc::new(InMemoryDataStore::new()),
            root.to_path_buf(),
            Arc::new(SilentReporter),
        )
    }

    fn execute<F: std::future::Future>(mut f: F) -> F::Output {
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        fn noop(_: *const ()) {}
        fn clone(p: *const ()) -> RawWaker {
            RawWaker::new(p, &VT)
        }
        static VT: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VT)) };
        let mut cx = Context::from_waker(&waker);
        let mut f = unsafe { std::pin::Pin::new_unchecked(&mut f) };
        loop {
            if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }

    #[test]
    fn the_digest_matches_the_official_vector_and_stays_small() {
        let _guard = isolated();
        let root = temp_root("vector");
        std::fs::write(root.join("a.txt"), b"abc").expect("write");
        let ctx = context(&root);
        let out = ChecksumTool::new()
            .work(&json!({"path": "a.txt"}), &ctx)
            .expect("hashes");
        assert_eq!(out.to_model, format!("sha256={ABC} bytes=3"));
        // THE "NO DATASTORE NEEDED" CLAIM, AS A NUMBER. The answer is a constant
        // size whatever the file is, which is the whole argument for keeping it
        // out of the bypass channel.
        assert!(out.to_model.len() < 120, "{}", out.to_model.len());
        assert_eq!(out.state, ToolState::Read);
    }

    /// A FILE LARGER THAN ONE CHUNK, so the streaming loop is exercised rather
    /// than a single `feed`. A digest that only ever saw one block would agree
    /// with the vector above and still be wrong for every real file.
    #[test]
    fn a_file_larger_than_one_chunk_hashes_the_same_as_the_whole_slice() {
        let _guard = isolated();
        let root = temp_root("chunked");
        let body: Vec<u8> = (0..(CHUNK * 2 + 777)).map(|i| (i % 251) as u8).collect();
        std::fs::write(root.join("big.bin"), &body).expect("write");
        let ctx = context(&root);
        let out = ChecksumTool::new()
            .work(&json!({"path": "big.bin"}), &ctx)
            .expect("hashes");
        assert_eq!(
            out.to_model,
            format!(
                "sha256={} bytes={}",
                tacet_kernel::sha256_hex(&body),
                body.len()
            )
        );
    }

    #[test]
    fn a_prefix_of_the_right_digest_is_not_a_match() {
        let _guard = isolated();
        let root = temp_root("prefix");
        std::fs::write(root.join("a.txt"), b"abc").expect("write");
        let ctx = context(&root);
        let tool = ChecksumTool::new();
        // The TRUNCATION ATTACK: a 64-character string that is the right digest
        // with its tail replaced. A `starts_with` comparison would accept the
        // first form; equality does not.
        let mut truncated = ABC[..32].to_string();
        truncated.push_str(&"0".repeat(32));
        let out = tool
            .work(&json!({"path": "a.txt", "expected": truncated}), &ctx)
            .expect("a mismatch is an answer");
        assert!(out.to_model.starts_with("digest_mismatch"));
        // And a genuinely short prefix is refused on SHAPE, before any
        // comparison happens at all.
        assert!(
            tool.work(&json!({"path": "a.txt", "expected": &ABC[..32]}), &ctx)
                .is_err()
        );
    }

    #[test]
    fn an_expected_that_is_not_sixty_four_hex_characters_is_refused() {
        let _guard = isolated();
        let root = temp_root("shape");
        std::fs::write(root.join("a.txt"), b"abc").expect("write");
        let ctx = context(&root);
        let tool = ChecksumTool::new();
        for bad in [
            &ABC[..63],         // one short
            &format!("{ABC}0"), // one long
            "not-hex-not-hex-not-hex-not-hex-not-hex-not-hex-not-hex-not-hexx",
            &format!("sha256:{}", &ABC[..57]), // the prefixed form
        ] {
            let err = tool
                .work(&json!({"path": "a.txt", "expected": bad}), &ctx)
                .err()
                .unwrap_or_else(|| panic!("`{bad}` was accepted as a digest"));
            assert!(matches!(err, ToolError::InvalidArgument(_)));
        }
    }

    #[test]
    fn case_does_not_change_the_verdict() {
        let _guard = isolated();
        let root = temp_root("case");
        std::fs::write(root.join("a.txt"), b"abc").expect("write");
        let ctx = context(&root);
        let out = ChecksumTool::new()
            .work(
                &json!({"path": "a.txt", "expected": ABC.to_uppercase()}),
                &ctx,
            )
            .expect("hashes");
        assert!(out.to_model.starts_with("digest_match"));
    }

    /// A MISMATCH IS A RESULT, NOT A FAILURE — the one design decision in this
    /// file worth a test of its own. `ToolOutcome::failed` would hand the model
    /// `ERROR_MODEL_TEXT`, and the model would then tell the user "I could not
    /// check it" when the truth is "I checked it and it does not match".
    #[test]
    fn a_mismatch_is_a_result_and_not_a_failure() {
        let _guard = isolated();
        let root = temp_root("mismatch");
        std::fs::write(root.join("a.txt"), b"abc").expect("write");
        let ctx = context(&root);
        let out = ChecksumTool::new()
            .work(&json!({"path": "a.txt", "expected": "0".repeat(64)}), &ctx)
            .expect("a mismatch is still a successful measurement");
        assert_eq!(out.state, ToolState::Read);
        assert!(out.to_model.contains("digest_mismatch"));
        assert_ne!(out.to_model, tacet_kernel::ERROR_MODEL_TEXT);
        assert!(out.chip_text.contains("DOES NOT MATCH"));
    }

    #[test]
    fn two_copies_are_identical_and_two_files_differ() {
        let _guard = isolated();
        let root = temp_root("compare");
        std::fs::write(root.join("a.txt"), b"abc").expect("write");
        std::fs::write(root.join("copy.txt"), b"abc").expect("write");
        std::fs::write(root.join("other.txt"), b"abd").expect("write");
        let ctx = context(&root);
        let tool = ChecksumTool::new();

        let same = tool
            .work(&json!({"path": "a.txt", "other": "copy.txt"}), &ctx)
            .expect("compares");
        assert!(same.to_model.starts_with("digest_match"));

        let different = tool
            .work(&json!({"path": "a.txt", "other": "other.txt"}), &ctx)
            .expect("compares");
        assert!(different.to_model.starts_with("digest_mismatch"));
        // BOTH digests come back on this arm and only this arm: it is the one
        // case where the second number tells the user something the first
        // cannot.
        assert!(different.to_model.contains("other_sha256="));
    }

    /// THE ONE RULE THE SCHEMA CANNOT EXPRESS. `ArgSchema` can force a type and
    /// a closed set; it has no "exactly one of these two". That is why the rule
    /// lives in the body and why it is measured here rather than assumed.
    #[test]
    fn expected_and_other_together_are_refused() {
        let _guard = isolated();
        let root = temp_root("both");
        std::fs::write(root.join("a.txt"), b"abc").expect("write");
        std::fs::write(root.join("b.txt"), b"abc").expect("write");
        let ctx = context(&root);
        let err = ChecksumTool::new()
            .work(
                &json!({"path": "a.txt", "expected": ABC, "other": "b.txt"}),
                &ctx,
            )
            .expect_err("both at once must be refused");
        assert!(matches!(err, ToolError::InvalidArgument(_)));
    }

    /// BOTH path arguments go through the SAME gate. `other` is as much a path
    /// from the model as `path` is, and a second gate written a second time is
    /// the one that ends up missing a case.
    #[test]
    fn both_path_arguments_are_sandboxed() {
        let _guard = isolated();
        let root = temp_root("sandbox");
        let outside = temp_root("sandbox-outside");
        std::fs::write(outside.join("secret.txt"), b"PRIVATE KEY MATERIAL").expect("write");
        std::fs::write(root.join("a.txt"), b"abc").expect("write");
        let ctx = context(&root);
        let tool = ChecksumTool::new();

        for args in [
            json!({"path": "../../secret.txt"}),
            json!({"path": "a.txt", "other": "../../secret.txt"}),
        ] {
            assert!(
                matches!(
                    tool.work(&args, &ctx).err(),
                    Some(ToolError::SandboxViolation(_))
                ),
                "{args} escaped the sandbox"
            );
        }

        // A PLANTED LINK ON EITHER ARGUMENT. `run_code` may legitimately create
        // a link inside the sandbox, so a lexical check would walk straight
        // through this one — and a digest identifies the file behind it exactly.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("link.txt"))
                .expect("plant");
            for args in [
                json!({"path": "link.txt"}),
                json!({"path": "a.txt", "other": "link.txt"}),
            ] {
                assert!(
                    matches!(
                        tool.work(&args, &ctx).err(),
                        Some(ToolError::SandboxViolation(_))
                    ),
                    "{args} was hashed through a planted link"
                );
            }
        }
    }

    #[test]
    fn a_missing_file_and_a_directory_are_both_refused() {
        let _guard = isolated();
        let root = temp_root("shapes");
        std::fs::create_dir(root.join("folder")).expect("dir");
        let ctx = context(&root);
        let tool = ChecksumTool::new();
        assert!(matches!(
            tool.work(&json!({"path": "nope.txt"}), &ctx).err(),
            Some(ToolError::FileNotFound(_))
        ));
        // A DIRECTORY IS NOT ZERO BYTES. `resolve_existing_file` proves it is a
        // file; `resolve_existing` alone would hand back a directory and the
        // read loop would report the digest of nothing.
        assert!(matches!(
            tool.work(&json!({"path": "folder"}), &ctx).err(),
            Some(ToolError::FileNotFound(_))
        ));
    }

    /// THE SIZE CEILING REFUSES RATHER THAN HANGS.
    ///
    /// The cap is lowered for the test through `with_cap` for one reason: at the
    /// 30.9 MiB/s this code runs at in a debug build, a test that wrote the real
    /// 512 MiB ceiling would take seventeen seconds and would not be run. A gate
    /// nobody measures is a gate nobody keeps.
    #[test]
    fn a_file_over_the_ceiling_is_refused_rather_than_hashed() {
        let _guard = isolated();
        let root = temp_root("cap");
        std::fs::write(root.join("big.bin"), vec![0u8; 4096]).expect("write");
        std::fs::write(root.join("small.bin"), vec![0u8; 1024]).expect("write");
        let ctx = context(&root);
        let tool = ChecksumTool::with_cap(2048);
        assert!(
            tool.work(&json!({"path": "big.bin"}), &ctx)
                .expect_err("over the cap")
                .short_error()
                .contains("over the"),
        );
        // And the ceiling is a ceiling, not an off switch.
        assert!(tool.work(&json!({"path": "small.bin"}), &ctx).is_ok());
        // THE PRODUCTION CEILING IS THE ONE IN THE CONSTANT — `with_cap` is a
        // test seam, so this asserts the seam has not become the default.
        assert_eq!(ChecksumTool::new().max_bytes, FILE_CAP);
    }

    #[test]
    fn a_refusal_does_not_taint_the_session_and_a_success_does() {
        let _guard = isolated();
        let root = temp_root("taint");
        std::fs::write(root.join("a.txt"), b"abc").expect("write");
        let mut ctx = context(&root);
        let tool = ChecksumTool::new();

        let refused = execute(tool.run(json!({"path": "../../nope.txt"}), &mut ctx));
        assert!(matches!(refused.state, ToolState::Failed(_)));
        assert!(!ctx.session_tainted(), "a refusal read nothing");

        let ok = execute(tool.run(json!({"path": "a.txt"}), &mut ctx));
        assert_eq!(ok.state, ToolState::Read);
        assert!(!ok.state.changed_world(), "hashing changes nothing on disk");
        assert!(
            ctx.session_tainted(),
            "a digest is a linkable fingerprint of the user's file"
        );
        assert!(tool.taints_session());
    }

    #[test]
    fn the_schema_is_the_boundary() {
        let js = ChecksumTool::new().schema().json_schema();
        assert_eq!(js["required"], json!(["path"]));
        assert_eq!(js["additionalProperties"], json!(false));
        assert_eq!(js["properties"]["expected"]["type"], json!("string"));
        assert_eq!(js["properties"]["other"]["type"], json!("string"));
        // There is no algorithm field: SHA-256 is the whole tool, so "md5" is
        // not something the model can ask for and be quietly given.
        assert!(js["properties"]["algorithm"].is_null());

        let _guard = isolated();
        let root = temp_root("schema");
        let ctx = context(&root);
        let tool = ChecksumTool::new();
        assert!(tool.work(&json!({}), &ctx).is_err());
        assert!(tool.work(&json!({"path": 12}), &ctx).is_err());
    }
}
