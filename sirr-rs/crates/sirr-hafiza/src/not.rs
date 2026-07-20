//! `HafizaNotu` — hafiza katmaninin tek kalici kaydi ve sinir sabitleri.
//!
//! Sinirlar SERT cunku eslesen notlar 4096 token penceresine enjekte edilir;
//! her karakter butcedir. Tavana gelindiginde OTOMATIK DUSURME YOKTUR: sessizce
//! silmek, "sessizce ogrenme yok" ilkesinin simetrigidir. Ne ogrenildigi de ne
//! unutuldugu da kullanicinin karari.

use serde::{Deserialize, Serialize};
use sirr_beceri::kucult;

/// Not metninin ust siniri (~40 token).
pub const METIN_SINIRI: usize = 160;

/// Alt sinir. Bundan kisa bir "olgu" ya gurultudur ya da modelin yarim
/// kopyaladigi bir parcadir; ikisi de hatirlamaya deger degil.
pub const EN_AZ_METIN: usize = 10;

/// Saklanabilecek en fazla not. Dolunca yeni kayit ALINMAZ.
pub const TOPLAM_TAVAN: usize = 50;

/// Not basina en fazla anahtar. Eslesme taramasi HER mesajda donduğu icin
/// sinirli: 50 not x sinirsiz anahtar, her turda odenen bir vergi olurdu.
pub const ANAHTAR_SINIRI: usize = 8;

/// Notun turu. v1'de yalniz panoda etiket; enjeksiyon onceliginde rol ALMAZ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HafizaTuru {
    Kimlik,
    Tercih,
    Iliski,
    #[default]
    Olgu,
}

impl HafizaTuru {
    /// Metinden tur cozer. Bilinmeyen deger `None` doner ve not DUSER —
    /// varsayilana dusurulmez: model turu uyduruyorsa notun kendisi de supheli.
    /// ASCII kucultme kullanilir, Turkce degil: bu degerler modele gosterilen
    /// SEMA ETIKETLERIDIR ("kimlik", "tercih"), Turkce sozcuk degil. Turkce
    /// kucultme "TERCIH"i "tercıh" yapardi ve model buyuk harfle yazdiginda
    /// gecerli bir tur reddedilirdi.
    pub fn coz(ham: &str) -> Option<Self> {
        match ham.trim().to_ascii_lowercase().as_str() {
            "kimlik" => Some(Self::Kimlik),
            "tercih" => Some(Self::Tercih),
            "iliski" => Some(Self::Iliski),
            "olgu" => Some(Self::Olgu),
            _ => None,
        }
    }

    pub fn ad(&self) -> &'static str {
        match self {
            Self::Kimlik => "kimlik",
            Self::Tercih => "tercih",
            Self::Iliski => "iliski",
            Self::Olgu => "olgu",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HafizaNotu {
    pub id: u64,
    /// Tek cumlelik olgu; kullanicinin kendi ifadesinden.
    pub metin: String,
    pub tur: HafizaTuru,
    /// Kucultulmus tetikleyiciler. Hatirlama bunlarla calisir.
    pub anahtarlar: Vec<String>,
    /// Unix saniyesi. Kalici bicim insan tarihine bagli olmasin diye ham sayi.
    pub olusturulma: u64,
    /// Kullanici kapatabilir; kapali not ENJEKTE EDILMEZ ama silinmez de.
    pub aktif: bool,
}

impl HafizaNotu {
    /// Tekillestirme anahtari.
    pub fn normal_metin(&self) -> String {
        karsilastirma_anahtari(&self.metin)
    }

    /// Kaydedilebilir mi — metin sinir icinde ve en az bir anahtar var.
    pub fn gecerli_mi(&self) -> bool {
        let n = self.metin.trim().chars().count();
        (EN_AZ_METIN..=METIN_SINIRI).contains(&n) && !self.anahtarlar.is_empty()
    }

    /// Panoda gosterilen altyazi: "yemek · restoran · aksam".
    pub fn ozet(&self) -> String {
        self.anahtarlar.join(" · ")
    }
}

/// TEKILLESTIRME anahtari — eslestirmeden AYRI ve bilerek daha BAGISLAYICI.
///
/// Turkce kucultme dogru olani yapar ve `I`yi `ı`ya cevirir; bu, ESLESTIRMEDE
/// dogru davranistir. Ama TEKILLESTIRMEDE ayni dogruluk bir delik acar:
/// "KULLANICI VEJETARYENDIR." -> "kullanıcı vejetaryendır." olur ve
/// "Kullanici vejetaryendir." ile esitlenmez; ayni olgu iki kez kaydedilir.
/// Model buyuk harf kullanip kullanmamayi tesadufen secer, dolayisiyla burada
/// noktali/noktasiz i AYNI sayilir. Yon farki bilincli: yanlis eslesmek pahali
/// (yanlis olgu enjekte edilir), fazladan tekillestirmek ucuz (kullanici zaten
/// ayni seyi yazmis).
pub fn karsilastirma_anahtari(metin: &str) -> String {
    kucult(metin.trim())
        .chars()
        .map(|k| if k == 'ı' { 'i' } else { k })
        .collect()
}

/// Anahtar listesini normalize eder: kucult, kirp, bosalari at, virgul iceren
/// parcayi ayir, sinirla.
///
/// Virgul BILEREK ayrilir: model "yemek, restoran" diye TEK anahtar
/// gonderdiginde bu dizgi hicbir mesajda gecmez ve not olu dogar.
pub fn anahtarlari_duzelt<S: AsRef<str>>(ham: &[S]) -> Vec<String> {
    let mut cikti: Vec<String> = Vec::new();
    for parca in ham {
        for tek in parca.as_ref().split(',') {
            let t = kucult(tek.trim());
            if t.is_empty() || cikti.contains(&t) {
                continue;
            }
            cikti.push(t);
            if cikti.len() == ANAHTAR_SINIRI {
                return cikti;
            }
        }
    }
    cikti
}

#[cfg(test)]
mod testler {
    use super::*;

    fn not(metin: &str, anahtarlar: &[&str]) -> HafizaNotu {
        HafizaNotu {
            id: 1,
            metin: metin.into(),
            tur: HafizaTuru::Olgu,
            anahtarlar: anahtarlari_duzelt(anahtarlar),
            olusturulma: 0,
            aktif: true,
        }
    }

    #[test]
    fn gecerlilik_sinirlari_uygulanir() {
        assert!(not("Kullanici vejetaryendir.", &["yemek"]).gecerli_mi());
        assert!(!not("kisa", &["yemek"]).gecerli_mi(), "10 karakterden kisa");
        assert!(!not(&"a".repeat(METIN_SINIRI + 1), &["yemek"]).gecerli_mi());
        assert!(!not("Kullanici vejetaryendir.", &[]).gecerli_mi(), "anahtarsiz");
    }

    #[test]
    fn tur_cozumu_uydurma_degeri_reddeder() {
        assert_eq!(HafizaTuru::coz(" Kimlik "), Some(HafizaTuru::Kimlik));
        assert_eq!(HafizaTuru::coz("TERCIH"), Some(HafizaTuru::Tercih));
        assert_eq!(HafizaTuru::coz("meslek"), None, "varsayilana DUSMEMELI");
        assert_eq!(HafizaTuru::default(), HafizaTuru::Olgu);
    }

    #[test]
    fn anahtarlar_virgulle_ayrilir_ve_sinirlanir() {
        assert_eq!(anahtarlari_duzelt(&["Yemek, Restoran"]), vec!["yemek", "restoran"]);
        assert_eq!(anahtarlari_duzelt(&["a", "A", " a "]), vec!["a"], "tekrar duser");
        let cok: Vec<String> = (0..20).map(|i| format!("k{i}")).collect();
        assert_eq!(anahtarlari_duzelt(&cok).len(), ANAHTAR_SINIRI);
    }

    #[test]
    fn tekillestirme_buyuk_kucuk_i_ayrimini_yutar() {
        // Tekillestirme noktali/noktasiz i ayrimini yutar (bkz. karsilastirma_anahtari).
        assert_eq!(not("İSTANBUL'DA YASIYORUM", &["sehir"]).normal_metin(), not("İstanbul'da yasiyorum", &["sehir"]).normal_metin());
    }

    #[test]
    fn ozet_anahtarlari_okunur_yazar() {
        assert_eq!(not("Kullanici vejetaryendir.", &["yemek", "restoran"]).ozet(), "yemek · restoran");
    }
}
