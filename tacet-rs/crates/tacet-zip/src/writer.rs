//! Zip writing — STORE only.
//!
//! Why we do not compress: OOXML consumers (Excel, Word, Pages, Numbers) open
//! STORE without trouble; the compression gain on the small documents produced
//! on device is negligible, whereas writing a deflate ENCODER would open a far
//! wider error surface than the decoder. On the reading side deflate is
//! mandatory (someone else's file), on the writing side it is not — the
//! asymmetry is deliberate.

use crate::byte::{write16, write32};
use crate::crc32::crc32;
use crate::error::{ZipError, ZipResult};

/// A single entry in the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipEntry {
    /// The in-zip path, e.g. "xl/workbook.xml". Always the '/' separator.
    pub name: String,
    pub data: Vec<u8>,
}

impl ZipEntry {
    pub fn new(name: impl Into<String>, data: impl Into<Vec<u8>>) -> Self {
        ZipEntry {
            name: name.into(),
            data: data.into(),
        }
    }
}

const LOCAL_SIG: u32 = 0x0403_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
pub(crate) const EOCD_SIG: u32 = 0x0605_4b50;

/// Packs the entries into a single zip archive.
///
/// The Swift version trapped on overflow through `UInt32(...)` narrowings; here
/// each of the zip32 limits is an explicit Err. Zip64 is not supported — in our
/// OOXML production a 4 GiB document is a symptom of a bug anyway.
pub fn pack(entries: &[ZipEntry]) -> ZipResult<Vec<u8>> {
    if entries.len() > u16::MAX as usize {
        return Err(ZipError::LimitExceeded(
            "exceeds the zip32 entry count limit",
        ));
    }

    let mut body: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();

    for entry in entries {
        let name = entry.name.as_bytes();
        if name.len() > u16::MAX as usize {
            return Err(ZipError::LimitExceeded("the entry name is too long"));
        }
        let size = u32::try_from(entry.data.len())
            .map_err(|_| ZipError::LimitExceeded("the entry exceeds the 4 GiB limit"))?;
        let offset = u32::try_from(body.len())
            .map_err(|_| ZipError::LimitExceeded("the archive exceeds the 4 GiB limit"))?;
        let crc = crc32(&entry.data);

        // Local file header
        write32(&mut body, LOCAL_SIG);
        write16(&mut body, 20); // version needed
        write16(&mut body, 0); // flags
        write16(&mut body, 0); // method: STORE
        write16(&mut body, 0); // time
        write16(&mut body, 0); // date
        write32(&mut body, crc);
        write32(&mut body, size); // compressed
        write32(&mut body, size); // uncompressed
        write16(&mut body, name.len() as u16);
        write16(&mut body, 0); // extra
        body.extend_from_slice(name);
        body.extend_from_slice(&entry.data);

        // Central directory record
        write32(&mut central, CENTRAL_SIG);
        write16(&mut central, 20); // version made by
        write16(&mut central, 20); // version needed
        write16(&mut central, 0); // flags
        write16(&mut central, 0); // method: STORE
        write16(&mut central, 0); // time
        write16(&mut central, 0); // date
        write32(&mut central, crc);
        write32(&mut central, size);
        write32(&mut central, size);
        write16(&mut central, name.len() as u16);
        write16(&mut central, 0); // extra
        write16(&mut central, 0); // comment
        write16(&mut central, 0); // disk
        write16(&mut central, 0); // internal attributes
        write32(&mut central, 0); // external attributes
        write32(&mut central, offset);
        central.extend_from_slice(name);
    }

    let central_offset = u32::try_from(body.len())
        .map_err(|_| ZipError::LimitExceeded("the archive exceeds the 4 GiB limit"))?;
    let central_size = u32::try_from(central.len())
        .map_err(|_| ZipError::LimitExceeded("the central directory exceeds the 4 GiB limit"))?;
    let count = entries.len() as u16;

    let mut result = body;
    result.extend_from_slice(&central);
    write32(&mut result, EOCD_SIG);
    write16(&mut result, 0); // disk number
    write16(&mut result, 0); // the disk the central directory is on
    write16(&mut result, count);
    write16(&mut result, count);
    write32(&mut result, central_size);
    write32(&mut result, central_offset);
    write16(&mut result, 0); // comment length
    Ok(result)
}
