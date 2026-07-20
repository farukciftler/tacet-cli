//! TokenSayaci — kaba belirtec tahmini + baglam butcesi kapisi.
//!
//! NEDEN TAHMIN: gercek sayim belirtecleyiciyi (dolayisiyla `tokenizers`
//! bagimliligini ve yuklu bir model dosyasini) gerektirir. Kirpma kararinin
//! agirliklar yuklenmeden, SahteMotor uzerinde ve CI'da verilebilmesi sart —
//! aksi halde butce mantigi yalnizca gercek modelle test edilebilirdi, yani
//! pratikte hic test edilmezdi. Gercek sayim eldeyken `olcum_ile` ile
//! degistirilebilir; politika ayni kalir.
//!
//! GUVENLI YON: tahmin bilerek YUKSEK cikar. Az tahmin edip butceyi asmak
//! modelin istemi ortasindan kesmesi demektir (sessiz, teshis edilemez bir
//! ariza); fazla tahmin edip bir tur fazladan dusurmek yalnizca biraz baglam
//! kaybettirir.

use crate::istem::{Istem, Tur};

/// Cihaz-ustu modelin baglam penceresi. Mimarinin sabiti (bkz. 4096 kanali).
pub const BAGLAM_BUTCESI: usize = 4096;

/// Uretime ayrilan pay; istem bundan geriye kalanina sigmalidir.
///
/// Istem butcenin tamamini doldurabilseydi modelin cevap yazacak yeri
/// kalmazdi — tam dolu bir istem sifir belirteclik bir cevap uretir.
pub const URETIM_PAYI: usize = 512;

/// Bir belirtece dusen ortalama BAYT — 5/2 = 2.5, kesirli oldugu icin pay ve
/// paydasi ayri.
///
/// GERCEK MODELDE OLCULDU, ve onceki deger (3) YANLIS CIKTI. Qwen2.5
/// belirtecleyicisiyle Turkce duzyazi ortalama **2.71 bayt/belirtec** veriyor.
/// Yani bayti 3'e bolen eski tahmin, "guvenli yon" gerekcesinin tam TERSINI
/// yapiyordu: gercegin ~%10 ALTINDA kaliyordu.
///
/// Somut ariza: 200 turluk bir gecmis kirpildiginda tahmin 3561 diyip 3584
/// tavanina "sigdi" karari veriyor, `dogrula()` basariyla donuyordu; ayni
/// istemin GERCEK belirtec sayisi ise 3937 idi. Uretim payiyla birlikte
/// (3937 + 512) 4096 penceresi ASILIYORDU — tam da bu dosyanin bastan
/// onlemeye calistigi sessiz kesilme.
///
/// 2.5 secildi, 2.71 degil: olcum tek bir metin turunun ortalamasidir ve
/// belirtec yogunlugu icerige gore oynar (kod ve Ingilizce daha seyrek, yogun
/// Turkce cekim daha sik). Aradaki ~%8 pay bilincli emniyet payidir. Tahminin
/// YUKSEK cikmasi ucuzdur (biraz baglam kaybi), DUSUK cikmasi pahali
/// (teshis edilemez kesilme).
const BAYT_PAYI: usize = 2;
const BAYT_PAYDASI: usize = 5;

pub struct TokenSayaci {
    /// Toplam pencere.
    pub butce: usize,
    /// Uretime ayrilan pay.
    pub uretim_payi: usize,
}

impl Default for TokenSayaci {
    fn default() -> Self {
        Self { butce: BAGLAM_BUTCESI, uretim_payi: URETIM_PAYI }
    }
}

/// Kirpmanin ne yaptigi — cagri yeri bunu loglar, testler bunu dogrular.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KirpmaRaporu {
    /// Bastan dusurulen eski tur sayisi.
    pub dusen_tur: usize,
    /// Kilavuz feda edildi mi.
    pub kilavuz_dustu: bool,
    /// Sorunun kendisi kirpildi mi (son care).
    pub soru_kirpildi: bool,
    /// Kirpma sonrasi tahmini belirtec sayisi.
    pub son_tahmin: usize,
}

impl KirpmaRaporu {
    pub fn degisti_mi(&self) -> bool {
        self.dusen_tur > 0 || self.kilavuz_dustu || self.soru_kirpildi
    }
}

impl TokenSayaci {
    pub fn yeni(butce: usize, uretim_payi: usize) -> Self {
        Self { butce, uretim_payi }
    }

    /// Istemin sigmasi gereken tavan.
    pub fn istem_tavani(&self) -> usize {
        self.butce.saturating_sub(self.uretim_payi)
    }

    /// Kaba belirtec tahmini.
    ///
    /// UTF-8 BAYT uzunlugu kullanilir, karakter degil: Turkce harfler (ç, ğ,
    /// ş, ı, ö, ü) cok baytlidir ve belirtecleyiciler de tam olarak o
    /// noktalarda fazladan parcalar. Karakter saysaydik Turkce metni
    /// sistematik olarak AZ tahmin ederdik.
    pub fn tahmin(metin: &str) -> usize {
        (metin.len() * BAYT_PAYI).div_ceil(BAYT_PAYDASI)
    }

    /// Istemin tamaminin tahmini.
    pub fn istem_tahmini(&self, istem: &Istem) -> usize {
        Self::tahmin(&istem.metin())
    }

    /// Butce asiliyorsa istemi POLITIKAYA gore kucultur.
    ///
    /// SIRA (en ucuzdan en pahaliya kayip):
    ///   1. En ESKI turlar dusurulur — kaybi en az olan baglam odur.
    ///   2. Kilavuz dusurulur — rehberligi kaybetmek isi zorlastirir ama
    ///      araclar ve soru yerinde durdugu icin tur yine yurur.
    ///   3. Son care: soru SONDAN korunarak kirpilir.
    ///
    /// SISTEM TALIMATI VE ARAC TARIFI HIC KIRPILMAZ. Talimat kirpilirsa model
    /// kim oldugunu ve hangi dilde cevap verecegini unutur; arac tarifi
    /// kirpilirsa var olmayan bir imza uydurur. Ikisi de gecmis kaybindan cok
    /// daha pahali arizalar.
    pub fn kirp(&self, istem: &mut Istem) -> KirpmaRaporu {
        let tavan = self.istem_tavani();
        let mut rapor = KirpmaRaporu::default();

        // 1) Eski turlari bastan dusur.
        while self.istem_tahmini(istem) > tavan && !istem.gecmis.is_empty() {
            istem.gecmis.remove(0);
            rapor.dusen_tur += 1;
        }

        // 2) Kilavuzu feda et.
        if self.istem_tahmini(istem) > tavan && istem.kilavuz.is_some() {
            istem.kilavuz = None;
            rapor.kilavuz_dustu = true;
        }

        // 3) Son care: soruyu kirp. SONU korunur — kullanicinin asil talebi
        // cumlenin sonunda olur ("...bunu tablo yap"), basi baglam kurar.
        if self.istem_tahmini(istem) > tavan {
            let fazla = self.istem_tahmini(istem) - tavan;
            let atilacak_bayt = fazla * BAYT_PAYDASI / BAYT_PAYI;
            let yeni = son_bayttan(&istem.soru, istem.soru.len().saturating_sub(atilacak_bayt));
            if yeni.len() < istem.soru.len() {
                istem.soru = yeni;
                rapor.soru_kirpildi = true;
            }
        }

        rapor.son_tahmin = self.istem_tahmini(istem);
        rapor
    }

    /// Kirpmadan sonra bile sigmiyorsa hata — cagri yeri sessizce devam
    /// etmesin, cunku o noktada model istemin ortasindan kesilmis bir metin
    /// gorur ve teshisi imkansiz sacmaliklar uretir.
    pub fn dogrula(&self, istem: &Istem) -> crate::hata::MotorSonuc<()> {
        let olculen = self.istem_tahmini(istem);
        if olculen > self.istem_tavani() {
            return Err(crate::hata::MotorHatasi::ButceAsimi {
                olculen,
                butce: self.istem_tavani(),
            });
        }
        Ok(())
    }

    /// Tur listesinin tahmini — cagri yerlerinin kirpma oncesi karar vermesi icin.
    pub fn turler_tahmini(turler: &[Tur]) -> usize {
        turler.iter().map(|t| Self::tahmin(&t.metin) + 4).sum()
    }
}

/// `metin`in son `en_cok_bayt` baytini, KARAKTER SINIRINA saygi gostererek
/// dondurur. Ham `&s[i..]` cok baytli bir harfin ortasina denk gelip panik
/// ederdi — Turkce metinde bu istisna degil, kural.
fn son_bayttan(metin: &str, en_cok_bayt: usize) -> String {
    if en_cok_bayt >= metin.len() {
        return metin.to_string();
    }
    let mut baslangic = metin.len() - en_cok_bayt;
    while baslangic < metin.len() && !metin.is_char_boundary(baslangic) {
        baslangic += 1;
    }
    metin[baslangic..].to_string()
}
