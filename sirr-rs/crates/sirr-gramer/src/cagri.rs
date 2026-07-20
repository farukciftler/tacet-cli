//! `CagriKisiti` — gramer ile uretim dongusu arasindaki KOPRU.
//!
//! NEDEN VAR: bu crate (derleme + PDA + maske) yazildi ama uretimde SIFIR
//! etkisi vardi. `sirr-motor` `sirr-gramer`i bagimlilik olarak bile bilmiyordu;
//! `motor.uret(...)`in kisit argumani her cagri yerinde `None` gidiyordu ve
//! `Kisitlayici`yi uygulayan tek somut tip hicbir seyi kisitlamayan
//! `SerbestKisit`ti. Yani "kucuk modeli gecerli JSON'a zorlama" amaci kagit
//! uzerinde kaliyordu. Bu dosya o boslugu kapatir.
//!
//! BAGIMLILIK YONU: `sirr-motor`daki `kisit.rs`in bastan soyledigi yon —
//! gramer motora bagimlidir, motor gramere DEGIL. Sozlesme orada, uygulama
//! burada. Boylece kisitsiz calisan bir kurulum gramer kodunu hic derlemez.
//!
//! NE ZORLANIR, NE ZORLANMAZ (durustce):
//!   ZORLANIR   — model bir kez `arac_adi(` yazdiktan sonra argumanlar o
//!                aracin semasinin GRAMERINE uymak zorundadir. Gecersiz JSON,
//!                semada olmayan alan, enum kumesi disi deger, aralik disi
//!                sayi: hicbiri URETILEMEZ hale gelir. Asil kacak yol buydu.
//!   ZORLANMAZ  — modelin arac cagirmayi SECMESI. Duz cevap mesru bir cikti
//!                oldugu icin baslangicta serbest metin acik kalmak zorunda;
//!                aksi halde "merhaba"ya bile arac cagirmaya zorlanirdi.
//!                Arac ADI da bu yuzden dolayli zorlanir: uydurma bir ada
//!                sahip cikti gramerce "duz metin" sayilir ve `AracYurutucu`nun
//!                1. kapisinda (katalogta yok) reddedilir. Ikinci savunma
//!                hatti orasidir; burasi degil.
//!
//! MASKELEME MALIYETI: serbest metin asamasinda maske HIC uretilmez (no-op),
//! cunku o asamada dagarcigin tamami gecerlidir ve trie gezmek bos yere
//! milisaniye yakardi. Maliyet yalniz argumanlarin icinde odenir — orada da
//! `TokenMaskesi` gecersiz dallari tek hamlede eler.

use crate::{Gramer, GramerDurumu, TokenMaskesi};
use sirr_cekirdek::AracKatalogu;
use sirr_motor::{KisitOturumu, Kisitlayici, MotorHatasi};
use std::sync::Arc;

/// Kisitin degismez govdesi. `Arc` ile paylasilir cunku `Kisitlayici::oturum`
/// `Box<dyn KisitOturumu>` (yani `'static`) doner: oturum kisiti odunc ALAMAZ,
/// pay sahibi olmak zorundadir.
struct Ic {
    /// Dagarcik trie'si GRAMERDEN BAGIMSIZDIR (yalnizca token metinlerine
    /// bakar), bu yuzden tum araclar icin TEK tane kurulur — arac basina bir
    /// trie kurmak ayni isi katalog boyu kadar tekrarlamak olurdu.
    maske: TokenMaskesi,
    dagarcik: Vec<String>,
    /// (arac adi, argumanlarinin derlenmis grameri) — katalog sirasinda.
    araclar: Vec<(String, Arc<Gramer>)>,
    /// ICINDE `(` GECEN belirtecler: (kimlik, `(` oncesi, ilk `(` sonrasi).
    ///
    /// NEDEN ONBELLEK: onek asamasindaki tek gercek tehlike, arac adini
    /// KAPATIP argumanlara da giren tek bir belirtectir (`("` gibi). Bu
    /// belirtecleri her adimda dagarcigi bastan tarayarak bulmak, maskenin
    /// kacinmak icin var oldugu maliyeti geri getirirdi. Parantezli belirtecler
    /// dagarcigin cok kucuk bir azinligi oldugu icin bir kez ayirmak yeter.
    parantezli: Vec<(usize, String, String)>,
}

/// Katalogdan turetilmis cagri kisiti. Bir kez kurulur, tum uretim boyunca
/// paylasilir.
pub struct CagriKisiti {
    ic: Arc<Ic>,
}

impl CagriKisiti {
    /// Katalogdaki her aracin semasini derler ve dagarcigi trie'ye kurar.
    ///
    /// `dagarcik` motordan gelir (`MotorSaglayici::dagarcik`): kisit ancak
    /// belirtecleyiciyi BILEN bir motorla anlamlidir, cunku maske belirtec
    /// kimligi uzerinden konusur.
    pub fn yeni(dagarcik: &[String], katalog: &AracKatalogu) -> Self {
        let araclar = katalog
            .araclar()
            .iter()
            .map(|a| (a.ad().to_string(), Gramer::derle(&a.sema())))
            .collect();
        let parantezli = dagarcik
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                let p = t.find('(')?;
                Some((i, t[..p].to_string(), t[p + 1..].to_string()))
            })
            .collect();
        Self {
            ic: Arc::new(Ic {
                maske: TokenMaskesi::yeni(dagarcik),
                dagarcik: dagarcik.to_vec(),
                araclar,
                parantezli,
            }),
        }
    }
}

impl Kisitlayici for CagriKisiti {
    fn oturum(&self) -> Box<dyn KisitOturumu> {
        Box::new(CagriOturumu {
            ic: Arc::clone(&self.ic),
            asama: Asama::Onek { yenen: String::new() },
        })
    }
    fn ad(&self) -> &str {
        "arac_cagrisi"
    }
}

/// Uretimin hangi bolgesinde oldugumuz.
enum Asama {
    /// Henuz ayrisma olmadi: cikti hala bir arac adinin oneki OLABILIR ama
    /// duz cevap da olabilir. Serbest metin acik oldugu icin maske yok.
    Onek { yenen: String },
    /// `arac_adi(` yendi; argumanlar artik GRAMERE tabi.
    Args { durum: GramerDurumu },
    /// Argumanlar kapandi, `)` de yendi — cagri tam.
    Kapali,
    /// Cikti bir arac adina benzemiyor: duz cevap. Bir daha kisit yok.
    Serbest,
}

struct CagriOturumu {
    ic: Arc<Ic>,
    asama: Asama,
}

/// `yenen` hala bir arac adina tamamlanabilir mi. Serbest fonksiyon: `yut`
/// icinde `self.asama` zaten degisken olarak odunc alinmis oluyor.
fn onek_yasiyor(ic: &Ic, yenen: &str) -> bool {
    let s = yenen.trim_start();
    // Bastaki bosluk henuz bir sey soylemez.
    if s.is_empty() {
        return true;
    }
    ic.araclar.iter().any(|(ad, _)| ad.starts_with(s))
}

impl CagriOturumu {
    /// Tek bir karakteri isler. Karakter bazinda ilerlemek sart: bir belirtec
    /// birden fazla karakter tasir ve asama gecisi (`(` gorulmesi) belirtecin
    /// ORTASINDA olabilir.
    fn yut(&mut self, c: char) -> Result<(), MotorHatasi> {
        let ic = Arc::clone(&self.ic);
        match &mut self.asama {
            Asama::Onek { yenen } => {
                // `(` ayrisma anidir: o ana kadar yenen sey tam bir arac adiysa
                // cagriya, degilse duz metne gecilir.
                if c == '(' {
                    let ad = yenen.trim().to_string();
                    if let Some((_, gramer)) = ic.araclar.iter().find(|(a, _)| *a == ad) {
                        self.asama = Asama::Args { durum: gramer.durum() };
                    } else {
                        self.asama = Asama::Serbest;
                    }
                    return Ok(());
                }
                yenen.push(c);
                if !onek_yasiyor(&ic, yenen) {
                    self.asama = Asama::Serbest;
                }
                Ok(())
            }
            Asama::Args { durum } => {
                // Gramer kabul durumundayken gelen `)` cagriyi kapatir.
                // Once bu bakilir: `)` argumanlarin grameri icinde gecerli
                // degildir, dolayisiyla asagiya birakilsa hata olurdu.
                if durum.bitti_mi() && c == ')' {
                    self.asama = Asama::Kapali;
                    return Ok(());
                }
                durum.ilerlet(&c.to_string()).map_err(|_| {
                    // Buraya gelmek maskeleme ile ilerletmenin ayristigi
                    // anlamina gelir; sessizce yutmak gecersiz bir cagriyi
                    // mesrulastirirdi.
                    MotorHatasi::KisitIhlali(c as u32)
                })
            }
            Asama::Kapali | Asama::Serbest => Ok(()),
        }
    }
}

impl CagriOturumu {
    /// Onek asamasinda YALNIZ parantezli belirtecleri eler (bkz. `maskele`).
    ///
    /// Elenme sarti dar tutuldu: belirtec, `(` oncesiyle birlikte TAM bir arac
    /// adi olusturuyor VE `(` sonrasi kalan o aracin gramerine uymuyorsa
    /// kapatilir. Ad tutmuyorsa belirtec zaten duz metne (`Serbest`) goturur,
    /// yani zararsizdir; kapatmak modelin `(` iceren normal cumleler yazmasini
    /// engellerdi.
    fn onek_maskesi(&self, yenen: &str, logits: &mut [f32]) {
        for (id, onces, kalan) in &self.ic.parantezli {
            if *id >= logits.len() {
                continue;
            }
            let ad = format!("{yenen}{onces}");
            let ad = ad.trim();
            let Some((_, gramer)) = self.ic.araclar.iter().find(|(a, _)| a == ad) else {
                continue;
            };
            if !kalan_gramere_uyuyor(gramer, kalan) {
                logits[*id] = f32::NEG_INFINITY;
            }
        }
    }
}

/// `arac_adi(` ardindan gelen kalan karakterlerin gramerde YASAYIP yasamadigi.
///
/// `yut`un `Asama::Args` dalinin ayni mantigi: kabul durumunda gelen `)` cagriyi
/// kapatir, sonrasi artik gramerin isi degildir.
fn kalan_gramere_uyuyor(gramer: &Arc<Gramer>, kalan: &str) -> bool {
    let mut durum = gramer.durum();
    for c in kalan.chars() {
        if durum.bitti_mi() && c == ')' {
            return true;
        }
        if durum.ilerlet(&c.to_string()).is_err() {
            return false;
        }
    }
    true
}

impl KisitOturumu for CagriOturumu {
    fn maskele(&self, logits: &mut [f32]) {
        let durum = match &self.asama {
            Asama::Args { durum } => durum,
            // ONEK ASAMASI: serbest metin mesru oldugu icin genel maske YOK.
            // Tek istisna, arac adini kapatip AYNI belirtec icinde argumanlara
            // da giren belirtecler (`("` gibi). Bunlar maskelenmezse gramer o
            // belirteci sonradan reddeder ve uretim `KisitIhlali` ile duser —
            // gozlenen ariza tam olarak buydu: `belge_oku` yazildiktan sonra
            // gelen tek belirtec hem `(` hem `"` tasiyordu, oysa gramer `(`
            // ardindan `{` bekliyor. Maskeleme ile ilerletmenin ayrismasi
            // burada onlenir; hatayi sonradan yakalamak gec kalmaktir.
            Asama::Onek { yenen } => {
                self.onek_maskesi(yenen, logits);
                return;
            }
            Asama::Kapali | Asama::Serbest => return,
        };
        // Sonlandirici `)` maskeye GEZI SIRASINDA katilir, sonradan degil.
        //
        // Onceki hali `dagarcik[id].starts_with(')')` diye bir son-ek kontroluydu
        // ve iki yonden de yanlisti. Kacirdigi: `"})` gibi gramer icinde BASLAYIP
        // sonlandiriciyla BITEN belirtecler — ki Qwen2.5'te gecerli bir cagrinin
        // dogal belirteclemesi tam olarak boyle biter, yani en olasi belirtec
        // kapali kaliyordu. Fazladan actigi: `))` ya da `) ...` gibi cagrinin
        // ardina gevezelik ekleyen her belirtec. Gezinin icinde yapilan kontrol
        // ikisini birden duzeltir: onek gramerden GECMEK, kalan ise TAM olarak
        // sonlandirici olmak zorunda.
        let izinli = self.ic.maske.maske_sonlandiricili(durum, Some(')'));
        for (id, logit) in logits.iter_mut().enumerate() {
            // Dagarcigin disindaki indeks: kisit onun hakkinda bir sey
            // soyleyemez, kapatmak en guvenlisi.
            if id >= izinli.len() || !izinli[id] {
                *logit = f32::NEG_INFINITY;
            }
        }
    }

    fn ilerlet(&mut self, belirtec: u32) -> Result<(), MotorHatasi> {
        let metin = self
            .ic
            .dagarcik
            .get(belirtec as usize)
            .cloned()
            .ok_or(MotorHatasi::KisitIhlali(belirtec))?;
        for c in metin.chars() {
            self.yut(c)?;
        }
        Ok(())
    }

    fn bitti_mi(&self) -> bool {
        // YALNIZ tamamlanmis cagri uretimi guvenle keser. Serbest metin
        // asamasinda `true` donmek modelin cumlesini ortasindan keserdi.
        matches!(self.asama, Asama::Kapali)
    }
}

#[cfg(test)]
mod testler {
    use super::*;
    use sirr_cekirdek::{
        Alan, Arac, AracBaglami, AracGelecegi, AracSonucu, ArgSema, kutula,
    };

    /// Zorunlu bir alani ve bir enum alani olan arac — kisitin iki en onemli
    /// iddiasi (eksik zorunlu alan, kume disi deger) bunun uzerinde olculur.
    struct BelgeArac;

    impl Arac for BelgeArac {
        fn ad(&self) -> &str {
            "belge_olustur"
        }
        fn aciklama(&self) -> &str {
            "Belge uretir."
        }
        fn sema(&self) -> ArgSema {
            ArgSema::nesne(vec![
                Alan::yeni("bicim", ArgSema::secenek(["excel", "markdown"])).zorunlu(),
                Alan::yeni("dosya_adi", ArgSema::metin()).zorunlu(),
            ])
        }
        fn calistir<'a>(
            &'a self,
            _args: serde_json::Value,
            _ctx: &'a mut AracBaglami,
        ) -> AracGelecegi<'a> {
            kutula(async move { AracSonucu::okundu("ok", "ok") })
        }
    }

    /// Kod noktasi = belirtec kimligi; SahteMotor'un uretim yolundaki kabulu.
    fn dagarcik() -> Vec<String> {
        (0..0x1000u32)
            .map(|i| char::from_u32(i).map(String::from).unwrap_or_default())
            .collect()
    }

    fn kisit() -> CagriKisiti {
        let mut k = sirr_cekirdek::AracKatalogu::yeni();
        k.ekle(Arc::new(BelgeArac));
        CagriKisiti::yeni(&dagarcik(), &k)
    }

    /// Bir karakterin SU AN uretilebilir olup olmadigi — maskeye sorar.
    fn izinli(oturum: &dyn KisitOturumu, c: char) -> bool {
        let mut logits = vec![0.0f32; 0x1000];
        oturum.maskele(&mut logits);
        logits[c as usize] != f32::NEG_INFINITY
    }

    fn besle(oturum: &mut Box<dyn KisitOturumu>, metin: &str) {
        for c in metin.chars() {
            oturum.ilerlet(c as u32).expect("gecerli metin kabul edilmeli");
        }
    }

    #[test]
    fn gecerli_cagri_bastan_sona_kabul_edilir() {
        let k = kisit();
        let mut o = k.oturum();
        besle(&mut o, r#"belge_olustur({"bicim":"excel","dosya_adi":"rapor"})"#);
        assert!(o.bitti_mi(), "tam cagri kabul durumunda bitmeliydi");
    }

    /// KISITIN ASIL ISI: zorunlu alan doldurulmadan nesne KAPATILAMAZ.
    /// Bu, eval'deki `belge-sema-ihlali` vakasinin uretim asamasindaki
    /// karsiligidir — orada sema kapisi (KAPI 2) olculur, burada gramerin
    /// ayni ihlali daha uretilmeden imkansiz kildigi.
    #[test]
    fn eksik_zorunlu_alanla_nesne_kapatilamaz() {
        let k = kisit();
        let mut o = k.oturum();
        besle(&mut o, r#"belge_olustur({"bicim":"excel""#);
        assert!(!izinli(o.as_ref(), '}'), "eksik `dosya_adi` ile `}}` uretilebiliyor");
        // Devam edip alani doldurunca kapanabilmeli.
        besle(&mut o, r#","dosya_adi":"x""#);
        assert!(izinli(o.as_ref(), '}'), "alan dolduktan sonra kapanamiyor");
    }

    /// Enum degerleri gramere harfi harfine gomulu: kume disi bir harf
    /// URETILEMEZ.
    #[test]
    fn enum_kumesi_disina_cikilamaz() {
        let k = kisit();
        let mut o = k.oturum();
        besle(&mut o, r#"belge_olustur({"bicim":""#);
        assert!(izinli(o.as_ref(), 'e'), "excel'in ilk harfi acik olmali");
        assert!(izinli(o.as_ref(), 'm'), "markdown'in ilk harfi acik olmali");
        assert!(!izinli(o.as_ref(), 'z'), "kume disi harf uretilebiliyor");
    }

    /// Duz cevap mesru bir ciktidir: baslangicta hicbir sey maskelenmez,
    /// yoksa "merhaba"ya bile arac cagirmaya zorlanirdi.
    #[test]
    fn duz_cevap_kisitlanmaz() {
        let k = kisit();
        let mut o = k.oturum();
        assert!(izinli(o.as_ref(), 'M'));
        besle(&mut o, "Merhaba, nasil yardimci olabilirim?");
        assert!(izinli(o.as_ref(), 'X'), "serbest metinde maske olmamali");
        assert!(!o.bitti_mi(), "serbest metin kisitca bitmis sayilmamali");
    }

    /// Katalogda olmayan bir ad cagriya DONUSMEZ: cikti duz metne duser ve
    /// karari `AracYurutucu`nun 1. kapisi verir (bkz. dosya basi).
    #[test]
    fn bilinmeyen_arac_adi_cagri_sayilmaz() {
        let k = kisit();
        let mut o = k.oturum();
        besle(&mut o, r#"hayali_arac({"x":"#);
        assert!(izinli(o.as_ref(), 'q'), "bilinmeyen ad serbest metin olmali");
        assert!(!o.bitti_mi());
    }

    /// Arac adinin oneki tutmayi birakinca kisit serbest metne duser ve bir
    /// daha geri donmez — yarim eslesme bir cagriyi zorlamamali.
    #[test]
    fn onek_bozulunca_serbest_kalir() {
        let k = kisit();
        let mut o = k.oturum();
        besle(&mut o, "belge_ol");
        besle(&mut o, "X");
        assert!(izinli(o.as_ref(), '('), "serbest metinde parantez de serbest");
        besle(&mut o, r#"({"bicim":"zzz"})"#);
        assert!(!o.bitti_mi(), "serbest metin cagri gibi kapanmamali");
    }
}
