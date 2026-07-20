//! Tipli veri deposu — 4096 token bypass kanalinin motoru.
//!
//! MIMARININ KALBI: toplu cihaz verisi modelden GECMEZ. Arac ham veriyi buraya
//! koyar, geriye kisa bir `KaynakRef` alir ve modele yalnizca "birkac satirlik
//! ozet + kaynak_ref" doner. Veriye gercekten ihtiyaci olan bir sonraki adim
//! yine bir ARACtir; o da referansi kullanip ham govdeye ulasir. Boylece 40
//! sayfalik bir belge ya da 500 satirlik bir tablo, baglam penceresini hic
//! zorlamadan islenir.
//!
//! NEDEN CEKIRDEKTEKI `VeriDeposu` YETMIYOR: cekirdek sozlesmesi govdeyi
//! `String` tutar — sozlesme katmani olarak dogru, cunku orada bicim bilgisi
//! olmamali. Ama araclarin tablo ile duz metni ayirt etmesi gerekiyor: bir
//! tablonun kirpilmis ozeti GECERLI markdown olmak zorunda (asagidaki nota
//! bak), duz metnin ozeti ise ilk n satir. Bu ayrimi burada, uygulama
//! katmaninda tipli olarak tasiyoruz.
//!
//! IKI KATMAN: `VeriDeposu` sahipligi net olan tipli depodur (`&mut self` ile
//! yazilir, `al` gercek referans doner — buyuk `Bayt` govdesi klonlanmaz).
//! `PaylasimliDepo` onu `Mutex` ile sarip cekirdegin `Arc<dyn ...>` bekleyen
//! sozlesmesine baglar. Tek tipte birlestirilseydi ya `al` klon donmek zorunda
//! kalirdi ya da sozlesme uyumu kaybolurdu.

use sirr_cekirdek::{Kayit, KaynakRef, VeriDeposu as CekirdekVeriDeposu};
use std::sync::Mutex;

/// Depoya konabilecek govde bicimleri.
///
/// Kapali bir kume: her yeni varyant bir de "ozeti nasil cikarilir" sorusunu
/// getirir, o yuzden bilerek dar tutuluyor.
#[derive(Debug, Clone, PartialEq)]
pub enum Deger {
    Metin(String),
    Tablo(Tablo),
    /// Ikili veri. Ozeti ASLA icerigi gostermez — sadece boyut. Baytin modele
    /// sizmasi hem anlamsiz hem de kisisel veri kacagi riski.
    Bayt(Vec<u8>),
}

impl Deger {
    /// `KaynakRef` uretiminde kullanilan kaba tur etiketi.
    pub fn tur(&self) -> &'static str {
        match self {
            Deger::Metin(_) => "metin",
            Deger::Tablo(_) => "tablo",
            Deger::Bayt(_) => "bayt",
        }
    }

    /// Modele gosterilmesi guvenli, tek satirlik boyut bilgisi.
    pub fn kisa_ozet(&self) -> String {
        match self {
            Deger::Metin(m) => format!("{} satir metin", m.lines().count()),
            Deger::Tablo(t) => {
                format!("{} satir x {} sutun tablo", t.satir_sayisi(), t.sutun_sayisi())
            }
            Deger::Bayt(b) => format!("{} bayt ikili veri", b.len()),
        }
    }
}

/// Basliklar + satirlar. Sutun sayisi `basliklar` ile tanimlidir; satirlar
/// kisa/uzun gelirse cikti uretilirken hizalanir (bkz. `markdown_kirpik`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tablo {
    pub basliklar: Vec<String>,
    pub satirlar: Vec<Vec<String>>,
}

impl Tablo {
    pub fn yeni(
        basliklar: impl IntoIterator<Item = impl Into<String>>,
        satirlar: impl IntoIterator<Item = Vec<String>>,
    ) -> Self {
        Self {
            basliklar: basliklar.into_iter().map(Into::into).collect(),
            satirlar: satirlar.into_iter().collect(),
        }
    }

    pub fn satir_sayisi(&self) -> usize {
        self.satirlar.len()
    }

    pub fn sutun_sayisi(&self) -> usize {
        self.basliklar.len()
    }

    /// Tabloyu en fazla `en_fazla_satir` veri satiriyla markdown'a cevirir.
    ///
    /// SWIFT'TE OGRENILEN DERS: kirpma sirasinda hizalama satiri dusurulur ya
    /// da sutun sayisi kayarsa cikti gecersiz markdown olur; model tabloyu
    /// yeniden kuramaz ve "tablo gosterildi" deyip icerigi atlar. Bu yuzden
    /// cikti HER ZAMAN gecerli: baslik satiri + hizalama satiri + her satirda
    /// tam olarak `sutun_sayisi` hucre. Kirpildi notu tablonun DISINA, bos bir
    /// satirdan sonra yazilir ki tablo blogunu bozmasin.
    pub fn markdown_kirpik(&self, en_fazla_satir: usize) -> String {
        if self.basliklar.is_empty() {
            return String::new();
        }
        let sutun = self.basliklar.len();
        let mut cikti = String::new();

        cikti.push_str(&satir_yaz(&self.basliklar, sutun));
        // Hizalama satiri: hucre sayisi baslikla birebir ayni olmali.
        let hizalama: Vec<String> = (0..sutun).map(|_| "---".to_string()).collect();
        cikti.push_str(&satir_yaz(&hizalama, sutun));

        let gosterilen = self.satirlar.len().min(en_fazla_satir);
        for satir in self.satirlar.iter().take(gosterilen) {
            cikti.push_str(&satir_yaz(satir, sutun));
        }

        let gizlenen = self.satirlar.len() - gosterilen;
        if gizlenen > 0 {
            cikti.push_str(&format!("\n(+{gizlenen} satir daha gosterilmedi)\n"));
        }
        cikti
    }
}

/// Bir satiri boru karakterli markdown satirina cevirir; hucre sayisini
/// `sutun`a sabitler (eksigi bos hucreyle doldurur, fazlasini atar).
fn satir_yaz(hucreler: &[String], sutun: usize) -> String {
    let mut s = String::from("|");
    for i in 0..sutun {
        let bos = String::new();
        let h = hucreler.get(i).unwrap_or(&bos);
        s.push(' ');
        s.push_str(&hucre_kacir(h));
        s.push_str(" |");
    }
    s.push('\n');
    s
}

/// Hucre icerigi tabloyu bozamasin: boru kacirilir, satir sonu bosluga cevrilir.
fn hucre_kacir(ham: &str) -> String {
    let mut s = String::with_capacity(ham.len());
    for c in ham.chars() {
        match c {
            '|' => s.push_str("\\|"),
            '\n' | '\r' => s.push(' '),
            _ => s.push(c),
        }
    }
    s
}

/// Tipli, oturum omurlu depo. Diske hicbir sey yazmaz; oturum bitince
/// `temizle()` ile bosalir.
#[derive(Default)]
pub struct VeriDeposu {
    kayitlar: Vec<(KaynakRef, String, Deger)>,
    sayac: u64,
}

impl VeriDeposu {
    pub fn yeni() -> Self {
        Self::default()
    }

    /// Veriyi saklar, referansini doner. Etiket `Deger` varyantindan turer.
    pub fn koy(&mut self, veri: Deger) -> KaynakRef {
        let tur = veri.tur().to_string();
        self.koy_etiketli(&tur, veri)
    }

    /// Cagiran kendi turunu vermek isterse ("takvim", "belge"): filtreleme ve
    /// kaynak_ref okunurlugu icin varyant adindan daha bilgilendirici olur.
    pub fn koy_etiketli(&mut self, tur: &str, veri: Deger) -> KaynakRef {
        self.sayac += 1;
        let kaynak_ref = KaynakRef(format!("{tur}#{}", self.sayac));
        self.kayitlar.push((kaynak_ref.clone(), tur.to_string(), veri));
        kaynak_ref
    }

    /// Ham govdeye referansla erisim — klon YOK, cunku govde megabaytlarca
    /// olabilir ve bu yol sicak yoldur.
    pub fn al(&self, kaynak_ref: &KaynakRef) -> Option<&Deger> {
        self.kayitlar.iter().find(|(r, _, _)| r == kaynak_ref).map(|(_, _, d)| d)
    }

    pub fn tur(&self, kaynak_ref: &KaynakRef) -> Option<&str> {
        self.kayitlar
            .iter()
            .find(|(r, _, _)| r == kaynak_ref)
            .map(|(_, t, _)| t.as_str())
    }

    pub fn turden(&self, tur: &str) -> Vec<&KaynakRef> {
        self.kayitlar
            .iter()
            .filter(|(_, t, _)| t == tur)
            .map(|(r, _, _)| r)
            .collect()
    }

    pub fn sayi(&self) -> usize {
        self.kayitlar.len()
    }

    /// Modele DONECEK metin. Bulunamayan referans icin de bir sey doner:
    /// modelin eline bos string gecerse uydurmaya baslar, acik bir "yok"
    /// cumlesi gecerse dogru davranir.
    pub fn ozet(&self, kaynak_ref: &KaynakRef, en_fazla_satir: usize) -> String {
        let Some(deger) = self.al(kaynak_ref) else {
            return format!("{kaynak_ref}: kayit bulunamadi");
        };
        match deger {
            Deger::Metin(m) => {
                let satirlar: Vec<&str> = m.lines().collect();
                let gosterilen = satirlar.len().min(en_fazla_satir);
                let mut s = satirlar[..gosterilen].join("\n");
                let gizlenen = satirlar.len() - gosterilen;
                if gizlenen > 0 {
                    if !s.is_empty() {
                        s.push('\n');
                    }
                    s.push_str(&format!("(+{gizlenen} satir daha gosterilmedi)"));
                }
                s
            }
            Deger::Tablo(t) => t.markdown_kirpik(en_fazla_satir),
            // Baytin icerigi hicbir kirpma seviyesinde modele gitmez.
            Deger::Bayt(b) => format!("{} bayt ikili veri", b.len()),
        }
    }

    /// Oturum sonu. Sayac SIFIRLANMAZ: elde kalan eski bir kaynak_ref yeni bir
    /// kayda denk gelip yanlis veriyi acmasin.
    pub fn temizle(&mut self) {
        self.kayitlar.clear();
    }
}

/// Cekirdek sozlesmesine baglayan paylasimli sarmalayici.
///
/// `AracBaglami` `Arc<dyn VeriDeposu>` tutar, yani tum yazma islemleri `&self`
/// uzerinden gelir; ic esitleme somut deponun isi (cekirdek karari 6 ile ayni
/// mantik). `Mutex` burada, tipli depoda degil — tipli depo tek sahipli
/// kullanildiginda kilit bedeli odemesin.
#[derive(Default)]
pub struct PaylasimliDepo {
    ic: Mutex<VeriDeposu>,
}

impl PaylasimliDepo {
    pub fn yeni() -> Self {
        Self::default()
    }

    /// Tipli API'ye kilitli erisim — araclar `Deger::Tablo` koymak istediginde
    /// cekirdek sozlesmesinin `String` govdesine dusmek zorunda kalmasin.
    pub fn koy_deger(&self, tur: &str, veri: Deger) -> KaynakRef {
        self.ic.lock().expect("veri deposu kilidi").koy_etiketli(tur, veri)
    }

    pub fn ozet(&self, kaynak_ref: &KaynakRef, en_fazla_satir: usize) -> String {
        self.ic.lock().expect("veri deposu kilidi").ozet(kaynak_ref, en_fazla_satir)
    }

    /// Klonlu erisim: kilit disina referans cikaramayiz, bu yolda klon kacinilmaz.
    pub fn deger(&self, kaynak_ref: &KaynakRef) -> Option<Deger> {
        self.ic.lock().expect("veri deposu kilidi").al(kaynak_ref).cloned()
    }
}

impl CekirdekVeriDeposu for PaylasimliDepo {
    fn koy(&self, tur: &str, _ozet: &str, govde: String) -> KaynakRef {
        // Sozlesme `String` govde verir; burada bicim bilgisi yok, Metin dogru
        // varsayilan. Tablo koymak isteyen `koy_deger` kullanir.
        self.ic.lock().expect("veri deposu kilidi").koy_etiketli(tur, Deger::Metin(govde))
    }

    fn al(&self, kaynak_ref: &KaynakRef) -> Option<Kayit> {
        let ic = self.ic.lock().expect("veri deposu kilidi");
        let deger = ic.al(kaynak_ref)?;
        let tur = ic.tur(kaynak_ref).unwrap_or("bilinmiyor").to_string();
        Some(Kayit {
            kaynak_ref: kaynak_ref.clone(),
            tur,
            ozet: deger.kisa_ozet(),
            govde: ic.ozet(kaynak_ref, usize::MAX),
        })
    }

    fn turden(&self, tur: &str) -> Vec<Kayit> {
        let refler: Vec<KaynakRef> = {
            let ic = self.ic.lock().expect("veri deposu kilidi");
            ic.turden(tur).into_iter().cloned().collect()
        };
        refler.iter().filter_map(|r| CekirdekVeriDeposu::al(self, r)).collect()
    }

    fn temizle(&self) {
        self.ic.lock().expect("veri deposu kilidi").temizle();
    }
}

#[cfg(test)]
mod testler {
    use super::*;

    fn ornek_tablo(satir: usize) -> Tablo {
        let satirlar = (0..satir)
            .map(|i| vec![format!("ad{i}"), format!("{i}"), "TR".to_string()])
            .collect::<Vec<_>>();
        Tablo::yeni(["Ad", "Sayi", "Ulke"], satirlar)
    }

    /// Bypass kanalinin ozu: 5000 satirlik govde depoda kalir, disari cikan
    /// ozet kucuk bir metindir.
    #[test]
    fn buyuk_veri_modelden_gecmeden_tasinir() {
        let mut depo = VeriDeposu::yeni();
        let buyuk: String = (0..5000).map(|i| format!("satir {i}\n")).collect();
        let uzunluk = buyuk.len();
        let r = depo.koy(Deger::Metin(buyuk));

        let ozet = depo.ozet(&r, 3);
        assert!(ozet.len() < 200, "ozet modele sigmali: {}", ozet.len());
        assert!(ozet.contains("(+4997 satir daha gosterilmedi)"));

        // Ham govde depoda, eksiksiz duruyor.
        match depo.al(&r).expect("kayit") {
            Deger::Metin(m) => assert_eq!(m.len(), uzunluk),
            _ => panic!("metin bekleniyordu"),
        }
    }

    #[test]
    fn kirpik_markdown_gecerli_kalir() {
        let t = ornek_tablo(100);
        let md = t.markdown_kirpik(4);
        let tablo_satirlari: Vec<&str> =
            md.lines().filter(|s| s.starts_with('|')).collect();
        // baslik + hizalama + 4 veri
        assert_eq!(tablo_satirlari.len(), 6);
        // Her satirda ayni sayida boru: sutun + 1
        for s in &tablo_satirlari {
            assert_eq!(s.matches('|').count(), 4, "bozuk satir: {s}");
        }
        assert!(tablo_satirlari[1].contains("---"));
        assert!(md.contains("(+96 satir daha gosterilmedi)"));
    }

    #[test]
    fn kirpma_gerekmeyince_not_yazilmaz() {
        let md = ornek_tablo(2).markdown_kirpik(10);
        assert!(!md.contains("gosterilmedi"));
        assert_eq!(md.lines().filter(|s| s.starts_with('|')).count(), 4);
    }

    #[test]
    fn dengesiz_satirlar_hizalanir() {
        let t = Tablo::yeni(
            ["A", "B", "C"],
            vec![vec!["yalniz".into()], vec!["1".into(), "2".into(), "3".into(), "4".into()]],
        );
        let md = t.markdown_kirpik(10);
        for s in md.lines().filter(|s| s.starts_with('|')) {
            assert_eq!(s.matches('|').count(), 4, "sutun kaymasi: {s}");
        }
    }

    #[test]
    fn hucredeki_boru_kacirilir() {
        let t = Tablo::yeni(["A", "B"], vec![vec!["x|y".into(), "z\nw".into()]]);
        let md = t.markdown_kirpik(5);
        assert!(md.contains("x\\|y"), "{md}");
        assert!(!md.contains("z\nw"));
        for s in md.lines().filter(|s| s.starts_with('|')) {
            // Kacirilan boru sutun sayisini bozmamali: \| sayilmadan 3 boru.
            assert_eq!(s.replace("\\|", "").matches('|').count(), 3, "{s}");
        }
    }

    #[test]
    fn tablo_ozeti_markdown_dondurur() {
        let mut depo = VeriDeposu::yeni();
        let r = depo.koy(Deger::Tablo(ornek_tablo(50)));
        assert_eq!(r.as_str(), "tablo#1");
        let ozet = depo.ozet(&r, 2);
        assert!(ozet.starts_with("| Ad |"));
        assert_eq!(ozet.lines().filter(|s| s.starts_with('|')).count(), 4);
    }

    #[test]
    fn bayt_icerigi_hicbir_zaman_sizmaz() {
        let mut depo = VeriDeposu::yeni();
        let r = depo.koy(Deger::Bayt(vec![b'S', b'I', b'R', b'R']));
        let ozet = depo.ozet(&r, 1000);
        assert_eq!(ozet, "4 bayt ikili veri");
        assert!(!ozet.contains("SIRR"));
    }

    #[test]
    fn bilinmeyen_ref_acik_cumle_dondurur() {
        let depo = VeriDeposu::yeni();
        let ozet = depo.ozet(&KaynakRef("yok#9".into()), 3);
        assert!(ozet.contains("bulunamadi"));
    }

    #[test]
    fn temizle_sayaci_sifirlamaz() {
        let mut depo = VeriDeposu::yeni();
        let eski = depo.koy(Deger::Metin("a".into()));
        depo.temizle();
        let yeni = depo.koy(Deger::Metin("b".into()));
        assert_ne!(eski, yeni, "eski ref yeni kayda denk gelmemeli");
        assert!(depo.al(&eski).is_none());
    }

    #[test]
    fn etiketli_koyma_turden_ile_filtrelenir() {
        let mut depo = VeriDeposu::yeni();
        depo.koy_etiketli("takvim", Deger::Metin("t1".into()));
        depo.koy_etiketli("belge", Deger::Metin("b1".into()));
        let r = depo.koy_etiketli("takvim", Deger::Metin("t2".into()));
        assert_eq!(depo.turden("takvim").len(), 2);
        assert_eq!(r.as_str(), "takvim#3");
        assert_eq!(depo.tur(&r), Some("takvim"));
    }

    #[test]
    fn paylasimli_depo_cekirdek_sozlesmesini_karsilar() {
        let depo = PaylasimliDepo::yeni();
        let d: &dyn CekirdekVeriDeposu = &depo;
        let r = d.koy("belge", "kisa", "bir\niki\nuc".to_string());
        let kayit = d.al(&r).expect("kayit");
        assert_eq!(kayit.tur, "belge");
        assert_eq!(kayit.govde, "bir\niki\nuc");
        assert_eq!(d.turden("belge").len(), 1);
        d.temizle();
        assert!(d.al(&r).is_none());
    }

    #[test]
    fn paylasimli_depo_tabloyu_tipli_saklar() {
        let depo = PaylasimliDepo::yeni();
        let r = depo.koy_deger("rapor", Deger::Tablo(ornek_tablo(9)));
        assert!(matches!(depo.deger(&r), Some(Deger::Tablo(_))));
        assert!(depo.ozet(&r, 2).contains("(+7 satir"));
    }
}
