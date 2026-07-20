//! Raporlayici — araclarin cip yasam dongusunu bildirdigi arayuz.
//!
//! Cip metnini ARAC uretir, model degil: model gorunen adimi haluisine
//! edemesin diye. Ekranda gorunen her satir kodda gercekten olmus bir olaydir.
//!
//! `&self` alir (`&mut self` degil): ayni cip akisini birden fazla arac ve
//! gorev es zamanli besleyebilir; ic esitleme somut uygulamanin isidir.

use crate::durum::AracDurumu;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Bir cipin kimligi. `u64` — uuid bagimligi eklemeye deger bir kazanc yok,
/// benzersizlik tek surec icinde yeterli.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IzKimligi(pub u64);

/// Cip guncellemesi. Verilmeyen (None) alanlar KORUNUR — cagri yerinin
/// bilmedigi bir alani sifirlamasi, iki farkli asamanin birbirinin bilgisini
/// silmesine yol acardi.
#[derive(Debug, Clone, Default)]
pub struct IzGuncelleme {
    pub durum: Option<AracDurumu>,
    pub metin: Option<String>,
    pub ham_girdi: Option<String>,
    pub ham_cikti: Option<String>,
    pub dosya_yolu: Option<PathBuf>,
}

impl IzGuncelleme {
    pub fn durum(durum: AracDurumu) -> Self {
        Self { durum: Some(durum), ..Default::default() }
    }

    pub fn metin(mut self, metin: impl Into<String>) -> Self {
        self.metin = Some(metin.into());
        self
    }

    pub fn ham_girdi(mut self, v: impl Into<String>) -> Self {
        self.ham_girdi = Some(v.into());
        self
    }

    pub fn ham_cikti(mut self, v: impl Into<String>) -> Self {
        self.ham_cikti = Some(v.into());
        self
    }

    pub fn dosya_yolu(mut self, v: impl Into<PathBuf>) -> Self {
        self.dosya_yolu = Some(v.into());
        self
    }
}

/// Tam bir cip kaydi — gorunum katmani bunu okur.
#[derive(Debug, Clone)]
pub struct AracIzi {
    pub id: IzKimligi,
    pub ikon: String,
    pub metin: String,
    pub durum: AracDurumu,
    pub ham_girdi: Option<String>,
    pub ham_cikti: Option<String>,
    pub dosya_yolu: Option<PathBuf>,
}

pub trait Raporlayici: Send + Sync {
    /// "Calisiyor" cipi dusurur, kimligini doner.
    fn baslat(&self, ikon: &str, metin: &str) -> IzKimligi;
    /// Cipi gunceller. Bilinmeyen kimlik sessizce yok sayilir (tur iptali
    /// sonrasi gec kalan bildirim panik sebebi olmamali).
    fn guncelle(&self, id: IzKimligi, guncelleme: IzGuncelleme);
}

/// Cip biriktiren varsayilan uygulama. Baslikta UI yok; motor ve eval
/// dogrudan bunu kullanir, gorunum katmani izleri okur.
#[derive(Default)]
pub struct IzToplayici {
    sayac: AtomicU64,
    izler: Mutex<Vec<AracIzi>>,
}

impl IzToplayici {
    pub fn yeni() -> Self {
        Self::default()
    }

    pub fn izler(&self) -> Vec<AracIzi> {
        self.izler.lock().expect("iz kilidi").clone()
    }

    /// Yeni tur — onceki turun cipleri tasindiktan sonra.
    pub fn sifirla(&self) {
        self.izler.lock().expect("iz kilidi").clear();
    }

    /// Bu turda `Yazildi` cipi dustu mu — motorun yeniden deneme kapisi.
    pub fn dunya_degisti(&self) -> bool {
        self.izler
            .lock()
            .expect("iz kilidi")
            .iter()
            .any(|i| i.durum.dunyayi_degistirdi())
    }
}

impl Raporlayici for IzToplayici {
    fn baslat(&self, ikon: &str, metin: &str) -> IzKimligi {
        let id = IzKimligi(self.sayac.fetch_add(1, Ordering::Relaxed));
        self.izler.lock().expect("iz kilidi").push(AracIzi {
            id,
            ikon: ikon.to_string(),
            metin: metin.to_string(),
            durum: AracDurumu::Calisiyor,
            ham_girdi: None,
            ham_cikti: None,
            dosya_yolu: None,
        });
        id
    }

    fn guncelle(&self, id: IzKimligi, g: IzGuncelleme) {
        let mut izler = self.izler.lock().expect("iz kilidi");
        let Some(iz) = izler.iter_mut().find(|i| i.id == id) else {
            return;
        };
        if let Some(d) = g.durum {
            iz.durum = d;
        }
        if let Some(v) = g.metin {
            iz.metin = v;
        }
        if let Some(v) = g.ham_girdi {
            iz.ham_girdi = Some(v);
        }
        if let Some(v) = g.ham_cikti {
            iz.ham_cikti = Some(v);
        }
        if let Some(v) = g.dosya_yolu {
            iz.dosya_yolu = Some(v);
        }
    }
}

/// Cip istemeyen cagri yerleri (eval, birim testleri) icin bos raporlayici.
pub struct SessizRaporlayici;

impl Raporlayici for SessizRaporlayici {
    fn baslat(&self, _ikon: &str, _metin: &str) -> IzKimligi {
        IzKimligi(0)
    }
    fn guncelle(&self, _id: IzKimligi, _g: IzGuncelleme) {}
}
