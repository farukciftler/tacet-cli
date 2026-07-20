//! Web katmaninin hata tipi.
//!
//! NEDEN AYRI BIR ENUM (`AracHatasi`'ni dogrudan kullanmak yerine): bu crate
//! `sirr-cekirdek`e bagimli DEGIL ve olmamali. Ag kodunun arac sozlesmesini
//! bilmesi, yarin ag disi bir cagiranin (ornegin bir CLI teshis komutu)
//! cekirdegi de cekmesini zorunlu kilardi. Ceviri `sirr-araclar` tarafinda,
//! arac sinirinda yapilir — orasi zaten iki dunyayi da tanir.
//!
//! HER HAL AYRI VARYANT: "ag hatasi" diye tek bir kova, kullaniciya "bir seyler
//! ters gitti" demekten baska bir sey uretmez. Sunucu ayakta ama JSON kapali
//! olmasi (SearXNG'nin bilinen tuzagi: `settings.yml` icinde `formats: json`)
//! ile ag olmamasi bambaska iki durumdur ve kullanicinin yapacagi sey de
//! bambaskadir.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebHatasi {
    /// Sunucuya hic ulasilamadi: DNS cozulmedi, baglanti reddedildi, TLS kurulmadi.
    UlasilamadI(String),
    /// Baglanti kuruldu ama yanit zaman asimi suresinde gelmedi.
    ZamanAsimi,
    /// HTTP 4xx/5xx. Kodu tasir: 404 yanlis yol, 429 hiz siniri, 5xx sunucu.
    SunucuKodu(u16),
    /// Yanit geldi ama JSON degil ya da bekledigimiz sekilde degil.
    ///
    /// SearXNG'de en sik sebebi `formats: json` kapali olmasidir; sunucu o
    /// halde HTML doner ve "200 OK" gorundugu icin sessizce yanlis olur.
    GecersizJson(String),
    /// Sorgu gitti, sunucu duzgun cevapladi, ama `results[]` bos.
    /// Hata degil ama basari da degil — model bunu UYDURMAMALI (bkz. `no_results`).
    BosSonuc,
    /// Yapilandirilan taban adres kullanilabilir degil.
    GecersizAdres(String),
}

impl fmt::Display for WebHatasi {
    /// Bu metinler KULLANICIYA gider (cip detayi, CLI teshisi) — Turkce ve
    /// eylem onerir. Modele giden metin bu degildir; o `sirr-araclar`da
    /// `AracSonucu::basarisiz` ile sabit Ingilizceye cevrilir.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WebHatasi::UlasilamadI(_) => f.write_str("Arama sunucusuna ulasilamadi."),
            WebHatasi::ZamanAsimi => f.write_str("Arama cok uzun surdu."),
            WebHatasi::SunucuKodu(k) => write!(f, "Arama sunucusu yanit vermedi ({k})."),
            WebHatasi::GecersizJson(_) => {
                f.write_str("Sunucu JSON dondurmedi; ayarlarinda 'formats: json' acik olmali.")
            }
            WebHatasi::BosSonuc => f.write_str("Arama sonuc dondurmedi."),
            WebHatasi::GecersizAdres(_) => f.write_str("Arama sunucusu adresi gecersiz."),
        }
    }
}

impl std::error::Error for WebHatasi {}

pub type WebSonuc<T> = Result<T, WebHatasi>;
