//! Arac sozlesmesi.
//!
//! KARAR — ASYNC BICIMI: `async fn calistir(...)` (AFIT, edition 2024) DENENDI
//! ve REDDEDILDI. AFIT'li bir trait dyn-uyumlu degildir; oysa `AracKatalogu`
//! tam olarak `Vec<Arc<dyn Arac>>` tutmak zorunda — araclar calisma aninda ad
//! ile bulunur ve heterojen bir listede yasar. Bu yuzden `calistir` elle
//! `Pin<Box<dyn Future<Output = AracSonucu> + Send + '_>>` doner. `async-trait`
//! bagimliligi EKLENMEDI; makro tam da bu donusumu yapiyor, biz de bir satirlik
//! yardimci (`kutula`) ile ayni seyi bagimliliksiz yapiyoruz.
//!
//! `Send` sinirinin gerekcesi: motor arac cagrilarini cok-is-parcacikli bir
//! calistiricida yurutecek.

use crate::baglam::AracBaglami;
use crate::sema::ArgSema;
use crate::sonuc::AracSonucu;
use std::future::Future;
use std::pin::Pin;

/// `calistir`in donus tipi. Omur `'a` baglama ve `self`e baglidir.
pub type AracGelecegi<'a> = Pin<Box<dyn Future<Output = AracSonucu> + Send + 'a>>;

/// Bir `async` blogu `AracGelecegi`ne cevirir — uygulamalar `calistir` govdesini
/// `kutula(async move { ... })` icine alir ve async/await ergonomisini korur.
pub fn kutula<'a, F>(gelecek: F) -> AracGelecegi<'a>
where
    F: Future<Output = AracSonucu> + Send + 'a,
{
    Box::pin(gelecek)
}

pub trait Arac: Send + Sync {
    /// Modelin cagirdigi ad. ASCII, snake_case, oturum boyunca sabit.
    fn ad(&self) -> &str;

    /// Modele gosterilen kisa tarif: NE ZAMAN kullanilacagini anlatir.
    fn aciklama(&self) -> &str;

    /// Argüman sozlesmesi. Model bu semaya ZORLANIR (bkz. sirr-gramer).
    fn sema(&self) -> ArgSema;

    /// Bu arac kisisel veri okuyor mu — onay kapisi bunu okur.
    ///
    /// Varsayilan `false`: yeni bir arac yazan kisi bilerek "evet" demeli.
    /// Ters varsayilan her araci gereksiz onay sorusuna sokar, onay sik
    /// gorulunce okunmaz hale gelir ve kapi islevini yitirir.
    fn kirletir_mi(&self) -> bool {
        false
    }

    /// Isin kendisi. HATA DONMEZ: her yol bir `AracSonucu` uretir, cunku
    /// hata da kullaniciya cip, modele sabit metin olarak donmelidir.
    /// Uygulamalar ic hatalarini `AracSonucu::basarisiz(&hata)` ile cevirir.
    fn calistir<'a>(
        &'a self,
        args: serde_json::Value,
        ctx: &'a mut AracBaglami,
    ) -> AracGelecegi<'a>;
}
