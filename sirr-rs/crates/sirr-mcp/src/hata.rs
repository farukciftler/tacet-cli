//! MCP hatalari — kullaniciya duz dille ayrisir.
//!
//! NEDEN AYRI VARYANTLAR: `mcp-baglanti-spec §3.1` "baglanti kurulamazsa neden
//! (zaman asimi, yetki, TLS) duz dille yazilir" diyor. Tek bir "ag hatasi"
//! varyanti kullaniciya hicbir sey ogretmez: yanlis anahtar mi girdi, sunucu mu
//! kapali, sertifika mi bozuk — uc ayri eylem gerektirir.
//!
//! Buradaki metinler KULLANICIYA gider. Modele giden metin bu degildir: arac
//! koprusu her hatayi `AracSonucu::basarisiz` uzerinden sabit Ingilizce
//! `tool_failed: ...` metnine cevirir (cift kanal kurali).

/// Tel/protokol duzeyindeki basarisizliklar.
///
/// Sunucunun KENDI arac hatasi (`isError: true`) buraya girmez: o bir ariza
/// degil, aracin normal sonucudur ve modele anlatilir.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MCPHatasi {
    #[error("zaman asimi")]
    ZamanAsimi,

    /// 401/403 — anahtar yok ya da kabul edilmedi.
    #[error("erisim anahtari kabul edilmedi")]
    Yetki,

    #[error("guvenli baglanti kurulamadi")]
    Tls,

    /// Ag yok, sunucu kapali, DNS cozulmedi.
    #[error("sunucuya erisilemedi")]
    Erisilemedi,

    /// JSON-RPC `error` govdesi ya da beklenmeyen HTTP kodu.
    #[error("sunucu hata dondu{}", if .0.is_empty() { String::new() } else { format!(": {}", .0) })]
    Sunucu(String),

    /// Yanit MCP'ye uymuyor (JSON degil, `result` yok, id eslesmedi).
    #[error("sunucunun yaniti anlasilmadi")]
    Bicimsiz,

    /// URL semasi kabul edilmedi (bkz. `istemci::url_dogrula`).
    #[error("adres kabul edilmedi: {0}")]
    GecersizAdres(String),
}

impl MCPHatasi {
    /// Kullaniciya gosterilecek cumle. Dramatize etmez, ne olduğunu soyler.
    pub fn kisa_hata(&self) -> String {
        self.to_string()
    }
}

/// Bu crate'in Result kisayolu.
pub type MCPSonuc<T> = Result<T, MCPHatasi>;

#[cfg(test)]
mod testler {
    use super::*;

    #[test]
    fn sunucu_hatasi_bos_mesajla_iki_nokta_koymaz() {
        assert_eq!(MCPHatasi::Sunucu(String::new()).kisa_hata(), "sunucu hata dondu");
        assert_eq!(
            MCPHatasi::Sunucu("HTTP 500".into()).kisa_hata(),
            "sunucu hata dondu: HTTP 500"
        );
    }

    #[test]
    fn hicbir_hata_metni_ingilizce_model_kanalina_benzemez() {
        // Kullanici kanaliyla model kanali karismasin: buradaki hicbir metin
        // "tool_failed" gibi model sozdizimi tasimamali.
        for h in [
            MCPHatasi::ZamanAsimi,
            MCPHatasi::Yetki,
            MCPHatasi::Tls,
            MCPHatasi::Erisilemedi,
            MCPHatasi::Bicimsiz,
        ] {
            assert!(!h.kisa_hata().contains(':'), "{h:?}");
        }
    }
}
