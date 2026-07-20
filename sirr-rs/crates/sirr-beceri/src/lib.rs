//! sirr-beceri — beceri (skill) katmani; Claude'un SKILL.md mantiginin
//! cihaz-ustu karsiligi.
//!
//! Her aracin ayrintili kullanim kilavuzu bir `.md` dosyasidir (frontmatter +
//! govde). 4096 token penceresini sismemek icin "progressive disclosure":
//! hepsi birden degil, o anki mesaja uyan TEK beceri, o TURUN istemine
//! iliştirilir.
//!
//! ## Degistirilmeyecek kararlar (Swift tarafinda OLCULEREK alindi)
//!
//! 1. **Beceriler sistem talimatina GOMULMEZ.** Gomulunce kucuk model araci
//!    CAGIRMAK yerine kilavuzu ANLATMAYA basladi; regresyonun kaynagi oydu.
//!    Sabit talimat kisa kalir.
//! 2. **Tek beceri, tur bazli enjeksiyon**, `<guidance>` citiyle, 700
//!    karakterle sinirli. Kullanici becerisi govdesi 500.
//! 3. **Puan = eslesen tetikleyicilerin UZUNLUK toplami**, adet degil.
//!    Adet sayilirsa beraberlikte sirayi rastgelelik belirler.
//!
//! ## Kabuga birakilan is
//!
//! Bu crate ISTEMI KURMAZ. Kabuk fazi `depo.eslesen(mesaj, araclar)` ile
//! beceriyi secer, `EnjeksiyonDurumu` ile bu turda gerekli mi diye bakar ve
//! `enjeksiyon_metni(...)` ciktisini `sirr_motor::Istem::kilavuzla`ya verir.
//! Boyle ayrildi cunku istem kurulumu motorun isi, beceri secimi bu katmanin.
//!
//! AG YOK: burada yalniz yerel dosya okunur.

pub mod beceri;
pub mod depo;
pub mod enjeksiyon;
pub mod eslesme;

pub use beceri::{Beceri, KULLANICI_GOVDE_SINIRI, ayristir};
pub use depo::BeceriDeposu;
pub use enjeksiyon::{
    CEKIRDEK_ISARETI, ENJEKSIYON_SINIRI, EnjeksiyonDurumu, cekirdek_ayir, enjeksiyon_govdesi,
    enjeksiyon_metni,
};
pub use eslesme::{TAM_SOZCUK_SINIRI, icerir, kucult, puan};

/// Kullanici becerilerinin okundugu dizin: `~/.sirr/beceriler`.
///
/// `SIRR_EV` ortam degiskeni ev dizinini gecersiz kilar — testler ve gelistirici
/// kabugu gercek ev dizinini kirletmeden calisabilsin diye.
pub fn kullanici_dizini() -> Option<std::path::PathBuf> {
    let ev = std::env::var_os("SIRR_EV")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".sirr")))?;
    Some(ev.join("beceriler"))
}

#[cfg(test)]
mod testler {
    use super::*;

    #[test]
    fn uctan_uca_secim_ve_enjeksiyon() {
        // Bu katmanin sozlesmesi: mesaj -> beceri -> citli metin, 700 icinde.
        let depo = BeceriDeposu::varsayilan();
        let araclar: Vec<String> = ["hesapla".into(), "belge_olustur".into()].into();
        let b = depo.eslesen("125 carpi 8 kac eder", Some(&araclar)).unwrap();
        assert_eq!(b.ad, "hesap");

        let metin = enjeksiyon_metni(b);
        assert!(metin.contains("<guidance name=\"hesap\">"));
        assert!(metin.contains("never compute in your head"));
        assert!(metin.chars().count() < ENJEKSIYON_SINIRI + 200, "cit + govde");
    }

    #[test]
    fn kullanici_dizini_sirr_ev_ile_yonlendirilir() {
        // SAFETY: tek is parcacikli test; ortam degiskeni yalniz burada okunur.
        unsafe { std::env::set_var("SIRR_EV", "/tmp/sirr-testi-ev") };
        assert_eq!(
            kullanici_dizini().unwrap(),
            std::path::PathBuf::from("/tmp/sirr-testi-ev/beceriler")
        );
        unsafe { std::env::remove_var("SIRR_EV") };
    }
}
