//! sirr-cli — headless gelistirici kabugu.
//!
//! NEDEN VAR: sirr'in mantik katmani (yonlendirme, istem kurulumu, gramer,
//! arac yurutme, bypass kanali) bir iOS uygulamasinin icinde saklandiginda
//! yalniz simulator acilarak gozlenebilir. Bu ikili ayni katmani terminalden
//! surer: eval CI'da koser, `gramer` komutu bir semanin modele NE DAYATTIGINI
//! gosterir, `araclar` komutu istemdeki tarifin birebir kaynagini basar.
//!
//! VARSAYILAN MOTOR SAHTE. `--motor candle` gercek cikarimi acar ve bu crate'i
//! `candle` ozelligiyle derlemeyi gerektirir; varsayilan derleme agirlik
//! agacini hic cekmez.
//!
//! AG YOK: hicbir komut ag cagrisi yapmaz.

use clap::{Parser, Subcommand, ValueEnum};
use sirr_araclar::belge_olustur::BelgeOlusturAraci;
use sirr_araclar::belge_oku::BelgeOkuAraci;
use sirr_araclar::hesap::HesapAraci;
use sirr_araclar::veri_deposu::PaylasimliDepo;
use sirr_araclar::yonlendirici::Yonlendirici;
use sirr_araclar::yurutucu::AracYurutucu;
use sirr_araclar::zaman::ZamanAraci;
use sirr_cekirdek::{
    AracBaglami, AracKatalogu, ArgSema, IzToplayici, Raporlayici,
    VeriDeposu as CekirdekVeriDeposu,
};
use sirr_eval::{SISTEM_TALIMATI, SahteSecici};
use sirr_gramer::{CagriKisiti, Gramer};
use sirr_motor::{Istem, MotorSaglayici, OrneklemeAyari, SahteMotor, TokenSayaci, Tur, bekle};
use std::io::{BufRead, Write};
use std::process::ExitCode;
use std::sync::Arc;

/// Eval'in "yesil" sayildigi esik. Yuzde 100: bu vakalarin hepsi deterministik
/// bir mantik iddiasi, birinin kirilmasi regresyondur. Gercek motorla
/// kosuldugunda esik `--esik` ile bilerek dusurulur.
const VARSAYILAN_ESIK: f64 = 1.0;

/// Dis dunyaya veri cikarabilen araclarin adlari — onay kapisinin girdisi.
///
/// BU TURDA BOS, ve bos olmasi dogru: katalogda (hesap, zaman, belge_oku,
/// belge_olustur) cihazdan veri cikaran tek bir arac yok. Sabitin yine de var
/// olmasinin sebebi, kapinin BAGLI olmasi: liste burada durdugu ve asagida
/// gercekten uygulandigi icin, ilk gercek dis arac eklendiginde yapilacak tek
/// sey adini buraya yazmaktir — kapiyi o gun ilk kez kesfetmek degil.
///
/// TODO(web/MCP turu): web arama ve MCP araclari bu tura dahil DEGIL (bkz.
/// README "Kapsam disi"). Eklendiklerinde adlari buraya girmeli; kapinin
/// mekanizmasi `sirr-eval`de `disari_gonder` ile halihazirda olculuyor.
const DIS_ARACLAR: &[&str] = &[];

#[derive(Parser)]
#[command(name = "sirr", about = "sirr — cihaz-ustu asistanin headless kabugu")]
struct Kabuk {
    #[command(subcommand)]
    komut: Komut,
}

#[derive(Subcommand)]
enum Komut {
    /// Etkilesimli tur: stdin'den mesaj okur, arac dongusunu surer.
    Sohbet {
        #[arg(long, value_enum, default_value_t = MotorSecimi::Sahte)]
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
        /// Tek mesaj kosup cikar (betikli tanilama icin).
        #[arg(long)]
        mesaj: Option<String>,
    },
    /// Eval setini kosar; cikis kodu basari oranina bagli.
    Eval {
        /// JSON rapor bas (metin tablosu yerine).
        #[arg(long)]
        json: bool,
        /// Gecme esigi, 0.0 - 1.0.
        #[arg(long, default_value_t = VARSAYILAN_ESIK)]
        esik: f64,
    },
    /// Katalogu ve semalarini listeler.
    Araclar {
        /// Semayi JSON Schema olarak bas (istemde gorunen bicim).
        #[arg(long)]
        sema: bool,
    },
    /// Bir aracin semasindan uretilen grameri gosterir (tanilama).
    Gramer {
        #[arg(long)]
        arac: String,
        /// Gramere karsi denenecek ornek JSON — kabul/ret gosterilir.
        #[arg(long)]
        dene: Option<String>,
    },
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum MotorSecimi {
    Sahte,
    Candle,
}

fn main() -> ExitCode {
    match Kabuk::parse().komut {
        Komut::Sohbet { motor, betik, goster_istem, dizin, mesaj } => {
            sohbet(motor, betik, goster_istem, &dizin, mesaj)
        }
        Komut::Eval { json, esik } => eval(json, esik),
        Komut::Araclar { sema } => araclar(sema),
        Komut::Gramer { arac, dene } => gramer(&arac, dene.as_deref()),
    }
}

// ---------------------------------------------------------------------------
// Katalog
// ---------------------------------------------------------------------------

/// Kabugun gordugu katalog. TEK dogruluk kaynagi: istem, gramer ve yurutme
/// hepsi bundan turer; ikinci bir "arac adlari" listesi yok.
fn katalog(depo: &Arc<PaylasimliDepo>) -> AracKatalogu {
    let mut k = AracKatalogu::yeni();
    k.ekle(Arc::new(HesapAraci))
        .ekle(Arc::new(ZamanAraci::yeni()))
        .ekle(Arc::new(BelgeOkuAraci::depo_ile(Arc::clone(depo))))
        .ekle(Arc::new(BelgeOlusturAraci::yeni()));
    k
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
) -> ExitCode {
    let motor: Arc<dyn MotorSaglayici> = match secim {
        MotorSecimi::Sahte => {
            Arc::new(SahteMotor::betik(betik).varsayilanla("Anladim. (sahte motor)"))
        }
        MotorSecimi::Candle => match candle_motor() {
            Ok(m) => m,
            Err(e) => {
                eprintln!("hata: {e}");
                return ExitCode::FAILURE;
            }
        },
    };

    let depo = Arc::new(PaylasimliDepo::yeni());
    let katalog = katalog(&depo);
    // ONAY KAPISI URETIMDE DE KURULU.
    //
    // Onceden `dis_arac(...)` YALNIZCA eval'de cagriliyordu; `dis_araclar`
    // kumesi uretimde bos kaldigi icin yurutucudeki kapi kosulu
    // (`dis_araclar.contains(ad) && oturum_kirli()`) `sirr sohbet` yolunda
    // hicbir zaman true olamiyordu. Kapi ve `IzinGerekli` dali fiilen
    // ulasilamaz koddu.
    //
    // Isaretleme neden burada: "disari cikarir mi" bir aracin kendi ozelligi
    // degil DAGITIMIN karari (ayni arac cevrimdisi kurulumda disari cikmaz),
    // bu yuzden cekirdek `Arac` sozlesmesinde boyle bir bayrak YOK. Liste
    // kabugun kendi bildirimi olmak zorunda.
    let mut yurutucu = AracYurutucu::yeni(katalog.clone());
    for ad in DIS_ARACLAR {
        yurutucu = yurutucu.dis_arac(*ad);
    }
    let izler = Arc::new(IzToplayici::yeni());
    let mut ctx = AracBaglami::yeni(
        Arc::clone(&depo) as Arc<dyn CekirdekVeriDeposu>,
        dizin,
        Arc::clone(&izler) as Arc<dyn Raporlayici>,
    );

    let yonlendirici = Yonlendirici::yeni();
    let sayaci = TokenSayaci::default();
    let mut gecmis: Vec<Tur> = Vec::new();

    // KISIT BURADA, DONGUNUN DISINDA KURULUR. `TokenMaskesi` dagarcigi bir
    // onek agacina cevirir; bu maliyet dagarcik boyuyla dogru orantilidir ve
    // gercek bir belirtecleyicide (~32k) her turda odenmesi anlamsizdir. Trie
    // yalniz token METINLERINE bakar, gramerden bagimsizdir — bir kez kurulup
    // tum uretim boyunca paylasilabilir.
    //
    // Katalog TAM katalogdur, yonlendiricinin darattigi `secili` degil: kisit
    // yurutucunun kabul ettigi seyi yansitmali, isteme yazilani degil.
    let kisit = motor.dagarcik().map(|d| CagriKisiti::yeni(&d, &katalog));
    if kisit.is_none() {
        eprintln!("(uyari: motor dagarcigini bildirmiyor — uretim KISITSIZ)");
    }

    println!("sirr — motor: {} · araclar: {}", motor.ad(), katalog.adlar().join(", "));
    println!("(bos satir ya da Ctrl-D cikar)\n");

    let girdiler: Box<dyn Iterator<Item = String>> = match tek_mesaj {
        Some(m) => Box::new(std::iter::once(m)),
        None => Box::new(std::io::stdin().lock().lines().map_while(Result::ok)),
    };

    for satir in girdiler {
        let mesaj = satir.trim().to_string();
        if mesaj.is_empty() {
            break;
        }

        // Her mesaj GERCEK bir yeni tur: yan etki bayragi burada sifirlanir,
        // kirlilik ve depo sifirlanmaz (oturum omurlu).
        let bilet = yurutucu.yeni_tur();
        izler.sifirla();
        gecmis.push(Tur::kullanici(&mesaj));

        // Arac butcesi yalniz KULLANICI MESAJINDAN turer: gecmis buyudukce
        // secim degismemeli, yoksa ayni soru iki oturumda farkli davranir.
        let secili: AracKatalogu = yonlendirici.sec(&mesaj, &katalog).into_iter().collect();

        let mut cevap = String::new();
        for _ in 0..sirr_eval::EN_FAZLA_TUR {
            let mut istem = Istem::yeni(SISTEM_TALIMATI, &mesaj)
                .araclarla(&secili)
                .gecmisle(gecmis.clone());
            let rapor = sayaci.kirp(&mut istem);
            if rapor.degisti_mi() {
                eprintln!("(baglam kirpildi: {} tur dustu)", rapor.dusen_tur);
            }
            if goster_istem {
                println!("--- ISTEM ---\n{}\n-------------", istem.metin());
            }

            let uretim = match bekle(motor.uret(
                &istem,
                kisit.as_ref().map(|k| k as &dyn sirr_motor::Kisitlayici),
                OrneklemeAyari::default(),
            )) {
                Ok(u) => u,
                Err(e) => {
                    eprintln!("motor hatasi: {e}");
                    break;
                }
            };
            // Yarim cikti AYRISTIRILMAZ: belirtec tavanina carpmis bir arac
            // cagrisi yarim JSON'dur, ayristirmaya kalkmak uydurma arguman uretir.
            if !uretim.bitis.tam_mi() {
                eprintln!("(uretim yarim kesildi)");
                break;
            }

            let Some(sonuc) = bekle(yurutucu.ham_yurut(&uretim.metin, bilet, &mut ctx)) else {
                cevap = uretim.metin;
                break;
            };
            let hata_mi = sonuc.hata_mi();
            let tekrar_denenebilir = sonuc.tekrar_denenebilir;
            gecmis.push(Tur::arac(sonuc.modele_donen.clone()));

            // YAN ETKIDEN SONRA KURTARMA TURU YOK.
            //
            // Bu bayrak yedi yerde YAZILIP hicbir yerde OKUNMUYORDU: dongu
            // onu hic sormadan bir sonraki tura geciyordu, yani "dunya
            // degistiyse ayni istemi tekrarlama" karari kodda duruyor ama
            // uygulanmiyordu. Swift tarafinda yakalanan "7 yazma 0 okuma"
            // deseninin birebir tekrariydi.
            //
            // Somut zarar: `belge_olustur` dosyayi yazip ARDINDAN bir hata
            // dondugunde model ayni cagriyi tekrarlar ve IKINCI bir dosya
            // olusurdu. Basarili cagrilarda dongu zaten devam etmeli — kapi
            // yalniz hata + geri alinamaz yan etki kesisiminde iner.
            if hata_mi && !tekrar_denenebilir {
                eprintln!("(yan etki gerceklesti — kurtarma turu acilmadi)");
                cevap = "Islem kismen gerceklesti; tekrar denemedim.".to_string();
                break;
            }
        }

        // Cipler: sirr yaptigini gizlemez.
        for iz in izler.izler() {
            if !iz.metin.trim().is_empty() {
                println!("  [{}] {} · {:?}", iz.ikon, iz.metin, iz.durum);
            }
        }
        if !cevap.is_empty() {
            println!("sirr: {cevap}\n");
            gecmis.push(Tur::asistan(cevap));
        }
        let _ = std::io::stdout().flush();
    }

    ExitCode::SUCCESS
}

#[cfg(feature = "candle")]
fn candle_motor() -> Result<Arc<dyn MotorSaglayici>, String> {
    // Yollar ORTAMDAN alinir: ag tekeli geregi hicbir sey indirilmez.
    let model = std::env::var("SIRR_MODEL")
        .map_err(|_| "SIRR_MODEL ayarlanmali (yerel .gguf yolu)".to_string())?;
    let belirtecleyici = std::env::var("SIRR_TOKENIZER")
        .map_err(|_| "SIRR_TOKENIZER ayarlanmali (yerel tokenizer.json yolu)".to_string())?;
    let ayar = sirr_motor::ModelAyari::yeni(model, belirtecleyici);
    sirr_motor::CandleMotor::yukle(&ayar)
        .map(|m| Arc::new(m) as Arc<dyn MotorSaglayici>)
        .map_err(|e| e.to_string())
}

#[cfg(not(feature = "candle"))]
fn candle_motor() -> Result<Arc<dyn MotorSaglayici>, String> {
    Err("bu ikili `candle` ozelligi olmadan derlendi; \
         `cargo build -p sirr-cli --features candle` ile derleyin"
        .into())
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
    // Cikis kodu esige bagli: CI'nin okudugu TEK sinyal budur.
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
    let depo = Arc::new(PaylasimliDepo::yeni());
    let katalog = katalog(&depo);
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
    let depo = Arc::new(PaylasimliDepo::yeni());
    let katalog = katalog(&depo);
    let Some(arac) = katalog.bul(ad) else {
        eprintln!("bilinmeyen arac: {ad}\nkatalog: {}", katalog.adlar().join(", "));
        return ExitCode::FAILURE;
    };

    let sema = arac.sema();
    let gramer = Gramer::derle(&sema);
    println!("arac : {ad}");
    println!("sema : {}", sema.json_schema());

    // Baslangic durumunda URETILEBILEN karakterler: gramerin modele ne
    // dayattigini gosteren en dogrudan sey.
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
