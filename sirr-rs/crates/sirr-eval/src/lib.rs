//! sirr-eval — Degerlendirme kosucusu.
//!
//! NE OLCER: modelin zekasini degil, SIRR'IN MANTIGINI. Dogru arac secildi mi,
//! selamlasmaya arac cagrildi mi, `kaynak_ref` kanali toplu veriyi modelden
//! gercekten uzak tuttu mu, tablo markdown zinciri bozulmadan `belge_olustur`a
//! ulasti mi, cozulemeyen bir tarih sessizce bugune mi dustu, kirli oturumda
//! onay kapisi acildi mi.
//!
//! NEDEN VARSAYILAN MOTOR SAHTE: bu sorularin hicbirinin cevabi modele bagli
//! degil. Olcum bir GGUF dosyasina baglansaydi CI'da hic kosmaz, dolayisiyla
//! pratikte hic olculmezdi. Gercek motor `TekMotor` ile AYNI vaka listesine
//! takilir; o zaman olculen sey model secimi olur.
//!
//! AG YOK, KALICI YAZMA YOK: her vaka kendi gecici dizininde koser ve dusunce
//! dizin silinir.

pub mod kosucu;
pub mod ortam;
pub mod rapor;
pub mod vaka;

pub use kosucu::{MotorSecici, SahteSecici, TekMotor, VakaSonucu, kos, kos_vaka};
// Tur butcesi ve sistem talimati ARTIK burada tanimli degil: ikisi de uretim
// davranisi, olcum ayari degil (bkz. sirr-motor/src/oturum.rs). Eski yoldan
// (`sirr_eval::EN_FAZLA_TUR`) gelen cagri yerleri kirilmasin diye yeniden
// disa vuruluyor; tek dogruluk kaynagi motor katmani.
pub use sirr_motor::{EN_FAZLA_TUR, SISTEM_TALIMATI};
pub use ortam::{DIS_ARAC, Ortam, SahteDisArac};
pub use rapor::EvalRaporu;
pub use vaka::{EvalVakasi, SABIT_EPOCH, TABLO_DOSYASI, UZUN_DOSYA, hepsi};

#[cfg(test)]
mod testler {
    use super::*;

    #[test]
    fn vaka_seti_yeterince_genis_ve_adlar_benzersiz() {
        let v = hepsi();
        assert!(v.len() >= 15, "en az 15 vaka bekleniyor, {} var", v.len());
        let mut adlar: Vec<&str> = v.iter().map(|x| x.ad.as_str()).collect();
        adlar.sort_unstable();
        let onceki = adlar.len();
        adlar.dedup();
        assert_eq!(onceki, adlar.len(), "vaka adlari benzersiz olmali");
    }

    #[test]
    fn arac_cagirmamasi_gereken_vakalar_var() {
        // Arac istahi regresyonu bu vakalar olmadan gorunmez.
        assert!(hepsi().iter().any(|v| v.beklenen_arac.is_none()));
    }

    #[test]
    fn tum_vakalar_sahte_motorda_gecer() {
        // Bu testin anlami: eval'in KENDISI dogru. Vakalar sahte motorda
        // gecmiyorsa olcum araci bozuktur, olculen sey degil.
        let rapor = kos(&hepsi(), &SahteSecici);
        assert!(rapor.hepsi_gecti(), "\n{}", rapor.tablo());
    }

    #[test]
    fn kosu_belirlenimcidir() {
        // Iki kosu bit-birebir ayni JSON'u vermeli; aksi halde eval'ler
        // karsilastirilamaz ve "duzeldi mi" sorusu cevapsiz kalir.
        let a = kos(&hepsi(), &SahteSecici).json();
        let b = kos(&hepsi(), &SahteSecici).json();
        assert_eq!(a, b);
    }

    #[test]
    fn rapor_json_ve_tablo_ayni_sayiyi_verir() {
        let rapor = kos(&hepsi(), &SahteSecici);
        let json: serde_json::Value = serde_json::from_str(&rapor.json()).unwrap();
        assert_eq!(json["toplam"].as_u64().unwrap() as usize, rapor.toplam);
        assert_eq!(json["gecen"].as_u64().unwrap() as usize, rapor.gecen);
        assert!(rapor.tablo().contains(&format!("{}/{}", rapor.gecen, rapor.toplam)));
    }

    #[test]
    fn bozuk_vaka_yakalanir() {
        // Kosucunun gercekten OLCTUGUNUN kaniti: kasten yanlis bir iddia
        // KALDI donmeli. Aksi halde "hepsi gecti" hicbir sey soylemez.
        let bozuk = EvalVakasi::yeni("bozuk", "Merhaba")
            .betik(&["Merhaba!"])
            .kanit(&["boyle bir metin yok"]);
        let sonuc = kos_vaka(&bozuk, &SahteSecici);
        assert!(!sonuc.gecti);
        assert!(!sonuc.kusurlar.is_empty());
    }

    #[test]
    fn selamlasmada_arac_cagrilirsa_vaka_kalir() {
        let bozuk = EvalVakasi::yeni("istahli", "Merhaba")
            .betik(&[r#"zaman({"tur":"hepsi"})"#, "Merhaba!"]);
        let sonuc = kos_vaka(&bozuk, &SahteSecici);
        assert!(!sonuc.gecti, "arac istahi yakalanmaliydi");
    }
}
