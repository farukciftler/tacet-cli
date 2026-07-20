//! CandleMotor — saf-Rust GGUF cikarimi. `candle` ozelligi arkasinda.
//!
//! KULLANICI KARARI: llama.cpp FFI YOK. Cikarim Candle ile, saf Rust.
//!
//! BU DOSYA VARSAYILAN DERLEMEDE YOKTUR. `cargo build -p sirr-motor` candle
//! agacini hic cekmez; tum eval ve CI `SahteMotor` uzerinde koser. Bu dosya
//! yalniz `--features candle` ile derlenir.
//!
//! AG YOK: model ve belirtecleyici YEREL yoldan yuklenir. `hf-hub` uzantisi
//! bilerek acilmadi — bu crate hicbir kosulda indirme yapmaz.

use crate::hata::{MotorHatasi, MotorSonuc};
use crate::istem::{Istem, Sablon};
use crate::kisit::Kisitlayici;
use crate::saglayici::{
    BitisNedeni, MotorSaglayici, OrneklemeAyari, Uretim, UretimGelecegi, kutula_uretim,
};

use candle_core::{Device, Tensor, quantized::gguf_file};
use candle_transformers::generation::{LogitsProcessor, Sampling};
// MIMARI MODULU ARTIK SABIT DEGIL — bkz. `Mimari`.
//
// Onceki tur tek module (`quantized_qwen2`) sabitlenmisti. Gerekce hala
// gecerli: Qwen2.5 dikkat katmanlarinda llama'da BULUNMAYAN q/k/v BIAS
// tensorleri vardir ve llama'nin `forward_attn`i bias TOPLAMAZ. Degisen,
// artik UC farkli mimarinin desteklenmesi; hangisinin yuklenecegi GGUF
// ustverisinden OKUNUR, varsayilmaz.
use candle_transformers::models::{quantized_gemma3, quantized_qwen2, quantized_qwen3};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tokenizers::Tokenizer;

/// Cikarimin kosacagi aygit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aygit {
    Islemci,
    /// Apple GPU. candle-core'un `metal` ozelligi kapaliyken hata doner —
    /// sessizce islemciye DUSMEZ: kullanici GPU istedigini bilerek soyledi,
    /// 10 kat yavas kosan bir motoru "calisiyor" diye sunmak yanlis olur.
    Metal,
}

/// Varsayilan aygit DERLEME ZAMANINDA belirlenir.
///
/// `metal` ozelligi acikken varsayilan Metal'dir; olculdugu icin boyle
/// (Qwen2.5-3B q4_k_m, ayni istem): cozme 31 -> 73 belirtec/sn, onyukleme
/// 113 -> 775 belirtec/sn. Ozelligi acikca derleyip yine islemcide kosmak
/// kimsenin isteyecegi sey degil; tersine, "neden yavas" sorusunu dogurur.
///
/// Ozellik kapaliyken Metal secilemez zaten (aygit acilirken hata doner), o
/// yuzden varsayilan islemcidir. Bu ayrimin `cfg` ile yapilmasi bilincli:
/// calisma aninda denenip sessizce islemciye dusulseydi, GPU'nun neden
/// kullanilmadigi hicbir yerde gorunmezdi.
impl Default for Aygit {
    fn default() -> Self {
        #[cfg(feature = "metal")]
        {
            Aygit::Metal
        }
        #[cfg(not(feature = "metal"))]
        {
            Aygit::Islemci
        }
    }
}

/// Desteklenen GGUF mimarileri.
///
/// NEDEN OKUNUR, VARSAYILMAZ: agirlik dosyasinin adi ("model.gguf") mimari
/// hakkinda hicbir sey soylemez ve klasor adi kullanicinin keyfidir. Tek
/// guvenilir kaynak GGUF ustverisindeki `general.architecture` anahtaridir;
/// dosyayi ureten donusturucu onu yazmak zorundadir.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mimari {
    Qwen2,
    Qwen3,
    Gemma3,
}

impl Mimari {
    /// GGUF ustverisindeki adi mimariye cevirir.
    ///
    /// BILINMEYEN MIMARI HATA DONER, en yakin module DUSMEZ. Yanlis modul
    /// yuklemek iki sonuctan birini verir: ya ustveri anahtari bulunamaz
    /// (gurultulu, zarasiz) ya da anahtarlar rastlantiyla ortusur ve model
    /// SESSIZCE COPLUK uretir. Ikincisi kullaniciya "model aptal" gibi
    /// gorunur ve teshisi saatler alir; onu bastan imkansiz kilmak icin
    /// eslesme TAM olmak zorunda.
    pub fn cozumle(ad: &str) -> MotorSonuc<Self> {
        match ad {
            "qwen2" => Ok(Mimari::Qwen2),
            "qwen3" => Ok(Mimari::Qwen3),
            "gemma3" => Ok(Mimari::Gemma3),
            baska => Err(MotorHatasi::Cikarim(format!(
                "desteklenmeyen GGUF mimarisi: '{baska}' \
                 (destekleniyor: qwen2, qwen3, gemma3)"
            ))),
        }
    }

    /// Modelin egitildigi sohbet sablonu. Mimariye BAGLI, secilebilir degil:
    /// yanlis sablon rol sinirlarini gorunmez kilar ve model bitis belirteci
    /// uretmeyip gevezelige duser.
    pub fn sablon(self) -> Sablon {
        match self {
            Mimari::Qwen2 | Mimari::Qwen3 => Sablon::ChatML,
            Mimari::Gemma3 => Sablon::Gemma,
        }
    }

    pub fn adi(self) -> &'static str {
        match self {
            Mimari::Qwen2 => "qwen2",
            Mimari::Qwen3 => "qwen3",
            Mimari::Gemma3 => "gemma3",
        }
    }
}

/// Yuklu agirliklar — mimariye gore ayrisan tek yer.
///
/// Uc modulun `ModelWeights` tipleri ORTAK BIR TRAIT PAYLASMIYOR (candle
/// bunlari bagimsiz somut tipler olarak sunuyor), bu yuzden koprulemek icin
/// elle bir enum gerekiyor. `Box<dyn ...>` bir secenek degildi: yazilacak
/// trait'i biz tanimlasak bile `forward` imzalari ayni olmasina ragmen tipler
/// yabanci ve trait'i onlar icin uygulamak yine bu enum kadar kod olurdu.
enum MimariModel {
    Qwen2(quantized_qwen2::ModelWeights),
    Qwen3(quantized_qwen3::ModelWeights),
    Gemma3(quantized_gemma3::ModelWeights),
}

impl MimariModel {
    fn forward(&mut self, girdi: &Tensor, konum: usize) -> candle_core::Result<Tensor> {
        match self {
            MimariModel::Qwen2(m) => m.forward(girdi, konum),
            MimariModel::Qwen3(m) => m.forward(girdi, konum),
            MimariModel::Gemma3(m) => m.forward(girdi, konum),
        }
    }

    /// KV onbellegini uretimler arasi sifirlar.
    ///
    /// GEMMA3 ICIN GOVDE BOSTUR VE BU BIR EKSIK DEGIL. candle'in
    /// `quantized_gemma3` moduluu bilerek `clear_kv_cache` SUNMAZ, cunku
    /// ihtiyaci yoktur: dikkat katmani onbellegi birlestirmeden once
    /// `if index_pos == 0 { (k, v) }` diye bakar, yani onyukleme adiminda eski
    /// onbellegi KULLANMAZ ve uzerine yazar. Uretim dongumuz her seferinde
    /// `konum = 0` ile basladigi icin Gemma3 kendiliginde temizlenir.
    /// Qwen2/Qwen3 ise kosulsuz `Tensor::cat` yapar — onlarda temizlik SART.
    ///
    /// Bu ayrim kaynaktan OKUNDU, varsayilmadi; yanlis tarafi secmek sinsi bir
    /// bozulma verirdi (ilk cevap duzgun, sonrakiler giderek bozuk).
    fn onbellegi_temizle(&mut self) {
        match self {
            MimariModel::Qwen2(m) => m.clear_kv_cache(),
            MimariModel::Qwen3(m) => m.clear_kv_cache(),
            MimariModel::Gemma3(_) => {}
        }
    }
}

/// Model yukleme ayarlari.
#[derive(Debug, Clone)]
pub struct ModelAyari {
    /// GGUF agirlik dosyasi (yerel).
    pub model_yolu: PathBuf,
    /// `tokenizer.json` (yerel).
    pub belirtecleyici_yolu: PathBuf,
    pub aygit: Aygit,
    /// Uretimi durduran belirtec kimlikleri. Bos birakilirsa belirtecleyicinin
    /// sozlugunde yaygin adlar aranir.
    pub bitis_belirtecleri: Vec<u32>,
}

impl ModelAyari {
    pub fn yeni(model_yolu: impl Into<PathBuf>, belirtecleyici_yolu: impl Into<PathBuf>) -> Self {
        Self {
            model_yolu: model_yolu.into(),
            belirtecleyici_yolu: belirtecleyici_yolu.into(),
            aygit: Aygit::default(),
            bitis_belirtecleri: Vec::new(),
        }
    }

    pub fn aygitla(mut self, aygit: Aygit) -> Self {
        self.aygit = aygit;
        self
    }
}

pub struct CandleMotor {
    /// `forward` `&mut self` ister, oysa `MotorSaglayici::uret` `&self` alir.
    /// Mutex hem bu boslugu kapatir hem de dogru olani dayatir: KV onbellegi
    /// modelin ICINDE tutuluyor, iki uretim ayni anda kosarsa birbirinin
    /// onbellegini bozar. Kilit bir baris odunu degil, dogruluk sarti.
    model: Mutex<MimariModel>,
    /// GGUF ustverisinden OKUNAN mimari. Sablonun ve tanilama ciktisinin
    /// kaynagi; kullanici hangi modulun yuklendigini gorebilmeli.
    mimari: Mimari,
    belirtecleyici: Tokenizer,
    aygit: Device,
    bitis_belirtecleri: Vec<u32>,
    /// Belirtec kimligi -> YUZEY metni. Kisit kurmanin sarti (bkz. `dagarcik`).
    /// Yukleme aninda bir kez uretilir: 32k belirtec icin coz cagrisi ucuz
    /// degil, ama gguf yuklemesinin yaninda gorunmez ve uretim basina
    /// tekrarlanmasi anlamsiz olurdu.
    dagarcik: Vec<String>,
}

impl CandleMotor {
    /// Agirliklari ve belirtecleyiciyi YEREL dosyadan yukler.
    pub fn yukle(ayar: &ModelAyari) -> MotorSonuc<Self> {
        let aygit = match ayar.aygit {
            Aygit::Islemci => Device::Cpu,
            Aygit::Metal => Device::new_metal(0)
                .map_err(|e| MotorHatasi::Cikarim(format!("metal aygiti acilamadi: {e}")))?,
        };

        let mut dosya = std::fs::File::open(&ayar.model_yolu)
            .map_err(|_| MotorHatasi::ModelYuklenemedi(ayar.model_yolu.clone()))?;
        let icerik = gguf_file::Content::read(&mut dosya)
            .map_err(|_| MotorHatasi::ModelYuklenemedi(ayar.model_yolu.clone()))?;

        // MIMARI AGIRLIKLARDAN ONCE OKUNUR. Ustveri zaten bellekte; yanlis
        // modulle 2.5 GB yuklemeye baslayip sonunda hata almak gereksiz.
        let mimari = mimariyi_oku(&icerik)?;

        // `from_gguf` UC MODULDE DE ayni imzaya sahip (ct, reader, device) —
        // kaynaktan dogrulandi, varsayilmadi.
        let model = match mimari {
            Mimari::Qwen2 => quantized_qwen2::ModelWeights::from_gguf(icerik, &mut dosya, &aygit)
                .map(MimariModel::Qwen2),
            Mimari::Qwen3 => quantized_qwen3::ModelWeights::from_gguf(icerik, &mut dosya, &aygit)
                .map(MimariModel::Qwen3),
            Mimari::Gemma3 => quantized_gemma3::ModelWeights::from_gguf(icerik, &mut dosya, &aygit)
                .map(MimariModel::Gemma3),
        }
        .map_err(|e| MotorHatasi::Cikarim(format!("gguf cozulemedi ({}): {e}", mimari.adi())))?;

        let belirtecleyici = Tokenizer::from_file(&ayar.belirtecleyici_yolu)
            .map_err(|e| MotorHatasi::Belirtecleme(e.to_string()))?;

        let bitis_belirtecleri = if ayar.bitis_belirtecleri.is_empty() {
            bitis_belirtecleri_bul(&belirtecleyici)
        } else {
            ayar.bitis_belirtecleri.clone()
        };

        let dagarcik = dagarcik_kur(&belirtecleyici);

        Ok(Self {
            model: Mutex::new(model),
            mimari,
            belirtecleyici,
            aygit,
            bitis_belirtecleri,
            dagarcik,
        })
    }

    /// Yuklenen GGUF'un mimarisi — tanilama ve kabuk ciktisi icin.
    pub fn mimari(&self) -> Mimari {
        self.mimari
    }

    /// Dosyalarin varligini YUKLEMEDEN once dogrular — gguf yuklemesi uzun
    /// surer, eksik dosyayi o surenin sonunda ogrenmek gereksiz bir bekleyis.
    pub fn dosyalar_var_mi(ayar: &ModelAyari) -> MotorSonuc<()> {
        for yol in [&ayar.model_yolu, &ayar.belirtecleyici_yolu] {
            if !Path::new(yol).is_file() {
                return Err(MotorHatasi::ModelYuklenemedi(yol.clone()));
            }
        }
        Ok(())
    }

    /// Uretimi durduran belirtec kimlikleri.
    ///
    /// DISA ACIK cunku bos kalmasi SESSIZ bir arizadir: uretim o zaman yalniz
    /// belirtec tavaninda durur ve "model cok konusuyor" gibi gorunur. Cagri
    /// yeri bunu basip gozle dogrulayabilmeli.
    pub fn bitis_belirtecleri(&self) -> &[u32] {
        &self.bitis_belirtecleri
    }

    /// Yuklu belirtecleyici — olcum ve tani icin.
    pub fn belirtecleyici(&self) -> &Tokenizer {
        &self.belirtecleyici
    }

    fn belirtecle(&self, metin: &str) -> MotorSonuc<Vec<u32>> {
        self.belirtecleyici
            .encode(metin, true)
            .map(|e| e.get_ids().to_vec())
            .map_err(|e| MotorHatasi::Belirtecleme(e.to_string()))
    }

    fn coz(&self, belirtecler: &[u32]) -> MotorSonuc<String> {
        self.belirtecleyici
            .decode(belirtecler, true)
            .map_err(|e| MotorHatasi::Belirtecleme(e.to_string()))
    }

    /// Asil uretim dongusu. Ayri ve SENKRON: cikarim islemciye/GPU'ya baglidir,
    /// icinde beklenecek bir G/C yoktur — `async` govdeye sarilmasi yalnizca
    /// sozlesmeyi karsilamak icin.
    fn dongu(
        &self,
        istem: &Istem,
        kisit: Option<&dyn Kisitlayici>,
        ayar: OrneklemeAyari,
        dinleyici: Option<&(dyn Fn(&str) + Send + Sync)>,
    ) -> MotorSonuc<Uretim> {
        // Sablonu MOTOR bildirir (bkz. `MotorSaglayici::sablon`): Qwen2.5
        // ChatML ile egitildi, duz metin beslendiginde rolleri kaybedip
        // gevezelige duser.
        //
        // `true` = ozel belirtecleri AYRISTIR. ChatML citleri (`<|im_start|>`)
        // metin olarak degil, TEK belirtec olarak gecmek zorunda; aksi halde
        // model kendi cerceve isaretlerini tanimaz ve bitis belirteci hic
        // uretilmez (uretim tavana kadar surer).
        let girdi = self.belirtecle(&istem.metin_sablonlu(self.sablon()))?;
        if girdi.is_empty() {
            return Err(MotorHatasi::Belirtecleme("istem bos belirteclendi".into()));
        }

        // Sicaklik 0 -> ArgMax (greedy). Varsayilan bu: eval'in tekrarlanabilir
        // olmasi ornekleme cesitliliginden once gelir.
        let ornekleme = if ayar.sicaklik <= f32::EPSILON {
            Sampling::ArgMax
        } else if ayar.top_p >= 1.0 {
            Sampling::All { temperature: ayar.sicaklik as f64 }
        } else {
            Sampling::TopP { p: ayar.top_p as f64, temperature: ayar.sicaklik as f64 }
        };
        let mut ornekleyici = LogitsProcessor::from_sampling(ayar.tohum, ornekleme);

        let mut oturum = kisit.map(|k| k.oturum());
        let mut model = self.model.lock().expect("model kilidi");

        // KV ONBELLEGI HER URETIMDE SIFIRLANIR. Onbellek modelin ICINDE
        // yasiyor ve uretimler arasinda kaliyor; oysa `konum` asagida 0'dan
        // basliyor. Temizlenmeseydi ikinci uretim, birincinin onbellek
        // satirlarinin UZERINE yazar ve dikkat, o turun istemiyle bir onceki
        // turun artiklarinin karisimini gorurdu. Belirti sinsi: ilk cevap
        // duzgun, sonrakiler giderek bozuluyor.
        model.onbellegi_temizle();

        let mut uretilen: Vec<u32> = Vec::with_capacity(ayar.en_cok_belirtec);
        // Dinleyiciye halihazirda yayilmis metnin bayt uzunlugu — bir sonraki
        // adimda yalniz eki gonderebilmek icin.
        let mut yazilan = String::new();
        let mut bitis = BitisNedeni::Uzunluk;
        // Istem tek seferde islenir (prefill); sonraki adimlarda tek belirtec
        // beslenir ve `konum` KV onbelleginin nerede oldugunu soyler.
        let mut konum = 0usize;
        let mut sonraki: Vec<u32> = girdi;

        for _ in 0..ayar.en_cok_belirtec {
            let girdi_t = Tensor::new(sonraki.as_slice(), &self.aygit)
                .and_then(|t| t.unsqueeze(0))
                .map_err(|e| MotorHatasi::Cikarim(e.to_string()))?;

            let logits = model
                .forward(&girdi_t, konum)
                .map_err(|e| MotorHatasi::Cikarim(e.to_string()))?;
            // forward [batch, vocab] doner; tek satira indiriyoruz.
            let logits = logits
                .squeeze(0)
                .and_then(|t| t.to_dtype(candle_core::DType::F32))
                .map_err(|e| MotorHatasi::Cikarim(e.to_string()))?;

            konum += sonraki.len();

            // KISIT HAM LOGITS UZERINDE UYGULANIR, ornekleyiciye girmeden once.
            //
            // Candle'in `LogitsProcessor::sample_f` geri cagrisi cazip
            // gorunuyor ama BURAYA UYMUYOR, iki nedenle: (1) geri cagri
            // softmax'tan SONRAKI olasiliklar uzerinde calisir, oysa maske
            // "-sonsuz logit" dilinde konusur; (2) `Sampling::ArgMax` yolunda
            // geri cagri HIC cagrilmaz — yani tam da varsayilan greedy
            // modumuzda kisit sessizce devre disi kalirdi. Maskeyi burada
            // uygulamak, hangi ornekleme secilirse secilsin kisiti baglayici kilar.
            let belirtec = if let Some(o) = oturum.as_mut() {
                let mut ham: Vec<f32> = logits
                    .to_vec1()
                    .map_err(|e| MotorHatasi::Cikarim(e.to_string()))?;
                o.maskele(&mut ham);
                // Her sey yasaklandiysa ornekleyici anlamsiz bir secim yapardi
                // (NaN olasilik dagilimi); bu bir gramer hatasidir, sessizce
                // gecistirilmemeli.
                if !ham.iter().any(|v| v.is_finite()) {
                    return Err(MotorHatasi::Cikarim("kisit tum belirtecleri yasakladi".into()));
                }
                let maskeli = Tensor::new(ham.as_slice(), &self.aygit)
                    .map_err(|e| MotorHatasi::Cikarim(e.to_string()))?;
                ornekleyici
                    .sample(&maskeli)
                    .map_err(|e| MotorHatasi::Cikarim(e.to_string()))?
            } else {
                ornekleyici
                    .sample(&logits)
                    .map_err(|e| MotorHatasi::Cikarim(e.to_string()))?
            };

            if self.bitis_belirtecleri.contains(&belirtec) {
                bitis = BitisNedeni::Belirtec;
                break;
            }

            if let Some(o) = oturum.as_mut() {
                o.ilerlet(belirtec)?;
            }
            uretilen.push(belirtec);
            sonraki = vec![belirtec];

            // AKIS: her adimda birikmis metni cozup YENI eki dinleyiciye ver.
            // Tam vektoru her seferinde cozmek O(n^2) gorunur ama n birkac yuz
            // belirtectir ve maliyeti bir `forward` gecisinin yaninda ihmal
            // edilebilir; kazanci, BPE'nin cok baytli parcalarini (Turkce 'ş',
            // 'ı') tek belirtecten yanlis kesmeden dogru sinirda yaymak.
            // Kisit acikken de calisir: dinleyici gordugu metin gercekten
            // uretilen metindir, halusinasyon degil.
            if let Some(f) = dinleyici
                && let Ok(tam) = self.coz(&uretilen)
                && tam.len() > yazilan.len()
                && tam.is_char_boundary(yazilan.len())
            {
                f(&tam[yazilan.len()..]);
                yazilan = tam;
            }

            // Kisit kabul durumuna geldi: dilbilgisi tamamlandi, burada durmak
            // GUVENLI. Devam etmek, modelin gecerli JSON'un ardina gevezelik
            // eklemesine izin vermek olurdu.
            if oturum.as_ref().is_some_and(|o| o.bitti_mi()) {
                bitis = BitisNedeni::KisitTamam;
                break;
            }
        }

        let metin = self.coz(&uretilen)?;
        Ok(Uretim::yeni(metin, uretilen.len(), bitis))
    }
}

impl MotorSaglayici for CandleMotor {
    fn ad(&self) -> &str {
        "candle"
    }

    /// Sablon YUKLENEN MIMARIDEN turer, sabit degildir. Bunu bildirmek
    /// zorunlu: duz metin beslenirse model rol sinirlarini goremez, bitis
    /// belirtecini uretmez ve kendi kendine "Kullanici:" yazip konusmayi
    /// surdurur. Gemma'ya ChatML beslemek de ayni sonucu verir — citler
    /// tanidik gelmez.
    fn sablon(&self) -> Sablon {
        self.mimari.sablon()
    }

    /// Kisit kurmanin sarti. Bunu bildirmeseydik `CagriKisiti` hic kurulamaz
    /// ve GERCEK model tam da kisitin en cok gerektigi yerde (kucuk model,
    /// serbest uretim) kisitsiz kosardi — sahte motorda calisan bir guvenlik
    /// uretimde yok olurdu.
    fn dagarcik(&self) -> Option<Vec<String>> {
        Some(self.dagarcik.clone())
    }

    fn uret<'a>(
        &'a self,
        istem: &'a Istem,
        kisit: Option<&'a dyn Kisitlayici>,
        ayar: OrneklemeAyari,
    ) -> UretimGelecegi<'a> {
        kutula_uretim(async move { self.dongu(istem, kisit, ayar, None) })
    }

    /// Akan uretim: `dongu`ya dinleyiciyi gecirir. Varsayilan (tek-parca)
    /// uygulamayi EZER cunku candle gercekten belirtec-belirtec ureten tek
    /// motor; 3B modelde ilk belirtece kadarki bekleme burada gizlenir.
    fn uret_akan<'a>(
        &'a self,
        istem: &'a Istem,
        kisit: Option<&'a dyn Kisitlayici>,
        ayar: OrneklemeAyari,
        dinleyici: &'a (dyn Fn(&str) + Send + Sync),
    ) -> UretimGelecegi<'a> {
        kutula_uretim(async move { self.dongu(istem, kisit, ayar, Some(dinleyici)) })
    }
}

/// GGUF ustverisinden `general.architecture` degerini okur.
///
/// Anahtar YOKSA hata doner, tahmine gidilmez: bu anahtar GGUF sartnamesinde
/// ZORUNLUDUR, yoksa dosya ya bozuktur ya da GGUF degildir. Boyle bir dosyada
/// mimari tahmin etmek, cozulecek bir sorun yokken risk uretmek olurdu.
fn mimariyi_oku(icerik: &gguf_file::Content) -> MotorSonuc<Mimari> {
    let deger = icerik.metadata.get("general.architecture").ok_or_else(|| {
        MotorHatasi::Cikarim(
            "GGUF ustverisinde 'general.architecture' yok — dosya bozuk ya da GGUF degil".into(),
        )
    })?;
    let ad = deger
        .to_string()
        .map_err(|e| MotorHatasi::Cikarim(format!("'general.architecture' metin degil: {e}")))?;
    Mimari::cozumle(ad)
}

/// Belirtecleyicinin sozlugunden yaygin bitis belirteclerini toplar.
///
/// GGUF ailesi tek bir ada yerlesmedi (llama `</s>`, ChatML `<|im_end|>`,
/// llama-3 `<|eot_id|>`). Hicbiri bulunamazsa liste bos kalir ve uretim
/// yalnizca belirtec tavaninda durur: yanlis bir kimligi bitis saymaktansa
/// gec durmak yeglenir — ilki ciktiyi ortasindan keser, ikincisi yalniz
/// biraz fazla belirtec harcar.
///
/// ADI ARAMAK YETMEZ, OZEL OLDUGU DA DOGRULANIR. Olculdu: Qwen2.5 dagarciginda
/// `</s>` ORTALAMA bir BPE belirtecidir (id 128247) — ozel degil, yalnizca o
/// harfleri tasiyan siradan bir parca. Yalniz ada baksaydik model `</s>`
/// dizgisini metin olarak yazdigi anda (XML/HTML anlatirken pekala olur)
/// uretim cumlenin ortasinda kesilirdi. `is_special_token` bu ayrimi yapar:
/// gercek bitis belirteci dagarciga SONRADAN EKLENMIS ve ozel isaretlenmis
/// olandir (`<|im_end|>`, id 151645).
fn bitis_belirtecleri_bul(belirtecleyici: &Tokenizer) -> Vec<u32> {
    let eklenmis = belirtecleyici.get_added_vocabulary();
    ["</s>", "<|im_end|>", "<|eot_id|>", "<|end_of_text|>", "<end_of_turn>", "<|endoftext|>"]
        .iter()
        .filter(|ad| eklenmis.is_special_token(ad))
        .filter_map(|ad| belirtecleyici.token_to_id(ad))
        .collect()
}

/// Belirtec kimliklerini YUZEY metnine cevirir (kisit maskesinin girdisi).
///
/// NEDEN `id_to_token` DEGIL `decode`: `id_to_token` belirtecin HAM bicimini
/// verir ve BPE aileleri orada gorunmez isaretler tasir — GPT-2 turevi
/// dagarciklarda bosluk `Ġ`, sentencepiece'te `▁` olarak kodlanir. Gramer
/// karakter bazinda calistigi icin ham bicimi beslemek maskeyi bastan yanlis
/// kurardi: model gercek bir bosluk uretirken gramer `Ġ` gorurdu ve gecerli
/// JSON reddedilirdi. `decode` bu kodlamayi cozup gercekten yazilacak metni
/// verir — maske ile uretilen metnin ayni alfabeyi konusmasi sarttir.
///
/// Cozulemeyen kimlik BOS string olur; `TokenMaskesi` bos metinli belirtecleri
/// zaten "ozel/notr" sayip maskede kapali tutar, yani ozel belirtecler
/// gramerin ortasina sizamaz.
///
/// DOGRULANMADI: bu yol gercek bir GGUF + tokenizer ciftiyle HENUZ
/// kosturulmadi (bkz. DURUM.md "Bilinen riskler"). Sahte motorda kod noktasi =
/// belirtec oldugu icin bu donusum orada olculemiyor.
pub fn dagarcik_kur(belirtecleyici: &Tokenizer) -> Vec<String> {
    let boy = belirtecleyici.get_vocab_size(true);
    (0..boy as u32)
        .map(|id| belirtecleyici.decode(&[id], false).unwrap_or_default())
        .collect()
}
