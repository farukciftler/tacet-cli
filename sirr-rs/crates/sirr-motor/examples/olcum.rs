//! Gercek model olcum kosusu — `--features candle` ile derlenir.
//!
//! NEDEN ORNEK, NEDEN TEST DEGIL: bu kosu 2 GB'lik bir agirlik dosyasi ister
//! ve dakikalar surer. `cargo test` icine konsaydi ya CI'da model yoklugundan
//! kirmizi yanardi ya da atlanmis (`ignore`) bir test olarak yesillik
//! yanilsamasi uretirdi. Ornek olarak durunca kosturmak BILINCLI bir eylemdir.
//!
//! Kosum:
//!   SIRR_MODEL=... SIRR_TOKENIZER=... \
//!     cargo run -p sirr-motor --features candle --release --example olcum

use sirr_motor::istem::Sablon;
use sirr_motor::{
    BitisNedeni, Istem, MotorSaglayici, OrneklemeAyari, TokenSayaci, Tur, Uretim, bekle,
};
use std::time::Instant;

fn main() {
    let Ok(model) = std::env::var("SIRR_MODEL") else {
        eprintln!("SIRR_MODEL ayarlanmali");
        std::process::exit(2);
    };
    let Ok(belirtecleyici) = std::env::var("SIRR_TOKENIZER") else {
        eprintln!("SIRR_TOKENIZER ayarlanmali");
        std::process::exit(2);
    };

    // Aygit ORTAMDAN secilir; varsayilan islemci. `metal` istendiginde
    // crate'in `metal` ozelligi de acik olmali — yoksa acikca hata verir,
    // sessizce islemciye DUSMEZ (bkz. `Aygit::Metal`).
    let aygit = match std::env::var("SIRR_AYGIT").as_deref() {
        Ok("metal") => sirr_motor::candle_motor::Aygit::Metal,
        _ => sirr_motor::candle_motor::Aygit::Islemci,
    };
    let ayar = sirr_motor::ModelAyari::yeni(&model, &belirtecleyici).aygitla(aygit);
    println!("aygit istendi : {aygit:?}");

    // --- 1. Yukleme suresi ---
    let t = Instant::now();
    let motor = match sirr_motor::CandleMotor::yukle(&ayar) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("yukleme basarisiz: {e}");
            std::process::exit(1);
        }
    };
    let yukleme = t.elapsed();
    println!("== YUKLEME ==");
    println!("sure          : {:.2} sn", yukleme.as_secs_f64());
    println!("sablon        : {:?}", motor.sablon());

    // --- 2. Dagarcik: `dagarcik_kur` gercek belirtecleyiciyle olculuyor ---
    // `dagarcik_kur` MALIYETI ayrica olculuyor: 32k belirtecin tek tek
    // `decode` edilmesi yukleme icinde gorunmez ama ucuz olmayabilir.
    let t = Instant::now();
    let _ = sirr_motor::candle_motor::dagarcik_kur(motor.belirtecleyici());
    let kurma_suresi = t.elapsed();

    let t = Instant::now();
    let dagarcik = motor.dagarcik().expect("candle motoru dagarcik bildirir");
    let dagarcik_suresi = t.elapsed();
    println!("\n== dagarcik_kur MALIYETI ==");
    println!("kurma         : {:.1} ms", kurma_suresi.as_secs_f64() * 1000.0);
    let bos = dagarcik.iter().filter(|s| s.is_empty()).count();
    println!("\n== DAGARCIK (dagarcik_kur) ==");
    println!("boy           : {}", dagarcik.len());
    println!("klonlama      : {:.1} ms", dagarcik_suresi.as_secs_f64() * 1000.0);
    println!("bos metinli   : {bos}  (ozel/kontrol belirtecleri)");
    // Bu ornekler kritigi: gramer karakter bazinda calisir, yani burada
    // `Ġ` ya da `▁` gorunuyorsa maske bastan yanlis kurulmus demektir.
    for id in [15u32, 264, 5867, 314, 341, 8, 9, 314] {
        if let Some(s) = dagarcik.get(id as usize) {
            println!("  id {id:<6} -> {s:?}");
        }
    }
    let isaretli = dagarcik.iter().filter(|s| s.contains('Ġ') || s.contains('▁')).count();
    println!("BPE isaretli  : {isaretli}  (0 olmali — decode isaretleri cozmeli)");

    // Birlesik belirtecler gercekten var mi (DURUM.md'deki risk)?
    println!("\n== BIRLESIK BELIRTECLER (gramer sinirini asan adaylar) ==");
    for aday in ["\"})", "\"}", "})", "\":", "\": \"", "\",", "\":\"", "({", "()"] {
        let bulundu: Vec<usize> = dagarcik
            .iter()
            .enumerate()
            .filter(|(_, s)| s.as_str() == aday)
            .map(|(i, _)| i)
            .collect();
        println!("  {aday:<8?} -> {bulundu:?}");
    }

    // --- 3. Bitis belirteci gercekten bulundu mu ---
    println!("\n== BITIS BELIRTECLERI ==");
    println!("{:?}", motor.bitis_belirtecleri());

    // --- 4. Uretim hizi ---
    // Basarim istemi UZUN cevap ister: kisa cevap bitis belirteciyle erkenden
    // durur ve fark yontemi (asagida) uygulanamaz.
    let istem = Istem::yeni(
        sirr_motor::SISTEM_TALIMATI,
        "Bir sehir gezisini butun ayrintilariyla, uzun uzun anlat.",
    );
    println!("\n== ISTEM, motorun sablonuyla (ilk 320 karakter) ==");
    // Sablon MOTORDAN alinir, sabit yazilmaz: cok-mimarili yuklemede
    // (Qwen -> ChatML, Gemma -> Gemma) sabit ChatML, Gemma'ya YANLIS citler
    // besler ve olcum gercek uretimi degil bozuk bir kosuyu olcerdi.
    let tel = istem.metin_sablonlu(motor.sablon());
    println!("{}", &tel[..tel.len().min(320)]);
    let istem_boyu = motor
        .belirtecleyici()
        .encode(tel.as_str(), true)
        .map(|e| e.get_ids().len())
        .unwrap_or(0);

    // ONYUKLEME (prefill) ile COZME (decode) AYRI OLCULUR. Tek bir
    // "belirtec/saniye" sayisi yaniltir: kisa cevaplarda sure neredeyse
    // tamamen istemin tek seferlik islenmesine gider ve motor oldugundan cok
    // daha yavas gorunur. Ayni istemle N ve 2N belirtec uretip FARKI almak
    // onyuklemeyi sadelestirir; kalan saf cozme hizidir.
    let olc = |n: usize| -> (f64, Uretim) {
        let t = Instant::now();
        let u = bekle(motor.uret(
            &istem,
            None,
            OrneklemeAyari { en_cok_belirtec: n, ..Default::default() },
        ))
        .expect("uretim");
        (t.elapsed().as_secs_f64(), u)
    };

    // ISINMA KOSUSU — olcume KATILMAZ. GGUF agirliklari diskten tembel
    // sayfalanir; ilk uretim 2 GB'lik sayfa-iceri maliyetini tek basina oder.
    // Isinma olmadan o maliyet "onyukleme" hanesine yaziliyor ve sacma bir
    // sonuc uretiyordu: onyukleme belirtec basina cozmeden YAVAS gorunuyordu,
    // oysa onyukleme toplu islenir ve hep daha hizlidir.
    let _ = olc(8);

    let (t1, u1) = olc(32);
    let (t2, u2) = olc(64);
    // Fark ancak IKI kosu da tavana carptiysa anlamli. Greedy uretim
    // tekrarlanabilir oldugu icin ikisi ayni metni yazar; biri bitis
    // belirteciyle erken durursa fark "0 belirtec / negatif sure" olur ve
    // olcum sessizce sacmalar. Bu yuzden kosul acikca kontrol edilir.
    let tavana_carpti =
        u1.bitis == BitisNedeni::Uzunluk && u2.bitis == BitisNedeni::Uzunluk;
    let cozme = if tavana_carpti && u2.belirtec_sayisi > u1.belirtec_sayisi {
        (u2.belirtec_sayisi - u1.belirtec_sayisi) as f64 / (t2 - t1)
    } else {
        f64::NAN
    };
    let onyukleme = t1 - u1.belirtec_sayisi as f64 / cozme;
    if !tavana_carpti {
        println!(
            "(not: uretim tavandan once bitti — fark yontemi uygulanamadi; \
             yalniz uctan uca hiz anlamli)"
        );
    }

    println!("\n== BASARIM ==");
    println!("istem belirteci   : {}", istem_boyu);
    println!("onyukleme (prefill): {onyukleme:.2} sn  ({:.1} belirtec/sn)", istem_boyu as f64 / onyukleme);
    println!("cozme (decode)     : {cozme:.2} belirtec/sn");
    println!("32 belirtec toplam : {t1:.2} sn");
    println!("64 belirtec toplam : {t2:.2} sn");

    // --- 5. Baglam butcesi kirpmasi GERCEK belirtecleyiciyle ---
    //
    // Tahmin (`TokenSayaci`) karakter basina sabit bir orana dayanir; burada
    // kirpilmis istemi GERCEKTEN belirtecleyip tavanin altinda kalip
    // kalmadigina bakiyoruz. Tahmin AZ cikarsa model istemi ortasindan
    // kesilir — sessiz ve teshis edilemez bir ariza.
    println!("\n== BAGLAM BUTCESI KIRPMASI ==");
    let sayac = TokenSayaci::default();
    let mut uzun = Istem::yeni(sirr_motor::SISTEM_TALIMATI, "Ozetle: son durum ne?")
        .gecmisle((0..200).map(|i| {
            Tur::kullanici(format!(
                "{i}. tur: cihaz uzerinde calisan asistanin baglam penceresini \
                 doldurmak icin yazilmis uzunca bir Turkce cumle."
            ))
        }));
    println!("kirpma oncesi tahmin : {}", sayac.istem_tahmini(&uzun));
    let rapor = sayac.kirp(&mut uzun);
    println!("dusen tur            : {}", rapor.dusen_tur);
    println!("kilavuz dustu        : {}", rapor.kilavuz_dustu);
    println!("soru kirpildi        : {}", rapor.soru_kirpildi);
    println!("kirpma sonrasi tahmin: {}", rapor.son_tahmin);
    println!("istem tavani         : {}", sayac.istem_tavani());
    println!("dogrula()            : {:?}", sayac.dogrula(&uzun).is_ok());

    // Iki sablonun GERCEK belirtec sayisi — tahmin duz bicim uzerinden
    // yapiliyor, uretim ise ChatML gonderiyor. Fark buradan gorunur.
    let gercek = |s: Sablon| -> usize {
        motor
            .belirtecleyici()
            .encode(uzun.metin_sablonlu(s), true)
            .map(|e| e.get_ids().len())
            .unwrap_or(0)
    };
    let duz_gercek = gercek(Sablon::Duz);
    let chatml_gercek = gercek(Sablon::ChatML);
    println!("GERCEK belirtec (duz)   : {duz_gercek}");
    println!("GERCEK belirtec (ChatML): {chatml_gercek}");
    println!(
        "ChatML tavana siğdi mi  : {}",
        chatml_gercek <= sayac.istem_tavani()
    );

    let (sure, uretim) = olc(160);
    println!("\n== URETIM ==");
    println!("belirtec      : {}", uretim.belirtec_sayisi);
    println!("sure          : {sure:.2} sn");
    println!("uctan uca hiz : {:.2} belirtec/sn", uretim.belirtec_sayisi as f64 / sure);
    println!("bitis         : {:?}", uretim.bitis);
    println!("--- METIN ---\n{}", uretim.metin);
}
