//! sirr-hafiza — kalici baglam katmani: kullanici tercihleri ve olgular.
//!
//! Bulut asistanlarinda hafiza bir gizlilik endisesidir; burada markanin
//! kendisidir: **cihazdan cikmayan hafiza**. Dosya `~/.sirr/hafiza.json`,
//! duz JSON, 0600. Hicbir ag yuzeyi yok.
//!
//! ## Temel karar: AYIKLAMA MODELE, HATIRLAMA KODA
//!
//! Bir notun nasil dogdugu (metin, tur, anahtarlar) modele sorulabilir — ama
//! hangi notun hangi tura girecegine DETERMINISTIK KOD karar verir
//! (`HafizaDeposu::eslesen`). Boylece ayni mesaj her zaman ayni notu getirir
//! ve davranis sinanabilir kalir.
//!
//! ## Ilkeler
//!
//! 1. **Sessizce ogrenme yok.** Kaydedilen her not listelenebilir, guncellenebilir
//!    ve silinebilir. Tavan dolunca OTOMATIK DUSURME yapilmaz — sessizce silmek,
//!    sessizce ogrenmenin simetrigidir ve ayni olcude yasaktir.
//! 2. **Butce kutsaldir.** Enjeksiyon `ENJEKSIYON_SINIRI` (600 karakter, ~200
//!    token) ile SERT tavanlidir ve beceri enjeksiyonuyla (700) ayni tura denk
//!    gelebilecegi hesaba katilmistir. Tum hafizayi her isteme gomen tasarim
//!    4096 baglami oldurur.
//! 3. **Az ama dogru.** Gurultulu elli nottansa dogru on not; filtreler
//!    agresif, tavanlar dusuk. Supheda kaydetme.
//!
//! ## Kabuga birakilan is
//!
//! Bu crate ISTEMI KURMAZ. Kabuk fazi `depo.enjeksiyon_metni(mesaj)` ciktisini
//! (varsa) `NotIzleyici` ile suzup o turun isteminin basina koyar.

pub mod depo;
pub mod not;

pub use depo::{
    EN_COK_NOT, ENJEKSIYON_SINIRI, HafizaDeposu, HafizaHatasi, NotIzleyici,
};
pub use not::{
    ANAHTAR_SINIRI, EN_AZ_METIN, HafizaNotu, HafizaTuru, METIN_SINIRI, TOPLAM_TAVAN,
    anahtarlari_duzelt,
};

#[cfg(test)]
mod testler {
    use super::*;

    #[test]
    fn uctan_uca_hatirla_hatirla_unut() {
        // Bu katmanin sozlesmesi: olgu gir -> ilgili mesajda geri gelsin ->
        // "unut" dendiginde gitsin.
        let mut d = HafizaDeposu::bellekte();
        d.ekle(
            "Kullanici vejetaryendir.",
            HafizaTuru::Tercih,
            &["yemek".to_string(), "restoran".to_string()],
        )
        .unwrap();

        let istem = d.enjeksiyon_metni("aksamki restoran icin oneri ver").unwrap();
        assert!(istem.contains("Kullanici vejetaryendir."));
        assert!(istem.chars().count() <= ENJEKSIYON_SINIRI);

        assert_eq!(d.unut("vejetaryen"), 1);
        assert!(d.enjeksiyon_metni("aksamki restoran icin oneri ver").is_none());
    }

    #[test]
    fn alakasiz_mesaj_butceden_hicbir_sey_yemez() {
        let mut d = HafizaDeposu::bellekte();
        for i in 0..TOPLAM_TAVAN {
            d.ekle(
                &format!("Kullanici {i} numarali olguyu soyledi."),
                HafizaTuru::Olgu,
                &["olgu".to_string()],
            )
            .unwrap();
        }
        // 50 not dolu ama mesaj hicbirine uymuyor: istem BUYUMEMELI.
        assert!(d.enjeksiyon_metni("iki kere iki kac eder").is_none());
    }
}
