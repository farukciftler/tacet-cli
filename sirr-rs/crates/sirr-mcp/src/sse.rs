//! Streamable HTTP'nin SSE yarisi.
//!
//! NEDEN AYRI MODUL: ayristirma agdan tamamen bagimsiz — girdisi bir `BufRead`.
//! Boylece kotu bicimli akislar (yarim olay, arada heartbeat, batch dizi,
//! bizim olmayan id) AGA CIKMADAN test edilebilir. Ag'a cikan tek test
//! `#[ignore]`; asil davranis burada kanitlanir.
//!
//! Bicim (WHATWG EventSource'un ihtiyacimiz olan alt kumesi): `data:` satirlari
//! birikir, BOS SATIR olayi tamamlar. `:` ile baslayan satir yorum/heartbeat'tir.
//! `event:`/`id:`/`retry:` alanlari MCP JSON-RPC tasimasinda anlam tasimaz,
//! atlanir.

use crate::hata::{MCPHatasi, MCPSonuc};
use serde_json::Value;
use std::io::BufRead;

/// Akisi okur ve `kimlik`e ait JSON-RPC yanitini dondurur.
///
/// NEDEN "BULANA KADAR OKU": sunucu bizim yanittan once ilerleme/log olaylari
/// yollayabilir (uzun bir build'in ciktisi gibi). Ilk olayi alip donsek uzun
/// suren her cagri "bicimsiz" derdi.
///
/// Tavan yok cunku ustteki `zaman_sinirli` katmani (istemci) sureyi kesiyor;
/// burada ikinci bir sayac tutmak iki ayri "ne kadar bekleriz" gercegi yaratirdi.
pub fn olay_bul<R: BufRead>(okuyucu: R, kimlik: u64) -> MCPSonuc<Value> {
    let mut tampon: Vec<String> = Vec::new();

    for satir in okuyucu.lines() {
        let Ok(satir) = satir else {
            // Akis ortasinda kopan baglanti: elimizde yarim olay olabilir,
            // once ona bakariz — sunucu son olayi bos satirla kapatmamis olabilir.
            break;
        };
        // `\r\n` ile gonderen sunucular var; `lines()` yalniz `\n` ayirir.
        let satir = satir.trim_end_matches('\r');

        if satir.is_empty() {
            if let Some(olay) = olayi_coz(&mut tampon, kimlik) {
                return Ok(olay);
            }
            continue;
        }
        if satir.starts_with(':') {
            continue; // yorum / heartbeat
        }
        let Some(parca) = satir.strip_prefix("data:") else {
            continue; // event:/id:/retry: bizi ilgilendirmiyor
        };
        // Alan degerinin basindaki TEK bosluk bicimin parcasidir, verinin degil.
        tampon.push(parca.strip_prefix(' ').unwrap_or(parca).to_string());
    }

    // Akis kapandi: son olay bos satirla kapanmamis olabilir.
    olayi_coz(&mut tampon, kimlik).ok_or(MCPHatasi::Bicimsiz)
}

/// Biriken `data:` satirlarini tek JSON govdesi sayip bizim id'ye bakar.
/// Bizim olmayan olay SESSIZCE atilir (tampon bosaltilir) ve okuma surer.
fn olayi_coz(tampon: &mut Vec<String>, kimlik: u64) -> Option<Value> {
    if tampon.is_empty() {
        return None;
    }
    let govde = tampon.join("\n");
    tampon.clear();
    let coz: Value = serde_json::from_str(&govde).ok()?;
    crate::jsonrpc::yanit_sec(&coz, kimlik).cloned()
}

#[cfg(test)]
mod testler {
    use super::*;
    use std::io::Cursor;

    fn oku(metin: &str, kimlik: u64) -> MCPSonuc<Value> {
        olay_bul(Cursor::new(metin.to_string()), kimlik)
    }

    #[test]
    fn tek_olay_okunur() {
        let akis = "data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\n\n";
        let v = oku(akis, 1).expect("olay");
        assert_eq!(v["result"]["ok"], serde_json::json!(true));
    }

    #[test]
    fn heartbeat_ve_alakasiz_alanlar_atlanir() {
        let akis = ": ping\nevent: message\nid: 7\nretry: 100\n\
                    data: {\"jsonrpc\":\"2.0\",\"id\":3,\"result\":1}\n\n";
        assert_eq!(oku(akis, 3).expect("olay")["result"], serde_json::json!(1));
    }

    #[test]
    fn bizim_olmayan_olaylar_atlanir_okuma_surer() {
        // Sunucu once ilerleme bildirimi, sonra baska bir id, en son bizimkini
        // yolluyor. Ilk olayda donsek uzun islerde hep "bicimsiz" alirdik.
        let akis = "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\n\
                    data: {\"jsonrpc\":\"2.0\",\"id\":99,\"result\":\"baskasinin\"}\n\n\
                    data: {\"jsonrpc\":\"2.0\",\"id\":5,\"result\":\"benim\"}\n\n";
        assert_eq!(oku(akis, 5).expect("olay")["result"], serde_json::json!("benim"));
    }

    #[test]
    fn cok_satirli_data_birlestirilir() {
        let akis = "data: {\"jsonrpc\":\"2.0\",\n\
                    data: \"id\":2,\n\
                    data: \"result\":{\"a\":1}}\n\n";
        assert_eq!(oku(akis, 2).expect("olay")["result"]["a"], serde_json::json!(1));
    }

    #[test]
    fn bos_satirla_kapanmamis_son_olay_yine_okunur() {
        let akis = "data: {\"jsonrpc\":\"2.0\",\"id\":4,\"result\":\"son\"}";
        assert_eq!(oku(akis, 4).expect("olay")["result"], serde_json::json!("son"));
    }

    #[test]
    fn crlf_ile_gonderen_sunucu_calisir() {
        let akis = "data: {\"jsonrpc\":\"2.0\",\"id\":8,\"result\":\"crlf\"}\r\n\r\n";
        assert_eq!(oku(akis, 8).expect("olay")["result"], serde_json::json!("crlf"));
    }

    #[test]
    fn batch_dizi_icinden_bizim_id_secilir() {
        let akis = "data: [{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"a\"},\
                    {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":\"b\"}]\n\n";
        assert_eq!(oku(akis, 2).expect("olay")["result"], serde_json::json!("b"));
    }

    #[test]
    fn bizim_id_hic_gelmezse_bicimsiz() {
        let akis = "data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"a\"}\n\n";
        assert_eq!(oku(akis, 42).unwrap_err(), MCPHatasi::Bicimsiz);
    }

    #[test]
    fn bos_akis_bicimsiz() {
        assert_eq!(oku("", 1).unwrap_err(), MCPHatasi::Bicimsiz);
    }

    #[test]
    fn bozuk_json_akisi_panik_yapmaz() {
        let akis = "data: {bu json degil\n\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":9}\n\n";
        assert_eq!(oku(akis, 1).expect("olay")["result"], serde_json::json!(9));
    }
}
