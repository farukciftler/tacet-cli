//! Gemma3 yukleme tanilamasi — GECICI DEGIL, kalici bir olcum araci.
//!
//! NEDEN VAR: cok-mimarili yuklemede Gemma3 dogru modulle ve dogru sablonla
//! yuklendigi halde bozuk uretiyordu. "Sablon mu, baglam uzunlugu mu, yoksa
//! yuklemenin kendisi mi" sorusunu ayirmak icin EN KISA istemle koşan bir
//! nokta gerekiyordu; olcum ornegi tam katalogla kostugu icin bunu ayiramaz.
//!
//! Kosum:
//!   SIRR_MODEL=~/models/gemma3-4b/model.gguf \
//!   SIRR_TOKENIZER=~/models/gemma3-4b/tokenizer.json \
//!     cargo run -p sirr-motor --features metal --release --example gemma_tani

use sirr_motor::{Istem, MotorSaglayici, OrneklemeAyari, bekle};

fn main() {
    let model = std::env::var("SIRR_MODEL").expect("SIRR_MODEL");
    let belirtecleyici = std::env::var("SIRR_TOKENIZER").expect("SIRR_TOKENIZER");

    let aygit = match std::env::var("SIRR_AYGIT").as_deref() {
        Ok("islemci") => sirr_motor::candle_motor::Aygit::Islemci,
        _ => sirr_motor::candle_motor::Aygit::Metal,
    };
    let ayar = sirr_motor::ModelAyari::yeni(&model, &belirtecleyici).aygitla(aygit);
    let motor = sirr_motor::CandleMotor::yukle(&ayar).expect("yukleme");

    println!("mimari : {}", motor.mimari().adi());
    println!("sablon : {:?}", motor.sablon());
    println!("bitis  : {:?}", motor.bitis_belirtecleri());

    // Belirtecleme: Gemma <bos> ile BASLAMALI. Yoksa model dagilimin disinda
    // koşar ve tekrara duser — degerini gozle gormek sart.
    let ornek = "<start_of_turn>user\nMerhaba<end_of_turn>\n<start_of_turn>model\n";
    let kodlanmis = motor.belirtecleyici().encode(ornek, true).expect("encode");
    println!("\nilk 8 belirtec : {:?}", &kodlanmis.get_ids()[..8.min(kodlanmis.len())]);
    println!("ilk 8 metin    : {:?}", &kodlanmis.get_tokens()[..8.min(kodlanmis.len())]);

    // EN KISA istem: arac tarifi yok, gecmis yok. Burada da bozuksa sorun
    // baglam uzunlugunda degil, yuklemenin kendisindedir.
    let istem = Istem::yeni("Sen yardimci bir asistansin.", "Merhaba, nasilsin?");
    let uretim = bekle(motor.uret(&istem, None, OrneklemeAyari::default())).expect("uretim");
    println!("\n== KISA ISTEM ==");
    println!("bitis   : {:?}", uretim.bitis);
    println!("metin   : {:?}", uretim.metin);

    // ISTEM UZUNLUGU TARAMASI — kayma penceresi esigini gorunur kilar.
    //
    // Gemma3'un `attention.sliding_window` degeri 1024. candle'in
    // `quantized_gemma3::forward`i cozme adiminda (`seq_len == 1`) maskeyi
    // HIC kurmuyor; yani yerel (kayan pencereli) katmanlar onbellekteki TUM
    // belirtecleri goruyor. Istem pencereden kisayken fark yok, asinca
    // uretim tekrara duser. Bu tarama tam olarak o esigi olcer.
    println!("\n== ISTEM UZUNLUGU TARAMASI ==");
    // DOLGU DEGISKEN OLMALI. Ilk denemede sabit bir cumle tekrarlanmisti ve
    // olcum GECERSIZDI: tekrarli baglam modeli zaten tekrara tesvik eder, yani
    // "uzun istemde bozuluyor" ile "tekrarli istemde bozuluyor" ayirt
    // edilemiyordu. Her cumle farkli sayi ve sozcuk tasiyor.
    for tekrar in [1usize, 40, 120, 300] {
        let dolgu: String = (0..tekrar)
            .map(|i| {
                format!(
                    "{i}. maddede sunu belirtelim: {} sayisi {} ile carpildiginda farkli bir \
                     sonuc verir ve bu durum {} baglaminda incelenir. ",
                    i * 7 + 3,
                    i % 9 + 2,
                    ["tarih", "cografya", "muzik", "mimari", "tip", "hukuk"][i % 6]
                )
            })
            .collect();
        let istem = Istem::yeni(&format!("Sen yardimci bir asistansin. {dolgu}"), "Merhaba!");
        let n = motor
            .belirtecleyici()
            .encode(istem.metin_sablonlu(motor.sablon()), true)
            .map(|e| e.len())
            .unwrap_or(0);
        let u = bekle(motor.uret(&istem, None, OrneklemeAyari::default())).expect("uretim");
        let kisa: String = u.metin.chars().take(90).collect();
        println!("  istem {n:>5} belirtec -> {:?} {kisa:?}", u.bitis);
    }
}
