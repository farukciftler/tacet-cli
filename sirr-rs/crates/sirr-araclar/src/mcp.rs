//! MCP koprusu: uzak bir MCP aracini yerel bir `Arac` gibi gosterir.
//!
//! SWIFT'IN DERSI: orada `MCPAraci` yazildi, derlendi, testleri gecti ve HIC
//! ORNEKLENMEDI — mekanizma kuruldu, kimse baglamadi. Ayni hataya dusmemek
//! icin burada koprunun yaninda BAGLAMA fonksiyonlari da var
//! (`katalogu_besle`, `yurutucuyu_baglar`) ve ikisi de gercekten test ediliyor:
//! "kurdum" degil, "katalogda gorunuyor ve onay kapisini tetikliyor".
//!
//! ## Bu dosya AGA CIKMAZ
//!
//! Soket `sirr-mcp`de. Burada olan sey cevirmenlik: `ArgSema`ya cevrilmis bir
//! sema, `AracSonucu`na cevrilmis bir yanit, `AracHatasi`na cevrilmis bir ag
//! hatasi. Ag tekeli bu ayrimla korunur.
//!
//! ## Guvenlik modeli (spec §3.3, §5.4) — burada NE VAR, NE YOK
//!
//! B-izinli modelin uc savunmasi var:
//!
//! - **(a) profil ayrimi:** kisisel veri araclari MCP ile ayni profile GIRMEZ.
//!   Bunu `Yonlendirici` kurar; buradaki katkisi `MCPAraci::kirletir_mi()`in
//!   `false` donmesidir — MCP araci kisisel veri KAYNAGI degil, CIKISIDIR.
//! - **(b) kirli oturumda onay kapisi:** `AracYurutucu`da. Bu dosya araclarin
//!   adlarini o kapinin `dis_araclar` listesine BAGLAR (`yurutucuyu_baglar`).
//!   Baglanmazsa kapi girdisiz kalir — birinci turun tam olarak yaptigi hata.
//! - **(c) onayda gercek icerigin gosterilmesi:** `AracYurutucu` onay istegine
//!   `cagri.args`i birebir koyuyor. Buraya dusen is yok, ama bozulmadigini
//!   test ediyoruz.
//!
//! "Her zaman izin ver" (spec §3.6) v1'de BILEREK YOK: `OnayKapisi`nin
//! uygulamasi ne derse o olur, burada kapiyi atlayan bir yol acilmadi.
//!
//! ## Bloklama
//!
//! `sirr-mcp` bloklayan bir istemci (gerekcesi orada). `calistir` govdesi
//! `async` ama icinde bekleyen bir soket var; bu is agacinda calistirici
//! olmadigi ve `sirr-motor::bekle` gelecekleri tek is parcaciginda surdugu icin
//! davranis dogru. Bedeli acikca soylenmeli: 120 saniyelik bir MCP cagrisi o
//! is parcacigini o sure boyunca mesgul eder.

use serde_json::Value;
use sirr_cekirdek::{
    Arac, AracBaglami, AracHatasi, AracKatalogu, AracSonucu, ArgSema, IzGuncelleme, kutula,
};
use sirr_mcp::{
    BaglantiAyari, MCPHatasi, MCPIstemcisi, Yapilandirma, aciklamayi_kirp, semayi_cevir,
};
use std::sync::Arc;

/// Modele donen ham cikti tavani (karakter).
///
/// Bunun ustundeki cikti `VeriDeposu`na gider, modele ozet + `kaynak_ref`
/// (§5.5, 4096 bypass kanali). Sayi ~200 belirtece denk gelecek sekilde
/// secildi: bir dosya listesi ya da kisa bir komut ciktisi olduğu gibi gecsin,
/// on sayfalik build logu GECMESIN.
pub const KISA_CIKTI_SINIRI: usize = 800;

/// Uzun ciktida modele giden kuyruk (satir).
///
/// NEDEN KUYRUK, BAS DEGIL: komut/log turu ciktida hata SONDA yasar. Bası
/// gostermek modele "her sey yolunda" izlenimi verirdi.
pub const KUYRUK_SATIR: usize = 30;

// ---------------------------------------------------------------------------
// Kopru araci
// ---------------------------------------------------------------------------

/// Tek bir uzak MCP aracini temsil eden yerel arac.
pub struct MCPAraci {
    /// Katalogdaki ad. `<baglanti>_<uzak_ad>` — bkz. `arac_adi_uret`.
    ad: String,
    /// Sunucudaki gercek ad; `tools/call` bunu ister.
    uzak_ad: String,
    /// Cip metninin basinda gorunen kullanici dostu baglanti adi. Kullanici
    /// "bu is cihaz disinda oldu"yu cipten okur (§3.2).
    baglanti_adi: String,
    aciklama: String,
    sema: ArgSema,
    istemci: Arc<MCPIstemcisi>,
}

impl MCPAraci {
    pub fn yeni(
        ad: impl Into<String>,
        uzak_ad: impl Into<String>,
        baglanti_adi: impl Into<String>,
        aciklama: impl Into<String>,
        sema: ArgSema,
        istemci: Arc<MCPIstemcisi>,
    ) -> Self {
        Self {
            ad: ad.into(),
            uzak_ad: uzak_ad.into(),
            baglanti_adi: baglanti_adi.into(),
            aciklama: aciklama.into(),
            sema,
            istemci,
        }
    }

    pub fn uzak_ad(&self) -> &str {
        &self.uzak_ad
    }

    pub fn baglanti_adi(&self) -> &str {
        &self.baglanti_adi
    }
}

impl Arac for MCPAraci {
    fn ad(&self) -> &str {
        &self.ad
    }

    fn aciklama(&self) -> &str {
        &self.aciklama
    }

    fn sema(&self) -> ArgSema {
        self.sema.clone()
    }

    /// MCP araci oturumu KIRLETMEZ.
    ///
    /// Kirlilik "cihazdan kisisel veri OKUNDU" demektir; MCP araci veriyi
    /// okumaz, DISARI CIKARIR. Burada `true` donmek kapiyi kendi uzerine
    /// kapatirdi: ilk MCP cagrisi oturumu kirletir, ikincisi onay isterdi —
    /// oysa arada cihazdan hicbir sey okunmamistir. Onay yorgunlugu boyle
    /// baslar ve §2.4'un "nadir olan onay okunur" ilkesini oldururdu.
    fn kirletir_mi(&self) -> bool {
        false
    }

    fn calistir<'a>(&'a self, args: Value, ctx: &'a mut AracBaglami) -> AracGelecegi<'a> {
        kutula(async move {
            let iz = ctx.cip_baslat("baglanti", &format!("{} · çalışıyor…", self.baglanti_adi));
            ctx.cip_guncelle(
                iz,
                IzGuncelleme::default()
                    // Seffafligin birinci kati: giden icerik cipte durur.
                    // §3.6'daki "her zaman izin ver" modu bile bunu dusurmez.
                    .ham_girdi(serde_json::to_string_pretty(&args).unwrap_or_default()),
            );

            let sonuc = self.istemci.arac_cagir(&self.uzak_ad, &args);

            match sonuc {
                Err(hata) => {
                    // Cift kanal: cipe Turkce cumle, modele SABIT Ingilizce metin.
                    // Ceviri `AracSonucu::basarisiz`in tek gecis noktasindan
                    // gecer; buradan ham `ureq`/sunucu metni SIZMAZ.
                    let cevrilmis = hatayi_cevir(&hata);
                    let cip = format!("{} · {}", self.baglanti_adi, hata.kisa_hata());
                    ctx.cip_guncelle(
                        iz,
                        IzGuncelleme::durum(sirr_cekirdek::AracDurumu::Basarisiz(cip.clone()))
                            .metin(cip.clone()),
                    );
                    let mut s = AracSonucu::basarisiz(&cevrilmis);
                    s.cip_metni = cip;
                    s
                }
                Ok((metin, sunucu_hatasi)) => {
                    let sonuc = self.ciktiyi_isle(ctx, metin, sunucu_hatasi);
                    ctx.cip_guncelle(
                        iz,
                        IzGuncelleme::durum(sonuc.durum.clone())
                            .metin(sonuc.cip_metni.clone())
                            .ham_cikti(sonuc.ham_cikti.clone().unwrap_or_default()),
                    );
                    sonuc
                }
            }
        })
    }
}

impl MCPAraci {
    /// §5.5 — MCP ciktisi ASLA ham haliyle baglama girmez.
    fn ciktiyi_isle(&self, ctx: &AracBaglami, metin: String, sunucu_hatasi: bool) -> AracSonucu {
        let etiket = if sunucu_hatasi { "başarısız" } else { "tamam" };
        let cip = format!("{} · {} {}", self.baglanti_adi, self.uzak_ad, etiket);

        if metin.trim().is_empty() {
            // Bos cikti "hicbir sey olmadi" demek DEGIL (bir komut sessizce
            // basarili olmus olabilir); modele bunu acikca soyleriz, yoksa
            // arac cagrisini tekrarlamaya kalkar.
            return AracSonucu::okundu(cip, "tool returned no output").ham_cikti(metin);
        }

        // Kisa cikti oldugu gibi gecer: bypass kanalini gereksiz kullanmak
        // modele bir tur daha attirir, kucuk modelde bu tur bosa gider.
        if metin.chars().count() <= KISA_CIKTI_SINIRI {
            let modele = if sunucu_hatasi {
                // SUNUCUNUN arac hatasi bir ariza degil, aracin sonucudur;
                // model okuyup kullaniciya anlatabilmeli. Bu yuzden
                // `HATA_MODEL_METNI` KULLANILMAZ.
                format!("tool_error: {metin}")
            } else {
                metin.clone()
            };
            return AracSonucu::okundu(cip, modele).ham_cikti(metin);
        }

        // Uzun cikti: tamami depoya, modele son satirlar + kaynak_ref.
        let satirlar: Vec<&str> = metin.lines().collect();
        let kuyruk = satirlar[satirlar.len().saturating_sub(KUYRUK_SATIR)..].join("\n");
        let ozet = format!(
            "{} lines of output; last {} lines:\n{}",
            satirlar.len(),
            satirlar.len().min(KUYRUK_SATIR),
            kirp(&kuyruk, KISA_CIKTI_SINIRI),
        );
        let kaynak_ref = ctx.depola(
            "mcp",
            &format!("{} satir çıktı", satirlar.len()),
            metin.clone(),
        );
        AracSonucu::ozetle(cip, ozet, kaynak_ref.as_str()).ham_cikti(metin)
    }
}

/// Uzun metni karakter siniriyla kirpar (UTF-8 sinirlarina saygili).
fn kirp(metin: &str, sinir: usize) -> String {
    if metin.chars().count() <= sinir {
        return metin.to_string();
    }
    metin.chars().take(sinir).collect::<String>() + "…"
}

/// `MCPHatasi` -> `AracHatasi`.
///
/// Modele giden metin her halukarda `HATA_MODEL_METNI` olacak (cekirdek
/// karari); ayrim yalniz KULLANICIYA gorunen cumleyi belirler.
fn hatayi_cevir(hata: &MCPHatasi) -> AracHatasi {
    match hata {
        MCPHatasi::ZamanAsimi => AracHatasi::ZamanAsimi,
        diger => AracHatasi::Diger(diger.kisa_hata()),
    }
}

// ---------------------------------------------------------------------------
// Ad uretimi
// ---------------------------------------------------------------------------

/// Katalog adi: `<baglanti>_<uzak_ad>`, ASCII snake_case.
///
/// NEDEN ONEK: iki sunucu da `calistir` adinda arac sunabilir. Onek olmadan
/// ikincisi birincisini golgeler ve model hangi sunucuya gittigini bilemez;
/// ustelik onay kapisinin ret onbellegi `kaynak` olarak arac adini kullaniyor,
/// yani bir sunucuya verilen ret otekini de susturacakti.
///
/// NEDEN SADELESTIRME: model adi gramerde birebir uretmek zorunda ve
/// `AracCagrisi::ayristir` yalniz `[A-Za-z0-9_]` kabul ediyor. "ev sunucusu"
/// gibi bosluklu bir baglanti adi cagrilamaz bir arac uretirdi.
pub fn arac_adi_uret(baglanti_adi: &str, uzak_ad: &str) -> String {
    format!("{}_{}", sadelestir_ad(baglanti_adi), sadelestir_ad(uzak_ad))
}

fn sadelestir_ad(ham: &str) -> String {
    let mut cikti = String::new();
    let mut alt_cizgi_bekliyor = false;
    for c in ham.chars() {
        if c.is_ascii_alphanumeric() {
            if alt_cizgi_bekliyor && !cikti.is_empty() {
                cikti.push('_');
            }
            alt_cizgi_bekliyor = false;
            cikti.extend(c.to_lowercase());
        } else {
            // Turkce harfler ve bosluk dahil ASCII disi her sey ayirac sayilir;
            // ardisik ayiraclar tek alt cizgiye iner.
            alt_cizgi_bekliyor = true;
        }
    }
    cikti
}

// ---------------------------------------------------------------------------
// Yukleme
// ---------------------------------------------------------------------------

/// Ice aktarilamayan arac — SESSIZCE YUTULMAZ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtlananArac {
    pub baglanti: String,
    pub uzak_ad: String,
    pub sebep: String,
}

/// Bir baglantinin yukleme sonucu.
#[derive(Default)]
pub struct YuklemeSonucu {
    pub araclar: Vec<Arc<dyn Arac>>,
    pub atlananlar: Vec<AtlananArac>,
    /// Baglanti kurulamadiysa nedeni (kullaniciya gosterilir).
    pub baglanti_hatalari: Vec<(String, String)>,
    /// Cevrilirken dusurulen kisitlar: "arac: alan: `pattern` zorlanmiyor".
    pub notlar: Vec<String>,
}

impl YuklemeSonucu {
    /// Katalogda gorunecek adlar. `AracYurutucu::dis_arac` girdisi tam da budur.
    pub fn adlar(&self) -> Vec<String> {
        self.araclar.iter().map(|a| a.ad().to_string()).collect()
    }

    fn birlestir(&mut self, diger: YuklemeSonucu) {
        self.araclar.extend(diger.araclar);
        self.atlananlar.extend(diger.atlananlar);
        self.baglanti_hatalari.extend(diger.baglanti_hatalari);
        self.notlar.extend(diger.notlar);
    }
}

/// Tek baglanti: `initialize` + `tools/list` + sema koprusu. **AGA CIKAR.**
pub fn baglantiyi_yukle(ayar: &BaglantiAyari) -> YuklemeSonucu {
    let mut sonuc = YuklemeSonucu::default();

    let istemci = match MCPIstemcisi::yeni(ayar.url.clone(), ayar.cozulmus_anahtar()) {
        Ok(i) => Arc::new(i),
        Err(e) => {
            sonuc.baglanti_hatalari.push((ayar.ad.clone(), e.kisa_hata()));
            return sonuc;
        }
    };

    let tanimlar = match istemci.araclar() {
        Ok(t) => t,
        Err(e) => {
            sonuc.baglanti_hatalari.push((ayar.ad.clone(), e.kisa_hata()));
            return sonuc;
        }
    };

    for tanim in tanimlar {
        match semayi_cevir(&tanim.sema) {
            Ok(cevrim) => {
                for not in &cevrim.notlar.0 {
                    sonuc.notlar.push(format!("{}: {not}", tanim.ad));
                }
                sonuc.araclar.push(Arc::new(MCPAraci::yeni(
                    arac_adi_uret(&ayar.ad, &tanim.ad),
                    tanim.ad.clone(),
                    ayar.ad.clone(),
                    aciklamayi_kirp(&tanim.aciklama),
                    cevrim.sema,
                    Arc::clone(&istemci),
                )) as Arc<dyn Arac>);
            }
            // CEVRILEMEYENI SESSIZCE KABUL ETMEK YASAK: yanlis daraltilmis bir
            // sema, gramerin modeli sunucunun reddedecegi bir sekle zorlamasi
            // demektir ve bunu kimse goremez. Arac atlanir, sebep kayda gecer.
            Err(sebep) => sonuc.atlananlar.push(AtlananArac {
                baglanti: ayar.ad.clone(),
                uzak_ad: tanim.ad,
                sebep: sebep.kisa(),
            }),
        }
    }

    sonuc
}

/// Yapilandirmadaki tum gecerli baglantilar. **AGA CIKAR.**
///
/// Baglanti yoksa bos sonuc doner — HATA DEGIL (§2.1 "varsayilan kapali").
pub fn yapilandirmadan_yukle(y: &Yapilandirma) -> YuklemeSonucu {
    let mut toplam = YuklemeSonucu::default();
    for ayar in y.gecerliler() {
        toplam.birlestir(baglantiyi_yukle(ayar));
    }
    toplam
}

/// `~/.sirr/mcp.json`dan yukler. Dosya yoksa sessizce bos. **AGA CIKAR.**
pub fn varsayilandan_yukle() -> YuklemeSonucu {
    let mut sonuc = YuklemeSonucu::default();
    match sirr_mcp::yapilandirma::varsayilandan_oku() {
        Ok(y) => sonuc.birlestir(yapilandirmadan_yukle(&y)),
        Err(e) => sonuc
            .baglanti_hatalari
            .push(("yapılandırma".into(), e.kisa_hata())),
    }
    sonuc
}

/// Yuklenen araclari kataloga ekler ve adlarini doner.
///
/// Donen adlar `yurutucuyu_baglar`a verilmeli — YOKSA onay kapisi girdisiz
/// kalir ve dosyanin basindaki Swift hatasi tekrarlanir.
#[must_use = "donen adlar AracYurutucu'nun dis_araclar listesine baglanmali"]
pub fn katalogu_besle(katalog: &mut AracKatalogu, sonuc: &YuklemeSonucu) -> Vec<String> {
    for arac in &sonuc.araclar {
        katalog.ekle(Arc::clone(arac));
    }
    sonuc.adlar()
}

/// MCP araclarini onay kapisinin `dis_araclar` listesine baglar.
///
/// `AracYurutucu::dis_arac` `self`i tuketen bir kurucu oldugu icin dongu
/// disaridan yazilmak zorunda; ayni dongu her cagri yerinde tekrarlanmasin
/// diye burada bir kez duruyor.
pub fn yurutucuyu_baglar(
    mut yurutucu: crate::yurutucu::AracYurutucu,
    adlar: &[String],
) -> crate::yurutucu::AracYurutucu {
    for ad in adlar {
        yurutucu = yurutucu.dis_arac(ad.clone());
    }
    yurutucu
}

use sirr_cekirdek::AracGelecegi;

#[cfg(test)]
mod testler {
    use super::*;
    use crate::yurutucu::{
        AracCagrisi, AracYurutucu, DaimaOnay, RET_MODEL_METNI, SessizRet, YurutmeSebebi,
    };
    use serde_json::json;
    use sirr_cekirdek::{
        Alan, AracDurumu, BellekVeriDeposu, HATA_MODEL_METNI, SessizRaporlayici, kutula,
    };
    use sirr_motor::bekle;

    fn baglam() -> AracBaglami {
        AracBaglami::yeni(
            Arc::new(BellekVeriDeposu::yeni()),
            std::env::temp_dir(),
            Arc::new(SessizRaporlayici),
        )
    }

    /// Ag'a cikmadan MCP aracinin YERINI alan sahte: `Arac` sozlesmesini
    /// gercek `MCPAraci` ile ayni sekilde doldurur (ayni ad uretimi, ayni
    /// `kirletir_mi`), boylece onay kapisi testleri gercek yolu olcer.
    struct SahteMCPAraci {
        ad: String,
    }

    impl Arac for SahteMCPAraci {
        fn ad(&self) -> &str {
            &self.ad
        }
        fn aciklama(&self) -> &str {
            "Uzak sunucuda komut calistirir."
        }
        fn sema(&self) -> ArgSema {
            ArgSema::nesne(vec![Alan::yeni("komut", ArgSema::metin()).zorunlu()])
        }
        fn kirletir_mi(&self) -> bool {
            false
        }
        fn calistir<'a>(&'a self, _a: Value, _c: &'a mut AracBaglami) -> AracGelecegi<'a> {
            kutula(async move { AracSonucu::okundu("gonderildi", "ok") })
        }
    }

    /// Kisisel veri araci — oturumu kirletir.
    struct KisiselArac;
    impl Arac for KisiselArac {
        fn ad(&self) -> &str {
            "takvim_oku"
        }
        fn aciklama(&self) -> &str {
            "Takvimi okur."
        }
        fn sema(&self) -> ArgSema {
            ArgSema::bos()
        }
        fn kirletir_mi(&self) -> bool {
            true
        }
        fn calistir<'a>(&'a self, _a: Value, _c: &'a mut AracBaglami) -> AracGelecegi<'a> {
            kutula(async move { AracSonucu::okundu("okundu", "3 etkinlik") })
        }
    }

    fn mcp_katalogu() -> (AracKatalogu, Vec<String>) {
        let ad = arac_adi_uret("ev sunucusu", "run_command");
        let mut katalog = AracKatalogu::yeni();
        katalog.ekle(Arc::new(KisiselArac));
        katalog.ekle(Arc::new(SahteMCPAraci { ad: ad.clone() }));
        (katalog, vec![ad])
    }

    // --- Ad uretimi ---

    #[test]
    fn arac_adi_cagrilabilir_bicimde_uretilir() {
        let ad = arac_adi_uret("ev sunucusu", "run-command");
        assert_eq!(ad, "ev_sunucusu_run_command");
        // Gramer ve `AracCagrisi::ayristir` yalniz bu kumeyi kabul ediyor.
        assert!(ad.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "{ad}");
    }

    #[test]
    fn turkce_ve_ardisik_ayirac_tek_alt_cizgiye_iner() {
        assert_eq!(arac_adi_uret("Çalışma  Ağı", "get::info"), "al_ma_a_get_info");
    }

    #[test]
    fn ayni_arac_adi_iki_sunucuda_carpismaz() {
        assert_ne!(
            arac_adi_uret("ev", "calistir"),
            arac_adi_uret("is", "calistir"),
        );
    }

    // --- Onay kapisi: mekanizma GERCEKTEN tetikleniyor mu ---

    #[test]
    fn temiz_oturumda_mcp_cagrisi_sorulmadan_gecer() {
        // §3.3: kisisel veri araclarina dokunulmamis oturumda onay SORULMAZ.
        // Sik gorunen onay okunmaz hale gelir; kapinin degeri nadirliginda.
        let (katalog, adlar) = mcp_katalogu();
        let yurutucu = yurutucuyu_baglar(AracYurutucu::yeni(katalog), &adlar).kapiyla(SessizRet);
        let mut ctx = baglam();
        let bilet = yurutucu.aktif_tur();

        let sonuc = bekle(yurutucu.yurut(
            &AracCagrisi::yeni(&adlar[0], json!({"komut": "git pull"})),
            bilet,
            &mut ctx,
        ));
        assert_eq!(sonuc.sebep, YurutmeSebebi::Tamam, "temiz oturumda onay sorulmamali");
    }

    #[test]
    fn kirli_oturumda_mcp_cagrisi_onay_kapisina_duser() {
        // Birinci turun eksigi buydu: kapi vardi, `DIS_ARACLAR` bostu, yani
        // tetiklenemiyordu. Bu test tam olarak o baglantiyi olcer.
        let (katalog, adlar) = mcp_katalogu();
        let yurutucu = yurutucuyu_baglar(AracYurutucu::yeni(katalog), &adlar).kapiyla(SessizRet);
        let mut ctx = baglam();
        let bilet = yurutucu.aktif_tur();

        // Once kisisel veri okunur -> oturum kirlenir.
        let ilk = bekle(yurutucu.yurut(&AracCagrisi::yeni("takvim_oku", json!({})), bilet, &mut ctx));
        assert_eq!(ilk.sebep, YurutmeSebebi::Tamam);
        assert!(yurutucu.oturum_kirli(), "kisisel arac oturumu kirletmeliydi");

        // Simdi MCP cagrisi kapiya takilmali.
        let sonuc = bekle(yurutucu.yurut(
            &AracCagrisi::yeni(&adlar[0], json!({"komut": "toplantiyi gonder"})),
            bilet,
            &mut ctx,
        ));
        assert_eq!(sonuc.sebep, YurutmeSebebi::OnayReddedildi);
        assert_eq!(sonuc.durum, AracDurumu::IzinGerekli);
        // Ret modele NORMAL bir sonuc olarak doner, ariza olarak degil (§3.3).
        assert_eq!(sonuc.modele_donen, RET_MODEL_METNI);
        assert!(!sonuc.hata_mi(), "ret bir ariza degil, kullanicinin karari");
    }

    #[test]
    fn onaylanan_kirli_oturumda_cagri_gecer() {
        let (katalog, adlar) = mcp_katalogu();
        let yurutucu = yurutucuyu_baglar(AracYurutucu::yeni(katalog), &adlar).kapiyla(DaimaOnay);
        let mut ctx = baglam();
        let bilet = yurutucu.aktif_tur();

        bekle(yurutucu.yurut(&AracCagrisi::yeni("takvim_oku", json!({})), bilet, &mut ctx));
        let sonuc = bekle(yurutucu.yurut(
            &AracCagrisi::yeni(&adlar[0], json!({"komut": "gonder"})),
            bilet,
            &mut ctx,
        ));
        assert_eq!(sonuc.sebep, YurutmeSebebi::Tamam);
    }

    #[test]
    fn baglanmamis_mcp_araci_kapiyi_hic_tetiklemez() {
        // Bu testin varlik sebebi: "baglamayi unutmak" sessiz bir guvenlik
        // kaybidir ve derleme onu yakalamaz. Burada gorunur oluyor.
        let (katalog, adlar) = mcp_katalogu();
        let yurutucu = AracYurutucu::yeni(katalog).kapiyla(SessizRet); // BAGLANMADI
        let mut ctx = baglam();
        let bilet = yurutucu.aktif_tur();

        bekle(yurutucu.yurut(&AracCagrisi::yeni("takvim_oku", json!({})), bilet, &mut ctx));
        let sonuc = bekle(yurutucu.yurut(
            &AracCagrisi::yeni(&adlar[0], json!({"komut": "gonder"})),
            bilet,
            &mut ctx,
        ));
        assert_eq!(
            sonuc.sebep,
            YurutmeSebebi::Tamam,
            "baglanmadiginda veri sorulmadan cikiyor — `yurutucuyu_baglar` sart"
        );
    }

    /// Sorulan her onayi kaydeden kapi: hem KAC KEZ soruldugunu hem NE
    /// gosterildigini olcer. Daima reddeder.
    struct KayitKapisi(std::sync::Mutex<Vec<String>>);

    impl crate::yurutucu::OnayKapisi for Arc<KayitKapisi> {
        fn iste(&self, istek: &crate::yurutucu::OnayIstegi) -> bool {
            self.0.lock().expect("kayit kilidi").push(istek.icerik.clone());
            false
        }
    }

    fn kayit_kapisi() -> Arc<KayitKapisi> {
        Arc::new(KayitKapisi(std::sync::Mutex::new(Vec::new())))
    }

    #[test]
    fn ayni_oturumda_ikinci_onay_sorulmaz() {
        let (katalog, adlar) = mcp_katalogu();
        let kapi = kayit_kapisi();
        let yurutucu = yurutucuyu_baglar(AracYurutucu::yeni(katalog), &adlar)
            .kapiyla(Arc::clone(&kapi));
        let mut ctx = baglam();
        let bilet = yurutucu.aktif_tur();
        bekle(yurutucu.yurut(&AracCagrisi::yeni("takvim_oku", json!({})), bilet, &mut ctx));

        for _ in 0..3 {
            let s = bekle(yurutucu.yurut(
                &AracCagrisi::yeni(&adlar[0], json!({"komut": "gonder"})),
                bilet,
                &mut ctx,
            ));
            assert_eq!(s.sebep, YurutmeSebebi::OnayReddedildi);
        }
        // §3.3: "AracYurutucu ayni oturumda ikinci onay cipi URETMEZ" — model
        // israr dongusune giremez.
        assert_eq!(kapi.0.lock().expect("kayit kilidi").len(), 1);
    }

    #[test]
    fn onayda_gosterilen_icerik_gercek_payloaddir() {
        let (katalog, adlar) = mcp_katalogu();
        let kapi = kayit_kapisi();
        let yurutucu = yurutucuyu_baglar(AracYurutucu::yeni(katalog), &adlar)
            .kapiyla(Arc::clone(&kapi));
        let mut ctx = baglam();
        let bilet = yurutucu.aktif_tur();
        bekle(yurutucu.yurut(&AracCagrisi::yeni("takvim_oku", json!({})), bilet, &mut ctx));
        bekle(yurutucu.yurut(
            &AracCagrisi::yeni(&adlar[0], json!({"komut": "yarin 14:00 toplanti"})),
            bilet,
            &mut ctx,
        ));

        let gorulen = kapi.0.lock().expect("kayit kilidi").clone();
        assert_eq!(gorulen.len(), 1);
        // §3.3: govde "kategori ozeti degil, gercek icerik".
        assert!(gorulen[0].contains("yarin 14:00 toplanti"), "{}", gorulen[0]);
    }

    // --- Katalog baglanmasi ---

    #[test]
    fn katalogu_besle_araclari_gercekten_gorunur_yapar() {
        let mut sonuc = YuklemeSonucu::default();
        sonuc.araclar.push(Arc::new(SahteMCPAraci {
            ad: arac_adi_uret("ev", "calistir"),
        }));

        let mut katalog = AracKatalogu::yeni();
        let adlar = katalogu_besle(&mut katalog, &sonuc);

        assert_eq!(adlar, vec!["ev_calistir".to_string()]);
        assert!(katalog.bul("ev_calistir").is_some(), "katalogda GORUNMELI");
        // MCP araci kisisel veri KAYNAGI degil: kirletenler listesine girmemeli.
        assert!(katalog.kirletenler().is_empty());
    }

    #[test]
    fn baglanti_yoksa_yukleme_bos_ve_hatasiz() {
        // §2.1 varsayilan kapali: baglanti yoksa ag'a hic cikilmaz, hata da yok.
        let sonuc = yapilandirmadan_yukle(&Yapilandirma::default());
        assert!(sonuc.araclar.is_empty());
        assert!(sonuc.baglanti_hatalari.is_empty());
        assert!(sonuc.atlananlar.is_empty());
    }

    #[test]
    fn gecersiz_adres_aga_cikmadan_hata_olarak_kayda_gecer() {
        let ayar = BaglantiAyari {
            ad: "kotu".into(),
            url: "http://uzak-sunucu.example.com/mcp".into(),
            anahtar: None,
            anahtar_ortam: None,
            etkin: true,
        };
        let sonuc = baglantiyi_yukle(&ayar);
        assert!(sonuc.araclar.is_empty());
        assert_eq!(sonuc.baglanti_hatalari.len(), 1);
        assert_eq!(sonuc.baglanti_hatalari[0].0, "kotu");
    }

    // --- Cikti isleme (4096 bypass) ---

    fn arac_ornegi() -> MCPAraci {
        MCPAraci::yeni(
            "ev_calistir",
            "calistir",
            "ev sunucusu",
            "Komut calistirir.",
            ArgSema::bos(),
            Arc::new(MCPIstemcisi::yeni("https://ornek.com/mcp", None).expect("istemci")),
        )
    }

    #[test]
    fn kisa_cikti_oldugu_gibi_gecer() {
        let ctx = baglam();
        let sonuc = arac_ornegi().ciktiyi_isle(&ctx, "iki dosya guncellendi".into(), false);
        assert_eq!(sonuc.modele_donen, "iki dosya guncellendi");
        assert!(sonuc.cip_metni.starts_with("ev sunucusu · "));
    }

    #[test]
    fn uzun_cikti_modele_dokulmez_depoya_gider() {
        let ctx = baglam();
        // Sadece sonda gecen bir isaret koyalim: kuyruk kurali kanitlansin.
        let mut satirlar: Vec<String> = (0..500).map(|i| format!("satir {i} dolgu dolgu")).collect();
        satirlar.push("HATA: derleme basarisiz".into());
        let ham = satirlar.join("\n");

        let sonuc = arac_ornegi().ciktiyi_isle(&ctx, ham.clone(), false);

        // Toplu veri modele GECMEMELI.
        assert!(!sonuc.modele_donen.contains("satir 10 dolgu"), "{}", sonuc.modele_donen);
        assert!(sonuc.modele_donen.len() < ham.len() / 4);
        // Ama hata kuyrukta yasar (§5.5).
        assert!(sonuc.modele_donen.contains("HATA: derleme basarisiz"));
        // Tel bicimi tek: `kaynak_ref_eki`.
        assert!(sonuc.modele_donen.contains("kaynak_ref=mcp#1"), "{}", sonuc.modele_donen);
        // Tamami depoda; cip detayinda da ham hali duruyor (seffaflik).
        let kayit = ctx
            .depodan(&sirr_cekirdek::KaynakRef("mcp#1".into()))
            .expect("depoda olmali");
        assert_eq!(kayit.govde, ham);
        assert_eq!(sonuc.ham_cikti.as_deref(), Some(ham.as_str()));
    }

    #[test]
    fn sunucunun_arac_hatasi_modele_anlatilir_ariza_gibi_gizlenmez() {
        let ctx = baglam();
        let sonuc = arac_ornegi().ciktiyi_isle(&ctx, "exit 1: dosya yok".into(), true);
        // `isError` tasima arizasi DEGIL: model okuyup kullaniciya anlatabilmeli.
        assert!(sonuc.modele_donen.contains("dosya yok"));
        assert_ne!(sonuc.modele_donen, HATA_MODEL_METNI);
        assert!(sonuc.cip_metni.contains("başarısız"));
    }

    #[test]
    fn bos_cikti_sessiz_kalmaz() {
        let ctx = baglam();
        let sonuc = arac_ornegi().ciktiyi_isle(&ctx, "   ".into(), false);
        assert_eq!(sonuc.modele_donen, "tool returned no output");
    }

    #[test]
    fn tasima_hatasi_cift_kanaldan_gecer() {
        // Kullaniciya Turkce cumle, modele SABIT Ingilizce metin. Ham `ureq`
        // ya da sunucu metni modele SIZMAMALI.
        let hata = hatayi_cevir(&MCPHatasi::Sunucu("gizli sunucu izi".into()));
        let sonuc = AracSonucu::basarisiz(&hata);
        assert_eq!(sonuc.modele_donen, HATA_MODEL_METNI);
        assert!(!sonuc.modele_donen.contains("gizli"));

        assert!(matches!(
            hatayi_cevir(&MCPHatasi::ZamanAsimi),
            AracHatasi::ZamanAsimi
        ));
    }
}
