//! JSON-RPC 2.0 cerceveleme — SAF, agdan bagimsiz.
//!
//! NEDEN AYRI: istek uretimi ve yanit secimi protokolun tek kirilgan yeri
//! (id eslesmesi, batch dizi, `error` vs `result`). Soketten ayirinca hepsi
//! ag'a cikmadan test edilebilir; `istemci` modulune yalniz "bayt tasi" isi kalir.

use crate::hata::{MCPHatasi, MCPSonuc};
use serde_json::{Value, json};

/// Yanit bekleyen istek govdesi.
pub fn istek_govdesi(kimlik: u64, metot: &str, parametre: Value) -> Vec<u8> {
    json!({
        "jsonrpc": "2.0",
        "id": kimlik,
        "method": metot,
        "params": parametre,
    })
    .to_string()
    .into_bytes()
}

/// Yanit BEKLENMEYEN bildirim govdesi (`id` YOK — JSON-RPC'de fark budur).
///
/// `notifications/initialized` bunun tek kullanicisi: MCP el sikismasinin
/// ikinci yarisidir ve atlanirsa kati sunucular `tools/list`i reddeder.
pub fn bildirim_govdesi(metot: &str) -> Vec<u8> {
    json!({ "jsonrpc": "2.0", "method": metot }).to_string().into_bytes()
}

/// Gelen govdeden BIZIM id'mize ait yaniti secer.
///
/// Sunucu tek nesne de dondurebilir, batch dizi de. Dizi halinde bizimkini
/// SECMEK zorunludur: korukorune ilkini almak baska bir cagrinin sonucunu
/// bizim cagriya mal ederdi.
pub fn yanit_sec(govde: &Value, kimlik: u64) -> Option<&Value> {
    match govde {
        Value::Array(ogeler) => ogeler.iter().find(|o| kimlik_esit(o, kimlik)),
        nesne if kimlik_esit(nesne, kimlik) => Some(nesne),
        _ => None,
    }
}

/// JSON-RPC id'si sayi ya da METIN olabilir (spec ikisine de izin verir ve
/// gercek sunucular ikisini de yolluyor). Biz her zaman sayi gonderiyoruz ama
/// yaniti metin sarmalayan sunucuyu reddetmek gereksiz katilik olurdu.
fn kimlik_esit(nesne: &Value, kimlik: u64) -> bool {
    match nesne.get("id") {
        Some(Value::Number(n)) => n.as_u64() == Some(kimlik),
        Some(Value::String(s)) => s.parse::<u64>().ok() == Some(kimlik),
        _ => false,
    }
}

/// `result`i cikarir; `error` varsa sunucu hatasina cevirir.
pub fn sonucu_ayikla(yanit: &Value) -> MCPSonuc<Value> {
    if let Some(hata) = yanit.get("error") {
        let mesaj = hata.get("message").and_then(Value::as_str).unwrap_or_default();
        return Err(MCPHatasi::Sunucu(mesaj.to_string()));
    }
    yanit.get("result").cloned().ok_or(MCPHatasi::Bicimsiz)
}

#[cfg(test)]
mod testler {
    use super::*;

    #[test]
    fn istek_govdesi_jsonrpc_alanlarini_tasir() {
        let bayt = istek_govdesi(7, "tools/list", json!({"cursor": "a"}));
        let v: Value = serde_json::from_slice(&bayt).expect("json");
        assert_eq!(v["jsonrpc"], json!("2.0"));
        assert_eq!(v["id"], json!(7));
        assert_eq!(v["method"], json!("tools/list"));
        assert_eq!(v["params"]["cursor"], json!("a"));
    }

    #[test]
    fn bildirimde_id_bulunmaz() {
        let v: Value = serde_json::from_slice(&bildirim_govdesi("notifications/initialized"))
            .expect("json");
        assert!(v.get("id").is_none(), "bildirimde id olmamali: {v}");
        assert_eq!(v["method"], json!("notifications/initialized"));
    }

    #[test]
    fn batch_icinden_dogru_id_secilir() {
        let govde = json!([
            {"jsonrpc":"2.0","id":1,"result":"a"},
            {"jsonrpc":"2.0","id":2,"result":"b"},
        ]);
        assert_eq!(yanit_sec(&govde, 2).expect("yanit")["result"], json!("b"));
        assert!(yanit_sec(&govde, 3).is_none());
    }

    #[test]
    fn metin_id_de_eslesir() {
        let govde = json!({"jsonrpc":"2.0","id":"12","result":1});
        assert!(yanit_sec(&govde, 12).is_some());
    }

    #[test]
    fn error_govdesi_sunucu_hatasina_cevrilir() {
        let yanit = json!({"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"yok"}});
        assert_eq!(sonucu_ayikla(&yanit).unwrap_err(), MCPHatasi::Sunucu("yok".into()));
    }

    #[test]
    fn ne_result_ne_error_bicimsizdir() {
        assert_eq!(
            sonucu_ayikla(&json!({"jsonrpc":"2.0","id":1})).unwrap_err(),
            MCPHatasi::Bicimsiz
        );
    }
}
