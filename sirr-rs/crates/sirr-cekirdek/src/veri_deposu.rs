//! VeriDeposu — 4096 token bypass kanalinin kalbi.
//!
//! Kural: toplu cihaz verisi MODELDEN GECMEZ. Arac veriyi buraya koyar, modele
//! yalniz kisa ozet + `kaynak_ref` doner. Sonraki adimda veriye ihtiyac duyan
//! yine bir ARACtir; veriyi depodan referansla alir. Boylece 100 takvim kaydi
//! ya da 40 sayfalik belge baglami sismeden islenebilir.
//!
//! Trait olarak tanimli cunku somut depo (bellek, disk, sifreli) sonraki
//! fazlarda degisecek; araclar bu sozlesmeden fazlasini bilmemeli.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;

/// Depodaki bir kaydin adresi. Modele giden TEK veri parcasi budur; icerigi
/// hakkinda ipucu tasimaz (kisisel veri sizmasin diye sadece tur + sira).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KaynakRef(pub String);

impl KaynakRef {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for KaynakRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Depoya konan tek kayit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Kayit {
    pub kaynak_ref: KaynakRef,
    /// Kaba tur etiketi ("takvim", "belge", "dosya") — kaynak_ref uretiminde
    /// ve araclarin filtrelemesinde kullanilir.
    pub tur: String,
    /// Modele gosterilmesi GUVENLI kisa ozet ("12 etkinlik, 3 gun").
    pub ozet: String,
    /// Asil govde. Buraya ne konursa konsun modele dogrudan verilmez.
    pub govde: String,
}

pub trait VeriDeposu: Send + Sync {
    /// Veriyi saklar ve referansini doner.
    fn koy(&self, tur: &str, ozet: &str, govde: String) -> KaynakRef;
    fn al(&self, kaynak_ref: &KaynakRef) -> Option<Kayit>;
    /// Verilen turdeki kayitlar, ekleme sirasiyla.
    fn turden(&self, tur: &str) -> Vec<Kayit>;
    /// Sohbet sifirlaninca cagrilir — oturum omurlu veri disari tasmasin.
    fn temizle(&self);
}

/// Varsayilan somut depo: surec omurlu, bellekte. Diske hicbir sey yazmaz —
/// cihaz verisi surec bitince kendiliginden yok olur.
#[derive(Default)]
pub struct BellekVeriDeposu {
    ic: Mutex<Ic>,
}

#[derive(Default)]
struct Ic {
    kayitlar: Vec<Kayit>,
    sayac: u64,
}

impl BellekVeriDeposu {
    pub fn yeni() -> Self {
        Self::default()
    }
}

impl VeriDeposu for BellekVeriDeposu {
    fn koy(&self, tur: &str, ozet: &str, govde: String) -> KaynakRef {
        let mut ic = self.ic.lock().expect("veri deposu kilidi");
        ic.sayac += 1;
        let kaynak_ref = KaynakRef(format!("{tur}#{}", ic.sayac));
        let kayit = Kayit {
            kaynak_ref: kaynak_ref.clone(),
            tur: tur.to_string(),
            ozet: ozet.to_string(),
            govde,
        };
        ic.kayitlar.push(kayit);
        kaynak_ref
    }

    fn al(&self, kaynak_ref: &KaynakRef) -> Option<Kayit> {
        let ic = self.ic.lock().expect("veri deposu kilidi");
        ic.kayitlar.iter().find(|k| &k.kaynak_ref == kaynak_ref).cloned()
    }

    fn turden(&self, tur: &str) -> Vec<Kayit> {
        let ic = self.ic.lock().expect("veri deposu kilidi");
        ic.kayitlar.iter().filter(|k| k.tur == tur).cloned().collect()
    }

    fn temizle(&self) {
        let mut ic = self.ic.lock().expect("veri deposu kilidi");
        ic.kayitlar.clear();
        // Sayac SIFIRLANMAZ: eski bir kaynak_ref elde kalmissa yeni bir kayda
        // denk gelip yanlis veriyi acmasin.
    }
}
