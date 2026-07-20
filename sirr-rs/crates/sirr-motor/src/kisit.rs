//! Kisitlayici — uretimi bir dilbilgisine ZORLAYAN kapi.
//!
//! BAGIMLILIK YONU: sirr-motor, sirr-gramer'e BAGIMLI DEGILDIR. Sozlesme
//! burada durur, sirr-gramer onu UYGULAR (motor -> gramer degil, gramer ->
//! motor). Gerekcesi: motor "hangi belirtec yasak" sorusunun cevabini uretmez,
//! yalnizca sorar. Bagimlilik ters yone kurulsaydi motor gramerin ic
//! temsilini (DFA, yigin, token maskesi bit vektoru) bilmek zorunda kalir ve
//! gramer degistikce motor da degisirdi. Boylece ayrica: kisitsiz calisan
//! motor (serbest metin turu) gramer kodunu hic derlemez.
//!
//! IKI KATMAN, cunku gramer ARTIMLIDIR: `Kisitlayici` uretim boyunca yasayan
//! paylasilabilir tanimdir (`&self`, `Sync`); `KisitOturumu` ise TEK bir
//! uretimin ilerleyen durumudur. Tek katman olsaydi ya her belirtecte durum
//! bastan hesaplanirdi (uzunlukta karesel maliyet) ya da tanimin kendisi
//! degisken olur, ayni kisiti iki uretimde paylasmak imkansizlasirdi.

use crate::hata::MotorHatasi;

/// Bir uretimi kisitlayan dilbilgisinin tanimi. Paylasilir, degismez.
pub trait Kisitlayici: Send + Sync {
    /// Yeni bir uretim icin taze durum acar.
    fn oturum(&self) -> Box<dyn KisitOturumu>;

    /// Tanilama/log icin kisa ad ("arac_cagrisi", "json").
    fn ad(&self) -> &str {
        "kisit"
    }
}

/// Tek bir uretimin ilerleyen kisit durumu. Uretim dongusuyle birlikte yasar.
pub trait KisitOturumu: Send {
    /// Su anki durumda YASAK olan belirteclerin logit'ini `f32::NEG_INFINITY`
    /// yapar. Ornekleyici bundan sonra calisir; boylece hangi ornekleme
    /// stratejisi secilirse secilsin (greedy, top-p, sicaklik) kisit delinmez.
    ///
    /// `logits` dilimi kelime dagarciginin TAMAMI kadar uzundur; indeks =
    /// belirtec kimligi.
    fn maskele(&self, logits: &mut [f32]);

    /// Secilen belirteci duruma isler. Maskeleme dogru calistiysa buraya
    /// yasak bir belirtec gelmez; yine de hata donebilir cunku maskeyi
    /// atlayan cagri yerleri (ornekleyici hatasi, elle beslenen belirtec)
    /// sessizce yanlis bir kabule yol acmamali.
    fn ilerlet(&mut self, belirtec: u32) -> Result<(), MotorHatasi>;

    /// Dilbilgisi kabul durumunda mi — uretim burada GUVENLE kesilebilir.
    fn bitti_mi(&self) -> bool;
}

/// Hicbir seyi kisitlamayan uygulama.
///
/// Var olma sebebi `Option<&dyn Kisitlayici>`'i sadelestirmek degil (o zaten
/// var); serbest metin turunu kisitli turla AYNI kod yolundan gecirebilmek.
/// Iki ayri dongu yazilsaydi ornekleme/durdurma hatalari yalniz birinde
/// duzelirdi.
pub struct SerbestKisit;

impl Kisitlayici for SerbestKisit {
    fn oturum(&self) -> Box<dyn KisitOturumu> {
        Box::new(SerbestOturum)
    }
    fn ad(&self) -> &str {
        "serbest"
    }
}

struct SerbestOturum;

impl KisitOturumu for SerbestOturum {
    fn maskele(&self, _logits: &mut [f32]) {}
    fn ilerlet(&mut self, _belirtec: u32) -> Result<(), MotorHatasi> {
        Ok(())
    }
    fn bitti_mi(&self) -> bool {
        false
    }
}
