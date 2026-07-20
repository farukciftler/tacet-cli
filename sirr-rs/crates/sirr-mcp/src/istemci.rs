//! Tek bir MCP sunucusuyla konusan istemci — bu crate'teki TEK soket sahibi.
//!
//! Tasima: **Streamable HTTP**. JSON-RPC 2.0 govdeleri HTTP POST ile gider;
//! yanit `application/json` DA olabilir `text/event-stream` (SSE) DE. Ikisini
//! de kabul ederiz, hangisinin gelecegini sunucu secer.
//!
//! ## Neden bloklayan
//!
//! Bu is agacinda calistirici (runtime) YOK: `sirr-motor::bekle` gelecekleri
//! tek is parcaciginda mesgul-bekleyerek suruyor. `MCPIstemcisi` bloklayan
//! oldugu icin cagri yeri (`sirr-araclar::mcp`) onu dogrudan `async` govdeden
//! cagirabiliyor ve mimariye tokio dayatilmiyor. Bunun bedeli acikca soylenmeli:
//! bir MCP cagrisi surerken o is parcacigi baska is yapmaz.
//!
//! ## Neden `&self` + `Mutex`
//!
//! `Arac::calistir` `&self` alir ve katalog `Arc<dyn Arac>` tutar; oturum
//! kimligi gibi degisen durum ic kilit altinda yasamak zorunda.

use crate::hata::{MCPHatasi, MCPSonuc};
use crate::jsonrpc;
use crate::sse;
use serde_json::{Value, json};
use std::io::BufReader;
use std::sync::Mutex;
use std::time::Duration;

/// Bu istemcinin konustugu MCP surumu. Sunucu baskasini onerirse onunkine
/// uyulur (MCP surum anlasmasi).
pub const PROTOKOL_SURUMU: &str = "2025-06-18";

/// §5.7 — uzak taraf build gibi uzun isler yapabilir.
pub const ZAMAN_ASIMI_SN: u64 = 120;

/// `tools/list` sayfalama dongusunun tavani: kotu davranan sunucu ayni imleci
/// tekrar tekrar dondurup bizi sonsuza kilitlemesin.
const EN_FAZLA_SAYFA: usize = 20;

/// Arac sayisi tavani. 4096'lik pencereye zaten 6-8 arac giriyor; binlerce
/// araci bellege almanin kimseye faydasi yok.
const EN_FAZLA_ARAC: usize = 200;

/// Sunucudan gelen tek arac tanimi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AracTanimi {
    pub ad: String,
    /// HAM aciklama — 100-500 belirtec olabilir. Baglama BU HALIYLE GIRMEZ;
    /// `kopru::aciklamayi_kirp` ile kisaltilir (§5.3).
    pub aciklama: String,
    /// `inputSchema` (JSON Semasi). Kopru bunu `ArgSema`ya cevirir.
    pub sema: Value,
}

/// Sunucu-durum: el sikisma sonrasi tasinan seyler.
#[derive(Default)]
struct Durum {
    oturum_kimligi: Option<String>,
    uzlasilan_surum: Option<String>,
    el_sikisildi: bool,
    sayac: u64,
}

pub struct MCPIstemcisi {
    url: String,
    /// Bearer anahtari. Yalniz bellekte; loga ve cip metnine ASLA yazilmaz.
    anahtar: Option<String>,
    ajan: ureq::Agent,
    durum: Mutex<Durum>,
}

impl MCPIstemcisi {
    /// `url` kabul edilmezse burada durur — ag'a hic cikilmaz.
    pub fn yeni(url: impl Into<String>, anahtar: Option<String>) -> MCPSonuc<Self> {
        let url = url.into();
        url_dogrula(&url)?;
        let ayar = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(ZAMAN_ASIMI_SN)))
            // Durum kodunu KENDIMIZ okumak istiyoruz: 401 ile 500 kullaniciya
            // farkli cumleler soyler, ureq'in tek "status error"u bu ayrimi siler.
            .http_status_as_error(false)
            .user_agent("sirr/1.0")
            .build();
        Ok(Self {
            url,
            anahtar,
            ajan: ayar.into(),
            durum: Mutex::new(Durum::default()),
        })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    // --- Uc metot (v1 kapsami) ---

    /// El sikisma. Bir kez yapilir; sonraki cagrilar dokunmaz.
    pub fn el_sikis(&self) -> MCPSonuc<()> {
        if self.durum.lock().expect("mcp durum kilidi").el_sikisildi {
            return Ok(());
        }
        let sonuc = self.cagir(
            "initialize",
            json!({
                "protocolVersion": PROTOKOL_SURUMU,
                "capabilities": { "tools": {} },
                "clientInfo": { "name": "sirr", "version": "1.0" },
            }),
        )?;
        {
            let mut d = self.durum.lock().expect("mcp durum kilidi");
            if let Some(s) = sonuc
                .get("protocolVersion")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                d.uzlasilan_surum = Some(s.to_string());
            }
            d.el_sikisildi = true;
        }
        // El sikismanin ikinci yarisi: yanit beklenmeyen bildirim. Ayri bir
        // "metot" degil, `initialize`in parcasi — atlanirsa kati sunucular
        // `tools/list`i reddediyor. Basarisizligi yutuyoruz cunku sunuculardan
        // bir kismi bildirime 202 disi bir sey donuyor ve bu bizi durdurmamali.
        let _ = self.bildir("notifications/initialized");
        Ok(())
    }

    /// Sunucunun araclari. Sayfalama (`nextCursor`) sonuna kadar donulur.
    pub fn araclar(&self) -> MCPSonuc<Vec<AracTanimi>> {
        self.el_sikis()?;
        let mut toplam: Vec<AracTanimi> = Vec::new();
        let mut imlec: Option<String> = None;
        let mut sayfa = 0usize;

        loop {
            sayfa += 1;
            let parametre = match &imlec {
                Some(i) => json!({ "cursor": i }),
                None => json!({}),
            };
            let sonuc = self.cagir("tools/list", parametre)?;
            toplam.extend(tanimlari_ayikla(&sonuc));

            let sonraki = sonuc
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            // Ayni imleci tekrar veren sunucu bizi dongude tutamasin.
            imlec = if sonraki == imlec { None } else { sonraki };
            if imlec.is_none() || sayfa >= EN_FAZLA_SAYFA || toplam.len() >= EN_FAZLA_ARAC {
                break;
            }
        }

        toplam.truncate(EN_FAZLA_ARAC);
        Ok(toplam)
    }

    /// Uzak araci cagirir ve icerigi duz metne indirger.
    ///
    /// ONEMLI: onay kapisi buradan ONCE, `AracYurutucu`da gecilir. Buraya gelen
    /// her sey kullanicinin gordugu ve onayladigi seydir.
    ///
    /// Donen `hatali_mi`, SUNUCUNUN kendi arac hatasidir (`isError`): tasima
    /// hatasi degil, aracin normal sonucudur ve modele anlatilir.
    pub fn arac_cagir(&self, ad: &str, argumanlar: &Value) -> MCPSonuc<(String, bool)> {
        self.el_sikis()?;
        let sonuc = self.cagir(
            "tools/call",
            json!({ "name": ad, "arguments": argumanlar }),
        )?;
        Ok(icerigi_duzlestir(&sonuc))
    }

    // --- Tasima ---

    fn sonraki_kimlik(&self) -> u64 {
        let mut d = self.durum.lock().expect("mcp durum kilidi");
        d.sayac += 1;
        d.sayac
    }

    fn istek_kur(&self) -> ureq::RequestBuilder<ureq::typestate::WithBody> {
        let d = self.durum.lock().expect("mcp durum kilidi");
        let mut istek = self
            .ajan
            .post(&self.url)
            .header("Content-Type", "application/json")
            // Iki bicimi de kabul ederiz; hangisini kullanacagini sunucu secer.
            .header("Accept", "application/json, text/event-stream");
        if d.el_sikisildi {
            let surum = d.uzlasilan_surum.as_deref().unwrap_or(PROTOKOL_SURUMU);
            istek = istek.header("MCP-Protocol-Version", surum);
        }
        if let Some(oturum) = &d.oturum_kimligi {
            istek = istek.header("Mcp-Session-Id", oturum);
        }
        if let Some(anahtar) = self.anahtar.as_deref().filter(|a| !a.is_empty()) {
            istek = istek.header("Authorization", &format!("Bearer {anahtar}"));
        }
        istek
    }

    /// Yanit beklemeyen bildirim.
    fn bildir(&self, metot: &str) -> MCPSonuc<()> {
        let govde = jsonrpc::bildirim_govdesi(metot);
        self.istek_kur().send(&govde[..]).map_err(cevir_hata)?;
        Ok(())
    }

    /// Tek JSON-RPC cagrisi — `result` nesnesini dondurur.
    fn cagir(&self, metot: &str, parametre: Value) -> MCPSonuc<Value> {
        let kimlik = self.sonraki_kimlik();
        let govde = jsonrpc::istek_govdesi(kimlik, metot, parametre);
        let mut yanit = self.istek_kur().send(&govde[..]).map_err(cevir_hata)?;

        // Oturum kimligi el sikismada gelir, sonraki her istekte tasinir.
        if let Some(oturum) = yanit
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
        {
            self.durum.lock().expect("mcp durum kilidi").oturum_kimligi = Some(oturum);
        }

        let kod = yanit.status().as_u16();
        match kod {
            200..=299 => {}
            401 | 403 => return Err(MCPHatasi::Yetki),
            _ => return Err(MCPHatasi::Sunucu(format!("HTTP {kod}"))),
        }

        let sse_mi = yanit
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.to_ascii_lowercase().contains("text/event-stream"));

        let okuyucu = BufReader::new(yanit.body_mut().as_reader());
        let govde = if sse_mi {
            sse::olay_bul(okuyucu, kimlik)?
        } else {
            let coz: Value = serde_json::from_reader(okuyucu).map_err(|_| MCPHatasi::Bicimsiz)?;
            jsonrpc::yanit_sec(&coz, kimlik)
                .cloned()
                .ok_or(MCPHatasi::Bicimsiz)?
        };

        jsonrpc::sonucu_ayikla(&govde)
    }
}

/// `tools/list` sonucundan tanimlari cikarir. SAF — testi ag istemiyor.
///
/// Adsiz oge SESSIZCE atlanir: adi olmayan bir araci katalogda gosteremeyiz,
/// modelin cagirabilecegi bir imzasi yoktur.
pub fn tanimlari_ayikla(sonuc: &Value) -> Vec<AracTanimi> {
    sonuc
        .get("tools")
        .and_then(Value::as_array)
        .map(|ogeler| {
            ogeler
                .iter()
                .filter_map(|o| {
                    let ad = o.get("name").and_then(Value::as_str)?;
                    if ad.is_empty() {
                        return None;
                    }
                    Some(AracTanimi {
                        ad: ad.to_string(),
                        aciklama: o
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        sema: o.get("inputSchema").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `tools/call` sonucunun `content` dizisini duz metne indirger.
///
/// Metin disi icerik (gorsel/ses) v1'de tasinmaz ama SESSIZCE YUTULMAZ: yerine
/// bir isaret konur, yoksa model "arac bos dondu" sanip isi tekrar dener.
pub fn icerigi_duzlestir(sonuc: &Value) -> (String, bool) {
    let parcalar: Vec<String> = sonuc
        .get("content")
        .and_then(Value::as_array)
        .map(|ogeler| {
            ogeler
                .iter()
                .filter_map(|o| {
                    if let Some(t) = o.get("text").and_then(Value::as_str) {
                        return Some(t.to_string());
                    }
                    match o.get("type").and_then(Value::as_str) {
                        Some(tur) if tur != "text" => Some(format!("[{tur} content omitted]")),
                        _ => None,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let hatali = sonuc.get("isError").and_then(Value::as_bool).unwrap_or(false);
    (parcalar.join("\n"), hatali)
}

/// Adres kabul kurali (§3.1): `https://` her yerde, duz `http://` YALNIZ yerel
/// ag adreslerinde.
///
/// NEDEN BURADA: kural ag katmaninin kendisinde durmali. Ayarlar ekraninda
/// (ya da yapilandirma dosyasini elle yazan kullanicinin kafasinda) durursa
/// ikinci bir cagri yolu onu atlar ve token duz metin gider.
pub fn url_dogrula(url: &str) -> MCPSonuc<()> {
    let hata = || MCPHatasi::GecersizAdres(url.to_string());
    if let Some(kalan) = url.strip_prefix("https://") {
        return if kalan.is_empty() { Err(hata()) } else { Ok(()) };
    }
    let Some(kalan) = url.strip_prefix("http://") else {
        return Err(hata());
    };
    let konak = kalan
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .rsplit_once(':')
        .map(|(k, _)| k)
        .unwrap_or_else(|| kalan.split(['/', '?', '#']).next().unwrap_or_default())
        .trim_matches(['[', ']'])
        .to_ascii_lowercase();

    if yerel_mi(&konak) { Ok(()) } else { Err(hata()) }
}

/// Yerel ag adresi mi. Liste dar tutuldu: emin olamadigimiz her adres
/// `https` istemeye zorlanir.
fn yerel_mi(konak: &str) -> bool {
    if matches!(konak, "localhost" | "127.0.0.1" | "::1") || konak.ends_with(".local") {
        return true;
    }
    let sekizli: Vec<u8> = konak.split('.').filter_map(|p| p.parse::<u8>().ok()).collect();
    if sekizli.len() != 4 || konak.split('.').count() != 4 {
        return false;
    }
    match sekizli.as_slice() {
        [10, ..] => true,
        [192, 168, ..] => true,
        [172, ikinci, ..] if (16..=31).contains(ikinci) => true,
        [127, ..] => true,
        _ => false,
    }
}

/// `ureq::Error` -> duz dille ayrisan `MCPHatasi` (§3.1).
fn cevir_hata(hata: ureq::Error) -> MCPHatasi {
    match hata {
        ureq::Error::Timeout(_) => MCPHatasi::ZamanAsimi,
        ureq::Error::Tls(_) | ureq::Error::Pem(_) => MCPHatasi::Tls,
        ureq::Error::StatusCode(401) | ureq::Error::StatusCode(403) => MCPHatasi::Yetki,
        ureq::Error::StatusCode(kod) => MCPHatasi::Sunucu(format!("HTTP {kod}")),
        ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::TimedOut => MCPHatasi::ZamanAsimi,
        // Geri kalan her sey kullanicinin gozunde ayni sey: sunucuya
        // ulasilamadi. Ham `ureq` metnini gostermek kimseye yardim etmez.
        _ => MCPHatasi::Erisilemedi,
    }
}

#[cfg(test)]
mod testler {
    use super::*;
    use serde_json::json;

    #[test]
    fn https_her_zaman_kabul() {
        assert!(url_dogrula("https://ornek.com/mcp").is_ok());
    }

    #[test]
    fn duz_http_yalniz_yerel_agda() {
        assert!(url_dogrula("http://localhost:8080/mcp").is_ok());
        assert!(url_dogrula("http://127.0.0.1:3000").is_ok());
        assert!(url_dogrula("http://192.168.1.20/mcp").is_ok());
        assert!(url_dogrula("http://10.0.0.5/mcp").is_ok());
        assert!(url_dogrula("http://172.20.0.3/mcp").is_ok());
        assert!(url_dogrula("http://sunucu.local/mcp").is_ok());

        assert!(url_dogrula("http://ornek.com/mcp").is_err());
        assert!(url_dogrula("http://172.32.0.1/mcp").is_err(), "172.32 ozel aralik disi");
        assert!(url_dogrula("http://8.8.8.8/mcp").is_err());
    }

    #[test]
    fn http_disi_semalar_reddedilir() {
        for u in ["ws://localhost/mcp", "file:///etc/passwd", "ornek.com", ""] {
            assert!(url_dogrula(u).is_err(), "{u} kabul edilmemeliydi");
        }
    }

    #[test]
    fn gecersiz_adres_istemciyi_kurdurmaz() {
        // Kural ag katmaninda: kotu adresle bir istemci NESNESI bile olusmaz,
        // dolayisiyla o adrese kazara istek atacak bir yol kalmaz.
        assert!(MCPIstemcisi::yeni("http://ornek.com/mcp", None).is_err());
    }

    #[test]
    fn tanimlar_ayiklanir_adsiz_oge_atlanir() {
        let sonuc = json!({"tools": [
            {"name": "calistir", "description": "komut", "inputSchema": {"type": "object"}},
            {"description": "adsiz"},
            {"name": ""},
            {"name": "sade"},
        ]});
        let tanimlar = tanimlari_ayikla(&sonuc);
        assert_eq!(tanimlar.len(), 2);
        assert_eq!(tanimlar[0].ad, "calistir");
        assert_eq!(tanimlar[1].ad, "sade");
        assert_eq!(tanimlar[1].sema, Value::Null, "semasiz arac null semayla gelir");
    }

    #[test]
    fn icerik_duzlestirilir() {
        let sonuc = json!({"content": [
            {"type": "text", "text": "birinci"},
            {"type": "text", "text": "ikinci"},
        ]});
        assert_eq!(icerigi_duzlestir(&sonuc), ("birinci\nikinci".into(), false));
    }

    #[test]
    fn metin_disi_icerik_sessizce_yutulmaz() {
        let sonuc = json!({"content": [{"type": "image", "data": "..."}]});
        let (metin, hatali) = icerigi_duzlestir(&sonuc);
        assert_eq!(metin, "[image content omitted]");
        assert!(!hatali);
    }

    #[test]
    fn sunucunun_kendi_arac_hatasi_isaretlenir() {
        let sonuc = json!({"content": [{"type":"text","text":"exit 1"}], "isError": true});
        assert_eq!(icerigi_duzlestir(&sonuc), ("exit 1".into(), true));
    }

    /// AG'A CIKAR — bilincli `#[ignore]`. `cargo test -p sirr-mcp -- --ignored`
    /// ile ve SIRR_MCP_TEST_URL verildiginde kosar.
    #[test]
    #[ignore = "aga cikar; SIRR_MCP_TEST_URL gerekir"]
    fn gercek_sunucuyla_el_sikisma() {
        let Ok(url) = std::env::var("SIRR_MCP_TEST_URL") else {
            panic!("SIRR_MCP_TEST_URL tanimli degil");
        };
        let istemci =
            MCPIstemcisi::yeni(url, std::env::var("SIRR_MCP_TEST_ANAHTAR").ok()).expect("istemci");
        istemci.el_sikis().expect("el sikisma");
        let araclar = istemci.araclar().expect("tools/list");
        println!("{} arac bulundu", araclar.len());
        for a in &araclar {
            println!("  {} — {}", a.ad, crate::kopru::aciklamayi_kirp(&a.aciklama));
        }
    }
}
