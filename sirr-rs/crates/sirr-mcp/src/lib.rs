//! sirr-mcp — MCP (Model Context Protocol) istemcisi.
//!
//! **AG TEKELI.** `sirr-web` ile birlikte, ag cagrisi yapmasina izin verilen iki
//! crate'ten biri. Baska hicbir crate soket acmaz; kural denetlenebilir olsun
//! diye HTTP bagimliligi de yalniz bu iki manifestte durur.
//!
//! Elle yazilmis JSON-RPC 2.0 + Streamable HTTP + SSE. Resmi bir MCP SDK'si
//! CEKILMEDI: v1'in ihtiyaci uc metot (`initialize`, `tools/list`,
//! `tools/call`) ve sirr'in kimligi hazir crate cekmemek uzerine kurulu.
//! Istisna TLS: onu elle yazmak sorumsuzluk olurdu (bkz. `Cargo.toml`).
//!
//! ## Vaat
//!
//! > sirr kendiliginden internete cikmaz. Sen bir sunucu baglarsan, oraya ne
//! > gonderildigini her seferinde gorursun.
//!
//! Bu crate o vaadin ILK yarisini tasir: baglanti yoksa hicbir sey olmaz
//! (`yapilandirma` varsayilani bostur). IKINCI yarisi — "gormeden cikmaz" —
//! burada DEGIL, `sirr-araclar::yurutucu`daki deterministik onay kapisindadir.
//! Bu ayrim bilincli: ag katmani kendi kapisini kendi tutsaydi, ag katmanini
//! degistiren biri kapiyi da degistirebilirdi.
//!
//! ## Moduller
//!
//! | Modul | Isi | Aga cikar mi |
//! | --- | --- | --- |
//! | `jsonrpc` | Istek cerceveleme, id eslesmesi, `result`/`error` | hayir |
//! | `sse` | `text/event-stream` ayristirmasi | hayir |
//! | `kopru` | MCP JSON Semasi -> `ArgSema`, aciklama kirpma | hayir |
//! | `yapilandirma` | `~/.sirr/mcp.json` | hayir |
//! | `istemci` | HTTP POST + oturum durumu | **EVET, tek yer** |
//!
//! Ag'a cikan tek modul `istemci`; geri kalan her sey saf ve ag'a cikmadan
//! test edilir. Testlerdeki tek ag testi `#[ignore]`.

pub mod hata;
pub mod istemci;
pub mod jsonrpc;
pub mod kopru;
pub mod sse;
pub mod yapilandirma;

pub use hata::{MCPHatasi, MCPSonuc};
pub use istemci::{AracTanimi, MCPIstemcisi, PROTOKOL_SURUMU, ZAMAN_ASIMI_SN};
pub use kopru::{Cevrim, CevrilemezSebep, CevrimNotlari, aciklamayi_kirp, semayi_cevir};
pub use yapilandirma::{BaglantiAyari, Yapilandirma};
