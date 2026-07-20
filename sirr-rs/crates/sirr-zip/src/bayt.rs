//! Sinir kontrollu bayt okuma/yazma yardimcilari.
//!
//! Swift tarafindaki `oku16`/`oku32` diziye dogrudan indeksliyordu; bozuk bir
//! .xlsx'te bu crash demekti. Burada okuma yolunun TAMAMI Result doner ve
//! indeksleme yalnizca once dogrulanmis dilimler uzerinden yapilir — panik
//! yuzeyi hic olusmasin diye tip seviyesinde kapatiliyor.

use crate::hata::{ZipHatasi, ZipSonuc};

/// Verilen ofsetten 2 bayt little-endian okur.
pub fn oku16(b: &[u8], i: usize) -> ZipSonuc<u16> {
    let dilim = b
        .get(i..i + 2)
        .ok_or(ZipHatasi::Bozuk("16-bit okuma sinir disi"))?;
    Ok(u16::from_le_bytes([dilim[0], dilim[1]]))
}

/// Verilen ofsetten 4 bayt little-endian okur.
pub fn oku32(b: &[u8], i: usize) -> ZipSonuc<u32> {
    let dilim = b
        .get(i..i + 4)
        .ok_or(ZipHatasi::Bozuk("32-bit okuma sinir disi"))?;
    Ok(u32::from_le_bytes([dilim[0], dilim[1], dilim[2], dilim[3]]))
}

/// `i`den baslayan `uzunluk` baytlik dilim; tasarsa Err.
pub fn dilim(b: &[u8], i: usize, uzunluk: usize) -> ZipSonuc<&[u8]> {
    // Once tasmasiz toplama: i + uzunluk 64-bit'te bile tasabilir (usize::MAX
    // yakini degerler bozuk arsivden gelebilir), checked_add bunu kapatir.
    let son = i
        .checked_add(uzunluk)
        .ok_or(ZipHatasi::Bozuk("ofset toplami tasti"))?;
    b.get(i..son)
        .ok_or(ZipHatasi::Bozuk("dilim sinir disi"))
}

/// Little-endian yazicilar. Yazma yolunda girdi bizim urettigimiz veridir,
/// bu yuzden Result yok — ama boyut daralmalari cagri yerinde denetlenir.
pub fn yaz16(hedef: &mut Vec<u8>, v: u16) {
    hedef.extend_from_slice(&v.to_le_bytes());
}

pub fn yaz32(hedef: &mut Vec<u8>, v: u32) {
    hedef.extend_from_slice(&v.to_le_bytes());
}
