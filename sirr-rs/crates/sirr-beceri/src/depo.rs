//! `BeceriDeposu` — becerileri tutar ve bir mesaja uyan TEK beceriyi secer.
//!
//! Iki kaynak: paketle gelen varsayilanlar (derlemeye GOMULU) ve kullanicinin
//! `~/.sirr/beceriler/*.md` dosyalari.
//!
//! NEDEN GOMULU: paket becerileri programin davranisinin parcasidir, verisi
//! degil. Diskten okunsalardi ikili tek basina calismaz, eksik dosya SESSIZ bir
//! davranis kaybi olurdu (model kilavuzsuz kalir ve kimse fark etmez).
//! Kullanici becerileri tam tersine veridir ve diskten gelir.

use crate::beceri::{Beceri, ayristir};
use crate::eslesme::{kucult, puan};
use std::path::Path;

/// Derlemeye gomulu varsayilan beceriler. Her satir bir dosya; yeni beceri
/// eklemek bilincli bir karardir, dizine dosya birakmak degil.
const PAKET_DOSYALARI: &[(&str, &str)] = &[
    ("hesap", include_str!("../beceriler/hesap.md")),
    ("belge-oku", include_str!("../beceriler/belge-oku.md")),
    ("belge-olustur", include_str!("../beceriler/belge-olustur.md")),
    ("zaman", include_str!("../beceriler/zaman.md")),
];

#[derive(Debug, Clone, Default)]
pub struct BeceriDeposu {
    paket: Vec<Beceri>,
    kullanici: Vec<Beceri>,
}

impl BeceriDeposu {
    /// Bos depo — testler ve "yalniz kullanici becerileri" senaryosu icin.
    pub fn bos() -> Self {
        Self::default()
    }

    /// Paketle gelen varsayilan set.
    pub fn varsayilan() -> Self {
        Self {
            paket: PAKET_DOSYALARI
                .iter()
                .filter_map(|(ad, ham)| ayristir(ad, ham))
                .collect(),
            kullanici: Vec::new(),
        }
    }

    /// Bir dizindeki `.md` dosyalarini KULLANICI becerisi olarak yukler; kac
    /// tanesinin yuklendigini doner.
    ///
    /// Dizin yoksa ya da bir dosya bozuksa SESSIZCE atlanir ve 0/eksik doner:
    /// kullanicinin yarim yazdigi bir dosya yuzunden asistanin acilmamasi
    /// kabul edilemez bir takas olurdu. Ayristirilamayan dosya "beceri yok"
    /// demektir, "hata" degil.
    pub fn dizinden_yukle(&mut self, dizin: impl AsRef<Path>) -> usize {
        let Ok(girdiler) = std::fs::read_dir(dizin) else {
            return 0;
        };
        let mut sayi = 0;
        for girdi in girdiler.flatten() {
            let yol = girdi.path();
            if yol.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let ad = yol
                .file_stem()
                .and_then(|a| a.to_str())
                .unwrap_or("beceri")
                .to_string();
            let Ok(ham) = std::fs::read_to_string(&yol) else {
                continue;
            };
            let Some(b) = ayristir(&ad, &ham) else {
                continue;
            };
            // Diskten gelen her sey KULLANICI becerisidir: gozden gecirilmemis
            // metin paket becerisinin (700) genis butcesini hak etmez.
            self.kullanici
                .push(Beceri::kullanicinin(b.ad, b.tetikler, b.metin));
            sayi += 1;
        }
        sayi
    }

    /// Kullanici becerilerini toptan degistirir (pano her kayitta cagirir).
    pub fn kullaniciyi_yenile(&mut self, beceriler: Vec<Beceri>) {
        self.kullanici = beceriler
            .into_iter()
            .map(|b| Beceri::kullanicinin(b.ad, b.tetikler, b.metin))
            .collect();
    }

    /// Kullanici ONCE: esit puanda kullanicininki kazanir (bkz. `eslesen`).
    pub fn hepsi(&self) -> impl Iterator<Item = &Beceri> {
        self.kullanici.iter().chain(self.paket.iter())
    }

    pub fn beceri(&self, ad: &str) -> Option<&Beceri> {
        self.hepsi().find(|b| b.ad == ad)
    }

    pub fn sayi(&self) -> usize {
        self.kullanici.len() + self.paket.len()
    }

    /// Verilen mesaja en iyi uyan TEK beceriyi doner (yoksa `None`).
    ///
    /// TEK, cunku amac progressive disclosure: hepsini birden enjekte etmek
    /// 4096 penceresini kilavuzla doldurur ve model arac cagirmak yerine
    /// kilavuz anlatir. Puan, eslesen tetikleyicilerin UZUNLUK toplamidir
    /// (bkz. `eslesme::puan`), adet degil.
    ///
    /// `mevcut_araclar` verilirse kilavuzun EMRETTIGI araci bulundurmayan
    /// beceriler elenir — tek dogruluk kaynagi arac katalogudur, elle tutulan
    /// bir harita degil. `None` gecilirse eleme yapilmaz.
    pub fn eslesen(&self, mesaj: &str, mevcut_araclar: Option<&[String]>) -> Option<&Beceri> {
        let m = kucult(mesaj);
        let mut en_iyi: Option<(&Beceri, usize)> = None;
        for b in self.hepsi() {
            if !b.araclar_var_mi(mevcut_araclar) {
                continue;
            }
            let p = puan(&m, &b.tetikler);
            // KATI buyuktur: esitlikte ONCE gelen kalir, yani `hepsi` sirasi
            // geregi kullanicininki. Beraberligi rastgelelige birakmiyoruz.
            if p > 0 && p > en_iyi.map_or(0, |(_, s)| s) {
                en_iyi = Some((b, p));
            }
        }
        en_iyi.map(|(b, _)| b)
    }

    /// Verilen adlarin becerilerini tek metinde birlestirir (onizleme/denetim
    /// icin; uretim yolu TEK beceri enjekte eder).
    pub fn birlestir(&self, adlar: &[&str]) -> String {
        adlar
            .iter()
            .filter_map(|a| self.beceri(a))
            .map(|b| format!("## {}\n{}", b.ad, b.metin))
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

#[cfg(test)]
mod testler {
    use super::*;
    use crate::enjeksiyon::{ENJEKSIYON_SINIRI, enjeksiyon_metni};

    fn araclar() -> Vec<String> {
        ["hesapla", "zaman", "belge_oku", "belge_olustur"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn paket_becerileri_gomulu_gelir_ve_hepsi_ayristirilir() {
        let d = BeceriDeposu::varsayilan();
        assert_eq!(d.sayi(), PAKET_DOSYALARI.len(), "bir dosya ayristirilamadi");
        assert!(d.beceri("hesap").is_some());
        assert!(d.beceri("belge-olustur").is_some());
    }

    #[test]
    fn ozgul_ifade_genel_kelimeyi_yener() {
        let d = BeceriDeposu::varsayilan();
        // "tablo olarak" (12) belge-oku'da; belge-olustur'da "tablo yap" var
        // ama bu cumlede gecmiyor. Adet sayilsaydi sira rastgele olurdu.
        let b = d.eslesen("bunu tablo olarak goster", Some(&araclar())).unwrap();
        assert_eq!(b.ad, "belge-oku");
    }

    #[test]
    fn arac_kapisi_becerinin_secilmesini_engeller() {
        let d = BeceriDeposu::varsayilan();
        let eksik = vec!["zaman".to_string()];
        assert!(d.eslesen("bunu excel yap", Some(&eksik)).is_none());
        assert!(d.eslesen("bunu excel yap", Some(&araclar())).is_some());
    }

    #[test]
    fn eslesme_yoksa_hicbir_sey_enjekte_edilmez() {
        let d = BeceriDeposu::varsayilan();
        assert!(d.eslesen("merhaba, nasilsin", Some(&araclar())).is_none());
    }

    #[test]
    fn kisa_kok_yanlis_beceriyi_cagirmaz() {
        let d = BeceriDeposu::varsayilan();
        // "dok" tetikleyicisi "dokuz"un icinde durur; tam sozcuk sarti tutmali.
        assert!(d.eslesen("dokuz kere dort", Some(&araclar())).map(|b| b.ad.clone()) != Some("belge-olustur".into()));
    }

    #[test]
    fn kullanici_becerisi_esitlikte_kazanir() {
        let mut d = BeceriDeposu::varsayilan();
        // Ayni tetikleyici: puanlar esit, `hepsi` sirasi kullaniciyi one alir.
        d.kullaniciyi_yenile(vec![Beceri::kullanicinin(
            "benim-hesap",
            vec!["hesapla".into()],
            "Sonucu her zaman tabloda ver.",
        )]);
        let b = d.eslesen("bunu hesapla", Some(&araclar())).unwrap();
        assert_eq!(b.ad, "benim-hesap");
    }

    #[test]
    fn diskten_yuklenen_beceri_kullanici_becerisidir() {
        let dizin = std::env::temp_dir().join("sirr-beceri-testi-1");
        let _ = std::fs::remove_dir_all(&dizin);
        std::fs::create_dir_all(&dizin).unwrap();
        std::fs::write(
            dizin.join("benim.md"),
            format!("---\nad: benim\ntetikler: sivri tetik\n---\n{}", "g".repeat(900)),
        )
        .unwrap();
        // Bozuk dosya: sessizce atlanmali, yukleme cokmemeli.
        std::fs::write(dizin.join("bozuk.md"), "tetiksiz govde").unwrap();
        std::fs::write(dizin.join("okunmaz.txt"), "---\ntetikler: a\n---\nx").unwrap();

        let mut d = BeceriDeposu::bos();
        assert_eq!(d.dizinden_yukle(&dizin), 1);
        let b = d.beceri("benim").unwrap();
        assert!(b.kullanicinin_mi);
        assert_eq!(b.metin.chars().count(), crate::beceri::KULLANICI_GOVDE_SINIRI);
        let _ = std::fs::remove_dir_all(&dizin);
    }

    #[test]
    fn olmayan_dizin_hata_degil_sifir_doner() {
        let mut d = BeceriDeposu::bos();
        assert_eq!(d.dizinden_yukle("/kesinlikle/olmayan/dizin"), 0);
        assert_eq!(d.sayi(), 0);
    }

    #[test]
    fn her_paket_becerisinin_enjeksiyonu_butceyi_asmaz() {
        let d = BeceriDeposu::varsayilan();
        for b in d.hepsi() {
            let govde = crate::enjeksiyon::enjeksiyon_govdesi(&b.metin, ENJEKSIYON_SINIRI);
            assert!(
                govde.chars().count() <= ENJEKSIYON_SINIRI,
                "{} tasti: {}",
                b.ad,
                govde.chars().count()
            );
            // Cekirdek isareti modele SIZMAMALI; o bir dosya isareti.
            assert!(!enjeksiyon_metni(b).contains(crate::enjeksiyon::CEKIRDEK_ISARETI));
        }
    }

    #[test]
    fn birlestir_yalniz_var_olan_adlari_alir() {
        let d = BeceriDeposu::varsayilan();
        let m = d.birlestir(&["hesap", "yok-boyle-bir-sey"]);
        assert!(m.starts_with("## hesap"));
        assert!(!m.contains("yok-boyle"));
    }
}
