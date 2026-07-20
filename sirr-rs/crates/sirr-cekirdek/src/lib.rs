//! sirr-cekirdek — mimarinin sozlesme katmani.
//!
//! Bu crate'te IS YAPILMAZ; burada yalniz herkesin uzerinde anlastigi tipler
//! durur: bir aracin ne oldugu (`Arac`), argümanlarini nasil tarif ettigi
//! (`ArgSema`), ne dondurdugu (`AracSonucu`), nasil basarisiz oldugu
//! (`AracHatasi`) ve toplu veriyi modelden nasil UZAK tuttugu (`VeriDeposu`).
//!
//! Bilerek bagimsizdir: ne dosya sistemine, ne OOXML'e, ne model calistiricisina
//! bakar. Ustteki tum crate'ler buna bagimli, bu hicbirine bagimli degil —
//! yani sozlesme, uygulamalarin baskisiyla egilmez.
//!
//! AG YOK: bu crate'te ve altinda hicbir yerde ag cagrisi bulunmaz.

pub mod arac;
pub mod baglam;
pub mod durum;
pub mod hata;
pub mod katalog;
pub mod raporlayici;
pub mod sema;
pub mod sonuc;
pub mod veri_deposu;

pub use arac::{Arac, AracGelecegi, kutula};
pub use baglam::AracBaglami;
pub use durum::AracDurumu;
pub use hata::{AracHatasi, AracSonuc, HATA_MODEL_METNI};
pub use katalog::AracKatalogu;
pub use raporlayici::{
    AracIzi, IzGuncelleme, IzKimligi, IzToplayici, Raporlayici, SessizRaporlayici,
};
pub use sema::{Alan, ArgSema, SemaTipi};
pub use sonuc::{AracSonucu, kaynak_ref_eki};
pub use veri_deposu::{BellekVeriDeposu, Kayit, KaynakRef, VeriDeposu};

#[cfg(test)]
mod testler {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    /// Sozlesmenin dyn-uyumlulugunu test SEVIYESINDE kanitlar: bu arac
    /// derleniyorsa `Vec<Arc<dyn Arac>>` de derleniyor demektir.
    struct SahteArac;

    impl Arac for SahteArac {
        fn ad(&self) -> &str {
            "sahte_ara"
        }
        fn aciklama(&self) -> &str {
            "Test amacli; cok kayit uretir."
        }
        fn sema(&self) -> ArgSema {
            ArgSema::nesne(vec![
                Alan::yeni("sorgu", ArgSema::metin().aciklama("aranacak metin")).zorunlu(),
                Alan::yeni("kapsam", ArgSema::secenek(["tumu", "yakin"])),
                Alan::yeni("adet", ArgSema::tamsayi().aralik(Some(1.0), Some(50.0))),
            ])
        }
        fn kirletir_mi(&self) -> bool {
            true
        }
        fn calistir<'a>(
            &'a self,
            args: serde_json::Value,
            ctx: &'a mut AracBaglami,
        ) -> AracGelecegi<'a> {
            kutula(async move {
                if let Err(h) = self.sema().dogrula(&args) {
                    return AracSonucu::basarisiz(&h);
                }
                // Bypass kanali: govde depoda kalir, modele ozet + referans gider.
                let r = ctx.depola("sahte", "3 kayit", "cok uzun govde".repeat(100));
                ctx.kirlet();
                AracSonucu::ozetle("3 kayit bulundu", "3 kayit", r.as_str())
            })
        }
    }

    #[test]
    fn katalog_dyn_tutar_ve_ad_ile_bulur() {
        let mut k = AracKatalogu::yeni();
        k.ekle(Arc::new(SahteArac));
        assert!(k.bul("sahte_ara").is_some());
        assert!(k.bul("yok").is_none());
        assert_eq!(k.kirletenler(), vec!["sahte_ara"]);
    }

    #[test]
    fn sema_zorunlu_alani_ve_secenegi_dogrular() {
        let s = SahteArac.sema();
        assert!(s.dogrula(&json!({"sorgu": "a"})).is_ok());
        assert!(s.dogrula(&json!({})).is_err());
        assert!(s.dogrula(&json!({"sorgu": "a", "kapsam": "uzak"})).is_err());
        assert!(s.dogrula(&json!({"sorgu": "a", "adet": 99})).is_err());
        assert!(s.dogrula(&json!({"sorgu": 5})).is_err());
    }

    #[test]
    fn sema_json_schema_ve_serde_gidis_donus() {
        let s = SahteArac.sema();
        let js = s.json_schema();
        assert_eq!(js["type"], "object");
        assert_eq!(js["required"], json!(["sorgu"]));
        let metin = serde_json::to_string(&s).unwrap();
        let geri: ArgSema = serde_json::from_str(&metin).unwrap();
        assert_eq!(geri, s);
    }

    #[test]
    fn hata_modele_sabit_ingilizce_doner() {
        let h = AracHatasi::DosyaYok("/a/b".into());
        let s = AracSonucu::basarisiz(&h);
        assert_eq!(s.modele_donen, HATA_MODEL_METNI);
        assert_eq!(s.cip_metni, "Dosya bulunamadi.");
        assert!(matches!(s.durum, AracDurumu::Basarisiz(_)));
    }

    #[test]
    fn kum_havuzu_disari_cikmayi_engeller() {
        let ctx = AracBaglami::yeni(
            Arc::new(BellekVeriDeposu::yeni()),
            "/tmp/sirr",
            Arc::new(SessizRaporlayici),
        );
        assert!(ctx.yolu_coz("a/b.txt").is_ok());
        assert!(ctx.yolu_coz("a/../b.txt").is_ok());
        assert!(ctx.yolu_coz("../disari.txt").is_err());
        assert!(ctx.yolu_coz("/etc/passwd").is_err());
    }

    #[test]
    fn bypass_kanali_govdeyi_modelden_uzak_tutar() {
        let mut ctx = AracBaglami::yeni(
            Arc::new(BellekVeriDeposu::yeni()),
            "/tmp/sirr",
            Arc::new(IzToplayici::yeni()),
        );
        let sonuc = futures_yok(SahteArac.calistir(json!({"sorgu": "test"}), &mut ctx));
        assert!(sonuc.modele_donen.len() < 80, "modele giden metin kisa olmali");
        assert!(sonuc.modele_donen.contains("kaynak_ref"));
        assert!(ctx.oturum_kirli());
        let r = KaynakRef("sahte#1".into());
        assert!(ctx.depodan(&r).unwrap().govde.len() > 1000);
    }

    /// tokio bagimliligi cekirdekte yok; test icin yeter minimum yurutucu.
    fn futures_yok<F: std::future::Future>(mut f: F) -> F::Output {
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
