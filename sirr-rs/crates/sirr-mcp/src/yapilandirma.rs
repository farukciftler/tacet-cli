//! Baglanti listesi: `~/.sirr/mcp.json`.
//!
//! VARSAYILAN BOS. Dosya yoksa hicbir baglanti yok demektir ve bu bir HATA
//! DEGILDIR — spec §2.1'in "varsayilan kapali" ilkesi: hic baglanti
//! eklenmemisse ag trafigi sifirdir. Kullanicinin bu makinede kosan MCP
//! sunuculari olabilir; sirr onlari KENDILIGINDEN bulmaya calismaz, kullanici
//! elle yazar.
//!
//! Bicim:
//!
//! ```json
//! {
//!   "baglantilar": [
//!     {
//!       "ad": "ev sunucusu",
//!       "url": "https://ornek.com/mcp",
//!       "anahtar": "tokeni buraya",
//!       "etkin": true
//!     }
//!   ]
//! }
//! ```
//!
//! `anahtar` ve `etkin` istege baglidir (`etkin` varsayilani `true`).
//!
//! ANAHTAR SAKLAMA UYARISI: iOS tarafinda token Keychain'de duruyor (§5.8).
//! Burada duz dosyada; masaustunde esdeger bir kasa yok. Bu yuzden dosya 0600
//! izinle olusturulmali ve `anahtar_yolu` ile bir ortam degiskenine
//! yonlendirilebiliyor — token'i git deposuna dusen bir dosyada tutmak
//! zorunda kalmayin.

use crate::hata::{MCPHatasi, MCPSonuc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaglantiAyari {
    /// Kullaniciya gorunen ad; cip metninin basinda bu yazar ("ev sunucusu · ...").
    pub ad: String,
    pub url: String,
    /// Bearer token — dosyada duz metin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anahtar: Option<String>,
    /// Token'i dosyaya yazmak istemeyenler icin: anahtari su ORTAM
    /// DEGISKENINDEN oku. `anahtar` ile ikisi de varsa bu kazanir.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anahtar_ortam: Option<String>,
    #[serde(default = "dogru")]
    pub etkin: bool,
}

fn dogru() -> bool {
    true
}

impl BaglantiAyari {
    /// Kullanilacak anahtar — once ortam degiskeni, sonra dosyadaki deger.
    pub fn cozulmus_anahtar(&self) -> Option<String> {
        let ortamdan = self
            .anahtar_ortam
            .as_ref()
            .and_then(|ad| std::env::var(ad).ok())
            .filter(|v| !v.is_empty());
        ortamdan.or_else(|| self.anahtar.clone().filter(|a| !a.is_empty()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Yapilandirma {
    #[serde(default)]
    pub baglantilar: Vec<BaglantiAyari>,
}

impl Yapilandirma {
    /// Yalniz etkin ve adresi kabul edilen baglantilar.
    ///
    /// Adres burada da suzulur ki kotu bir satir TUM dosyayi cope atmasin:
    /// bes baglantidan biri bozuksa digerleri calismaya devam etmeli.
    pub fn gecerliler(&self) -> Vec<&BaglantiAyari> {
        self.baglantilar
            .iter()
            .filter(|b| b.etkin && !b.ad.trim().is_empty())
            .filter(|b| crate::istemci::url_dogrula(&b.url).is_ok())
            .collect()
    }
}

/// Varsayilan yapilandirma yolu: `$SIRR_MCP_YAPILANDIRMA` ya da `~/.sirr/mcp.json`.
pub fn varsayilan_yol() -> Option<PathBuf> {
    if let Some(y) = std::env::var("SIRR_MCP_YAPILANDIRMA").ok().filter(|y| !y.is_empty()) {
        return Some(PathBuf::from(y));
    }
    std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".sirr").join("mcp.json"))
}

/// Dosyadan okur. **Dosya yoksa bos yapilandirma doner — hata degil.**
pub fn oku(yol: &Path) -> MCPSonuc<Yapilandirma> {
    let metin = match std::fs::read_to_string(yol) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Yapilandirma::default()),
        // Var ama okunamiyor (izin): bu SESSIZ gecilmemeli, kullanici baglanti
        // eklemis ve gorunmuyor olmasinin nedenini bilmeli.
        Err(_) => return Err(MCPHatasi::GecersizAdres(yol.display().to_string())),
    };
    ayristir(&metin)
}

/// Metinden ayristirir — saf, dosya sistemi istemez.
pub fn ayristir(metin: &str) -> MCPSonuc<Yapilandirma> {
    if metin.trim().is_empty() {
        return Ok(Yapilandirma::default());
    }
    serde_json::from_str(metin).map_err(|_| MCPHatasi::Bicimsiz)
}

/// Varsayilan yoldan okur; yol bile bulunamiyorsa bos doner.
pub fn varsayilandan_oku() -> MCPSonuc<Yapilandirma> {
    match varsayilan_yol() {
        Some(y) => oku(&y),
        None => Ok(Yapilandirma::default()),
    }
}

#[cfg(test)]
mod testler {
    use super::*;

    #[test]
    fn bos_metin_bos_yapilandirma() {
        assert_eq!(ayristir("").expect("bos"), Yapilandirma::default());
        assert!(ayristir("   \n").expect("bos").baglantilar.is_empty());
    }

    #[test]
    fn olmayan_dosya_hata_degil() {
        let yok = Path::new("/tmp/kesinlikle-olmayan-sirr-mcp-dosyasi.json");
        assert!(oku(yok).expect("hata olmamali").baglantilar.is_empty());
    }

    #[test]
    fn tam_kayit_ayristirilir() {
        let y = ayristir(
            r#"{"baglantilar":[{"ad":"ev","url":"https://ornek.com/mcp","anahtar":"abc"}]}"#,
        )
        .expect("ayristirilmali");
        assert_eq!(y.baglantilar.len(), 1);
        assert_eq!(y.baglantilar[0].ad, "ev");
        assert!(y.baglantilar[0].etkin, "etkin varsayilani true olmali");
        assert_eq!(y.baglantilar[0].cozulmus_anahtar().as_deref(), Some("abc"));
    }

    #[test]
    fn bozuk_json_bicimsiz() {
        assert_eq!(ayristir("{bu json degil").unwrap_err(), MCPHatasi::Bicimsiz);
    }

    #[test]
    fn gecersizler_suzulur_digerleri_kalir() {
        let y = ayristir(
            r#"{"baglantilar":[
                {"ad":"iyi","url":"https://a.com/mcp"},
                {"ad":"kapali","url":"https://b.com/mcp","etkin":false},
                {"ad":"kotu-adres","url":"http://uzak.com/mcp"},
                {"ad":"","url":"https://c.com/mcp"}
            ]}"#,
        )
        .expect("ayristirilmali");
        let gecerli = y.gecerliler();
        assert_eq!(gecerli.len(), 1);
        assert_eq!(gecerli[0].ad, "iyi");
    }
}
