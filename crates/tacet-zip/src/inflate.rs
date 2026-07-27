//! Hand-written DEFLATE decoder (RFC 1951): stored blocks + fixed Huffman +
//! dynamic Huffman.
//!
//! Why by hand: Tacet's identity is to pull in no ready-made crates; and once
//! the decoder is ours, WE set the limit on "how much output may be produced" —
//! the only real defence against a zip bomb (see `max_output`).
//!
//! The decoding strategy is a bit-by-bit canonical Huffman walk instead of a
//! code table (the puff approach): OOXML parts are small, speed is not the
//! bottleneck; whereas a mistake made while building a table produces silently
//! wrong output. What is simple is what can be verified.

use crate::error::{ZipError, ZipResult};

/// LSB-first bit reader. Every read is bounds-checked.
struct BitReader<'a> {
    data: &'a [u8],
    /// The absolute position of the next bit to read (byte * 8 + bit).
    position: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        BitReader { data, position: 0 }
    }

    fn bit(&mut self) -> ZipResult<u32> {
        let byte_index = self.position >> 3;
        let byte = *self
            .data
            .get(byte_index)
            .ok_or(ZipError::Malformed("the deflate stream ended unexpectedly"))?;
        let value = (byte >> (self.position & 7)) & 1;
        self.position += 1;
        Ok(value as u32)
    }

    /// Reads `count` bits LSB-first (count <= 16).
    fn bits(&mut self, count: u32) -> ZipResult<u32> {
        let mut value = 0u32;
        for i in 0..count {
            value |= self.bit()? << i;
        }
        Ok(value)
    }

    /// Aligns to a byte boundary (the start of a stored block).
    fn align(&mut self) {
        self.position = (self.position + 7) & !7;
    }

    fn byte_position(&self) -> usize {
        self.position >> 3
    }

    fn seek_byte(&mut self, target: usize) {
        self.position = target << 3;
    }
}

/// A canonical Huffman code table: code count per length + sorted symbols.
struct Huffman {
    /// counts[l] = the number of codes of length l bits (l = 0..=15).
    counts: [u16; 16],
    /// Symbols sorted first by length, then by symbol value.
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> ZipResult<Huffman> {
        let mut counts = [0u16; 16];
        for &l in lengths {
            if l > 15 {
                return Err(ZipError::Malformed("a huffman code length exceeds 15"));
            }
            counts[l as usize] += 1;
        }
        // Entries with length 0 are unused symbols; if all are 0 the table is empty.
        if counts[0] as usize == lengths.len() {
            return Err(ZipError::Malformed("empty huffman table"));
        }

        // Code-space overflow check: incomplete tables are legitimate in deflate
        // only for a single-symbol distance table; an over-subscribed table is
        // always malformed.
        let mut left = 1i32;
        for &count in counts.iter().skip(1) {
            left = left.saturating_mul(2) - count as i32;
            if left < 0 {
                return Err(ZipError::Malformed("over-subscribed huffman table"));
            }
        }

        let mut offsets = [0u16; 16];
        for l in 1..15 {
            offsets[l + 1] = offsets[l] + counts[l];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (symbol, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[offsets[l as usize] as usize] = symbol as u16;
                offsets[l as usize] += 1;
            }
        }
        Ok(Huffman { counts, symbols })
    }

    /// Decodes the canonical code by advancing bit by bit.
    fn decode(&self, br: &mut BitReader<'_>) -> ZipResult<u16> {
        let mut code = 0i32;
        let mut first = 0i32;
        let mut index = 0i32;
        for l in 1..16 {
            code |= br.bit()? as i32;
            let count = self.counts[l] as i32;
            if code - count < first {
                let slot = (index + (code - first)) as usize;
                return self
                    .symbols
                    .get(slot)
                    .copied()
                    .ok_or(ZipError::Malformed("huffman symbol index out of bounds"));
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err(ZipError::Malformed("invalid huffman code"))
    }
}

// RFC 1951, 3.2.5 — the length and distance tables.
const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u32; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DISTANCE_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DISTANCE_EXTRA: [u32; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
/// The order of the code-length alphabet in the stream (RFC 1951, 3.2.7).
const CODE_LENGTH_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// Decodes a raw DEFLATE stream.
///
/// `max_output` is a hard cap: exceeding it returns `LimitExceeded`. The
/// declared size (in the header) is NOT TRUSTED — that is exactly where a zip
/// bomb lies.
pub fn inflate(data: &[u8], max_output: usize) -> ZipResult<Vec<u8>> {
    let mut br = BitReader::new(data);
    let mut output: Vec<u8> = Vec::new();

    loop {
        let last_block = br.bit()? == 1;
        let kind = br.bits(2)?;
        match kind {
            0 => stored_block(&mut br, &mut output, max_output)?,
            1 => {
                let (lit, dist) = fixed_tables();
                huffman_block(&mut br, &mut output, &lit, &dist, max_output)?;
            }
            2 => {
                let (lit, dist) = dynamic_tables(&mut br)?;
                huffman_block(&mut br, &mut output, &lit, &dist, max_output)?;
            }
            _ => return Err(ZipError::Malformed("invalid deflate block kind")),
        }
        if last_block {
            break;
        }
    }
    Ok(output)
}

fn stored_block(br: &mut BitReader<'_>, output: &mut Vec<u8>, max_output: usize) -> ZipResult<()> {
    br.align();
    let start = br.byte_position();
    let length = crate::byte::read16(br.data, start)? as usize;
    let inverse = crate::byte::read16(br.data, start + 2)? as usize;
    // The LEN / ~LEN consistency catches a malformed stream early.
    if length != (!inverse & 0xFFFF) {
        return Err(ZipError::Malformed("inconsistent stored block length"));
    }
    let body = crate::byte::slice(br.data, start + 4, length)?;
    if output.len() + length > max_output {
        return Err(ZipError::LimitExceeded("decoded size cap exceeded"));
    }
    output.extend_from_slice(body);
    br.seek_byte(start + 4 + length);
    Ok(())
}

fn fixed_tables() -> (Huffman, Huffman) {
    let mut lit_lengths = [0u8; 288];
    for (i, l) in lit_lengths.iter_mut().enumerate() {
        *l = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    // The fixed tables are valid by definition; new() never returns Err here.
    let lit = Huffman::new(&lit_lengths).expect("the fixed literal table is valid");
    let dist = Huffman::new(&[5u8; 30]).expect("the fixed distance table is valid");
    (lit, dist)
}

fn dynamic_tables(br: &mut BitReader<'_>) -> ZipResult<(Huffman, Huffman)> {
    let hlit = br.bits(5)? as usize + 257;
    let hdist = br.bits(5)? as usize + 1;
    let hclen = br.bits(4)? as usize + 4;
    if hlit > 286 || hdist > 30 {
        return Err(ZipError::Malformed(
            "a dynamic table counter exceeds the alphabet",
        ));
    }

    let mut code_lengths = [0u8; 19];
    for &slot in CODE_LENGTH_ORDER.iter().take(hclen) {
        code_lengths[slot] = br.bits(3)? as u8;
    }
    let code_table = Huffman::new(&code_lengths)?;

    // The literal and distance lengths are encoded in a SINGLE stream; the
    // 16/17/18 repeat commands may cross the boundary between the two alphabets,
    // so they are decoded together.
    let total = hlit + hdist;
    let mut lengths = vec![0u8; total];
    let mut i = 0usize;
    while i < total {
        let symbol = code_table.decode(br)?;
        match symbol {
            0..=15 => {
                lengths[i] = symbol as u8;
                i += 1;
            }
            16 => {
                // Repeat the previous length 3..6 times.
                if i == 0 {
                    return Err(ZipError::Malformed(
                        "repeat command with no preceding length",
                    ));
                }
                let previous = lengths[i - 1];
                let repeat = 3 + br.bits(2)? as usize;
                if i + repeat > total {
                    return Err(ZipError::Malformed("the length repeat overflows the table"));
                }
                lengths[i..i + repeat].fill(previous);
                i += repeat;
            }
            17 | 18 => {
                let repeat = if symbol == 17 {
                    3 + br.bits(3)? as usize
                } else {
                    11 + br.bits(7)? as usize
                };
                if i + repeat > total {
                    return Err(ZipError::Malformed("the zero repeat overflows the table"));
                }
                lengths[i..i + repeat].fill(0);
                i += repeat;
            }
            _ => return Err(ZipError::Malformed("invalid code length symbol")),
        }
    }

    if lengths[256] == 0 {
        return Err(ZipError::Malformed(
            "the end-of-block symbol is not encoded",
        ));
    }
    let lit = Huffman::new(&lengths[..hlit])?;
    // Streams with a single distance code (or that use no distance at all) are
    // legitimate; in that case the table is incomplete and Huffman::new calls an
    // empty table an Err. If distance is never used, a placeholder table is
    // enough — decode() is never called anyway.
    let dist_lengths = &lengths[hlit..];
    let dist = if dist_lengths.iter().all(|&l| l == 0) {
        Huffman::new(&[1u8, 1u8])?
    } else {
        Huffman::new(dist_lengths)?
    };
    Ok((lit, dist))
}

fn huffman_block(
    br: &mut BitReader<'_>,
    output: &mut Vec<u8>,
    lit: &Huffman,
    dist: &Huffman,
    max_output: usize,
) -> ZipResult<()> {
    loop {
        let symbol = lit.decode(br)?;
        match symbol {
            0..=255 => {
                if output.len() >= max_output {
                    return Err(ZipError::LimitExceeded("decoded size cap exceeded"));
                }
                output.push(symbol as u8);
            }
            256 => return Ok(()),
            257..=285 => {
                let idx = symbol as usize - 257;
                let length = LENGTH_BASE[idx] as usize + br.bits(LENGTH_EXTRA[idx])? as usize;

                let dist_symbol = dist.decode(br)? as usize;
                if dist_symbol >= DISTANCE_BASE.len() {
                    return Err(ZipError::Malformed("invalid distance symbol"));
                }
                let distance = DISTANCE_BASE[dist_symbol] as usize
                    + br.bits(DISTANCE_EXTRA[dist_symbol])? as usize;
                // If the back-reference points outside the window the stream is
                // malformed; leaving it unchecked would mean an index panic.
                if distance == 0 || distance > output.len() {
                    return Err(ZipError::Malformed("back-reference outside the window"));
                }
                if output.len() + length > max_output {
                    return Err(ZipError::LimitExceeded("decoded size cap exceeded"));
                }
                // Copied byte by byte: the source and target ranges may overlap
                // (distance < length), LZ77's "repeats itself" case.
                for source in (output.len() - distance..).take(length) {
                    let b = output[source];
                    output.push(b);
                }
            }
            _ => return Err(ZipError::Malformed("invalid literal/length symbol")),
        }
    }
}
