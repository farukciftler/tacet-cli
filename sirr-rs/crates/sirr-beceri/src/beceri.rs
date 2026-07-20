//! Tek bir beceri ve `.md` bicimi.
//!
//! Bicim Swift tarafindan devralindi (`ketum/Beceriler/*.md`): frontmatter
//! (`ad`, `tetikler`, `araclar`) + markdown govde. Dosyalar "cekirdek-once"
//! yazilir: somut `arac(args)` ornegi ve kirilmaz kurallar `<!--/cekirdek-->`
//! isaretinin USTUNDE, insan referansi altinda durur.

use crate::eslesme::kucult;

/// Kullanicinin kendi yazdigi becerinin govde siniri.
///
/// Paket becerisinden (700) DAHA DAR: paket dosyalari olculdu, gozden gecirildi
/// ve cekirdek-once yazildi; kullanicinin serbest metni ise denetlenmemis bir
/// girdidir ve 4096 penceresinin en pahali yerine (sorunun hemen onune) girer.
/// 500, tipik bir "su tarzda cevap ver" talimatini rahat alir ama uzun bir
/// makaleyi istemden uzak tutar.
pub const KULLANICI_GOVDE_SINIRI: usize = 500;

/// Bir beceri: ad, tetikleyiciler ve kilavuz metni.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Beceri {
    pub ad: String,
    /// Kucultulmus tetikleyiciler; eslestirme bunlarla calisir.
    pub tetikler: Vec<String>,
    pub metin: String,
    /// Bu kilavuzun EMRETTIGI araclarin adlari (frontmatter `araclar:`).
    /// Bossa beceri aractan bagimsizdir ve her katalogda serbesttir.
    pub araclar: Vec<String>,
    /// Kullanicinin kendi yazdigi mi — esitlikte kullanicininki kazanir.
    pub kullanicinin_mi: bool,
}

impl Beceri {
    /// Paket becerisi (gozden gecirilmis, cekirdek-once yazilmis).
    pub fn paket(
        ad: impl Into<String>,
        tetikler: Vec<String>,
        metin: impl Into<String>,
        araclar: Vec<String>,
    ) -> Self {
        Self {
            ad: ad.into(),
            tetikler: tetikler.iter().map(|t| kucult(t)).collect(),
            metin: metin.into(),
            araclar,
            kullanicinin_mi: false,
        }
    }

    /// Kullanici becerisi. Govde BURADA kirpilir, enjeksiyonda degil: kirpma
    /// tek yerde olursa "kaydedilen ile modele giden ayni sey" garantisi
    /// kalir; iki yerde olsaydi pano 900 karakter gosterip model 500 gorurdu.
    pub fn kullanicinin(
        ad: impl Into<String>,
        tetikler: Vec<String>,
        metin: impl Into<String>,
    ) -> Self {
        let ham: String = metin.into();
        let kirpik: String = ham.chars().take(KULLANICI_GOVDE_SINIRI).collect();
        Self {
            ad: ad.into(),
            tetikler: tetikler.iter().map(|t| kucult(t)).collect(),
            metin: kirpik,
            araclar: Vec::new(),
            kullanicinin_mi: true,
        }
    }

    /// Becerinin bildirdigi TUM araclar katalogda var mi.
    ///
    /// Kapi "hepsi" uzerinden cunku kilavuz iki adimli bir akis anlatabiliyor
    /// (once `belge_oku`, sonra `belge_olustur`); yarisi eksikse kilavuz zaten
    /// uygulanamaz ve enjekte edilmesi modele olmayan araci cagirtir.
    pub fn araclar_var_mi(&self, mevcut: Option<&[String]>) -> bool {
        let Some(mevcut) = mevcut else {
            return true; // nil gecilirse eleme yok (test/onizleme yolu)
        };
        if self.araclar.is_empty() {
            return true;
        }
        self.araclar.iter().all(|a| mevcut.iter().any(|m| m == a))
    }
}

/// Frontmatter + govdeyi ayristirir. Tetikleyicisi olmayan dosya beceri
/// SAYILMAZ (`None`): tetiksiz beceri hicbir zaman secilemez, ama katalogda
/// durup "neden calismiyor" sorusu urettirir.
pub fn ayristir(varsayilan_ad: &str, ham: &str) -> Option<Beceri> {
    let satirlar: Vec<&str> = ham.split('\n').map(|s| s.trim_end_matches('\r')).collect();
    let mut ad = varsayilan_ad.to_string();
    let mut tetikler: Vec<String> = Vec::new();
    let mut araclar: Vec<String> = Vec::new();
    let mut govde = ham.trim().to_string();

    if satirlar.first().map(|s| s.trim()) == Some("---")
        && let Some(kapanis) = satirlar
            .iter()
            .skip(1)
            .position(|s| s.trim() == "---")
            .map(|i| i + 1)
    {
        for satir in &satirlar[1..kapanis] {
            let Some((anahtar, deger)) = satir.split_once(':') else {
                continue;
            };
            let deger = deger.trim();
            match anahtar.trim() {
                "ad" => ad = deger.to_string(),
                "tetikler" => tetikler = virgullu(deger, true),
                "araclar" => araclar = virgullu(deger, false),
                _ => {}
            }
        }
        govde = satirlar[(kapanis + 1)..].join("\n").trim().to_string();
    }

    if tetikler.is_empty() {
        return None;
    }
    Some(Beceri::paket(ad, tetikler, govde, araclar))
}

/// "a, b, c" -> ["a","b","c"]; bos parcalar atilir.
fn virgullu(deger: &str, kucult_mu: bool) -> Vec<String> {
    deger
        .split(',')
        .map(|p| {
            let t = p.trim();
            if kucult_mu { kucult(t) } else { t.to_string() }
        })
        .filter(|p| !p.is_empty())
        .collect()
}

#[cfg(test)]
mod testler {
    use super::*;

    #[test]
    fn frontmatter_ad_tetik_ve_arac_okur() {
        let b = ayristir(
            "dosya-adi",
            "---\nad: hesap\ntetikler: Hesapla, kac eder\naraclar: hesapla\n---\n# Arithmetic\nGovde.",
        )
        .expect("gecerli beceri");
        assert_eq!(b.ad, "hesap");
        assert_eq!(b.tetikler, vec!["hesapla", "kac eder"]);
        assert_eq!(b.araclar, vec!["hesapla"]);
        assert_eq!(b.metin, "# Arithmetic\nGovde.");
        assert!(!b.kullanicinin_mi);
    }

    #[test]
    fn tetiksiz_dosya_beceri_sayilmaz() {
        assert!(ayristir("x", "---\nad: bos\n---\nGovde").is_none());
        assert!(ayristir("x", "frontmatter yok, duz metin").is_none());
    }

    #[test]
    fn kullanici_govdesi_500_karakterde_kirpilir() {
        let uzun = "a".repeat(900);
        let b = Beceri::kullanicinin("benim", vec!["tetik".into()], uzun);
        assert_eq!(b.metin.chars().count(), KULLANICI_GOVDE_SINIRI);
        assert!(b.kullanicinin_mi);
    }

    #[test]
    fn arac_kapisi_eksik_araci_eler() {
        let b = ayristir(
            "x",
            "---\nad: iki-adim\ntetikler: duzenle\naraclar: belge_oku, belge_duzenle\n---\nGovde",
        )
        .unwrap();
        let tam = vec!["belge_oku".to_string(), "belge_duzenle".to_string()];
        let yarim = vec!["belge_oku".to_string()];
        assert!(b.araclar_var_mi(Some(&tam)));
        assert!(!b.araclar_var_mi(Some(&yarim)), "yarim akis enjekte edilmemeli");
        assert!(b.araclar_var_mi(None), "None = eleme yok");
    }

    #[test]
    fn arac_bildirmeyen_beceri_her_katalogda_serbesttir() {
        let b = Beceri::kullanicinin("benim", vec!["tetik".into()], "govde");
        assert!(b.araclar_var_mi(Some(&[])));
    }
}
