//! `belge_oku` — algi araci: .xlsx tablosunu ya da duz metin/markdown dosyasini
//! okur, modele GECERLI markdown doner.
//!
//! ZINCIRIN KALBI (Swift surumunde pahaliya ogrenildi): tablolu bir belgede
//! modele giden metin `Tablo::markdown_kirpik` ciktisidir — borulu, hizalama
//! satirli, her satirda tam sutun sayisi. Kucuk model bu blogu neredeyse
//! oldugu gibi aktarir, ust katman ayristirip gercek tabloyu cizer. Once duz
//! `ozet` (borusuz, 5 satirda kesik) doneniyordu; model tabloyu ondan yeniden
//! kuramiyor, "tablo gosterildi" deyip icerigi atliyordu. Bu yuzden burada
//! kirpma HER ZAMAN markdown_kirpik uzerinden yapilir, karakter tavani bile
//! satir sinirinda uygulanir (bkz. `satir_sinirinda_kirp`): yarim kalan bir
//! `| a | b` satiri butun blogu gecersiz kilardi.
//!
//! BYPASS KANALI: belge buyukse ham tablo/metin VeriDeposu'na konur, modele
//! kisa onizleme + `kaynak_ref` doner. Kirpma boylece VERI KAYBI degil yalnizca
//! bir pencere karari olur; kirpilan kisma bir sonraki arac referanstan
//! eksiksiz ulasir.
//!
//! XML: xlsx icindeki sheet/sharedStrings bu dosyada elle taranir. Hazir bir
//! XML crate'i cekmiyoruz (sifir-bagimlilik kimligi); ihtiyacimiz olan altkume
//! -- etiket adi, oznitelik, metin dugumu, temel varlik cozumu -- 150 satir.

use crate::veri_deposu::{Deger, PaylasimliDepo, Tablo};
use serde_json::Value;
use sirr_cekirdek::{
    Alan, Arac, AracBaglami, AracGelecegi, AracHatasi, AracSonuc, AracSonucu, ArgSema, IzGuncelleme,
    KaynakRef, kutula,
};
use std::sync::Arc;

/// Depoya konmadiginda modele gosterilen tablo satiri sayisi.
///
/// Yuksek tutulmasinin sebebi: veri geri alinamiyorsa pencereye basmak tek
/// sanstir. Depo bagliysa 30 satirin token bedelinin gerekcesi kalmaz.
const ONIZLEME_TAM: usize = 30;
/// Depoya konuldugunda yeten onizleme: model tablonun BICIMINI gorsun yeter,
/// govdeye kaynak_ref ile ulasilir.
const ONIZLEME_REFLI: usize = 10;
/// Modele giden govdenin karakter tavani (~375 token).
const MODEL_TAVANI: usize = 1500;
/// Metin belgeyi depoya tasima esigi.
const METIN_DEPO_ESIGI: usize = 1500;
/// Okunacak dosyanin ust siniri (32 MiB). Bunun ustu bir belge degil, bir hata
/// belirtisidir; zip tavanlari (sirr-zip) bu kapinin arkasinda ayrica calisir.
const DOSYA_TAVANI: u64 = 32 * 1024 * 1024;

/// Metin olarak okunmasina izin verilen uzantilar.
///
/// Beyaz liste, kara liste degil: bilinmeyen bir uzantiyi "herhalde metindir"
/// diye okumak, ikili bir dosyayi bozuk UTF-8 olarak modele bosaltmak demektir.
const METIN_UZANTILARI: &[&str] = &["txt", "md", "markdown", "text", "log", "csv", "tsv", "json"];

/// Belgeyi okur ve modele markdown/metin ozeti doner.
pub struct BelgeOkuAraci {
    /// Tipli depo. Opsiyonel, cunku cekirdek sozlesmesi (`ctx.depola`) yalniz
    /// `String` govde alir; tabloyu TABLO olarak saklayabilmek icin somut
    /// depoya ihtiyac var. Bagli degilse metne duserek yine de calisir.
    depo: Option<Arc<PaylasimliDepo>>,
}

impl Default for BelgeOkuAraci {
    fn default() -> Self {
        Self::yeni()
    }
}

impl BelgeOkuAraci {
    pub fn yeni() -> Self {
        Self { depo: None }
    }

    pub fn depo_ile(depo: Arc<PaylasimliDepo>) -> Self {
        Self { depo: Some(depo) }
    }

    /// Tabloyu depoya koyar; tipli depo yoksa cekirdek sozlesmesine duser.
    fn tabloyu_depola(&self, ctx: &AracBaglami, tablo: &Tablo) -> KaynakRef {
        match &self.depo {
            Some(d) => d.koy_deger("belge", Deger::Tablo(tablo.clone())),
            None => ctx.depola("belge", &tablo_ozeti(tablo), tablo.markdown_kirpik(usize::MAX)),
        }
    }

    fn metni_depola(&self, ctx: &AracBaglami, metin: &str) -> KaynakRef {
        match &self.depo {
            Some(d) => d.koy_deger("belge", Deger::Metin(metin.to_string())),
            None => ctx.depola(
                "belge",
                &format!("{} satir metin", metin.lines().count()),
                metin.to_string(),
            ),
        }
    }
}

impl Arac for BelgeOkuAraci {
    fn ad(&self) -> &str {
        "belge_oku"
    }

    fn aciklama(&self) -> &str {
        "Reads a document from disk (.xlsx spreadsheet, .md or plain text) and returns its \
         content. Call this IMMEDIATELY when the user asks about a file ('summarize it', \
         \"what's in it\", 'show it as a table', in any language); read before describing. \
         Spreadsheets come back as a markdown table you can pass through as-is."
    }

    fn sema(&self) -> ArgSema {
        ArgSema::nesne(vec![
            Alan::yeni(
                "yol",
                ArgSema::metin().aciklama(
                    "Path to the document to read, relative to the working directory. \
                     Supported: .xlsx, .md, .txt, .csv, .log, .json",
                ),
            )
            .zorunlu(),
            Alan::yeni(
                "odak",
                ArgSema::metin().aciklama(
                    "Optional: the topic the user cares about. Leave empty to read the \
                     whole document.",
                ),
            ),
        ])
        .aciklama("Read a document and return its content")
    }

    /// Kullanicinin kendi belgesini okuyor: icerik kisisel veri olabilir, bu
    /// yuzden oturum kirlenir.
    fn kirletir_mi(&self) -> bool {
        true
    }

    fn calistir<'a>(&'a self, args: Value, ctx: &'a mut AracBaglami) -> AracGelecegi<'a> {
        kutula(async move {
            let iz = ctx.cip_baslat("doc", "Belge okunuyor…");

            let sonuc = self.oku(&args, ctx);
            let (sonuc, kirlendi) = match sonuc {
                Ok(s) => (s, true),
                Err(h) => (AracSonucu::basarisiz(&h), false),
            };

            ctx.cip_guncelle(
                iz,
                IzGuncelleme::durum(sonuc.durum.clone())
                    .metin(sonuc.cip_metni.clone())
                    .ham_girdi(args.to_string())
                    .ham_cikti(sonuc.ham_cikti.clone().unwrap_or_default()),
            );
            // Kirletme yalniz GERCEKTEN veri okundugunda: basarisiz yol hicbir
            // icerige dokunmadi, oturumu kirletmesi kullaniciyi gereksiz onay
            // sorularina sokardi.
            if kirlendi {
                ctx.kirlet();
            }
            sonuc
        })
    }
}

impl BelgeOkuAraci {
    /// Senkron govde: `calistir` yalnizca cip/kirletme kabugunu tutar, boylece
    /// hata yolu tek yerde (`AracSonucu::basarisiz`) toplanir.
    fn oku(&self, args: &Value, ctx: &AracBaglami) -> AracSonuc<AracSonucu> {
        let ham_yol = args
            .get("yol")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| AracHatasi::EksikAlan("yol".into()))?;

        let yol = ctx.yolu_coz(ham_yol)?;
        let ust = yol.metadata().map_err(|_| AracHatasi::DosyaYok(yol.clone()))?;
        if !ust.is_file() {
            return Err(AracHatasi::DosyaYok(yol.clone()));
        }
        if ust.len() > DOSYA_TAVANI {
            return Err(AracHatasi::Diger(format!(
                "dosya cok buyuk ({} bayt), tavan {DOSYA_TAVANI}",
                ust.len()
            )));
        }

        let ad = yol
            .file_name()
            .map(|a| a.to_string_lossy().into_owned())
            .unwrap_or_else(|| ham_yol.to_string());
        let uzanti = yol
            .extension()
            .map(|u| u.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let bayt = std::fs::read(&yol)?;

        if uzanti == "xlsx" {
            let tablo = xlsx_coz(&bayt)?;
            Ok(self.tablo_sonucu(ctx, &ad, tablo).dosya_yolu(yol))
        } else if METIN_UZANTILARI.contains(&uzanti.as_str()) {
            let metin = metne_cevir(&bayt)?;
            Ok(self.metin_sonucu(ctx, &ad, &metin).dosya_yolu(yol))
        } else {
            Err(AracHatasi::GecersizArguman(format!(
                "desteklenmeyen belge bicimi: .{uzanti}"
            )))
        }
    }

    fn tablo_sonucu(&self, ctx: &AracBaglami, ad: &str, tablo: Tablo) -> AracSonucu {
        if tablo.sutun_sayisi() == 0 {
            return AracSonucu::okundu(
                format!("{ad} okundu, tablo bos"),
                "document_empty (the spreadsheet has no rows)",
            );
        }

        // Depoya yalnizca onizlemeye sigmayan tablo konur: kucuk tablo zaten
        // eksiksiz gidiyor, referans uretmek modele gereksiz bir dolayli adim
        // ekler.
        let kaynak = (tablo.satir_sayisi() > ONIZLEME_TAM).then(|| self.tabloyu_depola(ctx, &tablo));
        let onizleme = if kaynak.is_some() {
            ONIZLEME_REFLI
        } else {
            ONIZLEME_TAM
        };

        let govde = satir_sinirinda_kirp(&tablo.markdown_kirpik(onizleme), MODEL_TAVANI);
        let ek = kaynak
            .as_ref()
            .map(|r| sirr_cekirdek::kaynak_ref_eki(r.as_str()))
            .unwrap_or_default();

        AracSonucu::okundu(
            format!(
                "{ad} okundu ({} satir × {} sutun)",
                tablo.satir_sayisi(),
                tablo.sutun_sayisi()
            ),
            format!("{govde}{ek}"),
        )
        // Cip detayi TAM tabloyu tasir: modelin penceresi kirpilir, kullanicinin
        // gordugu kirpilmaz (seffaflik ikinci katman).
        .ham_cikti(tablo.markdown_kirpik(usize::MAX))
    }

    fn metin_sonucu(&self, ctx: &AracBaglami, ad: &str, metin: &str) -> AracSonucu {
        if metin.trim().is_empty() {
            return AracSonucu::okundu(
                format!("{ad} okundu, icerik bos"),
                "document_empty (the file has no text)",
            );
        }

        let kaynak = (metin.len() > METIN_DEPO_ESIGI).then(|| self.metni_depola(ctx, metin));
        let govde = satir_sinirinda_kirp(metin, MODEL_TAVANI);
        let ek = kaynak
            .as_ref()
            .map(|r| sirr_cekirdek::kaynak_ref_eki(r.as_str()))
            .unwrap_or_default();

        AracSonucu::okundu(
            format!("{ad} okundu ({} satir)", metin.lines().count()),
            format!("{govde}{ek}"),
        )
        .ham_cikti(metin.to_string())
    }
}

fn tablo_ozeti(tablo: &Tablo) -> String {
    format!(
        "{} satir x {} sutun tablo",
        tablo.satir_sayisi(),
        tablo.sutun_sayisi()
    )
}

/// Karakter tavanini SATIR sinirinda uygular.
///
/// Ortadan kesmek markdown tablosunu gecersiz kilardi (yarim `| a | b`);
/// gecersiz tablo = modelin icerigi atlamasi. Tavani asan metin son tam
/// satirda kesilir ve kesildigi tablo blogunun DISINDA belirtilir.
fn satir_sinirinda_kirp(metin: &str, tavan: usize) -> String {
    if metin.len() <= tavan {
        return metin.to_string();
    }
    let mut kesim = 0usize;
    for (i, _) in metin.match_indices('\n') {
        if i >= tavan {
            break;
        }
        kesim = i;
    }
    if kesim == 0 {
        // Tek satirlik dev metin: burada bozulacak bir tablo yapisi yok.
        let mut son = tavan.min(metin.len());
        while son > 0 && !metin.is_char_boundary(son) {
            son -= 1;
        }
        return format!("{}…", &metin[..son]);
    }
    format!("{}\n\n(devami kirpildi)", &metin[..kesim])
}

/// Baytlari metne cevirir; ikili dosyayi reddeder.
///
/// NUL bayti tek basina yeterli bir isaret: metin belgelerde gecmez, ikili
/// bicimlerde neredeyse her zaman gecer. Yanlis pozitif ihtimali, ikili bir
/// dosyanin modele bozuk karakter yigini olarak bosaltilmasi riskinden ucuz.
fn metne_cevir(bayt: &[u8]) -> AracSonuc<String> {
    if bayt.contains(&0) {
        return Err(AracHatasi::GecersizArguman(
            "dosya metin degil (ikili icerik)".into(),
        ));
    }
    Ok(String::from_utf8_lossy(bayt).into_owned())
}

// ---------------------------------------------------------------------------
// xlsx cozumu
// ---------------------------------------------------------------------------

/// .xlsx baytlarini `Tablo`ya cevirir. Ilk satir baslik kabul edilir.
pub fn xlsx_coz(bayt: &[u8]) -> AracSonuc<Tablo> {
    let harita = sirr_zip::ac_harita(bayt).map_err(|h| AracHatasi::Diger(h.to_string()))?;

    // sheet1.xml yaygin ad ama zorunlu degil; bulunamazsa alfabetik ilk sayfa
    // secilir (BTreeMap sirali oldugu icin secim belirlenimcidir).
    let sayfa = harita
        .get("xl/worksheets/sheet1.xml")
        .or_else(|| {
            harita
                .iter()
                .find(|(ad, _)| ad.starts_with("xl/worksheets/") && ad.ends_with(".xml"))
                .map(|(_, v)| v)
        })
        .ok_or_else(|| AracHatasi::Diger("xlsx icinde calisma sayfasi bulunamadi".into()))?;

    let paylasilan = harita
        .get("xl/sharedStrings.xml")
        .map(|d| paylasilan_ayristir(&String::from_utf8_lossy(d)))
        .unwrap_or_default();

    let satirlar = sayfa_ayristir(&String::from_utf8_lossy(sayfa), &paylasilan);
    let mut it = satirlar.into_iter();
    let Some(basliklar) = it.next() else {
        return Ok(Tablo::default());
    };
    Ok(Tablo::yeni(basliklar, it))
}

/// XML tarayicisinin urettigi parcalar.
#[derive(Debug, PartialEq)]
enum Parca<'a> {
    Ac {
        ad: &'a str,
        oznitelikler: &'a str,
        kendine_kapali: bool,
    },
    Kapa(&'a str),
    Metin(&'a str),
}

/// Cok kucuk bir XML tarayicisi.
///
/// SINIRI ACIK: oznitelik degerinin icinde kacirilmamis '>' varsa etiket erken
/// biter. OOXML uretiminde bu karakter daima `&gt;` olarak kacirilir, o yuzden
/// pratikte erisilmez bir yol; tam bir XML ayristiricisi cekmenin bedeli bu tek
/// sinir icin fazla.
fn parcala(xml: &str) -> Vec<Parca<'_>> {
    let mut parcalar = Vec::new();
    let mut i = 0usize;
    while i < xml.len() {
        if xml.as_bytes()[i] == b'<' {
            let Some(uzunluk) = xml[i..].find('>') else { break };
            let ic = &xml[i + 1..i + uzunluk];
            i += uzunluk + 1;
            // Bildirim (<?xml?>), yorum ve DOCTYPE bizi ilgilendirmiyor.
            if ic.starts_with('?') || ic.starts_with('!') {
                continue;
            }
            if let Some(ad) = ic.strip_prefix('/') {
                parcalar.push(Parca::Kapa(yerel_ad(ad.trim())));
            } else {
                let kendine_kapali = ic.ends_with('/');
                let govde = ic.trim_end_matches('/').trim();
                let (ad, oznitelikler) = govde
                    .split_once(char::is_whitespace)
                    .unwrap_or((govde, ""));
                parcalar.push(Parca::Ac {
                    ad: yerel_ad(ad),
                    oznitelikler,
                    kendine_kapali,
                });
            }
        } else {
            let son = xml[i..].find('<').map(|p| i + p).unwrap_or(xml.len());
            parcalar.push(Parca::Metin(&xml[i..son]));
            i = son;
        }
    }
    parcalar
}

/// `x:t` -> `t`. Ad alani onekleri anlam tasimiyor, tek sayfali OOXML'de
/// karisacak iki `t` yok.
fn yerel_ad(ad: &str) -> &str {
    ad.rsplit(':').next().unwrap_or(ad)
}

/// Etiketin oznitelik metninden tek bir degeri cikarir.
fn oznitelik<'a>(oznitelikler: &'a str, aranan: &str) -> Option<&'a str> {
    let b = oznitelikler.as_bytes();
    let n = b.len();
    let mut i = 0usize;
    while i < n {
        while i < n && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let bas = i;
        while i < n && b[i] != b'=' && !b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i == bas {
            break;
        }
        let anahtar = &oznitelikler[bas..i];
        while i < n && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= n || b[i] != b'=' {
            continue;
        }
        i += 1;
        while i < n && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= n || (b[i] != b'"' && b[i] != b'\'') {
            break;
        }
        let tirnak = b[i];
        i += 1;
        let deger_bas = i;
        while i < n && b[i] != tirnak {
            i += 1;
        }
        let deger = &oznitelikler[deger_bas..i.min(n)];
        i += 1;
        if yerel_ad(anahtar) == aranan {
            return Some(deger);
        }
    }
    None
}

/// `sharedStrings.xml` icindeki `<si>` degerleri.
fn paylasilan_ayristir(xml: &str) -> Vec<String> {
    let mut degerler = Vec::new();
    let mut govde = String::new();
    let mut si_ici = false;
    let mut topluyor = false;

    for parca in parcala(xml) {
        match parca {
            Parca::Ac { ad: "si", .. } => {
                si_ici = true;
                govde.clear();
            }
            // Zengin metinde tek bir <si> birden fazla <t> tasir; hepsi
            // birlestirilir, yoksa bicimlenmis hucre yarim okunur.
            Parca::Ac { ad: "t", .. } if si_ici => topluyor = true,
            Parca::Metin(m) if topluyor => govde.push_str(&varlik_coz(m)),
            Parca::Kapa("t") => topluyor = false,
            Parca::Kapa("si") => {
                degerler.push(std::mem::take(&mut govde));
                si_ici = false;
            }
            _ => {}
        }
    }
    degerler
}

/// `sheet*.xml` icindeki satir/hucre degerleri.
fn sayfa_ayristir(xml: &str, paylasilan: &[String]) -> Vec<Vec<String>> {
    let mut satirlar: Vec<Vec<String>> = Vec::new();
    let mut aktif: Vec<String> = Vec::new();
    let mut govde = String::new();
    let mut topluyor = false;
    let mut paylasilan_mi = false;
    let mut sutun: Option<usize> = None;

    for parca in parcala(xml) {
        match parca {
            Parca::Ac { ad: "row", .. } => aktif = Vec::new(),
            Parca::Ac {
                ad: "c",
                oznitelikler,
                kendine_kapali,
            } => {
                paylasilan_mi = oznitelik(oznitelikler, "t") == Some("s");
                sutun = oznitelik(oznitelikler, "r").and_then(sutun_indisi);
                govde.clear();
                if kendine_kapali {
                    hucre_yerlestir(&mut aktif, sutun, String::new());
                }
            }
            Parca::Ac { ad: "t" | "v", .. } => topluyor = true,
            Parca::Metin(m) if topluyor => govde.push_str(&varlik_coz(m)),
            Parca::Kapa("t" | "v") => topluyor = false,
            Parca::Kapa("c") => {
                let deger = if paylasilan_mi {
                    // Bozuk indeks hucreyi bos birakir, panige gitmez: girdi
                    // baskasinin urettigi bir dosya olabilir.
                    govde
                        .trim()
                        .parse::<usize>()
                        .ok()
                        .and_then(|i| paylasilan.get(i).cloned())
                        .unwrap_or_default()
                } else {
                    govde.clone()
                };
                hucre_yerlestir(&mut aktif, sutun, deger);
                govde.clear();
            }
            Parca::Kapa("row") => satirlar.push(std::mem::take(&mut aktif)),
            _ => {}
        }
    }
    satirlar
}

/// Hucreyi KENDI sutununa koyar.
///
/// Swift surumu hucreleri sirayla ekliyordu; Excel bos hucre icin `<c>`
/// yazmadigindan bir bosluk sonrasi tum satir bir sutun sola kayiyordu.
/// `r="C2"` referansi varken bu hatayi tekrarlamanin sebebi yok.
fn hucre_yerlestir(satir: &mut Vec<String>, sutun: Option<usize>, deger: String) {
    match sutun {
        Some(i) => {
            while satir.len() < i {
                satir.push(String::new());
            }
            if satir.len() == i {
                satir.push(deger);
            } else {
                satir[i] = deger;
            }
        }
        None => satir.push(deger),
    }
}

/// `C12` -> `Some(2)`. Bijektif 26 tabani, 0 tabanli indekse cevrilir.
fn sutun_indisi(hucre_ref: &str) -> Option<usize> {
    let mut n = 0usize;
    let mut harf_var = false;
    for c in hucre_ref.chars() {
        if c.is_ascii_alphabetic() {
            harf_var = true;
            n = n.checked_mul(26)?.checked_add(
                (c.to_ascii_uppercase() as usize) - ('A' as usize) + 1,
            )?;
        } else {
            break;
        }
    }
    // Excel'in sutun tavani 16384; ustu bozuk ya da kotu niyetli bir referanstir
    // ve dogrudan devasa bir Vec ayirmaya donusurdu.
    if !harf_var || n == 0 || n > 16_384 {
        return None;
    }
    Some(n - 1)
}

/// XML varlik cozumu — OOXML'in urettigi kume.
fn varlik_coz(ham: &str) -> String {
    if !ham.contains('&') {
        return ham.to_string();
    }
    let mut cikti = String::with_capacity(ham.len());
    let mut kalan = ham;
    while let Some(bas) = kalan.find('&') {
        cikti.push_str(&kalan[..bas]);
        let govde = &kalan[bas..];
        let Some(son) = govde.find(';').filter(|s| *s <= 12) else {
            cikti.push('&');
            kalan = &govde[1..];
            continue;
        };
        let ad = &govde[1..son];
        match ad {
            "amp" => cikti.push('&'),
            "lt" => cikti.push('<'),
            "gt" => cikti.push('>'),
            "quot" => cikti.push('"'),
            "apos" => cikti.push('\''),
            _ => {
                let sayisal = ad
                    .strip_prefix("#x")
                    .or_else(|| ad.strip_prefix("#X"))
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| ad.strip_prefix('#').and_then(|d| d.parse::<u32>().ok()))
                    .and_then(char::from_u32);
                match sayisal {
                    Some(c) => cikti.push(c),
                    // Tanimadigimiz varligi AYNEN birakmak, sessizce yutmaktan
                    // iyidir: veri kaybolmaz, gozle gorulur.
                    None => cikti.push_str(&govde[..=son]),
                }
            }
        }
        kalan = &govde[son + 1..];
    }
    cikti.push_str(kalan);
    cikti
}

#[cfg(test)]
mod tests {
    use super::*;
    use sirr_cekirdek::{AracDurumu, BellekVeriDeposu, SessizRaporlayici, HATA_MODEL_METNI};
    use sirr_zip::{ZipGiris, paketle};

    /// tokio bagimliligi yok; test icin yeten minimum yurutucu.
    fn futures_yok<F: std::future::Future>(mut f: F) -> F::Output {
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        fn bos(_: *const ()) {}
        fn klon(p: *const ()) -> RawWaker {
            RawWaker::new(p, &VT)
        }
        static VT: RawWakerVTable = RawWakerVTable::new(klon, bos, bos, bos);
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VT)) };
        let mut cx = Context::from_waker(&waker);
        let mut f = unsafe { std::pin::Pin::new_unchecked(&mut f) };
        loop {
            if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }

    fn gecici_dizin(etiket: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "sirr-belge-oku-{etiket}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn baglam(dizin: &std::path::Path) -> AracBaglami {
        AracBaglami::yeni(
            Arc::new(BellekVeriDeposu::yeni()),
            dizin,
            Arc::new(SessizRaporlayici),
        )
    }

    /// Basliklar inlineStr, veri satirlari <v>: gercek Excel ciktisinin karisimi.
    fn xlsx_uret(basliklar: &[&str], satirlar: &[Vec<&str>]) -> Vec<u8> {
        let mut sayfa = String::from(
            "<?xml version=\"1.0\"?><worksheet><sheetData>",
        );
        sayfa.push_str("<row r=\"1\">");
        for (k, b) in basliklar.iter().enumerate() {
            sayfa.push_str(&format!(
                "<c r=\"{}1\" t=\"inlineStr\"><is><t>{b}</t></is></c>",
                sutun_harfi(k)
            ));
        }
        sayfa.push_str("</row>");
        for (i, satir) in satirlar.iter().enumerate() {
            sayfa.push_str(&format!("<row r=\"{}\">", i + 2));
            for (k, h) in satir.iter().enumerate() {
                sayfa.push_str(&format!(
                    "<c r=\"{}{}\"><v>{h}</v></c>",
                    sutun_harfi(k),
                    i + 2
                ));
            }
            sayfa.push_str("</row>");
        }
        sayfa.push_str("</sheetData></worksheet>");
        paketle(&[ZipGiris::yeni("xl/worksheets/sheet1.xml", sayfa.into_bytes())]).unwrap()
    }

    fn sutun_harfi(i: usize) -> char {
        (b'A' + i as u8) as char
    }

    #[test]
    fn paylasilan_dizeler_zengin_metni_birlestirir() {
        let xml = "<sst><si><t>Ad</t></si><si><r><t>So</t></r><r><t>yad</t></r></si>\
                   <si><t>Ya&amp;s</t></si></sst>";
        assert_eq!(paylasilan_ayristir(xml), vec!["Ad", "Soyad", "Ya&s"]);
    }

    #[test]
    fn bos_hucre_sutun_kaydirmaz() {
        // B sutunu yok: r="C2" olmasaydi "30" B'ye kayardi.
        let xml = "<sheetData><row r=\"1\"><c r=\"A1\"><v>a</v></c><c r=\"B1\"><v>b</v></c>\
                   <c r=\"C1\"><v>c</v></c></row>\
                   <row r=\"2\"><c r=\"A2\"><v>x</v></c><c r=\"C2\"><v>30</v></c></row>\
                   </sheetData>";
        let satirlar = sayfa_ayristir(xml, &[]);
        assert_eq!(satirlar[1], vec!["x", "", "30"]);
    }

    #[test]
    fn paylasilan_indeks_ve_varlik_cozulur() {
        let paylasilan = vec!["Sirket & Co".to_string(), "İkinci".to_string()];
        let xml = "<sheetData><row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c>\
                   <c r=\"B1\" t=\"s\"><v>1</v></c><c r=\"C1\" t=\"s\"><v>99</v></c></row></sheetData>";
        let satirlar = sayfa_ayristir(xml, &paylasilan);
        // Sinir disi indeks bos hucreye duser, panige gitmez.
        assert_eq!(satirlar[0], vec!["Sirket & Co", "İkinci", ""]);
    }

    #[test]
    fn oznitelik_ve_sutun_indisi_dogru_okunur() {
        assert_eq!(oznitelik("r=\"C2\" s=\"1\" t=\"s\"", "t"), Some("s"));
        assert_eq!(oznitelik("r='A1'", "r"), Some("A1"));
        assert_eq!(oznitelik("r=\"A1\"", "t"), None);
        assert_eq!(sutun_indisi("A1"), Some(0));
        assert_eq!(sutun_indisi("Z9"), Some(25));
        assert_eq!(sutun_indisi("AA1"), Some(26));
        assert_eq!(sutun_indisi("1"), None);
        assert_eq!(sutun_indisi("ZZZZZ1"), None);
    }

    #[test]
    fn xlsx_okunur_ve_modele_gecerli_markdown_doner() {
        let dizin = gecici_dizin("xlsx");
        let bayt = xlsx_uret(
            &["Ay", "Gelir"],
            &[vec!["Ocak", "100"], vec!["Subat", "200"]],
        );
        std::fs::write(dizin.join("rapor.xlsx"), bayt).unwrap();

        let arac = BelgeOkuAraci::yeni();
        let mut ctx = baglam(&dizin);
        let sonuc = futures_yok(arac.calistir(
            serde_json::json!({ "yol": "rapor.xlsx" }),
            &mut ctx,
        ));

        assert_eq!(sonuc.durum, AracDurumu::Okundu);
        let m = &sonuc.modele_donen;
        assert!(m.starts_with("| Ay | Gelir |"), "markdown baslik yok: {m}");
        assert!(m.contains("| --- | --- |"), "hizalama satiri yok: {m}");
        assert!(m.contains("| Ocak | 100 |"), "veri satiri yok: {m}");
        // Her satir tam sutun sayisi tasimali — gecersiz markdown modeli bozar.
        for satir in m.lines().filter(|s| s.starts_with('|')) {
            assert_eq!(satir.matches('|').count(), 3, "sutun sayisi bozuk: {satir}");
        }
        assert!(ctx.oturum_kirli(), "belge okundu, oturum kirlenmeli");
    }

    #[test]
    fn buyuk_tablo_depoya_gider_ve_kaynak_ref_doner() {
        let dizin = gecici_dizin("buyuk");
        let satirlar: Vec<Vec<String>> = (0..80)
            .map(|i| vec![format!("s{i}"), format!("{}", i * 3)])
            .collect();
        let gorunum: Vec<Vec<&str>> = satirlar
            .iter()
            .map(|s| s.iter().map(String::as_str).collect())
            .collect();
        std::fs::write(
            dizin.join("buyuk.xlsx"),
            xlsx_uret(&["Ad", "Deger"], &gorunum),
        )
        .unwrap();

        let depo = Arc::new(PaylasimliDepo::yeni());
        let arac = BelgeOkuAraci::depo_ile(depo.clone());
        let mut ctx = baglam(&dizin);
        let sonuc = futures_yok(arac.calistir(
            serde_json::json!({ "yol": "buyuk.xlsx" }),
            &mut ctx,
        ));

        let m = &sonuc.modele_donen;
        assert!(m.contains("kaynak_ref="), "referans donmedi: {m}");
        // Kirpilmis olsa da hala tablo: model bicimi gorebilmeli.
        assert!(m.starts_with("| Ad | Deger |"));
        assert!(m.contains("satir daha gosterilmedi"));

        let kaynak = m
            .split("kaynak_ref=")
            .nth(1)
            .unwrap()
            .trim_end_matches(')')
            .trim()
            .to_string();
        let Some(Deger::Tablo(t)) = depo.deger(&KaynakRef(kaynak)) else {
            panic!("depoda tablo yok");
        };
        assert_eq!(t.satir_sayisi(), 80, "ham veri eksiksiz saklanmali");
        // Cip detayi kirpilmaz.
        assert!(sonuc.ham_cikti.unwrap().contains("| s79 | 237 |"));
    }

    #[test]
    fn duz_metin_ve_markdown_okunur() {
        let dizin = gecici_dizin("metin");
        std::fs::write(dizin.join("not.md"), "# Baslik\n\nIcerik burada.\n").unwrap();

        let arac = BelgeOkuAraci::yeni();
        let mut ctx = baglam(&dizin);
        let sonuc =
            futures_yok(arac.calistir(serde_json::json!({ "yol": "not.md" }), &mut ctx));

        assert_eq!(sonuc.durum, AracDurumu::Okundu);
        assert!(sonuc.modele_donen.contains("# Baslik"));
        assert!(sonuc.modele_donen.contains("Icerik burada."));
        assert!(!sonuc.modele_donen.contains("kaynak_ref"), "kucuk metin depoya gitmemeli");
        assert_eq!(sonuc.dosya_yolu.unwrap().file_name().unwrap(), "not.md");
    }

    #[test]
    fn uzun_metin_depoya_gider() {
        let dizin = gecici_dizin("uzunmetin");
        let icerik: String = (0..400).map(|i| format!("satir {i}\n")).collect();
        std::fs::write(dizin.join("gunluk.txt"), &icerik).unwrap();

        let depo = Arc::new(PaylasimliDepo::yeni());
        let arac = BelgeOkuAraci::depo_ile(depo.clone());
        let mut ctx = baglam(&dizin);
        let sonuc =
            futures_yok(arac.calistir(serde_json::json!({ "yol": "gunluk.txt" }), &mut ctx));

        assert!(sonuc.modele_donen.contains("kaynak_ref="));
        assert!(sonuc.modele_donen.len() < MODEL_TAVANI + 120);
        // Kirpma satir sinirinda: son satir yarim kalmamali.
        assert!(sonuc.modele_donen.contains("(devami kirpildi)"));
        assert_eq!(sonuc.ham_cikti.unwrap().lines().count(), 400);
    }

    #[test]
    fn hatalar_tek_kapidan_gecer() {
        let dizin = gecici_dizin("hata");
        std::fs::write(dizin.join("resim.png"), b"\x89PNG\r\n").unwrap();
        let arac = BelgeOkuAraci::yeni();
        let mut ctx = baglam(&dizin);

        // Olmayan dosya.
        let yok = futures_yok(arac.calistir(serde_json::json!({ "yol": "yok.md" }), &mut ctx));
        assert!(matches!(yok.durum, AracDurumu::Basarisiz(_)));
        assert_eq!(yok.modele_donen, HATA_MODEL_METNI);

        // Desteklenmeyen bicim.
        let png = futures_yok(arac.calistir(serde_json::json!({ "yol": "resim.png" }), &mut ctx));
        assert_eq!(png.modele_donen, HATA_MODEL_METNI);

        // Eksik alan.
        let bos = futures_yok(arac.calistir(serde_json::json!({}), &mut ctx));
        assert_eq!(bos.modele_donen, HATA_MODEL_METNI);

        // Kum havuzu kacisi.
        let kacis =
            futures_yok(arac.calistir(serde_json::json!({ "yol": "../../etc/hosts" }), &mut ctx));
        assert_eq!(kacis.modele_donen, HATA_MODEL_METNI);

        // Hicbir basarisiz yol oturumu kirletmemeli.
        assert!(!ctx.oturum_kirli());
    }

    #[test]
    fn sema_modeli_dogru_cagriya_zorlar() {
        let sema = BelgeOkuAraci::yeni().sema();
        let alanlar = sema.alanlar();
        assert_eq!(alanlar.len(), 2);
        assert_eq!(alanlar[0].ad, "yol");
        assert!(alanlar[0].zorunlu, "yol zorunlu olmali");
        assert_eq!(alanlar[1].ad, "odak");
        assert!(!alanlar[1].zorunlu);
        assert!(sema.dogrula(&serde_json::json!({ "yol": "a.md" })).is_ok());
        assert!(sema.dogrula(&serde_json::json!({ "odak": "gelir" })).is_err());
        assert!(BelgeOkuAraci::yeni().kirletir_mi());
    }
}
