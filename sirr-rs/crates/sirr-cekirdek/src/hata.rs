//! Arac hatalari: iki ayri izleyici, iki ayri metin.
//!
//! Kullaniciya giden metin Turkce ve tanidik ("Dosya bulunamadi."); modele
//! giden metin SABIT ve INGILIZCE. Ayrim bilincli: model arac hatasini
//! yanitina oldugu gibi yansitsa bile ne Turkce sizar, ne ham hata kodu,
//! ne de dosya yolu gibi kisisel iz. Swift tarafinda ayni kural gecerli.

use std::path::PathBuf;

/// Modele donen sabit hata metni. Bilerek `const`: cagri yerlerinin kendi
/// varyantini uydurmasi bu garantiyi delerdi.
pub const HATA_MODEL_METNI: &str =
    "tool_failed: the action could not be completed; no result was produced";

#[derive(Debug, thiserror::Error)]
pub enum AracHatasi {
    /// Model semaya uymayan argüman gonderdi (alan eksik, tip yanlis).
    #[error("argüman gecersiz: {0}")]
    GecersizArguman(String),

    /// Semada zorunlu olan alan hic gelmedi.
    #[error("zorunlu alan eksik: {0}")]
    EksikAlan(String),

    #[error("dosya bulunamadi: {0}")]
    DosyaYok(PathBuf),

    /// Calisma dizininin disina cikma girisimi — kum havuzu ihlali.
    #[error("izin verilen dizin disinda: {0}")]
    KumHavuzuIhlali(PathBuf),

    #[error("cihazda yer kalmadi")]
    YerYok,

    /// Kullanici onay kapisinda "hayir" dedi.
    #[error("kullanici izin vermedi: {0}")]
    IzinYok(String),

    #[error("islem zaman asimina ugradi")]
    ZamanAsimi,

    /// Sarmalanan G/C hatasi.
    #[error("dosya islemi tamamlanamadi")]
    Giris(#[from] std::io::Error),

    #[error("veri cozumlenemedi")]
    Serde(#[from] serde_json::Error),

    /// Siniflandirilamayan hata; kullaniciya yine de duzgun cumle gosterilir.
    #[error("{0}")]
    Diger(String),
}

impl AracHatasi {
    /// Cipte gosterilecek kisa Turkce cumle.
    ///
    /// Ham `io::Error` metni ("No such file or directory (os error 2)") asla
    /// ekrana cikmaz: kullaniciya sistem hatasi degil, olan bitenin insan
    /// diline cevrilmis hali gosterilir.
    pub fn kisa_hata(&self) -> String {
        match self {
            AracHatasi::GecersizArguman(_) | AracHatasi::EksikAlan(_) => {
                "Bu istek anlasilamadi.".into()
            }
            AracHatasi::DosyaYok(_) => "Dosya bulunamadi.".into(),
            AracHatasi::KumHavuzuIhlali(_) => "Bu konuma erisim yok.".into(),
            AracHatasi::YerYok => "Cihazda yer kalmadi.".into(),
            AracHatasi::IzinYok(_) => "Paylasim onaylanmadi.".into(),
            AracHatasi::ZamanAsimi => "Islem cok uzun surdu.".into(),
            AracHatasi::Giris(_) => "Dosya islemi tamamlanamadi.".into(),
            AracHatasi::Serde(_) => "Veri okunamadi.".into(),
            AracHatasi::Diger(m) if !m.is_empty() => m.clone(),
            AracHatasi::Diger(_) => "Bu adim tamamlanamadi.".into(),
        }
    }

    /// Modele donecek metin — her zaman ayni, her zaman Ingilizce.
    pub fn model_metni(&self) -> &'static str {
        HATA_MODEL_METNI
    }
}

pub type AracSonuc<T> = Result<T, AracHatasi>;
