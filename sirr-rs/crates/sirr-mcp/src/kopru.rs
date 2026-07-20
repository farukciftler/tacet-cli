//! ARAC KOPRUSU: MCP'nin JSON Semasi -> bizim `ArgSema`.
//!
//! BURASI EN TEHLIKELI YER. `ArgSema` bilerek KAPALI ve KUCUKtur (bkz.
//! `sirr-cekirdek::sema`): gramere cevrilebilmesinin sarti bu. MCP sunuculari
//! ise tam JSON Semasi yazar — `oneOf`, `$ref`, `pattern`, `additionalProperties`.
//! Gelen sema bizimkinden GENIS oldugunda uc secenek var:
//!
//! 1. Sessizce daralt ve araci kabul et  -> YASAK. Gramer modeli sunucunun
//!    kabul etmeyecegi bir sekle ZORLAR; model dogru cagriyi uretemez ve neden
//!    olduğunu kimse goremez. Sessiz bozulma, en kotu bozulmadir.
//! 2. GUVENLI daraltma yap -> serbest. "Guvenli"nin tanimi asagida.
//! 3. Araci ATLA ve nedenini kaydet -> varsayilan.
//!
//! ## Guvenli daraltma nedir
//!
//! Bir kisiti dusurmek iki yone gidebilir:
//!
//! - **Daraltma** (gramerin kabul ettigi kume kuculur): tehlikeli olabilir,
//!   cunku sunucunun mesru sayacagi bir cagriyi model URETEMEZ. Yalniz kaybi
//!   olmayan haller kabul edilir: `["string","null"]` -> `Metin` (JSON'da
//!   atlanan alan zaten null sayilir, `dogrula` da oyle davranir), tek dalli
//!   `anyOf`/`oneOf` -> o dal.
//! - **Genisletme** (gramer daha cok sey kabul eder): guvenlidir, cunku
//!   sunucu kendi dogrulamasini yapmaya devam eder ve reddi modele NORMAL bir
//!   arac hatasi olarak doner. `pattern`, `format`, `multipleOf`, `uniqueItems`
//!   boyle dusurulur — ama SESSIZ degil: `AracKoprusu::notlar`a yazilir.
//!
//! ## Aciklama ozeti
//!
//! Swift tarafinin dersi (`mcp-baglanti-spec §5.3`): MCP arac aciklamalari
//! buyuk modeller icin yazilir, 100-500 belirtec surer. 20 araclik bir sunucu
//! 4096'lik pencereyi TEK BASINA bitirir. Swift bunu cihaz-ustu modele
//! ozetletiyordu; burada DETERMINISTIK kirpma yapiyoruz — ozetlemek icin model
//! cagirmak, arac katalogunu kurmayi model kalitesine bagimli kilardi ve ayni
//! sunucu her acilista farkli tanim uretirdi (eval karsilastirilamaz olurdu).

use serde_json::Value;
use sirr_cekirdek::{Alan, ArgSema, SemaTipi};

/// Ic ice gecme tavani. Kok nesne 1. seviyedir.
///
/// `mcp-baglanti-spec §5.2`nin "sema derinligi filtresi". Uc seviyeden derin
/// semalar hem gramerde yigin derinligi hem istemde okunabilirlik olarak
/// pahalilasiyor; ustelik 4096 pencereye giren bir arac tarifi zaten o kadar
/// derin olamaz.
pub const EN_COK_DERINLIK: usize = 3;

/// Ice aktarilan aciklamanin ust siniri (karakter).
///
/// Iki satirlik bir cumle ~160 karakter tutar; `§5.3`in "1-2 satira ozetlenir"
/// karari bu sayiya cevrildi.
pub const ACIKLAMA_SINIRI: usize = 160;

/// Bir aracin neden ice aktarilamadigi. Kullaniciya baglanti detayinda
/// "desteklenmiyor" diye gosterilir; sessizce yutulmaz.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CevrilemezSebep {
    /// `oneOf` / `anyOf` / `allOf` / `not` — kapali alt kumemizde karsiligi yok.
    BirlesikSema(String),
    /// `$ref` / `$defs` — cozumleme gerektirir; yerel olmayan referans da olabilir.
    Referans,
    /// Kok sema nesne degil (MCP `inputSchema` her zaman nesne olmali).
    KokNesneDegil,
    /// Tanimadigimiz ya da tasiyamadigimiz tip (`null`, `integer` disi ozel tipler).
    DesteklenmeyenTip(String),
    /// Tip hic bildirilmemis ve icerikten cikarilamiyor.
    TipYok(String),
    /// `EN_COK_DERINLIK` asildi.
    CokDerin,
    /// `enum` var ama degerlerin hepsi metin degil.
    KarisikEnum,
}

impl CevrilemezSebep {
    /// Kayda/loga yazilan tek satir. Kullaniciya gosterilir, modele GITMEZ.
    pub fn kisa(&self) -> String {
        match self {
            CevrilemezSebep::BirlesikSema(k) => format!("`{k}` semasi tasinamiyor"),
            CevrilemezSebep::Referans => "sema `$ref` iceriyor".into(),
            CevrilemezSebep::KokNesneDegil => "kok sema nesne degil".into(),
            CevrilemezSebep::DesteklenmeyenTip(t) => format!("desteklenmeyen tip: {t}"),
            CevrilemezSebep::TipYok(a) => format!("alanin tipi bildirilmemis: {a}"),
            CevrilemezSebep::CokDerin => {
                format!("sema {EN_COK_DERINLIK} seviyeden derin")
            }
            CevrilemezSebep::KarisikEnum => "enum degerleri metin degil".into(),
        }
    }
}

/// Cevrilen semanin yaninda, DUSURULEN kisitlarin dokumu.
///
/// Bos degilse arac yine de kullanilir; kayit kullaniciya "bu araci aldik ama
/// su kisiti zorlayamiyoruz" demek icindir.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CevrimNotlari(pub Vec<String>);

impl CevrimNotlari {
    fn dusuruldu(&mut self, yol: &str, kisit: &str) {
        self.0.push(format!("{yol}: `{kisit}` zorlanmiyor"));
    }

    pub fn bos_mu(&self) -> bool {
        self.0.is_empty()
    }
}

/// Cevrim sonucu.
pub struct Cevrim {
    pub sema: ArgSema,
    pub notlar: CevrimNotlari,
}

/// MCP `inputSchema` -> `ArgSema`.
///
/// Hata donerse arac ATLANIR; cagri yeri sebebi loglar.
pub fn semayi_cevir(giris: &Value) -> Result<Cevrim, CevrilemezSebep> {
    let mut notlar = CevrimNotlari::default();
    // MCP araclari argumansiz olabilir; sunucu `inputSchema`yi hic yollamamis
    // ya da `{}` yollamis olabilir. Ikisi de "arguman almaz" demektir, hata degil.
    if giris.is_null() || giris.as_object().is_some_and(|o| o.is_empty()) {
        return Ok(Cevrim { sema: ArgSema::bos(), notlar });
    }
    if !giris.is_object() {
        return Err(CevrilemezSebep::KokNesneDegil);
    }
    if tip_adi(giris).is_some_and(|t| t != "object") {
        return Err(CevrilemezSebep::KokNesneDegil);
    }
    let sema = dugum_cevir(giris, "arg", 1, &mut notlar)?;
    if !matches!(sema.tip, SemaTipi::Nesne { .. }) {
        return Err(CevrilemezSebep::KokNesneDegil);
    }
    Ok(Cevrim { sema, notlar })
}

fn dugum_cevir(
    d: &Value,
    yol: &str,
    derinlik: usize,
    notlar: &mut CevrimNotlari,
) -> Result<ArgSema, CevrilemezSebep> {
    if derinlik > EN_COK_DERINLIK {
        return Err(CevrilemezSebep::CokDerin);
    }
    let Some(nesne) = d.as_object() else {
        // `true`/`false` sema (JSON Schema'da gecerlidir) bize hicbir sey
        // soylemiyor; tipsiz alani kabul edersek gramer ne uretecegini bilemez.
        return Err(CevrilemezSebep::TipYok(yol.to_string()));
    };

    if nesne.contains_key("$ref") || nesne.contains_key("$defs") || nesne.contains_key("definitions")
    {
        return Err(CevrilemezSebep::Referans);
    }
    for anahtar in ["allOf", "not"] {
        if nesne.contains_key(anahtar) {
            return Err(CevrilemezSebep::BirlesikSema(anahtar.into()));
        }
    }
    // GUVENLI DARALTMA: tek dalli `anyOf`/`oneOf` bir secim degildir, sadece
    // gereksiz sarmaldir; o dala inmek hicbir mesru cagriyi elemez.
    for anahtar in ["anyOf", "oneOf"] {
        if let Some(dallar) = nesne.get(anahtar).and_then(Value::as_array) {
            let anlamli: Vec<&Value> = dallar.iter().filter(|b| !bos_tip(b)).collect();
            if anlamli.len() == 1 {
                return dugum_cevir(anlamli[0], yol, derinlik, notlar);
            }
            return Err(CevrilemezSebep::BirlesikSema(anahtar.into()));
        }
    }

    // `enum` tipten once bakilir: `{"enum":["a","b"]}` tip bildirmese de
    // kapali bir kumedir ve `Secenek`e birebir oturur.
    if let Some(degerler) = nesne.get("enum").and_then(Value::as_array) {
        let secenekler: Option<Vec<String>> = degerler
            .iter()
            .map(|v| v.as_str().map(str::to_string))
            .collect();
        let secenekler = secenekler.ok_or(CevrilemezSebep::KarisikEnum)?;
        if secenekler.is_empty() {
            return Err(CevrilemezSebep::KarisikEnum);
        }
        return Ok(aciklamayla(ArgSema::secenek(secenekler), nesne));
    }

    let tip = tip_adi(d).ok_or_else(|| tipsiz_sebep(nesne, yol))?;

    let sema = match tip.as_str() {
        "object" => {
            let mut alanlar = Vec::new();
            let zorunlular: Vec<&str> = nesne
                .get("required")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            // `properties` SIRASI serde_json'un `preserve_order` ozelligi
            // acik degilse alfabetiktir; hangisi olursa olsun deterministiktir,
            // yani ayni sunucu her acilista ayni gramere derlenir.
            if let Some(ozellikler) = nesne.get("properties").and_then(Value::as_object) {
                for (ad, alt) in ozellikler {
                    let alt_yol = format!("{yol}.{ad}");
                    let alt_sema = dugum_cevir(alt, &alt_yol, derinlik + 1, notlar)?;
                    let mut alan = Alan::yeni(ad.clone(), alt_sema);
                    if zorunlular.contains(&ad.as_str()) {
                        alan = alan.zorunlu();
                    }
                    alanlar.push(alan);
                }
            }
            // `additionalProperties` bir SEMA ise (bool degil) tanimadigimiz
            // ek alanlara tip veriyor demektir; `ArgSema`da karsiligi yok ve
            // yok saymak modele uydurma alan actirirdi.
            if nesne.get("additionalProperties").is_some_and(|v| v.is_object()) {
                return Err(CevrilemezSebep::BirlesikSema("additionalProperties".into()));
            }
            ArgSema::nesne(alanlar)
        }
        "array" => {
            let eleman = match nesne.get("items") {
                Some(i) => dugum_cevir(i, &format!("{yol}[]"), derinlik + 1, notlar)?,
                // Elemani tarif edilmemis dizi: gramer ne uretecegini bilemez.
                None => return Err(CevrilemezSebep::TipYok(format!("{yol}[]"))),
            };
            if nesne.get("prefixItems").is_some() {
                return Err(CevrilemezSebep::BirlesikSema("prefixItems".into()));
            }
            if nesne.contains_key("uniqueItems") {
                notlar.dusuruldu(yol, "uniqueItems");
            }
            ArgSema::dizi(eleman).uzunluk(sayi_usize(nesne.get("minItems")), sayi_usize(nesne.get("maxItems")))
        }
        "string" => {
            for kisit in ["pattern", "format", "minLength"] {
                if nesne.contains_key(kisit) {
                    notlar.dusuruldu(yol, kisit);
                }
            }
            let mut s = ArgSema::metin();
            if let Some(n) = sayi_usize(nesne.get("maxLength")) {
                s.tip = SemaTipi::Metin { en_cok_uzunluk: Some(n) };
            }
            s
        }
        "integer" | "number" => {
            for kisit in ["multipleOf", "exclusiveMinimum", "exclusiveMaximum"] {
                if nesne.contains_key(kisit) {
                    notlar.dusuruldu(yol, kisit);
                }
            }
            let temel = if tip == "integer" { ArgSema::tamsayi() } else { ArgSema::sayi() };
            temel.aralik(
                nesne.get("minimum").and_then(Value::as_f64),
                nesne.get("maximum").and_then(Value::as_f64),
            )
        }
        "boolean" => ArgSema::bool(),
        diger => return Err(CevrilemezSebep::DesteklenmeyenTip(diger.to_string())),
    };

    Ok(aciklamayla(sema, nesne))
}

/// Neden tip bulunamadi: bilgisiz mi, yoksa `null` gibi tasiyamadigimiz bir
/// tip mi? Ayrim kullaniciya gosterilen notta ise yarar.
fn tipsiz_sebep(
    nesne: &serde_json::Map<String, Value>,
    yol: &str,
) -> CevrilemezSebep {
    match nesne.get("type") {
        Some(Value::String(s)) => CevrilemezSebep::DesteklenmeyenTip(s.clone()),
        Some(Value::Array(_)) => CevrilemezSebep::DesteklenmeyenTip("birlesik tip".into()),
        _ => CevrilemezSebep::TipYok(yol.to_string()),
    }
}

/// Tip adi. `["string","null"]` gibi ikili birlesim GUVENLI DARALTMA ile
/// `string`e indirgenir: `null` bizim modelimizde "alan yok" demektir ve
/// `ArgSema::dogrula` da null'i eksik alan gibi ele alir — kayip yok.
fn tip_adi(d: &Value) -> Option<String> {
    match d.get("type") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(tipler)) => {
            let bos_olmayan: Vec<&str> = tipler
                .iter()
                .filter_map(Value::as_str)
                .filter(|t| *t != "null")
                .collect();
            match bos_olmayan.as_slice() {
                [tek] => Some((*tek).to_string()),
                _ => None,
            }
        }
        _ => None,
    }
}

/// `{}` ya da `{"type":"null"}` gibi bilgi tasimayan dal.
fn bos_tip(d: &Value) -> bool {
    match d.as_object() {
        Some(o) if o.is_empty() => true,
        Some(o) => o.get("type").and_then(Value::as_str) == Some("null"),
        None => false,
    }
}

fn sayi_usize(v: Option<&Value>) -> Option<usize> {
    v.and_then(Value::as_u64).map(|n| n as usize)
}

fn aciklamayla(sema: ArgSema, nesne: &serde_json::Map<String, Value>) -> ArgSema {
    match nesne.get("description").and_then(Value::as_str) {
        Some(a) if !a.trim().is_empty() => sema.aciklama(aciklamayi_kirp(a)),
        _ => sema,
    }
}

/// Uzun MCP aciklamasini istemde tasinabilir bir cumleye indirir (§5.3).
///
/// KURAL: once ilk cumleyi dene (nokta/soru/unlem); hala uzunsa kelime
/// sinirindan kes ve "…" koy. Kelime ortasindan kesmek modele bozuk bir sozcuk
/// ogretir; "…" ise "burada devami var" isaretidir ve modelin uydurmasini
/// engellemez ama kullaniciya durustur.
pub fn aciklamayi_kirp(ham: &str) -> String {
    let tek_satir = ham.split_whitespace().collect::<Vec<_>>().join(" ");
    if tek_satir.chars().count() <= ACIKLAMA_SINIRI {
        return tek_satir;
    }

    // Ilk cumle yeterince kisaysa en iyi ozet odur — yazarin kendi ozeti.
    if let Some(son) = ilk_cumle_sonu(&tek_satir) {
        let ilk: String = tek_satir.chars().take(son).collect();
        if ilk.chars().count() <= ACIKLAMA_SINIRI {
            return ilk;
        }
    }

    let kirpik: String = tek_satir.chars().take(ACIKLAMA_SINIRI).collect();
    let govde = match kirpik.rfind(' ') {
        // Son bosluk cok basta kaldiysa (uzun tek kelime) kelime siniri
        // aramanin anlami yok.
        Some(i) if i > ACIKLAMA_SINIRI / 2 => &kirpik[..i],
        _ => &kirpik,
    };
    format!("{}…", govde.trim_end_matches([',', ';', ' ']))
}

/// Ilk cumlenin bitis KARAKTER indeksi (noktalama dahil).
fn ilk_cumle_sonu(metin: &str) -> Option<usize> {
    let mut sayac = 0usize;
    for c in metin.chars() {
        sayac += 1;
        if matches!(c, '.' | '!' | '?') {
            return Some(sayac);
        }
    }
    None
}

#[cfg(test)]
mod testler {
    use super::*;
    use serde_json::json;

    fn cevir(v: Value) -> Result<ArgSema, CevrilemezSebep> {
        semayi_cevir(&v).map(|c| c.sema)
    }

    // --- CEVRILEBILIR ornekler ---

    #[test]
    fn duz_nesne_cevrilir_ve_zorunluluk_tasinir() {
        let sema = cevir(json!({
            "type": "object",
            "properties": {
                "komut": {"type": "string", "description": "kabuk komutu"},
                "zaman_asimi": {"type": "integer", "minimum": 1, "maximum": 600},
            },
            "required": ["komut"],
        }))
        .expect("cevrilmeli");

        let alanlar = sema.alanlar();
        assert_eq!(alanlar.len(), 2);
        let komut = alanlar.iter().find(|a| a.ad == "komut").expect("komut");
        assert!(komut.zorunlu);
        assert_eq!(komut.sema.aciklama.as_deref(), Some("kabuk komutu"));
        let ts = alanlar.iter().find(|a| a.ad == "zaman_asimi").expect("zaman_asimi");
        assert!(!ts.zorunlu);
        assert_eq!(
            ts.sema.tip,
            SemaTipi::Sayi { tam_mi: true, en_az: Some(1.0), en_cok: Some(600.0) }
        );
    }

    #[test]
    fn enum_secenege_cevrilir() {
        let sema = cevir(json!({
            "type": "object",
            "properties": { "kip": {"type": "string", "enum": ["oku", "yaz"]} },
        }))
        .expect("cevrilmeli");
        assert_eq!(
            sema.alanlar()[0].sema.tip,
            SemaTipi::Secenek { secenekler: vec!["oku".into(), "yaz".into()] }
        );
    }

    #[test]
    fn dizi_ve_sinirlari_tasinir() {
        let sema = cevir(json!({
            "type": "object",
            "properties": {
                "yollar": {"type": "array", "items": {"type": "string"}, "maxItems": 5},
            },
        }))
        .expect("cevrilmeli");
        let SemaTipi::Dizi { eleman, en_cok, .. } = &sema.alanlar()[0].sema.tip else {
            panic!("dizi bekleniyordu");
        };
        assert_eq!(*en_cok, Some(5));
        assert!(matches!(eleman.tip, SemaTipi::Metin { .. }));
    }

    #[test]
    fn argumansiz_arac_bos_semaya_duser() {
        assert_eq!(cevir(json!({})).expect("bos").alanlar().len(), 0);
        assert_eq!(cevir(Value::Null).expect("null").alanlar().len(), 0);
    }

    #[test]
    fn ic_ice_nesne_sinira_kadar_cevrilir() {
        // kok(1) -> dis(2) -> ic(3): tam sinirda, gecmeli.
        assert!(
            cevir(json!({
                "type": "object",
                "properties": { "dis": {
                    "type": "object",
                    "properties": { "ic": {"type": "string"} },
                }},
            }))
            .is_ok()
        );
    }

    // --- GUVENLI DARALTMALAR ---

    #[test]
    fn string_null_ikilisi_metne_daralir() {
        // JSON'da null = "alan yok"; `ArgSema::dogrula` da oyle davraniyor,
        // yani bu daraltma hicbir mesru cagriyi elemiyor.
        let sema = cevir(json!({
            "type": "object",
            "properties": { "not": {"type": ["string", "null"]} },
        }))
        .expect("cevrilmeli");
        assert!(matches!(sema.alanlar()[0].sema.tip, SemaTipi::Metin { .. }));
    }

    #[test]
    fn tek_dalli_anyof_o_dala_iner() {
        let sema = cevir(json!({
            "type": "object",
            "properties": { "x": {"anyOf": [{"type": "integer"}, {"type": "null"}]} },
        }))
        .expect("cevrilmeli");
        assert!(matches!(sema.alanlar()[0].sema.tip, SemaTipi::Sayi { tam_mi: true, .. }));
    }

    #[test]
    fn pattern_dusurulur_ama_kayda_gecer() {
        let cevrim = semayi_cevir(&json!({
            "type": "object",
            "properties": { "sha": {"type": "string", "pattern": "^[0-9a-f]{40}$"} },
        }))
        .expect("cevrilmeli");
        // Arac KABUL edilir (genisletme guvenlidir: sunucu kendi dogrular),
        // ama dusurulen kisit gorunur kalir.
        assert!(!cevrim.notlar.bos_mu());
        assert!(cevrim.notlar.0[0].contains("pattern"), "{:?}", cevrim.notlar);
    }

    // --- CEVRILEMEZLER: sessizce kabul EDILMEZ ---

    #[test]
    fn cok_dalli_oneof_reddedilir() {
        let hata = cevir(json!({
            "type": "object",
            "properties": { "hedef": {"oneOf": [{"type": "string"}, {"type": "integer"}]} },
        }))
        .unwrap_err();
        assert_eq!(hata, CevrilemezSebep::BirlesikSema("oneOf".into()));
    }

    #[test]
    fn ref_reddedilir() {
        let hata = cevir(json!({
            "type": "object",
            "properties": { "a": {"$ref": "#/$defs/Sey"} },
        }))
        .unwrap_err();
        assert_eq!(hata, CevrilemezSebep::Referans);
    }

    #[test]
    fn allof_reddedilir() {
        let hata = cevir(json!({
            "type": "object",
            "properties": { "a": {"allOf": [{"type": "string"}]} },
        }))
        .unwrap_err();
        assert_eq!(hata, CevrilemezSebep::BirlesikSema("allOf".into()));
    }

    #[test]
    fn cok_derin_sema_reddedilir() {
        let hata = cevir(json!({
            "type": "object",
            "properties": { "a": { "type": "object", "properties": {
                "b": { "type": "object", "properties": { "c": {"type": "string"} } },
            }}},
        }))
        .unwrap_err();
        assert_eq!(hata, CevrilemezSebep::CokDerin);
    }

    #[test]
    fn tipsiz_alan_reddedilir() {
        let hata = cevir(json!({
            "type": "object",
            "properties": { "serbest": {"description": "her sey olabilir"} },
        }))
        .unwrap_err();
        assert_eq!(hata, CevrilemezSebep::TipYok("arg.serbest".into()));
    }

    #[test]
    fn semali_additional_properties_reddedilir() {
        let hata = cevir(json!({
            "type": "object",
            "properties": {},
            "additionalProperties": {"type": "string"},
        }))
        .unwrap_err();
        assert_eq!(hata, CevrilemezSebep::BirlesikSema("additionalProperties".into()));
    }

    #[test]
    fn karisik_enum_reddedilir() {
        let hata = cevir(json!({
            "type": "object",
            "properties": { "k": {"enum": ["a", 3]} },
        }))
        .unwrap_err();
        assert_eq!(hata, CevrilemezSebep::KarisikEnum);
    }

    #[test]
    fn kok_nesne_degilse_reddedilir() {
        assert_eq!(
            cevir(json!({"type": "string"})).unwrap_err(),
            CevrilemezSebep::KokNesneDegil
        );
    }

    #[test]
    fn elemansiz_dizi_reddedilir() {
        assert_eq!(
            cevir(json!({"type": "object", "properties": {"a": {"type": "array"}}})).unwrap_err(),
            CevrilemezSebep::TipYok("arg.a[]".into())
        );
    }

    // --- Cevrilen sema GERCEKTEN calisir mi ---

    #[test]
    fn cevrilen_sema_dogru_argumani_kabul_yanlisini_reddeder() {
        // Kopru "derlendi" demek yetmez: uretilen sema cekirdegin dogrulamasindan
        // beklenen sekilde gecmeli, yoksa gramer modeli yanlis yere zorlar.
        let sema = cevir(json!({
            "type": "object",
            "properties": {
                "komut": {"type": "string"},
                "kip": {"type": "string", "enum": ["hizli", "yavas"]},
            },
            "required": ["komut"],
        }))
        .expect("cevrilmeli");

        assert!(sema.dogrula(&json!({"komut": "ls"})).is_ok());
        assert!(sema.dogrula(&json!({"komut": "ls", "kip": "hizli"})).is_ok());
        assert!(sema.dogrula(&json!({"kip": "hizli"})).is_err(), "zorunlu alan eksik");
        assert!(sema.dogrula(&json!({"komut": "ls", "kip": "orta"})).is_err(), "kume disi");
        assert!(sema.dogrula(&json!({"komut": 5})).is_err(), "tip yanlis");
    }

    // --- Aciklama kirpma ---

    #[test]
    fn kisa_aciklama_dokunulmaz() {
        assert_eq!(aciklamayi_kirp("Dosya okur."), "Dosya okur.");
    }

    #[test]
    fn satir_sonlari_tek_satira_indirgenir() {
        assert_eq!(aciklamayi_kirp("Dosya\n  okur.\n"), "Dosya okur.");
    }

    #[test]
    fn uzun_aciklamada_ilk_cumle_secilir() {
        let ham = format!("Kisa ozet cumlesi. {}", "Cok uzun ayrinti. ".repeat(40));
        let kirpik = aciklamayi_kirp(&ham);
        assert_eq!(kirpik, "Kisa ozet cumlesi.");
    }

    #[test]
    fn ilk_cumle_de_uzunsa_kelime_sinirindan_kesilir() {
        let ham = "kelime ".repeat(80);
        let kirpik = aciklamayi_kirp(&ham);
        assert!(kirpik.chars().count() <= ACIKLAMA_SINIRI + 1, "{kirpik}");
        assert!(kirpik.ends_with('…'));
        assert!(!kirpik.contains("kelim…"), "kelime ortasindan kesilmemeli: {kirpik}");
    }

    #[test]
    fn tek_uzun_kelime_panik_yapmaz() {
        let kirpik = aciklamayi_kirp(&"a".repeat(500));
        assert!(kirpik.chars().count() <= ACIKLAMA_SINIRI + 1);
    }
}
