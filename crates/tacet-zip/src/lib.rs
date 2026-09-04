//! tacet-zip: hand-written zip/deflate/crc32 — producing and reading OOXML
//! (.xlsx/.docx).
//!
//! The [dependencies] section of this crate is DELIBERATELY empty: this is the
//! most concrete point of Tacet's "zero dependency" identity. Zip writing,
//! inflate and CRC32 are written by hand; that way WE decide the decoder's
//! limits (the zip-bomb cap, the bounds checks) — not the defaults of some
//! ready-made crate.
//!
//! Writing is STORE only, reading is STORE + DEFLATE. The rationale is in the
//! header of writer.rs.

mod byte;
mod crc32;
mod error;
mod inflate;
mod reader;
mod writer;

pub use crc32::{crc32, crc32_continue};
pub use error::{ZipError, ZipResult};
pub use inflate::inflate;
pub use reader::{ARCHIVE_CAP, ENTRY_CAP, ZipListing, list, open, open_map, open_selected};
pub use writer::{ZipEntry, pack};
