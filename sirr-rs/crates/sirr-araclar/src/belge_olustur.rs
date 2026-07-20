//! Belge uretimi — `belge_olustur` araci, `BelgeMotoru` sozlesmesi ve iki motor
//! (Excel/xlsx, duz metin/markdown).
//!
//! TEK YAZMA KAPISI: serbest fonksiyon `belge_yaz`. Trait yalniz `yaz_ham`
//! tanimlar; klasor hazirlama, benzersiz ad uretimi ve yazma sonrasi dosya
//! izni (0600) tek yerde, trait'in DISINDA durur. Swift tarafinda ayni yapi
//! FileProtection'in unutulmasini imkansiz kilmisti. Onceden bu `yaz` adli
//! varsayilan govdeli bir trait metoduydu ve "motorlar gecersiz kilmaz"
//! yorumuna guveniyordu — Rust'ta bunu engelleyen bir sey olmadigi icin yeni
//! bir motor izin damgasini sessizce dusurebilirdi. Serbest fonksiyonda
//! gecersiz kilinacak bir sey yok. `yaz_ham` hedef yolu HAZIR alir — dosya
//! adini secme yetkisi bile motorda degil, boylece iki motor iki farkli
//! adlandirma kurali uretemez.
//!
//! BYPASS KANALI: `kaynak_ref` verilmisse veri modelden GECMEZ; govde depodan
//! okunur. Ve ref BAGLAYICIDIR: cozulemeyen ref sessizce `icerik`e dusmez.
//! Swift'te en tehlikeli hata sinifi buydu — ref cozulmeyince `icerik` de bos
//! oldugu icin BOS dosya yazilip "olusturuldu" raporlaniyordu; kullanici dolu
//! sandigi dosyayi tasiyordu. Burada dosya HIC yazilmaz.

use std::fs;
use std::path::{Path, PathBuf};

use sirr_cekirdek::{
    Alan, Arac, AracBaglami, AracGelecegi, AracHatasi, AracSonuc, AracSonucu, ArgSema, AracDurumu,
    IzGuncelleme, KaynakRef, kutula,
};

use crate::veri_deposu::Tablo;

/// Ciktilarin yazildigi, calisma dizinine gore sabit alt klasor.
///
/// Sabit olmasi bilincli: model dosyanin nereye yazilacagini secemez. Yol
/// argumani olsaydi kum havuzu kapisi tek savunma hatti olurdu; boyle iki kat
/// var (once sabit klasor, sonra `yolu_coz`).
pub const CIKTI_KLASORU: &str = "sirr";

/// Modele donen ozetin en fazla kac satir tasiyacagi.
const OZET_SATIR: usize = 12;

// ---------------------------------------------------------------------------
// Bicim
// ---------------------------------------------------------------------------

/// Uretilebilen dosya bicimleri.
///
/// Kapali bir kume, serbest metin DEGIL. Swift'te bicim `.contains` ile bulanik
/// cozuluyordu ve eslesmeyen her deger sessizce duz metne dusuyordu: kullanici
/// "excel" isteyip .txt aliyordu. Semada `Secenek` olarak durdugu icin gramer
/// gecersiz degeri URETILEMEZ yapiyor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BelgeBicimi {
    Excel,
    Markdown,
    Metin,
}

impl BelgeBicimi {
    /// Semadaki `Secenek` degerleri ile birebir ayni sira — gramer ve istem her
    /// calistirmada bit-birebir ayni ciksin.
    pub const HEPSI: [BelgeBicimi; 3] =
        [BelgeBicimi::Excel, BelgeBicimi::Markdown, BelgeBicimi::Metin];

    pub fn anahtar(&self) -> &'static str {
        match self {
            BelgeBicimi::Excel => "excel",
            BelgeBicimi::Markdown => "markdown",
            BelgeBicimi::Metin => "metin",
        }
    }

    pub fn uzanti(&self) -> &'static str {
        match self {
            BelgeBicimi::Excel => "xlsx",
            BelgeBicimi::Markdown => "md",
            BelgeBicimi::Metin => "txt",
        }
    }

    /// Kullaniciya gosterilen cip etiketi.
    pub fn etiket(&self) -> &'static str {
        match self {
            BelgeBicimi::Excel => "Excel",
            BelgeBicimi::Markdown => "Markdown",
            BelgeBicimi::Metin => "Metin",
        }
    }

    pub fn ikon(&self) -> &'static str {
        match self {
            BelgeBicimi::Excel => "tablo",
            BelgeBicimi::Markdown => "belge",
            BelgeBicimi::Metin => "belge",
        }
    }

    /// Bu bicim yapilandirilmis tablo bekliyor mu? Yalniz xlsx.
    pub fn tablo_yapisi(&self) -> bool {
        matches!(self, BelgeBicimi::Excel)
    }

    pub fn coz(ham: &str) -> Option<BelgeBicimi> {
        BelgeBicimi::HEPSI
            .into_iter()
            .find(|b| b.anahtar().eq_ignore_ascii_case(ham.trim()))
    }
}

// ---------------------------------------------------------------------------
// BelgeMotoru sozlesmesi
// ---------------------------------------------------------------------------

/// Tek bicimden sorumlu motor.
pub trait BelgeMotoru: Send + Sync {
    fn bicim(&self) -> BelgeBicimi;

    /// Ham yazma. `hedef` COZULMUS ve benzersiz bir yoldur; motor yalnizca
    /// icerigi uretip diske doker. Cagiranlar bunu KULLANMAZ — `belge_yaz`
    /// uzerinden gidilir.
    fn yaz_ham(
        &self,
        hedef: &Path,
        baslik: Option<&str>,
        govde: Option<&str>,
        tablo: Option<&Tablo>,
    ) -> AracSonuc<()>;
}

/// TEK YAZMA KAPISI.
///
/// NEDEN TRAIT METODU DEGIL SERBEST FONKSIYON: bu daha once `BelgeMotoru`nun
/// varsayilan govdeli `yaz` metoduydu ve yorumunda "motorlar bunu gecersiz
/// kilmaz" yaziyordu — ama Rust'ta bunu engelleyen hicbir sey yoktu. Yeni bir
/// motor kendi `fn yaz(...)`ini yazsa `klasor_hazirla` ve `son_islemler` (0600
/// izin damgasi) SESSIZCE atlanirdi; ne derleyici ne test uyarirdi. Serbest
/// fonksiyon olarak gecersiz kilinacak bir sey kalmaz: genisleme noktasi
/// yalniz `yaz_ham`dir ve ortak adimlar her yolda calisir.
pub fn belge_yaz(
    motor: &dyn BelgeMotoru,
    dosya_adi: &str,
    baslik: Option<&str>,
    govde: Option<&str>,
    tablo: Option<&Tablo>,
    klasor: &Path,
) -> AracSonuc<PathBuf> {
    klasor_hazirla(klasor)?;
    let hedef = hedef_yol(dosya_adi, motor.bicim(), klasor);
    motor.yaz_ham(&hedef, baslik, govde, tablo)?;
    son_islemler(&hedef)?;
    Ok(hedef)
}

fn klasor_hazirla(klasor: &Path) -> AracSonuc<()> {
    fs::create_dir_all(klasor)?;
    Ok(())
}

/// Yazma sonrasi ortak adim: dosyayi yalniz sahibine okunur/yazilir yapar.
///
/// Swift'teki `korumayiUygula` karsiligi. Uretilen belge kullanicinin kisisel
/// verisidir; varsayilan umask'a guvenmek yerine izni ACIKCA basiyoruz.
/// Unix disinda sessizce gecilir — orada dosya izni ayni anlami tasimiyor.
fn son_islemler(hedef: &Path) -> AracSonuc<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(hedef, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        let _ = hedef;
    }
    Ok(())
}

/// Benzersiz, guvenli hedef yol uretir (cakisma varsa `-2`, `-3` ... ekler).
///
/// Ad temizligi sert: yol ayraci ve `..` gibi bilesenler ADA girmez. Kum havuzu
/// kapisi zaten var ama dosya adi hicbir zaman yol olusturamamali — iki kat
/// savunma.
fn hedef_yol(dosya_adi: &str, bicim: BelgeBicimi, klasor: &Path) -> PathBuf {
    let temiz: String = dosya_adi
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '\0' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let temiz = temiz.trim().trim_matches('.').trim().to_string();
    let taban = if temiz.is_empty() { "sirr-belge".to_string() } else { temiz };

    let mut aday = klasor.join(format!("{taban}.{}", bicim.uzanti()));
    let mut i = 2;
    while aday.exists() {
        aday = klasor.join(format!("{taban}-{i}.{}", bicim.uzanti()));
        i += 1;
    }
    aday
}

// ---------------------------------------------------------------------------
// MetinMotor
// ---------------------------------------------------------------------------

/// Duz metin / markdown yazar. Ayni motor iki bicime hizmet eder: fark yalniz
/// uzanti, icerik uretimi birebir ayni.
#[derive(Debug, Clone, Copy)]
pub struct MetinMotor {
    bicim: BelgeBicimi,
}

impl MetinMotor {
    pub fn yeni(bicim: BelgeBicimi) -> Self {
        Self { bicim }
    }
}

impl BelgeMotoru for MetinMotor {
    fn bicim(&self) -> BelgeBicimi {
        self.bicim
    }

    fn yaz_ham(
        &self,
        hedef: &Path,
        baslik: Option<&str>,
        govde: Option<&str>,
        tablo: Option<&Tablo>,
    ) -> AracSonuc<()> {
        let mut icerik = String::new();
        if let Some(b) = baslik.map(str::trim).filter(|b| !b.is_empty()) {
            // Markdown'da baslik isaretlensin; duz metinde de zarar vermez ama
            // gurultu olur, o yuzden yalniz md'de '#' basiyoruz.
            if self.bicim == BelgeBicimi::Markdown {
                icerik.push_str("# ");
            }
            icerik.push_str(b);
            icerik.push_str("\n\n");
        }
        // Govde yoksa tablo markdown'a cevrilir: veri kaybetmektense bicim
        // degistirmek dogru. Ikisi de yoksa BOS DOSYA YAZMAYIZ — bos ama
        // "olusturuldu" denen dosya kullaniciyi yaniltan en pahali sonuctur.
        let metin = match (govde.map(str::trim).filter(|g| !g.is_empty()), tablo) {
            (Some(g), _) => g.to_string(),
            (None, Some(t)) if !t.basliklar.is_empty() => t.markdown_kirpik(usize::MAX),
            _ => {
                return Err(AracHatasi::GecersizArguman(
                    "belge icin icerik yok: 'icerik' ya da cozulebilir bir 'kaynak_ref' ver"
                        .into(),
                ));
            }
        };
        icerik.push_str(&metin);
        if !icerik.ends_with('\n') {
            icerik.push('\n');
        }
        fs::write(hedef, icerik.as_bytes())?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// ExcelMotor
// ---------------------------------------------------------------------------

/// .xlsx (OOXML SpreadsheetML) yazar. Paketleme sirr-zip ile (STORE).
///
/// Hucreler `inlineStr`; sharedStrings tablosu KULLANILMAZ. Gerekce: paylasilan
/// dizge tablosu ikinci bir indeks tutmak demek, indeks ile hucre arasindaki
/// tutarlilik el yazimi uretici icin fazladan bir hata yuzeyi. Cihazda uretilen
/// kucuk tablolarda boyut kazanci bu riski karsilamiyor.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExcelMotor;

impl ExcelMotor {
    pub fn yeni() -> Self {
        Self
    }
}

impl BelgeMotoru for ExcelMotor {
    fn bicim(&self) -> BelgeBicimi {
        BelgeBicimi::Excel
    }

    fn yaz_ham(
        &self,
        hedef: &Path,
        baslik: Option<&str>,
        govde: Option<&str>,
        tablo: Option<&Tablo>,
    ) -> AracSonuc<()> {
        let (basliklar, satirlar) = tabloya_indir(baslik, govde, tablo)?;
        let sheet = sheet_xml(&basliklar, &satirlar);

        let girisler = vec![
            sirr_zip::ZipGiris::yeni("[Content_Types].xml", CONTENT_TYPES_XML.as_bytes()),
            sirr_zip::ZipGiris::yeni("_rels/.rels", RELS_XML.as_bytes()),
            sirr_zip::ZipGiris::yeni("xl/workbook.xml", WORKBOOK_XML.as_bytes()),
            sirr_zip::ZipGiris::yeni("xl/_rels/workbook.xml.rels", WORKBOOK_RELS_XML.as_bytes()),
            sirr_zip::ZipGiris::yeni("xl/styles.xml", STYLES_XML.as_bytes()),
            sirr_zip::ZipGiris::yeni("xl/worksheets/sheet1.xml", sheet.into_bytes()),
        ];

        let paket = sirr_zip::paketle(&girisler)
            .map_err(|e| AracHatasi::Diger(format!("xlsx paketlenemedi: {e}")))?;
        fs::write(hedef, paket)?;
        Ok(())
    }
}

/// xlsx'in tek girdisi bir tablodur. Yapilandirilmis tablo yoksa govdeden
/// KURTARILMAYA calisilir; kurtarilamiyorsa ACIK HATA doner.
///
/// Buradaki sessiz dusus dali Swift'te bilerek kaldirildi: govde satir satir TEK
/// SUTUNA dokuluyordu. Model markdown tablosu yazdiginda ("| Ad | Yas |") sonuc
/// her satiri tek hucreye sikismis cop bir xlsx oluyor, ustelik arac
/// "olusturuldu" diye BASARI raporluyordu. Kural:
/// - govde ayristirilabilir tabloysa -> gercek tabloya cevrilir,
/// - govdede boru var ama tablo cikmiyorsa -> hata (model bir kez duzeltir),
/// - govde borusuz duz liste ise -> tek sutun MESRUDUR,
/// - tablo da govde de yoksa -> hata.
fn tabloya_indir(
    baslik: Option<&str>,
    govde: Option<&str>,
    tablo: Option<&Tablo>,
) -> AracSonuc<(Vec<String>, Vec<Vec<String>>)> {
    if let Some(t) = tablo.filter(|t| !t.basliklar.is_empty()) {
        if !t.satirlar.is_empty() {
            return Ok((t.basliklar.clone(), t.satirlar.clone()));
        }
        // Baslik var, satir yok: govdeden satir kurtarmayi dene.
        if let Some(a) = govde.and_then(markdown_dan).filter(|a| !a.satirlar.is_empty()) {
            return Ok((a.basliklar, a.satirlar));
        }
        return Ok((t.basliklar.clone(), Vec::new()));
    }

    let metin = govde.unwrap_or("").trim();
    if metin.is_empty() {
        return Err(AracHatasi::GecersizArguman(
            "elektronik tablo icin icerik yok: sutun basliklari ve satirlari olan bir tablo ver"
                .into(),
        ));
    }

    // 1) Govde bir markdown tablosu mu? (ayrac satiri opsiyonel)
    if let Some(a) = markdown_dan(metin).filter(|a| !a.satirlar.is_empty()) {
        return Ok((a.basliklar, a.satirlar));
    }

    // 2) Boru var ama tablo cikmadi -> sessizce tek sutuna dokme, DUZELTTIR.
    if metin.contains('|') {
        return Err(AracHatasi::GecersizArguman(
            "tablo ayristirilamadi: baslik satiri ve veri satirlari olan, her satiri bir satir \
             ve hucreleri | ile ayrilmis bir tablo ver"
                .into(),
        ));
    }

    // 3) Borusuz duz liste: tek sutun mesru sonuctur.
    let dilimler: Vec<Vec<String>> = metin
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| vec![s.to_string()])
        .collect();
    if dilimler.is_empty() {
        return Err(AracHatasi::GecersizArguman(
            "elektronik tablo icin icerik yok: sutun basliklari ve satirlari olan bir tablo ver"
                .into(),
        ));
    }
    let bas = baslik.map(str::trim).filter(|b| !b.is_empty()).unwrap_or("Metin");
    Ok((vec![bas.to_string()], dilimler))
}

/// Sayfa XML'i. Tamamen sayisal kolonlarin altina GERCEK `=SUM()` formulu yazar.
///
/// Hesaplanmis sabit degil FORMUL yazmak bilincli bir karar: kullanici dosyayi
/// acip bir hucreyi degistirdiginde toplam kendini guncellemeli. Sabit yazsaydik
/// belge acildigi anda sessizce yanlislasabilecek bir sayi tasirdi.
fn sheet_xml(basliklar: &[String], satirlar: &[Vec<String>]) -> String {
    let sutun_sayisi = basliklar.len();

    // Sayisal kolon tespiti: veri satirlarinda o kolonun TUM dolu hucreleri sayi.
    let mut sayisal_kolon = vec![false; sutun_sayisi];
    for (k, bayrak) in sayisal_kolon.iter_mut().enumerate() {
        let mut en_az_bir = false;
        let mut hepsi_sayi = true;
        for s in satirlar {
            let Some(h) = s.get(k).map(|h| h.trim()) else { continue };
            if h.is_empty() {
                continue;
            }
            en_az_bir = true;
            if !sayi_mi(h) {
                hepsi_sayi = false;
                break;
            }
        }
        *bayrak = en_az_bir && hepsi_sayi;
    }
    let sayisal_var = sayisal_kolon.iter().any(|b| *b);

    let mut govde = String::new();

    // 1. satir: basliklar (hepsi inlineStr).
    govde.push_str("<row r=\"1\">");
    for (k, b) in basliklar.iter().enumerate() {
        govde.push_str(&inline_hucre(&format!("{}1", kolon_harfi(k)), b));
    }
    govde.push_str("</row>");

    // Veri satirlari.
    for (idx, s) in satirlar.iter().enumerate() {
        let r = idx + 2;
        govde.push_str(&format!("<row r=\"{r}\">"));
        for k in 0..sutun_sayisi {
            let hucre_ref = format!("{}{r}", kolon_harfi(k));
            let bos = String::new();
            let deger = s.get(k).unwrap_or(&bos);
            let kirp = deger.trim();
            if !kirp.is_empty() && sayi_mi(kirp) {
                govde.push_str(&format!("<c r=\"{hucre_ref}\"><v>{kirp}</v></c>"));
            } else {
                govde.push_str(&inline_hucre(&hucre_ref, deger));
            }
        }
        govde.push_str("</row>");
    }

    // Ozet (Toplam) satiri: en az bir sayisal kolon varsa.
    if sayisal_var && !satirlar.is_empty() {
        let ilk_veri = 2;
        let son_veri = satirlar.len() + 1;
        let toplam_satir = son_veri + 1;
        govde.push_str(&format!("<row r=\"{toplam_satir}\">"));
        for (k, sayisal) in sayisal_kolon.iter().enumerate() {
            let hucre_ref = format!("{}{toplam_satir}", kolon_harfi(k));
            if k == 0 {
                govde.push_str(&inline_hucre(&hucre_ref, "Toplam"));
            } else if *sayisal {
                let harf = kolon_harfi(k);
                govde.push_str(&format!(
                    "<c r=\"{hucre_ref}\"><f>SUM({harf}{ilk_veri}:{harf}{son_veri})</f></c>"
                ));
            } else {
                govde.push_str(&inline_hucre(&hucre_ref, ""));
            }
        }
        govde.push_str("</row>");
    }

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
         <sheetData>{govde}</sheetData></worksheet>"
    )
}

fn inline_hucre(hucre_ref: &str, metin: &str) -> String {
    format!(
        "<c r=\"{hucre_ref}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>",
        xml_kacir(metin)
    )
}

/// Excel'in `<v>` icinde kabul ettigi sayi bicimi.
///
/// `f64::parse` yetmiyor: Rust "inf", "NaN", "1e5" gibi degerleri de ayristirir,
/// oysa bunlar sayfaya ham yazilinca ya gecersiz XML degeri olur ya da Excel'de
/// bambaska gorunur. Metin olarak yazilmalari daha dogru.
fn sayi_mi(ham: &str) -> bool {
    if ham.is_empty() {
        return false;
    }
    let govde = ham.strip_prefix('-').unwrap_or(ham);
    if govde.is_empty() {
        return false;
    }
    let mut nokta = false;
    let mut rakam = false;
    for c in govde.chars() {
        match c {
            '0'..='9' => rakam = true,
            '.' if !nokta => nokta = true,
            _ => return false,
        }
    }
    rakam
}

/// Sutun indisini (0 tabanli) harfe cevirir: 0->A, 25->Z, 26->AA.
fn kolon_harfi(indis: usize) -> String {
    let mut n = indis as i64;
    let mut s = String::new();
    loop {
        let r = (n % 26) as u8;
        s.insert(0, (b'A' + r) as char);
        n = n / 26 - 1;
        if n < 0 {
            break;
        }
    }
    s
}

fn xml_kacir(metin: &str) -> String {
    let mut s = String::with_capacity(metin.len());
    for c in metin.chars() {
        match c {
            '&' => s.push_str("&amp;"),
            '<' => s.push_str("&lt;"),
            '>' => s.push_str("&gt;"),
            '"' => s.push_str("&quot;"),
            '\'' => s.push_str("&apos;"),
            // XML 1.0 bu araligi HICBIR kacisla kabul etmez; dusurmek tek
            // secenek, aksi halde uretilen dosya acilamaz.
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' && c != '\r' => {}
            c => s.push(c),
        }
    }
    s
}

// ---------------------------------------------------------------------------
// Sabit OOXML parcalari
// ---------------------------------------------------------------------------
//
// Bu bes parca her xlsx'te birebir ayni. Sablon motoru yok, olmasi da
// gerekmiyor: degisen tek parca sheet1.xml. Parcalar arasindaki bagi
// (Content_Types -> parca yolu -> rels hedefi) bozmadan duzenle; biri
// tutmazsa Excel dosyayi "onarilamaz" sayar.

const CONTENT_TYPES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
</Types>"#;

const RELS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

const WORKBOOK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets><sheet name="Sayfa1" sheetId="1" r:id="rId1"/></sheets></workbook>"#;

const WORKBOOK_RELS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>"#;

const STYLES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts>
<fills count="1"><fill><patternFill patternType="none"/></fill></fills>
<borders count="1"><border/></borders>
<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
<cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs>
</styleSheet>"#;

// ---------------------------------------------------------------------------
// Markdown tablo ayristirma
// ---------------------------------------------------------------------------

/// Markdown tablosunu `Tablo`ya cevirir; tablo cikmiyorsa `None`.
///
/// Ayrac satiri (`| --- |`) OPSIYONEL: kucuk modeller onu siklikla atliyor ve
/// yoklugu yuzunden veriyi cope atmak kullaniciya hicbir sey kazandirmiyor.
/// Buna karsilik ayrac satiri VARSA veri sayilmaz — yoksa tablonun ilk satiri
/// bir demet tire olurdu.
pub fn markdown_dan(ham: &str) -> Option<Tablo> {
    let satirlar: Vec<&str> = ham
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter(|s| s.contains('|'))
        .collect();
    let (bas, kalan) = satirlar.split_first()?;

    let basliklar = hucrelere_bol(bas);
    if basliklar.is_empty() {
        return None;
    }

    let veri: Vec<Vec<String>> = kalan
        .iter()
        .filter(|s| !ayrac_satiri_mi(s))
        .map(|s| hucrelere_bol(s))
        .filter(|h| !h.is_empty())
        .collect();

    Some(Tablo::yeni(basliklar, veri))
}

/// `| --- | :---: |` gibi hizalama satiri mi?
fn ayrac_satiri_mi(satir: &str) -> bool {
    let hucreler = hucrelere_bol(satir);
    !hucreler.is_empty()
        && hucreler.iter().all(|h| {
            let h = h.trim();
            !h.is_empty() && h.chars().all(|c| c == '-' || c == ':')
        })
}

/// Bir markdown satirini hucrelere boler. Kacirilmis boru (`\|`) ayrac SAYILMAZ
/// — `Tablo::markdown_kirpik` tam da bu kacisi uretiyor, yani depoya konan bir
/// tablo ozeti geri okundugunda hucre sinirlari korunsun.
fn hucrelere_bol(satir: &str) -> Vec<String> {
    let govde = satir.trim().trim_start_matches('|').trim_end_matches('|');
    let mut hucreler = Vec::new();
    let mut aktif = String::new();
    let mut kacis = false;
    for c in govde.chars() {
        if kacis {
            // Yalniz boruyu kacis olarak yorumla; digerlerinde ters bolu metnin
            // kendisidir (Windows yollari hucrede sik gecer).
            if c != '|' {
                aktif.push('\\');
            }
            aktif.push(c);
            kacis = false;
        } else if c == '\\' {
            kacis = true;
        } else if c == '|' {
            hucreler.push(aktif.trim().to_string());
            aktif = String::new();
        } else {
            aktif.push(c);
        }
    }
    if kacis {
        aktif.push('\\');
    }
    hucreler.push(aktif.trim().to_string());
    if hucreler.iter().all(|h| h.is_empty()) {
        return Vec::new();
    }
    hucreler
}

// ---------------------------------------------------------------------------
// Arac
// ---------------------------------------------------------------------------

/// `belge_olustur` — sohbet verisinden ya da depodaki bir kaynaktan dosya uretir.
#[derive(Debug, Clone, Copy, Default)]
pub struct BelgeOlusturAraci;

impl BelgeOlusturAraci {
    pub fn yeni() -> Self {
        Self
    }

    fn motor(bicim: BelgeBicimi) -> Box<dyn BelgeMotoru> {
        match bicim {
            BelgeBicimi::Excel => Box::new(ExcelMotor::yeni()),
            b => Box::new(MetinMotor::yeni(b)),
        }
    }
}

impl Arac for BelgeOlusturAraci {
    fn ad(&self) -> &str {
        "belge_olustur"
    }

    fn aciklama(&self) -> &str {
        "Creates an Excel, Markdown or plain text file. Call this IMMEDIATELY when the user asks \
         for a file, table, list or report ('make an excel/table/report', in any language) — do \
         not ask or narrate. Write markdown into 'icerik'; for a table write a markdown table \
         (| ... |). For bulk device data pass 'kaynak_ref' instead of 'icerik'."
    }

    fn sema(&self) -> ArgSema {
        ArgSema::nesne(vec![
            Alan::yeni(
                "bicim",
                ArgSema::secenek(BelgeBicimi::HEPSI.map(|b| b.anahtar()))
                    .aciklama("File format: 'excel' (spreadsheet), 'markdown' or 'metin' (plain text)."),
            )
            .zorunlu(),
            Alan::yeni(
                "dosya_adi",
                ArgSema::metin()
                    .aciklama("File name WITHOUT extension, e.g. 'temmuz-toplantilari'."),
            )
            .zorunlu(),
            Alan::yeni("baslik", ArgSema::metin().aciklama("Document title (optional).")),
            Alan::yeni(
                "icerik",
                ArgSema::metin().aciklama(
                    "Document body as MARKDOWN. For a table write a header row (| Col1 | Col2 |), \
                     then | --- | --- |, then the data rows. Excel files are built from that table.",
                ),
            ),
            Alan::yeni(
                "kaynak_ref",
                ArgSema::metin().aciklama(
                    "Data reference returned by another tool. If given, the full data is pulled \
                     from the store — leave 'icerik' empty.",
                ),
            ),
        ])
    }

    /// Uretilen dosya kullanicinin kisisel verisini tasir ve `kaynak_ref` yolu
    /// cihaz verisini okur: oturum kirlenir.
    fn kirletir_mi(&self) -> bool {
        true
    }

    fn calistir<'a>(
        &'a self,
        args: serde_json::Value,
        ctx: &'a mut AracBaglami,
    ) -> AracGelecegi<'a> {
        kutula(async move {
            let sema = self.sema();
            if let Err(e) = sema.dogrula(&args) {
                return AracSonucu::basarisiz(&e);
            }

            let metin_alan = |ad: &str| -> Option<String> {
                args.get(ad)
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            };

            let Some(bicim) = metin_alan("bicim").as_deref().and_then(BelgeBicimi::coz) else {
                return AracSonucu::basarisiz(&AracHatasi::EksikAlan("bicim".into()));
            };
            let Some(dosya_adi) = metin_alan("dosya_adi") else {
                return AracSonucu::basarisiz(&AracHatasi::EksikAlan("dosya_adi".into()));
            };
            let baslik = metin_alan("baslik");
            let mut govde = metin_alan("icerik");
            let kaynak_ref = metin_alan("kaynak_ref");

            let cip = ctx.cip_baslat(bicim.ikon(), &format!("{} belgesi olusturuluyor", bicim.etiket()));

            // --- Bypass kanali: ref VARSA BAGLAYICIDIR. ---
            let mut tablo: Option<Tablo> = None;
            if let Some(r) = kaynak_ref.as_deref() {
                let Some(kayit) = ctx.depodan(&KaynakRef(r.to_string())) else {
                    // Cozulemeyen ref: dosya HIC yazilmaz. Modele yalniz olgu
                    // doner — hangi ref istendi, ne olmadi. Imperatif yonerge
                    // yok; yeniden deneme yonergesi beceri dosyasinin isi.
                    let hata = AracHatasi::Diger(format!("bilinmeyen kaynak_ref: {r}"));
                    ctx.cip_guncelle(
                        cip,
                        IzGuncelleme::durum(AracDurumu::Basarisiz("Kaynak veri bulunamadi".into()))
                            .metin("Kaynak veri bulunamadi".to_string())
                            .ham_cikti(format!("kaynak_ref={r}")),
                    );
                    let mut sonuc = AracSonucu::basarisiz(&hata);
                    sonuc.cip_metni = "Kaynak veri bulunamadi".to_string();
                    sonuc.modele_donen =
                        format!("unknown_data_ref: \"{r}\"; no file was created");
                    return sonuc;
                };
                // Depodaki govde `Tablo` icin gecerli markdown tablosudur;
                // Excel isteniyorsa yapilandirilmis tabloya geri cevrilir.
                if bicim.tablo_yapisi()
                    && let Some(t) = markdown_dan(&kayit.govde).filter(|t| !t.satirlar.is_empty())
                {
                    tablo = Some(t);
                    govde = None;
                } else {
                    govde = Some(kayit.govde);
                }
            } else if bicim.tablo_yapisi()
                && let Some(t) = govde.as_deref().and_then(markdown_dan).filter(|t| !t.satirlar.is_empty())
            {
                tablo = Some(t);
                govde = None;
            }

            // Kum havuzu kapisi: cikti klasoru calisma dizini altinda sabit.
            let klasor = match ctx.yolu_coz(CIKTI_KLASORU) {
                Ok(k) => k,
                Err(e) => {
                    ctx.cip_guncelle(cip, IzGuncelleme::durum(AracDurumu::Basarisiz(e.kisa_hata())));
                    return AracSonucu::basarisiz(&e);
                }
            };

            let motor = Self::motor(bicim);
            let yol = match belge_yaz(
                motor.as_ref(),
                &dosya_adi,
                baslik.as_deref(),
                govde.as_deref(),
                tablo.as_ref(),
                &klasor,
            ) {
                Ok(y) => y,
                Err(e) => {
                    ctx.cip_guncelle(cip, IzGuncelleme::durum(AracDurumu::Basarisiz(e.kisa_hata())));
                    return AracSonucu::basarisiz(&e);
                }
            };

            let ad = yol
                .file_name()
                .map(|a| a.to_string_lossy().into_owned())
                .unwrap_or_else(|| dosya_adi.clone());

            // MODELE DONEN YOL, CALISMA DIZININE GORELIDIR — cip metni gibi
            // yalin dosya adi DEGIL.
            //
            // NEDEN: dosyalar `CIKTI_KLASORU` altina yaziliyor ama modele
            // "ornek_tablo.xlsx" donuyordu. Model ayni oturumda "onu tablo
            // olarak goster" dedigimde belge_oku'yu tam o adla cagiriyor,
            // `yolu_coz` onu kokte ariyor ve "Dosya bulunamadi" doniyordu —
            // ardindan model dosyayi okuyamadigi icin tabloyu KENDI HAFIZASINDAN
            // uyduruyordu. Modelin gordugu yol, belge_oku'ya geri verilebilecek
            // yol olmak zorunda.
            let goreli = yol
                .strip_prefix(&ctx.calisma_dizini)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| ad.clone());
            let cip_metni = format!("{} olusturuldu: {ad}", bicim.etiket());

            ctx.cip_guncelle(
                cip,
                IzGuncelleme::durum(AracDurumu::Yazildi)
                    .metin(cip_metni.clone())
                    .ham_cikti(yol.display().to_string())
                    .dosya_yolu(yol.clone()),
            );
            ctx.kirlet();

            // Modele yalniz olgu doner; "onizle/paylas" gibi UI yonergesi yazma
            // — model onu papaganlar ve kullaniciya var olmayan bir dugmeyi
            // tarif eder.
            AracSonucu::yazildi(cip_metni, format!("file_created ({}): {goreli}", bicim.anahtar()))
                .ham_cikti(yol.display().to_string())
                .dosya_yolu(yol)
        })
    }
}

/// Bir tabloyu depoya koyup modele kisa ozet donen kisayol — bypass kanalini
/// kullanan araclarin `belge_olustur` ile bulusma noktasi.
pub fn tabloyu_depola(ctx: &AracBaglami, tur: &str, tablo: &Tablo) -> KaynakRef {
    ctx.depola(
        tur,
        &format!("{} satir x {} sutun tablo", tablo.satir_sayisi(), tablo.sutun_sayisi()),
        tablo.markdown_kirpik(usize::MAX),
    )
}

/// Modele gosterilecek kirpilmis ozet uzunlugu — disari acik ki eval ayni
/// sabiti kullanabilsin.
pub const fn ozet_satir_siniri() -> usize {
    OZET_SATIR
}

// ---------------------------------------------------------------------------
// Testler
// ---------------------------------------------------------------------------

#[cfg(test)]
mod testler {
    use super::*;
    use crate::veri_deposu::{Deger, PaylasimliDepo};
    use sirr_cekirdek::{SessizRaporlayici, VeriDeposu as CekirdekVeriDeposu};
    use std::future::Future;
    use std::sync::Arc;

    /// Cekirdekte tokio yok; bu crate de calistirici secmemeli. Sonuca kadar
    /// dondurulen ~15 satirlik minimal poll dongusu yeterli (cekirdek karari 3).
    fn beklet<F: Future>(gelecek: F) -> F::Output {
        use std::pin::pin;
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

        static VTABLE: RawWakerVTable = RawWakerVTable::new(
            |_| RawWaker::new(std::ptr::null(), &VTABLE),
            |_| {},
            |_| {},
            |_| {},
        );
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
        let mut ctx = Context::from_waker(&waker);
        let mut gelecek = pin!(gelecek);
        loop {
            if let Poll::Ready(cikti) = gelecek.as_mut().poll(&mut ctx) {
                return cikti;
            }
        }
    }

    fn gecici_dizin(etiket: &str) -> PathBuf {
        let yol = std::env::temp_dir().join(format!(
            "sirr-belge-{etiket}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&yol).unwrap();
        yol
    }

    fn baglam(kok: &Path, depo: Arc<PaylasimliDepo>) -> AracBaglami {
        AracBaglami::yeni(depo, kok, Arc::new(SessizRaporlayici))
    }

    #[test]
    fn kolon_harfi_asar_26yi() {
        assert_eq!(kolon_harfi(0), "A");
        assert_eq!(kolon_harfi(25), "Z");
        assert_eq!(kolon_harfi(26), "AA");
        assert_eq!(kolon_harfi(27), "AB");
        assert_eq!(kolon_harfi(51), "AZ");
        assert_eq!(kolon_harfi(52), "BA");
    }

    /// Kirpik markdown ciktisi ayristiriciya geri verildiginde hucre sinirlari
    /// korunmali — depoya konan tablonun Excel'e donusu bu tur bagli.
    #[test]
    fn markdown_gidis_donus_korunur() {
        let tablo = Tablo::yeni(
            ["Ad", "Tutar"],
            vec![
                vec!["Ali | Veli".to_string(), "120".to_string()],
                vec!["Ayse".to_string(), "80".to_string()],
            ],
        );
        let md = tablo.markdown_kirpik(usize::MAX);
        let geri = markdown_dan(&md).expect("tablo cozulmeli");
        assert_eq!(geri.basliklar, vec!["Ad", "Tutar"]);
        assert_eq!(geri.satirlar.len(), 2, "ayrac satiri veri sayilmamali");
        assert_eq!(geri.satirlar[0][0], "Ali | Veli");
        assert_eq!(geri.satirlar[1][1], "80");
    }

    /// Sayisal kolonun altina hesaplanmis sabit degil GERCEK formul yazilir.
    #[test]
    fn sayisal_kolona_gercek_sum_formulu_yazilir() {
        let basliklar = vec!["Ad".to_string(), "Tutar".to_string()];
        let satirlar = vec![
            vec!["Ali".to_string(), "120".to_string()],
            vec!["Ayse".to_string(), "80".to_string()],
        ];
        let xml = sheet_xml(&basliklar, &satirlar);
        assert!(xml.contains("<f>SUM(B2:B3)</f>"), "formul yok: {xml}");
        assert!(!xml.contains("<v>200</v>"), "sabit toplam yazilmis: {xml}");
        // Metin kolonu toplanmaz.
        assert!(!xml.contains("SUM(A2"), "metin kolonu toplanmis: {xml}");
        // Sayisal hucre inlineStr degil ham deger olmali (Excel sayi gorsun).
        assert!(xml.contains("<c r=\"B2\"><v>120</v></c>"), "{xml}");
    }

    /// Uretilen .xlsx gercek bir zip olmali ve OOXML parcalarini tasimali.
    #[test]
    fn xlsx_gercek_zip_ve_ooxml_parcalari_icerir() {
        let kok = gecici_dizin("xlsx");
        let tablo = Tablo::yeni(["Ad", "Tutar"], vec![vec!["Ali".into(), "12".into()]]);
        let yol = belge_yaz(&ExcelMotor::yeni(), "rapor", None, None, Some(&tablo), &kok)
            .expect("yazilmali");
        assert_eq!(yol.extension().unwrap(), "xlsx");

        let bayt = fs::read(&yol).unwrap();
        assert_eq!(&bayt[..2], b"PK", "zip imzasi yok");
        let girisler = sirr_zip::ac_harita(&bayt).expect("zip acilmali");
        for parca in [
            "[Content_Types].xml",
            "_rels/.rels",
            "xl/workbook.xml",
            "xl/_rels/workbook.xml.rels",
            "xl/styles.xml",
            "xl/worksheets/sheet1.xml",
        ] {
            assert!(girisler.contains_key(parca), "eksik parca: {parca}");
        }
        let sheet = String::from_utf8(girisler["xl/worksheets/sheet1.xml"].clone()).unwrap();
        assert!(sheet.contains("Ali"));
        fs::remove_dir_all(&kok).ok();
    }

    /// Borulu ama ayristirilamayan govde SESSIZCE tek sutuna dokulmez.
    #[test]
    fn ayristirilamayan_borulu_govde_hata_verir() {
        let hata = tabloya_indir(None, Some("| Ad Tutar"), None);
        assert!(matches!(hata, Err(AracHatasi::GecersizArguman(_))));
        // Borusuz duz liste ise tek sutun mesru.
        let (basliklar, satirlar) =
            tabloya_indir(Some("Notlar"), Some("bir\niki\nuc"), None).expect("mesru");
        assert_eq!(basliklar, vec!["Notlar"]);
        assert_eq!(satirlar.len(), 3);
    }

    /// Ayni ad iki kez istendiginde ustune YAZILMAZ.
    #[test]
    fn ayni_ad_ustune_yazmaz() {
        let kok = gecici_dizin("cakisma");
        let motor = MetinMotor::yeni(BelgeBicimi::Markdown);
        let bir = belge_yaz(&motor, "notlar", None, Some("ilk"), None, &kok).unwrap();
        let iki = belge_yaz(&motor, "notlar", None, Some("ikinci"), None, &kok).unwrap();
        assert_ne!(bir, iki);
        assert_eq!(fs::read_to_string(&bir).unwrap().trim(), "ilk");
        assert_eq!(fs::read_to_string(&iki).unwrap().trim(), "ikinci");
        // Ad yol olusturamaz.
        let ucuncu = belge_yaz(&motor, "../kacak", None, Some("x"), None, &kok).unwrap();
        assert_eq!(ucuncu.parent().unwrap(), kok);
        fs::remove_dir_all(&kok).ok();
    }

    /// Cozulemeyen kaynak_ref: dosya HIC yazilmamali (en pahali hata sinifi).
    #[test]
    fn cozulemeyen_ref_dosya_yazmaz() {
        let kok = gecici_dizin("refyok");
        let depo = Arc::new(PaylasimliDepo::yeni());
        let mut ctx = baglam(&kok, depo);
        let arac = BelgeOlusturAraci::yeni();
        let sonuc = beklet(arac.calistir(
            serde_json::json!({
                "bicim": "excel",
                "dosya_adi": "rapor",
                "kaynak_ref": "tablo#99"
            }),
            &mut ctx,
        ));
        assert!(matches!(sonuc.durum, AracDurumu::Basarisiz(_)));
        assert!(sonuc.modele_donen.contains("unknown_data_ref"));
        assert!(!kok.join(CIKTI_KLASORU).join("rapor.xlsx").exists());
        fs::remove_dir_all(&kok).ok();
    }

    /// Bypass kanali ucu uca: tablo depoda, model yalniz refi tasir, dosya dolu.
    #[test]
    fn kaynak_ref_ile_uretilen_dosya_doludur() {
        let kok = gecici_dizin("bypass");
        let depo = Arc::new(PaylasimliDepo::yeni());
        let tablo = Tablo::yeni(
            ["Ad", "Tutar"],
            (0..500).map(|i| vec![format!("kisi{i}"), format!("{i}")]).collect::<Vec<_>>(),
        );
        let kaynak_ref = depo.koy_deger("takvim", Deger::Tablo(tablo));
        let mut ctx = baglam(&kok, depo.clone());

        let arac = BelgeOlusturAraci::yeni();
        let sonuc = beklet(arac.calistir(
            serde_json::json!({
                "bicim": "excel",
                "dosya_adi": "takvim",
                "kaynak_ref": kaynak_ref.as_str()
            }),
            &mut ctx,
        ));
        assert_eq!(sonuc.durum, AracDurumu::Yazildi);
        // Modele donen metin KISA: 500 satirlik veri buradan gecmedi.
        assert!(sonuc.modele_donen.len() < 80, "{}", sonuc.modele_donen);
        assert!(!sonuc.modele_donen.contains("kisi499"));
        assert!(ctx.oturum_kirli(), "cihaz verisi yazildi, oturum kirlenmeli");

        let yol = sonuc.dosya_yolu.expect("dosya yolu");
        let bayt = fs::read(&yol).unwrap();
        let girisler = sirr_zip::ac_harita(&bayt).unwrap();
        let sheet = String::from_utf8(girisler["xl/worksheets/sheet1.xml"].clone()).unwrap();
        assert!(sheet.contains("kisi499"), "veri dosyaya gecmemis");
        assert!(sheet.contains("<f>SUM(B2:B501)</f>"), "formul yok");
        fs::remove_dir_all(&kok).ok();
    }

    /// Depodaki DUZ METIN markdown bicimine gectiginde aynen korunur.
    #[test]
    fn metin_bicimi_baslik_ve_govdeyi_yazar() {
        let kok = gecici_dizin("metin");
        let depo = Arc::new(PaylasimliDepo::yeni());
        let r = CekirdekVeriDeposu::koy(&*depo, "not", "3 satir", "alfa\nbeta\ngama".to_string());
        let mut ctx = baglam(&kok, depo);
        let arac = BelgeOlusturAraci::yeni();
        let sonuc = beklet(arac.calistir(
            serde_json::json!({
                "bicim": "markdown",
                "dosya_adi": "notlar",
                "baslik": "Notlarim",
                "kaynak_ref": r.as_str()
            }),
            &mut ctx,
        ));
        assert_eq!(sonuc.durum, AracDurumu::Yazildi);
        let metin = fs::read_to_string(sonuc.dosya_yolu.unwrap()).unwrap();
        assert!(metin.starts_with("# Notlarim\n\n"), "{metin}");
        assert!(metin.contains("beta"));
        fs::remove_dir_all(&kok).ok();
    }

    /// Gecersiz bicim degeri gramerden kacsa bile arac uretmez.
    #[test]
    fn gecersiz_bicim_reddedilir() {
        let kok = gecici_dizin("bicim");
        let depo = Arc::new(PaylasimliDepo::yeni());
        let mut ctx = baglam(&kok, depo);
        let arac = BelgeOlusturAraci::yeni();
        let sonuc = beklet(arac.calistir(
            serde_json::json!({"bicim": "powerpoint", "dosya_adi": "x", "icerik": "y"}),
            &mut ctx,
        ));
        assert!(matches!(sonuc.durum, AracDurumu::Basarisiz(_)));
        assert_eq!(sonuc.modele_donen, sirr_cekirdek::HATA_MODEL_METNI);
        fs::remove_dir_all(&kok).ok();
    }

    /// XML kacisi olmadan uretilen sayfa acilamaz hale gelir.
    #[test]
    fn xml_ozel_karakterleri_kacirilir() {
        let xml = sheet_xml(
            &["A&B".to_string()],
            &[vec!["<script>".to_string()], vec!["\"tirnak\"".to_string()]],
        );
        assert!(xml.contains("A&amp;B"));
        assert!(xml.contains("&lt;script&gt;"));
        assert!(!xml.contains("<script>"));
    }
}
