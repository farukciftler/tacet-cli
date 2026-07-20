//! Istem kurulumu — modele giden metnin TEK uretim yeri.
//!
//! Istem parcalanmis halde tutulur (sistem / arac tarifi / gecmis / kilavuz /
//! soru) ve ancak `metin()` cagrisinda birlestirilir. Gerekce: baglam butcesi
//! asildiginda kirpma politikasi hangi parcanin feda edilebilecegini bilmek
//! zorunda. Istem tek bir `String` olsaydi kirpma "sondan kes"ten ibaret
//! kalir, ilk kurban her zaman sorunun kendisi olurdu.
//!
//! BECERI KARARI (Swift tarafindan devraliniyor, olcumle alinmis): kilavuz
//! metni SISTEM TALIMATINA GOMULMEZ. Olcum, kucuk cihaz-ustu modelin fazla
//! sabit talimat altinda araci CAGIRMAK yerine ne yapacagini ANLATMAYA
//! basladigini gosterdi. Bunun yerine o mesaja uyan TEK beceri, o turun
//! istemine `<guidance>` citiyle ve `KILAVUZ_SINIRI` karakterle sinirli
//! olarak iliştirilir. Kilavuzun sorunun HEMEN ONUNDE durmasi da bilincli:
//! kucuk modelde son bloklarin agirligi en yuksektir.

use sirr_cekirdek::AracKatalogu;

/// Tek enjeksiyonda kilavuzdan alinacak en fazla karakter.
///
/// Swift tarafiyla ayni sayi (700). Daha buyugu 4096 pencerede gecmisi
/// yiyor, daha kucugu somut `arac(args)` ornegini ve anti-halusinasyon
/// kurallarini kesiyordu — kilavuzun var olma sebebi tam da o kisim.
pub const KILAVUZ_SINIRI: usize = 700;

/// Bir konusma turunun kaynagi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rol {
    Kullanici,
    Asistan,
    /// Arac sonucunun modele donen KISA metni. Toplu veri burada olmaz
    /// (bkz. VeriDeposu bypass kanali).
    Arac,
}

impl Rol {
    fn etiket(self) -> &'static str {
        match self {
            Rol::Kullanici => "Kullanici",
            Rol::Asistan => "Asistan",
            Rol::Arac => "Arac",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Tur {
    pub rol: Rol,
    pub metin: String,
}

impl Tur {
    pub fn kullanici(metin: impl Into<String>) -> Self {
        Self { rol: Rol::Kullanici, metin: metin.into() }
    }
    pub fn asistan(metin: impl Into<String>) -> Self {
        Self { rol: Rol::Asistan, metin: metin.into() }
    }
    pub fn arac(metin: impl Into<String>) -> Self {
        Self { rol: Rol::Arac, metin: metin.into() }
    }

    fn yaz(&self, hedef: &mut String) {
        hedef.push_str(self.rol.etiket());
        hedef.push_str(": ");
        hedef.push_str(&self.metin);
        hedef.push('\n');
    }
}

/// Modele gonderilecek istemin parcali hali.
#[derive(Debug, Clone, Default)]
pub struct Istem {
    /// Sabit oturum talimati. KIRPILMAZ — kimlik, dil capasi ve arac
    /// cagirma sozlesmesi burada; kirpilirsa model ne oldugunu unutur.
    pub sistem: String,
    /// Katalogdan turetilmis arac tarifi. Kirpilmaz: eksik tarif, modelin
    /// var olmayan bir imza uydurmasi demektir.
    pub araclar: String,
    /// Eski turlar. Butce baskisinda ILK feda edilen budur.
    pub gecmis: Vec<Tur>,
    /// O mesaja uyan TEK beceri kilavuzu, `<guidance>` citli. Sorunun hemen onunde.
    pub kilavuz: Option<String>,
    /// Bu turun kullanici sorusu. Kirpilir ama ASLA dusurulmez.
    pub soru: String,
}

impl Istem {
    pub fn yeni(sistem: impl Into<String>, soru: impl Into<String>) -> Self {
        Self { sistem: sistem.into(), soru: soru.into(), ..Default::default() }
    }

    /// Arac tarifini KATALOGDAN turetir — elle yazilmis ikinci bir liste yok.
    ///
    /// Sira katalogun `Vec` sirasidir: ayni katalog her calistirmada
    /// bit-birebir ayni istemi uretsin, eval sonuclari karsilastirilabilir olsun.
    pub fn araclarla(mut self, katalog: &AracKatalogu) -> Self {
        let mut m = String::new();
        for arac in katalog.araclar() {
            m.push_str("- ");
            m.push_str(arac.ad());
            m.push_str(": ");
            m.push_str(arac.aciklama());
            m.push('\n');
            m.push_str("  args: ");
            // json_schema tek satira serilestirilir: cok satirli sema
            // butcenin kaydadeger bir kismini girintiye harciyordu.
            m.push_str(&arac.sema().json_schema().to_string());
            m.push('\n');
        }
        self.araclar = m;
        self
    }

    /// Kilavuzu `KILAVUZ_SINIRI` karakterle SINIRLAYARAK ekler.
    ///
    /// TODO(beceri turu) — BU METOT SU AN URETIMDE CAGRILMIYOR. Kirpma
    /// mantigi ve 700 siniri yazili ve testli, ama onu besleyecek `BeceriDeposu`
    /// (mesaja uyan TEK beceriyi secen, tur-mesafeli tekrar engeli olan katman)
    /// BU TURUN KAPSAMI DISINDA — web arama, MCP, hafiza ve UI ile birlikte
    /// bilincli olarak ertelendi (bkz. README "Kapsam disi", DURUM.md).
    /// Dolayisiyla 700 siniri bugun bir birim testini geciyor, uretim yolunu
    /// degil. Beceri turu geldiginde baglanacak tek nokta burasidir:
    /// `Istem::yeni(...).araclarla(...).kilavuzla(secilen_beceri)`.
    ///
    /// Kesme BASTAN alir (sondan degil): beceri dosyalari "cekirdek-once"
    /// yazilir — somut cagri ornegi ve kirilmaz kurallar en ustte durur.
    /// Sondan kesmek tam da o cekirdegi birakip insan referansini atardi;
    /// bastan almak tersini yapar.
    pub fn kilavuzla(mut self, kilavuz: impl AsRef<str>) -> Self {
        let k = kilavuz.as_ref();
        let kirpilmis: String = k.chars().take(KILAVUZ_SINIRI).collect();
        self.kilavuz = (!kirpilmis.trim().is_empty()).then_some(kirpilmis);
        self
    }

    pub fn gecmisle(mut self, turler: impl IntoIterator<Item = Tur>) -> Self {
        self.gecmis = turler.into_iter().collect();
        self
    }

    /// Parcalari tek metne birlestirir. Sira sabittir ve anlamlidir:
    /// talimat -> araclar -> gecmis -> kilavuz -> soru. Soru EN SONDA kalir.
    pub fn metin(&self) -> String {
        let mut m = String::with_capacity(self.kaba_uzunluk());
        m.push_str("<sistem>\n");
        m.push_str(self.sistem.trim());
        m.push_str("\n</sistem>\n");

        if !self.araclar.trim().is_empty() {
            m.push_str("<araclar>\n");
            m.push_str(self.araclar.trim_end());
            m.push_str("\n</araclar>\n");
        }

        if !self.gecmis.is_empty() {
            m.push_str("<gecmis>\n");
            for t in &self.gecmis {
                t.yaz(&mut m);
            }
            m.push_str("</gecmis>\n");
        }

        if let Some(k) = &self.kilavuz {
            m.push_str("<guidance>\n");
            m.push_str(k.trim());
            m.push_str("\n</guidance>\n");
        }

        m.push_str("Kullanici: ");
        m.push_str(self.soru.trim());
        m.push_str("\nAsistan: ");
        m
    }

    fn kaba_uzunluk(&self) -> usize {
        self.sistem.len()
            + self.araclar.len()
            + self.soru.len()
            + self.kilavuz.as_ref().map_or(0, String::len)
            + self.gecmis.iter().map(|t| t.metin.len() + 12).sum::<usize>()
            + 64
    }
}
