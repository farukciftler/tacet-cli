//! sirr-web — kullanicinin kendi SearXNG ornegi uzerinden web aramasi.
//!
//! AG TEKELI (mimari kural): sirr'de ag cagrisi YALNIZ bu crate'te ve
//! `sirr-mcp`te bulunur. Baska hicbir crate soket acmaz, HTTP istemcisi
//! kurmaz, `ureq`/`reqwest` cekmez. Kuralin degeri denetlenebilirligindedir:
//! "kullanicinin sorgusu disariya nereden cikiyor" sorusunun cevabi tek
//! dosyadir (`istemci.rs`).
//!
//! BU CRATE `sirr-cekirdek`I BILMEZ. Arac sozlesmesi, cip, `AracSonucu`,
//! `VeriDeposu` — hicbiri burada gecmez. Ceviri `sirr-araclar/src/web_ara.rs`
//! sinirinda yapilir. Boylece ag katmani, arac mimarisi degistiginde
//! degismez; ve aga cikmayan bir cagiran (teshis komutu) cekirdegi cekmez.
//!
//! KATMANLAR:
//! - `istemci` — TEK ag yuzeyi (`ureq`), URL kurulumu, zaman asimi, adres kapisi.
//! - `sonuc`   — SearXNG JSON'unun yorumu. AG YOK, girdisi `&str`, tamami test edilebilir.
//! - `metin`   — HTML → duz metin. Bilerek basit; DOM ayristiricisi degil.
//! - `hata`    — her ariza hali AYRI varyant (bkz. oradaki gerekce).
//!
//! GIZLILIK: sorgu da veridir. Bu crate sorguyu hicbir yere KAYDETMEZ, log
//! basmaz ve diske yazmaz; yalnizca istegi kurar ve yanit doner. Kirli
//! oturumda sorgunun kullanici onayindan gecmesi cagiranin isidir
//! (`sirr-araclar` + `AracYurutucu` onay kapisi).

pub mod hata;
pub mod istemci;
pub mod metin;
pub mod sonuc;

pub use hata::{WebHatasi, WebSonuc};
pub use istemci::{
    ADRES_DEGISKENI, VARSAYILAN_ADRES, VARSAYILAN_ZAMAN_ASIMI, WebAramaIstemcisi,
};
pub use metin::metinlestir;
pub use sonuc::{AramaSonucu, alan_adi, ayristir, kelime_sinirinda_kirp};
