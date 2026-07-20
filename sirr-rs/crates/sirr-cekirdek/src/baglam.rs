//! AracBaglami — araclarin paylastigi calisma durumu.
//!
//! Araclara tek bir `&mut AracBaglami` gecirilir; kuresel durum yok. Boylece
//! eval ayni araci sahte depo ve sessiz raporlayiciyla, uygulama gercekleriyle
//! calistirir; arac ikisini ayirt etmez.

use crate::hata::{AracHatasi, AracSonuc};
use crate::raporlayici::{IzGuncelleme, IzKimligi, Raporlayici};
use crate::veri_deposu::{KaynakRef, VeriDeposu};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct AracBaglami {
    /// Bypass kanali. Toplu veri buraya, modele kisa ozet.
    pub veri_deposu: Arc<dyn VeriDeposu>,
    /// Dosya islemlerinin kum havuzu. Disina cikilamaz.
    pub calisma_dizini: PathBuf,
    pub raporlayici: Arc<dyn Raporlayici>,
    /// Bu oturumda kisisel veri araci en az bir kez BASARIYLA calisti mi.
    ///
    /// Tek yon (bkz. `kirlet`) ve tur degisiminde SIFIRLANMAZ: baglam
    /// ozetlendiginde ozetin kendisi kisisel veri tasiyabilir, dolayisiyla
    /// kirlilik de tasinir. Yalniz gercek sohbet sifirlamasi temizler.
    oturum_kirli: bool,
}

impl AracBaglami {
    pub fn yeni(
        veri_deposu: Arc<dyn VeriDeposu>,
        calisma_dizini: impl Into<PathBuf>,
        raporlayici: Arc<dyn Raporlayici>,
    ) -> Self {
        Self {
            veri_deposu,
            calisma_dizini: calisma_dizini.into(),
            raporlayici,
            oturum_kirli: false,
        }
    }

    pub fn oturum_kirli(&self) -> bool {
        self.oturum_kirli
    }

    /// Kisisel veri aracinin BASARILI cagrisindan sonra cagrilir. Geri donusu yok.
    pub fn kirlet(&mut self) {
        self.oturum_kirli = true;
    }

    /// Gercek sohbet sifirlamasi — oturum omurlu ne varsa burada biter.
    pub fn sohbeti_sifirla(&mut self) {
        self.oturum_kirli = false;
        self.veri_deposu.temizle();
    }

    // --- Bypass kanali kisayollari ---

    /// Toplu veriyi depoya koyar; modele bunun yerine referans gider.
    pub fn depola(&self, tur: &str, ozet: &str, govde: String) -> KaynakRef {
        self.veri_deposu.koy(tur, ozet, govde)
    }

    /// Onceki adimda depolanan veriyi geri alir.
    pub fn depodan(&self, kaynak_ref: &KaynakRef) -> Option<crate::veri_deposu::Kayit> {
        self.veri_deposu.al(kaynak_ref)
    }

    // --- Cip kisayollari ---

    pub fn cip_baslat(&self, ikon: &str, metin: &str) -> IzKimligi {
        self.raporlayici.baslat(ikon, metin)
    }

    pub fn cip_guncelle(&self, id: IzKimligi, guncelleme: IzGuncelleme) {
        self.raporlayici.guncelle(id, guncelleme);
    }

    // --- Kum havuzu ---

    /// Verilen yolu calisma dizinine gore cozer ve disari kacmadigini dogrular.
    ///
    /// Dogrulama, dosyanin var olmasini beklemeden bilesen bilesen yapilir:
    /// `canonicalize` var olmayan dosyada basarisiz olur, oysa YAZMA yolunda
    /// hedef henuz yoktur — kapinin en cok gerektigi yer tam da orasi.
    pub fn yolu_coz(&self, yol: impl AsRef<Path>) -> AracSonuc<PathBuf> {
        use std::path::Component;
        let yol = yol.as_ref();
        let mut cozulmus = self.calisma_dizini.clone();
        let bagil = yol.strip_prefix(&self.calisma_dizini).unwrap_or(yol);
        for bilesen in bagil.components() {
            match bilesen {
                Component::Normal(p) => cozulmus.push(p),
                Component::CurDir => {}
                Component::ParentDir => {
                    if !cozulmus.pop() || !cozulmus.starts_with(&self.calisma_dizini) {
                        return Err(AracHatasi::KumHavuzuIhlali(yol.to_path_buf()));
                    }
                }
                // Mutlak yol koku ya da Windows prefix'i: kum havuzunu bastan
                // devre disi birakma girisimi.
                Component::RootDir | Component::Prefix(_) => {
                    return Err(AracHatasi::KumHavuzuIhlali(yol.to_path_buf()));
                }
            }
        }
        if cozulmus.starts_with(&self.calisma_dizini) {
            Ok(cozulmus)
        } else {
            Err(AracHatasi::KumHavuzuIhlali(yol.to_path_buf()))
        }
    }
}
