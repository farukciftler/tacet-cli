//! sirr-cli — cihaz-ustu asistanin terminal kabugu.
//!
//! NEDEN VAR: sirr'in mantik katmani (yonlendirme, istem kurulumu, gramer,
//! arac yurutme, bypass kanali, beceri/hafiza enjeksiyonu) bir iOS uygulamasi
//! icinde saklandiginda yalniz simulator acilarak gozlenebilir. Bu ikili ayni
//! katmani terminalden surer: `sohbet` gercek bir modelle Claude Code tarzi
//! akici bir tur dongusu acar, `eval` CI'da koser, `gramer`/`araclar` istemin
//! birebir kaynagini basar.
//!
//! MOTOR SECIMI OTOMATIK. `--motor oto` (varsayilan) yerel bir Qwen agirligi
//! bulursa (env `SIRR_MODEL`/`SIRR_TOKENIZER` ya da `~/models/qwen2.5-3b`)
//! ve ikili `candle` ozelligiyle derlendiyse GERCEK modeli kullanir; yoksa
//! ANLAMLI bir mesajla SahteMotor'a duser — sessizce degil.
//!
//! AG: varsayilan kurulumda hicbir komut ag cagrisi yapmaz. Ag YALNIZCA
//! kullanicinin kendi ekledigi web aramasi (`web_ara`/`web_getir`) ya da MCP
//! baglantisi (`~/.sirr/mcp.json`) kullanildiginda olusur; ikisi de dis
//! araclardir ve kirli oturumda onay kapisindan gecer.
//!
//! BAGIMLILIK: renk/tty icin `std::io::IsTerminal` (std, 1.70+); satir
//! duzenleme kutuphanesi (rustyline/crossterm) BILEREK EKLENMEDI. sirr'in
//! kimligi "hazir crate cekmemek" uzerine kurulu; ANSI kacislari birkac sabit
//! dizgidir ve tty tespiti std'de vardir. Ok-tusuyla gecmis gezme bunun
//! bedeli — ama `/gecmis` komutu ayni bilgiyi verir ve kimlik korunur.

use clap::{Parser, Subcommand, ValueEnum};
use sirr_araclar::belge_duzenle::BelgeDuzenleAraci;
use sirr_araclar::belge_olustur::BelgeOlusturAraci;
use sirr_araclar::belge_oku::BelgeOkuAraci;
use sirr_araclar::dosya_ara::DosyaAraAraci;
use sirr_araclar::hafiza::{HafizaAraci, PaylasimliHafiza};
use sirr_araclar::hesap::HesapAraci;
use sirr_araclar::kod_calistir::{KodCalistirAraci, KodDurumu};
use sirr_araclar::mcp;
use sirr_araclar::veri_deposu::PaylasimliDepo;
use sirr_araclar::web_ara::{WebAraAraci, WebGetirAraci};
use sirr_araclar::yonlendirici::Yonlendirici;
use sirr_araclar::yurutucu::{AracYurutucu, OnayIstegi, OnayKapisi, SessizRet};
use sirr_araclar::zaman::ZamanAraci;
use sirr_beceri::{BeceriDeposu, EnjeksiyonDurumu, enjeksiyon_metni};
use sirr_cekirdek::{
    AracBaglami, AracKatalogu, ArgSema, IzToplayici, Raporlayici,
    VeriDeposu as CekirdekVeriDeposu,
};
use sirr_eval::{SISTEM_TALIMATI, SahteSecici};
use sirr_gramer::{CagriKisiti, Gramer};
use sirr_hafiza::HafizaDeposu;
use sirr_motor::{Istem, MotorSaglayici, OrneklemeAyari, SahteMotor, TokenSayaci, Tur, bekle};
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

/// Eval'in "yesil" sayildigi esik.
const VARSAYILAN_ESIK: f64 = 1.0;

/// Dis dunyaya veri CIKARAN araclarin adlari — onay kapisinin girdisi.
///
/// Bu liste artik BOS DEGIL: `web_ara` ve `web_getir` sorguyu (ki kullanici
/// verisi tasiyabilir) uzak bir sunucuya gonderir. `hafiza` KIRLETIR ama bu
/// listede DEGIL — cunku hafiza yerel diske yazar, disari VERI CIKARMAZ; onay
/// kapisi "disari cikan veri" icindir, "kisisel veri okundu" icin degil. MCP
/// araclarinin adlari calisma aninda katalogdan ogrenildigi icin burada sabit
/// yazilamaz; onlar `mcp::yurutucuyu_baglar` ile ayrica baglanir.
const DIS_ARACLAR: &[&str] = &["web_ara", "web_getir"];

/// Varsayilan model klasoru (`~/models/` altinda).
///
/// qwen3-4b SECILDI, en buyuk BFCL puani ONDE OLMAMASINA ragmen. Qwen3-8B
/// arac cagirmada daha iyi (genel 42.57 / cok-tur 41.75, karsisinda 35.68 /
/// 22.12) ama 5 GB ve beklenen hizi 28-39 bel/sn — yani cihaz-ustu bir kabuk
/// icin esigin dibinde. Bu urunde gecikme bir konfor ayrintisi degil: 4096
/// pencereli bir arac dongusunde her tur yeniden onyukleme yapilir, kullanici
/// her turda bekler. 4B, 2.5 GB'ta iki kat hizli koser. Daha iyi arac secimi
/// isteyen `--model qwen3-8b` yazar; secim ACIK, sessiz degil.
const VARSAYILAN_MODEL: &str = "qwen3-4b";

// ANSI kacislari — yalniz tty'de basilir (bkz. `Renk`).
const SIFIRLA: &str = "\x1b[0m";
const SONUK: &str = "\x1b[2m";
const CAMGOBEGI: &str = "\x1b[36m";
const SARI: &str = "\x1b[33m";
const YESIL: &str = "\x1b[32m";

#[derive(Parser)]
#[command(name = "sirr", about = "sirr — cihaz-ustu asistanin terminal kabugu")]
struct Kabuk {
    #[command(subcommand)]
    komut: Komut,
}

#[derive(Subcommand)]
enum Komut {
    /// Etkilesimli sohbet: mesaj okur, arac dongusunu surer, akan cevap basar.
    Sohbet {
        #[arg(long, value_enum, default_value_t = MotorSecimi::Oto)]
        motor: MotorSecimi,
        /// SahteMotor betigi (tekrarlanabilir). Verilmezse sabit cevap doner.
        #[arg(long)]
        betik: Vec<String>,
        /// Her turda modele giden istemin tamamini bas — tanilama.
        #[arg(long)]
        goster_istem: bool,
        /// Calisma dizini (kum havuzu koku).
        #[arg(long, default_value = ".")]
        dizin: String,
        /// Tek mesaj kosup cikar (betikli tanilama icin). Bu kipte onay kapisi
        /// SessizRet kalir: soracak kimse yoktur, veri CIKMAZ.
        #[arg(long)]
        mesaj: Option<String>,
        /// Kullanilacak yerel model klasoru (`~/models/<ad>`). Varsayilan
        /// `qwen3-4b`. `SIRR_MODEL`/`SIRR_TOKENIZER` bunu EZER.
        #[arg(long, default_value = VARSAYILAN_MODEL)]
        model: String,
    },
    /// Eval setini kosar; cikis kodu basari oranina bagli.
    Eval {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = VARSAYILAN_ESIK)]
        esik: f64,
    },
    /// Katalogu ve semalarini listeler.
    Araclar {
        #[arg(long)]
        sema: bool,
    },
    /// Bir aracin semasindan uretilen grameri gosterir (tanilama).
    Gramer {
        #[arg(long)]
        arac: String,
        #[arg(long)]
        dene: Option<String>,
    },
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum MotorSecimi {
    /// Yerel model varsa candle, yoksa sahte — otomatik.
    Oto,
    Sahte,
    Candle,
}

fn main() -> ExitCode {
    match Kabuk::parse().komut {
        Komut::Sohbet { motor, betik, goster_istem, dizin, mesaj, model } => {
            sohbet(motor, betik, goster_istem, &dizin, mesaj, &model)
        }
        Komut::Eval { json, esik } => eval(json, esik),
        Komut::Araclar { sema } => araclar(sema),
        Komut::Gramer { arac, dene } => gramer(&arac, dene.as_deref()),
    }
}

// ---------------------------------------------------------------------------
// Renk yardimcisi — tty degilse (borulanmis cikti, CI) renk YAZILMAZ.
// ---------------------------------------------------------------------------

struct Renk {
    tty: bool,
}

impl Renk {
    fn kur() -> Self {
        Self { tty: std::io::stdout().is_terminal() }
    }
    fn boya(&self, kod: &str, metin: &str) -> String {
        if self.tty {
            format!("{kod}{metin}{SIFIRLA}")
        } else {
            metin.to_string()
        }
    }
}

// ---------------------------------------------------------------------------
// Onay kapisi — terminalde gercek soru.
// ---------------------------------------------------------------------------

/// Kirli oturumda dis araca GERCEK PAYLOAD'i gosterip e/h sorar.
///
/// NEDEN `DaimaOnay` DEGIL: kapiyi hep "evet"e baglamak onu islevsiz kilar —
/// kullanici gonderilecek veriyi gormeden onaylamis olur. `SessizRet` de yanlis
/// olurdu: burada soracak biri VAR. Kapinin var olma sebebi, gonderilen icerigi
/// (sorgu dizesi) kullaniciya BIREBIR gostermek.
struct TerminalOnay;

impl OnayKapisi for TerminalOnay {
    fn iste(&self, istek: &OnayIstegi) -> bool {
        eprintln!();
        eprintln!("  ⚠ '{}' aracı dış dünyaya veri gönderecek:", istek.arac_adi);
        eprintln!("    {}", istek.icerik);
        eprint!("  Izin veriyor musun? [e/H] ");
        let _ = std::io::stderr().flush();
        let mut satir = String::new();
        if std::io::stdin().read_line(&mut satir).is_err() {
            return false;
        }
        matches!(satir.trim().to_lowercase().as_str(), "e" | "evet" | "y" | "yes")
    }
}

// ---------------------------------------------------------------------------
// Motor secimi
// ---------------------------------------------------------------------------

/// Yerel model dosyalarini cozer: once env, sonra `~/models/<ad>`.
///
/// AG YOK: hicbir sey indirilmez, yalniz var olan yerel dosyalar aranir.
///
/// MIMARI BURADA TAHMIN EDILMEZ. Klasor adi ("qwen3-4b") yalniz bir etikettir;
/// hangi modulun yuklenecegini GGUF ustverisi soyler (bkz. `Mimari::cozumle`).
/// Ad ile icerik ayrisirsa — kullanici klasore baska bir agirlik koyarsa —
/// dogru olan icerige uymaktir.
fn model_yollari(ad: &str) -> Option<(String, String)> {
    if let (Ok(m), Ok(t)) = (std::env::var("SIRR_MODEL"), std::env::var("SIRR_TOKENIZER")) {
        return Some((m, t));
    }
    let ev = std::env::var_os("HOME").map(PathBuf::from)?;
    let dizin = ev.join("models").join(ad);
    let tok = dizin.join("tokenizer.json");
    if !tok.is_file() {
        return None;
    }
    // Klasordeki ilk .gguf: dosya adi surumle degisir, sabit yazmak kirilgan.
    let gguf = std::fs::read_dir(&dizin)
        .ok()?
        .filter_map(Result::ok)
        .map(|g| g.path())
        .find(|p| p.extension().is_some_and(|e| e == "gguf"))?;
    Some((gguf.to_string_lossy().into_owned(), tok.to_string_lossy().into_owned()))
}

/// Secime gore motoru kurar. `Oto`: model varsa candle, yoksa sahte (mesajla).
fn motoru_kur(
    secim: MotorSecimi,
    betik: Vec<String>,
    model_adi: &str,
    renk: &Renk,
) -> Result<Arc<dyn MotorSaglayici>, String> {
    let sahte = |b: Vec<String>| -> Arc<dyn MotorSaglayici> {
        Arc::new(SahteMotor::betik(b).varsayilanla("Anladim. (sahte motor)"))
    };
    match secim {
        MotorSecimi::Sahte => Ok(sahte(betik)),
        MotorSecimi::Candle => match model_yollari(model_adi) {
            Some((m, t)) => candle_motor_yoldan(&m, &t),
            // `--motor candle` ACIK bir istektir: model yoksa sahteye dusmek
            // degil, HATA vermek dogrudur (bkz. `Oto` dali bunun tersini yapar).
            None => Err(format!("yerel model bulunamadi: ~/models/{model_adi}")),
        },
        MotorSecimi::Oto => match model_yollari(model_adi) {
            Some((m, t)) => match candle_motor_yoldan(&m, &t) {
                Ok(motor) => {
                    eprintln!("{}", renk.boya(SONUK, &format!("(model: {m})")));
                    Ok(motor)
                }
                // candle ozelligi kapaliysa ya da yukleme basarisizsa: sessizce
                // sahteye DUSMEK yanlis olurdu — kullanici gercek model bekliyor.
                Err(e) => {
                    eprintln!("{}", renk.boya(SARI, &format!("(gercek model kullanilamadi: {e})")));
                    eprintln!("{}", renk.boya(SONUK, "(SahteMotor'a dusuldu — cevaplar sabittir)"));
                    Ok(sahte(betik))
                }
            },
            None => {
                eprintln!(
                    "{}",
                    renk.boya(
                        SONUK,
                        &format!(
                            "(yerel model bulunamadi: SIRR_MODEL/SIRR_TOKENIZER ayarlayin ya da \
                             ~/models/{model_adi} altina koyun — simdilik SahteMotor)"
                        )
                    )
                );
                Ok(sahte(betik))
            }
        },
    }
}

// ---------------------------------------------------------------------------
// Katalog
// ---------------------------------------------------------------------------

/// Kabugun gordugu TAM katalog. Tek dogruluk kaynagi: istem, gramer ve yurutme
/// hepsi bundan turer.
///
/// `KodDurumu` DISARI verilir cunku deneme sayaci HER TURDA sifirlanmali ve
/// bunu ancak kabuk (tur sinirini bilen taraf) yapabilir; arac Arc icinde
/// katalogda kaybolunca ona ulasilamazdi.
fn oturum_katalogu(
    depo: &Arc<PaylasimliDepo>,
    hafiza: &PaylasimliHafiza,
    renk: &Renk,
) -> (AracKatalogu, Option<Arc<KodDurumu>>) {
    let mut k = AracKatalogu::yeni();
    // belge_duzenle UC KADEMELI cozer (acik yol -> oturum izleyicisi -> en
    // son degisen belge). Izleyici (2. kademe) `YurutmeSonucu`nun dosya
    // yolunu tasimadigi icin BESLENEMIYOR; bu yuzden 3. kademeyle (dosya
    // zamani) kuruluyor ve "excel yap -> onu tablo goster" akisi bugun de
    // calisiyor. Izleyiciyi baglamak yurutucunun dosya_yolu'nu disari
    // vermesini gerektirir — bkz. rapor "eksikler".
    k.ekle(Arc::new(HesapAraci))
        .ekle(Arc::new(ZamanAraci::yeni()))
        .ekle(Arc::new(BelgeOkuAraci::depo_ile(Arc::clone(depo))))
        .ekle(Arc::new(BelgeOlusturAraci::yeni()))
        .ekle(Arc::new(BelgeDuzenleAraci::yeni()))
        .ekle(Arc::new(DosyaAraAraci::yeni()))
        .ekle(Arc::new(WebAraAraci::depo_ile(Arc::clone(depo))))
        .ekle(Arc::new(WebGetirAraci::depo_ile(Arc::clone(depo))))
        .ekle(Arc::new(HafizaAraci::yeni(hafiza.clone())));

    // kod_calistir YALNIZCA guvenli calistirilabiliyorsa eklenir: yorumlayici
    // + ag kalkani olcumu gecmezse model icin tuzak olur (bkz. `kesfet`).
    let kod_durum = match KodCalistirAraci::kesfet() {
        Some(arac) => {
            let durum = arac.tur_durumu();
            k.ekle(Arc::new(arac));
            Some(durum)
        }
        None => {
            eprintln!("{}", renk.boya(SONUK, &format!("({})", KodCalistirAraci::tani())));
            None
        }
    };
    (k, kod_durum)
}

// ---------------------------------------------------------------------------
// sohbet
// ---------------------------------------------------------------------------

fn sohbet(
    secim: MotorSecimi,
    betik: Vec<String>,
    goster_istem: bool,
    dizin: &str,
    tek_mesaj: Option<String>,
    model_adi: &str,
) -> ExitCode {
    let renk = Renk::kur();
    let etkilesimli = tek_mesaj.is_none();

    let motor = match motoru_kur(secim, betik, model_adi, &renk) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("hata: {e}");
            return ExitCode::FAILURE;
        }
    };

    let depo = Arc::new(PaylasimliDepo::yeni());

    // HAFIZA: etkilesimli kipte ~/.sirr/hafiza.json'a KALICI; tek-mesaj/tanilama
    // kipinde bellek-ici (gercek ev dizinini kirletme). Diske yazamayan bir
    // ortamda da bellek-ici depoyla calisir.
    let hafiza = if etkilesimli {
        match HafizaDeposu::varsayilan_yol() {
            Some(y) => PaylasimliHafiza::yeni(HafizaDeposu::dosyadan(y)),
            None => PaylasimliHafiza::bellekte(),
        }
    } else {
        PaylasimliHafiza::bellekte()
    };

    // BECERI: gomulu beceriler + (varsa) kullanicinin ~/.sirr/beceriler'i.
    let mut beceri_depo = BeceriDeposu::varsayilan();
    if let Some(d) = sirr_beceri::kullanici_dizini()
        && d.is_dir()
    {
        beceri_depo.dizinden_yukle(d);
    }
    let mut enj_durum = EnjeksiyonDurumu::yeni();

    let (mut katalog, kod_durum) = oturum_katalogu(&depo, &hafiza, &renk);

    // MCP BAGLANTILARI — `~/.sirr/mcp.json`. Dosya yoksa hicbir sey yapmaz ve
    // AGA CIKILMAZ.
    let mcp_yukleme = mcp::varsayilandan_yukle();
    let mcp_adlari = mcp::katalogu_besle(&mut katalog, &mcp_yukleme);
    mcp_raporla(&mcp_yukleme, &renk);

    // ONAY KAPISI: etkilesimli kipte gercek soru, tanilama kipinde SessizRet.
    let mut yurutucu = AracYurutucu::yeni(katalog.clone());
    if etkilesimli {
        yurutucu = yurutucu.kapiyla(TerminalOnay);
    } else {
        yurutucu = yurutucu.kapiyla(SessizRet);
    }
    for ad in DIS_ARACLAR {
        yurutucu = yurutucu.dis_arac(*ad);
    }
    // MCP araclari da DIS ARACtir: kirli oturumda her biri onay kapisindan gecer.
    yurutucu = mcp::yurutucuyu_baglar(yurutucu, &mcp_adlari);

    let izler = Arc::new(IzToplayici::yeni());
    let mut ctx = AracBaglami::yeni(
        Arc::clone(&depo) as Arc<dyn CekirdekVeriDeposu>,
        dizin,
        Arc::clone(&izler) as Arc<dyn Raporlayici>,
    );

    let yonlendirici = Yonlendirici::yeni();
    let sayaci = TokenSayaci::default();
    let mut gecmis: Vec<Tur> = Vec::new();

    // KISIT dongunun DISINDA bir kez kurulur (trie maliyeti dagarcik boyuyla
    // orantili). Katalog TAM katalogdur — kisit yurutucunun kabul ettigini
    // yansitmali.
    let kisit = motor.dagarcik().map(|d| CagriKisiti::yeni(&d, &katalog));
    if kisit.is_none() {
        eprintln!("{}", renk.boya(SARI, "(uyari: motor dagarcigini bildirmiyor — uretim KISITSIZ)"));
    }

    println!(
        "{}",
        renk.boya(CAMGOBEGI, &format!("sirr — motor: {} · araclar: {}", motor.ad(), katalog.araclar().len()))
    );
    if etkilesimli {
        println!("{}", renk.boya(SONUK, "(/yardim komutlar · bos satir ya da /cik cikar)"));
    }
    println!();

    loop {
        let mesaj = match tek_mesaj.clone() {
            Some(m) => m,
            None => {
                if etkilesimli {
                    print!("{}", renk.boya(YESIL, "› "));
                    let _ = std::io::stdout().flush();
                }
                let mut satir = String::new();
                match std::io::stdin().read_line(&mut satir) {
                    Ok(0) => break, // EOF (Ctrl-D)
                    Ok(_) => satir.trim().to_string(),
                    Err(_) => break,
                }
            }
        };

        if mesaj.is_empty() {
            break;
        }

        // Slash komutlari: mesaj olarak modele GITMEZ.
        if mesaj.starts_with('/') {
            match slash(&mesaj, &katalog, &hafiza, &mut gecmis, &motor, &renk) {
                SlashSonuc::Cik => break,
                SlashSonuc::Islendi => {
                    if tek_mesaj.is_some() {
                        break;
                    }
                    continue;
                }
                SlashSonuc::Bilinmeyen => {
                    println!("{}", renk.boya(SONUK, "(bilinmeyen komut; /yardim)"));
                    if tek_mesaj.is_some() {
                        break;
                    }
                    continue;
                }
            }
        }

        // Yeni tur: yan etki bayragi ve deneme sayaci sifirlanir; kirlilik ve
        // depo sifirlanmaz (oturum omurlu).
        let bilet = yurutucu.yeni_tur();
        izler.sifirla();
        if let Some(d) = &kod_durum {
            d.yeni_tur();
        }
        enj_durum.tur_basla();

        // BU TURUN ARAC SONUCLARI. Kalici `gecmis`e tur BITINCE eklenirler.
        //
        // NEDEN AYRI: kullanici mesaji eskiden tur basinda `gecmis`e
        // itiliyordu, ama `Istem::yeni(..., &mesaj)` onu ZATEN `soru` olarak
        // aliyor — sonuc, mesajin isteme IKI KEZ girmesiydi (bir kez
        // `<gecmis>` icinde, bir kez sonda). `--goster-istem` ciktisinda
        // gorulup dogrulandi. Ikili yazim 4096'lik pencerede yer yiyor ve
        // kucuk modele ayni soruyu iki farkli baglamda gostererek soruyu
        // tekrar etmeye tesvik ediyordu.
        let mut tur_araclari: Vec<Tur> = Vec::new();

        // Arac butcesi YALNIZ kullanici mesajindan turer.
        let secili: AracKatalogu = yonlendirici.sec(&mesaj, &katalog).into_iter().collect();
        let secili_adlar: Vec<String> = secili.adlar().into_iter().map(String::from).collect();

        // BECERI ENJEKSIYONU (700 sinir, sistem talimatina GOMULMEZ): mesaja
        // uyan TEK beceri, o turun istemine `<guidance>` citiyle. Tur-mesafeli
        // tekrar engeli `enj_durum` ile: ayni beceri her turda tekrar
        // eklenmez.
        let kilavuz = beceri_depo.eslesen(&mesaj, Some(&secili_adlar)).and_then(|b| {
            if enj_durum.gerekli_mi(&b.ad) {
                enj_durum.isaretle(&b.ad);
                Some(enjeksiyon_metni(b))
            } else {
                None
            }
        });

        // HAFIZA ENJEKSIYONU (600 sinir): mesaja uyan notlar sistem blogunda.
        let hafiza_metni = hafiza.ile(|d| d.enjeksiyon_metni(&mesaj)).flatten();

        let mut cevap = String::new();
        for _ in 0..sirr_eval::EN_FAZLA_TUR {
            // Gecmis = onceki turlar + BU turda kosmus araclarin sonuclari.
            // Soru sonda ayrica durur (bkz. `tur_araclari` yorumu).
            // SORUNUN YERI TURA GORE DEGISIR ve bu, arac dongusunun can alici
            // noktasi:
            //
            // * Ilk tur — henuz arac kosmadi: soru `soru` alaninda, yani istemin
            //   SONUNDA durur (kucuk modelde son blogun agirligi en yuksek).
            // * Sonraki turlar — arac sonucu geldi: soru gecmise, arac
            //   cagrisinin ONUNE tasinir ve `soru` BOS birakilir. Eskiden soru
            //   her turda sonda tekrarlaniyordu; model arac sonucundan hemen
            //   sonra ayni istegi yeniden gorup onu cevaplanmamis sanip araci
            //   tekrar cagiriyordu. Dongu buradan geliyordu.
            let ilk_tur = tur_araclari.is_empty();
            let soru = if ilk_tur { mesaj.as_str() } else { "" };
            let onceki: Vec<Tur> = if ilk_tur {
                gecmis.clone()
            } else {
                gecmis
                    .iter()
                    .cloned()
                    .chain(std::iter::once(Tur::kullanici(&mesaj)))
                    .chain(tur_araclari.iter().cloned())
                    .collect()
            };
            let mut istem = Istem::yeni(SISTEM_TALIMATI, soru).araclarla(&secili).gecmisle(onceki);
            if let Some(k) = &kilavuz {
                istem = istem.kilavuzla(k);
            }
            if let Some(h) = &hafiza_metni {
                istem = istem.hafizayla(h);
            }
            let rapor = sayaci.kirp(&mut istem);
            if rapor.degisti_mi() {
                eprintln!("{}", renk.boya(SONUK, &format!("(baglam kirpildi: {} tur dustu)", rapor.dusen_tur)));
            }
            if goster_istem {
                // SABLON MOTORDAN alinir. Daha once burada `istem.metin()`
                // (yani daima `Sablon::Duz`) basiliyordu: baslikta "sablon:
                // ChatML" yazarken tanilama ciktisi duz metin gosteriyordu ve
                // istem hatalarini gormeyi imkansiz kiliyordu. Tanilama ciktisi
                // modele GERCEKTEN giden telle ayni olmak zorunda.
                let tel = istem.metin_sablonlu(motor.sablon());
                println!("--- ISTEM ({:?}) ---", motor.sablon());
                println!("{tel}");
                println!("--- ~{} belirtec (tahmin) ---", TokenSayaci::tahmin(&tel));
            }

            // AKAN URETIM: model belirtec urettikce ekrana dokulur. 3B modelde
            // ilk belirtece kadar gecen saniyelerde kullanici donmus ekrana
            // bakmasin. Akan metin dogrudan uretilen metindir (halusinasyon
            // degil); cip metnini yine ARAC uretecek.
            let akiyor = std::sync::atomic::AtomicBool::new(false);
            if etkilesimli {
                print!("{}", renk.boya(SONUK, "sirr> "));
                let _ = std::io::stdout().flush();
            }
            // Akis YALNIZ etkilesimli kipte ekrana dokulur. Tek-mesaj/tanilama
            // kipinde cevap sonda tek sefer basilir; akitmak ciktiyi
            // ikilerdi (akan metin + "sirr:" satiri).
            let dinleyici = |parca: &str| {
                if etkilesimli {
                    akiyor.store(true, std::sync::atomic::Ordering::Relaxed);
                    print!("{parca}");
                    let _ = std::io::stdout().flush();
                }
            };
            let uretim = match bekle(motor.uret_akan(
                &istem,
                kisit.as_ref().map(|k| k as &dyn sirr_motor::Kisitlayici),
                OrneklemeAyari::default(),
                &dinleyici,
            )) {
                Ok(u) => u,
                Err(e) => {
                    eprintln!("\nmotor hatasi: {e}");
                    break;
                }
            };
            if akiyor.load(std::sync::atomic::Ordering::Relaxed) {
                println!();
            }

            if !uretim.bitis.tam_mi() {
                eprintln!("{}", renk.boya(SARI, "(uretim yarim kesildi)"));
                break;
            }

            let Some(sonuc) = bekle(yurutucu.ham_yurut(&uretim.metin, bilet, &mut ctx)) else {
                cevap = uretim.metin;
                break;
            };
            let hata_mi = sonuc.hata_mi();
            let tekrar_denenebilir = sonuc.tekrar_denenebilir;

            // MODEL KENDI CAGRISINI GECMISTE GORMELI. Eskiden yalniz arac
            // SONUCU geri besleniyordu; model bir sonraki turda baglamsiz bir
            // sonuc satiri ile hemen altinda TEKRARLANAN kullanici sorusunu
            // goruyor, soruyu cevaplanmamis sanip ayni araci yeniden cagiriyordu
            // — tur sinirina kadar, kullaniciya hic cevap vermeden. Cagriyi
            // `assistant` turu olarak yazmak "bunu ben istedim, sonucu da geldi"
            // baglamini kuran sey.
            tur_araclari.push(Tur::asistan(uretim.metin.trim()));
            tur_araclari.push(Tur::arac(sonuc.modele_donen.clone()));

            // YAN ETKIDEN SONRA KURTARMA TURU YOK (yalniz hata + geri
            // alinamaz yan etki kesisiminde).
            if hata_mi && !tekrar_denenebilir {
                eprintln!("{}", renk.boya(SARI, "(yan etki gerceklesti — kurtarma turu acilmadi)"));
                cevap = "Islem kismen gerceklesti; tekrar denemedim.".to_string();
                break;
            }
        }

        // Cipler: sirr yaptigini gizlemez.
        for iz in izler.izler() {
            if !iz.metin.trim().is_empty() {
                println!("  {}", renk.boya(SONUK, &format!("[{}] {} · {:?}", iz.ikon, iz.metin, iz.durum)));
            }
        }
        // KALICI GECMIS tur bitince yazilir ve SIRASI anlamlidir:
        // kullanici -> arac sonuclari -> asistan.
        gecmis.push(Tur::kullanici(&mesaj));
        gecmis.append(&mut tur_araclari);
        if !cevap.is_empty() {
            gecmis.push(Tur::asistan(&cevap));
            // Etkilesimli kipte cevap zaten akarken basildi; tekrar basma.
            if !etkilesimli {
                println!("sirr: {cevap}");
            }
        }
        println!();
        let _ = std::io::stdout().flush();

        if tek_mesaj.is_some() {
            break;
        }
    }

    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// Slash komutlari
// ---------------------------------------------------------------------------

enum SlashSonuc {
    Cik,
    Islendi,
    Bilinmeyen,
}

fn slash(
    komut: &str,
    katalog: &AracKatalogu,
    hafiza: &PaylasimliHafiza,
    gecmis: &mut Vec<Tur>,
    motor: &Arc<dyn MotorSaglayici>,
    renk: &Renk,
) -> SlashSonuc {
    let ad = komut.split_whitespace().next().unwrap_or("");
    match ad {
        "/cik" | "/quit" | "/exit" => SlashSonuc::Cik,
        "/yardim" | "/help" => {
            println!("{}", renk.boya(CAMGOBEGI, "komutlar:"));
            for (k, a) in [
                ("/yardim", "bu liste"),
                ("/araclar", "kataloktaki araclar"),
                ("/hafiza", "kayitli hafiza notlari"),
                ("/gecmis", "bu oturumun konusma gecmisi"),
                ("/model", "aktif motor"),
                ("/temizle", "konusma gecmisini sil (hafiza kalir)"),
                ("/cik", "cikis"),
            ] {
                println!("  {:10} {}", k, renk.boya(SONUK, a));
            }
            SlashSonuc::Islendi
        }
        "/araclar" => {
            for arac in katalog.araclar() {
                let etiket = if arac.kirletir_mi() { renk.boya(SARI, " [kirletir]") } else { String::new() };
                println!("  {}{}", arac.ad(), etiket);
                println!("    {}", renk.boya(SONUK, arac.aciklama()));
            }
            SlashSonuc::Islendi
        }
        "/hafiza" => {
            let cikti = hafiza.ile(|d| {
                if d.sayi() == 0 {
                    "(hafiza bos)".to_string()
                } else {
                    d.notlar()
                        .iter()
                        .map(|n| format!("  · {}", n.metin))
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            });
            println!("{}", cikti.unwrap_or_else(|| "(hafiza okunamadi)".into()));
            SlashSonuc::Islendi
        }
        "/gecmis" => {
            if gecmis.is_empty() {
                println!("{}", renk.boya(SONUK, "(gecmis bos)"));
            }
            for t in gecmis.iter() {
                println!("  {:?}: {}", t.rol, t.metin.chars().take(80).collect::<String>());
            }
            SlashSonuc::Islendi
        }
        "/model" => {
            println!("  motor: {}", motor.ad());
            println!("  sablon: {:?}", motor.sablon());
            println!("  kisit: {}", if motor.dagarcik().is_some() { "acik" } else { "kapali" });
            SlashSonuc::Islendi
        }
        "/temizle" => {
            gecmis.clear();
            println!("{}", renk.boya(SONUK, "(gecmis silindi)"));
            SlashSonuc::Islendi
        }
        _ => SlashSonuc::Bilinmeyen,
    }
}

/// MCP yuklemesini kullaniciya bildirir — hicbiri sessizce yutulmaz.
fn mcp_raporla(yukleme: &mcp::YuklemeSonucu, renk: &Renk) {
    for (baglanti, neden) in &yukleme.baglanti_hatalari {
        eprintln!("{}", renk.boya(SARI, &format!("(mcp: '{baglanti}' baglantisi kurulamadi — {neden})")));
    }
    for atlanan in &yukleme.atlananlar {
        eprintln!(
            "{}",
            renk.boya(SONUK, &format!("(mcp: '{}' aracı atlandı [{}] — {})", atlanan.uzak_ad, atlanan.baglanti, atlanan.sebep))
        );
    }
    for not in &yukleme.notlar {
        eprintln!("{}", renk.boya(SONUK, &format!("(mcp: {not})")));
    }
    if !yukleme.araclar.is_empty() {
        eprintln!("{}", renk.boya(SONUK, &format!("(mcp: {} uzak araç kataloğa eklendi)", yukleme.araclar.len())));
    }
}

// ---------------------------------------------------------------------------
// candle motor kurulumu
// ---------------------------------------------------------------------------

#[cfg(feature = "candle")]
fn candle_motor_yoldan(model: &str, belirtecleyici: &str) -> Result<Arc<dyn MotorSaglayici>, String> {
    let ayar = sirr_motor::ModelAyari::yeni(model, belirtecleyici);
    // Dosya varligi 2.5 GB'lik yuklemeden ONCE denetlenir; eksik dosyayi o
    // surenin sonunda ogrenmek gereksiz bir bekleyis.
    sirr_motor::CandleMotor::dosyalar_var_mi(&ayar).map_err(|e| e.to_string())?;
    let motor = sirr_motor::CandleMotor::yukle(&ayar).map_err(|e| e.to_string())?;
    // Hangi MIMARININ yuklendigi basilir. Sessiz kalsaydi yanlis sablonla
    // kosan bir model "tuhaf cevap veriyor" diye gorunur, teshisi zor olurdu.
    eprintln!("(mimari: {}, sablon: {:?})", motor.mimari().adi(), motor.mimari().sablon());
    Ok(Arc::new(motor) as Arc<dyn MotorSaglayici>)
}

#[cfg(not(feature = "candle"))]
fn candle_motor_yoldan(_model: &str, _belirtecleyici: &str) -> Result<Arc<dyn MotorSaglayici>, String> {
    Err("bu ikili `candle` ozelligi olmadan derlendi".into())
}

// ---------------------------------------------------------------------------
// eval
// ---------------------------------------------------------------------------

fn eval(json: bool, esik: f64) -> ExitCode {
    let rapor = sirr_eval::kos(&sirr_eval::hepsi(), &SahteSecici);
    if json {
        println!("{}", rapor.json());
    } else {
        print!("{}", rapor.tablo());
    }
    if rapor.basari_orani + f64::EPSILON >= esik {
        ExitCode::SUCCESS
    } else {
        eprintln!("esik tutmadi: {:.3} < {esik:.3}", rapor.basari_orani);
        ExitCode::FAILURE
    }
}

// ---------------------------------------------------------------------------
// araclar
// ---------------------------------------------------------------------------

fn araclar(sema_bas: bool) -> ExitCode {
    let renk = Renk::kur();
    let depo = Arc::new(PaylasimliDepo::yeni());
    let hafiza = PaylasimliHafiza::bellekte();
    let (mut katalog, _) = oturum_katalogu(&depo, &hafiza, &renk);
    // MCP araclari BURADA DA gorunmeli: bu komut "istemde ne yaziyor"un birebir
    // kaynagi; sohbetin gordugu katalogtan farkli bir sey basmamali.
    let mcp_yukleme = mcp::varsayilandan_yukle();
    let _ = mcp::katalogu_besle(&mut katalog, &mcp_yukleme);
    mcp_raporla(&mcp_yukleme, &renk);
    for arac in katalog.araclar() {
        println!("{}{}", arac.ad(), if arac.kirletir_mi() { "  [kirletir]" } else { "" });
        println!("  {}", arac.aciklama());
        if sema_bas {
            println!("  args: {}", arac.sema().json_schema());
        } else {
            for alan in arac.sema().alanlar() {
                println!(
                    "  - {}{}: {}",
                    alan.ad,
                    if alan.zorunlu { "*" } else { "" },
                    tip_metni(&alan.sema)
                );
            }
        }
        println!();
    }
    println!("{} arac", katalog.araclar().len());
    ExitCode::SUCCESS
}

fn tip_metni(sema: &ArgSema) -> String {
    use sirr_cekirdek::SemaTipi::*;
    match &sema.tip {
        Nesne { alanlar } => format!("nesne({} alan)", alanlar.len()),
        Dizi { .. } => "dizi".into(),
        Metin { .. } => "metin".into(),
        Secenek { secenekler } => format!("secenek[{}]", secenekler.join("|")),
        Sayi { tam_mi, .. } => if *tam_mi { "tamsayi" } else { "sayi" }.into(),
        Bool => "bool".into(),
    }
}

// ---------------------------------------------------------------------------
// gramer
// ---------------------------------------------------------------------------

fn gramer(ad: &str, dene: Option<&str>) -> ExitCode {
    let renk = Renk::kur();
    let depo = Arc::new(PaylasimliDepo::yeni());
    let hafiza = PaylasimliHafiza::bellekte();
    let (katalog, _) = oturum_katalogu(&depo, &hafiza, &renk);
    let Some(arac) = katalog.bul(ad) else {
        eprintln!("bilinmeyen arac: {ad}\nkatalog: {}", katalog.adlar().join(", "));
        return ExitCode::FAILURE;
    };

    let sema = arac.sema();
    let gramer = Gramer::derle(&sema);
    println!("arac : {ad}");
    println!("sema : {}", sema.json_schema());

    let durum = gramer.durum();
    let izin = durum.izinli_onekler();
    let karakterler: String = izin.karakterler().collect();
    println!("\nbaslangic izinli karakterler: {karakterler:?}");
    println!("metin govdesi acik : {}", izin.metin_govdesi_mi());
    println!("bosluk serbest     : {}", izin.bosluk_serbest_mi());
    println!("burada bitebilir   : {}", izin.bitebilir());

    if let Some(ornek) = dene {
        let mut d = gramer.durum();
        println!("\ndeneme : {ornek}");
        match d.ilerlet(ornek) {
            Ok(()) => {
                println!("kabul  : {}", if d.bitti_mi() { "evet (tamamlandi)" } else { "kismi" });
                let kalan: String = d.izinli_onekler().karakterler().collect();
                println!("devami : {kalan:?}");
            }
            Err(e) => {
                println!("RET    : {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    ExitCode::SUCCESS
}
