//! Zip hatalari.
//!
//! thiserror BILEREK kullanilmadi: sirr-zip'in kimligi "sifir bagimlilik" —
//! crate'in Cargo.toml'unda [dependencies] bolumunun bos kalmasi denetlenebilir
//! bir guvencedir. Elle yazilan Display+Error 30 satir tutuyor, bedeli bu kadar.

use std::fmt;

pub type ZipSonuc<T> = Result<T, ZipHatasi>;

/// Bozuk girdi TEK bir hata turune indirgenir: cagri yerinin yapabilecegi tek
/// sey zaten "bu dosya acilamadi" demek. Ayrinti (`neden`) yalniz tanilama icin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZipHatasi {
    /// Arsiv yapisi tutarsiz: imza yok, ofset tasiyor, dizin kesik...
    Bozuk(&'static str),
    /// Girdi bilincli olarak siniri zorluyor (zip bomb, akil disi boyut).
    SinirAsimi(&'static str),
    /// Tanimadigimiz sikistirma yontemi (yalniz STORE ve DEFLATE destekli).
    DesteklenmeyenYontem(u16),
    /// CRC32 tutmuyor: veri cozuldu ama bozuk.
    CrcUyusmadi { ad: String },
}

impl fmt::Display for ZipHatasi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZipHatasi::Bozuk(neden) => write!(f, "zip dosyasi bozuk ({neden})"),
            ZipHatasi::SinirAsimi(neden) => write!(f, "zip icerigi siniri asiyor ({neden})"),
            ZipHatasi::DesteklenmeyenYontem(y) => {
                write!(f, "desteklenmeyen sikistirma yontemi: {y}")
            }
            ZipHatasi::CrcUyusmadi { ad } => write!(f, "zip icindeki '{ad}' dogrulanamadi"),
        }
    }
}

impl std::error::Error for ZipHatasi {}
