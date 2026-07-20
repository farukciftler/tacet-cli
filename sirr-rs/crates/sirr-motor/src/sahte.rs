//! SahteMotor — betikle surulen, agirlik ISTEMEYEN deterministik motor.
//!
//! NEDEN VARSAYILAN: tur dongusu, arac yonlendirmesi, butce kirpma ve kisit
//! akisi mantiktir; model degil. Bu mantigin testi bir GGUF dosyasina bagli
//! olsaydi CI'da hic kosmaz, dolayisiyla pratikte hic test edilmezdi. Tum
//! eval ve CI bu motorun uzerinde koser; `candle` ozelligi kapaliyken bile
//! sirr'in mantik katmani bastan sona dogrulanabilir.
//!
//! GORULEN ISTEMLERI KAYDEDER: testin "kilavuz istemin icinde miydi", "eski
//! tur dusuruldu mu" gibi sorulari dogrudan sorabilmesi icin. Ciktiyi
//! dogrulamak yetmez — istemin nasil KURULDUGU da bu crate'in sozudur.

use crate::hata::MotorHatasi;
use crate::istem::Istem;
use crate::kisit::Kisitlayici;
use crate::saglayici::{
    BitisNedeni, MotorSaglayici, OrneklemeAyari, Uretim, UretimGelecegi, kutula_uretim,
};
use std::sync::Mutex;

/// Betikteki tek adim.
#[derive(Debug, Clone)]
pub enum SahteAdim {
    /// Bu metni uret.
    Uret(String),
    /// Bu hatayi don — kurtarma yollarinin testi icin.
    Coz(String),
}

impl<S: Into<String>> From<S> for SahteAdim {
    fn from(v: S) -> Self {
        SahteAdim::Uret(v.into())
    }
}

#[derive(Default)]
struct Ic {
    adimlar: Vec<SahteAdim>,
    sirada: usize,
    gorulen_istemler: Vec<String>,
    /// Kisit oturumu kac kez ilerletildi — "kisit gercekten cagrildi mi"
    /// sorusunun tek kaniti.
    kisit_adimi: usize,
    kisit_adlari: Vec<String>,
}

pub struct SahteMotor {
    ic: Mutex<Ic>,
    /// Betik bitince hata yerine bu metin donsun (None = hata).
    varsayilan: Option<String>,
}

impl SahteMotor {
    /// Sirayla dondurulecek ciktilarla kurar.
    pub fn betik<S: Into<SahteAdim>>(adimlar: impl IntoIterator<Item = S>) -> Self {
        Self {
            ic: Mutex::new(Ic {
                adimlar: adimlar.into_iter().map(Into::into).collect(),
                ..Default::default()
            }),
            varsayilan: None,
        }
    }

    /// Betik bittiginde hata yerine sabit metin donen surum. Tur SAYISINI
    /// onceden bilmeyen testler (dongu sonlanma testleri) icin.
    pub fn varsayilanla(mut self, metin: impl Into<String>) -> Self {
        self.varsayilan = Some(metin.into());
        self
    }

    /// Motora gonderilmis istemlerin TAM metinleri, sirayla.
    pub fn gorulen_istemler(&self) -> Vec<String> {
        self.ic.lock().expect("sahte motor kilidi").gorulen_istemler.clone()
    }

    /// Son gorulen istem — testlerin en sik sordugu sey.
    pub fn son_istem(&self) -> Option<String> {
        self.ic.lock().expect("sahte motor kilidi").gorulen_istemler.last().cloned()
    }

    pub fn cagri_sayisi(&self) -> usize {
        self.ic.lock().expect("sahte motor kilidi").gorulen_istemler.len()
    }

    /// Kisit oturumunun kac belirtecle ilerletildigi.
    pub fn kisit_adimi(&self) -> usize {
        self.ic.lock().expect("sahte motor kilidi").kisit_adimi
    }

    /// Kullanilan kisitlarin adlari, cagri sirasiyla.
    pub fn kisit_adlari(&self) -> Vec<String> {
        self.ic.lock().expect("sahte motor kilidi").kisit_adlari.clone()
    }
}

/// Sahte kelime dagarciginin boyu. Gercek bir belirtecleyici yok; kod noktasi
/// = belirtec kimligi kabul ediliyor (asagidaki gerekceye bakiniz), dolayisiyla
/// dagarcik Unicode kod noktasi uzayinin testlere yeten bir dilimi kadar.
const SAHTE_DAGARCIK: usize = 0x1000;

impl MotorSaglayici for SahteMotor {
    fn ad(&self) -> &str {
        "sahte"
    }

    /// Kod noktasi = belirtec kimligi (uretim yolundaki kabulun aynisi), yani
    /// dagarcik ilk `SAHTE_DAGARCIK` kod noktasinin metinleri. Bunu bildirmek
    /// sahte motorda da GERCEK kisiti kosturur: gramerin uretim dongusune
    /// bagli oldugu eval'de sahte bir "kisit vardi" izlenimi degil, gercekten
    /// maskelenmis bir uretim olculur.
    fn dagarcik(&self) -> Option<Vec<String>> {
        Some(
            (0..SAHTE_DAGARCIK)
                .map(|i| char::from_u32(i as u32).map(String::from).unwrap_or_default())
                .collect(),
        )
    }

    fn uret<'a>(
        &'a self,
        istem: &'a Istem,
        kisit: Option<&'a dyn Kisitlayici>,
        ayar: OrneklemeAyari,
    ) -> UretimGelecegi<'a> {
        kutula_uretim(async move {
            let metin = {
                let mut ic = self.ic.lock().expect("sahte motor kilidi");
                ic.gorulen_istemler.push(istem.metin());
                let sira = ic.sirada;
                ic.sirada += 1;
                match ic.adimlar.get(sira).cloned() {
                    Some(SahteAdim::Uret(m)) => m,
                    Some(SahteAdim::Coz(m)) => return Err(MotorHatasi::Cikarim(m)),
                    None => match &self.varsayilan {
                        Some(m) => m.clone(),
                        None => return Err(MotorHatasi::BetikBitti { cagri: sira + 1 }),
                    },
                }
            };

            // KISIT GERCEKTEN SURULUR. Betikteki metni oylece dondurmek testi
            // yalanci yapardi: kisit yolu hic calismadan "gecti" gorunurdu.
            // Gercek belirtecleyici olmadigi icin KOD NOKTASI = belirtec
            // kabul ediliyor — mutlak kimlikler onemli degil, onemli olan
            // maskele/ilerlet cifti ile kabul durumunun akisi.
            let mut uretilen = 0usize;
            let mut cikti = String::new();
            let mut bitis = BitisNedeni::Belirtec;

            if let Some(k) = kisit {
                self.ic.lock().expect("sahte motor kilidi").kisit_adlari.push(k.ad().to_string());
                let mut oturum = k.oturum();
                let mut logits = vec![0.0f32; SAHTE_DAGARCIK];

                for ch in metin.chars() {
                    if uretilen >= ayar.en_cok_belirtec {
                        bitis = BitisNedeni::Uzunluk;
                        break;
                    }
                    logits.fill(0.0);
                    oturum.maskele(&mut logits);
                    let belirtec = ch as u32;
                    // Maske gercekten uygulaniyor mu: yasaklanmis bir belirteci
                    // uretmeye calisan betik SESSIZCE gecmemeli.
                    if (belirtec as usize) < logits.len()
                        && logits[belirtec as usize] == f32::NEG_INFINITY
                    {
                        return Err(MotorHatasi::KisitIhlali(belirtec));
                    }
                    oturum.ilerlet(belirtec)?;
                    self.ic.lock().expect("sahte motor kilidi").kisit_adimi += 1;
                    cikti.push(ch);
                    uretilen += 1;
                    if oturum.bitti_mi() {
                        bitis = BitisNedeni::KisitTamam;
                        break;
                    }
                }
            } else {
                for ch in metin.chars() {
                    if uretilen >= ayar.en_cok_belirtec {
                        bitis = BitisNedeni::Uzunluk;
                        break;
                    }
                    cikti.push(ch);
                    uretilen += 1;
                }
            }

            Ok(Uretim::yeni(cikti, uretilen, bitis))
        })
    }
}
