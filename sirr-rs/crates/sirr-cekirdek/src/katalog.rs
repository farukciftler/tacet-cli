//! AracKatalogu — calisma anindaki arac kumesi.
//!
//! Tek dogruluk kaynagi: hem istem metni, hem gramer, hem cagri yonlendirmesi
//! bu listeden turer. Ikinci bir "arac adlari" listesi tutulmaz; tutulsaydi er
//! gec ayrisir ve model var olmayan bir araci cagirirdi.

use crate::arac::Arac;
use std::sync::Arc;

#[derive(Default, Clone)]
pub struct AracKatalogu {
    araclar: Vec<Arc<dyn Arac>>,
}

impl AracKatalogu {
    pub fn yeni() -> Self {
        Self::default()
    }

    pub fn ekle(&mut self, arac: Arc<dyn Arac>) -> &mut Self {
        self.araclar.push(arac);
        self
    }

    pub fn araclar(&self) -> &[Arc<dyn Arac>] {
        &self.araclar
    }

    /// Ad ile arar. Model uydurma bir ad gonderirse `None` doner ve cagri
    /// yerinde sabit hata metnine cevrilir.
    pub fn bul(&self, ad: &str) -> Option<Arc<dyn Arac>> {
        self.araclar.iter().find(|a| a.ad() == ad).cloned()
    }

    pub fn adlar(&self) -> Vec<&str> {
        self.araclar.iter().map(|a| a.ad()).collect()
    }

    /// Kisisel veri okuyan araclar — onay politikasini kuran taraf bunu okur.
    pub fn kirletenler(&self) -> Vec<&str> {
        self.araclar
            .iter()
            .filter(|a| a.kirletir_mi())
            .map(|a| a.ad())
            .collect()
    }
}

impl FromIterator<Arc<dyn Arac>> for AracKatalogu {
    fn from_iter<T: IntoIterator<Item = Arc<dyn Arac>>>(iter: T) -> Self {
        Self { araclar: iter.into_iter().collect() }
    }
}
