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
pub use candle_motor::{CandleMotor, Mimari, ModelAyari};

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
    fn chatml_sablonu_rolleri_citle_ayirir() {
        use crate::istem::Sablon;
        let istem = Istem::yeni("Sen sirr'sin.", "yarin ne var?")
            .araclarla(&katalog())
            .gecmisle([Tur::kullanici("merhaba"), Tur::asistan("selam"), Tur::arac("sonuc: 3")]);
        let m = istem.metin_sablonlu(Sablon::ChatML);

        // Arac tarifi SISTEM turunun icinde: ayri bir tur olsaydi butce
        // baskisinda "eski tur" sanilip kirpilabilirdi.
        let sistem_sonu = m.find("<|im_end|>").unwrap();
        assert!(m.find("takvim_oku").unwrap() < sistem_sonu);

        assert!(m.contains("<|im_start|>system\n"));
        assert!(m.contains("<|im_start|>user\nmerhaba<|im_end|>"));
        assert!(m.contains("<|im_start|>assistant\nselam<|im_end|>"));
        // ARAC CIKTISI `user` ROLUNDE ve `<tool_response>` ile sarili.
        // Qwen3'un GGUF ustverisindeki resmi sablonundan dogrulandi: `tool`
        // rolu dalinda yazilan cit `<|im_start|>user`dir. Onceden burada
        // `<|im_start|>tool` uretiliyordu; model egitimde hic gormedigi bir rol
        // adi gorup arac sonucunu baglamsiz metin sanip araci tekrar cagiriyordu.
        assert!(
            m.contains("<|im_start|>user\n<tool_response>\nsonuc: 3\n</tool_response><|im_end|>"),
            "\n{m}"
        );
        assert!(!m.contains("<|im_start|>tool"), "\n{m}");
        // Uretim capasi en sonda ve ACIK kalir.
        assert!(m.ends_with("<|im_start|>assistant\n"));
    }

    /// Gemma'nin SISTEM ROLU YOKTUR. En kolay yapilacak hata, ChatML'i
    /// kopyalayip etiketleri degistirmek ve `<start_of_turn>system` yazmaktir;
    /// bu dizgi Gemma'nin egitim verisinde GECMEZ ve talimat baglayiciligini
    /// yitirir. Bu test tam da onu yasaklar.
    #[test]
    fn gemma_sablonu_sistem_rolu_uretmez() {
        use crate::istem::Sablon;
        let istem = Istem::yeni("Sen sirr'sin.", "yarin ne var?").araclarla(&katalog());
        let m = istem.metin_sablonlu(Sablon::Gemma);

        assert!(!m.contains("system"), "Gemma'da sistem rolu yok:\n{m}");
        assert!(!m.contains("<|im_start|>"), "ChatML citleri sizmis:\n{m}");
        // Sistem talimati ve arac tarifi ILK KULLANICI turunun icinde.
        assert!(m.starts_with("<start_of_turn>user\nSen sirr'sin."));
        assert!(m.contains("takvim_oku"));
        // Gecmis yokken tek bir user turu olur: iki ardisik `user` Gemma'nin
        // donusumlu sablonunu bozardi.
        assert_eq!(m.matches("<start_of_turn>user").count(), 1, "\n{m}");
        // Uretim capasi `model`, `assistant` DEGIL.
        assert!(m.ends_with("<start_of_turn>model\n"), "\n{m}");
    }

    #[test]
    fn gemma_sablonu_gecmiste_rolleri_donusumlu_yazar() {
        use crate::istem::Sablon;
        let istem = Istem::yeni("Sen sirr'sin.", "yarin ne var?")
            .gecmisle([Tur::kullanici("merhaba"), Tur::asistan("selam"), Tur::arac("sonuc: 3")]);
        let m = istem.metin_sablonlu(Sablon::Gemma);

        // Sistem blogu ilk `user` turunun basi oldugu icin "merhaba" ONUN
        // devamidir; ayri bir user turu acilsaydi iki ardisik `user` olurdu.
        assert!(m.starts_with("<start_of_turn>user\nSen sirr'sin.\n\nmerhaba<end_of_turn>"), "\n{m}");
        // Asistanin adi `model`.
        assert!(m.contains("<start_of_turn>model\nselam<end_of_turn>"));
        // ARAC CIKTISI `user` tarafina yazilir (ucuncu bir rol adi uydurmak
        // modelin hic gormedigi bir cit olurdu) ve `<tool_response>` ile
        // isaretlenir. SORU AYNI TURDE devam eder: arac sonucu da soru da
        // `user` oldugu icin ayri turlar acmak Gemma'nin donusumlu sablonunu
        // bozardi.
        assert!(
            m.contains(
                "<start_of_turn>user\n<tool_response>\nsonuc: 3\n</tool_response>\n\nyarin ne var?<end_of_turn>"
            ),
            "\n{m}"
        );
        // ASIL DEGISMEZ: hicbir yerde ardisik AYNI rol turu olmamali. Gemma
        // sablonu donusumlu olmak zorunda; iki `user` turu yan yana gelirse
        // model egitim dagiliminin disina cikar.
        let roller: Vec<&str> = m
            .match_indices("<start_of_turn>")
            .map(|(i, c)| if m[i + c.len()..].starts_with("user") { "user" } else { "model" })
            .collect();
        assert!(roller.windows(2).all(|p| p[0] != p[1]), "ardisik ayni rol: {roller:?}\n{m}");
        assert!(!m.contains("assistant") && !m.contains("tool\n"), "\n{m}");
    }

    /// Kilavuz sorunun HEMEN ONUNDE durmali (bkz. istem.rs BECERI KARARI).
    /// ChatML'de bu dogrulanmisti; Gemma dalinda ayri kod oldugu icin ayri
    /// dogrulama gerekiyor — sessizce kaymasi kolay.
    #[test]
    fn gemma_kilavuzu_sorunun_onunde_tutar() {
        use crate::istem::Sablon;
        for gecmisli in [false, true] {
            let mut istem =
                Istem::yeni("s", "asil soru burada").kilavuzla("KILAVUZ METNI".to_string());
            if gecmisli {
                istem = istem.gecmisle([Tur::kullanici("onceki")]);
            }
            let m = istem.metin_sablonlu(Sablon::Gemma);
            let k = m.find("KILAVUZ METNI").expect("kilavuz yok");
            let s = m.find("asil soru burada").expect("soru yok");
            assert!(k < s, "kilavuz sorunun onunde degil (gecmisli={gecmisli}):\n{m}");
            // Ikisi de AYNI turun icinde: araya rol siniri girmemeli.
            assert!(!m[k..s].contains("<start_of_turn>"), "araya rol siniri girmis:\n{m}");
        }
    }

    #[test]
    fn duz_sablon_chatml_citlerini_hic_kullanmaz() {
        use crate::istem::Sablon;
        let istem = Istem::yeni("s", "q").araclarla(&katalog());
        let m = istem.metin_sablonlu(Sablon::Duz);
        assert!(!m.contains("<|im_start|>"));
        // Varsayilan `metin()` duz bicimdir: butce olcumu ve CLI ciktisi buna dayanir.
        assert_eq!(m, istem.metin());
    }

    #[test]
    fn sahte_motor_duz_sablon_bildirir() {
        use crate::istem::Sablon;
        // Sablon MOTORUN ozelligidir; yanlis eslesme sessiz bozulma uretecegi
        // icin varsayilanin duz kalmasi bilincli.
        assert_eq!(SahteMotor::betik(Vec::<SahteAdim>::new()).sablon(), Sablon::Duz);
    }

    #[test]
    fn sistem_talimati_yer_tutucu_arac_adi_icermez() {
        // GERCEK MODELDE OLCULDU: 3B model istemdeki yer tutucu cagriyi
        // harfiyen kopyaliyor. Talimatta `arac_adi(` ya da `ad(` gibi sahte
        // bir imza belirirse o regresyon geri gelir.
        assert!(!SISTEM_TALIMATI.contains("arac_adi("));
        assert!(!SISTEM_TALIMATI.contains(" ad("));
        // Ornek GERCEK bir arac uzerinden verilmeli — ve VERILMEK ZORUNDA.
        // Ornegi kaldirmak Gemma'nin kopyalama arizasini cozuyor ama Qwen3-4B'yi
        // bozuyor: olculdu, ornek yokken arac ADINI hic yazmadan dogrudan ciplak
        // JSON (hatta ```json bloklari) uretti, yani cagri ayristirilamaz oldu.
        // Sozle tarif (\"once adi yaz, adi olmayan cagri gecersizdir\") tek
        // basina yetmedi. Ikisi arasinda birincil model kazanir; Gemma'nin
        // kopyalamasi bilinen ve raporlanan bir sinirlilik olarak kaldi.
        assert!(SISTEM_TALIMATI.contains("hesapla({"));
    }

    #[test]
    fn arac_tarifi_yalniz_katalogdan_turer() {
        let m = Istem::yeni("s", "q").araclarla(&katalog()).metin();
        assert!(m.contains("takvim_oku"));
        assert!(m.contains("Takvim etkinliklerini okur."));
        // IMZA KISA BICIMDE: model imza uydurmak zorunda kalmasin ama tam JSON
        // Schema da basilmasin. Argumanlari zaten gramer zorluyor; sema iki
        // yerde birden tutulursa er gec ayrisir ve 4096'lik pencerede tam sema
        // tek basina butcenin buyuk kismini yiyordu.
        assert!(m.contains("takvim_oku(gun: metin)"), "\n{m}");
        assert!(!m.contains("\"required\""), "tam JSON Schema sizmis:\n{m}");
        assert!(!m.contains("additionalProperties"), "tam JSON Schema sizmis:\n{m}");
    }

    /// Zorunlu/istege bagli ayrimi imzada GORUNMEK zorunda: model hangi alani
    /// atlayabilecegini baska hicbir yerden ogrenemez.
    #[test]
    fn kisa_imza_zorunlu_alani_isaretsiz_istege_baglıyi_soru_isaretli_yazar() {
        use sirr_cekirdek::{Alan, ArgSema};
        let sema = ArgSema::nesne(vec![
            Alan::yeni("ifade", ArgSema::metin()).zorunlu(),
            Alan::yeni("basamak", ArgSema::tamsayi()),
        ]);
        assert_eq!(sema.kisa_imza(), "ifade: metin, basamak?: tamsayi");
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

    /// OLCUME DAYALI REGRESYON: tahmin gercek belirtec sayisinin ALTINA
    /// dusmemeli.
    ///
    /// Qwen2.5 belirtecleyicisiyle Turkce duzyazi olculdu: ortalama 2.71
    /// bayt/belirtec. Tahmin bayti 3'e bolerken (eski hali) gercegin %10
    /// altinda kaliyor, kirpma "sigdi" deyip 4096 penceresini asiyordu.
    /// Bu test o orani sabitler: bayt basina en az 1/2.71 belirtec sayilmali.
    #[test]
    fn tahmin_olculen_turkce_yogunlugunun_altina_dusmez() {
        let metin = "Cihaz uzerinde calisan asistanin baglam penceresini \
                     doldurmak icin yazilmis uzunca bir Turkce cumle.";
        // Olculen en kotu hal: 2.71 bayt/belirtec.
        let gercekci_ust_sinir = (metin.len() as f64 / 2.71).ceil() as usize;
        assert!(
            TokenSayaci::tahmin(metin) >= gercekci_ust_sinir,
            "tahmin {} < olculen gercek {}: kirpma pencereyi asar",
            TokenSayaci::tahmin(metin),
            gercekci_ust_sinir
        );
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

/// Mimari cozumlemesi — `candle` ozelligi altinda.
///
/// AGIRLIK GEREKTIRMEZ: burada olculen sey, ustveri dizgisinden dogru modul
/// secimine giden karardir. Gercek GGUF yuklemesi 2.5 GB okur ve CI'da
/// kosamaz; oysa "bilinmeyen mimaride sessizce yanlis modul yukleme" riski
/// tam da bu saf fonksiyonda yasiyor.
#[cfg(all(test, feature = "candle"))]
mod mimari_testleri {
    use crate::candle_motor::Mimari;
    use crate::istem::Sablon;

    #[test]
    fn bilinen_mimariler_dogru_module_ve_sablona_gider() {
        assert_eq!(Mimari::cozumle("qwen2").unwrap(), Mimari::Qwen2);
        assert_eq!(Mimari::cozumle("qwen3").unwrap(), Mimari::Qwen3);
        assert_eq!(Mimari::cozumle("gemma3").unwrap(), Mimari::Gemma3);

        // Sablon mimariye BAGLI: Qwen ailesi ChatML, Gemma kendi bicimi.
        // Yanlis eslesme sessiz bozulmadir (model citleri tanimaz).
        assert_eq!(Mimari::Qwen2.sablon(), Sablon::ChatML);
        assert_eq!(Mimari::Qwen3.sablon(), Sablon::ChatML);
        assert_eq!(Mimari::Gemma3.sablon(), Sablon::Gemma);
    }

    /// EN ONEMLI TEST. Bilinmeyen mimaride "en yakin" module dusmek, modelin
    /// sessizce copluk uretmesi demektir; hata mesaji desteklenenleri saymali
    /// ki kullanici ne yapacagini bilsin.
    #[test]
    fn bilinmeyen_mimari_sessizce_dusmez_hata_doner() {
        for ad in ["llama", "gemma2", "phi3", "qwen", "", "QWEN3"] {
            let sonuc = Mimari::cozumle(ad);
            assert!(sonuc.is_err(), "'{ad}' sessizce kabul edildi");
            let mesaj = sonuc.unwrap_err().to_string();
            assert!(mesaj.contains("desteklenmeyen GGUF mimarisi"), "{mesaj}");
            assert!(mesaj.contains("qwen2, qwen3, gemma3"), "{mesaj}");
        }
    }
}
