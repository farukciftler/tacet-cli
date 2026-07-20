//! Eval kosucusu — bir vakayi bastan sona surer.
//!
//! TUR DONGUSU: istem kur -> uret -> cagri var mi -> yurut -> sonucu gecmise
//! yaz -> tekrar. Cagri yoksa uretilen metin CEVAPTIR ve dongu biter.
//!
//! KANIT HAVUZU: dogrulama yalniz son cumleye bakmaz. Modelin cumlesinde
//! "1000" yazmasi aracin 1000'i hesapladiginin kaniti degildir — model sayiyi
//! uydurmus olabilir. Havuz bu yuzden arac ciktilarini, cip metinlerini ve
//! yapisal bayraklari da tasir; iddia "sistem bunu URETTI" duzeyinde kurulur.
//!
//! MOTOR DISARIDAN: `MotorSecici` iki ucu ayirir — `SahteSecici` vakanin
//! betigini kosar (mantik olcumu, deterministik), `TekMotor` gercek bir
//! `MotorSaglayici`yi kosar (model olcumu). Ayni vaka listesi ikisini de
//! besler.

use crate::ortam::{DIS_ARAC, Ortam};
use crate::vaka::EvalVakasi;
use sirr_araclar::yonlendirici::Yonlendirici;
use sirr_gramer::CagriKisiti;
use sirr_araclar::yurutucu::{AracYurutucu, YurutmeSebebi};
use sirr_cekirdek::{AracBaglami, AracKatalogu, IzToplayici};
use sirr_motor::{
    EN_FAZLA_TUR, Istem, MotorSaglayici, OrneklemeAyari, SISTEM_TALIMATI, SahteMotor, Tur, bekle,
};
use std::sync::Arc;

/// Vakaya motor ureten merci.
pub trait MotorSecici {
    fn motor_ver(&self, vaka: &EvalVakasi) -> Arc<dyn MotorSaglayici>;
    fn ad(&self) -> &str;
}

/// Vakanin betigini kosan sahte motor. Varsayilan.
pub struct SahteSecici;

impl MotorSecici for SahteSecici {
    fn motor_ver(&self, vaka: &EvalVakasi) -> Arc<dyn MotorSaglayici> {
        // Betik bitince hata degil sabit metin: tur sayisini vaka onceden
        // bilmek zorunda kalmasin.
        Arc::new(SahteMotor::betik(vaka.betik.clone()).varsayilanla("Tamam."))
    }
    fn ad(&self) -> &str {
        "sahte"
    }
}

/// Gercek motor icin arayuz: tek bir saglayici tum vakalari kosar.
pub struct TekMotor(pub Arc<dyn MotorSaglayici>);

impl MotorSecici for TekMotor {
    fn motor_ver(&self, _vaka: &EvalVakasi) -> Arc<dyn MotorSaglayici> {
        Arc::clone(&self.0)
    }
    fn ad(&self) -> &str {
        self.0.ad()
    }
}

/// Tek bir vakanin sonucu.
#[derive(Debug, Clone, serde::Serialize)]
pub struct VakaSonucu {
    pub ad: String,
    pub gecti: bool,
    /// Cagrilan araclar, sirayla.
    pub cagrilan: Vec<String>,
    /// Neden gecmedi — bos ise gecti.
    pub kusurlar: Vec<String>,
    /// Modelin son cumlesi.
    pub cevap: String,
}

/// Bir vakayi kosar. `Result` DONMEZ: ortam kurulamasa bile bu bir VAKA
/// sonucudur (basarisiz), kosunun tamami degil.
pub fn kos_vaka(vaka: &EvalVakasi, secici: &dyn MotorSecici) -> VakaSonucu {
    let ortam = match Ortam::kur() {
        Ok(o) => o,
        Err(e) => {
            return VakaSonucu {
                ad: vaka.ad.clone(),
                gecti: false,
                cagrilan: Vec::new(),
                kusurlar: vec![format!("ortam kurulamadi: {e}")],
                cevap: String::new(),
            };
        }
    };

    let katalog = ortam.katalog();
    let yurutucu = AracYurutucu::yeni(katalog.clone()).dis_arac(DIS_ARAC);
    // IZ TOPLAYICI ARTIK ELDE TUTULUYOR. Onceden burada `Arc::new(...)` inline
    // yaratiliyordu ve tutamak atiliyordu: `izler()` hic cagrilamadigi icin
    // eval, `cip_guncelle` ile yapilan DURUM guncellemelerini hic gozlemiyordu.
    // Bir arac cipi yanlis duruma set etse (ornegin yazma isini `Okundu` diye
    // raporlasa) eval bunu yakalayamazdi — kanit havuzu yalnizca `AracSonucu`yu
    // goruyordu.
    let izler = Arc::new(IzToplayici::yeni());
    let mut ctx = AracBaglami::yeni(
        Arc::clone(&ortam.depo) as Arc<dyn sirr_cekirdek::VeriDeposu>,
        ortam.dizin(),
        Arc::clone(&izler) as Arc<dyn sirr_cekirdek::Raporlayici>,
    );

    let motor = secici.motor_ver(vaka);
    // Gramer kisiti eval'de de KOSAR: uretim yolu kisitli, eval kisitsiz
    // olsaydi olculen sey uygulamanin davranisi olmazdi. Istisna, yurutucunun
    // ALT katini olcen vakalar (bkz. `EvalVakasi::kisitsiz`).
    let kisit = (!vaka.kisitsiz)
        .then(|| motor.dagarcik().map(|d| CagriKisiti::yeni(&d, &katalog)))
        .flatten();
    let yonlendirici = Yonlendirici::yeni();
    let bilet = yurutucu.aktif_tur();

    let mut gecmis: Vec<Tur> = Vec::new();
    let mut cagrilan: Vec<String> = Vec::new();
    // Havuz: modelin degil SISTEMIN urettigi her sey.
    let mut kanit = String::new();
    let mut cevap = String::new();
    let mut kusurlar: Vec<String> = Vec::new();

    for _ in 0..EN_FAZLA_TUR {
        // Arac butcesi her turda YENIDEN uygulanir: gecmis buyudukce secim
        // degismemeli, secim yalniz kullanici mesajindan turemeli.
        let secili: AracKatalogu =
            yonlendirici.sec(&vaka.girdi, &katalog).into_iter().collect();
        let istem = Istem::yeni(SISTEM_TALIMATI, &vaka.girdi)
            .araclarla(&secili)
            .gecmisle(gecmis.clone());

        let uretim = match bekle(motor.uret(
            &istem,
            kisit.as_ref().map(|k| k as &dyn sirr_motor::Kisitlayici),
            OrneklemeAyari::default(),
        )) {
            Ok(u) => u,
            Err(e) => {
                kusurlar.push(format!("motor hatasi: {e}"));
                break;
            }
        };
        // Yarim cikti AYRISTIRILMAZ: belirtec tavanina carpmis bir arac
        // cagrisi yarim JSON'dur.
        if !uretim.bitis.tam_mi() {
            kusurlar.push("uretim yarim kesildi".into());
            break;
        }

        let Some(sonuc) = bekle(yurutucu.ham_yurut(&uretim.metin, bilet, &mut ctx)) else {
            cevap = uretim.metin.clone();
            kanit.push('\n');
            kanit.push_str(&cevap);
            break;
        };

        cagrilan.push(sonuc.arac_adi.clone());
        kanit.push('\n');
        kanit.push_str(&sonuc.modele_donen);
        kanit.push('\n');
        kanit.push_str(&sonuc.cip_metni);
        if let Some(h) = &sonuc.ham_cikti {
            kanit.push('\n');
            kanit.push_str(h);
        }
        // Yapisal bayraklar da kanittir: "retry yasak" iddiasi metinden degil
        // buradan dogrulanir (bkz. yurutucu, metin eslestirme yasagi).
        kanit.push_str(&format!(
            "\ntekrar_denenebilir={} dunya_degisti={} sebep={:?}",
            sonuc.tekrar_denenebilir, sonuc.dunya_degisti, sonuc.sebep
        ));

        // Onay reddi ve iptal turu BITIRIR: model israr dongusune girmesin.
        if matches!(sonuc.sebep, YurutmeSebebi::OnayReddedildi | YurutmeSebebi::Iptal) {
            gecmis.push(Tur::arac(sonuc.modele_donen.clone()));
            continue;
        }
        gecmis.push(Tur::arac(sonuc.modele_donen.clone()));
    }

    // CIP IZLERI DE KANITTIR. Toplayicidan gelen durum, `AracSonucu`nun
    // bildirdiginden BAGIMSIZ bir gozlemdir: arac sonucu "Yazildi" derken cipi
    // "Okundu"da biraksa (ya da hic guncellemese) fark yalnizca burada gorunur.
    for iz in izler.izler() {
        kanit.push('\n');
        kanit.push_str(&format!("cip[{}] {} {:?}", iz.ikon, iz.metin, iz.durum));
    }
    // Toplayicinin bagimsiz yan etki gorusu — yurutucununkiyle celisirse
    // ikisinden biri yalan soyluyordur.
    kanit.push_str(&format!("\ncip_dunya_degisti={}", izler.dunya_degisti()));

    // --- Iddialar ---

    match &vaka.beklenen_arac {
        Some(ad) if !cagrilan.iter().any(|c| c == ad) => {
            kusurlar.push(format!("beklenen arac cagrilmadi: {ad} (cagrilan: {cagrilan:?})"));
        }
        None if !cagrilan.is_empty() => {
            // Arac istahi: en sik gorulen regresyon.
            kusurlar.push(format!("arac cagrilmamaliydi, cagrildi: {cagrilan:?}"));
        }
        _ => {}
    }

    for parca in &vaka.beklenen_kanit {
        if !kanit.contains(parca.as_str()) {
            kusurlar.push(format!("kanit yok: {parca:?}"));
        }
    }
    for parca in &vaka.yasak {
        if kanit.contains(parca.as_str()) {
            kusurlar.push(format!("yasak kanit gorundu: {parca:?}"));
        }
    }

    VakaSonucu {
        ad: vaka.ad.clone(),
        gecti: kusurlar.is_empty(),
        cagrilan,
        kusurlar,
        cevap,
    }
}

/// Tum vakalari kosar.
pub fn kos(vakalar: &[EvalVakasi], secici: &dyn MotorSecici) -> crate::rapor::EvalRaporu {
    let sonuclar = vakalar.iter().map(|v| kos_vaka(v, secici)).collect();
    crate::rapor::EvalRaporu::yeni(secici.ad(), sonuclar)
}
