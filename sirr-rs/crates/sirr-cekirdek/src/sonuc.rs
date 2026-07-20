//! Bir arac calismasinin sonucu: cipe ne yazilacagi + modele ne donecegi.

use crate::durum::AracDurumu;
use crate::hata::AracHatasi;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Aracin tek cikti tipi.
///
/// `cip_metni` ile `modele_donen` AYRI alanlardir ve ayri olmak zorundadir:
/// cip kullanicinin gordugu seydir (Turkce, kisa, olan biteni anlatir),
/// `modele_donen` ise baglama giren seydir. 4096 token kanalinin kalbi burada:
/// toplu veri `modele_donen`e KOYULMAZ, VeriDeposu'na yazilip `kaynak_ref`
/// ile isaret edilir.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AracSonucu {
    /// Cipte gosterilecek son metin (~5 kelime, gerekirse " · detay").
    pub cip_metni: String,
    pub durum: AracDurumu,
    /// Modele donen KISA metin. Toplu veri buraya yazilmaz.
    pub modele_donen: String,
    /// Cip detay goruntusu icin ham cikti — seffafligin ikinci katmani.
    /// Modele gitmez; yalnizca kullanici acarsa gorur.
    pub ham_cikti: Option<String>,
    /// Bir dosya uretildiyse yolu — cipe dokununca onizleme acilir.
    pub dosya_yolu: Option<PathBuf>,
}

/// Bypass kanalinin TEK tel bicimi.
///
/// NEDEN TEK FONKSIYON: bu ek modele giden metne yazilir, yani modelin
/// OGRENDIGI sozdizimidir. Iki ayri cagri yeri kendi `format!`ini yazdiginda
/// (bir yerde "tam icerik hazir", baska yerde "[kaynak_ref: ...]") model iki
/// farkli bicim gorur ve hangisini geri vereceğini sasirir; ustelik biri
/// Turkcelesti ve kimse fark etmedi. Bicim burada bir kez tanimlanir.
///
/// METIN INGILIZCE: modele giden her sabit metin gibi. Kullaniciya gorunen
/// cip Turkcedir; ikisi ayri alanlardir ve karistirilmamalidir.
pub fn kaynak_ref_eki(kaynak_ref: &str) -> String {
    format!("\n(full content ready, kaynak_ref={kaynak_ref})")
}

impl AracSonucu {
    /// Tam kontrol gereken yerler icin temel yapici; gunluk kullanimda
    /// asagidaki kisa yapicilar tercih edilir.
    pub fn yeni(
        cip_metni: impl Into<String>,
        durum: AracDurumu,
        modele_donen: impl Into<String>,
    ) -> Self {
        Self {
            cip_metni: cip_metni.into(),
            durum,
            modele_donen: modele_donen.into(),
            ham_cikti: None,
            dosya_yolu: None,
        }
    }

    /// Salt-okuma bitti.
    pub fn okundu(cip_metni: impl Into<String>, modele_donen: impl Into<String>) -> Self {
        Self::yeni(cip_metni, AracDurumu::Okundu, modele_donen)
    }

    /// Dunya degisti — motor bunu gorunce yeniden denemeyi kapatir.
    pub fn yazildi(cip_metni: impl Into<String>, modele_donen: impl Into<String>) -> Self {
        Self::yeni(cip_metni, AracDurumu::Yazildi, modele_donen)
    }

    // NOT: bir zamanlar burada `izin_gerekli(...)` yapicisi vardi. Sifir
    // cagirani oldugu icin (test dahil) kaldirildi: onay karari aracin degil
    // `AracYurutucu`nun isidir ve `AracDurumu::IzinGerekli`yi oradaki kapi
    // uretir. Ikinci bir uretim yeri birakmak, kapiyi atlayan bir arac yazmayi
    // mesrulastiran davetti.

    /// Hatadan sonuc uretir. Cipe Turkce cumle, modele sabit Ingilizce metin —
    /// tek gecis noktasi burasi oldugu icin cagri yerleri bu kurali bozamaz.
    pub fn basarisiz(hata: &AracHatasi) -> Self {
        let neden = hata.kisa_hata();
        Self {
            cip_metni: neden.clone(),
            durum: AracDurumu::Basarisiz(neden.clone()),
            modele_donen: hata.model_metni().to_string(),
            ham_cikti: Some(neden),
            dosya_yolu: None,
        }
    }

    /// Bypass kanali kisayolu: buyuk veri depoya konmus, modele yalniz ozet
    /// ve referans gider.
    pub fn ozetle(
        cip_metni: impl Into<String>,
        ozet: impl AsRef<str>,
        kaynak_ref: impl AsRef<str>,
    ) -> Self {
        Self::okundu(
            cip_metni,
            format!("{}{}", ozet.as_ref(), kaynak_ref_eki(kaynak_ref.as_ref())),
        )
    }

    pub fn ham_cikti(mut self, ham: impl Into<String>) -> Self {
        self.ham_cikti = Some(ham.into());
        self
    }

    pub fn dosya_yolu(mut self, yol: impl Into<PathBuf>) -> Self {
        self.dosya_yolu = Some(yol.into());
        self
    }
}
