//! Zip reading — STORE + DEFLATE.
//!
//! Design rule: NO path leads to a panic. The input may be an .xlsx that came
//! from the user (or from someone malicious); in the Swift version malformed
//! headers went straight into array indexing and could produce a crash. Here
//! every field is read with the bounds-checked `byte::` helpers, every numeric
//! conversion is checked, and every decoded body is subject to a cap.

use crate::byte::{read16, read32, slice};
use crate::crc32::crc32;
use crate::error::{ZipError, ZipResult};
use crate::inflate::inflate;
use crate::writer::{EOCD_SIG, ZipEntry};
use std::collections::BTreeMap;

const CENTRAL_SIG: u32 = 0x0201_4b50;
const LOCAL_SIG: u32 = 0x0403_4b50;

/// The upper bound on the decoded form of a single entry (64 MiB).
///
/// Among OOXML parts the largest is typically sheet1.xml and it does not go past
/// a few MB; 64 MiB is a comfortable cap yet low enough to be useless to a zip bomb.
pub const ENTRY_CAP: usize = 64 * 1024 * 1024;

/// The total decoded size limit for the whole archive (256 MiB).
pub const ARCHIVE_CAP: usize = 256 * 1024 * 1024;

/// The maximum number of bytes searched backwards for the EOCD: a 22-byte fixed
/// record + at most 64 KiB of comment (the comment length is a u16).
const EOCD_SEARCH_WINDOW: usize = 22 + 65_536;

/// Opens the archive and returns the entries in file order.
pub fn open(zip: &[u8]) -> ZipResult<Vec<ZipEntry>> {
    let eocd = find_eocd(zip)?;

    let record_count = read16(zip, eocd + 10)? as usize;
    let central_offset = read32(zip, eocd + 16)? as usize;

    // The central directory cannot start after the EOCD; this check filters out
    // both malformed archives and specially crafted ones (where the offset winds
    // the data backwards).
    if central_offset > eocd {
        return Err(ZipError::Malformed("the central directory offset passes the EOCD"));
    }

    let mut entries = Vec::with_capacity(record_count.min(1024));
    let mut total_decoded = 0usize;
    let mut offset = central_offset;

    for _ in 0..record_count {
        let record = read_central_record(zip, offset)?;
        let data = decode_entry_data(zip, &record)?;

        total_decoded += data.len();
        if total_decoded > ARCHIVE_CAP {
            return Err(ZipError::LimitExceeded("the archive total size cap was exceeded"));
        }

        entries.push(ZipEntry {
            name: record.name,
            data,
        });
        offset = record.next_offset;
    }

    Ok(entries)
}

/// Opens the archive as a name -> content map. If the same name occurs more than
/// once, the LAST one wins (zip semantics: a later record shadows an earlier one).
pub fn open_map(zip: &[u8]) -> ZipResult<BTreeMap<String, Vec<u8>>> {
    Ok(open(zip)?
        .into_iter()
        .map(|e| (e.name, e.data))
        .collect())
}

/// Searches for the EOCD signature backwards from the end.
fn find_eocd(zip: &[u8]) -> ZipResult<usize> {
    if zip.len() < 22 {
        return Err(ZipError::Malformed("the file is too short for an EOCD record"));
    }
    let latest = zip.len() - 22;
    let earliest = latest.saturating_sub(EOCD_SEARCH_WINDOW);
    let mut i = latest;
    loop {
        if read32(zip, i)? == EOCD_SIG {
            // The comment length must agree with the end of the file; if it does
            // not, this signature is a false positive found inside the data, so
            // keep searching.
            let comment_length = read16(zip, i + 20)? as usize;
            if i + 22 + comment_length == zip.len() {
                return Ok(i);
            }
        }
        if i == earliest {
            return Err(ZipError::Malformed("no EOCD record found"));
        }
        i -= 1;
    }
}

/// The fields extracted from a central directory record, enough to locate the data.
struct CentralDirectory {
    name: String,
    method: u16,
    compressed_size: usize,
    raw_size: usize,
    crc: u32,
    local_offset: usize,
    /// The offset of the next central directory record.
    next_offset: usize,
}

fn read_central_record(zip: &[u8], offset: usize) -> ZipResult<CentralDirectory> {
    if read32(zip, offset)? != CENTRAL_SIG {
        return Err(ZipError::Malformed("wrong central directory record signature"));
    }
    let method = read16(zip, offset + 10)?;
    let crc = read32(zip, offset + 16)?;
    let compressed_size = read32(zip, offset + 20)? as usize;
    let raw_size = read32(zip, offset + 24)? as usize;
    let name_length = read16(zip, offset + 28)? as usize;
    let extra_length = read16(zip, offset + 30)? as usize;
    let comment_length = read16(zip, offset + 32)? as usize;
    let local_offset = read32(zip, offset + 42)? as usize;

    let name_bytes = slice(zip, offset + 46, name_length)?;
    // Invalid UTF-8 is not reason enough to reject the archive (old tools write
    // CP437); decoding lossily keeps the file openable.
    let name = String::from_utf8_lossy(name_bytes).into_owned();

    let next_offset = offset
        .checked_add(46)
        .and_then(|v| v.checked_add(name_length))
        .and_then(|v| v.checked_add(extra_length))
        .and_then(|v| v.checked_add(comment_length))
        .ok_or(ZipError::Malformed("the central directory offset overflowed"))?;

    Ok(CentralDirectory {
        name,
        method,
        compressed_size,
        raw_size,
        crc,
        local_offset,
        next_offset,
    })
}

fn decode_entry_data(zip: &[u8], record: &CentralDirectory) -> ZipResult<Vec<u8>> {
    // The data start is read from the local header: the name/extra lengths in the
    // central directory may DIFFER from the ones in the local header (a
    // legitimate situation).
    let local = record.local_offset;
    if read32(zip, local)? != LOCAL_SIG {
        return Err(ZipError::Malformed("wrong local file header signature"));
    }
    let local_name_length = read16(zip, local + 26)? as usize;
    let local_extra_length = read16(zip, local + 28)? as usize;
    let data_start = local
        .checked_add(30)
        .and_then(|v| v.checked_add(local_name_length))
        .and_then(|v| v.checked_add(local_extra_length))
        .ok_or(ZipError::Malformed("the local data offset overflowed"))?;

    let raw = slice(zip, data_start, record.compressed_size)?;

    let decoded = match record.method {
        0 => {
            if record.compressed_size != record.raw_size {
                return Err(ZipError::Malformed("the sizes of a STORE entry do not match"));
            }
            raw.to_vec()
        }
        8 => {
            // The declared raw_size is NOT TRUSTED; the cap is applied independently.
            inflate(raw, ENTRY_CAP)?
        }
        method => return Err(ZipError::UnsupportedMethod(method)),
    };

    // If the CRC is zero (some streaming writers leave it to the data
    // descriptor) it is skipped; otherwise this is the only real shield against
    // silent data corruption.
    if record.crc != 0 && crc32(&decoded) != record.crc {
        return Err(ZipError::CrcMismatch {
            name: record.name.clone(),
        });
    }

    Ok(decoded)
}
