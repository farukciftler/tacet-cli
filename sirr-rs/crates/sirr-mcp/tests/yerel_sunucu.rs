//! UCTAN UCA: gercek soket, gercek HTTP, gercek JSON-RPC.
//!
//! NEDEN `#[ignore]` DEGIL: kural "aga cikan test ignore olsun" — bu test
//! AGA CIKMAZ. Sunucu testin kendi icinde, `127.0.0.1`de, rastgele bir portta
//! dogar ve testle birlikte olur; ne DNS, ne internet, ne dis bagimlilik.
//!
//! NEDEN VAR: birim testleri ayristiricilari kanitlar, istemcinin GERCEKTEN
//! konustugunu kanitlamaz. "Derleme yesil" hicbir sey kanitlamaz; bu dosya
//! `initialize -> tools/list -> tools/call` akisinin bir sokette calistigini
//! kanitlar. Iki tasima bicimi de olculur: duz `application/json` ve SSE.

use sirr_mcp::istemci::MCPIstemcisi;
use sirr_mcp::kopru::semayi_cevir;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;

/// Sahte MCP sunucusu. `sse` true ise yanitlari `text/event-stream` olarak
/// dondurur — gercek sunucularin ikisini de yapabildigini yansitir.
fn sunucu_baslat(sse: bool) -> (String, thread::JoinHandle<()>) {
    let dinleyici = TcpListener::bind("127.0.0.1:0").expect("port");
    let port = dinleyici.local_addr().expect("adres").port();
    let (haberci, bekleyen) = mpsc::channel();

    let is = thread::spawn(move || {
        haberci.send(()).ok();
        // Uc istek bekliyoruz: initialize, notifications/initialized, tools/list,
        // tools/call. Bildirim yanit istemedigi icin dordu de ayni dongude.
        for _ in 0..4 {
            let Ok((akis, _)) = dinleyici.accept() else { return };
            if istegi_karsila(akis, sse).is_err() {
                return;
            }
        }
    });

    bekleyen.recv().expect("sunucu basladi");
    (format!("http://127.0.0.1:{port}/mcp"), is)
}

fn istegi_karsila(mut akis: TcpStream, sse: bool) -> std::io::Result<()> {
    let mut okuyucu = BufReader::new(akis.try_clone()?);

    // Basliklari oku, govde uzunlugunu ogren.
    let mut uzunluk = 0usize;
    loop {
        let mut satir = String::new();
        if okuyucu.read_line(&mut satir)? == 0 {
            return Ok(());
        }
        let kirpik = satir.trim();
        if kirpik.is_empty() {
            break;
        }
        if let Some(v) = kirpik.to_ascii_lowercase().strip_prefix("content-length:") {
            uzunluk = v.trim().parse().unwrap_or(0);
        }
    }
    let mut govde = vec![0u8; uzunluk];
    okuyucu.read_exact(&mut govde)?;
    let istek: serde_json::Value = serde_json::from_slice(&govde).unwrap_or(serde_json::Value::Null);

    let metot = istek.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let kimlik = istek.get("id").cloned();

    // Bildirim (id yok): 202 don, govde yok.
    let Some(kimlik) = kimlik else {
        akis.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n")?;
        return Ok(());
    };

    let sonuc = match metot {
        "initialize" => serde_json::json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "sahte", "version": "0.1"},
        }),
        "tools/list" => serde_json::json!({"tools": [
            {
                "name": "calistir",
                "description": "Uzak sunucuda kabuk komutu calistirir ve ciktisini doner. \
                                Bu aciklama bilerek uzun tutuldu; ice aktarmada kirpilmasi \
                                gerektigini gostermek icin defalarca ayni seyi tekrar eder, \
                                tekrar eder, tekrar eder ve hala devam eder.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "komut": {"type": "string", "description": "kabuk komutu"},
                        "kip": {"type": "string", "enum": ["hizli", "guvenli"]}
                    },
                    "required": ["komut"]
                }
            },
            {
                // CEVRILEMEZ: coklu `oneOf`. Sessizce kabul EDILMEMELI.
                "name": "esnek",
                "description": "Her tipi kabul eder.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "hedef": {"oneOf": [{"type": "string"}, {"type": "integer"}]}
                    }
                }
            }
        ]}),
        "tools/call" => serde_json::json!({
            "content": [{"type": "text", "text": "2 dosya guncellendi"}],
            "isError": false,
        }),
        _ => serde_json::json!({}),
    };

    let yanit = serde_json::json!({"jsonrpc": "2.0", "id": kimlik, "result": sonuc}).to_string();

    if sse {
        // Araya bir heartbeat ve bize ait olmayan bir olay serpistirilir:
        // istemci onlari atlayip kendi id'sini bulmak zorunda.
        let govde = format!(
            ": heartbeat\n\ndata: {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}}\n\n\
             data: {yanit}\n\n"
        );
        akis.write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                 Mcp-Session-Id: oturum-42\r\nContent-Length: {}\r\n\r\n{govde}",
                govde.len()
            )
            .as_bytes(),
        )?;
    } else {
        akis.write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                 Mcp-Session-Id: oturum-42\r\nContent-Length: {}\r\n\r\n{yanit}",
                yanit.len()
            )
            .as_bytes(),
        )?;
    }
    akis.flush()
}

/// Tam akis: el sikisma, arac listesi, sema koprusu, arac cagrisi.
fn tam_akis(sse: bool) {
    let (url, is) = sunucu_baslat(sse);
    let istemci = MCPIstemcisi::yeni(url, Some("gizli-token".into())).expect("istemci");

    let araclar = istemci.araclar().expect("tools/list");
    assert_eq!(araclar.len(), 2, "sunucu iki arac bildirdi");

    // 1) Cevrilebilen arac: sema GERCEKTEN kullanilabilir hale gelmeli.
    let calistir = araclar.iter().find(|a| a.ad == "calistir").expect("calistir");
    let cevrim = semayi_cevir(&calistir.sema).expect("cevrilmeli");
    assert!(cevrim.sema.dogrula(&serde_json::json!({"komut": "ls"})).is_ok());
    assert!(
        cevrim.sema.dogrula(&serde_json::json!({"komut": "ls", "kip": "yok"})).is_err(),
        "enum kumesi disi deger gecmemeli"
    );
    assert!(cevrim.sema.dogrula(&serde_json::json!({})).is_err(), "zorunlu alan");

    // 2) Cevrilemeyen arac SESSIZCE KABUL EDILMEZ.
    let esnek = araclar.iter().find(|a| a.ad == "esnek").expect("esnek");
    let sebep = semayi_cevir(&esnek.sema).err().expect("reddedilmeliydi");
    assert_eq!(sebep, sirr_mcp::CevrilemezSebep::BirlesikSema("oneOf".into()));

    // 3) Uzun aciklama baglama HAM girmez.
    let kirpik = sirr_mcp::aciklamayi_kirp(&calistir.aciklama);
    assert!(kirpik.chars().count() < calistir.aciklama.chars().count());
    assert!(kirpik.chars().count() <= sirr_mcp::kopru::ACIKLAMA_SINIRI + 1);

    // 4) Arac cagrisi.
    let (metin, hatali) = istemci
        .arac_cagir("calistir", &serde_json::json!({"komut": "git pull"}))
        .expect("tools/call");
    assert_eq!(metin, "2 dosya guncellendi");
    assert!(!hatali);

    drop(istemci);
    is.join().ok();
}

#[test]
fn duz_json_tasimasiyla_uctan_uca() {
    tam_akis(false);
}

#[test]
fn sse_tasimasiyla_uctan_uca() {
    tam_akis(true);
}
