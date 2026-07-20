//! Motor hatalari.
//!
//! `AracHatasi`'ndan AYRI bir tip: arac hatasi kullaniciya cip olarak gosterilen
//! ve modele sabit metinle donen bir seydir; motor hatasi ise tur DONGUSUNUN
//! kendisinin kirilmasidir (agirlik yuklenemedi, baglam tasti, kisit celisti).
//! Ikisi ayni enum'a sikistirilsaydi "modele ne donecek" sorusunun cevabi
//! anlamsizlasirdi — motor coktugunde donecek bir model yok.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum MotorHatasi {
    /// Model dosyasi (GGUF/safetensors) yok ya da okunamiyor.
    #[error("model dosyasi yuklenemedi: {0}")]
    ModelYuklenemedi(PathBuf),

    /// Belirtecleyici yuklenemedi ya da metni belirtece cevirenedi.
    #[error("belirtecleme basarisiz: {0}")]
    Belirtecleme(String),

    /// Istem kirpildiktan SONRA bile baglam butcesine sigmadi.
    #[error("istem baglam butcesine sigmadi: {olculen} > {butce}")]
    ButceAsimi { olculen: usize, butce: usize },

    /// Kisitlayici, maskeye ragmen gelen belirteci kabul etmedi. Bu bir
    /// mantik hatasidir (maskeleme ile ilerletme ayrismis), sessizce
    /// yutulmamali.
    #[error("kisit belirteci reddetti: {0}")]
    KisitIhlali(u32),

    /// SahteMotor'un betigi bitti — testin senaryosu eksik demektir.
    #[error("sahte motor betiginde adim kalmadi ({cagri}. cagri)")]
    BetikBitti { cagri: usize },

    #[error("cikarim basarisiz: {0}")]
    Cikarim(String),

    #[error("model dosyasi okunamadi")]
    Giris(#[from] std::io::Error),
}

pub type MotorSonuc<T> = Result<T, MotorHatasi>;
