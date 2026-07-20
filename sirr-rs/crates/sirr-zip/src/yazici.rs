//! Zip yazma — yalniz STORE.
//!
//! Neden sikistirmiyoruz: OOXML tuketicileri (Excel, Word, Pages, Numbers)
//! STORE'u sorunsuz acar; sikistirma kazanci cihazda uretilen kucuk belgelerde
//! onemsizken deflate KODLAYICISI yazmak cozucuden cok daha genis bir hata
//! yuzeyi acardi. Okuma tarafinda deflate zorunlu (baskasinin dosyasi), yazmada
//! degil — asimetri bilincli.

use crate::bayt::{yaz16, yaz32};
use crate::crc32::crc32;
use crate::hata::{ZipHatasi, ZipSonuc};

/// Arsivdeki tek bir giris.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipGiris {
    /// Zip ici yol, or. "xl/workbook.xml". Daima '/' ayraci.
    pub ad: String,
    pub veri: Vec<u8>,
}

impl ZipGiris {
    pub fn yeni(ad: impl Into<String>, veri: impl Into<Vec<u8>>) -> Self {
        ZipGiris {
            ad: ad.into(),
            veri: veri.into(),
        }
    }
}

const YEREL_IMZA: u32 = 0x0403_4b50;
const MERKEZ_IMZA: u32 = 0x0201_4b50;
pub(crate) const EOCD_IMZA: u32 = 0x0605_4b50;

/// Girisleri tek bir zip arsivine paketler.
///
/// Swift surumu `UInt32(...)` daralmalariyla tasmada trap ediyordu; burada
/// zip32 sinirlarinin her biri acik bir Err. Zip64 desteklenmiyor — OOXML
/// uretimimizde 4 GiB'lik bir belge zaten bir hatanin belirtisidir.
pub fn paketle(girisler: &[ZipGiris]) -> ZipSonuc<Vec<u8>> {
    if girisler.len() > u16::MAX as usize {
        return Err(ZipHatasi::SinirAsimi("zip32 giris sayisi sinirini asiyor"));
    }

    let mut govde: Vec<u8> = Vec::new();
    let mut merkez: Vec<u8> = Vec::new();

    for giris in girisler {
        let ad = giris.ad.as_bytes();
        if ad.len() > u16::MAX as usize {
            return Err(ZipHatasi::SinirAsimi("giris adi cok uzun"));
        }
        let boyut = u32::try_from(giris.veri.len())
            .map_err(|_| ZipHatasi::SinirAsimi("giris 4 GiB sinirini asiyor"))?;
        let ofset = u32::try_from(govde.len())
            .map_err(|_| ZipHatasi::SinirAsimi("arsiv 4 GiB sinirini asiyor"))?;
        let crc = crc32(&giris.veri);

        // Yerel dosya basligi
        yaz32(&mut govde, YEREL_IMZA);
        yaz16(&mut govde, 20); // gerekli surum
        yaz16(&mut govde, 0); // bayrak
        yaz16(&mut govde, 0); // yontem: STORE
        yaz16(&mut govde, 0); // saat
        yaz16(&mut govde, 0); // tarih
        yaz32(&mut govde, crc);
        yaz32(&mut govde, boyut); // sikistirilmis
        yaz32(&mut govde, boyut); // sikistirilmamis
        yaz16(&mut govde, ad.len() as u16);
        yaz16(&mut govde, 0); // extra
        govde.extend_from_slice(ad);
        govde.extend_from_slice(&giris.veri);

        // Merkezi dizin kaydi
        yaz32(&mut merkez, MERKEZ_IMZA);
        yaz16(&mut merkez, 20); // uretici surum
        yaz16(&mut merkez, 20); // gerekli surum
        yaz16(&mut merkez, 0); // bayrak
        yaz16(&mut merkez, 0); // yontem: STORE
        yaz16(&mut merkez, 0); // saat
        yaz16(&mut merkez, 0); // tarih
        yaz32(&mut merkez, crc);
        yaz32(&mut merkez, boyut);
        yaz32(&mut merkez, boyut);
        yaz16(&mut merkez, ad.len() as u16);
        yaz16(&mut merkez, 0); // extra
        yaz16(&mut merkez, 0); // yorum
        yaz16(&mut merkez, 0); // disk
        yaz16(&mut merkez, 0); // ic oznitelik
        yaz32(&mut merkez, 0); // dis oznitelik
        yaz32(&mut merkez, ofset);
        merkez.extend_from_slice(ad);
    }

    let merkez_ofset = u32::try_from(govde.len())
        .map_err(|_| ZipHatasi::SinirAsimi("arsiv 4 GiB sinirini asiyor"))?;
    let merkez_boyut = u32::try_from(merkez.len())
        .map_err(|_| ZipHatasi::SinirAsimi("merkezi dizin 4 GiB sinirini asiyor"))?;
    let sayac = girisler.len() as u16;

    let mut sonuc = govde;
    sonuc.extend_from_slice(&merkez);
    yaz32(&mut sonuc, EOCD_IMZA);
    yaz16(&mut sonuc, 0); // disk numarasi
    yaz16(&mut sonuc, 0); // merkezi dizinin bulundugu disk
    yaz16(&mut sonuc, sayac);
    yaz16(&mut sonuc, sayac);
    yaz32(&mut sonuc, merkez_boyut);
    yaz32(&mut sonuc, merkez_ofset);
    yaz16(&mut sonuc, 0); // yorum uzunlugu
    Ok(sonuc)
}
