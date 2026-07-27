//! Bounds-checked byte read/write helpers.
//!
//! On the Swift side `read16`/`read32` indexed straight into the array; on a
//! malformed .xlsx that meant a crash. Here the WHOLE read path returns Result
//! and indexing only happens through slices that were validated first — the
//! panic surface is closed at type level so it never comes into being.

use crate::error::{ZipError, ZipResult};

/// Reads 2 little-endian bytes at the given offset.
pub fn read16(b: &[u8], i: usize) -> ZipResult<u16> {
    let s = b
        .get(i..i + 2)
        .ok_or(ZipError::Malformed("16-bit read out of bounds"))?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}

/// Reads 4 little-endian bytes at the given offset.
pub fn read32(b: &[u8], i: usize) -> ZipResult<u32> {
    let s = b
        .get(i..i + 4)
        .ok_or(ZipError::Malformed("32-bit read out of bounds"))?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// The `length`-byte slice starting at `i`; Err if it overflows.
pub fn slice(b: &[u8], i: usize, length: usize) -> ZipResult<&[u8]> {
    // Overflow-free addition first: i + length can overflow even in 64 bits
    // (values near usize::MAX can come from a malformed archive), and
    // checked_add closes that.
    let end = i
        .checked_add(length)
        .ok_or(ZipError::Malformed("offset sum overflowed"))?;
    b.get(i..end)
        .ok_or(ZipError::Malformed("slice out of bounds"))
}

/// Little-endian writers. On the write path the input is data we produced
/// ourselves, so there is no Result — but narrowing conversions are checked at
/// the call site.
pub fn write16(target: &mut Vec<u8>, v: u16) {
    target.extend_from_slice(&v.to_le_bytes());
}

pub fn write32(target: &mut Vec<u8>, v: u32) {
    target.extend_from_slice(&v.to_le_bytes());
}
