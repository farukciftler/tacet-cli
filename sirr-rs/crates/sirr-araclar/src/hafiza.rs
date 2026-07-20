//! `HafizaAraci` — modelin "bunu hatirla" / "sunu unut" diyebilmesi.
//!
//! NEDEN BIR ARAC: hatirlama okuma yolu MODELSIZDIR (kod `eslesen` ile karar
//! verir), ama YAZMA yolunda kullanicinin niyeti gerekir. "Ben vejetaryenim"
//! cumlesini kendiliginden kaydetmek "sessizce ogrenme"dir; kullanicinin
//! "bunu unutma" demesi ise acik bir taleptir ve bir arac cagrisina denk duser.
//! Arac olmasinin ikinci faydasi seffaflik: her kayit ekranda bir CIP birakir,
//! yani hafizaya ne girdigi kullanicidan gizlenemez.
//!
//! KIRLETIR: notlar dogrudan kisisel veridir. Bir kez okundugunda oturum kirli
//! sayilir ve disari veri cikaracak her cagri onay kapisina takilir.

use sirr_cekirdek::{
    Alan, Arac, AracBaglami, AracDurumu, AracGelecegi, AracHatasi, AracSonucu, ArgSema,
    IzGuncelleme, kutula,
};
use sirr_hafiza::{HafizaDeposu, HafizaHatasi, HafizaTuru};
use std::sync::{Arc, Mutex};

/// Cipte gosterilecek not metninin siniri; cip tek satirdir.
const CIP_METIN_SINIRI: usize = 40;

/// Hafiza deposunu araclar ile kabuk arasinda paylastiran sarmalayici.
///
/// `PaylasimliDepo` ile ayni kalip: sahiplik tek yerde, erisim `Mutex` ile.
/// Kuresel durum degil — kabuk kimin hangi depoyu gordugune karar verir, boylece
/// eval gercek ev dizinine dokunmadan bellek deposuyla kosabilir.
#[derive(Clone)]
pub struct PaylasimliHafiza(Arc<Mutex<HafizaDeposu>>);

impl PaylasimliHafiza {
    pub fn yeni(depo: HafizaDeposu) -> Self {
        Self(Arc::new(Mutex::new(depo)))
    }

    /// Diske hic dokunmayan depo (test/eval).
    pub fn bellekte() -> Self {
        Self::yeni(HafizaDeposu::bellekte())
    }

    /// Kilidi alip `is`i calistirir. Kilit ZEHIRLENMISSE (baska bir is
    /// parcacigi panikledi) `None` doner: `unwrap` bir arac icinde tum turu
    /// goturur, oysa hafizasiz devam etmek mesru bir bozulma.
    pub fn ile<T>(&self, is: impl FnOnce(&mut HafizaDeposu) -> T) -> Option<T> {
        self.0.lock().ok().map(|mut d| is(&mut d))
    }
}

pub struct HafizaAraci {
    depo: PaylasimliHafiza,
}

impl HafizaAraci {
    pub fn yeni(depo: PaylasimliHafiza) -> Self {
        Self { depo }
    }
}

impl Arac for HafizaAraci {
    fn ad(&self) -> &str {
        "hafiza"
    }

    fn aciklama(&self) -> &str {
        "Kullanicinin kendisi hakkinda soyledigi kalici bir olguyu kaydeder, siler \
         ya da listeler. Yalnizca kullanici acikca 'bunu hatirla' / 'sunu unut' \
         dediginde cagir; siradan sohbetten kendi basina not cikarma."
    }

    fn sema(&self) -> ArgSema {
        // SEMA = MODELIN SINIRI: gramer bunu birebir kisita cevirir.
        ArgSema::nesne(vec![
            Alan::yeni(
                "eylem",
                ArgSema::secenek(["hatirla", "unut", "listele"])
                    .aciklama("hatirla: yeni olgu kaydet | unut: sil | listele: kayitlari say"),
            )
            .zorunlu(),
            Alan::yeni(
                "metin",
                ArgSema::metin().aciklama(
                    "hatirla: tek cumlelik olgu (ornek: Kullanici vejetaryendir.) | \
                     unut: silinecek olguyu tarif eden ifade.",
                ),
            ),
            Alan::yeni(
                "anahtarlar",
                ArgSema::metin().aciklama(
                    "hatirla icin: virgulle ayrilmis 2-4 anahtar kelime. Ornek: yemek, restoran",
                ),
            ),
            Alan::yeni(
                "tur",
                ArgSema::secenek(["kimlik", "tercih", "iliski", "olgu"])
                    .aciklama("Olgunun turu; emin degilsen olgu."),
            ),
        ])
    }

    /// Kisisel veri tasir — oturumu kirletir.
    fn kirletir_mi(&self) -> bool {
        true
    }

    fn calistir<'a>(
        &'a self,
        args: serde_json::Value,
        ctx: &'a mut AracBaglami,
    ) -> AracGelecegi<'a> {
        kutula(async move {
            // Gramer devre disi birakilabilir; kapi burada da duruyor.
            if let Err(hata) = self.sema().dogrula(&args) {
                return AracSonucu::basarisiz(&hata);
            }
            let eylem = args.get("eylem").and_then(|d| d.as_str()).unwrap_or("");
            let metin = args.get("metin").and_then(|d| d.as_str()).unwrap_or("").trim();

            let iz = ctx.cip_baslat("~", "Hafiza");

            let sonuc = match eylem {
                "hatirla" => self.hatirla(&args, metin),
                "unut" => self.unut(metin),
                "listele" => self.listele(ctx),
                // Sema kapisi buraya dusmeyi zaten engelliyor; yine de
                // `unreachable!` yazilmadi — bir arac panikle turu goturmemeli.
                _ => AracSonucu::basarisiz(&AracHatasi::GecersizArguman("bilinmeyen eylem".into())),
            };

            ctx.cip_guncelle(
                iz,
                IzGuncelleme::durum(sonuc.durum.clone()).metin(sonuc.cip_metni.clone()),
            );
            // KIRLETME yalniz BASARILI cagrida: basarisiz bir cagri hicbir
            // kisisel veriyi baglama sokmadi, oturumu kirletmesi de yanlis olur.
            if !matches!(sonuc.durum, AracDurumu::Basarisiz(_)) {
                ctx.kirlet();
            }
            sonuc
        })
    }
}

impl HafizaAraci {
    fn hatirla(&self, args: &serde_json::Value, metin: &str) -> AracSonucu {
        let tur = args
            .get("tur")
            .and_then(|d| d.as_str())
            .and_then(HafizaTuru::coz)
            .unwrap_or_default();
        let anahtarlar: Vec<String> = args
            .get("anahtarlar")
            .and_then(|d| d.as_str())
            .map(|s| s.split(',').map(|p| p.trim().to_string()).collect())
            .unwrap_or_default();

        // Donen tip `Result<(), String>`: hem filtre reddi hem disk arizasi
        // kullaniciya AYNI kanaldan (Turkce cumle) cikar, ama sebepleri ayri
        // yazilir — "bunu zaten biliyorum" ile "yazilamadi" ayni sey degil.
        let Some(sonuc) = self.depo.ile(|d| -> Result<(), String> {
            let id = d.ekle(metin, tur, &anahtarlar).map_err(|h: HafizaHatasi| {
                h.kisa_hata().to_string()
            })?;
            // Disk hatasi notu GERI ALIR: bellekte durup diske yazilamayan bir
            // not sonraki acilista sessizce yok olurdu — "hatirladim" demis
            // olurduk ve yalan cikardi. Ham io metni kullaniciya GOSTERILMEZ.
            if d.kaydet().is_err() {
                d.sil(id);
                return Err("Not kaydedilemedi.".to_string());
            }
            Ok(())
        }) else {
            return AracSonucu::basarisiz(&AracHatasi::Diger("Hafizaya su an erisilemiyor.".into()));
        };

        match sonuc {
            Ok(()) => AracSonucu::yazildi(format!("Not alindi · {}", kirp(metin)), "note saved"),
            Err(neden) => AracSonucu::basarisiz(&AracHatasi::Diger(neden)),
        }
    }

    fn unut(&self, ifade: &str) -> AracSonucu {
        if ifade.is_empty() {
            // Bos ifade TUM notlarla eslesirdi; bir silme aracinda bu kabul
            // edilemez, o yuzden kapi burada.
            return AracSonucu::basarisiz(&AracHatasi::EksikAlan("arg.metin".into()));
        }
        let Some(silinen) = self.depo.ile(|d| {
            let n = d.unut(ifade);
            if n > 0 {
                let _ = d.kaydet();
            }
            n
        }) else {
            return AracSonucu::basarisiz(&AracHatasi::Diger("Hafizaya su an erisilemiyor.".into()));
        };

        if silinen == 0 {
            return AracSonucu::okundu("Unutulacak bir not bulunamadi.", "no matching note");
        }
        AracSonucu::yazildi(format!("{silinen} not unutuldu"), "note removed")
    }

    /// Notlari listeler — BYPASS KANALIYLA.
    ///
    /// Elli notun tamami modele dokulmez: govde `VeriDeposu`ya konur, modele
    /// adet + `kaynak_ref` doner. Liste kucuk gorunse de kisisel veridir ve
    /// kanalin var olma sebebi tam olarak budur.
    fn listele(&self, ctx: &AracBaglami) -> AracSonucu {
        let Some((adet, govde)) = self.depo.ile(|d| {
            let govde = d
                .notlar()
                .iter()
                .map(|n| {
                    format!(
                        "- [{}] {} ({}){}",
                        n.tur.ad(),
                        n.metin,
                        n.ozet(),
                        if n.aktif { "" } else { " [kapali]" }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            (d.sayi(), govde)
        }) else {
            return AracSonucu::basarisiz(&AracHatasi::Diger("Hafizaya su an erisilemiyor.".into()));
        };

        if adet == 0 {
            return AracSonucu::okundu("Hafizada not yok.", "memory is empty");
        }
        let r = ctx.depola("hafiza", &format!("{adet} not"), govde);
        AracSonucu::ozetle(
            format!("{adet} not"),
            format!("{adet} notes stored"),
            r.as_str(),
        )
    }
}

/// Cip tek satir kalsin diye not metnini kisaltir.
fn kirp(metin: &str) -> String {
    if metin.chars().count() <= CIP_METIN_SINIRI {
        return metin.to_string();
    }
    let bas: String = metin.chars().take(CIP_METIN_SINIRI - 1).collect();
    format!("{bas}…")
}

#[cfg(test)]
mod testler {
    use super::*;
    use crate::veri_deposu::PaylasimliDepo;
    use serde_json::json;
    use sirr_cekirdek::{IzToplayici, Raporlayici, VeriDeposu};

    fn baglam(depo: Arc<PaylasimliDepo>, rapor: Arc<dyn Raporlayici>) -> AracBaglami {
        AracBaglami::yeni(depo, "/tmp/sirr-hafiza-arac", rapor)
    }

    fn kur() -> (HafizaAraci, PaylasimliHafiza, Arc<PaylasimliDepo>, AracBaglami) {
        let hafiza = PaylasimliHafiza::bellekte();
        let arac = HafizaAraci::yeni(hafiza.clone());
        let depo = Arc::new(PaylasimliDepo::yeni());
        let ctx = baglam(depo.clone(), Arc::new(IzToplayici::yeni()));
        (arac, hafiza, depo, ctx)
    }

    #[test]
    fn hatirla_notu_kaydeder_ve_dunyayi_degistirir() {
        let (arac, hafiza, _, mut ctx) = kur();
        let s = yurut(arac.calistir(
            json!({"eylem":"hatirla","metin":"Kullanici vejetaryendir.",
                   "anahtarlar":"yemek, restoran","tur":"tercih"}),
            &mut ctx,
        ));
        assert_eq!(s.durum, AracDurumu::Yazildi, "kayit dunyayi degistirir");
        assert!(s.cip_metni.starts_with("Not alindi"));
        assert_eq!(s.modele_donen, "note saved", "modele kisa ve Ingilizce");
        assert_eq!(hafiza.ile(|d| d.sayi()), Some(1));
        assert!(ctx.oturum_kirli(), "hafiza kisisel veri tasir");
    }

    #[test]
    fn kisa_not_reddedilir_ve_oturum_kirlenmez() {
        let (arac, hafiza, _, mut ctx) = kur();
        let s = yurut(arac.calistir(
            json!({"eylem":"hatirla","metin":"kisa","anahtarlar":"a"}),
            &mut ctx,
        ));
        assert!(matches!(s.durum, AracDurumu::Basarisiz(_)));
        assert_eq!(s.modele_donen, sirr_cekirdek::HATA_MODEL_METNI);
        assert_eq!(hafiza.ile(|d| d.sayi()), Some(0));
        assert!(!ctx.oturum_kirli(), "basarisiz cagri oturumu kirletmez");
    }

    #[test]
    fn unut_notu_siler_bos_ifadeyi_reddeder() {
        let (arac, hafiza, _, mut ctx) = kur();
        yurut(arac.calistir(
            json!({"eylem":"hatirla","metin":"Kullanici vejetaryendir.","anahtarlar":"yemek"}),
            &mut ctx,
        ));
        // Bos ifade TUM notlarla eslesirdi: kapi tutmali.
        let bos = yurut(arac.calistir(json!({"eylem":"unut","metin":"  "}), &mut ctx));
        assert!(matches!(bos.durum, AracDurumu::Basarisiz(_)));
        assert_eq!(hafiza.ile(|d| d.sayi()), Some(1));

        let s = yurut(arac.calistir(json!({"eylem":"unut","metin":"vejetaryen"}), &mut ctx));
        assert_eq!(s.durum, AracDurumu::Yazildi);
        assert_eq!(hafiza.ile(|d| d.sayi()), Some(0));
    }

    #[test]
    fn unut_eslesme_yoksa_durustce_soyler() {
        let (arac, _, _, mut ctx) = kur();
        let s = yurut(arac.calistir(json!({"eylem":"unut","metin":"olmayan sey"}), &mut ctx));
        assert_eq!(s.durum, AracDurumu::Okundu, "silinmediyse dunya degismedi");
        assert_eq!(s.modele_donen, "no matching note");
    }

    #[test]
    fn listele_bypass_kanalindan_gecer() {
        let (arac, _, depo, mut ctx) = kur();
        for i in 0..5 {
            yurut(arac.calistir(
                json!({"eylem":"hatirla","metin":format!("Kullanici {i} numarali olguyu soyledi."),
                       "anahtarlar":"olgu"}),
                &mut ctx,
            ));
        }
        let s = yurut(arac.calistir(json!({"eylem":"listele"}), &mut ctx));
        // Toplu veri MODELE GITMEZ: kisa ozet + kaynak_ref.
        assert!(s.modele_donen.contains("kaynak_ref"));
        assert!(s.modele_donen.len() < 80, "{}", s.modele_donen);
        assert!(!s.modele_donen.contains("numarali olguyu"));
        // Govde depoda TAM duruyor.
        let r = sirr_cekirdek::KaynakRef(
            s.modele_donen
                .split("kaynak_ref=")
                .nth(1)
                .unwrap()
                .trim_end_matches(')')
                .trim()
                .to_string(),
        );
        assert!(depo.al(&r).unwrap().govde.contains("4 numarali"));
    }

    #[test]
    fn bos_hafizada_listele_govde_depolamaz() {
        let (arac, _, _, mut ctx) = kur();
        let s = yurut(arac.calistir(json!({"eylem":"listele"}), &mut ctx));
        assert_eq!(s.modele_donen, "memory is empty");
        assert!(!s.modele_donen.contains("kaynak_ref"));
    }

    #[test]
    fn sema_semaya_uymayan_argumani_reddeder() {
        let (arac, _, _, mut ctx) = kur();
        // Zorunlu alan yok.
        assert!(matches!(
            yurut(arac.calistir(json!({}), &mut ctx)).durum,
            AracDurumu::Basarisiz(_)
        ));
        // Enum disi eylem.
        assert!(matches!(
            yurut(arac.calistir(json!({"eylem":"sil-hepsini"}), &mut ctx)).durum,
            AracDurumu::Basarisiz(_)
        ));
        // Uydurma anahtar kacak yol olmasin.
        let js = arac.sema().json_schema();
        assert_eq!(js["additionalProperties"], json!(false));
        assert_eq!(js["required"], json!(["eylem"]));
    }

    #[test]
    fn arac_kirletir_bayragini_bildirir() {
        let (arac, _, _, _) = kur();
        assert!(arac.kirletir_mi(), "kisisel veri onay kapisina takilmali");
        assert_eq!(arac.ad(), "hafiza");
    }

    #[test]
    fn uzun_not_cipte_kirpilir() {
        let (arac, _, _, mut ctx) = kur();
        let uzun = "Kullanici cok uzun bir olguyu tek cumlede anlatti ve bu cumle surdu.";
        let s = yurut(arac.calistir(
            json!({"eylem":"hatirla","metin":uzun,"anahtarlar":"olgu"}),
            &mut ctx,
        ));
        assert!(s.cip_metni.chars().count() < 60, "{}", s.cip_metni);
        assert!(s.cip_metni.contains('…'));
    }

    #[test]
    fn ayni_olgu_iki_kez_kaydedilmez() {
        let (arac, hafiza, _, mut ctx) = kur();
        let cagri = json!({"eylem":"hatirla","metin":"Kullanici vejetaryendir.","anahtarlar":"yemek"});
        yurut(arac.calistir(cagri.clone(), &mut ctx));
        let ikinci = yurut(arac.calistir(cagri, &mut ctx));
        assert!(matches!(ikinci.durum, AracDurumu::Basarisiz(_)));
        assert_eq!(hafiza.ile(|d| d.sayi()), Some(1));
    }

    /// Cekirdekte tokio yok; test icin yeten minimal yurutucu (hesap.rs ile ayni kalip).
    fn yurut<F: std::future::Future>(mut f: F) -> F::Output {
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        fn bos(_: *const ()) {}
        fn klon(p: *const ()) -> RawWaker {
            RawWaker::new(p, &VT)
        }
        static VT: RawWakerVTable = RawWakerVTable::new(klon, bos, bos, bos);
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VT)) };
        let mut cx = Context::from_waker(&waker);
        let mut f = unsafe { std::pin::Pin::new_unchecked(&mut f) };
        loop {
            if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }
}
