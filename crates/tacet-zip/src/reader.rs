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
        return Err(ZipError::Malformed(
            "the central directory offset passes the EOCD",
        ));
    }

    let mut entries = Vec::with_capacity(record_count.min(1024));
    let mut total_decoded = 0usize;
    let mut offset = central_offset;

    for _ in 0..record_count {
        let record = read_central_record(zip, offset)?;
        let data = decode_entry_data(zip, &record, ENTRY_CAP)?;

        total_decoded += data.len();
        if total_decoded > ARCHIVE_CAP {
            return Err(ZipError::LimitExceeded(
                "the archive total size cap was exceeded",
            ));
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
    Ok(open(zip)?.into_iter().map(|e| (e.name, e.data)).collect())
}

/// WHAT THE CENTRAL DIRECTORY SAYS ABOUT ONE ENTRY, with nothing decoded.
///
/// WHY THIS TYPE EXISTS AT ALL: `open` is the only way in today, and it INFLATES
/// every entry before the caller may look at a single name. For the OOXML path
/// that is right — it wants all the parts. For a caller that must decide
/// "should this archive be touched at all" it is backwards: the decision has to
/// be made from the DECLARED sizes, before any CPU is spent on a bomb.
///
/// EVERY FIELD HERE IS THE ARCHIVE'S OWN CLAIM AND NONE OF IT IS PROVEN. A zip
/// may declare 4 KiB and decode to 8 MiB; that is exactly what a lying header
/// is. Treat this as evidence to refuse on, never as a fact to rely on — the
/// proof only exists after `open_selected` has decoded and CRC-checked the data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipListing {
    pub name: String,
    /// 0 = STORE, 8 = DEFLATE. Anything else is refused at decode time.
    pub method: u16,
    pub compressed_size: usize,
    /// The size the archive CLAIMS the entry decodes to.
    pub declared_size: usize,
    pub crc: u32,
    /// See `CentralDirectory::external_attributes` — the unix mode lives in the
    /// high 16 bits when a unix tool wrote the archive.
    pub external_attributes: u32,
}

impl ZipListing {
    /// Is this entry a SYMLINK rather than a file.
    ///
    /// `S_IFLNK` is 0o120000, i.e. `0xA000` in the format nibble of the mode,
    /// and the mode sits in the high half of the external attributes. A
    /// DOS/Windows writer leaves that half zero, so the answer there is a plain
    /// `false` rather than a guess.
    ///
    /// NOT MEASURED against a Windows-written archive on this machine (no
    /// Windows here); what IS measured is the unix direction, in
    /// `a_symlink_entry_is_refused` over in tacet-tools, where the bits are set
    /// by hand exactly as `zip` sets them.
    pub fn is_symlink(&self) -> bool {
        (self.external_attributes >> 16) & 0xF000 == 0xA000
    }
}

/// Reads the central directory and returns what it says — WITHOUT decoding a
/// single byte of entry data.
///
/// This is the gate a caller needs before it inflates anything: it costs the
/// length of the directory, not the length of the archive.
pub fn list(zip: &[u8]) -> ZipResult<Vec<ZipListing>> {
    Ok(walk_central(zip)?
        .into_iter()
        .map(|r| ZipListing {
            name: r.name,
            method: r.method,
            compressed_size: r.compressed_size,
            declared_size: r.raw_size,
            crc: r.crc,
            external_attributes: r.external_attributes,
        })
        .collect())
}

/// Decodes ONLY the named entries, under caps the CALLER chooses.
///
/// WHY NOT `open` + a filter: `open` inflates everything, so filtering
/// afterwards means the bomb has already run. Here an entry nobody asked for is
/// never touched, and `entry_cap` reaches `inflate` itself — the difference
/// between refusing a 64 MiB expansion and refusing it after allocating it.
///
/// A name that is not in the archive is silently absent from the result rather
/// than an error: the caller already has the listing and can compare counts,
/// and turning "not present" into a failure would make a duplicate name in the
/// caller's list a hard error for no gain.
pub fn open_selected(
    zip: &[u8],
    names: &[String],
    entry_cap: usize,
    total_cap: usize,
) -> ZipResult<Vec<ZipEntry>> {
    let mut entries = Vec::new();
    let mut total_decoded = 0usize;
    for record in walk_central(zip)? {
        if !names.contains(&record.name) {
            continue;
        }
        let data = decode_entry_data(zip, &record, entry_cap)?;
        total_decoded = total_decoded
            .checked_add(data.len())
            .ok_or(ZipError::LimitExceeded("the decoded total size overflowed"))?;
        if total_decoded > total_cap {
            return Err(ZipError::LimitExceeded(
                "the archive total size cap was exceeded",
            ));
        }
        entries.push(ZipEntry {
            name: record.name,
            data,
        });
    }
    Ok(entries)
}

/// Walks the central directory into records. `open` DELIBERATELY DOES NOT USE
/// THIS: it interleaves reading a record with decoding it, so on a malformed
/// archive it reports the FIRST failure in file order. Rewriting it to pre-walk
/// would change which error a broken archive comes back with, and the malformed
/// -input tests in this crate are exactly a record of those answers.
fn walk_central(zip: &[u8]) -> ZipResult<Vec<CentralDirectory>> {
    let eocd = find_eocd(zip)?;
    let record_count = read16(zip, eocd + 10)? as usize;
    let central_offset = read32(zip, eocd + 16)? as usize;
    if central_offset > eocd {
        return Err(ZipError::Malformed(
            "the central directory offset passes the EOCD",
        ));
    }
    let mut records = Vec::with_capacity(record_count.min(1024));
    let mut offset = central_offset;
    for _ in 0..record_count {
        let record = read_central_record(zip, offset)?;
        offset = record.next_offset;
        records.push(record);
    }
    Ok(records)
}

/// Searches for the EOCD signature backwards from the end.
fn find_eocd(zip: &[u8]) -> ZipResult<usize> {
    if zip.len() < 22 {
        return Err(ZipError::Malformed(
            "the file is too short for an EOCD record",
        ));
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
    /// The `external file attributes` field. On a zip written by a unix tool the
    /// HIGH 16 bits carry the st_mode, which is the only place an archive says
    /// "this entry is a SYMLINK" — and a caller that materialises entries on
    /// disk has to be able to refuse those. A DOS/Windows writer leaves the high
    /// half zero, so reading it can never invent a mode that is not there.
    external_attributes: u32,
    local_offset: usize,
    /// The offset of the next central directory record.
    next_offset: usize,
}

fn read_central_record(zip: &[u8], offset: usize) -> ZipResult<CentralDirectory> {
    if read32(zip, offset)? != CENTRAL_SIG {
        return Err(ZipError::Malformed(
            "wrong central directory record signature",
        ));
    }
    let method = read16(zip, offset + 10)?;
    let crc = read32(zip, offset + 16)?;
    let compressed_size = read32(zip, offset + 20)? as usize;
    let raw_size = read32(zip, offset + 24)? as usize;
    let name_length = read16(zip, offset + 28)? as usize;
    let extra_length = read16(zip, offset + 30)? as usize;
    let comment_length = read16(zip, offset + 32)? as usize;
    let external_attributes = read32(zip, offset + 38)?;
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
        .ok_or(ZipError::Malformed(
            "the central directory offset overflowed",
        ))?;

    Ok(CentralDirectory {
        name,
        method,
        compressed_size,
        raw_size,
        crc,
        external_attributes,
        local_offset,
        next_offset,
    })
}

fn decode_entry_data(
    zip: &[u8],
    record: &CentralDirectory,
    entry_cap: usize,
) -> ZipResult<Vec<u8>> {
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
                return Err(ZipError::Malformed(
                    "the sizes of a STORE entry do not match",
                ));
            }
            raw.to_vec()
        }
        8 => {
            // The declared raw_size is NOT TRUSTED; the cap is applied independently.
            //
            // THE CAP IS A PARAMETER, NOT THE CONSTANT, and that is the whole
            // point of the change: a caller that materialises entries on the
            // user's disk wants a TIGHTER ceiling than the 64 MiB the OOXML path
            // needs, and until now the ceiling was hard-coded here — so the
            // tighter limit could only be applied AFTER 64 MiB had already been
            // inflated into memory. `open` still passes `ENTRY_CAP`, so nothing
            // on the document path moved.
            inflate(raw, entry_cap)?
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
