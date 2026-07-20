//! `dosya_ara` — kum havuzu icinde dosya adi ve icerik aramasi.
//!
//! Swift karsiligi `AramaAraci` (CoreSpotlight). Spotlight'in Rust'ta karsiligi
//! yok ve OLMAMALI: cihazin kendi indeksini sorgulamak bir platform yetenegidir,
//! bu crate ise kum havuzunun disini GORMEZ. Dolayisiyla burada indeks degil
//! dogrudan yurume var. Swift'ten devralinan asil ders arama algoritmasi degil,
//! ADLANDIRMA: Swift'te arac adi once `arama` idi ve kullanici "internette ara"
//! deyince model onu cagirip cihaz notlarina bakiyordu. Ad, aciklamadan daha
//! guclu bir sinyal oldugu icin `not_arama`ya cevrildi. Buradaki ad da bilerek
//! `dosya_ara`: NEREDE aradigini adin kendisi soyluyor, web ile karisamaz.
//!
//! SIFIR BAGIMLILIK: yurume ve esleme `std` ile yazildi. Bir `walkdir` ya da
//! `regex` cekmek, alt dizi aramasi ugruna denetlenmemis bir kod govdesini iceri
//! almak olurdu; ihtiyacimiz olan altkume (yineleyici yurume + kucuk harfe
//! katlanmis alt dizi) ~100 satir.
//!
//! BYPASS KANALI: cok sonuc modelden GECMEZ. Tam liste `VeriDeposu`na konur,
//! modele ilk birkac satir + `kaynak_ref` doner. 400 dosyalik bir eslesme
//! listesini baglama dokmek, 4096 penceresini tek arama ile bitirmek demek.

use std::fs;
use std::path::{Path, PathBuf};

use sirr_cekirdek::{
    Alan, Arac, AracBaglami, AracGelecegi, AracHatasi, AracSonuc, AracSonucu, ArgSema, IzGuncelleme,
    kutula,
};

/// Modele dogrudan gosterilen en fazla eslesme. Ustu bypass kanalina duser.
const MODELE_SATIR: usize = 12;

/// Varsayilan ve en fazla sonuc sayisi.
const VARSAYILAN_ADET: usize = 20;
const EN_COK_ADET: usize = 100;

/// Gezilecek en fazla giris. Kum havuzu buyukse arama sonsuza kadar surmemeli;
/// tavan asildiginda sonuc "yok" degil "kismi"dir ve bu modele SOYLENIR —
/// sessizce eksik liste donmek, modelin "boyle bir dosya yok" demesine yol acar.
const EN_COK_GIRIS: usize = 20_000;

/// Icerigi taranacak dosyanin ust siniri (1 MiB). Bunun ustu bir metin belgesi
/// degildir; okumak bellegi ve zamani bosa yer.
const ICERIK_TAVANI: u64 = 1024 * 1024;

/// Dizin ic ice gecme siniri. Yurume yinelemeli (yigin tabanli) oldugu icin
/// tasma riski yok; sinir baglanti donguleri ve patolojik agaclar icin.
const EN_COK_DERINLIK: usize = 24;

/// Icerigi taranan uzantilar — BEYAZ liste.
///
/// Kara liste olsaydi bilinmeyen her uzanti "herhalde metindir" diye okunurdu;
/// ikili bir dosyada bu, bozuk UTF-8'i eslestirmeye calismak demek. `belge_oku`
/// ile ayni kural, bilerek ayni sozluk.
const METIN_UZANTILARI: &[&str] = &[
    "txt", "md", "markdown", "text", "log", "csv", "tsv", "json", "yaml", "yml", "toml", "rs",
    "swift", "py", "js", "ts", "html", "css", "sh", "conf", "ini",
];

// ---------------------------------------------------------------------------
// Sonuc tipleri
// ---------------------------------------------------------------------------

/// Tek bir eslesme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Eslesme {
    /// Kum havuzu KOKUNE gore bagil yol. Mutlak yol asla disari verilmez:
    /// modele giden metinde kullanicinin ev dizini bir kisisel veridir.
    pub yol: String,
    /// Adda mi icerikte mi eslesti — model hangi sinyale guvenecegini bilsin.
    pub nerede: Nerede,
    /// Icerik eslesmesinde eslesen satirin kirpilmis hali.
    pub satir: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nerede {
    Ad,
    Icerik,
}

impl Nerede {
    pub fn etiket(&self) -> &'static str {
        match self {
            Nerede::Ad => "name",
            Nerede::Icerik => "content",
        }
    }
}

/// Bir aramanin tam sonucu.
#[derive(Debug, Clone, Default)]
pub struct AramaSonucu {
    pub eslesmeler: Vec<Eslesme>,
    /// Tavan asildi mi — asildiysa liste EKSIK ve bu sakli tutulmaz.
    pub kismi: bool,
    pub gezilen: usize,
}

impl AramaSonucu {
    /// Eslesmeleri modele/depoya giden satir listesine cevirir.
    pub fn satirlar(&self, en_fazla: usize) -> String {
        let mut s = String::new();
        for (i, e) in self.eslesmeler.iter().take(en_fazla).enumerate() {
            s.push_str(&format!("{}. {} [{}]", i + 1, e.yol, e.nerede.etiket()));
            if let Some(satir) = &e.satir {
                s.push_str(&format!(" — {satir}"));
            }
            s.push('\n');
        }
        let gizlenen = self.eslesmeler.len().saturating_sub(en_fazla);
        if gizlenen > 0 {
            s.push_str(&format!("(+{gizlenen} sonuc daha gosterilmedi)\n"));
        }
        s
    }
}

// ---------------------------------------------------------------------------
// Arama cekirdegi
// ---------------------------------------------------------------------------

/// Kum havuzunda arar. Arac disindan (test, eval) da cagrilabilsin diye serbest.
///
/// `kok` COZULMUS olmali: bu fonksiyon kum havuzu kapisini kendi kurmaz, cagiran
/// `AracBaglami::yolu_coz`dan gecirir. Iki yerde cozmek, iki farkli kapi demek.
pub fn ara(
    kok: &Path,
    desen: &str,
    icerik_ara: bool,
    en_cok: usize,
) -> AracSonuc<AramaSonucu> {
    let aranan = katla(desen);
    if aranan.is_empty() {
        return Err(AracHatasi::GecersizArguman("bos arama deseni".into()));
    }

    let mut sonuc = AramaSonucu::default();
    // Yinelemeli yurume: ozyineleme derin agacta yigin tasirir ve bu
    // yakalanabilir bir hata degil, dogrudan cokmedir.
    let mut yigin: Vec<(PathBuf, usize)> = vec![(kok.to_path_buf(), 0)];

    while let Some((dizin, derinlik)) = yigin.pop() {
        if derinlik > EN_COK_DERINLIK {
            continue;
        }
        // Okunamayan dizin ARAMAYI BITIRMEZ: izinsiz tek bir alt klasor
        // yuzunden butun sonucu kaybetmek, kullaniciya hicbir sey kazandirmaz.
        let Ok(girisler) = fs::read_dir(&dizin) else {
            continue;
        };

        // Siralama BELIRLENIMCILIK icin: `read_dir` dosya sistemi sirasini
        // verir, yani ayni kum havuzu iki calistirmada farkli liste uretebilir
        // ve eval karsilastirilamaz hale gelir.
        let mut adaylar: Vec<PathBuf> = girisler.flatten().map(|g| g.path()).collect();
        adaylar.sort();

        for yol in adaylar {
            if sonuc.gezilen >= EN_COK_GIRIS {
                sonuc.kismi = true;
                return Ok(sonuc);
            }
            sonuc.gezilen += 1;

            let ad = yol.file_name().map(|a| a.to_string_lossy().into_owned()).unwrap_or_default();
            // Gizli girisler atlanir: `.git` gibi agaclar aramanin butun
            // butcesini yer ve kullanicinin "dosyalarim" dedigi sey degildir.
            if ad.starts_with('.') {
                continue;
            }

            // `symlink_metadata`: baglanti IZLENMEZ. Izlenseydi kum havuzu
            // icindeki bir baglanti disariyi okuturdu — `yolu_coz`u bastan
            // gecersiz kilan tam da bu.
            let Ok(ust) = yol.symlink_metadata() else {
                continue;
            };
            if ust.file_type().is_symlink() {
                continue;
            }

            if ust.is_dir() {
                yigin.push((yol, derinlik + 1));
                continue;
            }
            if !ust.is_file() {
                continue;
            }

            let bagil = bagil_yol(kok, &yol);

            if katla(&ad).contains(&aranan) {
                sonuc.eslesmeler.push(Eslesme { yol: bagil, nerede: Nerede::Ad, satir: None });
            } else if icerik_ara
                && ust.len() <= ICERIK_TAVANI
                && metin_uzantisi_mi(&yol)
                && let Some(satir) = icerikte_ara(&yol, &aranan)
            {
                sonuc.eslesmeler.push(Eslesme {
                    yol: bagil,
                    nerede: Nerede::Icerik,
                    satir: Some(satir),
                });
            }

            if sonuc.eslesmeler.len() >= en_cok {
                // Istenen adet doldu: bu "kismi" DEGIL, kullanicinin kendi
                // tavani. Ikisini karistirmak modele yanlis bilgi verirdi.
                return Ok(sonuc);
            }
        }
    }
    Ok(sonuc)
}

/// Dosya icinde arar; ilk eslesen satirin kirpilmis halini doner.
fn icerikte_ara(yol: &Path, aranan: &str) -> Option<String> {
    let bayt = fs::read(yol).ok()?;
    // NUL bayti: metin belgelerde gecmez, ikili bicimlerde neredeyse hep gecer.
    if bayt.contains(&0) {
        return None;
    }
    let metin = String::from_utf8_lossy(&bayt);
    for satir in metin.lines() {
        if katla(satir).contains(aranan) {
            return Some(satir_kirp(satir));
        }
    }
    None
}

/// Cip ve model metni tek satir kalsin diye eslesen satiri kisaltir.
fn satir_kirp(satir: &str) -> String {
    const TAVAN: usize = 100;
    let temiz: String = satir.split_whitespace().collect::<Vec<_>>().join(" ");
    if temiz.chars().count() <= TAVAN {
        return temiz;
    }
    let bas: String = temiz.chars().take(TAVAN - 1).collect();
    format!("{bas}…")
}

/// Metnin karsilastirma bicimi: kucuk harf + Turkce karakter katlamasi.
///
/// `yonlendirici::sadelestir` ile AYNI kurali uygular ama bilerek KOPYALANMADI:
/// oradaki fonksiyon niyet tetikleyicileri icin var ve orasi degistiginde arama
/// davranisinin sessizce degismesi istenmez. Iki ayri amac, iki ayri tanim.
fn katla(metin: &str) -> String {
    let mut s = String::with_capacity(metin.len());
    for c in metin.chars() {
        let d = match c {
            'ı' | 'İ' | 'I' => 'i',
            'ş' | 'Ş' => 's',
            'ğ' | 'Ğ' => 'g',
            'ü' | 'Ü' => 'u',
            'ö' | 'Ö' => 'o',
            'ç' | 'Ç' => 'c',
            _ => c,
        };
        s.extend(d.to_lowercase());
    }
    s.trim().to_string()
}

fn metin_uzantisi_mi(yol: &Path) -> bool {
    yol.extension()
        .map(|u| u.to_string_lossy().to_ascii_lowercase())
        .is_some_and(|u| METIN_UZANTILARI.contains(&u.as_str()))
}

/// Koke gore bagil yol metni. Kok disindaysa (olmamali) dosya adina duser —
/// hicbir kosulda mutlak yol donmez.
fn bagil_yol(kok: &Path, yol: &Path) -> String {
    yol.strip_prefix(kok)
        .map(|y| y.to_string_lossy().into_owned())
        .unwrap_or_else(|_| {
            yol.file_name().map(|a| a.to_string_lossy().into_owned()).unwrap_or_default()
        })
}

// ---------------------------------------------------------------------------
// Arac
// ---------------------------------------------------------------------------

/// `dosya_ara` — kum havuzunda ad + icerik aramasi.
#[derive(Debug, Clone, Copy, Default)]
pub struct DosyaAraAraci;

impl DosyaAraAraci {
    pub fn yeni() -> Self {
        Self
    }
}

impl Arac for DosyaAraAraci {
    fn ad(&self) -> &str {
        "dosya_ara"
    }

    fn aciklama(&self) -> &str {
        // ADI VE ILK CUMLESI NEREDE ARADIGINI SOYLER (Swift dersi): model
        // "internette ara" isteginde bu araci cagirmasin. Aciklamadaki uyari
        // tek basina yetmiyor, ad tasiyor — ama ikisi birlikte daha iyi.
        "Searches the user's OWN files inside the working directory by file name and by \
         text content. Call this when the user asks to find a file or a note on this device \
         ('find the file about X', 'which file mentions Y', in any language). It cannot see \
         the internet and it cannot leave the working directory — never use it for weather, \
         news or general knowledge."
    }

    fn sema(&self) -> ArgSema {
        ArgSema::nesne(vec![
            Alan::yeni(
                "desen",
                ArgSema::metin()
                    .aciklama("Text to look for in file names and file contents, e.g. 'toplanti'."),
            )
            .zorunlu(),
            Alan::yeni(
                "klasor",
                ArgSema::metin().aciklama(
                    "Optional subfolder to search in, relative to the working directory. \
                     Leave empty to search everything.",
                ),
            ),
            Alan::yeni(
                "icerik_ara",
                ArgSema::bool()
                    .aciklama("Also search inside text files (default true). false = names only."),
            ),
            Alan::yeni(
                "en_cok",
                ArgSema::tamsayi()
                    .aralik(Some(1.0), Some(EN_COK_ADET as f64))
                    .aciklama("Maximum number of results (default 20)."),
            ),
        ])
        .aciklama("Find files by name or content in the working directory")
    }

    /// Kullanicinin kendi dosyalarina bakiyor ve aranan kelimenin kendisi de
    /// kisisel veridir: oturum kirlenir.
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
            let iz = ctx.cip_baslat("ara", "Dosyalar araniyor…");

            let (sonuc, kirlendi) = match self.yurut(&args, ctx) {
                Ok(s) => (s, true),
                Err(h) => (AracSonucu::basarisiz(&h), false),
            };

            ctx.cip_guncelle(
                iz,
                IzGuncelleme::durum(sonuc.durum.clone())
                    .metin(sonuc.cip_metni.clone())
                    .ham_girdi(args.to_string())
                    .ham_cikti(sonuc.ham_cikti.clone().unwrap_or_default()),
            );
            // Kirletme yalniz GERCEKTEN arama yapildiginda: basarisiz yol hicbir
            // dosyaya dokunmadi, oturumu kirletmesi kullaniciyi bedava onay
            // sorularina sokardi (belge_oku ile ayni kural).
            if kirlendi {
                ctx.kirlet();
            }
            sonuc
        })
    }
}

impl DosyaAraAraci {
    /// Senkron govde — `calistir` yalniz cip/kirletme kabugunu tutar, hata yolu
    /// tek noktada (`AracSonucu::basarisiz`) toplanir.
    fn yurut(
        &self,
        args: &serde_json::Value,
        ctx: &AracBaglami,
    ) -> AracSonuc<AracSonucu> {
        let desen = args
            .get("desen")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| AracHatasi::EksikAlan("desen".into()))?;

        // Klasor argumani da kum havuzu kapisindan gecer: model buraya
        // "../.." yazabilir ve yazacaktir.
        let bagil_klasor = args
            .get("klasor")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(".");
        let kok = ctx.yolu_coz(bagil_klasor)?;
        if !kok.is_dir() {
            return Err(AracHatasi::DosyaYok(kok));
        }

        let icerik_ara = args.get("icerik_ara").and_then(|v| v.as_bool()).unwrap_or(true);
        let en_cok = args
            .get("en_cok")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).clamp(1, EN_COK_ADET))
            .unwrap_or(VARSAYILAN_ADET);

        let arama = ara(&kok, desen, icerik_ara, en_cok)?;

        // Bos sonuc HATA DEGILDIR. Model durustce "bulamadim" desin; hataya
        // cevirmek modele sabit `tool_failed` metnini gonderirdi ve model
        // "arama calismadi" ile "sonuc yok" arasini ayirt edemezdi.
        if arama.eslesmeler.is_empty() {
            let kismi = if arama.kismi { " (partial scan: limit reached)" } else { "" };
            return Ok(AracSonucu::okundu(
                "Eslesen dosya yok",
                format!("no_results_found in the working directory{kismi}"),
            )
            .ham_cikti(format!("{} giris gezildi", arama.gezilen)));
        }

        let adet = arama.eslesmeler.len();
        let tam_liste = arama.satirlar(usize::MAX);
        let cip = format!("{adet} dosya bulundu");

        // Bypass kanali: liste modele sigmiyorsa depoya konur, modele ilk
        // birkac satir + kaynak_ref gider. Kirpma boylece VERI KAYBI degil,
        // pencere karari olur — kalan sonuclara bir sonraki arac referansla
        // eksiksiz ulasir.
        if adet > MODELE_SATIR {
            let kaynak = ctx.depola("dosya", &format!("{adet} dosya eslesmesi"), tam_liste.clone());
            let onizleme = arama.satirlar(MODELE_SATIR);
            return Ok(AracSonucu::ozetle(
                cip,
                format!("found {adet} files:\n{onizleme}"),
                kaynak.as_str(),
            )
            .ham_cikti(tam_liste));
        }

        let kismi = if arama.kismi { "\n(partial scan: limit reached)" } else { "" };
        Ok(
            AracSonucu::okundu(cip, format!("found {adet} files:\n{tam_liste}{kismi}"))
                .ham_cikti(tam_liste),
        )
    }
}

// ---------------------------------------------------------------------------
// Testler
// ---------------------------------------------------------------------------

#[cfg(test)]
mod testler {
    use super::*;
    use serde_json::json;
    use sirr_cekirdek::{AracDurumu, BellekVeriDeposu, KaynakRef, SessizRaporlayici};
    use std::sync::Arc;

    /// Cekirdekte tokio yok; bu crate de calistirici secmemeli (belge_olustur
    /// testlerindeki `beklet` ile ayni kalip).
    fn beklet<F: std::future::Future>(gelecek: F) -> F::Output {
        use std::pin::pin;
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        static VTABLE: RawWakerVTable = RawWakerVTable::new(
            |_| RawWaker::new(std::ptr::null(), &VTABLE),
            |_| {},
            |_| {},
            |_| {},
        );
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
        let mut cx = Context::from_waker(&waker);
        let mut gelecek = pin!(gelecek);
        loop {
            if let Poll::Ready(cikti) = gelecek.as_mut().poll(&mut cx) {
                return cikti;
            }
        }
    }

    fn gecici_dizin(etiket: &str) -> PathBuf {
        let yol = std::env::temp_dir().join(format!(
            "sirr-ara-{etiket}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&yol).expect("gecici dizin");
        yol
    }

    fn yaz(kok: &Path, bagil: &str, icerik: &str) {
        let yol = kok.join(bagil);
        if let Some(ust) = yol.parent() {
            fs::create_dir_all(ust).expect("klasor");
        }
        fs::write(yol, icerik).expect("yazma");
    }

    fn baglam(kok: &Path, depo: Arc<BellekVeriDeposu>) -> AracBaglami {
        AracBaglami::yeni(depo, kok, Arc::new(SessizRaporlayici))
    }

    /// 1) Ad eslesmesi ve icerik eslesmesi ayri ayri bulunur ve ETIKETLENIR.
    #[test]
    fn ad_ve_icerik_eslesmesi_ayri_isaretlenir() {
        let kok = gecici_dizin("ad-icerik");
        yaz(&kok, "toplanti-notlari.md", "bugun hava guzel");
        yaz(&kok, "gunluk.md", "aksam toplanti var");
        yaz(&kok, "alakasiz.md", "hicbir sey");

        let s = ara(&kok, "toplanti", true, 20).expect("arama");
        assert_eq!(s.eslesmeler.len(), 2, "{:?}", s.eslesmeler);
        let ad = s.eslesmeler.iter().find(|e| e.nerede == Nerede::Ad).expect("ad eslesmesi");
        assert_eq!(ad.yol, "toplanti-notlari.md");
        assert!(ad.satir.is_none(), "ad eslesmesinde satir olmamali");
        let ic = s.eslesmeler.iter().find(|e| e.nerede == Nerede::Icerik).expect("icerik");
        assert_eq!(ic.satir.as_deref(), Some("aksam toplanti var"));
        fs::remove_dir_all(&kok).ok();
    }

    /// 2) Turkce karakter ve buyuk/kucuk harf farki aramayi bozmaz.
    #[test]
    fn turkce_karakter_ve_buyuk_harf_katlanir() {
        let kok = gecici_dizin("katlama");
        yaz(&kok, "TOPLANTI.md", "x");
        yaz(&kok, "notlar.md", "Şubat ayında GÖRÜŞME");

        assert_eq!(ara(&kok, "toplanti", true, 20).unwrap().eslesmeler.len(), 1);
        assert_eq!(ara(&kok, "TOPLANTI", true, 20).unwrap().eslesmeler.len(), 1);
        // Kullanici "gorusme" yazar, dosyada "GÖRÜŞME" gecer: ikisi eslesmeli.
        assert_eq!(ara(&kok, "gorusme", true, 20).unwrap().eslesmeler.len(), 1);
        assert_eq!(ara(&kok, "şubat", true, 20).unwrap().eslesmeler.len(), 1);
        fs::remove_dir_all(&kok).ok();
    }

    /// 3) Kum havuzu: model "../.." yazsa bile disari CIKAMAZ.
    #[test]
    fn kum_havuzu_disina_cikilamaz() {
        let kok = gecici_dizin("kum");
        yaz(&kok, "ic.md", "veri");
        let depo = Arc::new(BellekVeriDeposu::yeni());
        let mut ctx = baglam(&kok, depo);

        for kacak in ["..", "../..", "/etc", "alt/../../disari"] {
            let sonuc = beklet(
                DosyaAraAraci::yeni().calistir(json!({"desen": "veri", "klasor": kacak}), &mut ctx),
            );
            assert!(
                matches!(sonuc.durum, AracDurumu::Basarisiz(_)),
                "kacak gecti: {kacak}"
            );
            assert_eq!(sonuc.modele_donen, sirr_cekirdek::HATA_MODEL_METNI);
        }
        // Kacak denemesi oturumu KIRLETMEMELI: hicbir veri okunmadi.
        assert!(!ctx.oturum_kirli());
        fs::remove_dir_all(&kok).ok();
    }

    /// 4) Ikili dosya ve buyuk dosya icerik taramasina girmez; sembolik
    ///    baglanti izlenmez (izlenseydi kum havuzu delinirdi).
    #[test]
    fn ikili_dosya_ve_sembolik_baglanti_atlanir() {
        let kok = gecici_dizin("ikili");
        fs::write(kok.join("veri.json"), b"gizli\0\x01gizli").expect("ikili yazma");
        yaz(&kok, "duz.md", "gizli kelime");

        let s = ara(&kok, "gizli", true, 20).expect("arama");
        // Ikili dosyanin ICERIGI eslesmemeli — yalniz duz metin bulunmali.
        assert_eq!(s.eslesmeler.len(), 1, "{:?}", s.eslesmeler);
        assert_eq!(s.eslesmeler[0].yol, "duz.md");

        // Baglanti izlenmiyor: disaridaki dosya adiyla eslesse bile girilmez.
        #[cfg(unix)]
        {
            let disari = gecici_dizin("baglanti-hedefi");
            yaz(&disari, "gizli-disarida.md", "gizli");
            std::os::unix::fs::symlink(&disari, kok.join("kapi")).expect("symlink");
            let s2 = ara(&kok, "gizli", true, 20).expect("arama");
            assert_eq!(s2.eslesmeler.len(), 1, "baglanti izlenmis: {:?}", s2.eslesmeler);
            fs::remove_dir_all(&disari).ok();
        }
        fs::remove_dir_all(&kok).ok();
    }

    /// 5) Cok sonuc bypass kanalindan gecer: model listenin TAMAMINI gormez.
    #[test]
    fn cok_sonuc_bypass_kanalindan_gecer() {
        let kok = gecici_dizin("bypass");
        for i in 0..40 {
            yaz(&kok, &format!("rapor-{i:03}.md", ), "x");
        }
        let depo = Arc::new(BellekVeriDeposu::yeni());
        let mut ctx = baglam(&kok, depo);
        let sonuc = beklet(
            DosyaAraAraci::yeni().calistir(json!({"desen": "rapor", "en_cok": 40}), &mut ctx),
        );

        assert_eq!(sonuc.durum, AracDurumu::Okundu);
        assert!(sonuc.modele_donen.contains("kaynak_ref"), "{}", sonuc.modele_donen);
        // 40 satirlik liste modele DOKULMEDI: son dosya adi modelde gecmemeli.
        assert!(!sonuc.modele_donen.contains("rapor-039"), "{}", sonuc.modele_donen);
        // Ama veri kaybolmadi: depodaki govde tam listeyi tasiyor.
        let kayit = ctx.depodan(&KaynakRef("dosya#1".into())).expect("depo kaydi");
        assert!(kayit.govde.contains("rapor-039.md"));
        assert!(ctx.oturum_kirli(), "kullanici dosyalari okundu");
        fs::remove_dir_all(&kok).ok();
    }

    /// 6) Bos sonuc HATA DEGIL: model "bulamadim" diyebilmeli.
    #[test]
    fn bos_sonuc_hata_degildir() {
        let kok = gecici_dizin("bos");
        yaz(&kok, "a.md", "alfa");
        let depo = Arc::new(BellekVeriDeposu::yeni());
        let mut ctx = baglam(&kok, depo);
        let sonuc =
            beklet(DosyaAraAraci::yeni().calistir(json!({"desen": "zeta"}), &mut ctx));

        assert_eq!(sonuc.durum, AracDurumu::Okundu, "bos sonuc basarisiz olmamali");
        assert!(sonuc.modele_donen.starts_with("no_results_found"));
        assert_ne!(sonuc.modele_donen, sirr_cekirdek::HATA_MODEL_METNI);
        fs::remove_dir_all(&kok).ok();
    }

    /// 7) Sema sozlesmesi: model uydurma alan gonderemez, aralik disi adet
    ///    gecemez.
    #[test]
    fn sema_sozlesmesi_baglayicidir() {
        let js = DosyaAraAraci::yeni().sema().json_schema();
        assert_eq!(js["type"], "object");
        assert_eq!(js["required"], json!(["desen"]));
        assert_eq!(js["additionalProperties"], json!(false));
        assert_eq!(js["properties"]["en_cok"]["type"], "integer");

        let sema = DosyaAraAraci::yeni().sema();
        assert!(sema.dogrula(&json!({"desen": "a"})).is_ok());
        assert!(sema.dogrula(&json!({})).is_err(), "zorunlu alan");
        assert!(sema.dogrula(&json!({"desen": 5})).is_err(), "tip");
        assert!(sema.dogrula(&json!({"desen": "a", "en_cok": 9999})).is_err(), "aralik");
        assert!(sema.dogrula(&json!({"desen": "a", "icerik_ara": "evet"})).is_err(), "bool");
    }

    /// 8) `icerik_ara: false` yalniz ad eslesmesi birakir; `en_cok` tavani
    ///    gercekten baglayicidir.
    #[test]
    fn icerik_kapali_ve_adet_tavani_uygulanir() {
        let kok = gecici_dizin("tavan");
        yaz(&kok, "bir.md", "anahtar");
        yaz(&kok, "iki.md", "anahtar");
        yaz(&kok, "anahtar-dosyasi.md", "bos");

        let sadece_ad = ara(&kok, "anahtar", false, 20).expect("arama");
        assert_eq!(sadece_ad.eslesmeler.len(), 1);
        assert_eq!(sadece_ad.eslesmeler[0].nerede, Nerede::Ad);

        let tavanli = ara(&kok, "anahtar", true, 2).expect("arama");
        assert_eq!(tavanli.eslesmeler.len(), 2, "tavan uygulanmali");
        fs::remove_dir_all(&kok).ok();
    }

    /// 9) Modele giden yol BAGIL: mutlak yol kullanicinin ev dizinini sizdirir.
    #[test]
    fn modele_bagil_yol_doner() {
        let kok = gecici_dizin("bagil");
        yaz(&kok, "alt/klasor/hedef.md", "icerik");
        let depo = Arc::new(BellekVeriDeposu::yeni());
        let mut ctx = baglam(&kok, depo);
        let sonuc =
            beklet(DosyaAraAraci::yeni().calistir(json!({"desen": "hedef"}), &mut ctx));

        assert!(sonuc.modele_donen.contains("alt/klasor/hedef.md"), "{}", sonuc.modele_donen);
        assert!(
            !sonuc.modele_donen.contains(&kok.display().to_string()),
            "mutlak yol sizdi: {}",
            sonuc.modele_donen
        );
        fs::remove_dir_all(&kok).ok();
    }

    /// 10) Yonlendirici bu araci "dosya ara" niyetinde secebilmeli — ad ve
    ///     aciklama Genel profilinin ipuclarini tasiyor mu.
    #[test]
    fn yonlendirici_ipuclari_tutar() {
        use crate::yonlendirici::{Yonlendirici, sadelestir};
        let metin = sadelestir(&format!(
            "{} {}",
            DosyaAraAraci::yeni().ad(),
            DosyaAraAraci::yeni().aciklama()
        ));
        // Genel profilinin ipuclari: "dosya", "ara".
        assert!(metin.contains("dosya"));
        assert!(metin.contains("ara"));

        let mut katalog = sirr_cekirdek::AracKatalogu::yeni();
        katalog.ekle(Arc::new(crate::hesap::HesapAraci));
        katalog.ekle(Arc::new(DosyaAraAraci::yeni()));
        let secilen = Yonlendirici::yeni().en_fazla(1).sec("su dosyayi bul", &katalog);
        assert_eq!(secilen[0].ad(), "dosya_ara", "yonlendirici yanlis araci sectI");
    }
}
