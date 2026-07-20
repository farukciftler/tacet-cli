//! `kod_calistir` — model ciktisini calistiran arac. BU DOSYA GUVENLIK
//! SINIRIDIR; buradaki her mekanizma bir saldiri ya da bir kaza icindir.
//!
//! Swift karsiligi `KodMotoru` + `KodCalistirAraci`. Orada motor
//! JavaScriptCore'du: isletim sisteminin icinde, dosya ve ag koprusu OLMAYAN
//! bir yorumlayici. Rust'ta bunun karsiligi YOK ve sifir-bagimlilik kimligi
//! altinda bir JS motoru cekmek soz konusu degil — o yuzden burada model kodu
//! AYRI BIR SURECTE, sistemde zaten kurulu bir yorumlayiciyla kosar. Bu, tehdit
//! modelini degistirir: JSC'de ag "koprulenmedigi icin" yoktu, burada ag
//! "acikca kesildigi icin" yok olmalidir. Kesemedigimiz yerde arac KENDINI
//! KATALOGA KOYMAZ (bkz. `kesfet`).
//!
//! DORT KAPI:
//!
//! 1. **AG** — macOS'ta `sandbox-exec` ile `(deny network*)`. Kesme KESFEDERKEN
//!    GERCEKTEN OLCULUR (`kalkan_dogrula`): profil dosyasi yazip "herhalde
//!    calisiyordur" demek, sessizce guvenli demenin ta kendisi olurdu. Kalkan
//!    kurulamazsa `kesfet` `None` doner ve arac katalogda GORUNMEZ.
//! 2. **DOSYA** — ayni profil yazmayi kum havuzuna kilitler ve kullanicinin ev
//!    dizinini OKUMAYA da kapatir. Yorumlayicinin kendi kitapligi (dyld,
//!    stdlib) okunabilir kalir; kapatilsaydi python3 hic baslamazdi.
//! 3. **ZAMAN** — zorunlu zaman asimi (varsayilan 10 sn). Asilirsa surec
//!    OLDURULUR (`kill` + `wait`), zombi birakilmaz.
//! 4. **CIKTI** — bayt tavani. Sonsuz dongu tum bellegi yemesin diye borular
//!    ayri is parcaciklarinda OKUNMAYA DEVAM EDER ama tavan ustu ATILIR:
//!    okumayi birakmak cocugu boruda kilitler, tavan koymamak bellegi yer.
//!
//! DENEME SAYACI (kod-spec §5.4): tur basina en fazla 2 gercek calistirma;
//! 3. cagri motoru HIC gormeden reddedilir. Swift'te sayac `weak` bir referanstaydi
//! ve baglanmadiginda "fail-closed" ile 3 sayilirdi. Burada sayac ARACIN
//! ICINDEDIR ve varsayilan olarak daima baglidir — unutulacak bir baglama yok.
//!
//! CIKTISIZ BASARI BIR BASARI DEGILDIR (Swift'te olculdu): "ok" goren model
//! sonucu UYDURUYORDU. Bastirmayan betik hata sayilir.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use sirr_cekirdek::{
    Alan, Arac, AracBaglami, AracDurumu, AracGelecegi, AracHatasi, AracSonuc, AracSonucu, ArgSema,
    IzGuncelleme, kutula,
};

/// Varsayilan zaman asimi (saniye). Gorevin verdigi taban.
pub const VARSAYILAN_ZAMAN_ASIMI: u64 = 10;
/// Modelin isteyebilecegi en uzun sure. Tavani model SECEMEZ, yalniz kisaltabilir.
pub const EN_COK_ZAMAN_ASIMI: u64 = 30;

/// Yakalanan ciktinin bayt tavani. Ustu atilir ve atildigi SOYLENIR.
const CIKTI_TAVANI: usize = 10_000;
/// Modele giden ciktinin tavani — tam cikti cipe gider (kod-spec §5.2).
const MODEL_CIKTI_TAVANI: usize = 500;
/// Hata metninin tavani: 4096 butcesini yemesin.
const MODEL_HATA_TAVANI: usize = 400;
/// Betik kaynaginin tavani. Bunun ustu bir "kisa betik" degildir.
const KOD_TAVANI: usize = 20_000;
/// Tur basina gercek calistirma sayisi (kod-spec §5.4).
const EN_COK_DENEME: usize = 2;

/// Surecin bitisini yoklama araligi. Std'de `wait_timeout` yok; yoklama
/// araligi kisa tutulur ki zaman asimi olcumu duyarli kalsin, ama bosuna
/// islemci yakmayacak kadar da uzun.
const YOKLAMA_ARALIGI: Duration = Duration::from_millis(10);

// ---------------------------------------------------------------------------
// Ag kalkani
// ---------------------------------------------------------------------------

/// Cocuk surecin agini ve dosya erisimini kesen mekanizma.
///
/// Tek varyant olmasi bilincli: "kalkan yok" bir VARYANT DEGILDIR. Kalkan
/// kurulamiyorsa `KodCalistirAraci` hic var olmaz — "kalkansiz mod" bir enum
/// dali olsaydi, ilerideki bir yama onu sessizce varsayilan yapabilirdi.
#[derive(Debug, Clone)]
pub enum AgKalkani {
    /// macOS `sandbox-exec`. Profil satir ici (`-p`) verilir; disk uzerinde
    /// kurcalanabilecek bir profil dosyasi birakmayiz.
    SandboxExec { arac: PathBuf },
}

impl AgKalkani {
    /// Sistemde kullanilabilir bir kalkan arar. DOGRULAMA YAPMAZ — cagiran
    /// `kalkan_dogrula` ile gercekten kestigini olcer.
    pub fn bul() -> Option<AgKalkani> {
        let arac = Path::new("/usr/bin/sandbox-exec");
        arac.is_file().then(|| AgKalkani::SandboxExec { arac: arac.to_path_buf() })
    }

    pub fn ad(&self) -> &'static str {
        match self {
            AgKalkani::SandboxExec { .. } => "sandbox-exec",
        }
    }

    /// Komutu kalkanla sarar.
    ///
    /// `kum` MUTLAK ve COZULMUS (symlink'siz) olmali: macOS'ta `/tmp`
    /// `/private/tmp`e baglantidir ve `(subpath "/tmp/...")` yazan bir profil
    /// hicbir seye izin vermez — kural sessizce "her sey yasak"a duser ve betik
    /// anlasilmaz bicimde coker.
    fn sar(&self, kum: &Path, yorumlayici: &Path, betik: &Path) -> Command {
        match self {
            AgKalkani::SandboxExec { arac } => {
                let mut k = Command::new(arac);
                k.arg("-p").arg(profil(kum)).arg(yorumlayici).arg(betik);
                k
            }
        }
    }
}

/// sandbox-exec profili.
///
/// `(allow file-read*)` genis BIRAKILMAK ZORUNDA: yorumlayici kendi
/// kitapligini, dyld onbellegini ve yerel ayar dosyalarini okur; daraltinca
/// python3 hic baslamaz. Bunun yerine asil hedef ACIKCA kapatiliyor —
/// kullanicinin ev dizini — ve kum havuzu geri aciliyor. Yani "her seyi oku"
/// degil, "sistemi oku, kullaniciyi okuma".
fn profil(kum: &Path) -> String {
    let ev = std::env::var("HOME").unwrap_or_else(|_| "/Users".to_string());
    let kum = kum.display();
    format!(
        "(version 1)\
         (deny default)\
         (allow process-exec*)\
         (allow process-fork)\
         (allow file-read*)\
         (deny file-read* (subpath \"{ev}\"))\
         (allow file-read* (subpath \"{kum}\"))\
         (allow file-write* (subpath \"{kum}\"))\
         (allow file-write-data (literal \"/dev/null\"))\
         (allow sysctl-read)\
         (allow mach-lookup)\
         (allow ipc-posix-shm)\
         (allow signal)\
         (deny network*)"
    )
}

// ---------------------------------------------------------------------------
// Yorumlayici kesfi
// ---------------------------------------------------------------------------

/// Makinede bulunan bir yorumlayici.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Yorumlayici {
    /// Semadaki `Secenek` degeri ve modele gorunen ad.
    pub anahtar: &'static str,
    pub yol: PathBuf,
    pub uzanti: &'static str,
}

/// Aranan yorumlayicilar, TERCIH SIRASIYLA.
///
/// JS once geliyor cunku beceri dosyasi (`kod.md`) ve Swift tarafi JS uzerine
/// kurulu; ayni istem iki limanda ayni dili uretsin. Python varsa ikinci sirada
/// gercek bir secenektir — Swift'te olmayan bir kazanc, ve `dil` alani tam da
/// bu yuzden var (bkz. `sema`).
const ADAYLAR: &[(&str, &str, &[&str])] = &[
    ("js", "js", &["/opt/homebrew/bin/node", "/usr/local/bin/node", "/usr/bin/node"]),
    (
        "python",
        "py",
        &["/opt/homebrew/bin/python3", "/usr/local/bin/python3", "/usr/bin/python3"],
    ),
];

fn yorumlayicilari_bul() -> Vec<Yorumlayici> {
    let mut bulunan = Vec::new();
    for (anahtar, uzanti, yollar) in ADAYLAR {
        // PATH TARANMAZ, sabit yollara bakilir. Gerekce: `PATH` cagiran
        // surecin ortamindan gelir ve zehirlenebilir; "node" adli bir seyi
        // PATH'ten bulup calistirmak, model ciktisini calistiran bir araca
        // yakisan bir davranis degil.
        if let Some(y) = yollar.iter().map(Path::new).find(|y| y.is_file()) {
            bulunan.push(Yorumlayici {
                anahtar,
                yol: y.to_path_buf(),
                uzanti,
            });
        }
    }
    bulunan
}

// ---------------------------------------------------------------------------
// Calistirma sonucu
// ---------------------------------------------------------------------------

/// Bir calistirmanin ham sonucu (kod-spec §5.2/§5.3 ile ayni ayrim).
#[derive(Debug, Clone, PartialEq)]
pub enum KodSonucu {
    /// Betik hatasiz bitti. `cikti` BOS ise cagiran bunu basari SUNMAMALIDIR.
    Basarili { cikti: String, ms: u128, kirpildi: bool },
    /// Betik hata dondurdu (sifir disi cikis kodu) — stderr + kismi cikti.
    Hata(String),
    /// Sure doldu; surec oldurudu.
    ZamanAsimi,
}

/// Betigi kalkan altinda calistirir. Arac disindan (test, eval) cagrilabilsin
/// diye serbest ve saf: durum tutmaz.
pub fn calistir(
    kalkan: &AgKalkani,
    yorumlayici: &Yorumlayici,
    kum: &Path,
    kod: &str,
    zaman_asimi: Duration,
) -> AracSonuc<KodSonucu> {
    if kod.trim().is_empty() {
        return Err(AracHatasi::GecersizArguman("bos betik".into()));
    }
    if kod.len() > KOD_TAVANI {
        return Err(AracHatasi::GecersizArguman("betik cok uzun".into()));
    }

    // Betik kum havuzunun ICINE yazilir: kalkan profili yalniz orayi okutur,
    // yani betigin baska bir yerde durmasi onu okunamaz yapardi.
    let calisma = kum.join("kod");
    std::fs::create_dir_all(&calisma)?;
    let betik = calisma.join(format!(
        "betik-{}-{}.{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos() ^ (kod.len() as u128),
        yorumlayici.uzanti
    ));
    std::fs::write(&betik, kod.as_bytes())?;

    let sonuc = calistir_ic(kalkan, yorumlayici, &calisma, &betik, zaman_asimi);
    // Betik HER YOLDA silinir: model ciktisi diskte birikirse hem kum havuzu
    // kirlenir hem `dosya_ara` onlari kullanici belgesi sanip listeler.
    std::fs::remove_file(&betik).ok();
    sonuc
}

fn calistir_ic(
    kalkan: &AgKalkani,
    yorumlayici: &Yorumlayici,
    calisma: &Path,
    betik: &Path,
    zaman_asimi: Duration,
) -> AracSonuc<KodSonucu> {
    let mut komut = kalkan.sar(calisma, &yorumlayici.yol, betik);
    komut
        .current_dir(calisma)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // ORTAM SILINIR. Cagiran surecin ortaminda API anahtari, oturum belirteci
    // ve ev dizini yolu bulunur; hepsi model ciktisinin okuyabilecegi yerdedir.
    // `PATH` yalniz yorumlayicinin kendi alt sureclerini bulabilmesi icin
    // asgari kaliyor, `HOME` bilerek kum havuzuna cevriliyor ki yorumlayici
    // kullanicinin ayar dosyalarini aramasin.
    komut.env_clear();
    komut.env("PATH", "/usr/bin:/bin");
    komut.env("HOME", calisma);
    komut.env("LANG", "en_US.UTF-8");
    // Python'un kullanici site-packages'ini ve onbellek yazmasini kapat:
    // ikisi de kum havuzu disina uzanan yollardir.
    komut.env("PYTHONDONTWRITEBYTECODE", "1");
    komut.env("PYTHONNOUSERSITE", "1");
    komut.env("NODE_OPTIONS", "");

    let baslangic = Instant::now();
    let mut cocuk = komut.spawn()?;

    // Iki boru IKI AYRI IS PARCACIGINDA okunur. Tek parcacikla sirayla okumak
    // klasik kilitlenmedir: stdout'u sonuna kadar beklerken cocuk stderr
    // borusunu doldurup bloke olur ve ikisi birbirini sonsuza dek bekler.
    let stdout = cocuk.stdout.take();
    let stderr = cocuk.stderr.take();
    let o_kolu = std::thread::spawn(move || boru_oku(stdout));
    let h_kolu = std::thread::spawn(move || boru_oku(stderr));

    let mut zaman_asti = false;
    let cikis = loop {
        match cocuk.try_wait() {
            Ok(Some(durum)) => break Some(durum),
            Ok(None) => {}
            // Yoklama hata verdiyse sureci bilemiyoruz; oldurup cikmak,
            // belirsiz bir sureci arkada birakmaktan iyidir.
            Err(_) => {
                zaman_asti = true;
                break None;
            }
        }
        if baslangic.elapsed() >= zaman_asimi {
            zaman_asti = true;
            break None;
        }
        std::thread::sleep(YOKLAMA_ARALIGI);
    };

    if zaman_asti {
        // OLDUR VE TOPLA. `kill` tek basina yetmez: `wait` cagrilmazsa surec
        // zombi olarak kalir ve uzun oturumda surec tablosu dolar.
        cocuk.kill().ok();
        cocuk.wait().ok();
    }
    let ms = baslangic.elapsed().as_millis();

    // Boru okuyucular cocuk oldugu icin EOF alir ve biter; `join` bu yuzden
    // asilmaz. Yine de panige karsi varsayilana duseriz — bir arac cokerse
    // butun turu goturur.
    let (cikti, kirpik_o) = o_kolu.join().unwrap_or_default();
    let (hata_metni, kirpik_h) = h_kolu.join().unwrap_or_default();

    if zaman_asti {
        return Ok(KodSonucu::ZamanAsimi);
    }
    let Some(durum) = cikis else {
        return Ok(KodSonucu::ZamanAsimi);
    };

    if durum.success() {
        return Ok(KodSonucu::Basarili {
            cikti: cikti.trim_end().to_string(),
            ms,
            kirpildi: kirpik_o,
        });
    }

    // Hata yolunda stderr ONCE gelir ama KISMI CIKTI da tasinir: Swift'te
    // olculdu — "SyntaxError" tek basina modelin kodu duzeltmesine yetmiyor,
    // hatadan onceki cikti nereye kadar gelindigini gosteriyor.
    let mut metin = hata_metni.trim().to_string();
    if metin.is_empty() {
        metin = format!("script exited with status {durum}");
    }
    if kirpik_h {
        metin.push_str("\n(error output truncated)");
    }
    if !cikti.trim().is_empty() {
        metin.push_str(&format!("\n--- output before the error ---\n{}", cikti.trim()));
    }
    Ok(KodSonucu::Hata(metin))
}

/// Boruyu SONUNA KADAR okur ama tavan ustunu ATAR.
///
/// Okumayi birakmak cocugu boru dolunca kilitler (ve zaman asimina kadar
/// bekletir); hic tavan koymamak sonsuz ciktinin butun bellegi yemesine izin
/// verir. Ikisinin ortasi: okumaya devam et, saklamayi birak.
fn boru_oku<R: Read>(boru: Option<R>) -> (String, bool) {
    let Some(mut boru) = boru else {
        return (String::new(), false);
    };
    let mut biriken: Vec<u8> = Vec::new();
    let mut tampon = [0u8; 8192];
    let mut kirpildi = false;
    loop {
        match boru.read(&mut tampon) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let yer = CIKTI_TAVANI.saturating_sub(biriken.len());
                if yer == 0 {
                    kirpildi = true;
                    continue;
                }
                let alinan = n.min(yer);
                biriken.extend_from_slice(&tampon[..alinan]);
                if alinan < n {
                    kirpildi = true;
                }
            }
        }
    }
    (String::from_utf8_lossy(&biriken).into_owned(), kirpildi)
}

// ---------------------------------------------------------------------------
// Deneme sayaci
// ---------------------------------------------------------------------------

/// Tur basina deneme sayaci (kod-spec §5.4).
///
/// Swift'te bu `weak` bir referanstaydi ve baglanmadiginda tavan yok sayilmasin
/// diye "fail-closed" ile 3 donuluyordu. Burada arac sayaci KENDI TUTAR: sahipli
/// bir alan unutulamaz, dolayisiyla fail-closed daline hic ihtiyac yok.
/// Sifirlamayi kabuk `yeni_tur()` ile yapar.
#[derive(Debug, Default)]
pub struct KodDurumu {
    deneme: AtomicUsize,
}

impl KodDurumu {
    pub fn yeni() -> Self {
        Self::default()
    }

    /// Yeni tur — sayac sifirlanir.
    pub fn yeni_tur(&self) {
        self.deneme.store(0, Ordering::SeqCst);
    }

    /// Denemeyi kaydeder ve KACINCI oldugunu doner (1'den baslar).
    fn dene(&self) -> usize {
        self.deneme.fetch_add(1, Ordering::SeqCst) + 1
    }
}

// ---------------------------------------------------------------------------
// Arac
// ---------------------------------------------------------------------------

/// `kod_calistir` — kisa bir betigi kalkan altinda calistirir.
pub struct KodCalistirAraci {
    kalkan: AgKalkani,
    yorumlayicilar: Vec<Yorumlayici>,
    durum: std::sync::Arc<KodDurumu>,
    /// Semayi her cagrida yeniden kurmamak icin onbellek; `sema()` sicak yolda
    /// (istem kurulumu + gramer) cagriliyor.
    sema_onbellek: Mutex<Option<ArgSema>>,
}

impl KodCalistirAraci {
    /// Araci YALNIZCA gercekten guvenli calistirabiliyorsak kurar.
    ///
    /// `None` donmesi bir arıza degil, DOGRU davranistir: var olmayan bir
    /// yorumlayiciyi cagiran ya da agi kesemeyen bir arac, model icin tuzaktir
    /// — katalogda gorur, cagirir, her seferinde hata alir ve turu kaybeder.
    /// Kabuk `None` gorurse araci eklemez; nedenini `tani()` soyler.
    pub fn kesfet() -> Option<KodCalistirAraci> {
        let yorumlayicilar = yorumlayicilari_bul();
        if yorumlayicilar.is_empty() {
            return None;
        }
        let kalkan = AgKalkani::bul()?;
        // KALKAN OLCULUR, VARSAYILMAZ.
        if !kalkan_dogrula(&kalkan, &yorumlayicilar[0]) {
            return None;
        }
        Some(KodCalistirAraci {
            kalkan,
            yorumlayicilar,
            durum: std::sync::Arc::new(KodDurumu::yeni()),
            sema_onbellek: Mutex::new(None),
        })
    }

    /// Kesif neden basarisiz oldu — kabuk bunu kullaniciya/gelistiriciye
    /// basar. Sessiz yokluk, yanlislikla kaldirilmis bir araci fark
    /// ettirmezdi.
    pub fn tani() -> String {
        let yorumlayicilar = yorumlayicilari_bul();
        if yorumlayicilar.is_empty() {
            return "kod_calistir kapali: makinede node ya da python3 bulunamadi".to_string();
        }
        let adlar: Vec<&str> = yorumlayicilar.iter().map(|y| y.anahtar).collect();
        let Some(kalkan) = AgKalkani::bul() else {
            return format!(
                "kod_calistir kapali: yorumlayici var ({}) ama bu isletim sisteminde \
                 surecin agini kesecek bir kalkan yok; agi kesemeden model kodu calistirilmaz",
                adlar.join(", ")
            );
        };
        if !kalkan_dogrula(&kalkan, &yorumlayicilar[0]) {
            return format!(
                "kod_calistir kapali: {} bulundu ama olcum sirasinda betigi calistiramadi",
                kalkan.ad()
            );
        }
        format!(
            "kod_calistir acik: yorumlayici {}, kalkan {} (ag kesik, yazma kum havuzunda)",
            adlar.join(", "),
            kalkan.ad()
        )
    }

    /// Sayaci sifirlayan kabuga verilen kol — `AracYurutucu`nun tur kancasina
    /// baglanir.
    pub fn tur_durumu(&self) -> std::sync::Arc<KodDurumu> {
        self.durum.clone()
    }

    pub fn yorumlayicilar(&self) -> &[Yorumlayici] {
        &self.yorumlayicilar
    }

    fn yorumlayici_sec(&self, istenen: Option<&str>) -> &Yorumlayici {
        istenen
            .and_then(|a| self.yorumlayicilar.iter().find(|y| y.anahtar == a))
            // Tercih sirasindaki ilki: liste bos olamaz (`kesfet` garantisi).
            .unwrap_or(&self.yorumlayicilar[0])
    }
}

/// Kalkanin GERCEKTEN kurulup calistigini olcer.
///
/// Yalnizca "dosya var mi" diye bakmak yeterli degil: profil sozdizimi
/// tutmazsa, isletim sistemi surumu kurali reddederse ya da yorumlayici kalkan
/// altinda hic baslayamazsa sonuc her cagrida sessiz bir hata olurdu. Bir kez,
/// kesif aninda, gercek bir surecle olculur.
fn kalkan_dogrula(kalkan: &AgKalkani, yorumlayici: &Yorumlayici) -> bool {
    let Ok(kum) = gecici_olcum_dizini() else {
        return false;
    };
    let betik = kum.join(format!("olcum.{}", yorumlayici.uzanti));
    // Iki yorumlayicida da ayni: bir sey bas.
    let govde = match yorumlayici.anahtar {
        "python" => "print(1)\n",
        _ => "console.log(1)\n",
    };
    let sonuc = std::fs::write(&betik, govde).is_ok()
        && matches!(
            calistir_ic(kalkan, yorumlayici, &kum, &betik, Duration::from_secs(15)),
            Ok(KodSonucu::Basarili { .. })
        );
    std::fs::remove_dir_all(&kum).ok();
    sonuc
}

/// Olcum icin gecici, COZULMUS (symlink'siz) dizin.
///
/// `canonicalize` sart: macOS'ta `std::env::temp_dir()` `/var/folders/...`
/// altini verir ve `/var` `/private/var`e baglantidir. Cozulmemis bir yol
/// sandbox profiline yazildiginda kural hicbir seye uymaz.
fn gecici_olcum_dizini() -> std::io::Result<PathBuf> {
    let kok = std::env::temp_dir().join(format!(
        "sirr-kod-olcum-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&kok)?;
    kok.canonicalize()
}

impl Arac for KodCalistirAraci {
    fn ad(&self) -> &str {
        "kod_calistir"
    }

    fn aciklama(&self) -> &str {
        // "NO network, NO filesystem, NO access to this device" cumlesi bir
        // vaat degil, OLCULMUS bir olgudur (bkz. `kalkan_dogrula`) — bu yuzden
        // yazilabiliyor. Son cumleler Swift'te olculen iki arizayi kapatir:
        // model araci sunucu/konteyner incelemek icin cagiriyordu, ve hicbir
        // sey bastirmayan betikleri basari saniyordu.
        "Runs a short script in an isolated sandbox with NO network access, NO access to your \
         files outside the working folder and NO access to this device or any server. Call this \
         for any calculation or transformation too complex for the hesapla tool (loops, dates, \
         text processing, simulations). Never call it to inspect servers, containers, ports, \
         processes, disks or the operating system — it cannot see them. The script MUST print \
         its result; a script that prints nothing returns an error. If the tool returns an \
         error, fix the code and call it ONCE more."
    }

    fn sema(&self) -> ArgSema {
        if let Some(s) = self.sema_onbellek.lock().expect("sema kilidi").clone() {
            return s;
        }
        let mut alanlar = vec![
            Alan::yeni(
                "kod",
                ArgSema::metin()
                    .aciklama("The script. Keep it minimal and PRINT the final result."),
            )
            .zorunlu(),
        ];

        // `dil` ALANI YALNIZ IKI YORUMLAYICI VARSA EKLENIR. Swift'te bu alan
        // tek gecerli degeri olmasina ragmen duruyordu ve kucuk modelin HER
        // cagrida doldurmasi gereken bir decode yuvasini yiyordu; bu yuzden
        // kaldirilmisti. Burada gercekten bir SECIM varsa mesru, yoksa ayni
        // israf olurdu.
        if self.yorumlayicilar.len() > 1 {
            let secenekler: Vec<&str> = self.yorumlayicilar.iter().map(|y| y.anahtar).collect();
            let varsayilan = self.yorumlayicilar[0].anahtar;
            alanlar.push(Alan::yeni(
                "dil",
                ArgSema::secenek(secenekler)
                    .aciklama(format!("Script language (default '{varsayilan}').")),
            ));
        }

        alanlar.push(Alan::yeni(
            "zaman_asimi_sn",
            ArgSema::tamsayi()
                .aralik(Some(1.0), Some(EN_COK_ZAMAN_ASIMI as f64))
                .aciklama(format!(
                    "Seconds to allow before the script is killed (default {VARSAYILAN_ZAMAN_ASIMI})."
                )),
        ));

        let sema = ArgSema::nesne(alanlar).aciklama("Run a short script and return its output");
        *self.sema_onbellek.lock().expect("sema kilidi") = Some(sema.clone());
        sema
    }

    /// DIS ARAC. Cihazin uzerinde bir surec baslatir ve model ciktisini
    /// calistirir; `AracYurutucu`nun onay kapisi bunu gormek zorunda.
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

            // Sayac artisi RET KARARINDAN ONCE: 3. cagri motoru hic gormez.
            let deneme = self.durum.dene();
            if deneme > EN_COK_DENEME {
                return AracSonucu::yeni(
                    "Kod calistirma siniri",
                    AracDurumu::Basarisiz("iki deneme tukendi".into()),
                    "error_final: give the user a short honest answer, do NOT retry",
                );
            }

            let iz = ctx.cip_baslat("kod", "Kod calistiriliyor…");
            let sonuc = self.yurut(&args, ctx, deneme);

            ctx.cip_guncelle(
                iz,
                IzGuncelleme::durum(sonuc.durum.clone())
                    .metin(sonuc.cip_metni.clone())
                    .ham_girdi(args.get("kod").and_then(|k| k.as_str()).unwrap_or_default())
                    .ham_cikti(sonuc.ham_cikti.clone().unwrap_or_default()),
            );
            // Kirletme yalniz GERCEKTEN calisan betikte: reddedilen ya da
            // baslamamis bir cagri dis dunyaya dokunmadi.
            if !matches!(sonuc.durum, AracDurumu::Basarisiz(_)) {
                ctx.kirlet();
            }
            sonuc
        })
    }
}

impl KodCalistirAraci {
    fn yurut(
        &self,
        args: &serde_json::Value,
        ctx: &AracBaglami,
        deneme: usize,
    ) -> AracSonucu {
        let son_deneme = deneme >= EN_COK_DENEME;

        let Some(kod) = args.get("kod").and_then(|v| v.as_str()).filter(|k| !k.trim().is_empty())
        else {
            return AracSonucu::basarisiz(&AracHatasi::EksikAlan("kod".into()));
        };
        let yorumlayici = self.yorumlayici_sec(args.get("dil").and_then(|v| v.as_str()));
        let zaman_asimi = Duration::from_secs(
            args.get("zaman_asimi_sn")
                .and_then(|v| v.as_u64())
                .map(|n| n.clamp(1, EN_COK_ZAMAN_ASIMI))
                .unwrap_or(VARSAYILAN_ZAMAN_ASIMI),
        );

        // Kalkan profili kum havuzunu subpath olarak yazar; cozulmemis yol
        // sessizce "hicbir sey serbest"e duser (bkz. `AgKalkani::sar`).
        let kum = match ctx.yolu_coz(".").and_then(|k| {
            k.canonicalize().map_err(AracHatasi::Giris)
        }) {
            Ok(k) => k,
            Err(h) => return AracSonucu::basarisiz(&h),
        };

        let ham = match calistir(&self.kalkan, yorumlayici, &kum, kod, zaman_asimi) {
            Ok(s) => s,
            Err(h) => return AracSonucu::basarisiz(&h),
        };

        match ham {
            // CIKTISIZ BASARI BIR BASARI DEGILDIR (Swift'te olculdu): model
            // "ok" gorunce sonucu UYDURUYORDU.
            KodSonucu::Basarili { cikti, .. } if cikti.trim().is_empty() => AracSonucu::yeni(
                if son_deneme { "Kod calistirilamadi" } else { "Hata · yeniden deneniyor" },
                AracDurumu::Basarisiz("cikti yok".into()),
                hata_metni(
                    "error: the script ran but printed nothing. Print the value you need",
                    son_deneme,
                ),
            )
            .ham_cikti("no output"),

            KodSonucu::Basarili { cikti, ms, kirpildi } => {
                let kisa = kirp(&cikti, MODEL_CIKTI_TAVANI);
                let not = if kirpildi || kisa.len() < cikti.len() {
                    "\n(output truncated)"
                } else {
                    ""
                };
                // Cikti buyukse bypass kanali: tam govde depoya, modele ilk
                // 500 karakter + kaynak_ref. Modelin sonucu ozetlemesi icin
                // basi yeter; tamamina bir sonraki arac referansla ulasir.
                let ek = if cikti.len() > MODEL_CIKTI_TAVANI {
                    let r = ctx.depola(
                        "kod",
                        &format!("{} karakter kod ciktisi", cikti.len()),
                        cikti.clone(),
                    );
                    sirr_cekirdek::kaynak_ref_eki(r.as_str())
                } else {
                    String::new()
                };
                AracSonucu::okundu(
                    format!("Kod calistirildi · {ms} ms"),
                    format!("ok ({ms} ms)\n{kisa}{not}{ek}"),
                )
                .ham_cikti(cikti)
            }

            KodSonucu::Hata(mesaj) => {
                // DIL TESHISI ONCE GELIR (Swift'te olculdu): betik Python ama
                // motor JS ise ham ayristirici mesaji modeli YANLIS onarima
                // surukler — parantez ekler, dili degistirmez, kalan tek
                // denemeyi de kaybeder.
                let neden = if yorumlayici.anahtar == "js" && python_mu(kod) {
                    format!(
                        "error: this script is Python, but this sandbox runs JavaScript. \
                         Rewrite the SAME logic in JavaScript: for (let i = 0; i < n; i++) \
                         {{...}}, console.log(x). Do not use range(), def, or indent blocks.\n{mesaj}"
                    )
                } else {
                    format!("error: {mesaj}")
                };
                AracSonucu::yeni(
                    if son_deneme { "Kod calistirilamadi" } else { "Hata · yeniden deneniyor" },
                    AracDurumu::Basarisiz("betik hata dondurdu".into()),
                    hata_metni(&neden, son_deneme),
                )
                .ham_cikti(mesaj)
            }

            KodSonucu::ZamanAsimi => AracSonucu::yeni(
                "Kod zaman asimina ugradi",
                AracDurumu::Basarisiz("sure doldu".into()),
                hata_metni(
                    "error: the script did not finish in time — it probably has an endless \
                     loop; bound every loop",
                    son_deneme,
                ),
            )
            .ham_cikti("timeout"),
        }
    }
}

/// Hata donusunu tek yerde kurar: son denemede `error_final` eklenir
/// (kod-spec §5.4) — ama NEDEN de soylenir, cunku modelin kullaniciya verecegi
/// durust kisa cevap nedeni icermeli.
fn hata_metni(neden: &str, son_deneme: bool) -> String {
    let kisa = kirp(neden, MODEL_HATA_TAVANI);
    if son_deneme {
        format!("{kisa}\nerror_final: give the user a short honest answer, do NOT retry")
    } else {
        kisa
    }
}

/// Karakter sinirinda kirpar (bayt degil: cok baytli karakterin ortasindan
/// kesmek gecersiz UTF-8 uretirdi).
fn kirp(metin: &str, tavan: usize) -> String {
    if metin.chars().count() <= tavan {
        return metin.to_string();
    }
    metin.chars().take(tavan).collect()
}

/// Betik Python'a mi benziyor?
///
/// Swift'ten devralinan olcum bulgusu: kucuk model kod vakalarinin cogunda
/// Python yaziyor. Saf ve statik — modelsiz test edilir.
pub fn python_mu(kod: &str) -> bool {
    // Her biri JS'te ya sozdizimi hatasi ya tanimsizdir; gecerli bir JS
    // betigini yanlislikla Python sanma riski yok. `print(` BILEREK yok:
    // bircok kabukta JS tarafinda da tanimlanabiliyor.
    const PYTHONA_OZGU: &[&str] = &["range(", "def ", "elif ", "end=", " True", " False", " None"];
    if PYTHONA_OZGU.iter().any(|t| kod.contains(t)) {
        return true;
    }
    // Iki noktayla biten blok basligi. JS'te `for`/`if`/`while` basligi
    // parantezle acilir ve iki nokta ile BITMEZ. `{` varsa dokunma:
    // `for (const k in o) { ... }` gecerli JS'tir ve nesne degismezleri de
    // iki nokta tasir.
    if kod.contains('{') {
        return false;
    }
    const BLOK_BASI: &[&str] = &["for ", "if ", "while ", "else", "try", "except"];
    kod.contains(':') && BLOK_BASI.iter().any(|t| kod.contains(t))
}

// ---------------------------------------------------------------------------
// Testler
// ---------------------------------------------------------------------------

#[cfg(test)]
mod testler {
    use super::*;
    use serde_json::json;
    use sirr_cekirdek::{BellekVeriDeposu, KaynakRef, SessizRaporlayici};
    use std::sync::Arc;

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
            "sirr-kod-{etiket}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&yol).expect("gecici dizin");
        yol.canonicalize().expect("cozulmus yol")
    }

    fn baglam(kok: &Path) -> AracBaglami {
        AracBaglami::yeni(
            Arc::new(BellekVeriDeposu::yeni()),
            kok,
            Arc::new(SessizRaporlayici),
        )
    }

    /// Kalkan/yorumlayici yoksa (CI, Linux) calistirma testleri ATLANIR — ama
    /// SESSIZCE degil: `kesfet` `None` donuyorsa arac zaten katalogda yok ve
    /// bu dogru davranistir; test bunu yazdirarak gecer.
    fn arac_veya_atla() -> Option<KodCalistirAraci> {
        match KodCalistirAraci::kesfet() {
            Some(a) => Some(a),
            None => {
                eprintln!("kod_calistir kesfedilemedi, calistirma testleri atlandi: {}", KodCalistirAraci::tani());
                None
            }
        }
    }

    // --- Modelsiz, saf testler (her makinede kosar) ---

    /// 1) Python teshisi: JS motoruna Python gonderen modeli DOGRU yone cevirir.
    #[test]
    fn python_teshisi_yanlis_pozitif_vermez() {
        assert!(python_mu("for i in range(10): print(i)"));
        assert!(python_mu("def f(x):\n  return x"));
        assert!(python_mu("if x == 1:\n  y = True"));
        assert!(python_mu("for i from 0 to 19: print(i)"), "iki noktayla biten blok");
        // Gecerli JS Python sanilmamali.
        assert!(!python_mu("for (let i = 0; i < 10; i++) { console.log(i) }"));
        assert!(!python_mu("const o = {a: 1}; console.log(JSON.stringify(o))"));
        assert!(!python_mu("for (const k in o) { console.log(k) }"));
        assert!(!python_mu("console.log(new Date().toISOString())"));
    }

    /// 2) Hata metni son denemede `error_final` tasir, oncekinde TASIMAZ; her
    ///    iki durumda da NEDEN korunur.
    #[test]
    fn hata_metni_son_denemede_error_final_ekler() {
        let ilk = hata_metni("error: boom", false);
        assert_eq!(ilk, "error: boom");
        assert!(!ilk.contains("error_final"));

        let son = hata_metni("error: boom", true);
        assert!(son.starts_with("error: boom"));
        assert!(son.contains("error_final"));
        assert!(son.contains("do NOT retry"));

        // Tavan: uzun neden kirpilir ama error_final DUSMEZ.
        let uzun = hata_metni(&"x".repeat(5_000), true);
        assert!(uzun.contains("error_final"));
        assert!(uzun.chars().count() < MODEL_HATA_TAVANI + 120);
    }

    /// 3) Sayac: iki gercek deneme, ucuncude motor HIC gorunmez.
    #[test]
    fn deneme_sayaci_ucuncu_cagriyi_reddeder() {
        let d = KodDurumu::yeni();
        assert_eq!(d.dene(), 1);
        assert_eq!(d.dene(), 2);
        assert_eq!(d.dene(), 3, "ucuncu cagri EN_COK_DENEME'yi asmali");
        d.yeni_tur();
        assert_eq!(d.dene(), 1, "tur basinda sifirlanmali");
    }

    /// 4) Kalkan profili kum havuzunu ve ev dizinini DOGRU kurallara baglar.
    #[test]
    fn kalkan_profili_agi_ve_ev_dizinini_kapatir() {
        let p = profil(Path::new("/private/tmp/kum"));
        assert!(p.contains("(deny network*)"), "ag acik kalmis");
        assert!(p.contains("(deny default)"), "varsayilan izinli kalmis");
        assert!(p.contains("(allow file-write* (subpath \"/private/tmp/kum\"))"));
        // Yazma yalniz kum havuzuna: baska bir subpath'e yazma izni olmamali.
        assert_eq!(p.matches("allow file-write*").count(), 1);
        let ev = std::env::var("HOME").unwrap_or_else(|_| "/Users".into());
        assert!(p.contains(&format!("(deny file-read* (subpath \"{ev}\"))")));
    }

    /// 5) Kirpma karakter sinirinda calisir — cok baytli karakteri BOLMEZ.
    #[test]
    fn kirpma_utf8_bozmaz() {
        let metin = "ğüşiöç".repeat(200);
        let k = kirp(&metin, 10);
        assert_eq!(k.chars().count(), 10);
        assert!(k.is_char_boundary(k.len()), "gecersiz UTF-8 uretildi");
        assert_eq!(kirp("kisa", 100), "kisa");
    }

    // --- Gercek surecle kosan testler ---

    /// 6) Betik GERCEKTEN kosar ve ciktisi yakalanir. "Derleme yesil" bir sey
    ///    kanitlamaz; bu test kanitlar.
    #[test]
    fn betik_gercekten_kosar_ve_cikti_yakalanir() {
        let Some(arac) = arac_veya_atla() else { return };
        let kok = gecici_dizin("kos");
        let mut ctx = baglam(&kok);

        let kod = match arac.yorumlayicilar()[0].anahtar {
            "python" => "print(6*7)",
            _ => "console.log(6*7)",
        };
        let sonuc = beklet(arac.calistir(json!({"kod": kod}), &mut ctx));
        assert_eq!(sonuc.durum, AracDurumu::Okundu, "{}", sonuc.cip_metni);
        assert!(sonuc.modele_donen.contains("42"), "{}", sonuc.modele_donen);
        assert!(sonuc.modele_donen.starts_with("ok ("));
        assert!(ctx.oturum_kirli(), "dis arac calisti, oturum kirlenmeli");
        std::fs::remove_dir_all(&kok).ok();
    }

    /// 7) AG GERCEKTEN KESIK. Bu testin gecmesi, `aciklama`daki "NO network"
    ///    cumlesinin bir vaat degil olgu olmasinin tek dayanagidir.
    #[test]
    fn ag_gercekten_kesik() {
        let Some(arac) = arac_veya_atla() else { return };
        let kok = gecici_dizin("ag");
        let mut ctx = baglam(&kok);

        let kod = match arac.yorumlayicilar()[0].anahtar {
            "python" => {
                "import socket\n\
                 try:\n\
                 \x20   socket.create_connection(('1.1.1.1', 80), timeout=3)\n\
                 \x20   print('NET_OPEN')\n\
                 except Exception:\n\
                 \x20   print('NET_BLOCKED')\n"
            }
            _ => {
                "const s=require('net').connect(80,'1.1.1.1');\
                 s.on('connect',()=>{console.log('NET_OPEN');process.exit(0)});\
                 s.on('error',()=>{console.log('NET_BLOCKED');process.exit(0)});"
            }
        };
        let sonuc = beklet(arac.calistir(json!({"kod": kod, "zaman_asimi_sn": 20}), &mut ctx));
        let cikti = sonuc.ham_cikti.clone().unwrap_or_default();
        assert!(
            !cikti.contains("NET_OPEN"),
            "AG ACIK KALDI — arac katalogda olmamali: {cikti}"
        );
        assert!(cikti.contains("NET_BLOCKED"), "beklenmeyen cikti: {cikti} / {}", sonuc.modele_donen);
        std::fs::remove_dir_all(&kok).ok();
    }

    /// 8) KUM HAVUZU: betik disariya yazamaz, kullanicinin ev dizinini
    ///    okuyamaz; ama kendi klasorune yazabilir (mesru is bozulmasin).
    #[test]
    fn kum_havuzu_disina_yazilamaz_ev_dizini_okunamaz() {
        let Some(arac) = arac_veya_atla() else { return };
        let kok = gecici_dizin("kum");
        let mut ctx = baglam(&kok);
        let hedef = std::env::temp_dir().join(format!("sirr-kacak-{}.txt", std::process::id()));
        std::fs::remove_file(&hedef).ok();

        let kod = match arac.yorumlayicilar()[0].anahtar {
            "python" => format!(
                "import os\n\
                 try:\n\
                 \x20   open({hedef:?}, 'w').write('x')\n\
                 \x20   print('OUT_WRITE_OK')\n\
                 except Exception:\n\
                 \x20   print('OUT_WRITE_BLOCKED')\n\
                 try:\n\
                 \x20   os.listdir(os.path.expanduser('~'))\n\
                 \x20   print('HOME_READ_OK')\n\
                 except Exception:\n\
                 \x20   print('HOME_READ_BLOCKED')\n\
                 open('ic.txt','w').write('y')\n\
                 print('IN_WRITE_OK')\n"
            ),
            _ => format!(
                "const fs=require('fs');\
                 try{{fs.writeFileSync({hedef:?},'x');console.log('OUT_WRITE_OK')}}\
                 catch(e){{console.log('OUT_WRITE_BLOCKED')}}\
                 try{{fs.readdirSync(process.env.HOME_REAL||'/Users');console.log('HOME_READ_OK')}}\
                 catch(e){{console.log('HOME_READ_BLOCKED')}}\
                 fs.writeFileSync('ic.txt','y');console.log('IN_WRITE_OK')"
            ),
        };
        let sonuc = beklet(arac.calistir(json!({"kod": kod, "zaman_asimi_sn": 20}), &mut ctx));
        let cikti = sonuc.ham_cikti.clone().unwrap_or_default();
        assert!(cikti.contains("OUT_WRITE_BLOCKED"), "kum havuzu disina yazildi: {cikti}");
        assert!(!hedef.exists(), "dosya gercekten olusmus: {}", hedef.display());
        assert!(cikti.contains("IN_WRITE_OK"), "kum havuzuna yazamadi: {cikti}");
        std::fs::remove_file(&hedef).ok();
        std::fs::remove_dir_all(&kok).ok();
    }

    /// 9) SONSUZ DONGU: zaman asiminda surec OLDURULUR ve sure gercekten
    ///    tavanda kesilir (sessiz donma yok).
    #[test]
    fn sonsuz_dongu_zaman_asiminda_oldurulur() {
        let Some(arac) = arac_veya_atla() else { return };
        let kok = gecici_dizin("dongu");
        let mut ctx = baglam(&kok);
        let kod = match arac.yorumlayicilar()[0].anahtar {
            "python" => "while True: pass",
            _ => "while(true){}",
        };

        let basladi = Instant::now();
        let sonuc = beklet(arac.calistir(json!({"kod": kod, "zaman_asimi_sn": 2}), &mut ctx));
        let gecen = basladi.elapsed();

        assert!(matches!(sonuc.durum, AracDurumu::Basarisiz(_)), "{}", sonuc.cip_metni);
        assert!(sonuc.modele_donen.contains("did not finish in time"), "{}", sonuc.modele_donen);
        assert!(gecen < Duration::from_secs(10), "surec oldurulmemis: {gecen:?}");
        assert!(gecen >= Duration::from_secs(2), "tavandan once dondu: {gecen:?}");
        std::fs::remove_dir_all(&kok).ok();
    }

    /// 10) CIKTISIZ BASARI BIR BASARI DEGILDIR — model "ok" gorup sonucu
    ///     uyduramasin.
    #[test]
    fn ciktisiz_betik_hata_sayilir() {
        let Some(arac) = arac_veya_atla() else { return };
        let kok = gecici_dizin("sessiz");
        let mut ctx = baglam(&kok);
        let kod = match arac.yorumlayicilar()[0].anahtar {
            "python" => "x = 1 + 1",
            _ => "const x = 1 + 1;",
        };
        let sonuc = beklet(arac.calistir(json!({"kod": kod}), &mut ctx));
        assert!(matches!(sonuc.durum, AracDurumu::Basarisiz(_)), "{}", sonuc.cip_metni);
        assert!(sonuc.modele_donen.contains("printed nothing"), "{}", sonuc.modele_donen);
        std::fs::remove_dir_all(&kok).ok();
    }

    /// 11) DEVASA CIKTI bellegi yemez, kirpilir ve kirpildigi SOYLENIR; buyuk
    ///     cikti bypass kanalindan gecer.
    #[test]
    fn devasa_cikti_kirpilir_ve_bypass_kanalina_duser() {
        let Some(arac) = arac_veya_atla() else { return };
        let kok = gecici_dizin("dev");
        let mut ctx = baglam(&kok);
        let kod = match arac.yorumlayicilar()[0].anahtar {
            "python" => "for i in range(200000): print('satir', i)",
            _ => "for(let i=0;i<200000;i++) console.log('satir',i)",
        };
        let sonuc = beklet(arac.calistir(json!({"kod": kod, "zaman_asimi_sn": 25}), &mut ctx));

        let ham = sonuc.ham_cikti.clone().unwrap_or_default();
        assert!(ham.len() <= CIKTI_TAVANI + 16, "tavan asilmis: {}", ham.len());
        // Modele giden metin cok daha kisa: tam cikti bile modele DOKULMEZ.
        assert!(sonuc.modele_donen.len() < 900, "{}", sonuc.modele_donen.len());
        if sonuc.durum == AracDurumu::Okundu {
            assert!(sonuc.modele_donen.contains("kaynak_ref"), "{}", sonuc.modele_donen);
            let kayit = ctx.depodan(&KaynakRef("kod#1".into())).expect("depo kaydi");
            assert!(kayit.govde.len() > MODEL_CIKTI_TAVANI);
        }
        std::fs::remove_dir_all(&kok).ok();
    }

    /// 12) Ucuncu cagri MOTORU HIC GORMEZ: reddi arac verir, modelin saymasi
    ///     beklenmez.
    #[test]
    fn ucuncu_cagri_aractan_reddedilir() {
        let Some(arac) = arac_veya_atla() else { return };
        let kok = gecici_dizin("sayac");
        let mut ctx = baglam(&kok);
        let kod = match arac.yorumlayicilar()[0].anahtar {
            "python" => "print(1)",
            _ => "console.log(1)",
        };

        for tur in 1..=2 {
            let s = beklet(arac.calistir(json!({"kod": kod}), &mut ctx));
            assert_eq!(s.durum, AracDurumu::Okundu, "{tur}. deneme dusmemeli");
        }
        let ucuncu = beklet(arac.calistir(json!({"kod": kod}), &mut ctx));
        assert!(matches!(ucuncu.durum, AracDurumu::Basarisiz(_)));
        assert!(ucuncu.modele_donen.starts_with("error_final"), "{}", ucuncu.modele_donen);

        // Tur kancasi sayaci sifirlar; kabuk bunu `AracYurutucu`ya baglar.
        arac.tur_durumu().yeni_tur();
        let yeni_tur = beklet(arac.calistir(json!({"kod": kod}), &mut ctx));
        assert_eq!(yeni_tur.durum, AracDurumu::Okundu, "tur sifirlanmamis");
        std::fs::remove_dir_all(&kok).ok();
    }

    /// 13) Sema sozlesmesi: `dil` alani YALNIZ gercek bir secim varsa durur
    ///     (bos decode yuvasi acmaz), zaman asimi tavani baglayicidir.
    #[test]
    fn sema_sozlesmesi_baglayicidir() {
        let Some(arac) = arac_veya_atla() else { return };
        let sema = arac.sema();
        let js = sema.json_schema();
        assert_eq!(js["required"], json!(["kod"]));
        assert_eq!(js["additionalProperties"], json!(false));

        let dil_var = js["properties"].get("dil").is_some();
        assert_eq!(
            dil_var,
            arac.yorumlayicilar().len() > 1,
            "dil alani yalniz coklu yorumlayicida olmali"
        );

        assert!(sema.dogrula(&json!({"kod": "x"})).is_ok());
        assert!(sema.dogrula(&json!({})).is_err());
        assert!(sema.dogrula(&json!({"kod": "x", "zaman_asimi_sn": 999})).is_err(), "tavan");
        assert!(sema.dogrula(&json!({"kod": "x", "uydurma": 1})).is_ok(), "sema gormezden gelir");
        if dil_var {
            assert!(sema.dogrula(&json!({"kod": "x", "dil": "cobol"})).is_err());
        }
    }

    /// 14) Yonlendirici bu araci Hesap niyetinde secebilmeli.
    #[test]
    fn yonlendirici_ipuclari_tutar() {
        use crate::yonlendirici::{Yonlendirici, sadelestir};
        let Some(arac) = arac_veya_atla() else { return };
        let metin = sadelestir(&format!("{} {}", arac.ad(), arac.aciklama()));
        // Hesap profilinin ipuclari: "kod", "calistir".
        assert!(metin.contains("kod") && metin.contains("calistir"));

        let mut katalog = sirr_cekirdek::AracKatalogu::yeni();
        katalog.ekle(Arc::new(crate::zaman::ZamanAraci::yeni()));
        katalog.ekle(Arc::new(KodCalistirAraci::kesfet().expect("kesfedildi")));
        let secilen = Yonlendirici::yeni().en_fazla(1).sec("bunu python ile hesapla", &katalog);
        assert_eq!(secilen[0].ad(), "kod_calistir");
    }
}
