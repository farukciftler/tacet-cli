//! AracYurutucu — model ciktisi ile `Arac` arasindaki TEK gecis noktasi.
//!
//! Swift'teki `AracYurutucu` cip yasam dongusunu tutuyordu; burada o is
//! cekirdegin `Raporlayici`sine ait. Bu tipe kalan sey daha dar ve daha
//! kritik: bir arac cagrisini KABUL ETME karari.
//!
//! DORT KAPI, HEPSI YAPISAL:
//!
//! 1. AD — katalogta yoksa arac calismaz. Model uydurdugu bir imzayi
//!    calistiramaz.
//! 2. SEMA — argumanlar `ArgSema::dogrula`dan gecmeden arac gormez. Gramer
//!    zaten zorluyor ama gramer devre disi birakilabilir (eval, CLI, dogrudan
//!    cagri); kapinin iki katli olmasi bilincli.
//! 3. ONAY — kirli oturumda dis dunyaya veri cikaracak cagri kullanici
//!    karari olmadan gecmez.
//! 4. IPTAL — kullanici turu iptal ettiyse arac hic baslamaz.
//!
//! HATA TESPITI METINLE YAPILMAZ. Swift tarafinda bir kez sessizce
//! `modele_donen.contains("tool_failed")` yazildi; arac metnini degistiren
//! ilgisiz bir commit kurtarma yolunu olduruyordu ve hicbir test kirmiyordu.
//! Burada karar `YurutmeSonucu::sebep` (kapali enum) uzerinden verilir;
//! `hata_mi()` ve `tekrar_denenebilir` ondan turer, metinden degil.
//!
//! DUNYA DEGISTIYSE RETRY YOK. `AracDurumu::Yazildi` donmus bir turda ayni
//! istemi ikinci kez gondermek ikinci bir dosya/kayit yaratir. Bayrak TUR
//! OMURLU ve YAPISKAN: kurtarma denemesi onu dusuremez, yalniz gercek bir
//! yeni tur sifirlar.

use serde_json::Value;
use sirr_cekirdek::{
    AracDurumu, AracHatasi, AracKatalogu, AracSonucu, HATA_MODEL_METNI, AracBaglami,
};
use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Onay reddedildiginde modele donen SABIT metin.
///
/// Aracin kendi hata metninden ayri: bu bir arıza degil, bir karardir. Model
/// "tekrar dene" refleksine girmesin diye cumle kesin ve nedensiz — gerekce
/// kullanicinindir, modele acilmaz.
pub const RET_MODEL_METNI: &str =
    "permission_denied: the user did not allow this data to leave the device";

/// Iptal edilen turda modele donen sabit metin.
pub const IPTAL_MODEL_METNI: &str = "cancelled: the user stopped this turn";

// ---------------------------------------------------------------------------
// Cagri
// ---------------------------------------------------------------------------

/// Modelin urettigi tek bir arac cagrisi.
#[derive(Debug, Clone, PartialEq)]
pub struct AracCagrisi {
    pub ad: String,
    pub args: Value,
}

impl AracCagrisi {
    pub fn yeni(ad: impl Into<String>, args: Value) -> Self {
        Self { ad: ad.into(), args }
    }

    /// Model ciktisindan cagriyi ayristirir; cagri yoksa `None`.
    ///
    /// IKI BICIM kabul edilir cunku gramer acikken model kanonik JSON'u,
    /// gramer kapaliyken (serbest uretim, eval, elle CLI girdisi) daha sik
    /// `ad({...})` bicimini uretiyor. Ikisini de burada karsilamak, cagri
    /// yerlerinin her birinde ayri bir "acaba su bicim de mi" yamasi
    /// olusmasindan iyidir.
    ///
    /// `None` donmesi HATA DEGILDIR: model duz cevap vermis olabilir. Bu ayrim
    /// cagri yerine birakilir.
    pub fn ayristir(ham: &str) -> Option<Self> {
        let govde = kod_citini_soy(ham.trim());

        // Bicim 1 — kanonik: {"arac":"ad","args":{...}}
        if let Ok(Value::Object(nesne)) = serde_json::from_str::<Value>(govde) {
            let ad = nesne
                .get("arac")
                .or_else(|| nesne.get("ad"))
                .or_else(|| nesne.get("name"))
                .and_then(Value::as_str)?;
            let args = nesne
                .get("args")
                .or_else(|| nesne.get("arguments"))
                .cloned()
                .unwrap_or_else(|| Value::Object(Default::default()));
            return Some(Self::yeni(ad, args));
        }

        // Bicim 2 — cagri bicimi: ad({...})
        //
        // ONEKTE DUZ METIN OLABILIR. Eskiden `(` oncesindeki metnin TAMAMI
        // arac adi sayiliyordu; gercek modelde olculdu ve bu yanlis cikti.
        // Qwen3-4B tipik olarak once niyetini yaziyor:
        //
        //   "Istanbul'un hava durumunu sorgulamak icin web aramasi yapacagim.
        //    web_ara({"sorgu":"istanbul hava durumu"})"
        //
        // Eski kod bu ciktida adi "Istanbul'un ... yapacagim.\n\nweb_ara"
        // olarak okuyor, icinde harf-disi karakter gorup `None` donuyordu —
        // yani model araci DOGRU cagirmis olmasina ragmen arac HIC KOSMUYOR,
        // uretilen cagri sessizce yutuluyordu. Belirtisi en yanilticisiydi:
        // "model arac cagirmiyor" gibi gorunuyor, oysa cagiriyordu.
        //
        // Dogrusu, `(` oncesindeki SON tanimlayiciyi almak. Ad dogrulamasi
        // burada yapilmaz; bilinmeyen ad 1. kapida (`katalog.bul`) zaten
        // reddedilir ve modele duzgun bir hata doner.
        let kapanis = govde.rfind(')')?;
        // Adaylar SAGDAN SOLA denenir: kisit uretimi cagrinin kapanis
        // parantezinde bitirdigi icin gercek cagri metnin SONUNDADIR. Duz
        // metinde gecen parantezler ("... (matematik) ...") boylece cagriyi
        // golgelemez.
        for (acilis, _) in govde[..kapanis].char_indices().rev().filter(|(_, c)| *c == '(') {
            let ad = son_tanimlayici(&govde[..acilis]);
            if ad.is_empty() {
                continue;
            }
            let ic = govde[acilis + 1..kapanis].trim();
            let args = if ic.is_empty() {
                Value::Object(Default::default())
            } else {
                match serde_json::from_str(ic) {
                    Ok(v) => v,
                    // Bu `(` cagrinin acilisi degilmis; soldaki adaya gec.
                    Err(_) => continue,
                }
            };
            return Some(Self::yeni(ad, args));
        }
        None
    }
}

/// Metnin SONUNDAKI tanimlayiciyi (`[A-Za-z0-9_]*`) dondurur.
///
/// ASCII sinirli olmasi bilincli: arac adlari sozlesme geregi ASCII'dir
/// (bkz. isimlendirme kurali — semboller ASCII). Turkce harfleri de kabul
/// etseydik "yapacagim" gibi bir sozcugun sonu gecerli bir ad gibi gorunur,
/// 1. kapiya gereksiz gurultu tasirdi.
fn son_tanimlayici(metin: &str) -> &str {
    let bas = metin
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_ascii_alphanumeric() || *c == '_')
        .last()
        .map(|(i, _)| i)
        .unwrap_or(metin.len());
    &metin[bas..]
}

/// ```` ```json ... ``` ```` citini soyar. Kucuk model istense de istenmese de
/// bunu koyuyor; citi burada bir kez temizlemek her cagri yerinde ayni
/// kontrolu tekrarlamaktan ucuz.
fn kod_citini_soy(ham: &str) -> &str {
    let Some(kalan) = ham.strip_prefix("```") else {
        return ham;
    };
    // Ilk satir dil etiketidir ("json"), govde ondan sonra baslar.
    let govde = kalan.split_once('\n').map(|(_, g)| g).unwrap_or("");
    govde.trim().strip_suffix("```").unwrap_or(govde).trim()
}

// ---------------------------------------------------------------------------
// Sonuc
// ---------------------------------------------------------------------------

/// Yurutmenin YAPISAL sonucu. Cagri yerleri kararlarini bundan verir, metinden
/// degil (bkz. dosya basi).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YurutmeSebebi {
    /// Arac calisti ve basarili durum dondu.
    Tamam,
    /// Katalogta boyle bir arac yok.
    BilinmeyenArac,
    /// Argumanlar semaya uymadi; arac HIC calismadi.
    GecersizArgumanlar,
    /// Arac calisti ve `Basarisiz` dondu.
    AracBasarisiz,
    /// Onay kapisi kapali kaldi; arac HIC calismadi.
    OnayReddedildi,
    /// Tur iptal edildi; arac HIC calismadi.
    Iptal,
}

impl YurutmeSebebi {
    /// Bu sonuc bir ariza mi. Onay reddi ve iptal ARIZA DEGILDIR: ikisi de
    /// kullanicinin verdigi karardir, kurtarma yolu tetiklenmemeli.
    pub fn hata_mi(&self) -> bool {
        matches!(
            self,
            YurutmeSebebi::BilinmeyenArac
                | YurutmeSebebi::GecersizArgumanlar
                | YurutmeSebebi::AracBasarisiz
        )
    }
}

/// Bir arac cagrisinin yurutulmesinin sonucu.
#[derive(Debug, Clone)]
pub struct YurutmeSonucu {
    pub arac_adi: String,
    pub sebep: YurutmeSebebi,
    /// Modele donen KISA metin. Toplu veri burada olmaz (bypass kanali).
    pub modele_donen: String,
    /// Kullaniciya gorunen cip metni. Bos olabilir (onemsiz cagrilar).
    pub cip_metni: String,
    pub durum: AracDurumu,
    /// Bu cagri dis dunyayi degistirdi mi (`Yazildi`).
    pub dunya_degisti: bool,
    /// AYNI ISTEM ikinci kez gonderilebilir mi. Tur icinde bir kez dunya
    /// degistiyse FALSE — yoksa ikinci bir dosya/kayit olusur.
    pub tekrar_denenebilir: bool,
    /// Aracin ham ciktisi (cip detayi, eval kaniti). Modele GITMEZ.
    pub ham_cikti: Option<String>,
}

impl YurutmeSonucu {
    pub fn hata_mi(&self) -> bool {
        self.sebep.hata_mi()
    }
}

// ---------------------------------------------------------------------------
// Onay kapisi
// ---------------------------------------------------------------------------

/// Kullaniciya gosterilecek paylasim istegi. `icerik` GONDERILECEK
/// ARGUMANLARIN AYNISI'dir, kategori ozeti degil: kullanici neye izin
/// verdigini birebir gorsun.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnayIstegi {
    pub kaynak: String,
    pub arac_adi: String,
    pub icerik: String,
}

/// Onay kararini veren merci. UI'da bir sayfa, eval'de sabit bir cevap.
///
/// `&self`: ayni kapiyi es zamanli birden fazla cagri yoklayabilir; ic
/// esitleme somut uygulamanin isi (cekirdekteki `Raporlayici` karariyla ayni).
pub trait OnayKapisi: Send + Sync {
    fn iste(&self, istek: &OnayIstegi) -> bool;
}

/// Headless varsayilan: soracak kimse yok, dolayisiyla veri CIKMAZ.
///
/// Varsayilanin "reddet" olmasi bilincli — sormadan gecirmek, kapinin hic
/// olmamasiyla ayni sey olurdu.
pub struct SessizRet;

impl OnayKapisi for SessizRet {
    fn iste(&self, _istek: &OnayIstegi) -> bool {
        false
    }
}

/// Testler ve "onayli" eval senaryolari icin.
pub struct DaimaOnay;

impl OnayKapisi for DaimaOnay {
    fn iste(&self, _istek: &OnayIstegi) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// Yurutucu
// ---------------------------------------------------------------------------

/// Tur bileti. `yeni_tur()` doner, `yurut` alir.
///
/// NEDEN BILET: iptal bayragini duz bir `bool` tutsaydi, iptal edilen eski
/// turun temizligi (gec gelen bir `iptal_temizle`) YENI turun bayragini
/// dusururdu — kullanicinin ikinci "durdur"u yutulurdu. Bilet bir sayidir ve
/// iptal "su numarali tur iptal" diye kaydedilir; eski turun sozu yeniyi
/// baglamaz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurBileti(pub u64);

pub struct AracYurutucu {
    katalog: AracKatalogu,
    /// Dis dunyaya veri cikarabilen araclarin adlari. Cekirdek `Arac`
    /// sozlesmesinde boyle bir bayrak YOK ve eklenmedi: "disari cikar mi"
    /// sorusu aracin kendi ozelligi degil, DAGITIMIN karari (ayni arac
    /// cevrimdisi kurulumda disari cikmaz). Bu yuzden liste burada tutulur.
    dis_araclar: HashSet<String>,
    kapi: Box<dyn OnayKapisi>,
    /// Oturum omurlu. Kisisel veri araci BASARIYLA calistiginda kurulur;
    /// tur degisimi temizlemez (baglam ozeti kisisel veri tasiyabilir).
    oturum_kirli: AtomicBool,
    /// Tur omurlu, YAPISKAN.
    dunya_degisti: AtomicBool,
    /// Aktif tur numarasi.
    tur: AtomicU64,
    /// Iptal edilen tur numarasi. 0 = hic iptal yok (turler 1'den baslar).
    iptal_edilen: AtomicU64,
    /// Bu oturumda reddedilen kaynaklar — ayni kaynak icin ikinci kez
    /// sorulmaz, model israr dongusune giremez.
    reddedilen: Mutex<HashSet<String>>,
}

impl AracYurutucu {
    pub fn yeni(katalog: AracKatalogu) -> Self {
        Self {
            katalog,
            dis_araclar: HashSet::new(),
            kapi: Box::new(SessizRet),
            oturum_kirli: AtomicBool::new(false),
            dunya_degisti: AtomicBool::new(false),
            tur: AtomicU64::new(1),
            iptal_edilen: AtomicU64::new(0),
            reddedilen: Mutex::new(HashSet::new()),
        }
    }

    /// Bu araci "dis dunyaya veri cikarir" diye isaretler.
    pub fn dis_arac(mut self, ad: impl Into<String>) -> Self {
        self.dis_araclar.insert(ad.into());
        self
    }

    pub fn kapiyla(mut self, kapi: impl OnayKapisi + 'static) -> Self {
        self.kapi = Box::new(kapi);
        self
    }

    pub fn katalog(&self) -> &AracKatalogu {
        &self.katalog
    }

    pub fn oturum_kirli(&self) -> bool {
        self.oturum_kirli.load(Ordering::SeqCst)
    }

    pub fn dunya_degisti(&self) -> bool {
        self.dunya_degisti.load(Ordering::SeqCst)
    }

    /// Ayni istem ikinci kez gonderilebilir mi — kurtarma yolunun TEK karar
    /// noktasi.
    pub fn retry_guvenli(&self) -> bool {
        !self.dunya_degisti()
    }

    pub fn aktif_tur(&self) -> TurBileti {
        TurBileti(self.tur.load(Ordering::SeqCst))
    }

    /// Gercek yeni tur: yan etki bayragi sifirlanir, yeni bilet uretilir.
    pub fn yeni_tur(&self) -> TurBileti {
        self.dunya_degisti.store(false, Ordering::SeqCst);
        TurBileti(self.tur.fetch_add(1, Ordering::SeqCst) + 1)
    }

    /// Kurtarma denemesi — GERCEK yeni tur degil. Bilet ve yan etki bayragi
    /// KORUNUR: yan etkinin olup olmadigi bilgisi tam da retry kararinin
    /// verildigi anda kaybolmamali.
    pub fn kurtarma_denemesi(&self) -> TurBileti {
        self.aktif_tur()
    }

    /// Aktif turu iptal eder.
    pub fn iptal(&self) {
        self.iptal_edilen.store(self.tur.load(Ordering::SeqCst), Ordering::SeqCst);
    }

    pub fn iptal_mi(&self, bilet: TurBileti) -> bool {
        self.iptal_edilen.load(Ordering::SeqCst) == bilet.0
    }

    /// Oturum sonu — kirlilik ve ret onbellegi dahil her sey biter.
    pub fn sohbeti_sifirla(&self) {
        self.oturum_kirli.store(false, Ordering::SeqCst);
        self.dunya_degisti.store(false, Ordering::SeqCst);
        self.reddedilen.lock().expect("ret onbellegi kilidi").clear();
        self.tur.fetch_add(1, Ordering::SeqCst);
    }

    /// Ham model ciktisindan cagriyi ayristirip yurutur.
    ///
    /// `None` = model arac cagirmadi (duz cevap). Bu bir hata degildir.
    pub async fn ham_yurut(
        &self,
        ham: &str,
        bilet: TurBileti,
        ctx: &mut AracBaglami,
    ) -> Option<YurutmeSonucu> {
        let cagri = AracCagrisi::ayristir(ham)?;
        Some(self.yurut(&cagri, bilet, ctx).await)
    }

    /// Tek bir arac cagrisini dort kapidan gecirip yurutur.
    ///
    /// `Result` DONMEZ (cekirdek karari 4 ile ayni gerekce): her yol bir
    /// `YurutmeSonucu` uretir, boylece cagri yerleri "hata yolunu unutmak"
    /// diye bir secenek bulamaz.
    pub async fn yurut(
        &self,
        cagri: &AracCagrisi,
        bilet: TurBileti,
        ctx: &mut AracBaglami,
    ) -> YurutmeSonucu {
        // KAPI 4 — iptal. En basta: iptal edilmis bir turda hicbir arac
        // baslamamali, ucuz olani bile.
        if self.iptal_mi(bilet) {
            return self.sonuc(
                &cagri.ad,
                YurutmeSebebi::Iptal,
                IPTAL_MODEL_METNI.to_string(),
                "İptal edildi".to_string(),
                AracDurumu::Basarisiz("iptal".into()),
                None,
                false,
            );
        }

        // KAPI 1 — ad.
        let Some(arac) = self.katalog.bul(&cagri.ad) else {
            let hata = AracHatasi::GecersizArguman(format!("bilinmeyen arac: {}", cagri.ad));
            return self.sonuc(
                &cagri.ad,
                YurutmeSebebi::BilinmeyenArac,
                HATA_MODEL_METNI.to_string(),
                hata.kisa_hata(),
                AracDurumu::Basarisiz(hata.kisa_hata()),
                None,
                false,
            );
        };

        // KAPI 2 — sema. Arac kendi icinde de dogruluyor; buradaki kat
        // aracin HIC calismamasini garanti eder (yan etkili araclarda fark eder).
        if let Err(hata) = arac.sema().dogrula(&cagri.args) {
            return self.sonuc(
                &cagri.ad,
                YurutmeSebebi::GecersizArgumanlar,
                HATA_MODEL_METNI.to_string(),
                hata.kisa_hata(),
                AracDurumu::Basarisiz(hata.kisa_hata()),
                None,
                false,
            );
        }

        // KAPI 3 — onay. Yalniz kirli oturumda ve yalniz dis araclarda.
        // Temiz oturumda sorulmaz: nadir olan onay okunur, sik olan okunmaz
        // hale gelir ve kapi islevini yitirir.
        if self.dis_araclar.contains(&cagri.ad) && self.oturum_kirli() {
            let icerik = serde_json::to_string(&cagri.args).unwrap_or_default();
            let istek = OnayIstegi {
                kaynak: cagri.ad.clone(),
                arac_adi: cagri.ad.clone(),
                icerik,
            };
            if !self.onay_al(&istek) {
                let iz = ctx.cip_baslat("onay", &format!("{} · gönderilmedi", istek.kaynak));
                ctx.cip_guncelle(
                    iz,
                    sirr_cekirdek::IzGuncelleme::durum(AracDurumu::IzinGerekli)
                        .ham_girdi(istek.icerik.clone()),
                );
                return self.sonuc(
                    &cagri.ad,
                    YurutmeSebebi::OnayReddedildi,
                    RET_MODEL_METNI.to_string(),
                    format!("{} · gönderilmedi", istek.kaynak),
                    AracDurumu::IzinGerekli,
                    None,
                    false,
                );
            }
        }

        let sonuc: AracSonucu = arac.calistir(cagri.args.clone(), ctx).await;

        // Iptal arac KOSARKEN gelmis olabilir. Sonucu yine de dondururuz ama
        // yazma gerceklestiyse bayrak korunur — "iptal ettim, dosya yine de
        // olustu" durumu gizlenmemeli.
        let dunya = sonuc.durum.dunyayi_degistirdi();
        if dunya {
            self.dunya_degisti.store(true, Ordering::SeqCst);
            ctx.kirlet();
        }

        // KIRLETME: yalniz Okundu/Yazildi. IzinGerekli ve Basarisiz
        // KIRLETMEZ — hicbir veri okunmadi, oturumu kirletmek kapiyi
        // gereksiz yere sikilastirirdi.
        let basarili = matches!(sonuc.durum, AracDurumu::Okundu | AracDurumu::Yazildi);
        if basarili && arac.kirletir_mi() {
            self.oturum_kirli.store(true, Ordering::SeqCst);
            ctx.kirlet();
        }

        let sebep = match &sonuc.durum {
            AracDurumu::Basarisiz(_) => YurutmeSebebi::AracBasarisiz,
            AracDurumu::IzinGerekli => YurutmeSebebi::OnayReddedildi,
            _ => YurutmeSebebi::Tamam,
        };

        self.sonuc(
            &cagri.ad,
            sebep,
            sonuc.modele_donen,
            sonuc.cip_metni,
            sonuc.durum,
            sonuc.ham_cikti,
            dunya,
        )
    }

    fn onay_al(&self, istek: &OnayIstegi) -> bool {
        {
            let red = self.reddedilen.lock().expect("ret onbellegi kilidi");
            if red.contains(&istek.kaynak) {
                return false;
            }
        }
        let kabul = self.kapi.iste(istek);
        if !kabul {
            self.reddedilen
                .lock()
                .expect("ret onbellegi kilidi")
                .insert(istek.kaynak.clone());
        }
        kabul
    }

    #[allow(clippy::too_many_arguments)]
    fn sonuc(
        &self,
        ad: &str,
        sebep: YurutmeSebebi,
        modele_donen: String,
        cip_metni: String,
        durum: AracDurumu,
        ham_cikti: Option<String>,
        dunya_degisti: bool,
    ) -> YurutmeSonucu {
        YurutmeSonucu {
            arac_adi: ad.to_string(),
            sebep,
            modele_donen,
            cip_metni,
            durum,
            dunya_degisti,
            // TUR duzeyinde bakilir, bu cagri duzeyinde degil: turun onceki
            // adiminda dosya yazildiysa bu cagrinin masumiyeti retry'i guvenli
            // yapmaz.
            tekrar_denenebilir: !self.dunya_degisti(),
            ham_cikti,
        }
    }
}

#[cfg(test)]
mod testler {
    use super::*;
    use sirr_cekirdek::{
        Alan, Arac, AracGelecegi, ArgSema, BellekVeriDeposu, SessizRaporlayici, kutula,
    };
    use std::sync::Arc;

    // --- Yardimcilar ---

    /// Kisisel veri okuyan arac: oturumu kirletir.
    struct KisiselArac;
    impl Arac for KisiselArac {
        fn ad(&self) -> &str {
            "kisisel_oku"
        }
        fn aciklama(&self) -> &str {
            "Kisisel veri okur."
        }
        fn sema(&self) -> ArgSema {
            ArgSema::nesne(vec![Alan::yeni("ne", ArgSema::metin()).zorunlu()])
        }
        fn kirletir_mi(&self) -> bool {
            true
        }
        fn calistir<'a>(&'a self, _a: Value, _c: &'a mut AracBaglami) -> AracGelecegi<'a> {
            kutula(async move { AracSonucu::okundu("okundu", "veri: 42") })
        }
    }

    /// Disari veri gonderen arac.
    struct DisArac;
    impl Arac for DisArac {
        fn ad(&self) -> &str {
            "disari_gonder"
        }
        fn aciklama(&self) -> &str {
            "Veriyi disariya gonderir."
        }
        fn sema(&self) -> ArgSema {
            ArgSema::nesne(vec![Alan::yeni("govde", ArgSema::metin()).zorunlu()])
        }
        fn calistir<'a>(&'a self, _a: Value, _c: &'a mut AracBaglami) -> AracGelecegi<'a> {
            kutula(async move { AracSonucu::okundu("gonderildi", "sent") })
        }
    }

    /// Dunyayi degistiren arac.
    struct YazanArac;
    impl Arac for YazanArac {
        fn ad(&self) -> &str {
            "yaz"
        }
        fn aciklama(&self) -> &str {
            "Dosya yazar."
        }
        fn sema(&self) -> ArgSema {
            ArgSema::nesne(vec![])
        }
        fn calistir<'a>(&'a self, _a: Value, _c: &'a mut AracBaglami) -> AracGelecegi<'a> {
            kutula(async move { AracSonucu::yazildi("yazildi", "file_created") })
        }
    }

    /// Hep basarisiz olan arac.
    struct CokenArac;
    impl Arac for CokenArac {
        fn ad(&self) -> &str {
            "coken"
        }
        fn aciklama(&self) -> &str {
            "Hep coker."
        }
        fn sema(&self) -> ArgSema {
            ArgSema::nesne(vec![])
        }
        fn kirletir_mi(&self) -> bool {
            true
        }
        fn calistir<'a>(&'a self, _a: Value, _c: &'a mut AracBaglami) -> AracGelecegi<'a> {
            kutula(async move { AracSonucu::basarisiz(&AracHatasi::YerYok) })
        }
    }

    fn baglam() -> AracBaglami {
        AracBaglami::yeni(
            Arc::new(BellekVeriDeposu::yeni()),
            std::env::temp_dir(),
            Arc::new(SessizRaporlayici),
        )
    }

    fn yurutucu() -> AracYurutucu {
        let mut k = AracKatalogu::yeni();
        k.ekle(Arc::new(KisiselArac))
            .ekle(Arc::new(DisArac))
            .ekle(Arc::new(YazanArac))
            .ekle(Arc::new(CokenArac));
        AracYurutucu::yeni(k).dis_arac("disari_gonder")
    }

    /// Minimal poll dongusu — bu crate calistirici SECMEMELI (cekirdekteki
    /// gerekcenin aynisi), o yuzden testte de tokio yok.
    fn kos<F: Future<Output = T>, T>(f: F) -> T {
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        fn bos(_: *const ()) {}
        fn klon(p: *const ()) -> RawWaker {
            RawWaker::new(p, &VT)
        }
        static VT: RawWakerVTable = RawWakerVTable::new(klon, bos, bos, bos);
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VT)) };
        let mut cx = Context::from_waker(&waker);
        let mut f = std::pin::pin!(f);
        loop {
            if let Poll::Ready(v) = std::pin::Pin::as_mut(&mut f).poll(&mut cx) {
                return v;
            }
        }
    }

    use std::future::Future;

    // --- Ayristirma ---

    #[test]
    fn kanonik_json_ayristirilir() {
        let c = AracCagrisi::ayristir(r#"{"arac":"hesapla","args":{"ifade":"2+2"}}"#).unwrap();
        assert_eq!(c.ad, "hesapla");
        assert_eq!(c.args["ifade"], "2+2");
    }

    #[test]
    fn cagri_bicimi_ayristirilir() {
        let c = AracCagrisi::ayristir(r#"hesapla({"ifade":"2+2"})"#).unwrap();
        assert_eq!(c.ad, "hesapla");
        assert_eq!(c.args["ifade"], "2+2");
    }

    #[test]
    fn kod_citi_soyulur() {
        let c = AracCagrisi::ayristir("```json\n{\"arac\":\"zaman\",\"args\":{}}\n```").unwrap();
        assert_eq!(c.ad, "zaman");
    }

    #[test]
    fn duz_cevap_cagri_degildir() {
        assert!(AracCagrisi::ayristir("Merhaba, nasil yardimci olabilirim?").is_none());
        assert!(AracCagrisi::ayristir("").is_none());
    }

    /// GERCEK MODEL CIKTISI (Qwen3-4B, birebir). Model once niyetini yazip
    /// sonra cagriyi uretiyor; eski ayristirici bunu sessizce yutuyordu.
    #[test]
    fn duz_metin_onekli_cagri_ayristirilir() {
        let ham = "İstanbul'un hava durumunu sorgulamak için web araması yapacağım.\n\n\
                   web_ara({\"sorgu\":\"istanbul hava durumu\"})";
        let c = AracCagrisi::ayristir(ham).expect("onekli cagri ayristirilamadi");
        assert_eq!(c.ad, "web_ara");
        assert_eq!(c.args["sorgu"], "istanbul hava durumu");
    }

    /// Onekteki parantezler cagriyi golgelememeli: adaylar SAGDAN denenir.
    #[test]
    fn onekteki_parantez_cagriyi_golgelemez() {
        let ham = "Once bir hesap (basit aritmetik) yapmam gerek: hesapla({\"ifade\":\"2+2\"})";
        let c = AracCagrisi::ayristir(ham).expect("cagri bulunamadi");
        assert_eq!(c.ad, "hesapla");
        assert_eq!(c.args["ifade"], "2+2");
    }

    /// Parantez var ama ONUNDE tanimlayici yok — cagri degildir.
    #[test]
    fn tanimlayicisiz_parantez_cagri_sayilmaz() {
        assert!(AracCagrisi::ayristir("Bu bir aciklama (parantez icinde).").is_none());
        assert!(AracCagrisi::ayristir("({\"a\":1})").is_none());
    }

    // --- Kapilar ---

    #[test]
    fn bilinmeyen_arac_calismaz_ve_sabit_metin_doner() {
        let y = yurutucu();
        let mut ctx = baglam();
        let s = kos(y.yurut(&AracCagrisi::yeni("yok_boyle", serde_json::json!({})), y.aktif_tur(), &mut ctx));
        assert_eq!(s.sebep, YurutmeSebebi::BilinmeyenArac);
        assert!(s.hata_mi());
        assert_eq!(s.modele_donen, HATA_MODEL_METNI);
    }

    #[test]
    fn sema_ihlalinde_arac_hic_calismaz() {
        let y = yurutucu();
        let mut ctx = baglam();
        // "ne" zorunlu; verilmiyor.
        let s = kos(y.yurut(&AracCagrisi::yeni("kisisel_oku", serde_json::json!({})), y.aktif_tur(), &mut ctx));
        assert_eq!(s.sebep, YurutmeSebebi::GecersizArgumanlar);
        // Arac calismadigi icin oturum da kirlenmedi.
        assert!(!y.oturum_kirli());
    }

    #[test]
    fn basarili_kisisel_arac_oturumu_kirletir() {
        let y = yurutucu();
        let mut ctx = baglam();
        assert!(!y.oturum_kirli());
        kos(y.yurut(
            &AracCagrisi::yeni("kisisel_oku", serde_json::json!({"ne": "takvim"})),
            y.aktif_tur(),
            &mut ctx,
        ));
        assert!(y.oturum_kirli());
        assert!(ctx.oturum_kirli());
    }

    #[test]
    fn basarisiz_arac_oturumu_kirletmez() {
        // CokenArac kirletir_mi()==true ama Basarisiz donuyor: hicbir veri
        // okunmadi, kapiyi sikilastirmak icin sebep yok.
        let y = yurutucu();
        let mut ctx = baglam();
        let s = kos(y.yurut(&AracCagrisi::yeni("coken", serde_json::json!({})), y.aktif_tur(), &mut ctx));
        assert_eq!(s.sebep, YurutmeSebebi::AracBasarisiz);
        assert!(!y.oturum_kirli());
    }

    #[test]
    fn temiz_oturumda_dis_arac_sormadan_gecer() {
        let y = yurutucu(); // varsayilan kapi: SessizRet
        let mut ctx = baglam();
        let s = kos(y.yurut(
            &AracCagrisi::yeni("disari_gonder", serde_json::json!({"govde": "selam"})),
            y.aktif_tur(),
            &mut ctx,
        ));
        assert_eq!(s.sebep, YurutmeSebebi::Tamam);
    }

    #[test]
    fn kirli_oturumda_dis_arac_kapiya_takilir() {
        let y = yurutucu();
        let mut ctx = baglam();
        kos(y.yurut(
            &AracCagrisi::yeni("kisisel_oku", serde_json::json!({"ne": "takvim"})),
            y.aktif_tur(),
            &mut ctx,
        ));
        let s = kos(y.yurut(
            &AracCagrisi::yeni("disari_gonder", serde_json::json!({"govde": "veri: 42"})),
            y.aktif_tur(),
            &mut ctx,
        ));
        assert_eq!(s.sebep, YurutmeSebebi::OnayReddedildi);
        assert_eq!(s.modele_donen, RET_MODEL_METNI);
        // Ret bir ARIZA degil: kurtarma yolu tetiklenmemeli.
        assert!(!s.hata_mi());
    }

    #[test]
    fn onay_verilirse_kirli_oturumda_da_gecer() {
        let mut k = AracKatalogu::yeni();
        k.ekle(Arc::new(KisiselArac)).ekle(Arc::new(DisArac));
        let y = AracYurutucu::yeni(k).dis_arac("disari_gonder").kapiyla(DaimaOnay);
        let mut ctx = baglam();
        kos(y.yurut(
            &AracCagrisi::yeni("kisisel_oku", serde_json::json!({"ne": "x"})),
            y.aktif_tur(),
            &mut ctx,
        ));
        let s = kos(y.yurut(
            &AracCagrisi::yeni("disari_gonder", serde_json::json!({"govde": "x"})),
            y.aktif_tur(),
            &mut ctx,
        ));
        assert_eq!(s.sebep, YurutmeSebebi::Tamam);
    }

    #[test]
    fn bir_kez_reddedilen_kaynak_bir_daha_sorulmaz() {
        /// Kac kez SORULDUGUNU sayan kapi — ret onbelleginin tek kaniti.
        struct SayanKapi(Arc<std::sync::atomic::AtomicUsize>);
        impl OnayKapisi for SayanKapi {
            fn iste(&self, _i: &OnayIstegi) -> bool {
                self.0.fetch_add(1, Ordering::SeqCst);
                false
            }
        }
        let sayac = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let mut k = AracKatalogu::yeni();
        k.ekle(Arc::new(KisiselArac)).ekle(Arc::new(DisArac));
        let y = AracYurutucu::yeni(k)
            .dis_arac("disari_gonder")
            .kapiyla(SayanKapi(Arc::clone(&sayac)));
        let mut ctx = baglam();
        kos(y.yurut(
            &AracCagrisi::yeni("kisisel_oku", serde_json::json!({"ne": "x"})),
            y.aktif_tur(),
            &mut ctx,
        ));
        for _ in 0..3 {
            kos(y.yurut(
                &AracCagrisi::yeni("disari_gonder", serde_json::json!({"govde": "x"})),
                y.aktif_tur(),
                &mut ctx,
            ));
        }
        // Uc deneme, TEK soru: model israr dongusune giremez.
        assert_eq!(sayac.load(Ordering::SeqCst), 1);
    }

    // --- Yan etki / retry ---

    #[test]
    fn dunya_degisince_retry_yasaklanir() {
        let y = yurutucu();
        let mut ctx = baglam();
        assert!(y.retry_guvenli());
        let s = kos(y.yurut(&AracCagrisi::yeni("yaz", serde_json::json!({})), y.aktif_tur(), &mut ctx));
        assert!(s.dunya_degisti);
        assert!(!s.tekrar_denenebilir);
        assert!(!y.retry_guvenli());
    }

    #[test]
    fn yan_etkiden_sonraki_masum_cagri_da_retry_edilemez() {
        let y = yurutucu();
        let mut ctx = baglam();
        kos(y.yurut(&AracCagrisi::yeni("yaz", serde_json::json!({})), y.aktif_tur(), &mut ctx));
        let s = kos(y.yurut(
            &AracCagrisi::yeni("kisisel_oku", serde_json::json!({"ne": "x"})),
            y.aktif_tur(),
            &mut ctx,
        ));
        assert!(!s.dunya_degisti);
        // Cagri masum ama TUR degil.
        assert!(!s.tekrar_denenebilir);
    }

    #[test]
    fn kurtarma_denemesi_yan_etki_bayragini_dusurmez() {
        let y = yurutucu();
        let mut ctx = baglam();
        kos(y.yurut(&AracCagrisi::yeni("yaz", serde_json::json!({})), y.aktif_tur(), &mut ctx));
        let bilet = y.kurtarma_denemesi();
        assert_eq!(bilet, y.aktif_tur());
        assert!(!y.retry_guvenli());
        // Gercek yeni tur sifirlar.
        y.yeni_tur();
        assert!(y.retry_guvenli());
    }

    #[test]
    fn yeni_tur_kirliligi_temizlemez_sohbet_sifirlamasi_temizler() {
        let y = yurutucu();
        let mut ctx = baglam();
        kos(y.yurut(
            &AracCagrisi::yeni("kisisel_oku", serde_json::json!({"ne": "x"})),
            y.aktif_tur(),
            &mut ctx,
        ));
        y.yeni_tur();
        assert!(y.oturum_kirli(), "baglam ozeti kisisel veri tasiyabilir");
        y.sohbeti_sifirla();
        assert!(!y.oturum_kirli());
    }

    // --- Iptal ---

    #[test]
    fn iptal_edilen_turda_arac_calismaz() {
        let y = yurutucu();
        let mut ctx = baglam();
        let bilet = y.aktif_tur();
        y.iptal();
        let s = kos(y.yurut(&AracCagrisi::yeni("yaz", serde_json::json!({})), bilet, &mut ctx));
        assert_eq!(s.sebep, YurutmeSebebi::Iptal);
        assert!(!s.dunya_degisti);
        assert!(!y.dunya_degisti(), "iptal edilen arac hic calismadi");
    }

    #[test]
    fn eski_turun_iptali_yeni_turu_baglamaz() {
        let y = yurutucu();
        let mut ctx = baglam();
        let eski = y.aktif_tur();
        y.iptal();
        assert!(y.iptal_mi(eski));

        let yeni = y.yeni_tur();
        assert!(!y.iptal_mi(yeni), "eski turun iptali yeni turu dusurmemeli");
        let s = kos(y.yurut(&AracCagrisi::yeni("yaz", serde_json::json!({})), yeni, &mut ctx));
        assert_eq!(s.sebep, YurutmeSebebi::Tamam);
    }

    // --- Ham yurutme ---

    #[test]
    fn duz_metin_yurutulmez() {
        let y = yurutucu();
        let mut ctx = baglam();
        assert!(kos(y.ham_yurut("Merhaba!", y.aktif_tur(), &mut ctx)).is_none());
    }

    #[test]
    fn ham_json_yurutulur() {
        let y = yurutucu();
        let mut ctx = baglam();
        let s = kos(y.ham_yurut(
            r#"{"arac":"kisisel_oku","args":{"ne":"takvim"}}"#,
            y.aktif_tur(),
            &mut ctx,
        ))
        .unwrap();
        assert_eq!(s.sebep, YurutmeSebebi::Tamam);
        assert_eq!(s.modele_donen, "veri: 42");
    }
}
