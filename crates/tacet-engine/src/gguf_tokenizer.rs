//! The tokenizer that lives INSIDE the GGUF file.
//!
//! WHY THIS EXISTS: a GGUF weight file already carries the WHOLE tokenizer
//! (`tokenizer.ggml.tokens`, `.merges`, `.token_type`). Until now the engine
//! insisted on a separate `tokenizer.json` next to the weights, so a package the
//! user downloaded as a single 2.5 GB file counted as HALF and could not be
//! loaded at all. The data was there; we were just not reading it.
//!
//! ONLY THE METADATA IS READ. The tensor section is gigabytes; this module never
//! touches it. The file is walked forward key by key and every value we do not
//! need is skipped without being kept — so `gguf_has_tokenizer` costs a few MB
//! of sequential reads, not 2.5 GB.
//!
//! THE SILENT FAILURE THIS GUARDS AGAINST: a wrong tokenizer does not error. It
//! produces text that looks like a broken MODEL — mangled Turkish letters,
//! swallowed spaces, tool calls that never parse. There is no stack trace and
//! nothing points at the tokenizer. That is why the acceptance test for this
//! module is not "it builds" but "its token id sequences are BYTE-IDENTICAL to
//! the real `tokenizer.json`" (see `tests/gguf_tokenizer.rs`).
//!
//! NO NETWORK, NO NEW DEPENDENCY: the GGUF reader below is hand-written on
//! `std::io`, the same way this project writes its own zip/deflate.

use crate::error::{EngineError, EngineResult};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

// --- The GGUF metadata keys we care about ----------------------------------

const KEY_MODEL: &str = "tokenizer.ggml.model";
const KEY_PRE: &str = "tokenizer.ggml.pre";
const KEY_TOKENS: &str = "tokenizer.ggml.tokens";
const KEY_TOKEN_TYPE: &str = "tokenizer.ggml.token_type";
const KEY_MERGES: &str = "tokenizer.ggml.merges";

/// The context window the model itself declares — `<arch>.context_length`.
///
/// THE ARCHITECTURE PREFIX IS NOT WRITTEN OUT. Every file spells this key with
/// its own architecture name (`qwen3.context_length`, `qwen2.context_length`,
/// `gemma3.context_length`), and a hard-coded prefix would mean the next
/// architecture silently falls back to the default window instead of using the
/// window it declares. The key is matched by SUFFIX, and the architecture name is
/// whatever `general.architecture` says.
const KEY_CONTEXT_SUFFIX: &str = ".context_length";

/// Is this metadata key the model's own context window?
///
/// PUBLIC AND SHARED because two call sites ask the same question from different
/// places: discovery walks the raw file (`gguf_context_length`), the loader
/// already has the parsed metadata in hand (`candle_engine::read_context_length`).
/// Written twice, the two would drift and the loader would end up using a window
/// discovery never checked.
///
/// THE PREFIX MUST BE A SINGLE SEGMENT. The suffix alone would also match
/// `<arch>.rope.scaling.original_context_length` — the window BEFORE rope
/// scaling, a smaller and different number that would quietly shrink the budget
/// on any file carrying it.
pub fn is_context_length_key(key: &str) -> bool {
    match key.strip_suffix(KEY_CONTEXT_SUFFIX) {
        Some(prefix) => !prefix.is_empty() && !prefix.contains('.'),
        None => false,
    }
}

// --- GGUF value type tags (the on-disk numbering) ---------------------------

const T_U8: u32 = 0;
const T_I8: u32 = 1;
const T_U16: u32 = 2;
const T_I16: u32 = 3;
const T_U32: u32 = 4;
const T_I32: u32 = 5;
const T_F32: u32 = 6;
const T_BOOL: u32 = 7;
const T_STRING: u32 = 8;
const T_ARRAY: u32 = 9;
const T_U64: u32 = 10;
const T_I64: u32 = 11;
const T_F64: u32 = 12;

/// `token_type` values, from llama.cpp's `llama_token_attr`/`token_type` enum.
/// The numbering is part of the FILE FORMAT, not a local convention.
///
/// Only the tokenizer-BUILDING half of this module reads them, so they are cfg'd
/// the same way it is — the cheap presence check does not look at token types,
/// and an unconditional constant would warn as dead in the default build.
#[cfg(any(feature = "candle", test))]
mod token_type {
    pub const NORMAL: i32 = 1;
    pub const CONTROL: i32 = 3;
    pub const USER_DEFINED: i32 = 4;
    pub const UNUSED: i32 = 5;
}

/// The vocabulary family we can rebuild. `gpt2` in GGUF means byte-level BPE
/// with a `merges` table — the GPT-2/Qwen line.
///
/// `llama` (sentencepiece/unigram, i.e. Gemma) is DELIBERATELY not handled:
/// unigram needs scores rather than merges, and nothing on this machine could be
/// used to verify it. Claiming support we never measured is the exact failure
/// this module is written to avoid.
const SUPPORTED_VOCAB_MODEL: &str = "gpt2";

/// The pre-tokenizer split regex for `tokenizer.ggml.pre`.
///
/// GGUF stores only the NAME of the regex set ("qwen2"), not the regex. The
/// pattern below is the one in the real `tokenizer.json` of
/// Qwen3-4B-Instruct-2507, copied verbatim — and it is verified against that
/// file by the cross-check test, not assumed.
///
/// AN UNKNOWN NAME RETURNS `None` INSTEAD OF FALLING BACK to the "closest"
/// pattern. A near-miss regex splits `don't`, digit runs and newlines slightly
/// differently, which shifts a handful of token ids in a 151k vocabulary — the
/// output then looks like a broken model and nothing points here.
fn split_regex(pre: &str) -> Option<&'static str> {
    match pre {
        "qwen2" => Some(
            r"(?i:'s|'t|'re|'ve|'m|'ll|'d)|[^\r\n\p{L}\p{N}]?\p{L}+|\p{N}| ?[^\s\p{L}\p{N}]+[\r\n]*|\s*[\r\n]+|\s+(?!\S)|\s+",
        ),
        _ => None,
    }
}

/// The tokenizer-related part of the GGUF metadata.
///
/// The arrays are empty when the scan ran in "presence only" mode
/// (`gguf_has_tokenizer`); `has_*` says whether the key was actually there.
#[derive(Debug, Default)]
struct RawTokenizer {
    model: Option<String>,
    pre: Option<String>,
    tokens: Vec<String>,
    token_types: Vec<i32>,
    merges: Vec<String>,
    has_tokens: bool,
    has_token_types: bool,
    has_merges: bool,
    /// `<arch>.context_length`, when the file carries it. NOT tokenizer data —
    /// it rides along here because the metadata block is walked exactly once and
    /// a second full pass for one integer would be pure waste.
    context_length: Option<u64>,
    /// The four numbers the KV cache size is computed from, same reasoning as
    /// `context_length`: one walk, not four.
    block_count: Option<u64>,
    kv_head_count: Option<u64>,
    key_length: Option<u64>,
    value_length: Option<u64>,
    /// Only needed when the file omits `key_length`/`value_length`, which older
    /// Qwen2 exports do — the head dimension is then `embedding / heads`.
    embedding_length: Option<u64>,
    head_count: Option<u64>,
}

impl RawTokenizer {
    /// Everything needed to REBUILD the tokenizer is present AND of a kind we
    /// have actually verified.
    fn is_usable(&self) -> bool {
        self.model.as_deref() == Some(SUPPORTED_VOCAB_MODEL)
            && self.pre.as_deref().and_then(split_regex).is_some()
            && self.has_tokens
            && self.has_token_types
            && self.has_merges
    }
}

// ---------------------------------------------------------------------------
// The GGUF metadata reader
// ---------------------------------------------------------------------------

fn fail(message: impl Into<String>) -> EngineError {
    EngineError::Tokenization(message.into())
}

/// A forward-only reader over the GGUF header.
///
/// It is deliberately NOT seekable: the header is read once, start to finish,
/// and stopping at the end of the key/value block means the tensor section is
/// never even addressed. `position` is tracked only to bound the lengths read
/// from the file against the real file size — see `check_length`.
struct MetaReader<R: Read> {
    inner: R,
    position: u64,
    file_length: u64,
    /// Reused when skipping, so a 4 MB header does not become 300k allocations.
    scratch: Vec<u8>,
}

impl<R: Read> MetaReader<R> {
    fn new(inner: R, file_length: u64) -> Self {
        Self {
            inner,
            position: 0,
            file_length,
            scratch: vec![0u8; 64 * 1024],
        }
    }

    /// A length read FROM THE FILE may not exceed what is left of the file.
    ///
    /// WHY THIS GUARD: `tokens` is a u64 element count taken straight off disk.
    /// On a truncated or corrupt file that number can be nonsense, and
    /// `Vec::with_capacity(nonsense)` aborts the process instead of returning an
    /// error. The file's own size is the only honest upper bound we have.
    fn check_length(&self, count: u64, unit: u64) -> EngineResult<usize> {
        let left = self.file_length.saturating_sub(self.position);
        if count.saturating_mul(unit.max(1)) > left {
            return Err(fail(format!(
                "gguf metadata is corrupt: a length of {count} does not fit the remaining {left} bytes"
            )));
        }
        usize::try_from(count).map_err(|_| fail("gguf length does not fit in usize"))
    }

    fn read_exact(&mut self, buffer: &mut [u8]) -> EngineResult<()> {
        self.inner.read_exact(buffer)?;
        self.position += buffer.len() as u64;
        Ok(())
    }

    fn u32(&mut self) -> EngineResult<u32> {
        let mut b = [0u8; 4];
        self.read_exact(&mut b)?;
        Ok(u32::from_le_bytes(b))
    }

    fn i32(&mut self) -> EngineResult<i32> {
        Ok(self.u32()? as i32)
    }

    fn u64(&mut self) -> EngineResult<u64> {
        let mut b = [0u8; 8];
        self.read_exact(&mut b)?;
        Ok(u64::from_le_bytes(b))
    }

    /// GGUF strings are NOT null-terminated and NOT guaranteed valid UTF-8 by
    /// the format. They are lossy-decoded rather than rejected: one odd byte in
    /// a 151k vocabulary should not make the whole file unloadable, and the
    /// vocabulary-size post-condition in `tokenizer_from_gguf` still catches a
    /// vocabulary that really got mangled.
    fn string(&mut self) -> EngineResult<String> {
        let length = self.u64()?;
        let length = self.check_length(length, 1)?;
        let mut bytes = vec![0u8; length];
        self.read_exact(&mut bytes)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn skip(&mut self, mut count: u64) -> EngineResult<()> {
        self.check_length(count, 1)?;
        while count > 0 {
            let chunk = count.min(self.scratch.len() as u64) as usize;
            // `scratch` is borrowed and read into at the same time, so the buffer
            // is swapped out for the duration of the read.
            let mut buffer = std::mem::take(&mut self.scratch);
            let result = self.inner.read_exact(&mut buffer[..chunk]);
            self.scratch = buffer;
            result?;
            self.position += chunk as u64;
            count -= chunk as u64;
        }
        Ok(())
    }

    fn skip_string(&mut self) -> EngineResult<()> {
        let length = self.u64()?;
        self.skip(length)
    }

    /// The byte width of a scalar type; `None` for string/array.
    fn scalar_width(kind: u32) -> Option<u64> {
        match kind {
            T_U8 | T_I8 | T_BOOL => Some(1),
            T_U16 | T_I16 => Some(2),
            T_U32 | T_I32 | T_F32 => Some(4),
            T_U64 | T_I64 | T_F64 => Some(8),
            _ => None,
        }
    }

    fn skip_value(&mut self, kind: u32) -> EngineResult<()> {
        if let Some(width) = Self::scalar_width(kind) {
            return self.skip(width);
        }
        match kind {
            T_STRING => self.skip_string(),
            T_ARRAY => {
                let element = self.u32()?;
                let count = self.u64()?;
                if let Some(width) = Self::scalar_width(element) {
                    let total = count
                        .checked_mul(width)
                        .ok_or_else(|| fail("gguf array length overflows when multiplied out"))?;
                    return self.skip(total);
                }
                // Strings have to be walked one by one: their lengths are only
                // known from the stream.
                let count = self.check_length(count, 8)?;
                for _ in 0..count {
                    self.skip_value(element)?;
                }
                Ok(())
            }
            other => Err(fail(format!("unknown gguf value type: {other}"))),
        }
    }

    /// Reads a value that should be a non-negative integer, WHATEVER WIDTH it was
    /// written at, and steps over it if it is anything else.
    ///
    /// WHY ALL FOUR WIDTHS: `context_length` is a u32 in every file on this
    /// machine, but the GGUF format does not fix the width and converters differ.
    /// Accepting only u32 would make one converter's output look like "the model
    /// declares no window" — the same silent fallback the suffix match exists to
    /// avoid. A negative or non-integer value yields `None` RATHER THAN AN ERROR:
    /// the caller's question is "does this file declare a window", and a garbled
    /// value has the same answer as a missing one.
    fn unsigned_value(&mut self, kind: u32) -> EngineResult<Option<u64>> {
        match kind {
            T_U32 => Ok(Some(u64::from(self.u32()?))),
            T_U64 => Ok(Some(self.u64()?)),
            T_I32 => Ok(u64::try_from(self.i32()?).ok()),
            T_I64 => Ok(u64::try_from(self.u64()? as i64).ok()),
            other => {
                self.skip_value(other)?;
                Ok(None)
            }
        }
    }

    fn string_array(&mut self) -> EngineResult<Vec<String>> {
        let element = self.u32()?;
        let count = self.u64()?;
        if element != T_STRING {
            return Err(fail(format!(
                "expected a string array, the element type is {element}"
            )));
        }
        // 8 = the minimum on-disk size of one element (its u64 length prefix).
        let count = self.check_length(count, 8)?;
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            out.push(self.string()?);
        }
        Ok(out)
    }

    fn int_array(&mut self) -> EngineResult<Vec<i32>> {
        let element = self.u32()?;
        let count = self.u64()?;
        // Writers differ on whether `token_type` is signed; both widths are 4
        // bytes and the values are small, so both are accepted.
        if element != T_I32 && element != T_U32 {
            return Err(fail(format!(
                "expected a 32-bit int array, the element type is {element}"
            )));
        }
        let count = self.check_length(count, 4)?;
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            out.push(self.i32()?);
        }
        Ok(out)
    }
}

/// Walks the GGUF key/value block and picks out the tokenizer keys.
///
/// `with_arrays == false` reads only the two small strings and SKIPS the three
/// big arrays — that is what makes `gguf_has_tokenizer` cheap enough for the
/// discovery path to call it on every package it finds.
fn read_metadata(path: &Path, with_arrays: bool) -> EngineResult<RawTokenizer> {
    let file = File::open(path).map_err(|_| EngineError::ModelNotLoaded(path.to_path_buf()))?;
    let file_length = file.metadata()?.len();
    let mut reader = MetaReader::new(BufReader::with_capacity(256 * 1024, file), file_length);

    let mut magic = [0u8; 4];
    reader.read_exact(&mut magic)?;
    if &magic != b"GGUF" {
        return Err(fail("not a gguf file (the 'GGUF' magic is missing)"));
    }
    let version = reader.u32()?;
    // v1 wrote 32-bit counts; nothing in circulation uses it any more and
    // guessing at a layout we cannot test would be worse than refusing.
    if !(2..=3).contains(&version) {
        return Err(fail(format!(
            "unsupported gguf version: {version} (supported: 2, 3)"
        )));
    }
    let _tensor_count = reader.u64()?;
    let kv_count = reader.u64()?;
    // 12 = the smallest possible key/value pair (u64 name length + u32 type).
    let kv_count = reader.check_length(kv_count, 12)?;

    let mut raw = RawTokenizer::default();
    for _ in 0..kv_count {
        let key = reader.string()?;
        let kind = reader.u32()?;
        match key.as_str() {
            KEY_MODEL if kind == T_STRING => raw.model = Some(reader.string()?),
            KEY_PRE if kind == T_STRING => raw.pre = Some(reader.string()?),
            KEY_TOKENS if kind == T_ARRAY => {
                raw.has_tokens = true;
                if with_arrays {
                    raw.tokens = reader.string_array()?;
                } else {
                    reader.skip_value(kind)?;
                }
            }
            KEY_TOKEN_TYPE if kind == T_ARRAY => {
                raw.has_token_types = true;
                if with_arrays {
                    raw.token_types = reader.int_array()?;
                } else {
                    reader.skip_value(kind)?;
                }
            }
            KEY_MERGES if kind == T_ARRAY => {
                raw.has_merges = true;
                if with_arrays {
                    raw.merges = reader.string_array()?;
                } else {
                    reader.skip_value(kind)?;
                }
            }
            other if ends_with_segment(other, ".block_count") => {
                raw.block_count = reader.unsigned_value(kind)?;
            }
            other if other.ends_with(".attention.head_count_kv") => {
                raw.kv_head_count = reader.unsigned_value(kind)?;
            }
            other if other.ends_with(".attention.key_length") => {
                raw.key_length = reader.unsigned_value(kind)?;
            }
            other if other.ends_with(".attention.value_length") => {
                raw.value_length = reader.unsigned_value(kind)?;
            }
            other if ends_with_segment(other, ".embedding_length") => {
                raw.embedding_length = reader.unsigned_value(kind)?;
            }
            other if other.ends_with(".attention.head_count") => {
                raw.head_count = reader.unsigned_value(kind)?;
            }
            // `<arch>.context_length` — the model's own window; the architecture
            // name is not written out here (see `is_context_length_key`).
            other if is_context_length_key(other) => {
                raw.context_length = reader.unsigned_value(kind)?;
            }
            // Everything else — including the chat template and the whole
            // architecture block — is stepped over without being kept.
            _ => reader.skip_value(kind)?,
        }
    }
    Ok(raw)
}

/// Does this GGUF carry a tokenizer WE CAN REBUILD?
///
/// CHEAP: it reads only the metadata block, skipping the value of every key it
/// does not need, and never addresses the tensor section. On the 2.5 GB
/// Qwen3-4B file this is a few MB of sequential reading.
///
/// IT RETURNS FALSE, NOT AN ERROR, for an unreadable or foreign file. The caller
/// is discovery, and its question is "can I set this package up without a
/// `tokenizer.json`" — a missing file, a sentencepiece vocabulary and an
/// unknown pre-tokenizer name all have the same answer: no.
pub fn gguf_has_tokenizer(path: &Path) -> bool {
    read_metadata(path, false)
        .map(|r| r.is_usable())
        .unwrap_or(false)
}

/// `<arch>.<name>` with exactly one segment before the suffix — the same guard
/// `is_context_length_key` uses, for the same reason: a bare suffix match also
/// catches nested keys that mean something else.
fn ends_with_segment(key: &str, suffix: &str) -> bool {
    match key.strip_suffix(suffix) {
        Some(prefix) => !prefix.is_empty() && !prefix.contains('.'),
        None => false,
    }
}

/// Bytes of KV cache this model needs PER TOKEN of context.
///
/// `layers × kv_heads × (key_len + value_len) × 2` — two bytes because the cache
/// is f16, and `kv_heads` rather than `heads` because grouped-query attention
/// stores one K/V pair per GROUP. Measured against the files on disk: qwen3-4b
/// (36 layers, 8 kv heads, 128+128) comes to 0.56 GB at 4096 tokens and 4.5 GB
/// at 32768, which is why the window cannot simply be set to the 262144 the
/// model declares.
///
/// ARITHMETIC, NOT A BENCHMARK. The alternative was running inference at each
/// candidate size and watching memory, which on a 24 GB machine with a 4B model
/// and parallel builds took the machine down twice. The cache size is a product
/// of four integers the file states; measuring it is theatre.
///
/// `None` when any of the four is missing — the caller then keeps the default
/// window rather than dividing by a number it invented.
pub fn gguf_kv_bytes_per_token(path: &Path) -> Option<usize> {
    let raw = read_metadata(path, false).ok()?;
    let layers = usize::try_from(raw.block_count?).ok()?;
    let kv_heads = usize::try_from(raw.kv_head_count?).ok()?;
    // `key_length`/`value_length` when the file states them; otherwise the
    // standard derivation `embedding / heads`. MEASURED: qwen2.5-3b carries
    // neither key nor value length, so without this fallback it silently kept
    // the 4096 floor while every other model on the same disk got four times
    // that — the kind of gap that looks like the feature simply not working.
    let head_dim = |stated: Option<u64>| -> Option<usize> {
        match stated {
            Some(v) => usize::try_from(v).ok(),
            None => {
                let embedding = usize::try_from(raw.embedding_length?).ok()?;
                let heads = usize::try_from(raw.head_count?).ok()?;
                (heads > 0).then(|| embedding / heads).filter(|d| *d > 0)
            }
        }
    };
    let key = head_dim(raw.key_length)?;
    let value = head_dim(raw.value_length)?;
    let per_token = layers
        .checked_mul(kv_heads)?
        .checked_mul(key.checked_add(value)?)?
        .checked_mul(2)?;
    (per_token > 0).then_some(per_token)
}

/// The context window THE MODEL ITSELF DECLARES, from its GGUF metadata.
///
/// WHY THIS EXISTS: until now the context budget was a fixed 4096 (see
/// `token::CONTEXT_BUDGET`) — a number INHERITED FROM iOS, where Apple
/// FoundationModels really does hand out a 4096-token window. Running its own
/// weights through candle, this binary has no such limit, and the models on disk
/// declare far more: qwen3-4b 262144, gemma3-4b 131072, qwen2.5-3b 32768,
/// qwen3-8b 40960 (all read from the files with this reader). Keeping the iOS
/// number cost real context: with an 11-tool catalog the `<tools>` block alone is
/// 1559 tokens and the whole prompt 2586, leaving ~1100 for the actual
/// conversation — the user saw "2 older turns left the window" every single turn.
///
/// SAME COST AS `gguf_has_tokenizer`: only the metadata block is walked, the big
/// arrays are skipped, the tensor section is never addressed.
///
/// `None` means "the file does not say" — a missing key, an unreadable file, a
/// value of a type we cannot read. The caller must then keep the default window
/// (see `token::context_budget`); guessing a window a model may not have would
/// produce positions past its rope table, i.e. plausible-looking nonsense.
pub fn gguf_context_length(path: &Path) -> Option<usize> {
    read_metadata(path, false)
        .ok()?
        .context_length
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
}

// ---------------------------------------------------------------------------
// Building a `tokenizers::Tokenizer`
// ---------------------------------------------------------------------------

/// Appends `text` to `out` as a JSON string, escaping included.
///
/// The escaping is left to `serde_json` rather than hand-written: control
/// characters and lone quotes DO occur in a byte-level vocabulary, and a
/// hand-rolled escaper that is wrong for one token in 151k produces exactly the
/// silent, single-token corruption this module exists to prevent. `buffer` is
/// reused so this stays one allocation for the whole vocabulary.
#[cfg(feature = "candle")]
fn push_json_string(out: &mut String, buffer: &mut Vec<u8>, text: &str) {
    buffer.clear();
    serde_json::to_writer(&mut *buffer, text).expect("serializing a &str cannot fail");
    out.push_str(std::str::from_utf8(buffer).expect("serde_json emits utf-8"));
}

/// Builds a `tokenizers::Tokenizer` from the tokenizer inside a GGUF file.
///
/// WHY IT GOES THROUGH JSON: the target is to be INDISTINGUISHABLE from the
/// `tokenizer.json` sitting next to the weights, and the surest way to get there
/// is to hand `tokenizers` the very document shape it would have read from that
/// file. Assembling the same object through the builder API would mean
/// re-deriving each component's defaults by hand (`ByteLevel`'s defaults are
/// `add_prefix_space = true, use_regex = true` — both WRONG for Qwen and both
/// silent) and the difference would only show up as a few shifted token ids.
///
/// The document is built once at load time, next to a 2.5 GB weight read.
#[cfg(feature = "candle")]
pub fn tokenizer_from_gguf(path: &Path) -> EngineResult<tokenizers::Tokenizer> {
    use std::str::FromStr;

    let raw = read_metadata(path, true)?;

    let model = raw.model.as_deref().unwrap_or("");
    if model != SUPPORTED_VOCAB_MODEL {
        return Err(EngineError::Tokenization(format!(
            "the tokenizer in the gguf is of type '{model}'; only '{SUPPORTED_VOCAB_MODEL}' \
             (byte-level BPE) can be rebuilt — supply a tokenizer.json for this model"
        )));
    }
    let pre = raw.pre.as_deref().unwrap_or("");
    let regex = split_regex(pre).ok_or_else(|| {
        EngineError::Tokenization(format!(
            "unsupported gguf pre-tokenizer: '{pre}' (supported: qwen2) — supply a tokenizer.json \
             for this model"
        ))
    })?;

    if raw.tokens.len() != raw.token_types.len() {
        return Err(EngineError::Tokenization(format!(
            "gguf tokens ({}) and token_type ({}) have different lengths",
            raw.tokens.len(),
            raw.token_types.len()
        )));
    }
    if raw.tokens.is_empty() {
        return Err(EngineError::Tokenization(
            "the gguf tokenizer vocabulary is empty".into(),
        ));
    }

    // --- Splitting the vocabulary into three zones -------------------------
    //
    // `tokenizers` DOES NOT honour the `id` field of an added token: on load it
    // hands out ids sequentially starting at the BPE vocabulary size (verified
    // in tokenizers-0.22.2, `AddedVocabulary::add_tokens`). So the layout has to
    // be exactly: NORMAL tokens first, then the added ones, then the unused
    // padding. Any other order and the ids we produce SILENTLY SLIDE against the
    // model's output layer — the model would keep generating, just the wrong
    // words. Hence this is checked rather than assumed.
    let mut normal_count = 0usize;
    let mut added_count = 0usize;
    // The zone number must never DECREASE as ids grow: normal (0), then added
    // (1), then unused (2).
    let mut zone_so_far = 0u8;
    for (id, kind) in raw.token_types.iter().copied().enumerate() {
        let zone: u8 = match kind {
            token_type::NORMAL => 0,
            token_type::CONTROL | token_type::USER_DEFINED => 1,
            token_type::UNUSED => 2,
            other => {
                return Err(EngineError::Tokenization(format!(
                    "unknown token_type {other} at id {id} ('{}')",
                    raw.tokens[id]
                )));
            }
        };
        if zone < zone_so_far {
            return Err(EngineError::Tokenization(format!(
                "the gguf vocabulary is not laid out as normal/added/unused: id {id} \
                 ('{}') has token_type {kind}",
                raw.tokens[id]
            )));
        }
        zone_so_far = zone;
        match zone {
            0 => normal_count += 1,
            1 => added_count += 1,
            _ => {}
        }
    }

    // --- The tokenizer.json document ---------------------------------------
    //
    // Every flag below is COPIED FROM the real Qwen3 tokenizer.json, not taken
    // from a default:
    //   normalizer     NFC
    //   pre_tokenizer  Split(regex, Isolated) then ByteLevel with
    //                  add_prefix_space=false, trim_offsets=false, use_regex=false
    //   post_processor / decoder  the same ByteLevel flags
    //   model          BPE, no unk, byte_fallback=false, ignore_merges=false
    // ByteLevel is MANDATORY on both ends: without it a space arrives as the raw
    // marker `Ġ` and Turkish letters come apart at their byte boundaries.
    let mut json = String::with_capacity(raw.tokens.len() * 24 + raw.merges.len() * 16 + 4096);
    let mut buffer = Vec::with_capacity(64);

    json.push_str("{\"version\":\"1.0\",\"truncation\":null,\"padding\":null,\"added_tokens\":[");
    for (offset, id) in (normal_count..normal_count + added_count).enumerate() {
        if offset > 0 {
            json.push(',');
        }
        json.push_str("{\"id\":");
        json.push_str(&id.to_string());
        json.push_str(",\"content\":");
        push_json_string(&mut json, &mut buffer, &raw.tokens[id]);
        // `special` only changes what `decode(skip_special_tokens = true)` drops
        // and what `is_special_token` answers (the stop-token lookup) — it does
        // NOT affect encoding: `AddedVocabulary` puts special and ordinary added
        // tokens into the SAME match trie, partitioned by `normalized`, not by
        // `special`. CONTROL maps to special, USER_DEFINED does not.
        let special = raw.token_types[id] == token_type::CONTROL;
        json.push_str(
            ",\"single_word\":false,\"lstrip\":false,\"rstrip\":false,\"normalized\":false,\"special\":",
        );
        json.push_str(if special { "true}" } else { "false}" });
    }
    json.push_str("],\"normalizer\":{\"type\":\"NFC\"},\"pre_tokenizer\":{\"type\":\"Sequence\",\"pretokenizers\":[{\"type\":\"Split\",\"pattern\":{\"Regex\":");
    push_json_string(&mut json, &mut buffer, regex);
    json.push_str("},\"behavior\":\"Isolated\",\"invert\":false},{\"type\":\"ByteLevel\",\"add_prefix_space\":false,\"trim_offsets\":false,\"use_regex\":false}]}");
    json.push_str(",\"post_processor\":{\"type\":\"ByteLevel\",\"add_prefix_space\":false,\"trim_offsets\":false,\"use_regex\":false}");
    json.push_str(",\"decoder\":{\"type\":\"ByteLevel\",\"add_prefix_space\":false,\"trim_offsets\":false,\"use_regex\":false}");
    json.push_str(",\"model\":{\"type\":\"BPE\",\"dropout\":null,\"unk_token\":null,\"continuing_subword_prefix\":\"\",\"end_of_word_suffix\":\"\",\"fuse_unk\":false,\"byte_fallback\":false,\"ignore_merges\":false,\"vocab\":{");
    for id in 0..normal_count {
        if id > 0 {
            json.push(',');
        }
        push_json_string(&mut json, &mut buffer, &raw.tokens[id]);
        json.push(':');
        json.push_str(&id.to_string());
    }
    // The merges are handed over in GGUF's own "left right" form. `tokenizers`
    // accepts that spelling and splits it itself, with a length check that
    // reports the offending line — better than splitting it here and inventing
    // our own error for the same case.
    json.push_str("},\"merges\":[");
    for (rank, merge) in raw.merges.iter().enumerate() {
        if rank > 0 {
            json.push(',');
        }
        push_json_string(&mut json, &mut buffer, merge);
    }
    json.push_str("]}}");

    let tokenizer = tokenizers::Tokenizer::from_str(&json).map_err(|e| {
        EngineError::Tokenization(format!(
            "could not build the tokenizer inside the gguf: {e}"
        ))
    })?;

    // --- Post-conditions ---------------------------------------------------
    //
    // A DUPLICATE token in the vocabulary would collapse two JSON keys into one,
    // every id after it would shift by one, and nothing would report it. The
    // sizes are the cheapest place that discrepancy becomes visible.
    let base = tokenizer.get_vocab_size(false);
    let total = tokenizer.get_vocab_size(true);
    if base != normal_count || total != normal_count + added_count {
        return Err(EngineError::Tokenization(format!(
            "the rebuilt vocabulary does not match the gguf: expected {normal_count}+{added_count}, \
             got {base}+{}",
            total.saturating_sub(base)
        )));
    }
    Ok(tokenizer)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal in-memory GGUF header, so the reader can be measured without a
    /// 2.5 GB file. Only the metadata block is written — which is precisely all
    /// this module ever reads.
    struct Builder {
        bytes: Vec<u8>,
        count: u64,
    }

    impl Builder {
        fn new() -> Self {
            Self {
                bytes: Vec::new(),
                count: 0,
            }
        }
        fn raw_string(&mut self, s: &str) {
            self.bytes
                .extend_from_slice(&(s.len() as u64).to_le_bytes());
            self.bytes.extend_from_slice(s.as_bytes());
        }
        fn string_kv(&mut self, key: &str, value: &str) {
            self.raw_string(key);
            self.bytes.extend_from_slice(&T_STRING.to_le_bytes());
            self.raw_string(value);
            self.count += 1;
        }
        fn u32_kv(&mut self, key: &str, value: u32) {
            self.raw_string(key);
            self.bytes.extend_from_slice(&T_U32.to_le_bytes());
            self.bytes.extend_from_slice(&value.to_le_bytes());
            self.count += 1;
        }
        fn string_array_kv(&mut self, key: &str, values: &[&str]) {
            self.raw_string(key);
            self.bytes.extend_from_slice(&T_ARRAY.to_le_bytes());
            self.bytes.extend_from_slice(&T_STRING.to_le_bytes());
            self.bytes
                .extend_from_slice(&(values.len() as u64).to_le_bytes());
            for v in values {
                self.raw_string(v);
            }
            self.count += 1;
        }
        fn int_array_kv(&mut self, key: &str, values: &[i32]) {
            self.raw_string(key);
            self.bytes.extend_from_slice(&T_ARRAY.to_le_bytes());
            self.bytes.extend_from_slice(&T_I32.to_le_bytes());
            self.bytes
                .extend_from_slice(&(values.len() as u64).to_le_bytes());
            for v in values {
                self.bytes.extend_from_slice(&v.to_le_bytes());
            }
            self.count += 1;
        }
        /// Writes the file out: magic + version + tensor count + kv count + body,
        /// plus a fake "tensor section" so the reader can be shown to stop before
        /// it.
        fn finish(self, tensor_tail: usize) -> Vec<u8> {
            let mut out = Vec::new();
            out.extend_from_slice(b"GGUF");
            out.extend_from_slice(&3u32.to_le_bytes());
            out.extend_from_slice(&0u64.to_le_bytes());
            out.extend_from_slice(&self.count.to_le_bytes());
            out.extend_from_slice(&self.bytes);
            out.extend(std::iter::repeat_n(0xABu8, tensor_tail));
            out
        }
    }

    fn write_temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("tacet-gguf-{name}-{}", std::process::id()));
        std::fs::write(&path, bytes).expect("temp file");
        path
    }

    fn tiny_gguf() -> Builder {
        let mut b = Builder::new();
        b.string_kv("general.architecture", "qwen3");
        b.string_kv(KEY_MODEL, "gpt2");
        b.string_kv(KEY_PRE, "qwen2");
        // The same shape as the real Qwen3 file: normal tokens, then a CONTROL
        // token and a USER_DEFINED one, then the unused padding. The two added
        // kinds differ only in `special` — which is exactly the distinction the
        // stop-token lookup rests on.
        b.string_array_kv(
            KEY_TOKENS,
            &["a", "b", "ab", "<|im_end|>", "<tool_call>", "[PAD]"],
        );
        b.int_array_kv(
            KEY_TOKEN_TYPE,
            &[
                token_type::NORMAL,
                token_type::NORMAL,
                token_type::NORMAL,
                token_type::CONTROL,
                token_type::USER_DEFINED,
                token_type::UNUSED,
            ],
        );
        b.string_array_kv(KEY_MERGES, &["a b"]);
        b.u32_kv("tokenizer.ggml.eos_token_id", 3);
        // The real key from ~/models/qwen3-4b/model.gguf, value included.
        b.u32_kv("qwen3.context_length", 262_144);
        b
    }

    #[test]
    fn the_metadata_reader_finds_the_tokenizer_keys() {
        let path = write_temp("ok", &tiny_gguf().finish(0));
        let raw = read_metadata(&path, true).expect("read");
        assert_eq!(raw.model.as_deref(), Some("gpt2"));
        assert_eq!(raw.pre.as_deref(), Some("qwen2"));
        assert_eq!(
            raw.tokens,
            ["a", "b", "ab", "<|im_end|>", "<tool_call>", "[PAD]"]
        );
        assert_eq!(raw.merges, ["a b"]);
        assert!(raw.is_usable());
        let _ = std::fs::remove_file(&path);
    }

    /// THE POINT OF THE CHEAP CHECK: the tensor section is never touched. A 4 MB
    /// tail is appended and the reader is required to stop at the end of the
    /// metadata block — position, not the file size.
    #[test]
    fn the_cheap_check_does_not_read_the_tensor_section() {
        let tail = 4 * 1024 * 1024;
        let bytes = tiny_gguf().finish(tail);
        let path = write_temp("tail", &bytes);
        assert!(gguf_has_tokenizer(&path));

        let file = File::open(&path).unwrap();
        let length = file.metadata().unwrap().len();
        let mut reader = MetaReader::new(BufReader::new(file), length);
        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic).unwrap();
        let _ = reader.u32();
        let _ = reader.u64();
        let kv = reader.u64().unwrap();
        for _ in 0..kv {
            let _ = reader.string().unwrap();
            let kind = reader.u32().unwrap();
            reader.skip_value(kind).unwrap();
        }
        assert_eq!(
            reader.position,
            length - tail as u64,
            "the reader went past the end of the metadata"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_foreign_or_broken_file_is_not_claimed_to_have_a_tokenizer() {
        // Not a GGUF at all.
        let path = write_temp("junk", b"not a gguf file at all, just bytes");
        assert!(!gguf_has_tokenizer(&path));
        let _ = std::fs::remove_file(&path);

        // A GGUF with a sentencepiece vocabulary: real, but not one we can
        // rebuild — the answer must be "no", not a crash and not a lie.
        let mut b = Builder::new();
        b.string_kv(KEY_MODEL, "llama");
        b.string_array_kv(KEY_TOKENS, &["a"]);
        b.int_array_kv(KEY_TOKEN_TYPE, &[token_type::NORMAL]);
        let path = write_temp("spm", &b.finish(0));
        assert!(!gguf_has_tokenizer(&path));
        let _ = std::fs::remove_file(&path);

        // A GGUF whose pre-tokenizer name we have never verified.
        let mut b = Builder::new();
        b.string_kv(KEY_MODEL, "gpt2");
        b.string_kv(KEY_PRE, "llama3");
        b.string_array_kv(KEY_TOKENS, &["a"]);
        b.int_array_kv(KEY_TOKEN_TYPE, &[token_type::NORMAL]);
        b.string_array_kv(KEY_MERGES, &[]);
        let path = write_temp("pre", &b.finish(0));
        assert!(!gguf_has_tokenizer(&path));
        let _ = std::fs::remove_file(&path);

        // A file that does not exist.
        assert!(!gguf_has_tokenizer(Path::new("/definitely/not/here.gguf")));
    }

    /// A corrupt length must come back as an error, NOT as a multi-gigabyte
    /// allocation that takes the process down.
    #[test]
    fn a_corrupt_length_errors_instead_of_allocating() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"GGUF");
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());
        // A key name claiming to be 2^60 bytes long.
        bytes.extend_from_slice(&(1u64 << 60).to_le_bytes());
        let path = write_temp("corrupt", &bytes);
        let error = read_metadata(&path, true).unwrap_err().to_string();
        assert!(error.contains("corrupt"), "{error}");
        assert!(!gguf_has_tokenizer(&path));
        let _ = std::fs::remove_file(&path);
    }

    /// The window is read from the ARCHITECTURE-PREFIXED key, and the cheap
    /// (arrays-skipped) scan finds it too — that is the mode discovery runs in.
    #[test]
    fn the_context_length_is_read_without_hard_coding_the_architecture() {
        let path = write_temp("ctx", &tiny_gguf().finish(0));
        assert_eq!(gguf_context_length(&path), Some(262_144));
        let _ = std::fs::remove_file(&path);

        // A DIFFERENT architecture name, same suffix: no code change needed.
        let mut b = Builder::new();
        b.string_kv("general.architecture", "gemma3");
        b.u32_kv("gemma3.context_length", 131_072);
        let path = write_temp("ctx-gemma", &b.finish(0));
        assert_eq!(gguf_context_length(&path), Some(131_072));
        let _ = std::fs::remove_file(&path);
    }

    /// `rope.scaling.original_context_length` is the window BEFORE rope scaling —
    /// a smaller, different number. Matching on the suffix alone would pick it up
    /// and quietly shrink the budget, so the prefix has to be one segment.
    #[test]
    fn the_pre_scaling_window_is_not_mistaken_for_the_real_one() {
        let mut b = Builder::new();
        b.string_kv("general.architecture", "phi3");
        b.u32_kv("phi3.rope.scaling.original_context_length", 4096);
        b.u32_kv("phi3.context_length", 131_072);
        let path = write_temp("ctx-rope", &b.finish(0));
        assert_eq!(gguf_context_length(&path), Some(131_072));
        let _ = std::fs::remove_file(&path);
    }

    /// A file that does not declare a window answers "I do not know" rather than
    /// a number — the caller keeps the default rather than inventing one.
    #[test]
    fn a_file_without_a_window_says_so_instead_of_guessing() {
        let mut b = Builder::new();
        b.string_kv("general.architecture", "qwen3");
        let path = write_temp("ctx-none", &b.finish(0));
        assert_eq!(gguf_context_length(&path), None);
        let _ = std::fs::remove_file(&path);

        // A value of the wrong TYPE is stepped over, not misread as bytes.
        let mut b = Builder::new();
        b.string_kv("qwen3.context_length", "not a number");
        b.string_kv("general.architecture", "qwen3");
        let path = write_temp("ctx-text", &b.finish(0));
        assert_eq!(gguf_context_length(&path), None);
        let _ = std::fs::remove_file(&path);

        assert_eq!(
            gguf_context_length(Path::new("/definitely/not/here.gguf")),
            None
        );
    }

    #[test]
    fn an_unknown_pre_tokenizer_name_does_not_fall_back() {
        assert!(split_regex("qwen2").is_some());
        for name in ["llama3", "gpt-2", "default", "", "QWEN2"] {
            assert!(split_regex(name).is_none(), "'{name}' was accepted");
        }
    }

    #[cfg(feature = "candle")]
    #[test]
    fn a_tiny_gguf_becomes_a_working_tokenizer() {
        let path = write_temp("build", &tiny_gguf().finish(0));
        let tokenizer = tokenizer_from_gguf(&path).expect("build");
        // 3 normal + 2 added; the UNUSED padding is dropped, exactly as
        // tokenizer.json would.
        assert_eq!(tokenizer.get_vocab_size(false), 3);
        assert_eq!(tokenizer.get_vocab_size(true), 5);
        assert_eq!(tokenizer.token_to_id("<|im_end|>"), Some(3));
        assert_eq!(tokenizer.token_to_id("<tool_call>"), Some(4));
        // CONTROL is special (the stop-token lookup finds it), USER_DEFINED is
        // not — the same split the real tokenizer.json makes.
        let added = tokenizer.get_added_vocabulary();
        assert!(added.is_special_token("<|im_end|>"));
        assert!(!added.is_special_token("<tool_call>"));
        // BOTH are matched whole when encoding, special or not.
        assert_eq!(
            tokenizer.encode("<tool_call>", true).unwrap().get_ids(),
            [4]
        );
        // The merge "a b" really applies: "ab" is ONE token, not two.
        let ids = tokenizer.encode("ab", true).unwrap().get_ids().to_vec();
        assert_eq!(ids, vec![2]);
        let _ = std::fs::remove_file(&path);
    }

    /// A vocabulary whose zones are out of order must ERROR. Accepted silently,
    /// every id after the stray token slides by one and the model produces
    /// plausible-looking nonsense.
    #[cfg(feature = "candle")]
    #[test]
    fn an_out_of_order_vocabulary_is_refused() {
        let mut b = Builder::new();
        b.string_kv(KEY_MODEL, "gpt2");
        b.string_kv(KEY_PRE, "qwen2");
        b.string_array_kv(KEY_TOKENS, &["a", "<|im_end|>", "b"]);
        b.int_array_kv(
            KEY_TOKEN_TYPE,
            &[token_type::NORMAL, token_type::CONTROL, token_type::NORMAL],
        );
        b.string_array_kv(KEY_MERGES, &[]);
        let path = write_temp("order", &b.finish(0));
        let error = tokenizer_from_gguf(&path).unwrap_err().to_string();
        assert!(error.contains("normal/added/unused"), "{error}");
        let _ = std::fs::remove_file(&path);
    }
}
