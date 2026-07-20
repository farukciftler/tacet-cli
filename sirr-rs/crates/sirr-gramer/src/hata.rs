//! Gramer hatalari.
//!
//! Bu hatalar KULLANICIYA gosterilmek icin degil: sart-kosullu uretim dogru
//! calisiyorsa model gecersiz bir karakter zaten uretemez, dolayisiyla buraya
//! dusmek ya gramerin devre disi oldugu (serbest cozumleme) ya da bir hata
//! ayiklama senaryosudur. Bu yuzden metinler gelistiriciye konusur; arac
//! katmanindaki `AracHatasi` gibi ikili kanal kurmaya gerek yok.

/// Gramerin reddettigi durumlar.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GramerHatasi {
    /// Bu konumda bu karakter gramere gore uretilemez.
    #[error("konum {konum}: '{karakter}' burada gecerli degil")]
    BeklenmeyenKarakter { karakter: char, konum: usize },

    /// JSON kapandi ama arkasindan (bosluk disi) baska sey geldi.
    #[error("konum {konum}: gecerli JSON bitti, fazladan girdi var")]
    FazladanGirdi { konum: usize },

    /// Girdi tukendi ama yigin bos degil (acik nesne/dizi/metin kaldi).
    #[error("girdi yarim kaldi: yapi kapanmadi")]
    Tamamlanmadi,

    /// Sayi sozdizimi dogru ama semadaki araligin disinda.
    ///
    /// Ayri varyant: bu bir SOZDIZIMI hatasi degil, anlam hatasi. Aralik
    /// kisitini hane hane otomata gommek (ornegin 1..50 icin "ilk hane 1-5")
    /// gramerin boyunu kisitin degerinden cok daha fazla buyuturdu; kapiyi
    /// sayinin bittigi yerde tek seferde acmak dogru oran.
    #[error("sayi aralik disinda: {0}")]
    AralikDisi(String),
}
