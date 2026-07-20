//! CRC32 (IEEE 802.3, ters polinom 0xEDB88320) — zip'in zorunlu bütünlük alani.
//!
//! Tablo `const fn` ile derleme aninda uretiliyor: calisma aninda lazy-init
//! (OnceLock vb.) tutmak hem senkronizasyon maliyeti hem de gereksiz durum.

const TABLO: [u32; 256] = tablo_uret();

const fn tablo_uret() -> [u32; 256] {
    let mut tablo = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut c = i as u32;
        let mut bit = 0;
        while bit < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            bit += 1;
        }
        tablo[i] = c;
        i += 1;
    }
    tablo
}

/// Tek seferde CRC32.
pub fn crc32(veri: &[u8]) -> u32 {
    crc32_devam(0, veri)
}

/// Parcali CRC32 — buyuk govdeyi tek seferde bellege almadan dogrulayabilmek
/// icin acik birakildi (inflate ciktisi parca parca uretiliyor).
pub fn crc32_devam(onceki: u32, veri: &[u8]) -> u32 {
    let mut crc = !onceki;
    for &b in veri {
        crc = TABLO[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    !crc
}
