//! Zip okuma — STORE + DEFLATE.
//!
//! Tasarim kurali: HICBIR yol panige gitmez. Girdi kullanicidan gelen (ya da
//! kotu niyetli) bir .xlsx olabilir; Swift surumunde bozuk basliklar dogrudan
//! dizi indekslemesine gidiyordu ve crash uretebiliyordu. Burada her alan
//! `bayt::` yardimcilariyla sinir kontrollu okunur, her sayisal donusum
//! denetlenir, her cozulen govde bir tavana tabidir.

use crate::bayt::{dilim, oku16, oku32};
use crate::crc32::crc32;
use crate::hata::{ZipHatasi, ZipSonuc};
use crate::inflate::inflate;
use crate::yazici::{EOCD_IMZA, ZipGiris};
use std::collections::BTreeMap;

const MERKEZ_IMZA: u32 = 0x0201_4b50;
const YEREL_IMZA: u32 = 0x0403_4b50;

/// Tek bir girisin cozulmus halinin ust siniri (64 MiB).
///
/// OOXML parcalari icinde en buyugu tipik olarak sheet1.xml'dir ve birkac MB'i
/// gecmez; 64 MiB rahat bir tavan ama zip bomb'un isine yaramayacak kadar dusuk.
pub const GIRIS_TAVANI: usize = 64 * 1024 * 1024;

/// Tum arsivin cozulmus toplam boyut siniri (256 MiB).
pub const ARSIV_TAVANI: usize = 256 * 1024 * 1024;

/// EOCD'nin geriye dogru arandigi en fazla bayt: 22 bayt sabit kayit + en fazla
/// 64 KiB yorum (yorum uzunlugu u16).
const EOCD_ARAMA_PENCERESI: usize = 22 + 65_536;

/// Arsivi acar, girisleri dosyadaki sirayla dondurur.
pub fn ac(zip: &[u8]) -> ZipSonuc<Vec<ZipGiris>> {
    let eocd = eocd_bul(zip)?;

    let kayit_sayisi = oku16(zip, eocd + 10)? as usize;
    let merkez_ofset = oku32(zip, eocd + 16)? as usize;

    // Merkezi dizin EOCD'den sonra baslayamaz; bu kontrol hem bozuk hem de
    // ozellikle uydurulmus (ofsetin veriyi geri sardigi) arsivleri eler.
    if merkez_ofset > eocd {
        return Err(ZipHatasi::Bozuk("merkezi dizin ofseti EOCD'yi asiyor"));
    }

    let mut girisler = Vec::with_capacity(kayit_sayisi.min(1024));
    let mut toplam_cozulen = 0usize;
    let mut ofset = merkez_ofset;

    for _ in 0..kayit_sayisi {
        let kayit = merkez_kaydi_oku(zip, ofset)?;
        let veri = giris_verisini_coz(zip, &kayit)?;

        toplam_cozulen += veri.len();
        if toplam_cozulen > ARSIV_TAVANI {
            return Err(ZipHatasi::SinirAsimi("arsiv toplam boyut tavani asildi"));
        }

        girisler.push(ZipGiris {
            ad: kayit.ad,
            veri,
        });
        ofset = kayit.sonraki_ofset;
    }

    Ok(girisler)
}

/// Arsivi ad -> icerik haritasi olarak acar. Ayni ad birden fazla kez geciyorsa
/// SONUNCUSU kazanir (zip semantigi: sonraki kayit oncekini golgeler).
pub fn ac_harita(zip: &[u8]) -> ZipSonuc<BTreeMap<String, Vec<u8>>> {
    Ok(ac(zip)?
        .into_iter()
        .map(|g| (g.ad, g.veri))
        .collect())
}

/// EOCD imzasini sondan geriye dogru arar.
fn eocd_bul(zip: &[u8]) -> ZipSonuc<usize> {
    if zip.len() < 22 {
        return Err(ZipHatasi::Bozuk("dosya EOCD kaydi icin fazla kisa"));
    }
    let en_son = zip.len() - 22;
    let en_erken = en_son.saturating_sub(EOCD_ARAMA_PENCERESI);
    let mut i = en_son;
    loop {
        if oku32(zip, i)? == EOCD_IMZA {
            // Yorum uzunlugu dosyanin sonuyla uyumlu olmali; degilse bu imza
            // veri icinde rastlanan bir yanlis pozitiftir, aramaya devam.
            let yorum_uzunlugu = oku16(zip, i + 20)? as usize;
            if i + 22 + yorum_uzunlugu == zip.len() {
                return Ok(i);
            }
        }
        if i == en_erken {
            return Err(ZipHatasi::Bozuk("EOCD kaydi bulunamadi"));
        }
        i -= 1;
    }
}

/// Merkezi dizin kaydindan cikarilan, veriyi bulmaya yetecek alanlar.
struct MerkezKaydi {
    ad: String,
    yontem: u16,
    sik_boyut: usize,
    ham_boyut: usize,
    crc: u32,
    yerel_ofset: usize,
    /// Bir sonraki merkezi dizin kaydinin ofseti.
    sonraki_ofset: usize,
}

fn merkez_kaydi_oku(zip: &[u8], ofset: usize) -> ZipSonuc<MerkezKaydi> {
    if oku32(zip, ofset)? != MERKEZ_IMZA {
        return Err(ZipHatasi::Bozuk("merkezi dizin kayit imzasi yanlis"));
    }
    let yontem = oku16(zip, ofset + 10)?;
    let crc = oku32(zip, ofset + 16)?;
    let sik_boyut = oku32(zip, ofset + 20)? as usize;
    let ham_boyut = oku32(zip, ofset + 24)? as usize;
    let ad_uzunluk = oku16(zip, ofset + 28)? as usize;
    let extra_uzunluk = oku16(zip, ofset + 30)? as usize;
    let yorum_uzunluk = oku16(zip, ofset + 32)? as usize;
    let yerel_ofset = oku32(zip, ofset + 42)? as usize;

    let ad_baytlari = dilim(zip, ofset + 46, ad_uzunluk)?;
    // Gecersiz UTF-8 arsivi reddetmek icin yeterli bir sebep degil (eski
    // araclar CP437 yazar); kayipli cozmek dosyayi acilabilir tutar.
    let ad = String::from_utf8_lossy(ad_baytlari).into_owned();

    let sonraki_ofset = ofset
        .checked_add(46)
        .and_then(|v| v.checked_add(ad_uzunluk))
        .and_then(|v| v.checked_add(extra_uzunluk))
        .and_then(|v| v.checked_add(yorum_uzunluk))
        .ok_or(ZipHatasi::Bozuk("merkezi dizin ofseti tasti"))?;

    Ok(MerkezKaydi {
        ad,
        yontem,
        sik_boyut,
        ham_boyut,
        crc,
        yerel_ofset,
        sonraki_ofset,
    })
}

fn giris_verisini_coz(zip: &[u8], kayit: &MerkezKaydi) -> ZipSonuc<Vec<u8>> {
    // Veri baslangici yerel basliktan okunur: merkezi dizindeki ad/extra
    // uzunluklari yerel basliktakinden FARKLI olabilir (mesru bir durum).
    let yerel = kayit.yerel_ofset;
    if oku32(zip, yerel)? != YEREL_IMZA {
        return Err(ZipHatasi::Bozuk("yerel dosya basligi imzasi yanlis"));
    }
    let yerel_ad_uzunluk = oku16(zip, yerel + 26)? as usize;
    let yerel_extra_uzunluk = oku16(zip, yerel + 28)? as usize;
    let veri_bas = yerel
        .checked_add(30)
        .and_then(|v| v.checked_add(yerel_ad_uzunluk))
        .and_then(|v| v.checked_add(yerel_extra_uzunluk))
        .ok_or(ZipHatasi::Bozuk("yerel veri ofseti tasti"))?;

    let ham = dilim(zip, veri_bas, kayit.sik_boyut)?;

    let cozulen = match kayit.yontem {
        0 => {
            if kayit.sik_boyut != kayit.ham_boyut {
                return Err(ZipHatasi::Bozuk("STORE girisinde boyutlar uyusmuyor"));
            }
            ham.to_vec()
        }
        8 => {
            // Bildirilen ham_boyut'a GUVENILMEZ; tavan bagimsiz olarak uygulanir.
            inflate(ham, GIRIS_TAVANI)?
        }
        yontem => return Err(ZipHatasi::DesteklenmeyenYontem(yontem)),
    };

    // CRC sifirsa (bazi akis-yazan araclar veri tanimlayiciya birakir) atlanir;
    // aksi halde bu, sessiz veri bozulmasina karsi tek gercek kalkandir.
    if kayit.crc != 0 && crc32(&cozulen) != kayit.crc {
        return Err(ZipHatasi::CrcUyusmadi {
            ad: kayit.ad.clone(),
        });
    }

    Ok(cozulen)
}
