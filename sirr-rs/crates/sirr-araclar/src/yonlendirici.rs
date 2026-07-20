//! Yonlendirici — oturum basina arac butcesi.
//!
//! KARAR (Swift tarafindan devralindi): modele bir oturumda EN FAZLA 8 arac
//! gosterilir. Nedeni olcum: kucuk bir model 20 aracin arasindan dogru olani
//! secemiyor; secim hatasi arac sayisiyla birlikte hizla buyuyor. Katalog
//! buyudukce cozum "modeli daha cok zorlamak" degil, ona daha az secenek
//! vermek. Secimi kullanici gormez — sessizce niyet profilinden turer.
//!
//! PUANLAMA — UZUNLUK TOPLAMI, ADET DEGIL. Swift'te ogrenilen ders: adet
//! sayilirsa "tablo" ile "tablo olarak" ayni agirligi tasir, beraberlikte
//! sirayi HashMap gezinme sirasi (yani rastgelelik) belirler ve ayni mesaj iki
//! calistirmada farkli arac kumesi uretir. Eslesen tetikleyicilerin karakter
//! uzunlugunu toplayinca ozgul ifade genel kelimeyi dogal olarak yener ve
//! sonuc belirlenimci olur — eval karsilastirilabilir kalir.

use sirr_cekirdek::{Arac, AracKatalogu};
use std::sync::Arc;

/// Bir oturumda modele gosterilecek en fazla arac sayisi.
pub const EN_FAZLA_ARAC: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NiyetProfili {
    Belge,
    Zaman,
    Hesap,
    Genel,
}

impl NiyetProfili {
    pub const HEPSI: [NiyetProfili; 4] =
        [NiyetProfili::Belge, NiyetProfili::Zaman, NiyetProfili::Hesap, NiyetProfili::Genel];

    pub fn ad(&self) -> &'static str {
        match self {
            NiyetProfili::Belge => "belge",
            NiyetProfili::Zaman => "zaman",
            NiyetProfili::Hesap => "hesap",
            NiyetProfili::Genel => "genel",
        }
    }

    /// Kullanici mesajinda aranan ifadeler.
    ///
    /// Ozgul ifadeler ("tablo olarak") genel kelimelerle ("tablo") ayni listede
    /// durur; ayirmaya gerek yok, uzunluk toplami siralamayi zaten kuruyor.
    fn mesaj_tetikleyicileri(&self) -> &'static [&'static str] {
        match self {
            NiyetProfili::Belge => &[
                "belge",
                "docx",
                "word",
                "xlsx",
                "excel",
                "sunum",
                "pptx",
                "slayt",
                "pdf",
                "rapor",
                "tablo",
                "tablo olarak",
                "tablo halinde",
                "belgeye ekle",
                "belge olustur",
                "dosyaya yaz",
                "bicimlendir",
                "basliklandir",
            ],
            NiyetProfili::Zaman => &[
                "takvim",
                "etkinlik",
                "toplanti",
                "randevu",
                "hatirlatici",
                "saat kacta",
                "bugun",
                "yarin",
                "bu hafta",
                "gelecek hafta",
                "tarih",
                "program",
                "ajanda",
            ],
            NiyetProfili::Hesap => &[
                "hesapla",
                "topla",
                "carp",
                "yuzde",
                "ortalama",
                "formul",
                "kod calistir",
                "python",
                "betik",
                "matematik",
                "kac eder",
                "toplam tutar",
            ],
            NiyetProfili::Genel => &[
                "dosya",
                "klasor",
                "dizin",
                "ara",
                "bul",
                "listele",
                "oku",
                "not",
                "hatirla",
                "ozetle",
            ],
        }
    }

    /// Bir aracin bu profile ait olup olmadigini anlamak icin arac adinda ve
    /// aciklamasinda aranan ipuclari.
    ///
    /// Arac adlari elimizde sabit degil (katalog calisma aninda dolar), o yuzden
    /// esleme isimle degil metinle yapilir: yeni bir arac eklendiginde
    /// yonlendiriciyi guncellemek gerekmesin.
    fn arac_ipuclari(&self) -> &'static [&'static str] {
        match self {
            NiyetProfili::Belge => {
                &["belge", "docx", "xlsx", "pptx", "pdf", "tablo", "rapor", "sunum", "duzenle"]
            }
            NiyetProfili::Zaman => {
                &["takvim", "etkinlik", "toplanti", "randevu", "tarih", "saat", "hatirlatici"]
            }
            NiyetProfili::Hesap => &["hesap", "kod", "python", "calistir", "formul", "matematik"],
            NiyetProfili::Genel => &["dosya", "dizin", "ara", "oku", "yaz", "liste", "hafiza"],
        }
    }
}

/// Bir mesajdan cikan profil puanlari; en yuksek puanli profil baskin niyettir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NiyetPuanlari {
    puanlar: Vec<(NiyetProfili, usize)>,
}

impl NiyetPuanlari {
    pub fn puan(&self, profil: NiyetProfili) -> usize {
        self.puanlar.iter().find(|(p, _)| *p == profil).map(|(_, s)| *s).unwrap_or(0)
    }

    /// Baskin profil. Hicbir tetikleyici eslesmediyse `Genel` — bilinmeyen
    /// mesaji ozel bir profile zorlamak yanlis araci one cikarir.
    pub fn baskin(&self) -> NiyetProfili {
        self.puanlar
            .iter()
            .filter(|(_, s)| *s > 0)
            // max_by_key esitlikte SONUNCUYU secer; HEPSI sirasindaki ilk
            // profili kazandirmak icin ters cevriliyor — belirlenimcilik sarti.
            .rev()
            .max_by_key(|(_, s)| *s)
            .map(|(p, _)| *p)
            .unwrap_or(NiyetProfili::Genel)
    }

    pub fn tumu(&self) -> &[(NiyetProfili, usize)] {
        &self.puanlar
    }
}

/// Turkce karakterleri ASCII'ye katlar ve kucuk harfe cevirir.
///
/// NEDEN: kullanici "toplantı" da yazar "toplanti" da; tetikleyici listesini
/// iki bicimde tutmak listeyi ikiye katlar ve biri gunceldigi gibi otekini
/// unutturur. Tek normal bicime indirgemek tek dogruluk kaynagi birakir.
pub fn sadelestir(metin: &str) -> String {
    let mut s = String::with_capacity(metin.len());
    for c in metin.chars() {
        let d = match c {
            'ı' | 'İ' | 'I' => 'i',
            'ş' | 'Ş' => 's',
            'ğ' | 'Ğ' => 'g',
            'ü' | 'Ü' => 'u',
            'ö' | 'Ö' => 'o',
            'ç' | 'Ç' => 'c',
            'â' | 'Â' => 'a',
            'î' | 'Î' => 'i',
            'û' | 'Û' => 'u',
            _ => c,
        };
        s.extend(d.to_lowercase());
    }
    s
}

/// Mesaji puanlar. Puan = eslesen tetikleyicilerin karakter uzunluk toplami.
pub fn niyet_puanla(mesaj: &str) -> NiyetPuanlari {
    let sade = sadelestir(mesaj);
    let puanlar = NiyetProfili::HEPSI
        .iter()
        .map(|p| {
            let toplam = p
                .mesaj_tetikleyicileri()
                .iter()
                .filter(|t| sade.contains(**t))
                .map(|t| t.len())
                .sum();
            (*p, toplam)
        })
        .collect();
    NiyetPuanlari { puanlar }
}

/// Arac butcesini uygulayan yonlendirici.
///
/// Durumsuz: ayni mesaj + ayni katalog daima ayni listeyi verir.
#[derive(Debug, Clone, Copy, Default)]
pub struct Yonlendirici {
    en_fazla: Option<usize>,
}

impl Yonlendirici {
    pub fn yeni() -> Self {
        Self::default()
    }

    /// Butceyi daraltmak icin (eval senaryolari). `EN_FAZLA_ARAC`in uzerine
    /// CIKILAMAZ: butce mimari bir karar, cagri yerinin ayari degil.
    pub fn en_fazla(mut self, adet: usize) -> Self {
        self.en_fazla = Some(adet.min(EN_FAZLA_ARAC));
        self
    }

    fn butce(&self) -> usize {
        self.en_fazla.unwrap_or(EN_FAZLA_ARAC)
    }

    /// Mesaja gore oturumda modele gosterilecek araclari secer.
    ///
    /// Siralama: yuksek puan once; esitlikte katalog sirasi korunur (stabil
    /// siralama). Puani sifir olan araclar da butce dolana kadar eklenir —
    /// modele bos liste gitmesindense katalog basindaki genel araclar gitsin.
    pub fn sec(&self, mesaj: &str, katalog: &AracKatalogu) -> Vec<Arc<dyn Arac>> {
        let puanlar = niyet_puanla(mesaj);
        let mut sirali: Vec<(usize, usize, Arc<dyn Arac>)> = katalog
            .araclar()
            .iter()
            .enumerate()
            .map(|(i, a)| (self.arac_puani(a.as_ref(), &puanlar), i, a.clone()))
            .collect();

        // Anahtar (–puan, katalog sirasi): tamamen belirlenimci.
        sirali.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        sirali.into_iter().take(self.butce()).map(|(_, _, a)| a).collect()
    }

    /// Bir aracin puani: aracin ait oldugu her profil icin, o profilin mesajdan
    /// aldigi puan ile aracta eslesen ipucu uzunlugunun carpimi.
    ///
    /// Carpim toplama tercih edildi: mesajda hic zaman ifadesi yoksa takvim
    /// araci ipucu eslese bile 0 alir. Toplansaydi ilgisiz arac, sirf adi
    /// uzun diye listeye tirmanirdi.
    fn arac_puani(&self, arac: &dyn Arac, puanlar: &NiyetPuanlari) -> usize {
        let metin = sadelestir(&format!("{} {}", arac.ad(), arac.aciklama()));
        NiyetProfili::HEPSI
            .iter()
            .map(|p| {
                let mesaj_puani = puanlar.puan(*p);
                if mesaj_puani == 0 {
                    return 0;
                }
                let ipucu: usize = p
                    .arac_ipuclari()
                    .iter()
                    .filter(|t| metin.contains(**t))
                    .map(|t| t.len())
                    .sum();
                mesaj_puani * ipucu
            })
            .sum()
    }
}

#[cfg(test)]
mod testler {
    use super::*;
    use serde_json::Value;
    use sirr_cekirdek::{
        Arac, AracBaglami, AracGelecegi, AracSonucu, ArgSema, BellekVeriDeposu,
        SessizRaporlayici, kutula,
    };

    struct SahteArac {
        ad: &'static str,
        aciklama: &'static str,
    }

    impl Arac for SahteArac {
        fn ad(&self) -> &str {
            self.ad
        }
        fn aciklama(&self) -> &str {
            self.aciklama
        }
        fn sema(&self) -> ArgSema {
            ArgSema::bos()
        }
        fn calistir<'a>(&'a self, _args: Value, _ctx: &'a mut AracBaglami) -> AracGelecegi<'a> {
            kutula(async move { AracSonucu::okundu("test", "ok") })
        }
    }

    fn arac(ad: &'static str, aciklama: &'static str) -> Arc<dyn Arac> {
        Arc::new(SahteArac { ad, aciklama })
    }

    fn katalog() -> AracKatalogu {
        [
            arac("dosya_oku", "bir dosyayi oku"),
            arac("dizin_listele", "dizin icerigini listele"),
            arac("belge_duzenle", "docx belge duzenle ve tablo ekle"),
            arac("sunum_olustur", "pptx sunum olustur"),
            arac("takvim_oku", "takvim etkinlik ve toplanti listesi"),
            arac("hatirlatici_kur", "hatirlatici ve randevu kur"),
            arac("kod_calistir", "python kod calistir, hesap yap"),
            arac("hafiza_yaz", "kalici hafiza notu yaz"),
            arac("tablo_uret", "xlsx tablo uret ve rapor bicimlendir"),
            arac("metin_ara", "metin icinde ara ve bul"),
        ]
        .into_iter()
        .collect()
    }

    // Kullanilmayan baglam kurulumunun derlendigini de gormek isteriz.
    #[test]
    fn baglam_ile_arac_calisir() {
        let ctx = AracBaglami::yeni(
            Arc::new(BellekVeriDeposu::yeni()),
            ".",
            Arc::new(SessizRaporlayici),
        );
        assert!(!ctx.oturum_kirli());
    }

    #[test]
    fn en_fazla_sekiz_arac_doner() {
        let k = katalog();
        assert_eq!(k.araclar().len(), 10);
        let secim = Yonlendirici::yeni().sec("bana bir belge hazirla", &k);
        assert_eq!(secim.len(), EN_FAZLA_ARAC);
    }

    #[test]
    fn ozgul_ifade_genel_kelimeyi_yener() {
        // "tablo" tek basina: belge profili "tablo" (5) alir.
        let tek = niyet_puanla("tablo");
        // "tablo olarak" hem "tablo" hem "tablo olarak" eslesir -> daha yuksek.
        let ozgul = niyet_puanla("tablo olarak ver");
        assert!(
            ozgul.puan(NiyetProfili::Belge) > tek.puan(NiyetProfili::Belge),
            "{:?} vs {:?}",
            ozgul.puan(NiyetProfili::Belge),
            tek.puan(NiyetProfili::Belge)
        );
    }

    #[test]
    fn puanlama_uzunluk_toplamidir_adet_degil() {
        // Tek uzun tetikleyici ("hatirlatici", 11) iki kisa tetikleyiciden
        // ("ara" 3 + "bul" 3) daha yuksek puan almali.
        let uzun = niyet_puanla("hatirlatici");
        let kisa = niyet_puanla("ara bul");
        assert_eq!(uzun.puan(NiyetProfili::Zaman), "hatirlatici".len());
        assert!(uzun.puan(NiyetProfili::Zaman) > kisa.puan(NiyetProfili::Genel));
    }

    #[test]
    fn turkce_karakterler_ayni_puani_verir() {
        assert_eq!(
            niyet_puanla("yarınki toplantıyı göster").puan(NiyetProfili::Zaman),
            niyet_puanla("yarinki toplantiyi goster").puan(NiyetProfili::Zaman)
        );
        assert!(niyet_puanla("TOPLANTI").puan(NiyetProfili::Zaman) > 0);
    }

    #[test]
    fn belge_niyeti_belge_araclarini_one_alir() {
        let secim = Yonlendirici::yeni().sec("bu veriyi docx belgeye tablo olarak ekle", &katalog());
        let adlar: Vec<&str> = secim.iter().map(|a| a.ad()).collect();
        assert!(
            adlar[0] == "belge_duzenle" || adlar[0] == "tablo_uret",
            "belge araclari basta olmali: {adlar:?}"
        );
        assert!(adlar.contains(&"belge_duzenle"));
    }

    #[test]
    fn zaman_niyeti_takvim_araclarini_one_alir() {
        let secim = Yonlendirici::yeni().sec("yarinki toplanti ve randevu var mi", &katalog());
        let adlar: Vec<&str> = secim.iter().map(|a| a.ad()).collect();
        assert!(adlar[..2].contains(&"takvim_oku"), "{adlar:?}");
        assert!(adlar[..2].contains(&"hatirlatici_kur"), "{adlar:?}");
    }

    #[test]
    fn hesap_niyeti_kod_aracini_one_alir() {
        let secim = Yonlendirici::yeni().sec("su yuzde hesapla, python ile calistir", &katalog());
        assert_eq!(secim[0].ad(), "kod_calistir");
    }

    #[test]
    fn alakasiz_mesaj_katalog_sirasini_korur() {
        let secim = Yonlendirici::yeni().sec("merhaba nasilsin", &katalog());
        let adlar: Vec<&str> = secim.iter().map(|a| a.ad()).collect();
        assert_eq!(adlar[0], "dosya_oku");
        assert_eq!(adlar[1], "dizin_listele");
        assert_eq!(adlar.len(), EN_FAZLA_ARAC);
    }

    #[test]
    fn secim_belirlenimcidir() {
        let k = katalog();
        let y = Yonlendirici::yeni();
        let bir: Vec<String> =
            y.sec("belge tablo takvim", &k).iter().map(|a| a.ad().to_string()).collect();
        for _ in 0..20 {
            let tekrar: Vec<String> =
                y.sec("belge tablo takvim", &k).iter().map(|a| a.ad().to_string()).collect();
            assert_eq!(bir, tekrar);
        }
    }

    #[test]
    fn butce_daraltilabilir_ama_asilamaz() {
        let k = katalog();
        assert_eq!(Yonlendirici::yeni().en_fazla(3).sec("belge", &k).len(), 3);
        // 50 istense bile tavan 8.
        assert_eq!(Yonlendirici::yeni().en_fazla(50).sec("belge", &k).len(), EN_FAZLA_ARAC);
    }

    #[test]
    fn kucuk_katalog_oldugu_gibi_doner() {
        let k: AracKatalogu = [arac("dosya_oku", "oku")].into_iter().collect();
        let secim = Yonlendirici::yeni().sec("takvimde ne var", &k);
        assert_eq!(secim.len(), 1);
    }

    #[test]
    fn baskin_profil_dogru_secilir() {
        assert_eq!(niyet_puanla("docx belge olustur").baskin(), NiyetProfili::Belge);
        assert_eq!(niyet_puanla("yarinki toplanti").baskin(), NiyetProfili::Zaman);
        assert_eq!(niyet_puanla("yuzde hesapla").baskin(), NiyetProfili::Hesap);
        // Hicbir tetikleyici yoksa Genel'e duser.
        assert_eq!(niyet_puanla("zzz qqq").baskin(), NiyetProfili::Genel);
    }

    #[test]
    fn ilgisiz_profil_araci_puan_almaz() {
        // Mesajda zaman ifadesi yok; takvim araci carpim kurali geregi 0 alir
        // ve puanli belge araclarinin ONUNE gecemez.
        let secim = Yonlendirici::yeni().sec("docx belge rapor bicimlendir", &katalog());
        let adlar: Vec<&str> = secim.iter().map(|a| a.ad()).collect();
        let belge = adlar.iter().position(|a| *a == "belge_duzenle").unwrap();
        let takvim = adlar.iter().position(|a| *a == "takvim_oku").unwrap();
        assert!(belge < takvim, "{adlar:?}");
    }
}
