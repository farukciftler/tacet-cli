//! Cip durumu: kullaniciya akista gorunen tek dogruluk kaynagi.

use serde::{Deserialize, Serialize};

/// Bir arac cipinin yasam dongusundeki durumu.
///
/// `Okundu`/`Yazildi` ayrimi kozmetik degil: motor "bu turda dunya degisti mi"
/// sorusunu YALNIZ `Yazildi`ya bakarak yanitlar ve hata sonrasi yeniden deneme
/// guvenligini buna dayandirir. Yanlis durum secmek cift yan etki demektir.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AracDurumu {
    /// Is suruyor; cipte spinner var.
    Calisiyor,
    /// Salt-okuma tamamlandi — geri alinacak bir sey yok.
    Okundu,
    /// Dunya degisti (dosya yazildi, kayit olusturuldu). Yeniden deneme yasak.
    Yazildi,
    /// Kullanici karari bekleniyor — kapi kodda, modelde degil.
    IzinGerekli,
    /// Basarisiz; tasidigi metin KULLANICIYA gosterilir, o yuzden Turkce ve
    /// insan cumlesidir. Modele giden metin bu degil, `HATA_MODEL_METNI`dir.
    Basarisiz(String),
}

impl AracDurumu {
    /// Bu durum "dunyayi degistirdi" mi — motorun yeniden deneme kapisi.
    pub fn dunyayi_degistirdi(&self) -> bool {
        matches!(self, AracDurumu::Yazildi)
    }

    /// Is bitti mi (cip artik canli degil).
    pub fn bitti_mi(&self) -> bool {
        !matches!(self, AracDurumu::Calisiyor | AracDurumu::IzinGerekli)
    }
}
