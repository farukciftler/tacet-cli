//! `AramaSonucu` ve SearXNG JSON'unun ayristirilmasi.
//!
//! Bu dosyada AG YOK: girdisi bir `&str`dir. Bilincli, cunku ayristirmanin
//! butun kirli kosesi (eksik alan, null, bos `results`, HTML donmus yanit)
//! boylece aga cikmadan test edilebilir. Ag katmani yalniz "bayt getir"
//! isini yapar (`istemci.rs`), yorum burada olur.

use crate::hata::{WebHatasi, WebSonuc};
use serde_json::Value;

/// Tek bir arama sonucu.
///
/// `kaynak` alan adidir (`www.mgm.gov.tr`), tam URL degil. NEDEN AYRI ALAN:
/// modele giden metinde tam URL'in isi yok — hem token yer, hem de model
/// gordugu uzun bir adresi yanitinda YENIDEN URETMEYE calisip var olmayan
/// linkler halusine ediyor. Alan adi kaynagi durustce gosterir, uydurmaya
/// malzeme vermez. Tam adres `url` alaninda durur ve cip detayinda gorunur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AramaSonucu {
    pub baslik: String,
    pub url: String,
    pub ozet: String,
    pub kaynak: String,
}

/// SearXNG yanit govdesini `Vec<AramaSonucu>`ye cevirir.
///
/// TOLERANSLI ALAN OKUMASI, KATI GOVDE KONTROLU: tek bir sonucun `content`i
/// eksikse o sonuc bos ozetle gecer (SearXNG motorlari arasinda alan doluluk
/// orani degisir, bir motorun eksigi butun aramayi dusurmemeli). Ama govde
/// hic JSON degilse ya da `results` bir dizi degilse HATA doner — orada
/// "yumusak gecmek" sunucunun yanlis yapilandirildigini gizlerdi.
pub fn ayristir(govde: &str) -> WebSonuc<Vec<AramaSonucu>> {
    let kok: Value = serde_json::from_str(govde)
        .map_err(|e| WebHatasi::GecersizJson(format!("govde JSON degil: {e}")))?;

    let dizi = kok
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| WebHatasi::GecersizJson("yanitta 'results' dizisi yok".into()))?;

    let mut sonuclar = Vec::with_capacity(dizi.len());

    // Bilgi kutusu (infobox) varsa BASA alinir: "dolar kac lira" gibi
    // sorgularda dogrudan cevap oradadir, on siradaki organik sonuc ise
    // genellikle o cevabi iceren bir haber sayfasidir.
    if let Some(kutu) = kok.get("infoboxes").and_then(Value::as_array).and_then(|d| d.first()) {
        let icerik = metin_alan(kutu, "content");
        if !icerik.is_empty() {
            let url = kutu
                .get("urls")
                .and_then(Value::as_array)
                .and_then(|u| u.first())
                .map(|u| metin_alan(u, "url"))
                .unwrap_or_default();
            sonuclar.push(AramaSonucu {
                baslik: metin_alan(kutu, "infobox"),
                kaynak: alan_adi(&url),
                url,
                ozet: icerik,
            });
        }
    }

    for oge in dizi {
        let url = metin_alan(oge, "url");
        // URL'siz sonuc kullanilamaz: ne kaynak gosterilebilir ne getirilebilir.
        if url.is_empty() {
            continue;
        }
        sonuclar.push(AramaSonucu {
            baslik: metin_alan(oge, "title"),
            kaynak: alan_adi(&url),
            url,
            ozet: metin_alan(oge, "content"),
        });
    }

    if sonuclar.is_empty() {
        return Err(WebHatasi::BosSonuc);
    }
    Ok(sonuclar)
}

/// JSON alanini metne cevirir; yok ya da `null` ise bos metin.
fn metin_alan(deger: &Value, ad: &str) -> String {
    deger.get(ad).and_then(Value::as_str).unwrap_or_default().trim().to_string()
}

/// URL'den alan adini cikarir. Tam bir URL ayristiricisi DEGIL ve olmamali:
/// ihtiyacimiz olan tek sey sema ile ilk `/` arasindaki yetkili kismi; kullanici
/// bilgisi ve port da atilir ki `kullanici@host:8443` gibi bir adres kaynak
/// satirinda gurultu yapmasin.
pub fn alan_adi(url: &str) -> String {
    let kalan = url.split_once("://").map(|(_, k)| k).unwrap_or(url);
    let yetkili = kalan.split(['/', '?', '#']).next().unwrap_or("");
    let konak = yetkili.rsplit('@').next().unwrap_or(yetkili);
    konak.split(':').next().unwrap_or(konak).to_string()
}

/// Metni kelime sinirinda kirpar.
///
/// KELIME SINIRI ONEMLI: ham karakter kesigi ("...Türkiye'de enflasyon o") hem
/// okunmaz, hem de modele yarim bir olgu verir; model yarim cumleyi tamamlamaya
/// calisirken UYDURUR. Sinira denk bosluk bulunamazsa (uzun tek kelime, URL)
/// sert kesilir — sonsuza kadar geri gitmek butun ozeti yok ederdi.
pub fn kelime_sinirinda_kirp(metin: &str, en_cok: usize) -> String {
    if metin.chars().count() <= en_cok {
        return metin.to_string();
    }
    let kirpik: String = metin.chars().take(en_cok).collect();
    let kesim = match kirpik.rfind(char::is_whitespace) {
        // Cok erken bir bosluga dusmek ozetin yarisindan fazlasini atardi.
        Some(i) if i >= kirpik.len() / 2 => i,
        _ => kirpik.len(),
    };
    format!("{}…", kirpik[..kesim].trim_end())
}

#[cfg(test)]
mod testler {
    use super::*;

    /// Gercek sunucudan alinmis yanitin kisaltilmis hali — bicim birebir.
    const ORNEK: &str = r#"{
        "query": "rust async",
        "number_of_results": 0,
        "results": [
            {"title": "Async Rust", "url": "https://doc.rust-lang.org/book/ch17-00.html",
             "content": "Learn how to use Rust's async and await syntax.",
             "engine": "duckduckgo", "score": 4.0},
            {"title": "Async book", "url": "https://rust-lang.github.io/async-book/",
             "content": "Learn how to write concurrent code.", "engine": "google", "score": 2.0}
        ]
    }"#;

    #[test]
    fn ornek_yanit_ayristirilir_ve_alan_adi_cikarilir() {
        let s = ayristir(ORNEK).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].baslik, "Async Rust");
        assert_eq!(s[0].kaynak, "doc.rust-lang.org");
        assert_eq!(s[1].kaynak, "rust-lang.github.io");
        assert!(s[0].ozet.starts_with("Learn how"));
    }

    #[test]
    fn eksik_alanlar_sonucu_dusurmez_ama_urlsiz_sonuc_atilir() {
        let govde = r#"{"results":[
            {"url":"https://a.example/x"},
            {"title":"urlsiz","content":"metin"}
        ]}"#;
        let s = ayristir(govde).unwrap();
        assert_eq!(s.len(), 1, "URL'siz sonuc atilmali");
        assert_eq!(s[0].kaynak, "a.example");
        assert_eq!(s[0].baslik, "");
        assert_eq!(s[0].ozet, "");
    }

    #[test]
    fn infobox_basa_alinir() {
        let govde = r#"{"results":[{"url":"https://haber.example/a","title":"haber"}],
            "infoboxes":[{"infobox":"USD","content":"1 USD = 41,2 TRY",
                          "urls":[{"url":"https://tcmb.gov.tr/kur"}]}]}"#;
        let s = ayristir(govde).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].ozet, "1 USD = 41,2 TRY");
        assert_eq!(s[0].kaynak, "tcmb.gov.tr");
    }

    #[test]
    fn icerigi_bos_infobox_yok_sayilir() {
        let govde = r#"{"results":[{"url":"https://a.example/x"}],
                        "infoboxes":[{"infobox":"bos","content":""}]}"#;
        assert_eq!(ayristir(govde).unwrap().len(), 1);
    }

    #[test]
    fn bos_results_bos_sonuc_hatasi_verir() {
        assert_eq!(ayristir(r#"{"results":[]}"#), Err(WebHatasi::BosSonuc));
    }

    /// SearXNG'de `formats: json` kapaliyken sunucu 200 + HTML doner.
    /// Bu SESSIZCE gecerse kullanici "arama bozuk" der, sebebini bulamaz.
    #[test]
    fn json_yerine_html_donerse_gecersiz_json() {
        let h = ayristir("<!DOCTYPE html><html><body>ara</body></html>").unwrap_err();
        assert!(matches!(h, WebHatasi::GecersizJson(_)));
        assert!(h.to_string().contains("formats: json"));
    }

    #[test]
    fn results_alani_yoksa_gecersiz_json() {
        let h = ayristir(r#"{"query":"x"}"#).unwrap_err();
        assert!(matches!(h, WebHatasi::GecersizJson(_)));
    }

    #[test]
    fn results_dizi_degilse_gecersiz_json() {
        assert!(matches!(ayristir(r#"{"results":"yok"}"#), Err(WebHatasi::GecersizJson(_))));
    }

    #[test]
    fn alan_adi_port_kullanici_ve_yolu_atar() {
        assert_eq!(alan_adi("https://kullanici@www.mgm.gov.tr:8443/tahmin?il=34"), "www.mgm.gov.tr");
        assert_eq!(alan_adi("http://localhost:8080/a"), "localhost");
        assert_eq!(alan_adi("bozuk-adres"), "bozuk-adres");
        assert_eq!(alan_adi(""), "");
    }

    #[test]
    fn kirpma_kelime_sinirini_korur() {
        let m = "bir iki uc dort bes alti yedi sekiz";
        let k = kelime_sinirinda_kirp(m, 20);
        assert!(k.ends_with('…'));
        assert!(!k.contains("dor…"), "kelime ortasindan kesilmemeli: {k}");
        assert_eq!(kelime_sinirinda_kirp("kisa", 20), "kisa");
    }

    #[test]
    fn bosluksuz_uzun_metin_sert_kesilir() {
        // Geri gidip bosluk arayan bir uygulama burada ozetin tamamini atardi.
        let k = kelime_sinirinda_kirp(&"a".repeat(100), 10);
        assert_eq!(k.chars().count(), 11, "10 karakter + elips");
    }

    #[test]
    fn kirpma_cok_baytli_karakterde_panik_yapmaz() {
        // `chars().take()` ile kesildigi icin UTF-8 siniri hep gecerli.
        let k = kelime_sinirinda_kirp("çığır açan ölçüm şğüöçİ ile", 12);
        assert!(k.ends_with('…'));
    }
}
