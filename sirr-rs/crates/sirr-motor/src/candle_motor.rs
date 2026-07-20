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
use crate::istem::Istem;
use crate::kisit::Kisitlayici;
use crate::saglayici::{
    BitisNedeni, MotorSaglayici, OrneklemeAyari, Uretim, UretimGelecegi, kutula_uretim,
};

use candle_core::{Device, Tensor, quantized::gguf_file};
use candle_transformers::generation::{LogitsProcessor, Sampling};
use candle_transformers::models::quantized_llama::ModelWeights;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tokenizers::Tokenizer;

/// Cikarimin kosacagi aygit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Aygit {
    #[default]
    Islemci,
    /// Apple GPU. candle-core'un `metal` ozelligi kapaliyken hata doner —
    /// sessizce islemciye DUSMEZ: kullanici GPU istedigini bilerek soyledi,
    /// 10 kat yavas kosan bir motoru "calisiyor" diye sunmak yanlis olur.
    Metal,
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
    model: Mutex<ModelWeights>,
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
        let model = ModelWeights::from_gguf(icerik, &mut dosya, &aygit)
            .map_err(|e| MotorHatasi::Cikarim(format!("gguf cozulemedi: {e}")))?;

        let belirtecleyici = Tokenizer::from_file(&ayar.belirtecleyici_yolu)
            .map_err(|e| MotorHatasi::Belirtecleme(e.to_string()))?;

        let bitis_belirtecleri = if ayar.bitis_belirtecleri.is_empty() {
            bitis_belirtecleri_bul(&belirtecleyici)
        } else {
            ayar.bitis_belirtecleri.clone()
        };

        let dagarcik = dagarcik_kur(&belirtecleyici);

        Ok(Self { model: Mutex::new(model), belirtecleyici, aygit, bitis_belirtecleri, dagarcik })
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
    ) -> MotorSonuc<Uretim> {
        let girdi = self.belirtecle(&istem.metin())?;
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

        let mut uretilen: Vec<u32> = Vec::with_capacity(ayar.en_cok_belirtec);
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
        kutula_uretim(async move { self.dongu(istem, kisit, ayar) })
    }
}

/// Belirtecleyicinin sozlugunden yaygin bitis belirteclerini toplar.
///
/// GGUF ailesi tek bir ada yerlesmedi (llama `</s>`, ChatML `<|im_end|>`,
/// llama-3 `<|eot_id|>`). Hicbiri bulunamazsa liste bos kalir ve uretim
/// yalnizca belirtec tavaninda durur: yanlis bir kimligi bitis saymaktansa
/// gec durmak yeglenir — ilki ciktiyi ortasindan keser, ikincisi yalniz
/// biraz fazla belirtec harcar.
fn bitis_belirtecleri_bul(belirtecleyici: &Tokenizer) -> Vec<u32> {
    ["</s>", "<|im_end|>", "<|eot_id|>", "<|end_of_text|>", "<end_of_turn>"]
        .iter()
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
fn dagarcik_kur(belirtecleyici: &Tokenizer) -> Vec<String> {
    let boy = belirtecleyici.get_vocab_size(true);
    (0..boy as u32)
        .map(|id| belirtecleyici.decode(&[id], false).unwrap_or_default())
        .collect()
}
