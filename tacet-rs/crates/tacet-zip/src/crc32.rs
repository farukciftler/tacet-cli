//! CRC32 (IEEE 802.3, reversed polynomial 0xEDB88320) — zip's mandatory
//! integrity field.
//!
//! The table is produced at compile time with a `const fn`: keeping a lazy-init
//! (OnceLock and friends) at run time costs both synchronization and needless
//! state.

const TABLE: [u32; 256] = generate_table();

const fn generate_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut c = i as u32;
        let mut bit = 0;
        while bit < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            bit += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

/// CRC32 in one shot.
pub fn crc32(data: &[u8]) -> u32 {
    crc32_continue(0, data)
}

/// Chunked CRC32 — left open so a large body can be verified without loading it
/// into memory all at once (the inflate output is produced chunk by chunk).
pub fn crc32_continue(previous: u32, data: &[u8]) -> u32 {
    let mut crc = !previous;
    for &b in data {
        crc = TABLE[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    !crc
}
