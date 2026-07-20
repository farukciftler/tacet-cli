//! `belge_duzenle` — var olan bir belgeyi yeni bir surum olarak yeniden yazar.
//!
//! AKIS: model once `belge_oku` ile icerigi alir, sonra TAM duzenlenmis icerigi
//! bu araca verir. Orijinal KORUNUR; "<ad> (duzenlendi)" adiyla yeni dosya
//! uretilir. Yazma → yesil cip.
//!
//! SWIFT DERSI — CALISILABILIR BELGE (atlanmasi kolay, pahaliya ogrenildi):
//! arac "sohbete EKLI belge"ye degil, "uzerinde CALISILABILIR belge"ye bakar.
//! Swift'te bu `calisilabilirBelge = ekli ?? sonUretilen` idi. Yalniz ekliye
//! bakildiginda "bana excel yap" → "onu tablo olarak goster" akisi ikinci
//! adimda "belge ekli degil" diyordu: kullanici az once uretilen dosyayi
//! kastediyordu, arac onu gormuyordu.
//!
//! RUST'TA KARSILIGI — NEDEN `AracBaglami`YA ALAN EKLENMEDI: `AracBaglami`
//! `sirr-cekirdek`tedir ve cekirdek TEK SAHIPLIDIR; ustelik "son belge" bir
//! sozlesme kavrami degil, bir UYGULAMA sezgisidir — cekirdege konsaydi her
//! arac onu tasimak zorunda kalirdi. Bunun yerine uc kademeli cozum var
//! (bkz. `calisilabilir_belge`): acik `yol` argumani → oturum izleyicisi
//! (`BelgeIzleyici`) → cikti klasorundeki EN SON degistirilmis belge. Ucuncu
//! kademe, kabuk hicbir sey baglamasa bile Swift'teki akisi BUGUN calistirir;
//! izleyici baglandiginda tahmin yerine olgu gecer.
//!
//! BICIM DONUSTURULMEZ. Swift'te olculen sessiz yalan: kullanici "bunu word'e
//! cevir" diyor, arac .xlsx'i .xlsx olarak yeniden yaziyor, model "Word'e
//! cevrildi" diyordu. Cozum kurali istemde tekrarlamak DEGIL, olguyu modele
//! bildirmek: donen metin bicimin DEGISMEDIGINI acikca soyler, model kendi
//! ciktisinda yalanla yuzlesir.
//!
//! TEK YAZMA KAPISI: yazma `belge_olustur::belge_yaz` serbest fonksiyonundan
//! gecer. Trait EZILMEZ — klasor hazirlama, benzersiz ad ve 0600 izin damgasi
//! bu aracin da uzerinden atlayamayacagi ortak adimlardir.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use sirr_cekirdek::{
    Alan, Arac, AracBaglami, AracDurumu, AracGelecegi, AracHatasi, AracSonuc, AracSonucu, ArgSema,
    IzGuncelleme, KaynakRef, kutula,
};

use crate::belge_oku::xlsx_coz;
use crate::belge_olustur::{
    BelgeBicimi, BelgeMotoru, CIKTI_KLASORU, ExcelMotor, MetinMotor, belge_yaz, markdown_dan,
};
use crate::veri_deposu::Tablo;

/// Duzenlenmis dosyanin ad eki. Orijinal ustune YAZILMAZ: kullanicinin elindeki
/// veriyi geri alinamaz bicimde degistirmek, modelin yanlis anladigi tek bir
/// istekte kalici kayip demektir.
const DUZENLENDI_EKI: &str = " (duzenlendi)";

/// Duzenlenebilir dosyanin ust siniri (32 MiB) — `belge_oku` ile ayni tavan.
const DOSYA_TAVANI: u64 = 32 * 1024 * 1024;

/// Cikti klasorunde "son belge" aranirken bakilan uzantilar.
const BELGE_UZANTILARI: &[&str] = &["xlsx", "md", "markdown", "txt", "text"];

// ---------------------------------------------------------------------------
// Oturum izleyicisi
// ---------------------------------------------------------------------------

/// Oturumda en son uretilen/okunan belgeyi tutar — Swift'teki `sonUretilen`in
/// karsiligi.
///
/// NEDEN AYRI BIR TIP: `AracSonucu::dosya_yolu` zaten her basarili belge
/// isleminde doludur, yani "son belge" bilgisi UYDURULMAZ, akan sonuclardan
/// TOPLANIR. Kabuk her tur sonunda `kaydet(&sonuc)` cagirdiginda izleyici
/// kendiliginde dolar; hicbir arac digerinin ic durumunu bilmek zorunda kalmaz.
///
/// Baglanmazsa arac tahmine (dosya zamani) duser ve yine calisir — bu bilincli:
/// baglanmamis bir izleyici yuzunden butun duzenleme akisinin olmesi, sessiz
/// bir entegrasyon hatasinin en pahali bicimi olurdu.
#[derive(Default)]
pub struct BelgeIzleyici {
    son: Mutex<Option<PathBuf>>,
}

impl BelgeIzleyici {
    pub fn yeni() -> Self {
        Self::default()
    }

    /// Bir arac sonucundan son belgeyi ogrenir. Yalniz BASARILI ve gercekten
    /// dosya ureten/okuyan sonuclar sayilir: basarisiz bir cagrinin tasidigi
    /// yol, var olmayan bir dosyayi "calisilabilir" gosterirdi.
    pub fn kaydet(&self, sonuc: &AracSonucu) {
        if matches!(sonuc.durum, AracDurumu::Basarisiz(_)) {
            return;
        }
        if let Some(yol) = &sonuc.dosya_yolu
            && yol.is_file()
        {
            *self.son.lock().expect("belge izleyici kilidi") = Some(yol.clone());
        }
    }

    /// Dogrudan bildirim — testler ve kabugun elle bagladigi yollar icin.
    pub fn ata(&self, yol: impl Into<PathBuf>) {
        *self.son.lock().expect("belge izleyici kilidi") = Some(yol.into());
    }

    pub fn son(&self) -> Option<PathBuf> {
        self.son.lock().expect("belge izleyici kilidi").clone()
    }

    /// Sohbet sifirlaninda cagrilir — oturum omurlu ne varsa burada biter.
    pub fn temizle(&self) {
        *self.son.lock().expect("belge izleyici kilidi") = None;
    }
}

// ---------------------------------------------------------------------------
// Calisilabilir belge cozumu
// ---------------------------------------------------------------------------

/// Uzerinde calisilacak belgenin nereden geldigi — cipte ve tanida gorunur,
/// ayrica testler hangi kademenin kazandigini dogrular.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BelgeKaynagi {
    /// Model acikca `yol` verdi.
    Arguman,
    /// Oturum izleyicisi biliyordu.
    Izleyici,
    /// Cikti klasorundeki en son degistirilmis belge (tahmin).
    SonDegisen,
}

/// Uc kademeli cozum. Sira ONEMLIDIR: acik istek tahmini daima yener.
fn calisilabilir_belge(
    ctx: &AracBaglami,
    izleyici: Option<&BelgeIzleyici>,
    acik_yol: Option<&str>,
) -> AracSonuc<(PathBuf, BelgeKaynagi)> {
    // 1) Acik yol — kum havuzu kapisindan gecer.
    if let Some(ham) = acik_yol {
        let yol = ctx.yolu_coz(ham)?;
        if !yol.is_file() {
            return Err(AracHatasi::DosyaYok(yol));
        }
        return Ok((yol, BelgeKaynagi::Arguman));
    }

    // 2) Oturum izleyicisi. Izleyicideki yol da kapidan GECIRILIR: izleyiciye
    // bir sekilde kum havuzu disi bir yol dustuyse burasi son savunmadir.
    if let Some(yol) = izleyici.and_then(BelgeIzleyici::son)
        && let Ok(cozulmus) = ctx.yolu_coz(&yol)
        && cozulmus.is_file()
    {
        return Ok((cozulmus, BelgeKaynagi::Izleyici));
    }

    // 3) Cikti klasorundeki en son degistirilmis belge.
    let klasor = ctx.yolu_coz(CIKTI_KLASORU)?;
    if let Some(yol) = son_degisen_belge(&klasor) {
        return Ok((yol, BelgeKaynagi::SonDegisen));
    }

    Err(AracHatasi::DosyaYok(klasor))
}

/// Klasordeki en son degistirilmis belgeyi bulur.
///
/// Esitlikte AD ile bozulur: iki dosya ayni milisaniyede yazildiginda dizin
/// gezinme sirasi (yani rastgelelik) karar veremez — ayni oturum iki kez
/// kosuldugunda ayni dosya secilmeli, aksi halde eval karsilastirilamaz.
fn son_degisen_belge(klasor: &Path) -> Option<PathBuf> {
    let mut en_iyi: Option<(std::time::SystemTime, PathBuf)> = None;
    for giris in fs::read_dir(klasor).ok()?.flatten() {
        let yol = giris.path();
        let Ok(ust) = yol.symlink_metadata() else { continue };
        if !ust.is_file() || ust.file_type().is_symlink() {
            continue;
        }
        let uzanti = yol
            .extension()
            .map(|u| u.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if !BELGE_UZANTILARI.contains(&uzanti.as_str()) {
            continue;
        }
        let Ok(zaman) = ust.modified() else { continue };
        en_iyi = match en_iyi {
            Some((z, y)) if (z, &y) >= (zaman, &yol) => Some((z, y)),
            _ => Some((zaman, yol)),
        };
    }
    en_iyi.map(|(_, y)| y)
}

/// Dosya uzantisindan bicim. Bilinmeyen uzanti duzenlenemez: "duzenledim"
/// deyip icerigi baska bir bicime dokmek, sessiz veri kaybidir.
fn bicimden(yol: &Path) -> Option<BelgeBicimi> {
    match yol.extension()?.to_string_lossy().to_ascii_lowercase().as_str() {
        "xlsx" => Some(BelgeBicimi::Excel),
        "md" | "markdown" => Some(BelgeBicimi::Markdown),
        "txt" | "text" => Some(BelgeBicimi::Metin),
        _ => None,
    }
}

fn motor(bicim: BelgeBicimi) -> Box<dyn BelgeMotoru> {
    match bicim {
        BelgeBicimi::Excel => Box::new(ExcelMotor::yeni()),
        b => Box::new(MetinMotor::yeni(b)),
    }
}

/// Duzenlenmis dosyanin taban adi. Zaten "(duzenlendi)" tasiyorsa ek TEKRAR
/// EKLENMEZ — ust uste uc duzenleme "rapor (duzenlendi) (duzenlendi)
/// (duzenlendi).xlsx" uretirdi.
fn duzenlenmis_ad(yol: &Path) -> String {
    let taban = yol
        .file_stem()
        .map(|a| a.to_string_lossy().into_owned())
        .unwrap_or_else(|| "belge".to_string());
    if taban.ends_with(DUZENLENDI_EKI) {
        taban
    } else {
        format!("{taban}{DUZENLENDI_EKI}")
    }
}

// ---------------------------------------------------------------------------
// Arac
// ---------------------------------------------------------------------------

/// `belge_duzenle` — calisilabilir belgeyi yeni bir surum olarak yeniden yazar.
#[derive(Default)]
pub struct BelgeDuzenleAraci {
    izleyici: Option<std::sync::Arc<BelgeIzleyici>>,
}

impl BelgeDuzenleAraci {
    pub fn yeni() -> Self {
        Self::default()
    }

    /// Oturum izleyicisiyle kurar — kabuk bunu tercih etmeli; izleyicisiz
    /// kurulum tahmin kademesine duser.
    pub fn izleyici_ile(izleyici: std::sync::Arc<BelgeIzleyici>) -> Self {
        Self { izleyici: Some(izleyici) }
    }
}

impl Arac for BelgeDuzenleAraci {
    fn ad(&self) -> &str {
        "belge_duzenle"
    }

    fn aciklama(&self) -> &str {
        "Edits the document in play — the file you just created or read — by writing a new \
         version of it. Call this when the user asks to change it ('add this row', 'delete \
         that line', 'change the title', in any language). First call belge_oku to get the \
         content, then pass the FULL edited content as 'yeni_icerik' (markdown; a markdown \
         table for Excel files). It never converts format: to change format call \
         belge_olustur with the new bicim."
    }

    fn sema(&self) -> ArgSema {
        ArgSema::nesne(vec![
            Alan::yeni(
                "yeni_icerik",
                ArgSema::metin().aciklama(
                    "The FULL edited content as markdown — the WHOLE document, not just the \
                     changed part; anything you leave out is deleted. For an Excel document \
                     write a markdown table (| ... |).",
                ),
            ),
            // `yeni_icerik` SEMADA ZORUNLU DEGIL ama arac govdesinde ZORUNLUDUR
            // (ikisinden BIRI gelmeli). Gerekce: gramer zorunlu alanlari
            // uretime zorlar, yani `zorunlu()` isaretlenseydi model her cagrida
            // govdeyi yazmak ZORUNDA kalirdi ve `kaynak_ref` yolu — yani bypass
            // kanalinin ta kendisi — kullanilamaz olurdu. `belge_olustur`da
            // `icerik`/`kaynak_ref` cifti icin ayni karar verildi; iki arac ayni
            // kalibi gostersin ki model tek bir sozdizimi ogrensin.
            Alan::yeni("baslik", ArgSema::metin().aciklama("Optional new title.")),
            Alan::yeni(
                "yol",
                ArgSema::metin().aciklama(
                    "Optional path of the document to edit, relative to the working directory. \
                     Leave empty to edit the document currently in play.",
                ),
            ),
            Alan::yeni(
                "kaynak_ref",
                ArgSema::metin().aciklama(
                    "Data reference returned by another tool. If given, the new content is \
                     pulled from the store — leave 'yeni_icerik' empty.",
                ),
            ),
        ])
        .aciklama("Rewrite the document in play with edited content")
    }

    /// Kullanicinin belgesi okunur ve yeni surumu diske yazilir: oturum kirlenir.
    fn kirletir_mi(&self) -> bool {
        true
    }

    fn calistir<'a>(
        &'a self,
        args: serde_json::Value,
        ctx: &'a mut AracBaglami,
    ) -> AracGelecegi<'a> {
        kutula(async move {
            if let Err(h) = self.sema().dogrula(&args) {
                return AracSonucu::basarisiz(&h);
            }
            let iz = ctx.cip_baslat("doc", "Belge duzenleniyor…");

            let (sonuc, yazildi) = match self.duzenle(&args, ctx) {
                Ok(s) => {
                    let yazildi = s.durum == AracDurumu::Yazildi;
                    (s, yazildi)
                }
                Err(h) => (AracSonucu::basarisiz(&h), false),
            };

            ctx.cip_guncelle(
                iz,
                IzGuncelleme::durum(sonuc.durum.clone())
                    .metin(sonuc.cip_metni.clone())
                    .ham_girdi(args.to_string())
                    .ham_cikti(sonuc.ham_cikti.clone().unwrap_or_default()),
            );

            if yazildi {
                ctx.kirlet();
                // Yeni surum artik CALISILABILIR belgedir: ard arda "bir satir
                // daha ekle" istekleri hep en son surumun uzerine binmeli.
                if let Some(iz) = &self.izleyici {
                    iz.kaydet(&sonuc);
                }
            }
            sonuc
        })
    }
}

impl BelgeDuzenleAraci {
    /// Senkron govde — hata yolu tek noktada toplanir.
    fn duzenle(
        &self,
        args: &serde_json::Value,
        ctx: &AracBaglami,
    ) -> AracSonuc<AracSonucu> {
        let metin_alan = |ad: &str| -> Option<String> {
            args.get(ad)
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };

        // --- Calisilabilir belge ---
        let acik_yol = metin_alan("yol");
        let cozum = calisilabilir_belge(ctx, self.izleyici.as_deref(), acik_yol.as_deref());
        let (yol, kaynak) = match cozum {
            Ok(v) => v,
            Err(AracHatasi::DosyaYok(_)) if acik_yol.is_none() => {
                // BELGE YOK bir HATA DEGIL, bir OLGUDUR. Sabit `tool_failed`
                // metnine cevirseydik model "arac bozuldu" ile "ortada belge
                // yok" arasini ayirt edemez ve kullaniciya yanlis sey soylerdi.
                return Ok(AracSonucu::okundu(
                    "Duzenlenecek belge yok",
                    "no_document_in_play (ask the user which file to edit, or create one first)",
                ));
            }
            Err(h) => return Err(h),
        };

        let Some(bicim) = bicimden(&yol) else {
            return Err(AracHatasi::GecersizArguman(format!(
                "duzenlenemeyen bicim: {}",
                yol.extension().map(|u| u.to_string_lossy().into_owned()).unwrap_or_default()
            )));
        };

        let ust = yol.metadata().map_err(|_| AracHatasi::DosyaYok(yol.clone()))?;
        if ust.len() > DOSYA_TAVANI {
            return Err(AracHatasi::Diger(format!(
                "belge cok buyuk ({} bayt), tavan {DOSYA_TAVANI}",
                ust.len()
            )));
        }

        // --- Yeni icerik: bypass kanali ya da dogrudan arguman ---
        //
        // `kaynak_ref` VARSA BAGLAYICIDIR (belge_olustur ile ayni kural):
        // cozulemeyen ref sessizce `yeni_icerik`e DUSMEZ. Swift'teki en pahali
        // hata sinifi buydu — ref cozulmeyince bos govdeyle yazilip
        // "duzenlendi" raporlaniyordu, yani kullanicinin belgesi BOSALIYORDU.
        let yeni_icerik = match metin_alan("kaynak_ref") {
            Some(r) => {
                let Some(kayit) = ctx.depodan(&KaynakRef(r.clone())) else {
                    let mut sonuc =
                        AracSonucu::basarisiz(&AracHatasi::Diger(format!("bilinmeyen kaynak_ref: {r}")));
                    sonuc.cip_metni = "Kaynak veri bulunamadi".to_string();
                    sonuc.modele_donen = format!("unknown_data_ref: \"{r}\"; nothing was edited");
                    return Ok(sonuc);
                };
                kayit.govde
            }
            None => metin_alan("yeni_icerik")
                .ok_or_else(|| AracHatasi::EksikAlan("yeni_icerik".into()))?,
        };

        // Bos icerikle "duzenleme" belgeyi SILMEKtir; model bunu kazara yapar.
        if yeni_icerik.trim().is_empty() {
            return Err(AracHatasi::GecersizArguman(
                "yeni icerik bos: duzenleme belgeyi bosaltamaz".into(),
            ));
        }

        // Excel'de ORIJINAL SUTUN SAYISI bir dogruluk sinyalidir: model tablonun
        // yarisini unutup tek sutunluk bir liste yollarsa dosya "duzenlendi"
        // gorunur ama veri kaybi olur. Okuma ucuz (dosya zaten elimizde) ve
        // uyari modele olgu olarak gider.
        let mut uyari = String::new();
        let tablo: Option<Tablo> = if bicim.tablo_yapisi() {
            let onceki = fs::read(&yol).ok().and_then(|b| xlsx_coz(&b).ok());
            let yeni = markdown_dan(&yeni_icerik).filter(|t| !t.satirlar.is_empty());
            if let (Some(o), Some(y)) = (&onceki, &yeni)
                && o.sutun_sayisi() > 0
                && y.sutun_sayisi() < o.sutun_sayisi()
            {
                uyari = format!(
                    "; warning: the new table has {} columns, the original had {}",
                    y.sutun_sayisi(),
                    o.sutun_sayisi()
                );
            }
            yeni
        } else {
            None
        };

        // Excel isteniyor ama tablo cikmadiysa `belge_yaz` icindeki
        // `tabloya_indir` kapisi devreye girer: borulu ama ayristirilamayan
        // govde SESSIZCE tek sutuna dokulmez, acik hata doner.
        let govde = if tablo.is_some() { None } else { Some(yeni_icerik.as_str()) };

        let klasor = ctx.yolu_coz(CIKTI_KLASORU)?;
        let ad = duzenlenmis_ad(&yol);
        let yeni_yol = belge_yaz(
            motor(bicim).as_ref(),
            &ad,
            metin_alan("baslik").as_deref(),
            govde,
            tablo.as_ref(),
            &klasor,
        )?;

        let yeni_ad = yeni_yol
            .file_name()
            .map(|a| a.to_string_lossy().into_owned())
            .unwrap_or_else(|| ad.clone());
        let cip = format!("{} duzenlendi: {yeni_ad}", bicim.etiket());

        // BICIM ACIKCA SOYLENIR: model "word'e cevirdim" diyemesin diye kurali
        // istemde tekrarlamak yerine OLGUYU donduruyoruz.
        Ok(AracSonucu::yazildi(
            cip,
            format!(
                "file_edited: {yeni_ad} (source: {}, format unchanged: {}; this tool never \
                 converts format — to change format call belge_olustur with the new bicim){uyari}",
                match kaynak {
                    BelgeKaynagi::Arguman => "given path",
                    BelgeKaynagi::Izleyici => "document in play",
                    BelgeKaynagi::SonDegisen => "most recent document",
                },
                bicim.uzanti(),
            ),
        )
        .ham_cikti(yeni_yol.display().to_string())
        .dosya_yolu(yeni_yol))
    }
}

// ---------------------------------------------------------------------------
// Testler
// ---------------------------------------------------------------------------

#[cfg(test)]
mod testler {
    use super::*;
    use crate::veri_deposu::PaylasimliDepo;
    use serde_json::json;
    use sirr_cekirdek::{SessizRaporlayici, VeriDeposu as CekirdekVeriDeposu};
    use std::sync::Arc;

    fn beklet<F: std::future::Future>(gelecek: F) -> F::Output {
        use std::pin::pin;
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        static VTABLE: RawWakerVTable = RawWakerVTable::new(
            |_| RawWaker::new(std::ptr::null(), &VTABLE),
            |_| {},
            |_| {},
            |_| {},
        );
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
        let mut cx = Context::from_waker(&waker);
        let mut gelecek = pin!(gelecek);
        loop {
            if let Poll::Ready(cikti) = gelecek.as_mut().poll(&mut cx) {
                return cikti;
            }
        }
    }

    fn gecici_dizin(etiket: &str) -> PathBuf {
        let yol = std::env::temp_dir().join(format!(
            "sirr-duzenle-{etiket}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&yol).expect("gecici dizin");
        yol
    }

    fn baglam(kok: &Path, depo: Arc<PaylasimliDepo>) -> AracBaglami {
        AracBaglami::yeni(depo, kok, Arc::new(SessizRaporlayici))
    }

    /// Cikti klasorune bir xlsx uretir (gercek `belge_yaz` yolundan).
    fn xlsx_uret(kok: &Path, ad: &str, tablo: Tablo) -> PathBuf {
        let klasor = kok.join(CIKTI_KLASORU);
        belge_yaz(&ExcelMotor::yeni(), ad, None, None, Some(&tablo), &klasor).expect("xlsx")
    }

    fn md_uret(kok: &Path, ad: &str, govde: &str) -> PathBuf {
        let klasor = kok.join(CIKTI_KLASORU);
        belge_yaz(&MetinMotor::yeni(BelgeBicimi::Markdown), ad, None, Some(govde), None, &klasor)
            .expect("md")
    }

    /// 1) SWIFT DERSININ TA KENDISI: kullanici yol vermez, izleyici de yoktur;
    ///    arac "belge yok" DEMEZ, cikti klasorundeki son belgeyi bulur.
    #[test]
    fn izleyicisiz_akis_son_uretilen_belgeyi_bulur() {
        let kok = gecici_dizin("son-uretilen");
        let onceki = xlsx_uret(
            &kok,
            "aylik-rapor",
            Tablo::yeni(["Ay", "Tutar"], vec![vec!["Ocak".into(), "100".into()]]),
        );
        assert!(onceki.exists());

        let mut ctx = baglam(&kok, Arc::new(PaylasimliDepo::yeni()));
        let sonuc = beklet(BelgeDuzenleAraci::yeni().calistir(
            json!({"yeni_icerik": "| Ay | Tutar |\n| --- | --- |\n| Ocak | 100 |\n| Subat | 250 |"}),
            &mut ctx,
        ));

        assert_eq!(sonuc.durum, AracDurumu::Yazildi, "{}", sonuc.cip_metni);
        assert!(sonuc.modele_donen.contains("file_edited"), "{}", sonuc.modele_donen);
        let yeni = sonuc.dosya_yolu.expect("dosya yolu");
        assert_eq!(yeni.extension().unwrap(), "xlsx", "bicim korunmali");
        assert!(yeni.file_name().unwrap().to_string_lossy().contains("(duzenlendi)"));
        // Orijinal DURUYOR: duzenleme geri alinamaz kayip olmamali.
        assert!(onceki.exists(), "orijinal silinmis");

        let bayt = fs::read(&yeni).unwrap();
        let girisler = sirr_zip::ac_harita(&bayt).expect("zip");
        let sheet = String::from_utf8(girisler["xl/worksheets/sheet1.xml"].clone()).unwrap();
        assert!(sheet.contains("Subat"), "yeni satir dosyaya girmemis");
        fs::remove_dir_all(&kok).ok();
    }

    /// 2) Izleyici bagliysa TAHMIN yerine OLGU kazanir: klasorde daha yeni
    ///    baska bir belge olsa bile izleyicinin dedigi duzenlenir.
    #[test]
    fn izleyici_tahmini_yener() {
        let kok = gecici_dizin("izleyici");
        let hedef = md_uret(&kok, "hedef", "eski govde");
        // Daha SONRA yazilan baska bir belge — tahmin kademesi bunu secerdi.
        let _daha_yeni = md_uret(&kok, "zzz-daha-yeni", "alakasiz");

        let izleyici = Arc::new(BelgeIzleyici::yeni());
        izleyici.ata(&hedef);
        let arac = BelgeDuzenleAraci::izleyici_ile(izleyici.clone());
        let mut ctx = baglam(&kok, Arc::new(PaylasimliDepo::yeni()));

        let sonuc = beklet(arac.calistir(json!({"yeni_icerik": "yeni govde"}), &mut ctx));
        assert_eq!(sonuc.durum, AracDurumu::Yazildi);
        let yeni = sonuc.dosya_yolu.clone().expect("yol");
        assert!(
            yeni.file_name().unwrap().to_string_lossy().starts_with("hedef"),
            "izleyici yerine tahmin kazanmis: {}",
            yeni.display()
        );
        assert!(sonuc.modele_donen.contains("document in play"));
        // Yeni surum artik calisilabilir belge olmali (zincirleme duzenleme).
        assert_eq!(izleyici.son().as_deref(), Some(yeni.as_path()));
        fs::remove_dir_all(&kok).ok();
    }

    /// 3) BICIM DONUSTURULMEZ ve bu modele ACIKCA soylenir (Swift'teki sessiz
    ///    yalanin kilidi).
    #[test]
    fn bicim_donusturulmez_ve_modele_soylenir() {
        let kok = gecici_dizin("bicim");
        let hedef = xlsx_uret(&kok, "tablo", Tablo::yeni(["A"], vec![vec!["1".into()]]));
        let izleyici = Arc::new(BelgeIzleyici::yeni());
        izleyici.ata(&hedef);
        let arac = BelgeDuzenleAraci::izleyici_ile(izleyici);
        let mut ctx = baglam(&kok, Arc::new(PaylasimliDepo::yeni()));

        let sonuc = beklet(arac.calistir(
            json!({"yeni_icerik": "| A |\n| --- |\n| 1 |\n| 2 |"}),
            &mut ctx,
        ));
        assert_eq!(sonuc.dosya_yolu.as_ref().unwrap().extension().unwrap(), "xlsx");
        assert!(
            sonuc.modele_donen.contains("format unchanged: xlsx"),
            "{}",
            sonuc.modele_donen
        );
        assert!(sonuc.modele_donen.contains("never converts format"));
        fs::remove_dir_all(&kok).ok();
    }

    /// 4) Belge yoksa bu bir HATA degil OLGUdur — modele sabit `tool_failed`
    ///    gitmemeli, yoksa "arac bozuk" ile "belge yok" ayrilamaz.
    #[test]
    fn belge_yoksa_olgu_doner_hata_degil() {
        let kok = gecici_dizin("bos");
        let mut ctx = baglam(&kok, Arc::new(PaylasimliDepo::yeni()));
        let sonuc =
            beklet(BelgeDuzenleAraci::yeni().calistir(json!({"yeni_icerik": "x"}), &mut ctx));

        assert_eq!(sonuc.durum, AracDurumu::Okundu);
        assert!(sonuc.modele_donen.starts_with("no_document_in_play"));
        assert_ne!(sonuc.modele_donen, sirr_cekirdek::HATA_MODEL_METNI);
        assert!(!ctx.oturum_kirli(), "hicbir belge okunmadi");
        fs::remove_dir_all(&kok).ok();
    }

    /// 5) Kum havuzu: acik `yol` argumaniyla disari CIKILAMAZ; cozulemeyen
    ///    `kaynak_ref` hicbir dosya YAZMAZ (belgenin bosalmasi en pahali hata).
    #[test]
    fn kum_havuzu_ve_cozulemeyen_ref_kapilari() {
        let kok = gecici_dizin("kapilar");
        let hedef = md_uret(&kok, "notlar", "govde");
        let izleyici = Arc::new(BelgeIzleyici::yeni());
        izleyici.ata(&hedef);
        let arac = BelgeDuzenleAraci::izleyici_ile(izleyici);
        let mut ctx = baglam(&kok, Arc::new(PaylasimliDepo::yeni()));

        for kacak in ["../disari.md", "/etc/hosts", "alt/../../kacak.md"] {
            let s = beklet(arac.calistir(json!({"yeni_icerik": "x", "yol": kacak}), &mut ctx));
            assert!(matches!(s.durum, AracDurumu::Basarisiz(_)), "kacak gecti: {kacak}");
            assert_eq!(s.modele_donen, sirr_cekirdek::HATA_MODEL_METNI);
        }

        let onceki_sayi = fs::read_dir(kok.join(CIKTI_KLASORU)).unwrap().count();
        let s = beklet(arac.calistir(json!({"kaynak_ref": "belge#404"}), &mut ctx));
        assert!(matches!(s.durum, AracDurumu::Basarisiz(_)));
        assert!(s.modele_donen.contains("unknown_data_ref"));
        assert!(s.modele_donen.contains("nothing was edited"));
        assert_eq!(
            fs::read_dir(kok.join(CIKTI_KLASORU)).unwrap().count(),
            onceki_sayi,
            "cozulemeyen ref dosya yazmis"
        );
        fs::remove_dir_all(&kok).ok();
    }

    /// 6) Bypass kanali: yeni icerik depodan gelir, model govdeyi HIC tasimaz.
    #[test]
    fn kaynak_ref_ile_duzenleme_bypass_kanalindan_gecer() {
        let kok = gecici_dizin("bypass");
        let hedef = xlsx_uret(&kok, "buyuk", Tablo::yeni(["Ad", "Tutar"], vec![vec!["x".into(), "1".into()]]));
        let izleyici = Arc::new(BelgeIzleyici::yeni());
        izleyici.ata(&hedef);
        let arac = BelgeDuzenleAraci::izleyici_ile(izleyici);

        let depo = Arc::new(PaylasimliDepo::yeni());
        let buyuk = Tablo::yeni(
            ["Ad", "Tutar"],
            (0..400).map(|i| vec![format!("kisi{i}"), format!("{i}")]).collect::<Vec<_>>(),
        );
        let r = CekirdekVeriDeposu::koy(&*depo, "belge", "400 satir", buyuk.markdown_kirpik(usize::MAX));
        let mut ctx = baglam(&kok, depo);

        let sonuc = beklet(arac.calistir(json!({"kaynak_ref": r.as_str()}), &mut ctx));
        assert_eq!(sonuc.durum, AracDurumu::Yazildi, "{}", sonuc.cip_metni);
        // 400 satir modelden GECMEDI.
        assert!(sonuc.modele_donen.len() < 260, "{}", sonuc.modele_donen);
        assert!(!sonuc.modele_donen.contains("kisi399"));
        // Ama dosya DOLU.
        let bayt = fs::read(sonuc.dosya_yolu.unwrap()).unwrap();
        let sheet = String::from_utf8(
            sirr_zip::ac_harita(&bayt).unwrap()["xl/worksheets/sheet1.xml"].clone(),
        )
        .unwrap();
        assert!(sheet.contains("kisi399"), "veri dosyaya gecmemis");
        fs::remove_dir_all(&kok).ok();
    }

    /// 7) Bos icerikle duzenleme belgeyi BOSALTAMAZ; sutun kaybi modele UYARI
    ///    olarak bildirilir (sessiz veri kaybi yok).
    #[test]
    fn bos_icerik_reddedilir_sutun_kaybi_bildirilir() {
        let kok = gecici_dizin("kayip");
        let hedef = xlsx_uret(
            &kok,
            "genis",
            Tablo::yeni(["Ad", "Tutar", "Not"], vec![vec!["a".into(), "1".into(), "n".into()]]),
        );
        let izleyici = Arc::new(BelgeIzleyici::yeni());
        izleyici.ata(&hedef);
        let arac = BelgeDuzenleAraci::izleyici_ile(izleyici);
        let mut ctx = baglam(&kok, Arc::new(PaylasimliDepo::yeni()));

        let bos = beklet(arac.calistir(json!({"yeni_icerik": "   "}), &mut ctx));
        assert!(matches!(bos.durum, AracDurumu::Basarisiz(_)), "bos icerik gecti");

        let dar = beklet(arac.calistir(json!({"yeni_icerik": "| Ad |\n| --- |\n| a |"}), &mut ctx));
        assert_eq!(dar.durum, AracDurumu::Yazildi);
        assert!(dar.modele_donen.contains("warning"), "{}", dar.modele_donen);
        assert!(dar.modele_donen.contains("3"), "{}", dar.modele_donen);
        fs::remove_dir_all(&kok).ok();
    }

    /// 8) Ad eki ust uste BIRIKMEZ ve dosya izni 0600 damgasini tasir
    ///    (tek yazma kapisi ezilmemis).
    #[test]
    fn ad_eki_birikmez_ve_izin_damgasi_durur() {
        assert_eq!(duzenlenmis_ad(Path::new("/a/rapor.xlsx")), "rapor (duzenlendi)");
        assert_eq!(
            duzenlenmis_ad(Path::new("/a/rapor (duzenlendi).xlsx")),
            "rapor (duzenlendi)",
            "ek birikmemeli"
        );

        let kok = gecici_dizin("izin");
        let hedef = md_uret(&kok, "not", "eski");
        let izleyici = Arc::new(BelgeIzleyici::yeni());
        izleyici.ata(&hedef);
        let arac = BelgeDuzenleAraci::izleyici_ile(izleyici);
        let mut ctx = baglam(&kok, Arc::new(PaylasimliDepo::yeni()));
        let sonuc = beklet(arac.calistir(json!({"yeni_icerik": "yeni"}), &mut ctx));
        let yeni = sonuc.dosya_yolu.expect("yol");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mod_ = fs::metadata(&yeni).unwrap().permissions().mode() & 0o777;
            assert_eq!(mod_, 0o600, "izin damgasi dusmus: {mod_:o}");
        }
        assert_eq!(fs::read_to_string(&yeni).unwrap().trim(), "yeni");
        fs::remove_dir_all(&kok).ok();
    }

    /// 9) Izleyici yalniz BASARILI ve gercekten var olan dosyalari kaydeder.
    #[test]
    fn izleyici_basarisiz_sonucu_kaydetmez() {
        let kok = gecici_dizin("izleyici-kapi");
        let gercek = md_uret(&kok, "gercek", "x");
        let iz = BelgeIzleyici::yeni();

        iz.kaydet(&AracSonucu::basarisiz(&AracHatasi::DosyaYok(gercek.clone())));
        assert!(iz.son().is_none(), "basarisiz sonuc kaydedilmis");

        iz.kaydet(&AracSonucu::yazildi("x", "y").dosya_yolu(kok.join("yok.md")));
        assert!(iz.son().is_none(), "var olmayan dosya kaydedilmis");

        iz.kaydet(&AracSonucu::yazildi("x", "y").dosya_yolu(gercek.clone()));
        assert_eq!(iz.son().as_deref(), Some(gercek.as_path()));

        iz.temizle();
        assert!(iz.son().is_none());
        fs::remove_dir_all(&kok).ok();
    }

    /// 10) Sema sozlesmesi ve yonlendirici tetikleyicileri.
    #[test]
    fn sema_ve_yonlendirici_sozlesmesi() {
        let js = BelgeDuzenleAraci::yeni().sema().json_schema();
        // `yeni_icerik` SEMADA zorunlu DEGIL: zorunlu olsaydi gramer modeli her
        // cagrida govde yazmaya zorlar ve `kaynak_ref` (bypass kanali) yolu
        // olurdu. Zorunluluk arac govdesinde, "ikisinden biri" kurali olarak.
        assert_eq!(js["required"], json!([]));
        assert_eq!(js["additionalProperties"], json!(false));

        let sema = BelgeDuzenleAraci::yeni().sema();
        assert!(sema.dogrula(&json!({"yeni_icerik": "x"})).is_ok());
        assert!(sema.dogrula(&json!({"kaynak_ref": "belge#1"})).is_ok());
        assert!(sema.dogrula(&json!({"yeni_icerik": 1})).is_err());

        // Ikisi de yoksa ARAC reddeder (sema degil): kapinin nerede oldugu
        // testte yaziyor ki ileride semaya tasinirsa bu test uyarsin.
        let kok = gecici_dizin("sema-kapi");
        md_uret(&kok, "x", "govde");
        let mut ctx = baglam(&kok, Arc::new(PaylasimliDepo::yeni()));
        let bos = beklet(BelgeDuzenleAraci::yeni().calistir(json!({}), &mut ctx));
        assert!(matches!(bos.durum, AracDurumu::Basarisiz(_)));
        fs::remove_dir_all(&kok).ok();

        // Yonlendirici Belge profilinden bu araci gorebilmeli: ad hem "belge"
        // hem "duzenle" ipucunu tasiyor.
        use crate::yonlendirici::{Yonlendirici, sadelestir};
        let metin = sadelestir(BelgeDuzenleAraci::yeni().ad());
        assert!(metin.contains("belge") && metin.contains("duzenle"));

        let mut katalog = sirr_cekirdek::AracKatalogu::yeni();
        katalog.ekle(Arc::new(crate::hesap::HesapAraci));
        katalog.ekle(Arc::new(BelgeDuzenleAraci::yeni()));
        let secilen = Yonlendirici::yeni().en_fazla(1).sec("tabloya bir satir ekle", &katalog);
        assert_eq!(secilen[0].ad(), "belge_duzenle");
    }
}
