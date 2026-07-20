//! `ArgSema` — arac argümanlarinin sozlesmesi.
//!
//! JSON Schema'nin TAMAMI degil, ihtiyacimiz olan alt kumesi. Gerekce: bu tip
//! iki yone birden cevrilecek — sirr-gramer onu kisitli uretim gramerine
//! (model semadan sapamaz), sirr-cli ise istem icindeki arac tarifine cevirir.
//! Tam JSON Schema (oneOf/allOf/$ref/desen) gramere cevrilemeyecek kadar
//! genistir; kucuk ve kapali tutmak modelin semaya ZORLANABILMESININ sarti.
//!
//! Alan sirasi `Vec` ile korunur (HashMap degil): gramerin ve istemin her
//! calistirmada bit-birebir ayni cikmasi, eval sonuclarinin karsilastirilabilir
//! olmasi icin gerekli.

use serde::{Deserialize, Serialize};

/// Bir argüman semasi: tip + insan aciklamasi.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArgSema {
    #[serde(flatten)]
    pub tip: SemaTipi,
    /// Modele gosterilen aciklama. Kisa ve emir kipinde olmali.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aciklama: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "tip", rename_all = "snake_case")]
pub enum SemaTipi {
    /// Alanlari sirali bir nesne. Araclarin kok semasi daima budur.
    Nesne { alanlar: Vec<Alan> },
    /// Homojen dizi.
    Dizi {
        eleman: Box<ArgSema>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        en_az: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        en_cok: Option<usize>,
    },
    Metin {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        en_cok_uzunluk: Option<usize>,
    },
    /// Kapali deger kumesi. Metin'den ayri varyant: gramer bunu birebir
    /// alternatif dizisine cevirir, model kume disina cikamaz.
    Secenek { secenekler: Vec<String> },
    Sayi {
        /// true ise tamsayi. Gramer ondalik noktayi buna gore uretir.
        #[serde(default)]
        tam_mi: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        en_az: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        en_cok: Option<f64>,
    },
    Bool,
}

/// Nesne alani.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alan {
    pub ad: String,
    pub sema: ArgSema,
    /// Zorunluluk alanin YANINDA tutulur, ayri bir `required` listesinde degil:
    /// iki yerde tutulan bilgi er gec ayrisir.
    #[serde(default)]
    pub zorunlu: bool,
}

impl Alan {
    pub fn yeni(ad: impl Into<String>, sema: ArgSema) -> Self {
        Self { ad: ad.into(), sema, zorunlu: false }
    }

    pub fn zorunlu(mut self) -> Self {
        self.zorunlu = true;
        self
    }
}

impl ArgSema {
    fn tipten(tip: SemaTipi) -> Self {
        Self { tip, aciklama: None }
    }

    /// Argüman almayan araclar icin bos nesne.
    pub fn bos() -> Self {
        Self::nesne(vec![])
    }

    pub fn nesne(alanlar: Vec<Alan>) -> Self {
        Self::tipten(SemaTipi::Nesne { alanlar })
    }

    pub fn dizi(eleman: ArgSema) -> Self {
        Self::tipten(SemaTipi::Dizi {
            eleman: Box::new(eleman),
            en_az: None,
            en_cok: None,
        })
    }

    pub fn metin() -> Self {
        Self::tipten(SemaTipi::Metin { en_cok_uzunluk: None })
    }

    pub fn secenek<S: Into<String>>(secenekler: impl IntoIterator<Item = S>) -> Self {
        Self::tipten(SemaTipi::Secenek {
            secenekler: secenekler.into_iter().map(Into::into).collect(),
        })
    }

    pub fn sayi() -> Self {
        Self::tipten(SemaTipi::Sayi { tam_mi: false, en_az: None, en_cok: None })
    }

    pub fn tamsayi() -> Self {
        Self::tipten(SemaTipi::Sayi { tam_mi: true, en_az: None, en_cok: None })
    }

    pub fn bool() -> Self {
        Self::tipten(SemaTipi::Bool)
    }

    pub fn aciklama(mut self, metin: impl Into<String>) -> Self {
        self.aciklama = Some(metin.into());
        self
    }

    /// Sayisal aralik; sayi olmayan semada sessizce yok sayilir.
    pub fn aralik(mut self, alt: Option<f64>, ust: Option<f64>) -> Self {
        if let SemaTipi::Sayi { en_az, en_cok, .. } = &mut self.tip {
            *en_az = alt;
            *en_cok = ust;
        }
        self
    }

    /// Dizi uzunluk siniri; dizi olmayan semada sessizce yok sayilir.
    pub fn uzunluk(mut self, alt: Option<usize>, ust: Option<usize>) -> Self {
        if let SemaTipi::Dizi { en_az, en_cok, .. } = &mut self.tip {
            *en_az = alt;
            *en_cok = ust;
        }
        self
    }

    /// Kok semanin alanlari (nesne degilse bos dilim) — gramer ve dogrulama
    /// bunu sik kullanir.
    pub fn alanlar(&self) -> &[Alan] {
        match &self.tip {
            SemaTipi::Nesne { alanlar } => alanlar,
            _ => &[],
        }
    }

    /// Klasik JSON Schema karsiligi. Yalniz dis dunyaya (log, istem metni,
    /// uyumluluk) bakan yuzeydir; ic akis daima `ArgSema` uzerinden gider.
    pub fn json_schema(&self) -> serde_json::Value {
        use serde_json::{Map, Value, json};
        let mut d: Map<String, Value> = match &self.tip {
            SemaTipi::Nesne { alanlar } => {
                let mut ozellikler = Map::new();
                let mut zorunlular = Vec::new();
                for a in alanlar {
                    ozellikler.insert(a.ad.clone(), a.sema.json_schema());
                    if a.zorunlu {
                        zorunlular.push(Value::String(a.ad.clone()));
                    }
                }
                let mut m = Map::new();
                m.insert("type".into(), json!("object"));
                m.insert("properties".into(), Value::Object(ozellikler));
                m.insert("required".into(), Value::Array(zorunlular));
                // Semada olmayan alan kabul edilmez: model uydurdugu bir anahtari
                // kacak yol olarak kullanamasin.
                m.insert("additionalProperties".into(), json!(false));
                m
            }
            SemaTipi::Dizi { eleman, en_az, en_cok } => {
                let mut m = Map::new();
                m.insert("type".into(), json!("array"));
                m.insert("items".into(), eleman.json_schema());
                if let Some(v) = en_az {
                    m.insert("minItems".into(), json!(v));
                }
                if let Some(v) = en_cok {
                    m.insert("maxItems".into(), json!(v));
                }
                m
            }
            SemaTipi::Metin { en_cok_uzunluk } => {
                let mut m = Map::new();
                m.insert("type".into(), json!("string"));
                if let Some(v) = en_cok_uzunluk {
                    m.insert("maxLength".into(), json!(v));
                }
                m
            }
            SemaTipi::Secenek { secenekler } => {
                let mut m = Map::new();
                m.insert("type".into(), json!("string"));
                m.insert("enum".into(), json!(secenekler));
                m
            }
            SemaTipi::Sayi { tam_mi, en_az, en_cok } => {
                let mut m = Map::new();
                m.insert("type".into(), json!(if *tam_mi { "integer" } else { "number" }));
                if let Some(v) = en_az {
                    m.insert("minimum".into(), json!(v));
                }
                if let Some(v) = en_cok {
                    m.insert("maximum".into(), json!(v));
                }
                m
            }
            SemaTipi::Bool => {
                let mut m = Map::new();
                m.insert("type".into(), json!("boolean"));
                m
            }
        };
        if let Some(a) = &self.aciklama {
            d.insert("description".into(), Value::String(a.clone()));
        }
        Value::Object(d)
    }

    /// Gelen argümanlarin semaya uygunlugunu dogrular.
    ///
    /// Gramer modeli zaten zorlasa da bu kapi duruyor: gramer devre disi
    /// birakilabilir, arac dogrudan (eval'den, CLI'dan) cagrilabilir. Sema tek
    /// sozlesme ise dogrulama da tek yerde olmali.
    pub fn dogrula(&self, deger: &serde_json::Value) -> crate::hata::AracSonuc<()> {
        self.dogrula_yol(deger, "arg")
    }

    fn dogrula_yol(&self, deger: &serde_json::Value, yol: &str) -> crate::hata::AracSonuc<()> {
        use crate::hata::AracHatasi::{EksikAlan, GecersizArguman};
        use serde_json::Value;
        match &self.tip {
            SemaTipi::Nesne { alanlar } => {
                let nesne = deger
                    .as_object()
                    .ok_or_else(|| GecersizArguman(format!("{yol}: nesne bekleniyordu")))?;
                for a in alanlar {
                    match nesne.get(&a.ad) {
                        Some(Value::Null) | None if a.zorunlu => {
                            return Err(EksikAlan(format!("{yol}.{}", a.ad)));
                        }
                        Some(v) if !v.is_null() => {
                            a.sema.dogrula_yol(v, &format!("{yol}.{}", a.ad))?;
                        }
                        _ => {}
                    }
                }
                Ok(())
            }
            SemaTipi::Dizi { eleman, en_az, en_cok } => {
                let dizi = deger
                    .as_array()
                    .ok_or_else(|| GecersizArguman(format!("{yol}: dizi bekleniyordu")))?;
                if en_az.is_some_and(|n| dizi.len() < n) || en_cok.is_some_and(|n| dizi.len() > n) {
                    return Err(GecersizArguman(format!("{yol}: eleman sayisi uygun degil")));
                }
                for (i, v) in dizi.iter().enumerate() {
                    eleman.dogrula_yol(v, &format!("{yol}[{i}]"))?;
                }
                Ok(())
            }
            SemaTipi::Metin { en_cok_uzunluk } => {
                let s = deger
                    .as_str()
                    .ok_or_else(|| GecersizArguman(format!("{yol}: metin bekleniyordu")))?;
                if en_cok_uzunluk.is_some_and(|n| s.chars().count() > n) {
                    return Err(GecersizArguman(format!("{yol}: metin cok uzun")));
                }
                Ok(())
            }
            SemaTipi::Secenek { secenekler } => {
                let s = deger
                    .as_str()
                    .ok_or_else(|| GecersizArguman(format!("{yol}: metin bekleniyordu")))?;
                if secenekler.iter().any(|x| x == s) {
                    Ok(())
                } else {
                    Err(GecersizArguman(format!("{yol}: gecersiz secenek '{s}'")))
                }
            }
            SemaTipi::Sayi { tam_mi, en_az, en_cok } => {
                let n = deger
                    .as_f64()
                    .ok_or_else(|| GecersizArguman(format!("{yol}: sayi bekleniyordu")))?;
                if *tam_mi && !deger.is_i64() && !deger.is_u64() {
                    return Err(GecersizArguman(format!("{yol}: tamsayi bekleniyordu")));
                }
                if en_az.is_some_and(|v| n < v) || en_cok.is_some_and(|v| n > v) {
                    return Err(GecersizArguman(format!("{yol}: aralik disinda")));
                }
                Ok(())
            }
            SemaTipi::Bool => deger
                .as_bool()
                .map(|_| ())
                .ok_or_else(|| GecersizArguman(format!("{yol}: dogru/yanlis bekleniyordu"))),
        }
    }
}
