//! El yazimi DEFLATE cozucu (RFC 1951): saklanmis bloklar + sabit Huffman +
//! dinamik Huffman.
//!
//! Neden el ile: sirr'in kimligi hazir crate cekmemek; ayrica cozucu bize ait
//! olunca "ne kadar cikti uretilebilir" sinirini biz koyariz — zip bomb'a karsi
//! tek gercek savunma bu (bkz. `en_cok_cikti`).
//!
//! Cozum stratejisi kod tablosu yerine bit-bit kanonik Huffman gezinmesi
//! (puff yaklasimi): OOXML parcalari kucuk, hiz darbogaz degil; buna karsilik
//! tablo insasinda yapilacak bir hata sessiz yanlis cikti uretir. Basit olan
//! dogrulanabilir olandir.

use crate::hata::{ZipHatasi, ZipSonuc};

/// LSB-onceli bit okuyucu. Her okuma sinir kontrollu.
struct BitOkuyucu<'a> {
    veri: &'a [u8],
    /// Bir sonraki okunacak bitin mutlak konumu (bayt * 8 + bit).
    konum: usize,
}

impl<'a> BitOkuyucu<'a> {
    fn yeni(veri: &'a [u8]) -> Self {
        BitOkuyucu { veri, konum: 0 }
    }

    fn bit(&mut self) -> ZipSonuc<u32> {
        let bayt_indeks = self.konum >> 3;
        let bayt = *self
            .veri
            .get(bayt_indeks)
            .ok_or(ZipHatasi::Bozuk("deflate akisi beklenmedik yerde bitti"))?;
        let deger = (bayt >> (self.konum & 7)) & 1;
        self.konum += 1;
        Ok(deger as u32)
    }

    /// `adet` biti LSB-onceli okur (adet <= 16).
    fn bitler(&mut self, adet: u32) -> ZipSonuc<u32> {
        let mut deger = 0u32;
        for i in 0..adet {
            deger |= self.bit()? << i;
        }
        Ok(deger)
    }

    /// Bayt sinirina hizalar (saklanmis blok basi).
    fn hizala(&mut self) {
        self.konum = (self.konum + 7) & !7;
    }

    fn bayt_konumu(&self) -> usize {
        self.konum >> 3
    }

    fn bayt_konumuna_atla(&mut self, yeni: usize) {
        self.konum = yeni << 3;
    }
}

/// Kanonik Huffman kod tablosu: uzunluk basina sembol sayisi + sirali semboller.
struct Huffman {
    /// sayimlar[u] = u bit uzunlugundaki kod adedi (u = 0..=15).
    sayimlar: [u16; 16],
    /// Once uzunluga, sonra sembol degerine gore sirali semboller.
    semboller: Vec<u16>,
}

impl Huffman {
    fn yeni(uzunluklar: &[u8]) -> ZipSonuc<Huffman> {
        let mut sayimlar = [0u16; 16];
        for &u in uzunluklar {
            if u > 15 {
                return Err(ZipHatasi::Bozuk("huffman kod uzunlugu 15'i asiyor"));
            }
            sayimlar[u as usize] += 1;
        }
        // Uzunlugu 0 olanlar kullanilmayan sembollerdir; hepsi 0 ise tablo bostur.
        if sayimlar[0] as usize == uzunluklar.len() {
            return Err(ZipHatasi::Bozuk("bos huffman tablosu"));
        }

        // Kod alani tasmasi kontrolu: eksik (incomplete) tablolar deflate'te
        // yalniz tek sembollu mesafe tablosunda mesru; asiri dolu tablo daima bozuk.
        let mut bos = 1i32;
        for &sayi in sayimlar.iter().skip(1) {
            bos = bos.saturating_mul(2) - sayi as i32;
            if bos < 0 {
                return Err(ZipHatasi::Bozuk("asiri dolu huffman tablosu"));
            }
        }

        let mut ofsetler = [0u16; 16];
        for u in 1..15 {
            ofsetler[u + 1] = ofsetler[u] + sayimlar[u];
        }
        let mut semboller = vec![0u16; uzunluklar.len()];
        for (sembol, &u) in uzunluklar.iter().enumerate() {
            if u != 0 {
                semboller[ofsetler[u as usize] as usize] = sembol as u16;
                ofsetler[u as usize] += 1;
            }
        }
        Ok(Huffman {
            sayimlar,
            semboller,
        })
    }

    /// Bit bit ilerleyerek kanonik kodu cozer.
    fn coz(&self, br: &mut BitOkuyucu<'_>) -> ZipSonuc<u16> {
        let mut kod = 0i32;
        let mut ilk = 0i32;
        let mut indeks = 0i32;
        for u in 1..16 {
            kod |= br.bit()? as i32;
            let sayi = self.sayimlar[u] as i32;
            if kod - sayi < ilk {
                let yer = (indeks + (kod - ilk)) as usize;
                return self
                    .semboller
                    .get(yer)
                    .copied()
                    .ok_or(ZipHatasi::Bozuk("huffman sembol indeksi sinir disi"));
            }
            indeks += sayi;
            ilk = (ilk + sayi) << 1;
            kod <<= 1;
        }
        Err(ZipHatasi::Bozuk("gecersiz huffman kodu"))
    }
}

// RFC 1951, 3.2.5 — uzunluk ve mesafe tablolari.
const UZUNLUK_TABAN: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const UZUNLUK_EK: [u32; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const MESAFE_TABAN: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const MESAFE_EK: [u32; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
/// Kod-uzunlugu alfabesinin akistaki sirasi (RFC 1951, 3.2.7).
const KOD_UZUNLUK_SIRASI: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// Ham DEFLATE akisini cozer.
///
/// `en_cok_cikti` sert bir tavandir: asilinca `SinirAsimi` doner. Bildirilen
/// (basliktaki) boyuta GUVENILMEZ — zip bomb tam olarak orada yalan soyler.
pub fn inflate(veri: &[u8], en_cok_cikti: usize) -> ZipSonuc<Vec<u8>> {
    let mut br = BitOkuyucu::yeni(veri);
    let mut cikti: Vec<u8> = Vec::new();

    loop {
        let son_blok = br.bit()? == 1;
        let tur = br.bitler(2)?;
        match tur {
            0 => saklanmis_blok(&mut br, &mut cikti, en_cok_cikti)?,
            1 => {
                let (lit, mes) = sabit_tablolar();
                huffman_blok(&mut br, &mut cikti, &lit, &mes, en_cok_cikti)?;
            }
            2 => {
                let (lit, mes) = dinamik_tablolar(&mut br)?;
                huffman_blok(&mut br, &mut cikti, &lit, &mes, en_cok_cikti)?;
            }
            _ => return Err(ZipHatasi::Bozuk("gecersiz deflate blok turu")),
        }
        if son_blok {
            break;
        }
    }
    Ok(cikti)
}

fn saklanmis_blok(
    br: &mut BitOkuyucu<'_>,
    cikti: &mut Vec<u8>,
    en_cok_cikti: usize,
) -> ZipSonuc<()> {
    br.hizala();
    let bas = br.bayt_konumu();
    let uzunluk = crate::bayt::oku16(br.veri, bas)? as usize;
    let ters = crate::bayt::oku16(br.veri, bas + 2)? as usize;
    // LEN ve ~LEN tutarliligi bozuk akisi erken yakalar.
    if uzunluk != (!ters & 0xFFFF) {
        return Err(ZipHatasi::Bozuk("saklanmis blok uzunlugu tutarsiz"));
    }
    let govde = crate::bayt::dilim(br.veri, bas + 4, uzunluk)?;
    if cikti.len() + uzunluk > en_cok_cikti {
        return Err(ZipHatasi::SinirAsimi("cozulen boyut tavani asildi"));
    }
    cikti.extend_from_slice(govde);
    br.bayt_konumuna_atla(bas + 4 + uzunluk);
    Ok(())
}

fn sabit_tablolar() -> (Huffman, Huffman) {
    let mut lit_uz = [0u8; 288];
    for (i, u) in lit_uz.iter_mut().enumerate() {
        *u = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    // Sabit tablolar tanim geregi gecerli; yeni() burada asla Err donmez.
    let lit = Huffman::yeni(&lit_uz).expect("sabit literal tablosu gecerlidir");
    let mes = Huffman::yeni(&[5u8; 30]).expect("sabit mesafe tablosu gecerlidir");
    (lit, mes)
}

fn dinamik_tablolar(br: &mut BitOkuyucu<'_>) -> ZipSonuc<(Huffman, Huffman)> {
    let hlit = br.bitler(5)? as usize + 257;
    let hdist = br.bitler(5)? as usize + 1;
    let hclen = br.bitler(4)? as usize + 4;
    if hlit > 286 || hdist > 30 {
        return Err(ZipHatasi::Bozuk("dinamik tablo sayaci alfabeyi asiyor"));
    }

    let mut kod_uz = [0u8; 19];
    for &yer in KOD_UZUNLUK_SIRASI.iter().take(hclen) {
        kod_uz[yer] = br.bitler(3)? as u8;
    }
    let kod_tablosu = Huffman::yeni(&kod_uz)?;

    // Literal ve mesafe uzunluklari TEK akista kodlanir; 16/17/18 tekrar
    // komutlari iki alfabenin sinirini asabilir, bu yuzden birlikte cozulur.
    let toplam = hlit + hdist;
    let mut uzunluklar = vec![0u8; toplam];
    let mut i = 0usize;
    while i < toplam {
        let sembol = kod_tablosu.coz(br)?;
        match sembol {
            0..=15 => {
                uzunluklar[i] = sembol as u8;
                i += 1;
            }
            16 => {
                // Onceki uzunlugu 3..6 kez tekrarla.
                if i == 0 {
                    return Err(ZipHatasi::Bozuk("tekrar komutu once gelen uzunluk yok"));
                }
                let onceki = uzunluklar[i - 1];
                let tekrar = 3 + br.bitler(2)? as usize;
                if i + tekrar > toplam {
                    return Err(ZipHatasi::Bozuk("uzunluk tekrari tabloyu tasiriyor"));
                }
                uzunluklar[i..i + tekrar].fill(onceki);
                i += tekrar;
            }
            17 | 18 => {
                let tekrar = if sembol == 17 {
                    3 + br.bitler(3)? as usize
                } else {
                    11 + br.bitler(7)? as usize
                };
                if i + tekrar > toplam {
                    return Err(ZipHatasi::Bozuk("sifir tekrari tabloyu tasiriyor"));
                }
                uzunluklar[i..i + tekrar].fill(0);
                i += tekrar;
            }
            _ => return Err(ZipHatasi::Bozuk("gecersiz kod uzunlugu sembolu")),
        }
    }

    if uzunluklar[256] == 0 {
        return Err(ZipHatasi::Bozuk("blok sonu sembolu kodlanmamis"));
    }
    let lit = Huffman::yeni(&uzunluklar[..hlit])?;
    // Tek mesafe kodu olan (ya da hic mesafe kullanmayan) akislar mesrudur;
    // bu durumda tablo eksiktir ve Huffman::yeni bos tabloya Err der. Mesafe
    // hic kullanilmiyorsa yer tutucu bir tablo yeter — coz() zaten cagrilmaz.
    let mesafe_uz = &uzunluklar[hlit..];
    let mes = if mesafe_uz.iter().all(|&u| u == 0) {
        Huffman::yeni(&[1u8, 1u8])?
    } else {
        Huffman::yeni(mesafe_uz)?
    };
    Ok((lit, mes))
}

fn huffman_blok(
    br: &mut BitOkuyucu<'_>,
    cikti: &mut Vec<u8>,
    lit: &Huffman,
    mes: &Huffman,
    en_cok_cikti: usize,
) -> ZipSonuc<()> {
    loop {
        let sembol = lit.coz(br)?;
        match sembol {
            0..=255 => {
                if cikti.len() >= en_cok_cikti {
                    return Err(ZipHatasi::SinirAsimi("cozulen boyut tavani asildi"));
                }
                cikti.push(sembol as u8);
            }
            256 => return Ok(()),
            257..=285 => {
                let idx = sembol as usize - 257;
                let uzunluk = UZUNLUK_TABAN[idx] as usize + br.bitler(UZUNLUK_EK[idx])? as usize;

                let mes_sembol = mes.coz(br)? as usize;
                if mes_sembol >= MESAFE_TABAN.len() {
                    return Err(ZipHatasi::Bozuk("gecersiz mesafe sembolu"));
                }
                let mesafe =
                    MESAFE_TABAN[mes_sembol] as usize + br.bitler(MESAFE_EK[mes_sembol])? as usize;
                // Geriye referans pencerenin disini gosteriyorsa akis bozuktur;
                // kontrolsuz birakmak indeks panigi demek olurdu.
                if mesafe == 0 || mesafe > cikti.len() {
                    return Err(ZipHatasi::Bozuk("geriye referans pencere disinda"));
                }
                if cikti.len() + uzunluk > en_cok_cikti {
                    return Err(ZipHatasi::SinirAsimi("cozulen boyut tavani asildi"));
                }
                // Bayt bayt kopyalanir: kaynak ve hedef araliklari cakisabilir
                // (mesafe < uzunluk), LZ77'nin "kendini tekrarlayan" durumu.
                for kaynak in (cikti.len() - mesafe..).take(uzunluk) {
                    let b = cikti[kaynak];
                    cikti.push(b);
                }
            }
            _ => return Err(ZipHatasi::Bozuk("gecersiz literal/uzunluk sembolu")),
        }
    }
}
