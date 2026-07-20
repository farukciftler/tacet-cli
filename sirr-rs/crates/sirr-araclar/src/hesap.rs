//! `HesapAraci` — ifade hesaplayici; ayni zamanda REFERANS ARAC.
//!
//! Bu dosya bilincli olarak en sade `Arac` uygulamasidir: yeni bir arac yazan
//! once buraya bakar. Gosterdigi kalip su — semayi dogrula, cipi baslat, isi
//! yap, tek cikis noktasindan (`AracSonucu`) don. Hicbir yolda `panic!` ya da
//! `unwrap` yok; bir arac cokerse butun turu goturur.
//!
//! NEDEN KENDI AYRISTIRICIMIZ: sifir-bagimlilik kimligi. Hazir bir ifade
//! motoru cekmek, dort islem ugruna denetlenmemis bir kod govdesini ve onun
//! panik davranisini iceri almak demekti. Ozyinelemeli inis burada ~150 satir
//! ve tamamen bizim denetimimizde.
//!
//! NEDEN BYPASS KANALI KULLANILMIYOR: `VeriDeposu` toplu veriyi modelden uzak
//! tutmak icin var. Buradaki cikti tek bir sayidir; depoya koymak modele bir
//! de referans cozdurmek demek olurdu — kanal, faydasi olan yerde kullanilir.

use sirr_cekirdek::{
    Alan, Arac, AracBaglami, AracDurumu, AracGelecegi, AracHatasi, AracSonuc, AracSonucu, ArgSema,
    IzGuncelleme, kutula,
};

/// Girdi siniri. Modelin uzun bir metni "ifade" diye gondermesi bir hatadir;
/// ayristiriciyi mesgul etmeden, sinirda kesmek daha dogru bir yanit.
const EN_COK_UZUNLUK: usize = 512;

/// Parantez ic ice gecme siniri. Ozyinelemeli inis, girdi derinligi kadar yigin
/// tuketir: sinir olmadan `"((((..."` seklinde bir girdi yigini tasirir ve bu
/// yakalanabilir bir hata degil, dogrudan cokme olur.
const EN_COK_DERINLIK: usize = 32;

const VARSAYILAN_BASAMAK: usize = 6;

/// Cipteki ifade metninin siniri: cip ~bir satir, uzun ifade tasarimi bozar.
const CIP_IFADE_SINIRI: usize = 48;

// ---------------------------------------------------------------------------
// Arac
// ---------------------------------------------------------------------------

/// Aritmetik ifade hesaplayici.
pub struct HesapAraci;

impl Arac for HesapAraci {
    fn ad(&self) -> &str {
        "hesapla"
    }

    fn aciklama(&self) -> &str {
        "Sayisal bir ifadeyi hesaplar: dort islem, parantez, yuzde (%) ve us (^). \
         Aritmetik iceren her istekte bunu cagir, sonucu kendin hesaplama."
    }

    fn sema(&self) -> ArgSema {
        // SEMA = MODELIN SINIRI: gramer bunu birebir kisita cevirir, yani burada
        // eksik birakilan her ayrinti modelin uydurma alani olur. Ornekler
        // aciklamaya bilerek konuyor; model bicimi ornekten ogreniyor.
        ArgSema::nesne(vec![
            Alan::yeni(
                "ifade",
                ArgSema::metin().aciklama(
                    "Hesaplanacak ifade. Ornek: (12 + 3) * 4  |  250 + 18%  |  2^10",
                ),
            )
            .zorunlu(),
            Alan::yeni(
                "basamak",
                ArgSema::tamsayi()
                    .aralik(Some(0.0), Some(10.0))
                    .aciklama("Ondalik basamak sayisi (varsayilan 6)."),
            ),
        ])
    }

    // Saf aritmetik; hicbir kisisel veri okunmaz, onay kapisina takilmaz.
    fn kirletir_mi(&self) -> bool {
        false
    }

    fn calistir<'a>(
        &'a self,
        args: serde_json::Value,
        ctx: &'a mut AracBaglami,
    ) -> AracGelecegi<'a> {
        kutula(async move {
            // Gramer devre disi olabilir ya da cagri eval/CLI'dan gelebilir;
            // sema dogrulamasi bu yuzden aracin kendi kapisinda da duruyor.
            if let Err(hata) = self.sema().dogrula(&args) {
                return AracSonucu::basarisiz(&hata);
            }
            let Some(ifade) = args.get("ifade").and_then(|d| d.as_str()) else {
                return AracSonucu::basarisiz(&AracHatasi::EksikAlan("arg.ifade".into()));
            };
            let basamak = args
                .get("basamak")
                .and_then(|d| d.as_u64())
                .map(|n| n as usize)
                .unwrap_or(VARSAYILAN_BASAMAK);

            let iz = ctx.cip_baslat("=", "Hesaplaniyor");

            match hesapla(ifade) {
                Ok(deger) => {
                    let sonuc = sayi_metni(deger, basamak);
                    let cip = format!("{} = {}", cip_icin_kirp(ifade), sonuc);
                    ctx.cip_guncelle(
                        iz,
                        IzGuncelleme::durum(AracDurumu::Okundu)
                            .metin(cip.clone())
                            .ham_girdi(ifade.trim())
                            .ham_cikti(sonuc.clone()),
                    );
                    // Modele giden metin cip metninden AYRI ve daha kisa:
                    // model sonucu kendi cumlesine yerlestirecek, ifadeyi
                    // tekrar okumasina gerek yok.
                    AracSonucu::okundu(cip, sonuc.clone()).ham_cikti(sonuc)
                }
                Err(hata) => {
                    // Tek hata gecis noktasi: cipe Turkce cumle, modele sabit metin.
                    let sonuc = AracSonucu::basarisiz(&hata);
                    ctx.cip_guncelle(
                        iz,
                        IzGuncelleme::durum(sonuc.durum.clone()).metin(sonuc.cip_metni.clone()),
                    );
                    sonuc
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Ayristirici
// ---------------------------------------------------------------------------

/// Bir ifadeyi hesaplar. Arac disindan da (eval, CLI, testler) dogrudan
/// cagrilabilsin diye ayri ve `pub`.
pub fn hesapla(ifade: &str) -> AracSonuc<f64> {
    if ifade.trim().is_empty() {
        return Err(AracHatasi::GecersizArguman("bos ifade".into()));
    }
    if ifade.chars().count() > EN_COK_UZUNLUK {
        return Err(AracHatasi::GecersizArguman("ifade cok uzun".into()));
    }

    // char dilimi: girdi Turkce/unicode olabilir ve konumla geri gidiyoruz;
    // bayt indisiyle calismak cok baytli karakterin ortasina dusme riski tasir.
    let karakterler: Vec<char> = ifade.chars().collect();
    let mut a = Ayristirici { girdi: &karakterler, konum: 0, derinlik: 0 };

    let deger = a.toplam()?;
    a.bosluk_atla();
    if a.konum < a.girdi.len() {
        return Err(AracHatasi::GecersizArguman(format!(
            "beklenmeyen karakter: '{}'",
            a.girdi[a.konum]
        )));
    }
    sonlu(deger)
}

struct Ayristirici<'a> {
    girdi: &'a [char],
    konum: usize,
    derinlik: usize,
}

impl Ayristirici<'_> {
    fn bosluk_atla(&mut self) {
        while matches!(self.girdi.get(self.konum), Some(c) if c.is_whitespace()) {
            self.konum += 1;
        }
    }

    /// Bosluklari atlayip siradaki karaktere bakar (tuketmez).
    fn bak(&mut self) -> Option<char> {
        self.bosluk_atla();
        self.girdi.get(self.konum).copied()
    }

    fn yut(&mut self, beklenen: char) -> bool {
        if self.bak() == Some(beklenen) {
            self.konum += 1;
            true
        } else {
            false
        }
    }

    /// toplam := carpim (('+' | '-') carpim)*
    fn toplam(&mut self) -> AracSonuc<f64> {
        let (mut sol, _) = self.carpim()?;
        loop {
            let islec = match self.bak() {
                Some(c @ ('+' | '-')) => c,
                _ => return Ok(sol),
            };
            self.konum += 1;
            let (sag, yuzde_mi) = self.carpim()?;
            // HESAP MAKINESI SEZGISI: "250 + 18%" kullanicinin kastettigi sey
            // 250 + 250*0.18'dir, 250.18 degil. Yuzde bayragi bu yuzden sag
            // terimden yukari tasiniyor. Ayni ifadeyi mutlak isteyen
            // parantezle yazar: "250 + (18%)" -> 250.18.
            let uygulanan = if yuzde_mi { sol * sag } else { sag };
            sol = sonlu(if islec == '+' { sol + uygulanan } else { sol - uygulanan })?;
        }
    }

    /// carpim := tekil (('*' | '/') tekil)*
    ///
    /// Yuzde bayragini yalniz TEK terimli carpimda yukari gecirir: "10%" yuzde
    /// terimidir, "10% * 2" artik duz bir sayidir (0.2).
    fn carpim(&mut self) -> AracSonuc<(f64, bool)> {
        let (mut sol, mut yuzde_mi) = self.tekil()?;
        loop {
            let islec = match self.bak() {
                // Kullanici da model de carpmayi 'x' ile yazabiliyor; tek
                // isleci kabul etmek gereksiz basarisizlik uretirdi.
                Some(c @ ('*' | 'x' | 'X' | '×' | '/' | '÷')) => c,
                _ => return Ok((sol, yuzde_mi)),
            };
            self.konum += 1;
            let (sag, _) = self.tekil()?;
            yuzde_mi = false;
            sol = if matches!(islec, '/' | '÷') {
                if sag == 0.0 {
                    return Err(AracHatasi::Diger("Sifira bolme yapilamaz.".into()));
                }
                sonlu(sol / sag)?
            } else {
                sonlu(sol * sag)?
            };
        }
    }

    /// tekil := ('+' | '-') tekil | atom ('^' tekil)? '%'*
    fn tekil(&mut self) -> AracSonuc<(f64, bool)> {
        match self.bak() {
            Some('-') => {
                self.konum += 1;
                let (deger, yuzde_mi) = self.tekil()?;
                return Ok((-deger, yuzde_mi));
            }
            Some('+') => {
                self.konum += 1;
                return self.tekil();
            }
            _ => {}
        }

        let taban = self.atom()?;
        // Us SAGA baglanir (2^3^2 = 2^9) ve `tekil`e inerek "2^-3" gibi
        // negatif ustlere de izin verir.
        let mut deger = if self.yut('^') {
            let (ust, _) = self.tekil()?;
            us_al(taban, ust)?
        } else {
            taban
        };

        let mut yuzde_mi = false;
        while self.bak() == Some('%') {
            self.konum += 1;
            deger /= 100.0;
            yuzde_mi = true;
        }
        Ok((deger, yuzde_mi))
    }

    /// atom := sayi | '(' toplam ')'
    fn atom(&mut self) -> AracSonuc<f64> {
        match self.bak() {
            Some('(') => {
                self.konum += 1;
                self.derinlik += 1;
                if self.derinlik > EN_COK_DERINLIK {
                    return Err(AracHatasi::GecersizArguman("ifade cok ic ice".into()));
                }
                let deger = self.toplam()?;
                if !self.yut(')') {
                    return Err(AracHatasi::GecersizArguman("kapanmayan parantez".into()));
                }
                self.derinlik -= 1;
                Ok(deger)
            }
            Some(c) if c.is_ascii_digit() || c == '.' || c == ',' => self.sayi(),
            Some(c) => Err(AracHatasi::GecersizArguman(format!(
                "beklenmeyen karakter: '{c}'"
            ))),
            None => Err(AracHatasi::GecersizArguman("ifade eksik bitti".into())),
        }
    }

    fn sayi(&mut self) -> AracSonuc<f64> {
        let mut metin = String::new();
        let mut ondalik_gorundu = false;
        while let Some(c) = self.girdi.get(self.konum).copied() {
            if c.is_ascii_digit() {
                metin.push(c);
            } else if (c == '.' || c == ',') && !ondalik_gorundu {
                // Turkce yazimda ondalik ayirici virguldur; ikisini de kabul
                // etmek modelin hangi bicimi urettiginden bagimsiz kilar.
                ondalik_gorundu = true;
                metin.push('.');
            } else if c == '_' {
                // Basamak gruplama; degeri etkilemez.
            } else {
                break;
            }
            self.konum += 1;
        }
        metin
            .parse::<f64>()
            .map_err(|_| AracHatasi::GecersizArguman(format!("sayi okunamadi: '{metin}'")))
    }
}

/// Us alma. `powf` sessizce `inf`/`NaN` uretir (0^-1, (-8)^0.5); bu degerler
/// ilerideki her islemi kirletecegi icin tam burada hataya cevriliyor.
fn us_al(taban: f64, ust: f64) -> AracSonuc<f64> {
    sonlu(taban.powf(ust))
}

/// Tasma/tanimsizlik kapisi. Rust'ta f64 tasmasi panik degil `inf`tir; sessizce
/// gecerse kullaniciya "inf" diye bir sonuc gosteririz.
fn sonlu(deger: f64) -> AracSonuc<f64> {
    if deger.is_finite() {
        Ok(deger)
    } else if deger.is_nan() {
        Err(AracHatasi::Diger("Bu islem tanimli degil.".into()))
    } else {
        Err(AracHatasi::Diger("Sonuc sayi araligina sigmiyor.".into()))
    }
}

/// Sayiyi insan icin yazar: tam sayilar ondaliksiz, kesirliler gereksiz sifirlar
/// kirpilmis halde. `1e15` esigi f64'un tamsayi kesinligini kaybettigi yer.
fn sayi_metni(deger: f64, basamak: usize) -> String {
    if deger == deger.trunc() && deger.abs() < 1e15 {
        return format!("{}", deger as i64);
    }
    let mut s = format!("{deger:.basamak$}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    s
}

/// Cip tek satir kalsin diye ifadeyi kisaltir.
fn cip_icin_kirp(ifade: &str) -> String {
    let temiz: String = ifade.split_whitespace().collect::<Vec<_>>().join(" ");
    if temiz.chars().count() <= CIP_IFADE_SINIRI {
        return temiz;
    }
    let bas: String = temiz.chars().take(CIP_IFADE_SINIRI - 1).collect();
    format!("{bas}…")
}

// ---------------------------------------------------------------------------
// Testler
// ---------------------------------------------------------------------------

#[cfg(test)]
mod testler {
    use super::*;
    use serde_json::json;
    use sirr_cekirdek::{AracIzi, BellekVeriDeposu, IzToplayici, Raporlayici};
    use std::sync::Arc;

    fn yaklasik(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn dort_islem_ve_oncelik() {
        assert!(yaklasik(hesapla("2 + 3 * 4").unwrap(), 14.0));
        assert!(yaklasik(hesapla("12 x 34").unwrap(), 408.0));
        assert!(yaklasik(hesapla("10 - 2 - 3").unwrap(), 5.0), "sola baglanmali");
        assert!(yaklasik(hesapla("100 / 4 / 5").unwrap(), 5.0));
        assert!(yaklasik(hesapla("-3 + 5").unwrap(), 2.0));
        assert!(yaklasik(hesapla("3,5 * 2").unwrap(), 7.0), "virgullu ondalik");
    }

    #[test]
    fn parantez_oncelikleri_degistirir() {
        assert!(yaklasik(hesapla("(2 + 3) * 4").unwrap(), 20.0));
        assert!(yaklasik(hesapla("((1+2)*(3+4))").unwrap(), 21.0));
        assert!(hesapla("(1 + 2").is_err(), "kapanmayan parantez");
        // Derinlik kapisi: yigin tasmasi yerine duzgun hata.
        let derin = format!("{}1{}", "(".repeat(200), ")".repeat(200));
        assert!(hesapla(&derin).is_err());
    }

    #[test]
    fn us_saga_baglanir_ve_negatif_ust_alir() {
        assert!(yaklasik(hesapla("2^10").unwrap(), 1024.0));
        assert!(yaklasik(hesapla("2^3^2").unwrap(), 512.0), "saga baglanmali");
        assert!(yaklasik(hesapla("2^-2").unwrap(), 0.25));
        // -2^2 = -(2^2): unary eksi ustten sonra uygulanir.
        assert!(yaklasik(hesapla("-2^2").unwrap(), -4.0));
    }

    #[test]
    fn yuzde_hesap_makinesi_sezgisiyle_calisir() {
        assert!(yaklasik(hesapla("50%").unwrap(), 0.5));
        assert!(yaklasik(hesapla("250 + 18%").unwrap(), 295.0));
        assert!(yaklasik(hesapla("200 - 10%").unwrap(), 180.0));
        // Parantez yuzde baglamini kapatir: kacis kapisi.
        assert!(yaklasik(hesapla("250 + (18%)").unwrap(), 250.18));
        // Carpimda yuzde bayragi dusmeli.
        assert!(yaklasik(hesapla("100 + 10% * 2").unwrap(), 100.2));
    }

    #[test]
    fn tasma_ve_sifira_bolme_panik_yerine_hata_doner() {
        assert!(hesapla("1 / 0").is_err());
        assert!(hesapla("5 / (3 - 3)").is_err());
        let h = hesapla("9^9^9").expect_err("tasmali");
        assert!(h.kisa_hata().contains("sigmiyor"), "{}", h.kisa_hata());
        assert!(hesapla("0^-1").is_err());
        assert!(hesapla("(-8)^0,5").is_err(), "NaN tanimsiz sayilmali");
    }

    #[test]
    fn bozuk_girdi_reddedilir() {
        assert!(hesapla("").is_err());
        assert!(hesapla("   ").is_err());
        assert!(hesapla("2 +").is_err());
        assert!(hesapla("rm -rf /").is_err());
        assert!(hesapla("1.2.3").is_err());
        assert!(hesapla(&"1+".repeat(400)).is_err(), "uzunluk siniri");
    }

    #[test]
    fn sayi_bicimi_okunur_kalir() {
        assert_eq!(sayi_metni(408.0, 6), "408");
        assert_eq!(sayi_metni(0.5, 6), "0.5");
        assert_eq!(sayi_metni(1.0 / 3.0, 4), "0.3333");
        assert_eq!(sayi_metni(2.0 / 3.0, 0), "1");
        assert_eq!(sayi_metni(-7.25, 6), "-7.25");
    }

    // --- Arac sozlesmesi ---

    fn baglam(rapor: Arc<dyn Raporlayici>) -> AracBaglami {
        AracBaglami::yeni(Arc::new(BellekVeriDeposu::yeni()), "/tmp/sirr-hesap", rapor)
    }

    #[test]
    fn arac_basarili_yolda_cip_ve_kisa_model_metni_uretir() {
        let toplayici = Arc::new(IzToplayici::yeni());
        let mut ctx = baglam(toplayici.clone());
        let sonuc = yurut(HesapAraci.calistir(json!({"ifade": "12 x 34"}), &mut ctx));

        assert_eq!(sonuc.cip_metni, "12 x 34 = 408");
        assert_eq!(sonuc.modele_donen, "408", "modele yalniz sonuc gitmeli");
        assert_eq!(sonuc.durum, AracDurumu::Okundu);
        // Salt-okuma araci dunyayi degistirmemeli.
        assert!(!sonuc.durum.dunyayi_degistirdi());
        assert!(!ctx.oturum_kirli(), "hesap kisisel veri okumaz");

        let izler: Vec<AracIzi> = toplayici.izler();
        assert_eq!(izler.len(), 1);
        assert_eq!(izler[0].metin, "12 x 34 = 408");
    }

    #[test]
    fn arac_hatayi_cipe_turkce_modele_sabit_metin_olarak_verir() {
        let mut ctx = baglam(Arc::new(IzToplayici::yeni()));
        let sonuc = yurut(HesapAraci.calistir(json!({"ifade": "1 / 0"}), &mut ctx));

        assert_eq!(sonuc.modele_donen, sirr_cekirdek::HATA_MODEL_METNI);
        assert_eq!(sonuc.cip_metni, "Sifira bolme yapilamaz.");
        assert!(matches!(sonuc.durum, AracDurumu::Basarisiz(_)));
    }

    #[test]
    fn arac_semaya_uymayan_argumani_reddeder() {
        let mut ctx = baglam(Arc::new(IzToplayici::yeni()));
        // Zorunlu alan yok.
        let a = yurut(HesapAraci.calistir(json!({}), &mut ctx));
        assert!(matches!(a.durum, AracDurumu::Basarisiz(_)));
        // Yanlis tip.
        let b = yurut(HesapAraci.calistir(json!({"ifade": 12}), &mut ctx));
        assert!(matches!(b.durum, AracDurumu::Basarisiz(_)));
        // Aralik disi basamak.
        let c = yurut(HesapAraci.calistir(json!({"ifade": "1+1", "basamak": 99}), &mut ctx));
        assert!(matches!(c.durum, AracDurumu::Basarisiz(_)));
    }

    #[test]
    fn basamak_argumani_ciktiyi_yuvarlar() {
        let mut ctx = baglam(Arc::new(IzToplayici::yeni()));
        let sonuc = yurut(HesapAraci.calistir(json!({"ifade": "1/3", "basamak": 3}), &mut ctx));
        assert_eq!(sonuc.modele_donen, "0.333");
    }

    #[test]
    fn sema_json_schema_beklenen_sozlesmeyi_verir() {
        let js = HesapAraci.sema().json_schema();
        assert_eq!(js["type"], "object");
        assert_eq!(js["required"], json!(["ifade"]));
        // Uydurma anahtar kacak yol olmasin.
        assert_eq!(js["additionalProperties"], json!(false));
        assert_eq!(js["properties"]["basamak"]["type"], "integer");
    }

    #[test]
    fn uzun_ifade_cipte_kirpilir() {
        let uzun = "1 + ".repeat(40) + "1";
        let mut ctx = baglam(Arc::new(IzToplayici::yeni()));
        let sonuc = yurut(HesapAraci.calistir(json!({"ifade": uzun}), &mut ctx));
        assert_eq!(sonuc.modele_donen, "41");
        assert!(sonuc.cip_metni.chars().count() < 70, "{}", sonuc.cip_metni);
        assert!(sonuc.cip_metni.contains('…'));
    }

    /// Cekirdekte tokio yok; test icin yeten minimal yurutucu (cekirdek
    /// testlerindeki `futures_yok` ile ayni kalip).
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
