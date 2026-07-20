//! `HafizaDeposu` — notlarin sahibi: disk bicimi, filtreler, hatirlama ve
//! butceli enjeksiyon.
//!
//! ## DISK BICIMI: neden duz JSON, neden SQLite degil
//!
//! `~/.sirr/hafiza.json`, tek dosya, tam yaz. Gerekce:
//!
//! - **Sifir-bagimlilik kimligi.** SQLite ya bir C kutuphanesi (`rusqlite`) ya
//!   da 30 bin satirlik saf-Rust bir motor demek. OOXML zip'i el ile yazmis bir
//!   projeye 50 notluk bir liste icin veritabani motoru cekmek tutarsiz olurdu.
//! - **Veri boyutu SABIT ve KUCUK.** Tavan 50 not x 160 karakter ~ 10 KB. Bu
//!   olcekte indeks, sorgu planlayici ve artimli yazmanin faydasi yok; zaten
//!   her turda TAMAMI belege yuklenip taraniyor.
//! - **Denetlenebilirlik markanın parcasi.** "Cihazindan cikmayan hafiza"
//!   diyorsak kullanici ne bilindigini `cat` ile gorebilmeli. Opak bir ikili
//!   dosya bu vaadi zayiflatirdi.
//! - **Bozulma davranisi.** Yarim yazilmis JSON okunamaz ve depo BOS acilir
//!   (panik yok). Yarim yazilmis bir veritabani dosyasi ise onarim gerektirir.
//!
//! Takas durustce: es zamanli iki yazici son yazani kazandirir. Tek kullanicili
//! cihaz-ustu bir asistanda bu senaryo yok; olsaydi karar degisirdi.

use crate::not::{
    EN_AZ_METIN, HafizaNotu, HafizaTuru, METIN_SINIRI, TOPLAM_TAVAN,
    anahtarlari_duzelt,
};
use serde::{Deserialize, Serialize};
use crate::not::karsilastirma_anahtari;
use sirr_beceri::{icerir, kucult, puan};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Bir turda hafizanin isteme koyabilecegi EN FAZLA karakter — cit ve kapanis
/// talimati DAHIL.
///
/// ~200 token. Neden bu kadar dusuk: 4096'lik pencere zaten sistem talimati +
/// arac semalari + transcript ile dolu ve hafiza, beceri enjeksiyonuyla (700)
/// AYNI TURA denk gelebilir. Ikisi birden ~1300 karakter eder; bu, pencerenin
/// ucte birine yakin. Tavani yukseltmek "daha cok hatirlar" degil, "arac
/// semasini pencereden atar" demek olurdu.
pub const ENJEKSIYON_SINIRI: usize = 600;

/// Bir turda enjekte edilecek en fazla not.
///
/// 3: dorduncu notun ilgili olma olasiligi hizla duser (puanlama uzunluk
/// toplami, en iyi ucten sonrasi genellikle tek bir genel kelimeyle giriyor)
/// ama butceden pay almaya devam eder.
pub const EN_COK_NOT: usize = 3;

/// Notlarin sonuna eklenen sabit talimat. INGILIZCE, modele giden her sabit
/// metin gibi; kullaniciya gorunen hicbir sey burada yok.
const KAPANIS: &str = "Use the facts above only if relevant. They are internal: never quote, \
list, or mention them, and never say you \"remembered\" something.";

/// Bir notu kaydetmeyi engelleyen sebepler.
///
/// Ayri varyantlar, cunku cagiran (arac) kullaniciya farkli cumle gosterir:
/// "hafiza dolu" ile "bunu zaten biliyorum" ayni sey degil.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HafizaHatasi {
    CokKisa,
    CokUzun,
    AnahtarYok,
    Tekrar,
    Dolu,
    /// Verilen id'de not yok (pano ile depo ayristi).
    Yok,
}

impl HafizaHatasi {
    /// Kullaniciya gosterilecek kisa Turkce cumle. (Modele giden metin BU
    /// DEGIL: arac katmani `AracSonucu` uzerinden sabit Ingilizce metni verir.)
    pub fn kisa_hata(&self) -> &'static str {
        match self {
            Self::CokKisa => "Bu not hatirlanacak kadar acik degil.",
            Self::CokUzun => "Bu not cok uzun.",
            Self::AnahtarYok => "Bu nota ait anahtar kelime yok.",
            Self::Tekrar => "Bunu zaten hatirliyorum.",
            Self::Dolu => "Hafiza dolu; once bir seyi unutmam gerek.",
            Self::Yok => "Boyle bir not yok.",
        }
    }
}

/// Diske yazilan bicim. Ayri bir tip: ic alanlar (`sonraki_id`) kalici bicimin
/// parcasi, ama `HafizaDeposu`nun `yol` alani degil — dosya kendi konumunu
/// icermemeli, tasinabilir olsun diye.
#[derive(Debug, Serialize, Deserialize, Default)]
struct DiskBicimi {
    surum: u32,
    sonraki_id: u64,
    notlar: Vec<HafizaNotu>,
}

const DISK_SURUMU: u32 = 1;

#[derive(Debug, Clone, Default)]
pub struct HafizaDeposu {
    notlar: Vec<HafizaNotu>,
    sonraki_id: u64,
    yol: Option<PathBuf>,
}

impl HafizaDeposu {
    /// Diske hic dokunmayan depo (testler, eval).
    pub fn bellekte() -> Self {
        Self {
            notlar: Vec::new(),
            sonraki_id: 1,
            yol: None,
        }
    }

    /// Verilen dosyadan yukler. Dosya yoksa ya da BOZUKSA bos depo doner ve
    /// PANIK YAPMAZ: okunamayan bir hafiza dosyasi yuzunden asistanin
    /// acilmamasi kabul edilemez. Bozuk dosya UZERINE YAZILMAZ da — ilk
    /// `kaydet`e kadar diskte durur, yani kullanici kurtarabilir.
    pub fn dosyadan(yol: impl Into<PathBuf>) -> Self {
        let yol = yol.into();
        let disk: DiskBicimi = std::fs::read_to_string(&yol)
            .ok()
            .and_then(|ham| serde_json::from_str(&ham).ok())
            .unwrap_or_default();

        // `sonraki_id` diskten geliyor ama guvenilmez (elle duzenlenmis olabilir);
        // en buyuk id'nin altinda kalirsa ID CAKISIRDI. Tabandan yukari cekiyoruz.
        let en_buyuk = disk.notlar.iter().map(|n| n.id).max().unwrap_or(0);
        Self {
            sonraki_id: disk.sonraki_id.max(en_buyuk + 1),
            notlar: disk.notlar,
            yol: Some(yol),
        }
    }

    /// `~/.sirr/hafiza.json` (ya da `SIRR_EV` ile yonlendirilen yer).
    pub fn varsayilan_yol() -> Option<PathBuf> {
        std::env::var_os("SIRR_EV")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".sirr")))
            .map(|ev| ev.join("hafiza.json"))
    }

    pub fn notlar(&self) -> &[HafizaNotu] {
        &self.notlar
    }

    pub fn sayi(&self) -> usize {
        self.notlar.len()
    }

    pub fn dolu_mu(&self) -> bool {
        self.notlar.len() >= TOPLAM_TAVAN
    }

    // -----------------------------------------------------------------------
    // Yazma yolu — filtreler KODDA, model ciktisina guvenilmez
    // -----------------------------------------------------------------------

    /// Not ekler. Filtreler sirayla: uzunluk, anahtar, tekillestirme, tavan.
    /// Herhangi biri duserse not KAYDEDILMEZ.
    ///
    /// Modele "iki notu birlestir" gorevi VERILMEZ — bu boyuttaki modelde bu
    /// veri kaybettirir; tekrar eden not sessizce reddedilir, o kadar.
    pub fn ekle(
        &mut self,
        metin: &str,
        tur: HafizaTuru,
        anahtarlar: &[String],
    ) -> Result<u64, HafizaHatasi> {
        // Tavan ONCE: dolu depoda dogrulama yapmanin anlami yok ve cagirana
        // gosterilecek dogru cumle "hafiza dolu"dur.
        if self.dolu_mu() {
            return Err(HafizaHatasi::Dolu);
        }

        let metin = metin.trim();
        let n = metin.chars().count();
        if n < EN_AZ_METIN {
            return Err(HafizaHatasi::CokKisa);
        }
        if n > METIN_SINIRI {
            return Err(HafizaHatasi::CokUzun);
        }

        let anahtarlar = anahtarlari_duzelt(anahtarlar);
        if anahtarlar.is_empty() {
            return Err(HafizaHatasi::AnahtarYok);
        }

        let normal = karsilastirma_anahtari(metin);
        if self.notlar.iter().any(|k| k.normal_metin() == normal) {
            return Err(HafizaHatasi::Tekrar);
        }

        let id = self.sonraki_id;
        self.sonraki_id += 1;
        self.notlar.push(HafizaNotu {
            id,
            metin: metin.to_string(),
            tur,
            anahtarlar,
            olusturulma: simdi(),
            aktif: true,
        });
        Ok(id)
    }

    /// Bir notu id ile siler. "Sunu unut" komutunun kesin yolu.
    pub fn sil(&mut self, id: u64) -> bool {
        let onceki = self.notlar.len();
        self.notlar.retain(|n| n.id != id);
        self.notlar.len() != onceki
    }

    /// Metinde ya da anahtarlarda gecen ifadeye gore siler; silinen adedi doner.
    ///
    /// Model "kahve sevdigimi unut" diyebilsin diye var. Eslesme `icerir` ile,
    /// yani beceri katmaniyla AYNI sozcuk kuralina uyar: "ara" gibi kisa bir
    /// ifadenin "aralik"i tasiyan notu silmesi, geri alinamaz bir veri kaybi
    /// olurdu — burada yanlis eslesmenin bedeli enjeksiyondakinden agir.
    pub fn unut(&mut self, ifade: &str) -> usize {
        let hedef = kucult(ifade.trim());
        if hedef.is_empty() {
            return 0;
        }
        let onceki = self.notlar.len();
        self.notlar.retain(|n| {
            let metin = n.normal_metin();
            !(icerir(&metin, &hedef) || n.anahtarlar.iter().any(|a| a == &hedef))
        });
        onceki - self.notlar.len()
    }

    /// Notun metnini/anahtarlarini gunceller (pano duzenleyicisi).
    pub fn guncelle(
        &mut self,
        id: u64,
        metin: &str,
        anahtarlar: &[String],
    ) -> Result<(), HafizaHatasi> {
        let metin = metin.trim();
        let n = metin.chars().count();
        if n < EN_AZ_METIN {
            return Err(HafizaHatasi::CokKisa);
        }
        if n > METIN_SINIRI {
            return Err(HafizaHatasi::CokUzun);
        }
        let anahtarlar = anahtarlari_duzelt(anahtarlar);
        if anahtarlar.is_empty() {
            return Err(HafizaHatasi::AnahtarYok);
        }
        let normal = karsilastirma_anahtari(metin);
        if self
            .notlar
            .iter()
            .any(|k| k.id != id && k.normal_metin() == normal)
        {
            return Err(HafizaHatasi::Tekrar);
        }
        match self.notlar.iter_mut().find(|n| n.id == id) {
            Some(not) => {
                not.metin = metin.to_string();
                not.anahtarlar = anahtarlar;
                Ok(())
            }
            None => Err(HafizaHatasi::Yok),
        }
    }

    /// Notu aktif/pasif yapar. Pasif not ENJEKTE EDILMEZ ama SILINMEZ:
    /// "kapat" ile "unut" ayri kararlardir.
    pub fn aktiflik(&mut self, id: u64, aktif: bool) -> bool {
        match self.notlar.iter_mut().find(|n| n.id == id) {
            Some(n) => {
                n.aktif = aktif;
                true
            }
            None => false,
        }
    }

    /// Tum notlari siler. Kullanicinin acik karari olmadan CAGRILMAZ.
    pub fn hepsini_sil(&mut self) -> usize {
        let n = self.notlar.len();
        self.notlar.clear();
        n
    }

    // -----------------------------------------------------------------------
    // Okuma yolu — MODEL DEVREDE DEGIL
    // -----------------------------------------------------------------------

    /// Mesaja uyan notlari puan sirasiyla doner (en fazla `EN_COK_NOT`).
    ///
    /// Puan, eslesen anahtarlarin UZUNLUK toplamidir — beceri katmaniyla AYNI
    /// kural, ayni uygulama. Hangi aninin hangi tura girecegine model degil
    /// deterministik kod karar verir; boylece ayni mesaj her zaman ayni notu
    /// getirir ve davranis sinanabilir kalir.
    pub fn eslesen(&self, mesaj: &str) -> Vec<&HafizaNotu> {
        let m = kucult(mesaj);
        let mut puanli: Vec<(&HafizaNotu, usize)> = self
            .notlar
            .iter()
            .filter(|n| n.aktif)
            .map(|n| (n, puan(&m, &n.anahtarlar)))
            .filter(|(_, p)| *p > 0)
            .collect();

        // Puan azalan; esitlikte YENI not once (taze bilgi daha degerli) —
        // sirali bir kural olmasa esitlikte sirayi Vec sirasi belirlerdi ve
        // "eski bir not yeniyi gizliyor" hatasi sessiz kalirdi.
        puanli.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.olusturulma.cmp(&a.0.olusturulma)));
        puanli.truncate(EN_COK_NOT);
        puanli.into_iter().map(|(n, _)| n).collect()
    }

    /// Bu mesaj icin isteme eklenecek metin; hic eslesme yoksa `None`.
    ///
    /// Ciktinin uzunlugu `ENJEKSIYON_SINIRI`yi ASLA asmaz: butceye sigmayan
    /// not eklenmez. Kirpma degil DUSURME, cunku yarim bir olgu ("Kullanicinin
    /// isi ogret") yanlis bir olgudur.
    pub fn enjeksiyon_metni(&self, mesaj: &str) -> Option<String> {
        self.enjeksiyondan(self.eslesen(mesaj))
    }

    /// Secilmis notlardan citli metni kurar. `NotIzleyici` ile suzulmus liste
    /// de buraya verilebilsin diye ayri.
    pub fn enjeksiyondan(&self, notlar: Vec<&HafizaNotu>) -> Option<String> {
        if notlar.is_empty() {
            return None;
        }
        // Sabit bolumler once olculur; notlara kalan gercek butce budur.
        let sabit = "<memory>\n</memory>\n".chars().count() + KAPANIS.chars().count();
        let mut butce = ENJEKSIYON_SINIRI.saturating_sub(sabit);

        let mut satirlar: Vec<String> = Vec::new();
        for n in notlar {
            let satir = format!("- {}\n", n.metin.trim());
            let uzunluk = satir.chars().count();
            if uzunluk > butce {
                continue; // sigmayan not DUSER; kirpilmis olgu yanlis olgudur
            }
            butce -= uzunluk;
            satirlar.push(satir);
        }
        if satirlar.is_empty() {
            return None;
        }
        Some(format!(
            "<memory>\n{}</memory>\n{}",
            satirlar.concat(),
            KAPANIS
        ))
    }

    // -----------------------------------------------------------------------
    // Kaliciklik
    // -----------------------------------------------------------------------

    /// Depoyu diske yazar. Bellek deposunda (`bellekte`) sessizce basarilidir.
    ///
    /// ONCE GECICI DOSYAYA, SONRA `rename`: dogrudan yazarken surec olurse
    /// dosya yarim kalir ve TUM hafiza gider. `rename` ayni dosya sisteminde
    /// atomiktir, yani en kotu ihtimalle eski surum durur.
    pub fn kaydet(&self) -> std::io::Result<()> {
        let Some(yol) = &self.yol else {
            return Ok(());
        };
        if let Some(dizin) = yol.parent() {
            std::fs::create_dir_all(dizin)?;
        }
        let disk = DiskBicimi {
            surum: DISK_SURUMU,
            sonraki_id: self.sonraki_id,
            notlar: self.notlar.clone(),
        };
        let metin = serde_json::to_string_pretty(&disk)
            .map_err(|h| std::io::Error::new(std::io::ErrorKind::InvalidData, h))?;

        let gecici = yol.with_extension("json.yeni");
        std::fs::write(&gecici, metin)?;
        izinleri_daralt(&gecici);
        std::fs::rename(&gecici, yol)
    }

    pub fn yol(&self) -> Option<&Path> {
        self.yol.as_deref()
    }
}

/// Hafiza dosyasi KISISEL VERIDIR: baska kullanicilar okuyamasin diye 0600.
/// Belge yazma yolundaki damganin aynisi. Unix disinda sessizce atlanir.
fn izinleri_daralt(yol: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(yol, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = yol;
}

fn simdi() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|s| s.as_secs())
        .unwrap_or(0)
}

/// Ayni notun ayni oturuma iki kez girmesini engelleyen izleyici.
///
/// Neden gerekli: hafiza her turda taranir ve "yemek" gecen her mesajda ayni
/// not yeniden girerdi. Bir kez soylenen olgu transcript'te zaten durur;
/// tekrari yalnizca butce yer ve modele o olguyu ONEMLI gosterir.
#[derive(Debug, Clone, Default)]
pub struct NotIzleyici {
    gorulen: HashSet<u64>,
}

impl NotIzleyici {
    pub fn yeni() -> Self {
        Self::default()
    }

    /// Daha once enjekte edilmis notlari eler; kalanlari GORULMUS isaretler.
    pub fn suz<'a>(&mut self, notlar: Vec<&'a HafizaNotu>) -> Vec<&'a HafizaNotu> {
        let mut kalan = Vec::new();
        for n in notlar {
            if self.gorulen.insert(n.id) {
                kalan.push(n);
            }
        }
        kalan
    }

    /// Yeni oturum = yeni baglam.
    pub fn sifirla(&mut self) {
        self.gorulen.clear();
    }
}

#[cfg(test)]
mod testler {
    use super::*;
    use crate::not::ANAHTAR_SINIRI;

    fn anahtar(liste: &[&str]) -> Vec<String> {
        liste.iter().map(|s| s.to_string()).collect()
    }

    fn dolu_depo() -> HafizaDeposu {
        let mut d = HafizaDeposu::bellekte();
        d.ekle(
            "Kullanici vejetaryendir.",
            HafizaTuru::Tercih,
            &anahtar(&["yemek", "restoran", "aksam yemegi"]),
        )
        .unwrap();
        d.ekle(
            "Kullanicinin isi ogretmenlik.",
            HafizaTuru::Kimlik,
            &anahtar(&["is", "okul", "meslek"]),
        )
        .unwrap();
        d
    }

    #[test]
    fn filtreler_kisa_uzun_ve_anahtarsiz_notu_duser() {
        let mut d = HafizaDeposu::bellekte();
        assert_eq!(d.ekle("kisa", HafizaTuru::Olgu, &anahtar(&["a"])), Err(HafizaHatasi::CokKisa));
        assert_eq!(
            d.ekle(&"a".repeat(METIN_SINIRI + 1), HafizaTuru::Olgu, &anahtar(&["a"])),
            Err(HafizaHatasi::CokUzun)
        );
        assert_eq!(
            d.ekle("Kullanici vejetaryendir.", HafizaTuru::Olgu, &anahtar(&["  ", ""])),
            Err(HafizaHatasi::AnahtarYok)
        );
        assert_eq!(d.sayi(), 0, "hicbiri kaydedilmemeli");
    }

    #[test]
    fn tekillestirme_ayni_olguyu_iki_kez_almaz() {
        let mut d = dolu_depo();
        let sonuc = d.ekle(
            "  KULLANICI VEJETARYENDIR.  ",
            HafizaTuru::Tercih,
            &anahtar(&["yemek"]),
        );
        assert_eq!(sonuc, Err(HafizaHatasi::Tekrar), "kucultme + kirpma sonrasi ayni");
        assert_eq!(d.sayi(), 2);
    }

    #[test]
    fn tavan_dolunca_yeni_not_alinmaz_ve_eski_dusurulmez() {
        let mut d = HafizaDeposu::bellekte();
        for i in 0..TOPLAM_TAVAN {
            d.ekle(&format!("Kullanici {i} numarali olguyu soyledi."), HafizaTuru::Olgu, &anahtar(&["olgu"]))
                .unwrap();
        }
        assert!(d.dolu_mu());
        assert_eq!(
            d.ekle("Kullanici baska bir sey soyledi.", HafizaTuru::Olgu, &anahtar(&["baska"])),
            Err(HafizaHatasi::Dolu)
        );
        // Sessiz dusurme YOK: eski notlar yerinde.
        assert_eq!(d.sayi(), TOPLAM_TAVAN);
    }

    #[test]
    fn eslesme_ozgul_anahtari_one_alir_ve_uc_notla_sinirli() {
        let mut d = HafizaDeposu::bellekte();
        for i in 0..6 {
            d.ekle(
                &format!("Kullanici {i}. genel olguyu soyledi."),
                HafizaTuru::Olgu,
                &anahtar(&["yemek"]),
            )
            .unwrap();
        }
        d.ekle(
            "Kullanici aksam yemegini gec yer.",
            HafizaTuru::Tercih,
            &anahtar(&["aksam yemegi"]),
        )
        .unwrap();

        let bulunan = d.eslesen("yemek icin ne onerirsin, aksam yemegi nerede");
        assert_eq!(bulunan.len(), EN_COK_NOT, "tavan uygulanmali");
        // "aksam yemegi" (12) + "yemek" (5) = 17 > 5: ozgul ifade genel kelimeyi yener.
        assert_eq!(bulunan[0].metin, "Kullanici aksam yemegini gec yer.");
    }

    #[test]
    fn kapali_not_enjekte_edilmez_ama_silinmez() {
        let mut d = dolu_depo();
        let id = d.notlar()[0].id;
        assert!(d.aktiflik(id, false));
        assert!(d.eslesen("restoran onerir misin").is_empty());
        assert_eq!(d.sayi(), 2, "kapatmak silmek degildir");
        d.aktiflik(id, true);
        assert_eq!(d.eslesen("restoran onerir misin").len(), 1);
    }

    #[test]
    fn eslesme_yoksa_enjeksiyon_yoktur() {
        let d = dolu_depo();
        assert!(d.enjeksiyon_metni("iki kere iki kac eder").is_none());
    }

    #[test]
    fn enjeksiyon_butceyi_asmaz_ve_sigmayan_not_duser() {
        let mut d = HafizaDeposu::bellekte();
        // Hepsi ayni anahtarla eslesen, sinira dayanmis uzun notlar.
        for i in 0..EN_COK_NOT {
            let metin = format!("{i} {}", "u".repeat(METIN_SINIRI - 2));
            d.ekle(&metin, HafizaTuru::Olgu, &anahtar(&["yemek"])).unwrap();
        }
        let metin = d.enjeksiyon_metni("yemek ne olsun").expect("eslesme var");
        assert!(
            metin.chars().count() <= ENJEKSIYON_SINIRI,
            "butce asildi: {}",
            metin.chars().count()
        );
        assert!(metin.starts_with("<memory>\n"));
        assert!(metin.contains("never say you \"remembered\""));
        // Sigmayan not KIRPILMADAN dusmeli: yarim olgu yanlis olgudur.
        for satir in metin.lines().filter(|s| s.starts_with("- ")) {
            assert!(satir.len() >= METIN_SINIRI, "not kirpilmis: {satir}");
        }
    }

    #[test]
    fn enjeksiyon_beceriyle_ayni_turda_pencereyi_bogmaz() {
        // En kotu durum: 700 karakterlik beceri + hafiza. Ikisi birden 4096
        // penceresinin ucte birini gecmemeli.
        let d = dolu_depo();
        let hafiza = d.enjeksiyon_metni("aksam yemegi nerede yenir").unwrap();
        let toplam = sirr_beceri::ENJEKSIYON_SINIRI + hafiza.chars().count();
        assert!(toplam <= 1400, "beceri + hafiza tavani: {toplam}");
    }

    #[test]
    fn unut_ifadeyi_tasiyan_notu_siler() {
        let mut d = dolu_depo();
        assert_eq!(d.unut("vejetaryen"), 1);
        assert_eq!(d.sayi(), 1);
        // Anahtar uzerinden de silinebilir.
        assert_eq!(d.unut("okul"), 1);
        assert_eq!(d.sayi(), 0);
        assert_eq!(d.unut(""), 0, "bos ifade her seyi silmemeli");
    }

    #[test]
    fn unut_sozcuk_kuralina_uyar() {
        let mut d = HafizaDeposu::bellekte();
        d.ekle("Kullanici aralik ayinda dogdu.", HafizaTuru::Olgu, &anahtar(&["dogum"]))
            .unwrap();
        // "ara" kisa kok: "aralik" icinde tutmamali, yoksa geri alinamaz kayip.
        assert_eq!(d.unut("ara"), 0);
        assert_eq!(d.sayi(), 1);
    }

    #[test]
    fn guncelleme_sinirlari_ve_tekrari_kontrol_eder() {
        let mut d = dolu_depo();
        let id = d.notlar()[0].id;
        assert!(d.guncelle(id, "Kullanici vegandir.", &anahtar(&["yemek"])).is_ok());
        assert_eq!(d.notlar()[0].metin, "Kullanici vegandir.");
        assert_eq!(d.guncelle(id, "kisa", &anahtar(&["yemek"])), Err(HafizaHatasi::CokKisa));
        // Baska notun metnine esitlenemez.
        assert_eq!(
            d.guncelle(id, "Kullanicinin isi ogretmenlik.", &anahtar(&["is"])),
            Err(HafizaHatasi::Tekrar)
        );
    }

    #[test]
    fn izleyici_ayni_notu_oturumda_ikinci_kez_vermez() {
        let d = dolu_depo();
        let mut iz = NotIzleyici::yeni();
        assert_eq!(iz.suz(d.eslesen("restoran")).len(), 1);
        assert!(iz.suz(d.eslesen("restoran")).is_empty(), "oturuma bir kez");
        iz.sifirla();
        assert_eq!(iz.suz(d.eslesen("restoran")).len(), 1);
    }

    #[test]
    fn diske_yazip_geri_okur_ve_id_cakismaz() {
        let dizin = std::env::temp_dir().join("sirr-hafiza-testi-1");
        let _ = std::fs::remove_dir_all(&dizin);
        let yol = dizin.join("hafiza.json");

        let mut d = HafizaDeposu::dosyadan(&yol);
        d.ekle("Kullanici vejetaryendir.", HafizaTuru::Tercih, &anahtar(&["yemek"]))
            .unwrap();
        d.ekle("Kullanicinin isi ogretmenlik.", HafizaTuru::Kimlik, &anahtar(&["is"]))
            .unwrap();
        d.kaydet().unwrap();

        let mut geri = HafizaDeposu::dosyadan(&yol);
        assert_eq!(geri.sayi(), 2);
        assert_eq!(geri.notlar()[0].tur, HafizaTuru::Tercih);
        // Yeni not eski id'lerle CAKISMAMALI.
        let yeni = geri
            .ekle("Kullanici Istanbul'da yasiyor.", HafizaTuru::Olgu, &anahtar(&["sehir"]))
            .unwrap();
        assert!(geri.notlar().iter().filter(|n| n.id == yeni).count() == 1);
        assert!(yeni > 2);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mod_ = std::fs::metadata(&yol).unwrap().permissions().mode() & 0o777;
            assert_eq!(mod_, 0o600, "hafiza kisisel veridir");
        }
        let _ = std::fs::remove_dir_all(&dizin);
    }

    #[test]
    fn bozuk_dosya_panik_yerine_bos_depo_verir() {
        let dizin = std::env::temp_dir().join("sirr-hafiza-testi-2");
        let _ = std::fs::remove_dir_all(&dizin);
        std::fs::create_dir_all(&dizin).unwrap();
        let yol = dizin.join("hafiza.json");
        std::fs::write(&yol, "{ bu json degil ]]").unwrap();

        let d = HafizaDeposu::dosyadan(&yol);
        assert_eq!(d.sayi(), 0);
        // Bozuk dosya UZERINE YAZILMAMIS olmali: kullanici kurtarabilsin.
        assert_eq!(std::fs::read_to_string(&yol).unwrap(), "{ bu json degil ]]");
        let _ = std::fs::remove_dir_all(&dizin);
    }

    #[test]
    fn bellek_deposu_diske_dokunmaz() {
        let mut d = HafizaDeposu::bellekte();
        d.ekle("Kullanici vejetaryendir.", HafizaTuru::Tercih, &anahtar(&["yemek"]))
            .unwrap();
        assert!(d.yol().is_none());
        assert!(d.kaydet().is_ok(), "yolsuz kaydet sessizce basarili");
    }

    #[test]
    fn hepsini_sil_ve_sil_calisir() {
        let mut d = dolu_depo();
        let id = d.notlar()[0].id;
        assert!(d.sil(id));
        assert!(!d.sil(id), "ikinci kez silinmez");
        assert_eq!(d.hepsini_sil(), 1);
        assert_eq!(d.sayi(), 0);
    }

    #[test]
    fn anahtar_siniri_kayitta_uygulanir() {
        let mut d = HafizaDeposu::bellekte();
        let cok: Vec<String> = (0..20).map(|i| format!("k{i}")).collect();
        let id = d.ekle("Kullanici bir sey soyledi.", HafizaTuru::Olgu, &cok).unwrap();
        let not = d.notlar().iter().find(|n| n.id == id).unwrap();
        assert_eq!(not.anahtarlar.len(), ANAHTAR_SINIRI);
    }
}
