//! sirr-gramer — `ArgSema`yi sart-kosullu uretim (constrained decoding)
//! gramerine cevirir.
//!
//! NEDEN VAR: Swift tarafinda Apple FoundationModels, `DynamicGenerationSchema`
//! ile modeli gecerli JSON uretmeye ZORLUYORDU. Saf Rust'a gectigimizde o
//! zorlamayi kaybettik; kucuk bir model (3B) serbest birakildiginda arac
//! cagirmak yerine "sunu yapayim mi?" diye anlatmaya basliyor — bu regresyon
//! bir kez yasandi. Dolayisiyla burasi konfor katmani degil, arac cagirmanin
//! calisma sartidir.
//!
//! AKIS:
//!   ArgSema --derle--> Gramer (degismez, Arc ile paylasilir)
//!   Gramer --durum()--> GramerDurumu (asagi-itmeli otomat, ucuz klonlanir)
//!   GramerDurumu + dagarcik --TokenMaskesi--> Vec<bool> (izinli token'lar)
//!
//! Ornek:
//! ```
//! use sirr_cekirdek::{Alan, ArgSema};
//! use sirr_gramer::Gramer;
//!
//! let sema = ArgSema::nesne(vec![
//!     Alan::yeni("sorgu", ArgSema::metin()).zorunlu(),
//!     Alan::yeni("kapsam", ArgSema::secenek(["tumu", "yakin"])),
//! ]);
//! let gramer = Gramer::derle(&sema);
//! let mut durum = gramer.durum();
//! durum.ilerlet(r#"{"sorgu":"rapor"}"#).unwrap();
//! assert!(durum.bitti_mi());
//! ```
//!
//! AG YOK, DOSYA YOK: bu crate saf hesaptir.

mod cagri;
mod derleme;
mod durum;
mod hata;
mod izin;
mod maske;

pub use cagri::CagriKisiti;
pub use derleme::Gramer;
pub use durum::GramerDurumu;
pub use hata::GramerHatasi;
pub use izin::IzinKumesi;
pub use maske::TokenMaskesi;

#[cfg(test)]
mod testler;
