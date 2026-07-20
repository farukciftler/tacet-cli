//! MotorSaglayici — model calistiricisinin sozlesmesi.
//!
//! ASYNC BICIMI: `Arac`taki karar burada da gecerli — elle
//! `Pin<Box<dyn Future + Send + '_>>`. Gerekce ayni: motor calisma aninda
//! secilir (SahteMotor mu, CandleMotor mu) ve `Arc<dyn MotorSaglayici>` olarak
//! tasinir; AFIT'li bir trait dyn-uyumlu olmadigi icin bu imkansiz olurdu.
//! `async-trait` yine EKLENMEDI; `kutula_uretim` ayni donusumu bir satirda
//! bagimliliksiz yapiyor.

use crate::hata::MotorSonuc;
use crate::istem::Istem;
use crate::kisit::Kisitlayici;
use std::future::Future;
use std::pin::Pin;

/// Uretimin nasil bittigi. Cagri yeri buna gore davranir: `Uzunluk` ile biten
/// bir arac cagrisi YARIM JSON'dur, ayristirilmadan atilmali.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitisNedeni {
    /// Model bitis belirteci uretti — dogal son.
    Belirtec,
    /// Kisitlayici kabul durumuna ulasti; dilbilgisi tamamlandi.
    KisitTamam,
    /// Belirtec tavanina carpildi. Cikti YARIM olabilir.
    Uzunluk,
    /// Durdurma dizgisi gorundu.
    Dizgi,
}

impl BitisNedeni {
    /// Cikti eksiksiz mi — `Uzunluk` disinda her son tamdir.
    pub fn tam_mi(self) -> bool {
        !matches!(self, BitisNedeni::Uzunluk)
    }
}

/// Bir uretimin sonucu.
#[derive(Debug, Clone)]
pub struct Uretim {
    pub metin: String,
    pub belirtec_sayisi: usize,
    pub bitis: BitisNedeni,
}

impl Uretim {
    pub fn yeni(metin: impl Into<String>, belirtec_sayisi: usize, bitis: BitisNedeni) -> Self {
        Self { metin: metin.into(), belirtec_sayisi, bitis }
    }
}

/// Ornekleme ayarlari. Varsayilan GREEDY (sicaklik 0): eval'in tekrarlanabilir
/// olmasi ornekleme kararindan once gelir; rastgelelik bilerek acilir.
#[derive(Debug, Clone, Copy)]
pub struct OrneklemeAyari {
    pub sicaklik: f32,
    /// Top-p (cekirdek ornekleme). 1.0 = kapali.
    pub top_p: f32,
    /// Uretilecek en fazla belirtec.
    pub en_cok_belirtec: usize,
    /// Rastgelelik tohumu — ayni tohum + ayni istem = ayni cikti.
    pub tohum: u64,
}

impl Default for OrneklemeAyari {
    fn default() -> Self {
        Self { sicaklik: 0.0, top_p: 1.0, en_cok_belirtec: 512, tohum: 0 }
    }
}

pub type UretimGelecegi<'a> = Pin<Box<dyn Future<Output = MotorSonuc<Uretim>> + Send + 'a>>;

/// Bir `async` blogunu `UretimGelecegi`ne cevirir (bkz. cekirdekteki `kutula`).
pub fn kutula_uretim<'a, F>(gelecek: F) -> UretimGelecegi<'a>
where
    F: Future<Output = MotorSonuc<Uretim>> + Send + 'a,
{
    Box::pin(gelecek)
}

pub trait MotorSaglayici: Send + Sync {
    /// Tanilama/log adi ("sahte", "candle").
    fn ad(&self) -> &str;

    /// Belirtec kimligi -> metin tablosu; kisit kurmak icin GEREKLI.
    ///
    /// NEDEN MOTORDA: maske belirtec kimligi uzerinden konusur, kimlikleri ise
    /// yalniz belirtecleyici bilir. Kisiti kuran taraf (CLI, eval) dagarcigi
    /// kendi uydursaydi maske baska bir modelin kimliklerini maskeler, yani
    /// sessizce YANLIS token'lari kapatirdi.
    ///
    /// `None` = bu motorun dagarcigi yok/bilinmiyor; cagri yeri o zaman
    /// kisitsiz uretir. Varsayilan `None`: dagarcigini bildirmeyen bir motor
    /// kisiti sessizce bozmaktansa hic kullanmamali.
    fn dagarcik(&self) -> Option<Vec<String>> {
        None
    }

    /// Istemi uretime cevirir.
    ///
    /// `kisit` verilirse HER adimda logits'e uygulanir; ornekleme ondan SONRA
    /// calisir, boylece hicbir ornekleme stratejisi kisiti delemez.
    fn uret<'a>(
        &'a self,
        istem: &'a Istem,
        kisit: Option<&'a dyn Kisitlayici>,
        ayar: OrneklemeAyari,
    ) -> UretimGelecegi<'a>;
}
