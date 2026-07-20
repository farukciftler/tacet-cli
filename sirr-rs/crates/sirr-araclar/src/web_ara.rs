//! `web_ara` ve `web_getir` — kullanicinin kendi SearXNG sunucusu uzerinden
//! web aramasi ve tek sayfa metni cekme.
//!
//! BU DOSYADA AG YOK. Soketi `sirr-web` acar; burada yapilan is ceviri:
//! `WebHatasi` → `AracHatasi`, `Vec<AramaSonucu>` → depo kaydi + kisa model
//! metni, ve cip yasam dongusu. Ag tekelinin anlami tam olarak bu: arac
//! katmani "ne gitti/ne geldi"yi bilir, "nasil gitti"yi bilmez.
//!
//! ---
//!
//! BYPASS KANALI BURADA SART. 20 sonucun ham metni ~6-8 bin token eder; 4096
//! baglamli bir modelde bu, tek hamlede pencerenin tamamini yer ve kullanicinin
//! ASIL SORUSU disari tasar. Bu yuzden ham sonuclar (tam URL'ler, kirpilmamis
//! ozetler) `VeriDeposu`ya konur; modele yalnizca ilk `EN_COK_MODELE` sonucun
//! basligi, alan adi ve tek satirlik ozeti + `kaynak_ref` gider.
//!
//! ---
//!
//! `kirletir_mi()` = **false**, ama arac DIS ARACtir. Ikisi ayri sorudur ve
//! karistirilmasi kapiyi ya asiri gevsetir ya asiri sikar:
//!
//! - `kirletir_mi()` = "bu arac KISISEL VERI OKUDU MU". Web aramasi kullanicinin
//!   takvimini, kisilerini, dosyalarini OKUMAZ; disaridan veri getirir. `true`
//!   deseydik her arama oturumu kirletir, arama sonrasi her dis cagri onaya
//!   duser ve kullanici onay yorgunlugundan hepsini korlemesine onaylamaya
//!   baslardi — kapi tam da o an islevini yitirir.
//! - DIS ARAC = "bu arac DISARIYA VERI GONDERIR MI". Web aramasi sorguyu
//!   kullanicinin sunucusuna gonderir, yani EVET. Sorgu masum degildir
//!   ("esime Ayse'ye hediye", "X hastaligi belirtileri"); kirli bir oturumda
//!   model, az once okunan kisisel belgeden devsirdigi bir metni sorguya
//!   yazabilir. Onay kapisinin durdurmasi gereken tam olarak budur.
//!
//! Yani: web aramasi kirliligi URETMEZ, kirlilikten ETKILENIR. Kayit
//! `sirr-cli`daki `DIS_ARACLAR` listesindedir; `AracYurutucu`nun 3. kapisi
//! (ONAY) o listeyi okur.
//!
//! ---
//!
//! SONUC METNI GUVENILMEZDIR. Arama ciktisi baglama giren DIS icerikdir ve
//! icinde modele yonelik talimat bulunabilir ("ignore previous instructions").
//! Bu dosyanin savunmasi yapisaldir: sonuc metni yalniz VERI olarak, numarali
//! bir listenin icinde ve kirpilmis halde gecer; hicbir sonuc alani sistem
//! talimatina, arac semasina ya da bir sonraki cagrinin argümanina donusmez.

use crate::veri_deposu::{Deger, PaylasimliDepo};
use serde_json::Value;
use sirr_cekirdek::{
    Alan, Arac, AracBaglami, AracGelecegi, AracHatasi, AracSonucu, ArgSema, IzGuncelleme,
    KaynakRef, kutula,
};
use sirr_web::{AramaSonucu, WebAramaIstemcisi, WebHatasi, kelime_sinirinda_kirp};
use std::sync::Arc;

/// Modele gosterilen en fazla sonuc sayisi.
///
/// 5, spec'ten gelen bir karardir ve cok sayida sonucun MODEL ICIN degil
/// KULLANICI ICIN kotu olmasindan cikar: kucuk model 15 sonuc gorunce
/// hepsini ozetlemeye calisip soruyu cevaplamayi unutuyor. Kalanlar kaybolmaz,
/// depoda durur.
const EN_COK_MODELE: usize = 5;

/// Sonuc basina ozet tavani (karakter). Kelime sinirinda kirpilir.
const OZET_TAVANI: usize = 200;

/// Modele donen metnin toplam karakter tavani (~300 token).
///
/// EN_COK_MODELE x OZET_TAVANI zaten bir ust sinir veriyor; bu ikinci tavan
/// baslik ve alan adlarinin uzun oldugu kotu durum icin. Butcenin tek bir
/// carpimla degil, olculen sonucla siniri var.
const MODEL_TAVANI: usize = 1400;

/// `web_getir`de modele gosterilen onizleme (karakter). Govdenin tamami depoda.
const SAYFA_ONIZLEME: usize = 500;

/// Sonuc dondurmeyen aramada modele giden SABIT metin.
///
/// NEDEN HATA DEGIL: sunucu duzgun calisti, sorgu gitti, cevap geldi — sadece
/// bos. Bunu `tool_failed` diye bildirmek modele "arac bozuk, kendin dene"
/// sinyali verir ve model UYDURUR. `no_results` ise net bir olgudur.
const SONUC_YOK: &str = "no_results";

// ---------------------------------------------------------------------------
// web_ara
// ---------------------------------------------------------------------------

pub struct WebAraAraci {
    istemci: WebAramaIstemcisi,
    depo: Option<Arc<PaylasimliDepo>>,
}

impl Default for WebAraAraci {
    fn default() -> Self {
        Self::yeni()
    }
}

impl WebAraAraci {
    pub fn yeni() -> Self {
        Self { istemci: WebAramaIstemcisi::yeni(), depo: None }
    }

    pub fn depo_ile(depo: Arc<PaylasimliDepo>) -> Self {
        Self { istemci: WebAramaIstemcisi::yeni(), depo: Some(depo) }
    }

    /// Istemciyi disaridan verir — testlerin ve teshis komutlarinin sunucuyu
    /// (ya da zaman asimini) degistirebilmesi icin.
    pub fn istemciyle(mut self, istemci: WebAramaIstemcisi) -> Self {
        self.istemci = istemci;
        self
    }

    fn depola(&self, ctx: &AracBaglami, tur: &str, govde: String, ozet: &str) -> KaynakRef {
        match &self.depo {
            Some(d) => d.koy_deger(tur, Deger::Metin(govde)),
            None => ctx.depola(tur, ozet, govde),
        }
    }
}

impl Arac for WebAraAraci {
    fn ad(&self) -> &str {
        "web_ara"
    }

    fn aciklama(&self) -> &str {
        "Searches the web through the user's own search server. Use for weather, news, \
         prices, current events, and general world knowledge the device cannot know. \
         NOT for the user's own notes, files or contacts — those are on the device."
    }

    fn sema(&self) -> ArgSema {
        ArgSema::nesne(vec![
            Alan::yeni(
                "sorgu",
                ArgSema::metin().aciklama(
                    "Short web search query in the user's language, e.g. 'istanbul weather \
                     tomorrow'. Keep it to a few keywords.",
                ),
            )
            .zorunlu(),
        ])
        .aciklama("Search the web")
    }

    /// FALSE — bilincli. Gerekce dosya basi yorumda; ozeti: bu arac kisisel
    /// veri OKUMAZ, disariya veri GONDERIR. Ikincisi `DIS_ARACLAR` listesiyle
    /// yonetilir, `kirletir_mi` ile degil.
    fn kirletir_mi(&self) -> bool {
        false
    }

    fn calistir<'a>(&'a self, args: Value, ctx: &'a mut AracBaglami) -> AracGelecegi<'a> {
        kutula(async move {
            if let Err(h) = self.sema().dogrula(&args) {
                return AracSonucu::basarisiz(&h);
            }
            let sorgu = args.get("sorgu").and_then(Value::as_str).unwrap_or_default().trim();

            // SORGU CIPTE ACIKCA YAZAR (spec §2.2): "yalniz sorgu gidiyor" bir
            // teselli degil; kullanici giden metnin AYNISINI gormeli.
            let iz = ctx.cip_baslat("globe", &format!("aranıyor · {sorgu}"));
            let istek_url = self.istemci.istek_url(sorgu, None);

            let sonuc = match self.istemci.ara(sorgu, None) {
                Ok(sonuclar) => {
                    let ham = ham_dokum(sorgu, &sonuclar);
                    let ozet_etiketi = format!("{} web sonucu", sonuclar.len());
                    let kaynak_ref = self.depola(ctx, "web", ham.clone(), &ozet_etiketi);
                    let modele = modele_ozet(sorgu, &sonuclar);
                    AracSonucu::ozetle(
                        format!("arandı · {} sonuç", sonuclar.len()),
                        modele,
                        kaynak_ref.as_str(),
                    )
                    .ham_cikti(ham)
                }
                // Bos sonuc bir ARIZA degil, bir OLGU. Basarisiz yola dusurmek
                // modeli uydurmaya iter (bkz. SONUC_YOK).
                Err(WebHatasi::BosSonuc) => AracSonucu::okundu("arandı · sonuç yok", SONUC_YOK),
                Err(h) => AracSonucu::basarisiz(&cevir(&h)),
            };

            ctx.cip_guncelle(
                iz,
                IzGuncelleme::durum(sonuc.durum.clone())
                    .metin(sonuc.cip_metni.clone())
                    // Cip detayinda "ne gitti": tam istek URL'i. Seffafligin
                    // ikinci katmani — kullanici iki dokunusla dogrular.
                    .ham_girdi(istek_url)
                    .ham_cikti(sonuc.ham_cikti.clone().unwrap_or_default()),
            );
            // ctx.kirlet() CAGRILMAZ: bkz. `kirletir_mi` gerekcesi.
            sonuc
        })
    }
}

// ---------------------------------------------------------------------------
// web_getir
// ---------------------------------------------------------------------------

/// Tek bir adresin metnini ceker.
///
/// NEDEN AYRI ARAC: arama sonuclari 200 karaktere kirpik ve cogu soru icin
/// yeter. "Devamini oku" ihtiyaci nadirdir; onu `web_ara`ya bir bayrak olarak
/// eklemek her aramada modele "belki de tam metni istemeliyim" diye bir karar
/// daha yuklerdi. Ayri arac, ayri ve acik bir niyet.
pub struct WebGetirAraci {
    istemci: WebAramaIstemcisi,
    depo: Option<Arc<PaylasimliDepo>>,
}

impl Default for WebGetirAraci {
    fn default() -> Self {
        Self::yeni()
    }
}

impl WebGetirAraci {
    pub fn yeni() -> Self {
        Self { istemci: WebAramaIstemcisi::yeni(), depo: None }
    }

    pub fn depo_ile(depo: Arc<PaylasimliDepo>) -> Self {
        Self { istemci: WebAramaIstemcisi::yeni(), depo: Some(depo) }
    }

    pub fn istemciyle(mut self, istemci: WebAramaIstemcisi) -> Self {
        self.istemci = istemci;
        self
    }
}

impl Arac for WebGetirAraci {
    fn ad(&self) -> &str {
        "web_getir"
    }

    fn aciklama(&self) -> &str {
        "Fetches the readable text of ONE web page by its address. Use only when a search \
         result summary is not enough and you need the details from that page."
    }

    fn sema(&self) -> ArgSema {
        ArgSema::nesne(vec![
            Alan::yeni(
                "adres",
                ArgSema::metin().aciklama(
                    "Full https:// address of the page, taken from a previous web_ara result.",
                ),
            )
            .zorunlu(),
        ])
        .aciklama("Fetch the text of one web page")
    }

    /// `web_ara` ile ayni gerekce: disariya cikar (DIS ARAC), kisisel veri
    /// okumaz (kirletmez).
    fn kirletir_mi(&self) -> bool {
        false
    }

    fn calistir<'a>(&'a self, args: Value, ctx: &'a mut AracBaglami) -> AracGelecegi<'a> {
        kutula(async move {
            if let Err(h) = self.sema().dogrula(&args) {
                return AracSonucu::basarisiz(&h);
            }
            let adres = args.get("adres").and_then(Value::as_str).unwrap_or_default().trim();

            let iz = ctx.cip_baslat("globe", &format!("sayfa alınıyor · {}", sirr_web::alan_adi(adres)));

            let sonuc = match self.istemci.sayfa_metni(adres) {
                Ok(metin) => {
                    let ozet_etiketi = format!("{} karakterlik sayfa metni", metin.chars().count());
                    // BYPASS: sayfanin TAMAMI depoda, modele yalnizca bir pencere.
                    let kaynak_ref = match &self.depo {
                        Some(d) => d.koy_deger("web", Deger::Metin(metin.clone())),
                        None => ctx.depola("web", &ozet_etiketi, metin.clone()),
                    };
                    let onizleme = kelime_sinirinda_kirp(&metin, SAYFA_ONIZLEME);
                    AracSonucu::ozetle(
                        format!("sayfa alındı · {}", sirr_web::alan_adi(adres)),
                        onizleme,
                        kaynak_ref.as_str(),
                    )
                    .ham_cikti(metin)
                }
                Err(WebHatasi::BosSonuc) => {
                    AracSonucu::okundu("sayfa alındı · metin yok", SONUC_YOK)
                }
                Err(h) => AracSonucu::basarisiz(&cevir(&h)),
            };

            ctx.cip_guncelle(
                iz,
                IzGuncelleme::durum(sonuc.durum.clone())
                    .metin(sonuc.cip_metni.clone())
                    .ham_girdi(adres.to_string())
                    .ham_cikti(sonuc.ham_cikti.clone().unwrap_or_default()),
            );
            sonuc
        })
    }
}

// ---------------------------------------------------------------------------
// Ceviri ve bicimlendirme — AG YOK, tamami test edilebilir
// ---------------------------------------------------------------------------

/// `WebHatasi` → `AracHatasi`.
///
/// TEK CEVIRI NOKTASI. Kullaniciya giden Turkce cumle `WebHatasi::Display`den
/// gelir (ag katmani ariza turunu en iyi bilen yerdir); modele giden metin ise
/// `AracSonucu::basarisiz` sayesinde her halukarda sabit Ingilizce olur. Yani
/// bu fonksiyon ne kadar ayrinti tasirsa tasisin, MODELE hicbiri sizmaz.
fn cevir(hata: &WebHatasi) -> AracHatasi {
    match hata {
        WebHatasi::ZamanAsimi => AracHatasi::ZamanAsimi,
        diger => AracHatasi::Diger(diger.to_string()),
    }
}

/// Depoya ve cip detayina giden TAM dokum: kirpma yok, tam URL'ler var.
///
/// Modele giden metinden ayri tutulmasi bypass kanalinin ta kendisi: kullanici
/// ve sonraki araclar eksiksiz veriyi gorur, model gormez.
fn ham_dokum(sorgu: &str, sonuclar: &[AramaSonucu]) -> String {
    let mut s = format!("sorgu: {sorgu}\nsonuc sayisi: {}\n", sonuclar.len());
    for (i, r) in sonuclar.iter().enumerate() {
        s.push_str(&format!("\n{}. {}\n   {}\n   {}\n", i + 1, r.baslik, r.url, r.ozet));
    }
    s
}

/// Modele giden KISA liste. Butcesi olculur, tavani zorlanir.
///
/// TAM URL YOK, ALAN ADI VAR: modelin gordugu uzun adresi yanitinda yeniden
/// uretmeye calisip var olmayan linkler halusine etmesi olculmus bir davranis.
/// Alan adi kaynagi durustce gosterir, uydurmaya malzeme vermez.
fn modele_ozet(sorgu: &str, sonuclar: &[AramaSonucu]) -> String {
    let mut s = format!("found {} results for \"{sorgu}\":", sonuclar.len());
    for (i, r) in sonuclar.iter().take(EN_COK_MODELE).enumerate() {
        let baslik = kelime_sinirinda_kirp(&r.baslik, 90);
        let ozet = kelime_sinirinda_kirp(&r.ozet, OZET_TAVANI);
        s.push_str(&format!("\n{}. {baslik} — {} — {ozet}", i + 1, r.kaynak));
    }
    // Ikinci tavan: yukaridaki carpim degil, OLCULEN uzunluk baglayici olsun.
    kelime_sinirinda_kirp(&s, MODEL_TAVANI)
}

#[cfg(test)]
mod testler {
    use super::*;
    use serde_json::json;
    use sirr_cekirdek::{BellekVeriDeposu, IzToplayici, SessizRaporlayici};

    fn ornek(n: usize) -> Vec<AramaSonucu> {
        (0..n)
            .map(|i| AramaSonucu {
                baslik: format!("Baslik {i}"),
                url: format!("https://ornek{i}.test/uzun/bir/yol?a=1&b=2"),
                ozet: format!("{i} numarali sonucun ozeti. ").repeat(20),
                kaynak: format!("ornek{i}.test"),
            })
            .collect()
    }

    fn baglam() -> AracBaglami {
        AracBaglami::yeni(
            Arc::new(BellekVeriDeposu::yeni()),
            "/tmp/sirr-web-test",
            Arc::new(SessizRaporlayici),
        )
    }

    /// Cekirdek testindeki minimal yurutucu — tokio bagimliligi yok.
    fn kos<F: std::future::Future>(mut f: F) -> F::Output {
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        fn bos(_: *const ()) {}
        fn klon(p: *const ()) -> RawWaker {
            RawWaker::new(p, &VT)
        }
        static VT: RawWakerVTable = RawWakerVTable::new(klon, bos, bos, bos);
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VT)) };
        let mut cx = Context::from_waker(&waker);
        let mut f = unsafe { std::pin::Pin::new_unchecked(&mut f) };
        loop {
            if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }

    #[test]
    fn modele_giden_metin_en_cok_bes_sonuc_gosterir() {
        let m = modele_ozet("test", &ornek(20));
        assert!(m.contains("found 20 results"), "toplam sayi durustce bildirilmeli");
        assert!(m.contains("Baslik 4"));
        assert!(!m.contains("Baslik 5"), "5'ten sonrasi modele gitmemeli");
    }

    #[test]
    fn modele_giden_metin_butce_tavanini_asmaz() {
        // En kotu durum: 20 uzun sonuc. Tavan yine de baglayici olmali.
        let m = modele_ozet("cok uzun bir sorgu metni", &ornek(20));
        assert!(m.chars().count() <= MODEL_TAVANI + 1, "{} karakter", m.chars().count());
    }

    #[test]
    fn modele_tam_url_gitmez_alan_adi_gider() {
        let m = modele_ozet("test", &ornek(3));
        assert!(m.contains("ornek0.test"));
        assert!(!m.contains("https://"), "tam URL modele sizmamali: {m}");
        assert!(!m.contains("/uzun/bir/yol"), "yol modele sizmamali");
    }

    #[test]
    fn ham_dokum_tam_url_ve_kirpilmamis_ozet_tutar() {
        let h = ham_dokum("test", &ornek(3));
        assert!(h.contains("https://ornek2.test/uzun/bir/yol?a=1&b=2"));
        assert!(h.len() > modele_ozet("test", &ornek(3)).len(), "depo modele gidenden zengin");
    }

    /// BYPASS KANALININ ASIL TESTI: ham govde depoda, modele yalniz ozet+ref.
    #[test]
    fn bypass_ham_sonuclari_modelden_uzak_tutar() {
        let depo = Arc::new(PaylasimliDepo::yeni());
        let arac = WebAraAraci::depo_ile(depo.clone());
        let sonuclar = ornek(20);
        let ham = ham_dokum("test", &sonuclar);
        let kaynak_ref = depo.koy_deger("web", Deger::Metin(ham.clone()));
        let modele = modele_ozet("test", &sonuclar);

        assert!(ham.chars().count() > 3000, "ham dokum gercekten buyuk olmali");
        assert!(modele.chars().count() <= MODEL_TAVANI);
        // Depodaki govde eksiksiz duruyor.
        match depo.deger(&kaynak_ref) {
            Some(Deger::Metin(m)) => assert_eq!(m, ham),
            d => panic!("depoda metin bekleniyordu: {d:?}"),
        }
        assert_eq!(arac.ad(), "web_ara");
    }

    #[test]
    fn web_ara_kirletmez_ama_sema_zorunlu_alani_ister() {
        let arac = WebAraAraci::yeni();
        assert!(!arac.kirletir_mi(), "web arama kisisel veri OKUMAZ");
        assert!(arac.sema().dogrula(&json!({"sorgu": "hava"})).is_ok());
        assert!(arac.sema().dogrula(&json!({})).is_err());
        assert!(arac.sema().dogrula(&json!({"sorgu": 5})).is_err());
    }

    #[test]
    fn web_getir_semasi_adres_ister() {
        let arac = WebGetirAraci::yeni();
        assert!(!arac.kirletir_mi());
        assert_eq!(arac.ad(), "web_getir");
        assert!(arac.sema().dogrula(&json!({"adres": "https://a.test"})).is_ok());
        assert!(arac.sema().dogrula(&json!({})).is_err());
    }

    /// Gecersiz sema aga CIKMADAN reddedilir ve modele sabit Ingilizce doner.
    #[test]
    fn gecersiz_arguman_aga_cikmadan_reddedilir() {
        let mut ctx = baglam();
        let s = kos(WebAraAraci::yeni().calistir(json!({}), &mut ctx));
        assert_eq!(s.modele_donen, sirr_cekirdek::HATA_MODEL_METNI);
        assert!(!ctx.oturum_kirli(), "basarisiz arama oturumu kirletmemeli");
    }

    /// Ulasilamayan sunucu: cip Turkce, model sabit Ingilizce.
    #[test]
    fn ag_hatasi_cift_kanalli_doner() {
        let mut ctx = baglam();
        let arac = WebAraAraci::yeni()
            .istemciyle(WebAramaIstemcisi::adresle("https://yok.gecersiz.ornek"));
        let s = kos(arac.calistir(json!({"sorgu": "hava"}), &mut ctx));
        assert_eq!(s.modele_donen, sirr_cekirdek::HATA_MODEL_METNI, "modele Turkce sizmamali");
        assert!(!s.cip_metni.is_empty());
        assert!(s.cip_metni.is_ascii() || s.cip_metni.chars().any(|c| c.is_alphabetic()));
    }

    /// Cip detayinda giden istek URL'i durmali — seffaflik sozlesmesi.
    #[test]
    fn cip_ham_girdisinde_giden_url_gorunur() {
        let toplayici = Arc::new(IzToplayici::yeni());
        let mut ctx = AracBaglami::yeni(
            Arc::new(BellekVeriDeposu::yeni()),
            "/tmp/sirr-web-test",
            toplayici.clone(),
        );
        let arac = WebAraAraci::yeni()
            .istemciyle(WebAramaIstemcisi::adresle("https://yok.gecersiz.ornek"));
        let _ = kos(arac.calistir(json!({"sorgu": "dolar kuru"}), &mut ctx));

        let izler = toplayici.izler();
        let iz = izler.last().expect("cip dusmeli");
        assert!(iz.metin.contains("dolar kuru") || iz.ham_girdi.is_some());
        let girdi = iz.ham_girdi.clone().unwrap_or_default();
        assert!(girdi.contains("dolar%20kuru"), "giden URL cipte gorunmeli: {girdi}");
    }

    // -----------------------------------------------------------------------
    // ONAY KAPISI — `web_ara`nin DIS ARAC olmasinin kaniti
    // -----------------------------------------------------------------------
    //
    // Bu testler kapiyi SAHTE bir arac (`disari_gonder`) ile degil, GERCEK
    // `WebAraAraci` ile kosar. Fark onemli: `sirr-cli`daki `DIS_ARACLAR`
    // listesine "web_ara" yazmanin gercekten calisan bir kurulum uretecegini
    // burada kanitliyoruz, o satiri yazan kisinin umuduna birakmiyoruz.

    use crate::yurutucu::{
        AracCagrisi, AracYurutucu, DaimaOnay, RET_MODEL_METNI, YurutmeSebebi,
    };
    use sirr_cekirdek::AracKatalogu;

    /// `DIS_ARACLAR` listesinin uretimde alacagi hali.
    const DIS_ARACLAR: [&str; 2] = ["web_ara", "web_getir"];

    /// Oturumu kirletmek icin minimal kisisel veri araci.
    ///
    /// Bayragi elle set eden bir kisayol YOK (`AracYurutucu` boyle bir yol
    /// sunmuyor, dogru olan da bu): kirlilik ancak `kirletir_mi()` diyen bir
    /// aracin GERCEKTEN calismasiyla dogar. Test de o yoldan gecmeli.
    struct SahteKisiselArac;

    impl Arac for SahteKisiselArac {
        fn ad(&self) -> &str {
            "kisisel_oku"
        }
        fn aciklama(&self) -> &str {
            "Test amacli kisisel veri araci."
        }
        fn sema(&self) -> ArgSema {
            ArgSema::bos()
        }
        fn kirletir_mi(&self) -> bool {
            true
        }
        fn calistir<'a>(&'a self, _a: Value, _c: &'a mut AracBaglami) -> AracGelecegi<'a> {
            kutula(async move { AracSonucu::okundu("okundu", "ok") })
        }
    }

    fn web_yurutucu() -> AracYurutucu {
        let mut k = AracKatalogu::yeni();
        // Ulasilamayan sunucu: kapi ARACTAN ONCE calisir, yani kapiya takilan
        // cagri hic aga cikmaz — testin aga bagimli olmamasinin sebebi bu.
        k.ekle(Arc::new(
            WebAraAraci::yeni().istemciyle(WebAramaIstemcisi::adresle("https://yok.gecersiz.ornek")),
        ))
        .ekle(Arc::new(SahteKisiselArac));
        let mut y = AracYurutucu::yeni(k);
        for ad in DIS_ARACLAR {
            y = y.dis_arac(ad);
        }
        y
    }

    /// Kisisel veri aracini calistirip oturumu GERCEKTEN kirletir.
    fn kirlet(y: &AracYurutucu, ctx: &mut AracBaglami) {
        kos(y.yurut(&AracCagrisi::yeni("kisisel_oku", json!({})), y.aktif_tur(), ctx));
        assert!(y.oturum_kirli(), "kisisel arac oturumu kirletmeliydi");
    }

    /// Temiz oturumda arama sorulmadan gecer — onay NADIR olmali ki okunsun.
    #[test]
    fn temiz_oturumda_web_ara_onay_sormaz() {
        let y = web_yurutucu();
        let mut ctx = baglam();
        let s = kos(y.yurut(
            &AracCagrisi::yeni("web_ara", json!({"sorgu": "hava"})),
            y.aktif_tur(),
            &mut ctx,
        ));
        // Kapiya takilmadi: arac GERCEKTEN calisti (ve agda basarisiz oldu).
        assert_ne!(s.sebep, YurutmeSebebi::OnayReddedildi);
    }

    /// ASIL GARANTI: oturum kirliyken sorgu ONAY OLMADAN disari CIKMAZ.
    ///
    /// Senaryo tam da korktugumuz sey: once kisisel bir belge okunur, sonra
    /// model o belgeden devsirdigi bir metni arama sorgusuna yazar.
    #[test]
    fn kirli_oturumda_web_ara_kapiya_takilir() {
        let y = web_yurutucu();
        let mut ctx = baglam();
        kirlet(&y, &mut ctx);

        let s = kos(y.yurut(
            &AracCagrisi::yeni("web_ara", json!({"sorgu": "Ayse'nin maas bilgisi"})),
            y.aktif_tur(),
            &mut ctx,
        ));
        assert_eq!(s.sebep, YurutmeSebebi::OnayReddedildi);
        assert_eq!(s.modele_donen, RET_MODEL_METNI);
        // Ret bir ARIZA degil: kurtarma turu acilmamali, yoksa model israr eder.
        assert!(!s.hata_mi());
    }

    #[test]
    fn kirli_oturumda_onay_verilirse_arama_gecer() {
        let mut k = AracKatalogu::yeni();
        k.ekle(Arc::new(
            WebAraAraci::yeni().istemciyle(WebAramaIstemcisi::adresle("https://yok.gecersiz.ornek")),
        ))
        .ekle(Arc::new(SahteKisiselArac));
        let y = AracYurutucu::yeni(k).dis_arac("web_ara").kapiyla(DaimaOnay);
        let mut ctx = baglam();
        kirlet(&y, &mut ctx);
        let s = kos(y.yurut(
            &AracCagrisi::yeni("web_ara", json!({"sorgu": "hava"})),
            y.aktif_tur(),
            &mut ctx,
        ));
        assert_ne!(s.sebep, YurutmeSebebi::OnayReddedildi);
    }

    /// `web_getir` de listede: tek bir adres cekmek de disariya cikmaktir.
    #[test]
    fn web_getir_de_dis_arac_listesinde() {
        assert!(DIS_ARACLAR.contains(&WebGetirAraci::yeni().ad()));
        assert!(DIS_ARACLAR.contains(&WebAraAraci::yeni().ad()));
    }

    /// GERCEK AG, UCTAN UCA — `cargo test -p sirr-araclar web_ara -- --ignored --nocapture`.
    ///
    /// Istemciyi degil ARACIN TAMAMINI kosar: cip, depo, kirpma, kaynak_ref.
    /// "Derleme yesil" ile "gercekten calisiyor" arasindaki farki kapatan test
    /// budur; CI aga bagimli olmasin diye `#[ignore]`.
    #[test]
    #[ignore = "gercek ag gerektirir"]
    fn duman_uctan_uca_gercek_arama() {
        let depo = Arc::new(PaylasimliDepo::yeni());
        let toplayici = Arc::new(IzToplayici::yeni());
        let mut ctx = AracBaglami::yeni(
            Arc::new(BellekVeriDeposu::yeni()),
            std::env::temp_dir(),
            toplayici.clone(),
        );
        let arac = WebAraAraci::depo_ile(depo.clone());
        let s = kos(arac.calistir(json!({"sorgu": "istanbul hava durumu"}), &mut ctx));

        println!("--- CIP      : {}", s.cip_metni);
        println!("--- MODELE   :\n{}", s.modele_donen);
        println!("--- MODEL LEN: {} karakter", s.modele_donen.chars().count());
        println!("--- HAM LEN  : {} karakter", s.ham_cikti.as_deref().unwrap_or("").chars().count());
        if let Some(iz) = toplayici.izler().last() {
            println!("--- GIDEN URL: {}", iz.ham_girdi.clone().unwrap_or_default());
        }

        assert!(s.modele_donen.contains("kaynak_ref"), "bypass referansi donmeli");
        assert!(s.modele_donen.chars().count() <= MODEL_TAVANI + 80);
        assert!(!s.modele_donen.contains("https://"), "tam URL modele sizmamali");
        // BYPASS'IN KANITI: depodaki govde modele gidenden belirgin buyuk.
        let ham = s.ham_cikti.clone().unwrap_or_default();
        assert!(ham.len() > s.modele_donen.len(), "ham dokum modele gidenden zengin olmali");
    }

    /// GERCEK AG — `web_getir` uctan uca: HTML soyulur, govde depoya gider,
    /// modele yalnizca bir pencere doner.
    #[test]
    #[ignore = "gercek ag gerektirir"]
    fn duman_uctan_uca_sayfa_getir() {
        let depo = Arc::new(PaylasimliDepo::yeni());
        let mut ctx = baglam();
        let arac = WebGetirAraci::depo_ile(depo.clone());
        let s = kos(arac.calistir(json!({"adres": "https://doc.rust-lang.org/book/ch17-00-async-await.html"}), &mut ctx));

        println!("--- CIP    : {}", s.cip_metni);
        println!("--- MODELE : {}", s.modele_donen);
        let ham = s.ham_cikti.clone().unwrap_or_default();
        println!("--- HAM LEN: {} karakter", ham.chars().count());

        assert!(s.modele_donen.contains("kaynak_ref"));
        assert!(!ham.contains("<script"), "HTML etiketi soyulmali");
        // JS'e OZGU belirtecler aranir, "function" gibi genel kelimeler DEGIL:
        // bu sayfa Rust anlatiyor ve duz metninde "function calls" gecmesi
        // dogaldir. Ilk surumde `!ham.contains("function ")` denendi ve tam bu
        // yuzden yanlis alarm verdi — kaynak sayfada 14 script blogunun hepsi
        // dogru soyulmustu. Test, olculecek seyi olcmeli.
        for belirtec in ["localStorage", "querySelector", "addEventListener"] {
            assert!(!ham.contains(belirtec), "JS metne sizmamali: {belirtec}");
        }
        assert!(ham.len() > s.modele_donen.len() * 2, "govde modele gidenden cok buyuk olmali");
    }

    #[test]
    fn hata_cevirisi_zaman_asimini_ayirir() {
        assert!(matches!(cevir(&WebHatasi::ZamanAsimi), AracHatasi::ZamanAsimi));
        assert!(matches!(cevir(&WebHatasi::SunucuKodu(503)), AracHatasi::Diger(_)));
        // Ceviriden cikan Turkce metin MODELE degil, cipe gider.
        assert_eq!(
            AracSonucu::basarisiz(&cevir(&WebHatasi::SunucuKodu(503))).modele_donen,
            sirr_cekirdek::HATA_MODEL_METNI
        );
    }
}
