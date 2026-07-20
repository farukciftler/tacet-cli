//! sirr-motor — model calistiricisinin SOYUTLAMASI.
//!
//! Bu crate bir modeli calistirmayi degil, "model calistirmak"in ne demek
//! oldugunu tanimlar: istem nasil kurulur (`Istem`), baglam butcesi asilinca
//! ne feda edilir (`TokenSayaci`), uretim nasil dilbilgisine zorlanir
//! (`Kisitlayici`) ve bunlari yapan sey nasil degistirilebilir olur
//! (`MotorSaglayici`).
//!
//! KULLANICI KARARI — SAF RUST: cikarim Candle / mistral.rs ailesiyle yapilir,
//! llama.cpp FFI YOK. Gerekce sirr'in kimligiyle ayni: C++ bir bagimliligin
//! derleme, capraz-derleme ve guvenlik yuzeyi tek basina projenin geri
//! kalanindan buyuk.
//!
//! VARSAYILAN DERLEME CANDLE CEKMEZ. Gercek cikarim `candle` ozelligi
//! arkasindadir; `SahteMotor` varsayilan olarak derlenir ve tum eval/CI onun
//! uzerinde koser. Bu tersine cevrilseydi mantik katmaninin testi bir agirlik
//! dosyasina ve dakikalarca suren bir derlemeye bagli olurdu.
//!
//! AG YOK: bu crate hicbir ag cagrisi yapmaz; model dosyasi disaridan verilir.

pub mod hata;
pub mod istem;
pub mod kisit;
pub mod oturum;
pub mod saglayici;
pub mod sahte;
pub mod token;
pub mod yurutucu;

#[cfg(feature = "candle")]
pub mod candle_motor;

pub use hata::{MotorHatasi, MotorSonuc};
pub use istem::{Istem, KILAVUZ_SINIRI, Rol, Tur};
pub use kisit::{KisitOturumu, Kisitlayici, SerbestKisit};
pub use oturum::{EN_FAZLA_TUR, SISTEM_TALIMATI};
pub use saglayici::{
    BitisNedeni, MotorSaglayici, OrneklemeAyari, Uretim, UretimGelecegi, kutula_uretim,
};
pub use sahte::{SahteAdim, SahteMotor};
pub use token::{BAGLAM_BUTCESI, KirpmaRaporu, TokenSayaci, URETIM_PAYI};
pub use yurutucu::bekle;

#[cfg(feature = "candle")]
pub use candle_motor::{CandleMotor, ModelAyari};

#[cfg(test)]
mod testler {
    use super::*;
    use sirr_cekirdek::{
        Alan, Arac, AracBaglami, AracGelecegi, AracKatalogu, AracSonucu, ArgSema, kutula,
    };
    use std::sync::Arc;

    // --- Test yardimcilari ---

    struct SahteArac;

    impl Arac for SahteArac {
        fn ad(&self) -> &str {
            "takvim_oku"
        }
        fn aciklama(&self) -> &str {
            "Takvim etkinliklerini okur."
        }
        fn sema(&self) -> ArgSema {
            ArgSema::nesne(vec![Alan::yeni("gun", ArgSema::metin()).zorunlu()])
        }
        fn calistir<'a>(
            &'a self,
            _args: serde_json::Value,
            _ctx: &'a mut AracBaglami,
        ) -> AracGelecegi<'a> {
            kutula(async move { AracSonucu::okundu("ok", "ok") })
        }
    }

    fn katalog() -> AracKatalogu {
        let mut k = AracKatalogu::yeni();
        k.ekle(Arc::new(SahteArac));
        k
    }

    /// Belirli bir karakteri yasaklayan, 5 belirtecte "kabul"e gecen oyuncak
    /// kisit. Gercek gramer sirr-gramer'in isi; buradaki tek gorevi
    /// maskele/ilerlet/bitti_mi ucgeninin motor tarafindan SURULDUGUNU
    /// kanitlamak.
    struct OyuncakKisit {
        yasak: char,
    }

    struct OyuncakOturum {
        yasak: char,
        sayac: usize,
    }

    impl Kisitlayici for OyuncakKisit {
        fn oturum(&self) -> Box<dyn KisitOturumu> {
            Box::new(OyuncakOturum { yasak: self.yasak, sayac: 0 })
        }
        fn ad(&self) -> &str {
            "oyuncak"
        }
    }

    impl KisitOturumu for OyuncakOturum {
        fn maskele(&self, logits: &mut [f32]) {
            let i = self.yasak as usize;
            if i < logits.len() {
                logits[i] = f32::NEG_INFINITY;
            }
        }
        fn ilerlet(&mut self, _belirtec: u32) -> MotorSonuc<()> {
            self.sayac += 1;
            Ok(())
        }
        fn bitti_mi(&self) -> bool {
            self.sayac >= 5
        }
    }

    // --- Istem kurulumu ---

    #[test]
    fn istem_parcalari_sabit_sirada_dizilir() {
        let istem = Istem::yeni("Sen sirr'sin.", "yarin ne var?")
            .araclarla(&katalog())
            .kilavuzla("Once takvim_oku cagir.")
            .gecmisle([Tur::kullanici("merhaba"), Tur::asistan("selam")]);

        let m = istem.metin();
        let sira = |igne: &str| m.find(igne).unwrap_or_else(|| panic!("yok: {igne}"));

        assert!(sira("<sistem>") < sira("<araclar>"));
        assert!(sira("<araclar>") < sira("<gecmis>"));
        assert!(sira("<gecmis>") < sira("<guidance>"));
        // Soru EN SONDA: kucuk modelde son blogun agirligi en yuksek.
        assert!(sira("<guidance>") < sira("yarin ne var?"));
        assert!(m.trim_end().ends_with("Asistan:"));
    }

    #[test]
    fn arac_tarifi_yalniz_katalogdan_turer() {
        let m = Istem::yeni("s", "q").araclarla(&katalog()).metin();
        assert!(m.contains("takvim_oku"));
        assert!(m.contains("Takvim etkinliklerini okur."));
        // Sema istemin icinde: model imza uydurmak zorunda kalmasin.
        assert!(m.contains("\"required\":[\"gun\"]"));
        assert!(m.contains("\"additionalProperties\":false"));
    }

    #[test]
    fn bos_katalog_arac_blogu_uretmez() {
        let m = Istem::yeni("s", "q").araclarla(&AracKatalogu::yeni()).metin();
        assert!(!m.contains("<araclar>"));
    }

    #[test]
    fn kilavuz_bastan_kirpilir_ve_siniri_asmaz() {
        // Cekirdek (somut cagri ornegi) dosyanin BASINDA durur; kesme sondan
        // olsaydi tam da o kisim atilirdi.
        let uzun = format!("CEKIRDEK: takvim_oku(gun)\n{}", "dolgu ".repeat(400));
        let istem = Istem::yeni("s", "q").kilavuzla(&uzun);
        let k = istem.kilavuz.as_deref().unwrap();
        assert!(k.starts_with("CEKIRDEK: takvim_oku(gun)"));
        assert_eq!(k.chars().count(), KILAVUZ_SINIRI);
    }

    #[test]
    fn bos_kilavuz_blok_acmaz() {
        let istem = Istem::yeni("s", "q").kilavuzla("   \n  ");
        assert!(istem.kilavuz.is_none());
        assert!(!istem.metin().contains("<guidance>"));
    }

    // --- Butce kirpma politikasi ---

    fn dolu_istem(tur_sayisi: usize, tur_uzunlugu: usize) -> Istem {
        Istem::yeni("SISTEM TALIMATI", "asil soru burada")
            .araclarla(&katalog())
            .kilavuzla("KILAVUZ METNI")
            .gecmisle(
                (0..tur_sayisi)
                    .map(|i| Tur::kullanici(format!("t{i} {}", "x".repeat(tur_uzunlugu)))),
            )
    }

    #[test]
    fn butce_asilinca_once_eski_turler_duser() {
        let sayaci = TokenSayaci::yeni(1024, 256);
        let mut istem = dolu_istem(40, 200);
        let rapor = sayaci.kirp(&mut istem);

        assert!(rapor.dusen_tur > 0, "eski turlar dusmeliydi");
        // Ucuz olan once feda edildi: kilavuza ve soruya sira gelmedi.
        assert!(!rapor.kilavuz_dustu);
        assert!(!rapor.soru_kirpildi);
        assert!(sayaci.dogrula(&istem).is_ok());
    }

    #[test]
    fn eski_turler_bastan_duser_yeniler_kalir() {
        let sayaci = TokenSayaci::yeni(1024, 256);
        let mut istem = dolu_istem(40, 200);
        sayaci.kirp(&mut istem);
        let kalan: Vec<_> = istem.gecmis.iter().map(|t| t.metin.clone()).collect();
        assert!(!kalan.is_empty());
        // En eski (t0) gitti, en yeni (t39) durdu.
        assert!(!kalan[0].starts_with("t0 "));
        assert!(kalan.last().unwrap().starts_with("t39 "));
    }

    #[test]
    fn turler_bitince_kilavuz_feda_edilir() {
        // Gecmis YOK: kirpmanin ilk basamagi bos, dolayisiyla sira dogrudan
        // ikinci basamaga (kilavuz) gelir.
        let sayaci = TokenSayaci::yeni(400, 100);
        let mut istem = Istem::yeni("SISTEM", "soru")
            .araclarla(&katalog())
            .kilavuzla("K".repeat(700));
        assert!(sayaci.istem_tahmini(&istem) > sayaci.istem_tavani());

        let rapor = sayaci.kirp(&mut istem);
        assert!(rapor.kilavuz_dustu);
        assert!(istem.kilavuz.is_none());
        // Kilavuzu feda etmek yetti: soruya sira gelmedi.
        assert!(!rapor.soru_kirpildi);
        assert!(sayaci.dogrula(&istem).is_ok());
    }

    #[test]
    fn sistem_talimati_ve_arac_tarifi_asla_kirpilmaz() {
        // Butce bilerek imkansiz kadar kucuk: politika yine de bu ikisine
        // dokunmamali, hata donmeli.
        let sayaci = TokenSayaci::yeni(40, 0);
        let sistem = "SISTEM TALIMATI TAM HALIYLE";
        let mut istem = dolu_istem(10, 100);
        istem.sistem = sistem.into();
        sayaci.kirp(&mut istem);

        assert_eq!(istem.sistem, sistem);
        assert!(istem.araclar.contains("takvim_oku"));
        assert!(matches!(
            sayaci.dogrula(&istem),
            Err(MotorHatasi::ButceAsimi { .. })
        ));
    }

    #[test]
    fn son_care_soruyu_sondan_koruyarak_kirpar() {
        let sayaci = TokenSayaci::yeni(300, 0);
        let mut istem = Istem::yeni("S", format!("{} ASIL TALEP", "onsoz ".repeat(300)));
        let rapor = sayaci.kirp(&mut istem);

        assert!(rapor.soru_kirpildi);
        // Kullanicinin asil talebi cumlenin SONUNDA olur.
        assert!(istem.soru.ends_with("ASIL TALEP"));
    }

    #[test]
    fn sigan_istem_hic_dokunulmadan_gecer() {
        let sayaci = TokenSayaci::default();
        let mut istem = dolu_istem(2, 10);
        let onceki = istem.metin();
        let rapor = sayaci.kirp(&mut istem);

        assert!(!rapor.degisti_mi());
        assert_eq!(istem.metin(), onceki);
    }

    #[test]
    fn tahmin_turkce_metni_az_saymaz() {
        // Cok baytli harfler bayt uzerinden sayilir; karakter sayilsaydi
        // Turkce sistematik olarak AZ tahmin edilirdi.
        let turkce = "igusocIGUSOC";
        let cok_baytli = "ığüşöçİĞÜŞÖÇ";
        assert!(TokenSayaci::tahmin(cok_baytli) > TokenSayaci::tahmin(turkce));
        assert_eq!(TokenSayaci::tahmin(""), 0);
    }

    // --- SahteMotor ve kisit akisi ---

    #[test]
    fn sahte_motor_betigi_sirayla_doner_ve_istemi_kaydeder() {
        let motor = SahteMotor::betik(["birinci", "ikinci"]);
        let istem = Istem::yeni("S", "soru bir");
        let u = bekle(motor.uret(&istem, None, OrneklemeAyari::default())).unwrap();
        assert_eq!(u.metin, "birinci");
        assert_eq!(u.bitis, BitisNedeni::Belirtec);

        let istem2 = Istem::yeni("S", "soru iki");
        let u2 = bekle(motor.uret(&istem2, None, OrneklemeAyari::default())).unwrap();
        assert_eq!(u2.metin, "ikinci");

        assert_eq!(motor.cagri_sayisi(), 2);
        assert!(motor.gorulen_istemler()[0].contains("soru bir"));
        assert!(motor.son_istem().unwrap().contains("soru iki"));
    }

    #[test]
    fn betik_bitince_hata_doner_varsayilan_verilirse_donmez() {
        let motor = SahteMotor::betik(["tek"]);
        let istem = Istem::yeni("S", "q");
        bekle(motor.uret(&istem, None, OrneklemeAyari::default())).unwrap();
        assert!(matches!(
            bekle(motor.uret(&istem, None, OrneklemeAyari::default())),
            Err(MotorHatasi::BetikBitti { cagri: 2 })
        ));

        let motor2 = SahteMotor::betik(["tek"]).varsayilanla("bitti");
        bekle(motor2.uret(&istem, None, OrneklemeAyari::default())).unwrap();
        let u = bekle(motor2.uret(&istem, None, OrneklemeAyari::default())).unwrap();
        assert_eq!(u.metin, "bitti");
    }

    #[test]
    fn kisit_her_belirtecte_surulur() {
        let motor = SahteMotor::betik(["abcdefgh"]);
        let kisit = OyuncakKisit { yasak: 'z' };
        let istem = Istem::yeni("S", "q");
        let u = bekle(motor.uret(&istem, Some(&kisit), OrneklemeAyari::default())).unwrap();

        // Kisit 5. belirtecte kabul durumuna geciyor; uretim ORADA duruyor.
        assert_eq!(u.metin, "abcde");
        assert_eq!(u.bitis, BitisNedeni::KisitTamam);
        assert_eq!(motor.kisit_adimi(), 5);
        assert_eq!(motor.kisit_adlari(), vec!["oyuncak"]);
    }

    #[test]
    fn maske_delinirse_uretim_sessizce_gecmez() {
        // Betik yasaklanmis belirteci uretmeye calisiyor: maskeleme gercekten
        // uygulaniyorsa bu bir hata olmali, sessiz kabul degil.
        let motor = SahteMotor::betik(["zebra"]);
        let kisit = OyuncakKisit { yasak: 'z' };
        let istem = Istem::yeni("S", "q");
        assert!(matches!(
            bekle(motor.uret(&istem, Some(&kisit), OrneklemeAyari::default())),
            Err(MotorHatasi::KisitIhlali(_))
        ));
    }

    #[test]
    fn serbest_kisit_hicbir_seyi_engellemez() {
        let motor = SahteMotor::betik(["zzz"]);
        let istem = Istem::yeni("S", "q");
        let u = bekle(motor.uret(&istem, Some(&SerbestKisit), OrneklemeAyari::default())).unwrap();
        assert_eq!(u.metin, "zzz");
        assert_eq!(motor.kisit_adimi(), 3);
    }

    #[test]
    fn belirtec_tavani_asilinca_cikti_yarim_isaretlenir() {
        let motor = SahteMotor::betik(["uzun bir cikti"]);
        let ayar = OrneklemeAyari { en_cok_belirtec: 4, ..Default::default() };
        let u = bekle(motor.uret(&Istem::yeni("S", "q"), None, ayar)).unwrap();

        assert_eq!(u.metin, "uzun");
        assert_eq!(u.bitis, BitisNedeni::Uzunluk);
        // Cagri yeri yarim JSON'u ayristirmaya kalkmasin diye.
        assert!(!u.bitis.tam_mi());
    }

    #[test]
    fn motor_dyn_olarak_tasinabilir() {
        // MotorSaglayici'nin dyn-uyumlulugu sozlesmenin sarti: motor calisma
        // aninda secilir (sahte mi, candle mi).
        let motor: Arc<dyn MotorSaglayici> = Arc::new(SahteMotor::betik(["ok"]));
        assert_eq!(motor.ad(), "sahte");
        let u = bekle(motor.uret(&Istem::yeni("S", "q"), None, OrneklemeAyari::default())).unwrap();
        assert_eq!(u.metin, "ok");
    }

    #[test]
    fn kirpma_sonrasi_istem_motora_sigar() {
        // Uctan uca: kirp -> dogrula -> uret akisi.
        let sayaci = TokenSayaci::yeni(1024, 256);
        let mut istem = dolu_istem(60, 150);
        sayaci.kirp(&mut istem);
        sayaci.dogrula(&istem).unwrap();

        let motor = SahteMotor::betik(["cevap"]);
        bekle(motor.uret(&istem, None, OrneklemeAyari::default())).unwrap();
        let gorulen = motor.son_istem().unwrap();
        assert!(TokenSayaci::tahmin(&gorulen) <= sayaci.istem_tavani());
        assert!(gorulen.contains("SISTEM TALIMATI"));
        assert!(gorulen.contains("asil soru burada"));
    }
}
