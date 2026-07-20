//! `WebAramaIstemcisi` — SearXNG'ye GET atan TEK yer.
//!
//! AG TEKELI: `ureq` bu dosyadan baska hicbir yerde cagrilmaz; `sirr-web`
//! disinda hicbir crate soket acmaz. Tekelin degeri denetlenebilir olmasinda:
//! "sorgu disariya nereden cikiyor" sorusunun cevabi tek dosya.
//!
//! SENKRON: `ara` bloklar. `Arac::calistir` async oldugu icin bu ilk bakista
//! ters gorunur, ama sirr bir calistirici SECMEMEK uzere kurulu (kok
//! Cargo.toml'da tokio'nun neden olmadigi yaziyor) ve async imza orada
//! dyn-uyumluluk icin var, es zamanlilik icin degil. Bloklayan cagriyi
//! bloklayan bicimde yazmak, sahte bir async cephesi kurmaktan durust.

use crate::hata::{WebHatasi, WebSonuc};
use crate::metin::metinlestir;
use crate::sonuc::{AramaSonucu, ayristir};
use std::time::Duration;

/// Sunucu adresinin okunacagi ortam degiskeni.
pub const ADRES_DEGISKENI: &str = "SIRR_SEARXNG";

/// Kullanicinin kendi ornegi. NEDEN GOMULU DEGIL DE VARSAYILAN: bu bir
/// gelistirme kolayligidir; adres tek satirda gorunur ve `SIRR_SEARXNG` ile
/// ezilir. Uygulama dagitiminda buraya hicbir adres ON TANIMLI gelmemeli
/// (bkz. web-arama-spec §3.1) — orada varsayilan BOS olur ve arama araci
/// sunucu tanimlanana dek katalogda hic gorunmez.
pub const VARSAYILAN_ADRES: &str = "https://abdullahfaruk.com/searxng";

/// Zaman asimi. ZORUNLU, ve `Option` degil: varsayilani "sinirsiz" olan bir
/// alan er gec sinirsiz kalir ve asilmis bir baglanti kullaniciyi sonsuza
/// kadar "aranıyor…" cipine bakarken birakir.
pub const VARSAYILAN_ZAMAN_ASIMI: Duration = Duration::from_secs(10);

/// Sayfa getirmede modele degil DEPOYA giden govdenin tavani.
///
/// NEDEN TAVAN VAR: `read_to_string` sinirsiz okursa kotu niyetli (ya da
/// sadece devasa) bir sayfa bellegi doldurur. 2 MB, metne indiginde her
/// makul makaleden fazlasidir.
const EN_COK_GOVDE: u64 = 2 * 1024 * 1024;

pub struct WebAramaIstemcisi {
    taban: String,
    zaman_asimi: Duration,
}

impl Default for WebAramaIstemcisi {
    fn default() -> Self {
        Self::yeni()
    }
}

impl WebAramaIstemcisi {
    /// Ortamdan yapilandirilmis istemci.
    ///
    /// ADRES KODA GOMULMEZ: sunucu kullanicinindir, degisir; sabitlenmis bir
    /// adres bu ozelligi tek bir makineye baglardi.
    pub fn yeni() -> Self {
        let taban = std::env::var(ADRES_DEGISKENI)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| VARSAYILAN_ADRES.to_string());
        Self::adresle(taban)
    }

    pub fn adresle(taban: impl Into<String>) -> Self {
        Self { taban: taban.into().trim_end_matches('/').to_string(), zaman_asimi: VARSAYILAN_ZAMAN_ASIMI }
    }

    pub fn zaman_asimiyla(mut self, sure: Duration) -> Self {
        self.zaman_asimi = sure;
        self
    }

    pub fn taban(&self) -> &str {
        &self.taban
    }

    /// Taban adresin kullanilabilirligini dogrular.
    ///
    /// DUZ `http://` YALNIZ YEREL AGDA: uzak bir sunucuya sifresiz giden sorgu,
    /// aradaki her sicramada okunabilir — "sorgu da veridir" ilkesi (spec §2.2)
    /// bunu yasaklar. Yerel adreste sifreleme beklemek ise kullaniciyi kendi
    /// makinesine sertifika kurmaya zorlardi.
    fn adresi_dogrula(&self) -> WebSonuc<()> {
        if let Some(kalan) = self.taban.strip_prefix("https://") {
            return if kalan.is_empty() {
                Err(WebHatasi::GecersizAdres("konak adi yok".into()))
            } else {
                Ok(())
            };
        }
        if let Some(kalan) = self.taban.strip_prefix("http://") {
            let konak = crate::sonuc::alan_adi(kalan);
            return if yerel_mi(&konak) {
                Ok(())
            } else {
                Err(WebHatasi::GecersizAdres(format!(
                    "duz http yalniz yerel agda kabul edilir: {konak}"
                )))
            };
        }
        Err(WebHatasi::GecersizAdres("adres https:// ile baslamali".into()))
    }

    /// Istek URL'ini kurar. AYRI VE PUBLIC: aga cikmadan test edilebilsin ve
    /// cip detayinda "ne gitti" diye kullaniciya AYNEN gosterilebilsin
    /// (spec §3.2 seffaflik deseni).
    pub fn istek_url(&self, sorgu: &str, dil: Option<&str>) -> String {
        let mut u = format!("{}/search?q={}&format=json&safesearch=1", self.taban, kacisla(sorgu));
        if let Some(d) = dil.map(str::trim).filter(|d| !d.is_empty()) {
            u.push_str("&language=");
            u.push_str(&kacisla(d));
        }
        u
    }

    /// Aramayi yapar. Bos sonuc HATA olarak doner (`WebHatasi::BosSonuc`):
    /// cagiran onu "no_results" diye modele bildirmek zorunda kalsin, sessizce
    /// bos bir liste gorup uydurmaya baslamasin.
    pub fn ara(&self, sorgu: &str, dil: Option<&str>) -> WebSonuc<Vec<AramaSonucu>> {
        let sorgu = sorgu.trim();
        if sorgu.is_empty() {
            return Err(WebHatasi::GecersizAdres("sorgu bos".into()));
        }
        self.adresi_dogrula()?;
        let govde = self.getir(&self.istek_url(sorgu, dil))?;
        ayristir(&govde)
    }

    /// Tek bir adresin metnini ceker (spec v1.1: "sonuc sayfasi cekme").
    ///
    /// Sonuc ozetleri 200 karaktere kirpik oldugu icin bazen yetmez; bu, model
    /// ONE adres secip devamini isteyebilsin diye var. Cikti HTML'den soyulmus
    /// duz metindir (bkz. `metin.rs` — basit olmasi bilincli).
    pub fn sayfa_metni(&self, adres: &str) -> WebSonuc<String> {
        if !adres.starts_with("https://") && !adres.starts_with("http://") {
            return Err(WebHatasi::GecersizAdres(format!("desteklenmeyen sema: {adres}")));
        }
        let ham = self.getir(adres)?;
        let metin = metinlestir(&ham);
        if metin.trim().is_empty() {
            // Betikle cizilen sayfa: HTML geldi ama okunacak metin yok.
            // Bos metni "sayfa bos" diye modele vermek yanlis bir olgu olurdu.
            return Err(WebHatasi::BosSonuc);
        }
        Ok(metin)
    }

    /// Tek ag cagrisi. HTTP hatalarini `WebHatasi`ya cevirmenin TEK yeri.
    fn getir(&self, url: &str) -> WebSonuc<String> {
        let ajan: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(self.zaman_asimi))
            // Yonlendirme tavani: sonsuz yonlendirme dongusu zaman asimini
            // bekletmeden kesilsin.
            .max_redirects(5)
            .build()
            .into();

        match ajan.get(url).call() {
            Ok(mut yanit) => yanit
                .body_mut()
                .with_config()
                .limit(EN_COK_GOVDE)
                .read_to_string()
                .map_err(|e| cevir(&e)),
            Err(e) => Err(cevir(&e)),
        }
    }
}

/// `ureq` hatasini `WebHatasi`ya cevirir.
///
/// TEK CEVIRI NOKTASI: cagri yerleri kendi eslemesini yazsaydi, biri zaman
/// asimini "ulasilamadi" sayar, digeri saymaz; kullanici ayni arizada iki
/// farkli cumle gorurdu.
fn cevir(hata: &ureq::Error) -> WebHatasi {
    match hata {
        ureq::Error::StatusCode(kod) => WebHatasi::SunucuKodu(*kod),
        ureq::Error::Timeout(_) => WebHatasi::ZamanAsimi,
        // `BodyExceedsLimit` dahil geri kalan her sey ulasim katmani sorunudur;
        // ayrintisi kullaniciya degil, cip detayina gider.
        diger => WebHatasi::UlasilamadI(diger.to_string()),
    }
}

/// Konak yerel ag adresi mi.
fn yerel_mi(konak: &str) -> bool {
    konak == "localhost"
        || konak == "127.0.0.1"
        || konak == "::1"
        || konak.ends_with(".local")
        || konak.starts_with("192.168.")
        || konak.starts_with("10.")
}

/// Yuzde kacisi (RFC 3986 `unreserved` disindaki her bayt kacar).
///
/// SIFIR BAGIMLILIK: `urlencoding`/`percent-encoding` crate'i EKLENMEDI —
/// kural on satir ve girdisi tek bir sorgu metni. BAYT BAYT calisir, karakter
/// karakter degil: cok baytli UTF-8 (Turkce "ç", "ğ") her bayti ayri kacmak
/// zorundadir, aksi halde sunucu bozuk sorgu alir.
fn kacisla(ham: &str) -> String {
    let mut cikti = String::with_capacity(ham.len());
    for b in ham.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                cikti.push(*b as char)
            }
            _ => cikti.push_str(&format!("%{b:02X}")),
        }
    }
    cikti
}

#[cfg(test)]
mod testler {
    use super::*;

    #[test]
    fn istek_url_dogru_kurulur() {
        let i = WebAramaIstemcisi::adresle("https://ornek.test/searxng");
        assert_eq!(
            i.istek_url("rust async", None),
            "https://ornek.test/searxng/search?q=rust%20async&format=json&safesearch=1"
        );
    }

    #[test]
    fn tabandaki_sondaki_egik_cizgi_cift_slash_uretmez() {
        let i = WebAramaIstemcisi::adresle("https://ornek.test/searxng/");
        assert!(i.istek_url("a", None).starts_with("https://ornek.test/searxng/search?"));
    }

    #[test]
    fn dil_verilirse_eklenir_bos_verilirse_eklenmez() {
        let i = WebAramaIstemcisi::adresle("https://ornek.test");
        assert!(i.istek_url("a", Some("tr")).ends_with("&language=tr"));
        assert!(!i.istek_url("a", Some("  ")).contains("language"));
        assert!(!i.istek_url("a", None).contains("language"));
    }

    #[test]
    fn turkce_ve_ozel_karakterler_bayt_bayt_kacar() {
        let i = WebAramaIstemcisi::adresle("https://ornek.test");
        let u = i.istek_url("çay & kahve?", None);
        // "ç" iki bayttir (C3 A7) ve ikisi de ayri kacmali.
        assert!(u.contains("%C3%A7ay"), "{u}");
        assert!(u.contains("%26"), "& sorgu ayraci olarak sizmamali: {u}");
        assert!(u.contains("%3F"), "? kacmali: {u}");
    }

    #[test]
    fn sorgu_enjeksiyonu_url_parametresine_donusemez() {
        // Model "x&format=html" gibi bir sorgu uretse bile parametre EKLEYEMEZ.
        let i = WebAramaIstemcisi::adresle("https://ornek.test");
        let u = i.istek_url("x&format=html", None);
        assert_eq!(u.matches("format=").count(), 1, "{u}");
    }

    #[test]
    fn duz_http_uzak_sunucuda_reddedilir() {
        let h = WebAramaIstemcisi::adresle("http://uzak.test").ara("a", None).unwrap_err();
        assert!(matches!(h, WebHatasi::GecersizAdres(_)), "{h:?}");
    }

    #[test]
    fn duz_http_yerel_agda_kabul_edilir() {
        // Adres kapisini gecmeli; sonrasinda ag hatasina dusmesi beklenir.
        let h = WebAramaIstemcisi::adresle("http://localhost:8888").ara("a", None).unwrap_err();
        assert!(!matches!(h, WebHatasi::GecersizAdres(_)), "{h:?}");
    }

    #[test]
    fn semasiz_adres_reddedilir() {
        let h = WebAramaIstemcisi::adresle("ornek.test").ara("a", None).unwrap_err();
        assert!(matches!(h, WebHatasi::GecersizAdres(_)));
    }

    #[test]
    fn bos_sorgu_aga_cikmadan_reddedilir() {
        let i = WebAramaIstemcisi::adresle("https://ornek.test");
        assert!(matches!(i.ara("   ", None), Err(WebHatasi::GecersizAdres(_))));
    }

    #[test]
    fn sayfa_metni_desteklenmeyen_semayi_reddeder() {
        let i = WebAramaIstemcisi::adresle("https://ornek.test");
        assert!(matches!(i.sayfa_metni("file:///etc/passwd"), Err(WebHatasi::GecersizAdres(_))));
        assert!(matches!(i.sayfa_metni("javascript:alert(1)"), Err(WebHatasi::GecersizAdres(_))));
    }

    #[test]
    fn zaman_asimi_varsayilani_sonsuz_degildir() {
        let i = WebAramaIstemcisi::adresle("https://ornek.test");
        assert_eq!(i.zaman_asimi, VARSAYILAN_ZAMAN_ASIMI);
        assert!(i.zaman_asimi <= Duration::from_secs(15));
    }

    /// GERCEK AG — elle kosulur: `cargo test -p sirr-web -- --ignored`.
    /// CI aga bagimli olmasin diye `#[ignore]`; ama silinmesin diye burada.
    #[test]
    #[ignore = "gercek ag gerektirir"]
    fn duman_gercek_sunucuya_baglanir() {
        let i = WebAramaIstemcisi::yeni();
        let s = i.ara("rust programlama dili", Some("tr")).expect("arama basarili olmali");
        assert!(!s.is_empty());
        assert!(s.iter().all(|x| !x.url.is_empty() && !x.kaynak.is_empty()));
    }
}
