//! Eval vakalari — sirr'in davranis sozlesmesinin YAZILI hali.
//!
//! NEDEN BETIK ALANI VAR: bir eval vakasi iki soruyu ayirmak zorunda —
//! "model dogru araci sectі mi" ve "sirr o secimi dogru YURUTTU mu". Ikincisi
//! mantiktir ve modelden bagimsiz olarak her calistirmada AYNI cikmak
//! zorundadir. `betik` alani modelin yerine gecerek ikinci soruyu yalitir;
//! gercek motorla kosuldugunda (bkz. `TekMotor`) betik yok sayilir ve ilk soru
//! olculur. Ayni vaka listesi iki olcumu de besler.
//!
//! KANIT NEREDEN TOPLANIR: yalniz modelin son cumlesinden degil — arac
//! ciktilarindan, cip metinlerinden ve `kaynak_ref`lerden de. Swift tarafinda
//! ogrenilen ders: modelin yanitinda "200" yazmasi aracin 200'u HESAPLADIGININ
//! kaniti degildir; model sayiyi kendi uydurmus olabilir. Kanit havuzu bu
//! yuzden aracin soyledigini de icerir.

use serde::Serialize;

/// Tek bir degerlendirme vakasi.
#[derive(Debug, Clone, Serialize)]
pub struct EvalVakasi {
    pub ad: String,
    /// Kullanicinin mesaji.
    pub girdi: String,
    /// Cagrilmasi beklenen arac. `None` = HIC arac cagrilmamali (selamlasma,
    /// sohbet). Bos birakmak "onemsemiyorum" demek degil, "arac istemiyorum"
    /// demektir — arac istahi en sik gorulen regresyon.
    pub beklenen_arac: Option<String>,
    /// Kanit havuzunda BULUNMASI gereken parcalar (hepsi).
    pub beklenen_kanit: Vec<String>,
    /// Kanit havuzunda BULUNMAMASI gereken parcalar (hicbiri) — uydurma ve
    /// sessiz duse tespiti.
    pub yasak: Vec<String>,
    /// SahteMotor betigi: turler sirasiyla bu ciktilari uretir.
    #[serde(skip)]
    pub betik: Vec<String>,
    /// Bu vaka gramer kisiti KAPALI kosar.
    ///
    /// NEDEN BOYLE BIR BAYRAK GEREKTI: `AracYurutucu` savunmayi iki katli
    /// kurar — gramer gecersiz cagriyi URETILEMEZ yapar, sema kapisi (KAPI 2)
    /// yine de dogrular. Yurutucunun kendi yorumu bunu acikca soyluyor:
    /// "gramer zaten zorluyor ama gramer devre disi birakilabilir; kapinin iki
    /// katli olmasi bilincli". Alt kati olcen bir vaka, ust kat aciksa
    /// URETIM ASAMASINDA takilir ve olcmek istedigi kapiya hic ulasamaz.
    /// Bayrak "bu vaka hangi kati olcuyor" sorusunu vakanin kendisinde
    /// cevaplatir; sessizce kisiti kapatmak yerine yazili bir karar yapar.
    #[serde(skip)]
    pub kisitsiz: bool,
}

impl EvalVakasi {
    pub fn yeni(ad: &str, girdi: &str) -> Self {
        Self {
            ad: ad.into(),
            girdi: girdi.into(),
            beklenen_arac: None,
            beklenen_kanit: Vec::new(),
            yasak: Vec::new(),
            betik: Vec::new(),
            kisitsiz: false,
        }
    }

    /// Kisiti kapatir — yalnizca YURUTUCU kapilarini olcen vakalar icin.
    pub fn kisitsiz(mut self) -> Self {
        self.kisitsiz = true;
        self
    }

    pub fn arac(mut self, ad: &str) -> Self {
        self.beklenen_arac = Some(ad.into());
        self
    }

    pub fn kanit(mut self, parcalar: &[&str]) -> Self {
        self.beklenen_kanit = parcalar.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn yasak(mut self, parcalar: &[&str]) -> Self {
        self.yasak = parcalar.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn betik(mut self, adimlar: &[&str]) -> Self {
        self.betik = adimlar.iter().map(|s| s.to_string()).collect();
        self
    }
}

/// Kosucunun calisma dizinine yazdigi kucuk tablo. Modele borulu markdown
/// olarak donmesi gerekir — zincirin kalbi (bkz. `belge_oku`).
pub const TABLO_DOSYASI: &str = "rapor.md";
/// `METIN_DEPO_ESIGI`ni (1500 bayt) asan dosya: bypass kanalini tetikler.
pub const UZUN_DOSYA: &str = "uzun.md";
/// Sabit "simdi" — 2026-07-20T00:00:00Z, Pazartesi. Gercek saate bagli bir
/// eval belirlenimci olamaz.
pub const SABIT_EPOCH: i64 = 1_784_505_600;

/// Tum vakalar.
///
/// Sira bilincli: once arac ISTAHI (sohbet), sonra dogru secim, sonra kanal ve
/// kapi davranislari. Kosu yarida kesilse bile en cok bilgi veren kisim
/// olculmus olur.
pub fn hepsi() -> Vec<EvalVakasi> {
    let mut v = Vec::new();
    v.extend(sohbet());
    v.extend(hesap());
    v.extend(zaman());
    v.extend(belge());
    v.extend(kanal());
    v.extend(kapi());
    v
}

/// Arac cagrilmamasi gereken durumlar. Kucuk model selamlasmaya bile arama
/// cagirir; bu kategori o istahi olcer.
fn sohbet() -> Vec<EvalVakasi> {
    vec![
        EvalVakasi::yeni("sohbet-selam", "Merhaba")
            .betik(&["Merhaba! Nasil yardimci olabilirim?"])
            .kanit(&["Merhaba"]),
        EvalVakasi::yeni("sohbet-tesekkur", "Cok tesekkur ederim")
            .betik(&["Rica ederim."])
            .kanit(&["Rica"]),
        // Cihaz-ustu kimlik: model kendini bulut asistani sanmamali.
        EvalVakasi::yeni("sohbet-cihaz-ustu", "Verilerimi buluta mi gonderiyorsun?")
            .betik(&["Hayir, her sey cihazinda kaliyor."])
            .kanit(&["cihazinda"])
            .yasak(&["sunucularimiza"]),
    ]
}

fn hesap() -> Vec<EvalVakasi> {
    vec![
        EvalVakasi::yeni("hesap-carpma", "125 carpi 8 kac eder?")
            .arac("hesapla")
            .betik(&[r#"hesapla({"ifade":"125*8"})"#, "125 x 8 = 1000 eder."])
            // 1000'i ARAC soylemeli; modelin cumlesindeki sayi kanit degil.
            .kanit(&["1000"]),
        EvalVakasi::yeni("hesap-yuzde", "250 liranin yuzde 20 indirimlisi kac lira?")
            .arac("hesapla")
            .betik(&[r#"hesapla({"ifade":"250-250*20%"})"#, "200 lira."])
            .kanit(&["200"]),
        // Desteklenmeyen ifade: arac SESSIZCE bir sayi uydurmamali.
        EvalVakasi::yeni("hesap-gecersiz", "sin(45) kac eder?")
            .arac("hesapla")
            .betik(&[r#"hesapla({"ifade":"sin(45)"})"#, "Bunu hesaplayamadim."])
            .kanit(&["tool_failed"]),
    ]
}

fn zaman() -> Vec<EvalVakasi> {
    vec![
        EvalVakasi::yeni("zaman-tarih", "Bugunun tarihi ne?")
            .arac("zaman")
            .betik(&[r#"zaman({"tur":"tarih"})"#, "Bugun 20 Temmuz 2026."])
            .kanit(&["date=2026-07-20"]),
        EvalVakasi::yeni("zaman-gun", "Bugun gunlerden ne?")
            .arac("zaman")
            .betik(&[r#"zaman({"tur":"gun"})"#, "Pazartesi."])
            .kanit(&["weekday=Monday"]),
        // Takvim aritmetigi ARACTA yapilir; model kendi saymamali.
        EvalVakasi::yeni("zaman-fark", "2 Aralik 2026'ya kac gun var?")
            .arac("zaman")
            .betik(&[
                r#"zaman({"tur":"fark","hedef":"2026-12-02"})"#,
                "135 gun var.",
            ])
            .kanit(&["days=135", "to=2026-12-02"]),
        // ZAMAN COZULEMEDIGINDE BASARISIZ DONMELI. Sessizce bugune dusmek
        // modele "0 gun" gosterir ve model bunu cevap sanar.
        EvalVakasi::yeni("zaman-cozulemez", "Falanca gune kac gun var?")
            .arac("zaman")
            .betik(&[
                r#"zaman({"tur":"fark","hedef":"falanca gun"})"#,
                "Tarihi anlayamadim, netlestirir misin?",
            ])
            .kanit(&["unparsable_date"])
            .yasak(&["days=0"]),
    ]
}

fn belge() -> Vec<EvalVakasi> {
    vec![
        // TABLO MARKDOWN ZINCIRI: modele giden metin BORULU olmali; borusuz
        // ozet donseydi model tabloyu yeniden kuramaz, "tablo gosterildi"
        // deyip icerigi atlardi.
        EvalVakasi::yeni("belge-oku-tablo", "rapor.md dosyasinda ne var?")
            .arac("belge_oku")
            .betik(&[
                r#"belge_oku({"yol":"rapor.md"})"#,
                "Dosyada haftalik yemek tablosu var.",
            ])
            .kanit(&["| Gun | Yemek |", "| --- |", "| Pazartesi | Mercimek |"]),
        EvalVakasi::yeni("belge-olustur-excel", "Haftalik yemek listesi icin bir excel yap")
            .arac("belge_olustur")
            .betik(&[
                r#"belge_olustur({"bicim":"excel","dosya_adi":"yemek","icerik":"| Gun | Yemek |\n| --- | --- |\n| Pazartesi | Mercimek |"})"#,
                "Excel dosyasini olusturdum.",
            ])
            .kanit(&["file_created (excel)", "yemek.xlsx"]),
        EvalVakasi::yeni("belge-olustur-markdown", "Kisa bir not dosyasi olustur")
            .arac("belge_olustur")
            .betik(&[
                r#"belge_olustur({"bicim":"markdown","dosya_adi":"not","icerik":"Merhaba"})"#,
                "Not dosyasi hazir.",
            ])
            .kanit(&["file_created (markdown)", "not.md"]),
        // Var olmayan dosya: arac SESSIZCE bos icerik uydurmamali.
        EvalVakasi::yeni("belge-oku-yok", "olmayan.md dosyasini ozetle")
            .arac("belge_oku")
            .betik(&[r#"belge_oku({"yol":"olmayan.md"})"#, "Dosyayi bulamadim."])
            .kanit(&["tool_failed"]),
        // Sema ihlali: arac HIC calismamali.
        EvalVakasi::yeni("belge-sema-ihlali", "Bir dosya olustur")
            .arac("belge_olustur")
            .betik(&[
                r#"belge_olustur({"bicim":"excel"})"#,
                "Dosya adini soyler misin?",
            ])
            .kanit(&["tool_failed"])
            .yasak(&["file_created"])
            // KISITSIZ: bu vaka gramerin degil, SEMA KAPISININ (KAPI 2)
            // olcumudur. Kisit acikken model `dosya_adi` zorunlu alanini
            // atlayip nesneyi kapatamaz — `}` maskelenir, uretim daha
            // baslangicta durur ve olculmek istenen kapiya hic varilmaz.
            // Gramerin ayni ihlali URETIM asamasinda engelledigi ayrica
            // birim testiyle kanitlaniyor (sirr-gramer/src/cagri.rs).
            .kisitsiz(),
    ]
}

/// 4096 token bypass kanali — mimarinin kalbi.
fn kanal() -> Vec<EvalVakasi> {
    vec![
        // Buyuk belge modele TAMAMEN gitmez: kisa onizleme + kaynak_ref doner.
        EvalVakasi::yeni("kanal-kaynak-ref", "uzun.md dosyasinda ne var?")
            .arac("belge_oku")
            .betik(&[r#"belge_oku({"yol":"uzun.md"})"#, "Uzun bir liste var."])
            .kanit(&["kaynak_ref=belge#1"]),
        // ZINCIR: cihaz verisi modelden GECMEDEN dosyaya iner. Model yalniz
        // referansi tasir; toplu veri hicbir turda istemde gorunmez.
        EvalVakasi::yeni("kanal-zincir", "uzun.md icerigini bir markdown dosyasina dok")
            .arac("belge_olustur")
            .betik(&[
                r#"belge_oku({"yol":"uzun.md"})"#,
                r#"belge_olustur({"bicim":"markdown","dosya_adi":"dokum","kaynak_ref":"belge#1"})"#,
                "Dosyayi olusturdum.",
            ])
            .kanit(&["kaynak_ref=belge#1", "file_created (markdown)", "dokum.md"]),
        // Cozulemeyen referans: dosya HIC yazilmamali.
        EvalVakasi::yeni("kanal-bilinmeyen-ref", "Depodaki veriyi dosyaya dok")
            .arac("belge_olustur")
            .betik(&[
                r#"belge_olustur({"bicim":"markdown","dosya_adi":"hayalet","kaynak_ref":"belge#99"})"#,
                "Kaynak veriyi bulamadim.",
            ])
            .kanit(&["unknown_data_ref"])
            .yasak(&["file_created"]),
    ]
}

/// Kirli oturum / onay kapisi. Bu turda gercek bir dis arac yok; kapinin
/// MEKANIZMASI `SahteDisArac` ile olculur.
fn kapi() -> Vec<EvalVakasi> {
    vec![
        // Temiz oturum: kapi sorulmaz, cagri gecer. Onay nadir olmali ki
        // okunsun.
        EvalVakasi::yeni("kapi-temiz-oturum", "Su notu sunucuya gonder: toplanti 14:00")
            .arac("disari_gonder")
            .betik(&[
                r#"disari_gonder({"govde":"toplanti 14:00"})"#,
                "Gonderdim.",
            ])
            .kanit(&["sent_ok"]),
        // KIRLI OTURUM: kisisel belge okunduktan SONRA disari gonderme
        // deterministik olarak kapiya takilir.
        EvalVakasi::yeni("kapi-kirli-oturum", "rapor.md'yi oku ve sunucuya gonder")
            .arac("disari_gonder")
            .betik(&[
                r#"belge_oku({"yol":"rapor.md"})"#,
                r#"disari_gonder({"govde":"| Pazartesi | Mercimek |"})"#,
                "Gondermedim.",
            ])
            .kanit(&["permission_denied"])
            .yasak(&["sent_ok"]),
        // YAN ETKI SONRASI RETRY YASAK: dosya yazildiktan sonra ayni istem
        // ikinci kez gonderilirse ikinci dosya olusur.
        EvalVakasi::yeni("kapi-retry-yasak", "Bir rapor dosyasi olustur")
            .arac("belge_olustur")
            .betik(&[
                r#"belge_olustur({"bicim":"markdown","dosya_adi":"rapor-cikti","icerik":"govde"})"#,
                "Olusturdum.",
            ])
            .kanit(&["file_created", "tekrar_denenebilir=false"]),
    ]
}
