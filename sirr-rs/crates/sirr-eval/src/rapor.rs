//! Eval raporu — metin tablosu ve JSON.
//!
//! IKI BICIM, TEK KAYNAK: metin tablosu insana (kosunun sonunda terminalde
//! okunur), JSON makineye (CI esigi, kosular arasi fark). Ikisi de AYNI
//! `EvalRaporu`dan turer; ayri uretilselerdi biri guncellenir digeri
//! unutulurdu ve "hangi sayi dogru" sorusu ortaya cikardi.
//!
//! BASARISIZ VAKALAR ONCE: rapor okunmak icin var. Gecen vakalarin listesi
//! ilgi cekmez; bakilan sey her zaman kirilanlardir.

use crate::kosucu::VakaSonucu;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct EvalRaporu {
    pub motor: String,
    pub toplam: usize,
    pub gecen: usize,
    /// 0.0 - 1.0. Vaka yoksa 0.0 — bos kosuyu "tam basari" saymak, vakalari
    /// yanlislikla filtreleyen bir degisikligi yesil gosterirdi.
    pub basari_orani: f64,
    pub vakalar: Vec<VakaSonucu>,
}

impl EvalRaporu {
    pub fn yeni(motor: &str, vakalar: Vec<VakaSonucu>) -> Self {
        let toplam = vakalar.len();
        let gecen = vakalar.iter().filter(|v| v.gecti).count();
        let basari_orani = if toplam == 0 { 0.0 } else { gecen as f64 / toplam as f64 };
        Self { motor: motor.to_string(), toplam, gecen, basari_orani, vakalar }
    }

    pub fn hepsi_gecti(&self) -> bool {
        self.toplam > 0 && self.gecen == self.toplam
    }

    /// Insan icin metin tablosu.
    pub fn tablo(&self) -> String {
        let ad_genisligi = self
            .vakalar
            .iter()
            .map(|v| v.ad.chars().count())
            .max()
            .unwrap_or(4)
            .max(4);

        let mut s = String::new();
        s.push_str(&format!("motor: {}\n\n", self.motor));
        s.push_str(&format!("{:<ad_genisligi$}  {:<6}  {}\n", "VAKA", "DURUM", "ARACLAR"));
        s.push_str(&format!("{}\n", "-".repeat(ad_genisligi + 40)));

        for v in &self.vakalar {
            let durum = if v.gecti { "gecti" } else { "KALDI" };
            s.push_str(&format!(
                "{:<ad_genisligi$}  {durum:<6}  {}\n",
                v.ad,
                if v.cagrilan.is_empty() { "-".to_string() } else { v.cagrilan.join(", ") }
            ));
        }

        let kalanlar: Vec<&VakaSonucu> = self.vakalar.iter().filter(|v| !v.gecti).collect();
        if !kalanlar.is_empty() {
            s.push_str("\nKALAN VAKALAR\n");
            for v in kalanlar {
                s.push_str(&format!("  {}\n", v.ad));
                for k in &v.kusurlar {
                    s.push_str(&format!("    - {k}\n"));
                }
            }
        }

        s.push_str(&format!(
            "\n{}/{} gecti  (%{:.1})\n",
            self.gecen,
            self.toplam,
            self.basari_orani * 100.0
        ));
        s
    }

    /// Makine icin JSON. Serilestirme basarisiz olamaz (tum alanlar duz veri);
    /// yine de panige donusmesin diye hata metni doner.
    pub fn json(&self) -> String {
        serde_json::to_string_pretty(self)
            .unwrap_or_else(|e| format!("{{\"hata\":\"rapor serilestirilemedi: {e}\"}}"))
    }
}
