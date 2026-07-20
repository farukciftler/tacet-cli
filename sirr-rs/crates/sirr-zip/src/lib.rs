//! sirr-zip: el yazimi zip/deflate/crc32 — OOXML (.xlsx/.docx) uretimi ve okumasi.
//!
//! Bu crate'in [dependencies] bolumu BILEREK bostur: sirr'in "sifir bagimlilik"
//! kimliginin en somut noktasi burasi. Zip yazimi, inflate ve CRC32 el ile
//! yazilir; boylece cozucunun sinirlarini (zip bomb tavani, sinir kontrolu) biz
//! belirleriz — hazir bir crate'in varsayilanlarina degil.
//!
//! Yazma yalniz STORE, okuma STORE + DEFLATE. Gerekce yazici.rs dosya basinda.

mod bayt;
mod crc32;
mod hata;
mod inflate;
mod okuyucu;
mod yazici;

pub use crc32::{crc32, crc32_devam};
pub use hata::{ZipHatasi, ZipSonuc};
pub use inflate::inflate;
pub use okuyucu::{ARSIV_TAVANI, GIRIS_TAVANI, ac, ac_harita};
pub use yazici::{ZipGiris, paketle};
