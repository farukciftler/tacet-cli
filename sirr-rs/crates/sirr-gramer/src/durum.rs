//! `GramerDurumu` — asagi-itmeli otomat (push-down automaton).
//!
//! NEDEN OTOMAT, NEDEN SONRADAN DOGRULAMA DEGIL: kucuk bir model (3B) serbest
//! birakildiginda arac cagirmak yerine "sunu yapayim mi?" diye ANLATMAYA
//! baslar. Uretilmis metni sonradan dogrulamak bu regresyonu yakalar ama
//! duzeltmez; tek care gecersiz karakteri daha uretilmeden imkansiz kilmaktir.
//! Bu yuzden otomat karakter bazinda ilerler ve her adimda "su an ne gecerli"
//! sorusuna cevap verebilir — maskeleme tam olarak bu cevaba dayanir.
//!
//! YIGIN: JSON ic ice gecebildigi icin duz bir DFA yetmez; acik nesne/dizi
//! cerceveleri yiginda tutulur. Cerceveler yalniz `usize` indeks + kucuk
//! sayaclar tasir (bkz. derleme.rs), boylece durum klonu ucuzdur — maskeleme
//! adim basina yuzlerce klon yapar.

use crate::derleme::{Dugum, Gramer};
use crate::hata::GramerHatasi;
use crate::izin::{BOSLUKLAR, IzinKumesi};
use std::sync::Arc;

/// Bool degerleri; gramere harfi harfine gomulur.
const SABITLER: [&str; 2] = ["true", "false"];

/// JSON kacis karakterinden sonra gelebilecekler.
const KACISLAR: [char; 8] = ['"', '\\', '/', 'b', 'f', 'n', 'r', 't'];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NesneAsama {
    /// `{` yendi: ya anahtar basliyor ya nesne kapaniyor.
    AnahtarYaDaKapat,
    /// Anahtarin acilis tirnagi yendi, harfler eslesiyor.
    AnahtarIcinde,
    /// Anahtarin kapanis tirnagi yendi, `:` bekleniyor.
    AnahtarSonrasi,
    /// `:` yendi, deger bekleniyor.
    IkiNoktaSonrasi,
    /// Deger cercevesi yiginda, ustte o var.
    DegerIcinde,
    /// Deger bitti: `,` ya da `}`.
    DegerSonrasi,
    /// `,` yendi: mutlaka yeni anahtar gelmeli.
    VirgulSonrasi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiziAsama {
    /// `[` yendi.
    Baslangic,
    /// Eleman cercevesi ustte.
    ElemanIcinde,
    /// Eleman bitti: `,` ya da `]`.
    ElemanSonrasi,
    /// `,` yendi: mutlaka yeni eleman gelmeli.
    ElemanOncesi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetinAsama {
    Govde,
    /// `\` yendi.
    Kacis,
    /// `\u` yendi; kac onaltilik hane kaldi.
    Onaltilik(u8),
}

/// JSON sayi grameri: `-? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE][+-]?[0-9]+)?`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SayiAsama {
    Basla,
    EksiSonrasi,
    /// Tek basina `0` yendi — arkasindan hane GELEMEZ (JSON bastaki sifiri yasaklar).
    Sifir,
    TamHane,
    /// `.` yendi, en az bir hane sart.
    Nokta,
    KesirHane,
    /// `e`/`E` yendi.
    Us,
    /// `+`/`-` yendi, hane sart.
    UsIsaret,
    UsHane,
}

impl SayiAsama {
    /// Sayi burada bitebilir mi (ust cerceveye tuketilmeden donulebilir mi).
    fn bitebilir(self) -> bool {
        matches!(
            self,
            SayiAsama::Sifir | SayiAsama::TamHane | SayiAsama::KesirHane | SayiAsama::UsHane
        )
    }
}

/// Sayi otomatinin tek gecis tablosu.
///
/// `cerceve_besle` ile `cerceve_izin` BU fonksiyonu paylasir. Ayri yazilsalardi
/// ikisi zamanla ayrisir, izin kumesi otomatin gercekten kabul ettiginden genis
/// kalir ve maske modele olu bir yol acardi — hata ancak uretim kilitlendiginde
/// gorulurdu.
fn sayi_gecis(asama: SayiAsama, c: char, ondalik: bool, en_az: Option<f64>) -> Option<SayiAsama> {
    let hane = c.is_ascii_digit();
    match (asama, c) {
        // Aralik tamamen negatif olmayan ise eksi isareti hic acilmaz.
        (SayiAsama::Basla, '-') if en_az.is_none_or(|v| v < 0.0) => Some(SayiAsama::EksiSonrasi),
        (SayiAsama::Basla | SayiAsama::EksiSonrasi, '0') => Some(SayiAsama::Sifir),
        (SayiAsama::Basla | SayiAsama::EksiSonrasi, _) if ('1'..='9').contains(&c) => {
            Some(SayiAsama::TamHane)
        }
        (SayiAsama::TamHane, _) if hane => Some(SayiAsama::TamHane),
        (SayiAsama::Sifir | SayiAsama::TamHane, '.') if ondalik => Some(SayiAsama::Nokta),
        (SayiAsama::Sifir | SayiAsama::TamHane, 'e' | 'E') if ondalik => Some(SayiAsama::Us),
        (SayiAsama::Nokta | SayiAsama::KesirHane, _) if hane => Some(SayiAsama::KesirHane),
        (SayiAsama::KesirHane, 'e' | 'E') => Some(SayiAsama::Us),
        (SayiAsama::Us, '+' | '-') => Some(SayiAsama::UsIsaret),
        (SayiAsama::Us | SayiAsama::UsIsaret | SayiAsama::UsHane, _) if hane => {
            Some(SayiAsama::UsHane)
        }
        _ => None,
    }
}

/// Sayi otomatinda denenebilecek karakterler; `sayi_gecis` bunlari eler.
const SAYI_ADAYLARI: [char; 15] = [
    '-', '+', '.', 'e', 'E', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9',
];

/// Bu ONEK hala aralik icinde bir sayiya tamamlanabilir mi.
///
/// NEDEN ONEK BAZINDA: aralik yalniz sayi kapanirken kontrol edilseydi, model
/// `[1,50]` araliginda `99` uretip cikmaza girerdi — sonraki her karakter
/// reddedilir, maske bosalir, uretim KILITLENIR. Kisitli uretimde "sonradan
/// hata vermek" diye bir secenek yok: gecersize goturen her onek daha
/// uretilmeden kapanmali.
///
/// Yontem: onekten ulasilabilecek deger araligi hesaplanir. `v` onegin tam
/// kismi ve arkasindan `k` hane daha gelebiliyorsa ulasilabilir degerler
/// `[v*10^k, v*10^k + 10^k)` araliklarinin birlesimidir; bunlardan biri
/// [en_az, en_cok] ile kesisiyorsa onek yasar.
fn aralik_uygun(
    tampon: &str,
    asama: SayiAsama,
    tam_mi: bool,
    en_az: Option<f64>,
    en_cok: Option<f64>,
) -> bool {
    if en_az.is_none() && en_cok.is_none() {
        return true;
    }
    // Us gosterimi degeri her iki yone de kaydirabilir; onekten anlamli bir
    // ust sinir cikmaz. Budama yapmayiz, kapanistaki kesin kontrol yeter.
    if matches!(asama, SayiAsama::Us | SayiAsama::UsIsaret | SayiAsama::UsHane) {
        return true;
    }
    let negatif = tampon.starts_with('-');
    let govde = tampon.trim_start_matches('-');
    let tam_kisim: String = govde.chars().take_while(char::is_ascii_digit).collect();
    if tam_kisim.is_empty() {
        return true; // henuz hane yok (yalniz '-' yazildi)
    }
    let Ok(v) = tam_kisim.parse::<f64>() else {
        return true;
    };
    let lo = en_az.unwrap_or(f64::NEG_INFINITY);
    let hi = en_cok.unwrap_or(f64::INFINITY);
    // Tam kisim ancak hane eklenebilen asamalarda buyur. `Sifir` disarida:
    // JSON bastaki sifirdan sonra hane kabul etmez.
    let buyuyebilir = matches!(asama, SayiAsama::TamHane);
    let en_cok_k = if buyuyebilir { 18 } else { 0 };
    for k in 0..=en_cok_k {
        let c = 10f64.powi(k);
        let taban = v * c;
        // Tamsayida ust sinir DAHIL (v*c + c - 1), ondalikta HARIC ((v+1)*c):
        // `[1,50]` araliginda `0` onegi ancak bu ayrimla elenir.
        let (alt, ust, ust_dahil) = if tam_mi {
            (taban, taban + c - 1.0, true)
        } else {
            (taban, (v + 1.0) * c, false)
        };
        let (alt, ust, alt_dahil, ust_dahil) = if negatif {
            (-ust, -alt, ust_dahil, true)
        } else {
            (alt, ust, true, ust_dahil)
        };
        let ust_tamam = if ust_dahil { ust >= lo } else { ust > lo };
        let alt_tamam = if alt_dahil { alt <= hi } else { alt < hi };
        if ust_tamam && alt_tamam {
            return true;
        }
        // Araliklar tek yonde kayiyor: hedefi gectiysek daha fazla hane kurtarmaz.
        if !negatif && alt > hi {
            break;
        }
        if negatif && ust < lo {
            break;
        }
    }
    false
}

#[derive(Debug, Clone)]
enum Cerceve {
    /// Henuz baslamamis bir deger: ilk karakter tipi secer.
    Deger { dugum: usize },
    Nesne {
        dugum: usize,
        /// Alan gorulme bayraklari; zorunlulari atlamayi ve anahtari
        /// tekrar etmeyi ayni yerden engeller.
        gorulen: Vec<bool>,
        asama: NesneAsama,
        /// Anahtar yazilirken hala mumkun olan alan indeksleri.
        adaylar: Vec<usize>,
        ilerleme: usize,
        /// Anahtari kapanmis, degeri beklenen alan.
        aktif: usize,
    },
    Dizi { dugum: usize, sayac: usize, asama: DiziAsama },
    Metin { dugum: usize, uzunluk: usize, asama: MetinAsama },
    Secenek { dugum: usize, adaylar: Vec<usize>, ilerleme: usize },
    Sayi { dugum: usize, asama: SayiAsama, tampon: String },
    Sabit { adaylar: Vec<usize>, ilerleme: usize },
}

/// Bir karakterin cerceve uzerindeki etkisi.
enum Adim {
    /// Karakter yendi, cerceve devam ediyor.
    Tuketildi,
    /// Karakter yendi ve cerceve tamamlandi.
    TuketildiVeBitti,
    /// Cerceve tamamlandi ama karakteri YEMEDI; ust cerceve denemeli.
    /// (Sayilar boyle biter: `12` ancak `,` gorulunce kapanir.)
    BittiTuketilmedi,
    /// Cerceve degisti/yeni cerceve itildi; ayni karakter bastan denenmeli.
    Yeniden,
}

/// Kisitli uretimin canli durumu.
#[derive(Debug, Clone)]
pub struct GramerDurumu {
    gramer: Arc<Gramer>,
    yigin: Vec<Cerceve>,
    konum: usize,
}

impl GramerDurumu {
    pub fn yeni(gramer: Arc<Gramer>) -> Self {
        let kok = gramer.kok;
        Self { gramer, yigin: vec![Cerceve::Deger { dugum: kok }], konum: 0 }
    }

    /// Uretilmis parcayi otomatta ilerletir.
    ///
    /// ATOMIK: hata halinde durum HIC degismez (kopya uzerinde calisilir).
    /// Gerekce: cagri yerleri hatadan sonra ayni durumla devam etmek ister;
    /// yarim ilerlemis bir otomat sessizce yanlis maske uretirdi.
    pub fn ilerlet(&mut self, parca: &str) -> Result<(), GramerHatasi> {
        let mut deneme = self.clone();
        for c in parca.chars() {
            deneme.tek_karakter(c)?;
            deneme.konum += 1;
        }
        *self = deneme;
        Ok(())
    }

    /// Tek karakterlik dal: yeni durumu dondurur, kendisini degistirmez.
    /// Maskeleme sicak yolunda dagarcik agacinda gezerken kullanilir.
    pub fn dallan(&self, c: char) -> Result<Self, GramerHatasi> {
        let mut d = self.clone();
        d.tek_karakter(c)?;
        d.konum += 1;
        Ok(d)
    }

    /// Su an kabul edilebilir karakterler.
    pub fn izinli_onekler(&self) -> IzinKumesi {
        let mut k = IzinKumesi::default();
        let mut i = self.yigin.len();
        // `cocuk_bitti`: bir alt cerceve tuketmeden kapanabiliyorsa, ust
        // cerceveye "cocugum bitmis gibi" bakmak gerekir. Tek boyle cerceve
        // sayidir (`12` sonrasi `,` ust nesnenin isi), ama dongu genel yazildi.
        let mut cocuk_bitti = false;
        loop {
            if i == 0 {
                k.bitebilir_ac();
                k.bosluk_ac();
                break;
            }
            i -= 1;
            if !self.cerceve_izin(i, cocuk_bitti, &mut k) {
                break;
            }
            cocuk_bitti = true;
        }
        k
    }

    /// Gecerli, tamamlanmis bir JSON uretildi mi.
    pub fn bitti_mi(&self) -> bool {
        self.yigin.is_empty()
    }

    /// Uretimi kapatir; yapi acik kaldiysa hata verir.
    pub fn bitir(&self) -> Result<(), GramerHatasi> {
        if self.bitti_mi() { Ok(()) } else { Err(GramerHatasi::Tamamlanmadi) }
    }

    /// Onbelleksiz maske — kucuk dagarciklar ve testler icin.
    /// Sicak yolda `TokenMaskesi::yeni()` ile agaci BIR KEZ kurup yeniden
    /// kullanin; bu surum her cagride agaci bastan kurar.
    pub fn maske(&self, dagarcik: &[String]) -> Vec<bool> {
        crate::TokenMaskesi::yeni(dagarcik).maske(self)
    }

    /// Bu karakter otomatin GELECEKTEKI kararlarini degistirmiyor mu?
    ///
    /// Sinirsiz uzunluktaki bir metin govdesinde siradan karakterler hicbir
    /// gecisi etkilemez: sonrasinda da ayni kume izinlidir. Maskeleme sicak
    /// yolunda bu durum baskindir — serbest metin alaninda dagarcigin
    /// neredeyse TAMAMI gecerlidir, yani 32k dalin her birinde durum
    /// klonlanirdi. Olculen etki: adim basina 6.1ms -> 0.2ms.
    pub(crate) fn notr_mu(&self, c: char) -> bool {
        match self.yigin.last() {
            Some(Cerceve::Metin { dugum, asama: MetinAsama::Govde, .. }) => {
                matches!(
                    self.gramer.dugumler[*dugum],
                    Dugum::Metin { en_cok_uzunluk: None }
                ) && c != '"'
                    && c != '\\'
                    && !c.is_control()
            }
            _ => false,
        }
    }

    // ---- otomat ----

    fn tek_karakter(&mut self, c: char) -> Result<(), GramerHatasi> {
        loop {
            if self.yigin.is_empty() {
                // JSON kapandi: yalniz bosluk kabul edilir.
                return if BOSLUKLAR.contains(&c) {
                    Ok(())
                } else {
                    Err(GramerHatasi::FazladanGirdi { konum: self.konum })
                };
            }
            match self.cerceve_besle(c)? {
                Adim::Tuketildi => return Ok(()),
                Adim::TuketildiVeBitti => {
                    self.yigin.pop();
                    self.cocuk_bitti();
                    return Ok(());
                }
                Adim::BittiTuketilmedi => {
                    self.yigin.pop();
                    self.cocuk_bitti();
                }
                Adim::Yeniden => {}
            }
        }
    }

    /// Ust cerceveye "cocuk deger tamamlandi" bilgisini isler.
    fn cocuk_bitti(&mut self) {
        match self.yigin.last_mut() {
            Some(Cerceve::Nesne { asama, .. }) => *asama = NesneAsama::DegerSonrasi,
            Some(Cerceve::Dizi { asama, sayac, .. }) => {
                *sayac += 1;
                *asama = DiziAsama::ElemanSonrasi;
            }
            _ => {}
        }
    }

    fn cerceve_besle(&mut self, c: char) -> Result<Adim, GramerHatasi> {
        let n = self.yigin.len() - 1;
        let konum = self.konum;
        let hata = || GramerHatasi::BeklenmeyenKarakter { karakter: c, konum };
        let bosluk = BOSLUKLAR.contains(&c);

        // Disjoint alan odunclemeleri: `gramer` salt-okunur, `yigin` degisken.
        let gramer = &*self.gramer;
        let cerceve = &mut self.yigin[n];
        let mut itilecek: Option<Cerceve> = None;

        let adim = match cerceve {
            // --- baslamamis deger: ilk karakter tipi secer ---
            Cerceve::Deger { dugum } => {
                let d = *dugum;
                if bosluk {
                    Adim::Tuketildi
                } else {
                    match &gramer.dugumler[d] {
                        Dugum::Nesne { alanlar } if c == '{' => {
                            *cerceve = Cerceve::Nesne {
                                dugum: d,
                                gorulen: vec![false; alanlar.len()],
                                asama: NesneAsama::AnahtarYaDaKapat,
                                adaylar: Vec::new(),
                                ilerleme: 0,
                                aktif: 0,
                            };
                            Adim::Tuketildi
                        }
                        Dugum::Dizi { .. } if c == '[' => {
                            *cerceve = Cerceve::Dizi {
                                dugum: d,
                                sayac: 0,
                                asama: DiziAsama::Baslangic,
                            };
                            Adim::Tuketildi
                        }
                        Dugum::Metin { .. } if c == '"' => {
                            *cerceve = Cerceve::Metin {
                                dugum: d,
                                uzunluk: 0,
                                asama: MetinAsama::Govde,
                            };
                            Adim::Tuketildi
                        }
                        Dugum::Secenek { secenekler } if c == '"' => {
                            *cerceve = Cerceve::Secenek {
                                dugum: d,
                                adaylar: (0..secenekler.len()).collect(),
                                ilerleme: 0,
                            };
                            Adim::Tuketildi
                        }
                        Dugum::Sayi { .. } => {
                            // Karakteri sayi cercevesi yesin: isaret/hane ayrimi orada.
                            *cerceve = Cerceve::Sayi {
                                dugum: d,
                                asama: SayiAsama::Basla,
                                tampon: String::new(),
                            };
                            Adim::Yeniden
                        }
                        Dugum::Bool => {
                            *cerceve = Cerceve::Sabit {
                                adaylar: (0..SABITLER.len()).collect(),
                                ilerleme: 0,
                            };
                            Adim::Yeniden
                        }
                        _ => return Err(hata()),
                    }
                }
            }

            // --- nesne ---
            Cerceve::Nesne { dugum, gorulen, asama, adaylar, ilerleme, aktif } => {
                let Dugum::Nesne { alanlar } = &gramer.dugumler[*dugum] else {
                    unreachable!("nesne cercevesi nesne olmayan dugumle acilamaz")
                };
                let kalan_var = gorulen.iter().any(|g| !g);
                let zorunlular_tamam = alanlar
                    .iter()
                    .zip(gorulen.iter())
                    .all(|(a, g)| !a.zorunlu || *g);

                match *asama {
                    NesneAsama::AnahtarYaDaKapat | NesneAsama::VirgulSonrasi => {
                        let kapanabilir =
                            *asama == NesneAsama::AnahtarYaDaKapat && zorunlular_tamam;
                        if bosluk {
                            Adim::Tuketildi
                        } else if c == '}' && kapanabilir {
                            Adim::TuketildiVeBitti
                        } else if c == '"' && kalan_var {
                            // Gorulmus anahtar aday degil: tekrar eden anahtar
                            // uretilemez, sema ile ayni sozlesme.
                            *adaylar = (0..alanlar.len()).filter(|i| !gorulen[*i]).collect();
                            *ilerleme = 0;
                            *asama = NesneAsama::AnahtarIcinde;
                            Adim::Tuketildi
                        } else {
                            return Err(hata());
                        }
                    }
                    NesneAsama::AnahtarIcinde => {
                        if c == '"' {
                            // Tam eslesen adayi sec; yoksa anahtar yarim kalmis.
                            match adaylar
                                .iter()
                                .copied()
                                .find(|i| alanlar[*i].ad_harfleri.len() == *ilerleme)
                            {
                                Some(i) => {
                                    gorulen[i] = true;
                                    *aktif = i;
                                    *asama = NesneAsama::AnahtarSonrasi;
                                    Adim::Tuketildi
                                }
                                None => return Err(hata()),
                            }
                        } else {
                            let kalanlar: Vec<usize> = adaylar
                                .iter()
                                .copied()
                                .filter(|i| {
                                    alanlar[*i].ad_harfleri.get(*ilerleme) == Some(&c)
                                })
                                .collect();
                            if kalanlar.is_empty() {
                                return Err(hata());
                            }
                            *adaylar = kalanlar;
                            *ilerleme += 1;
                            Adim::Tuketildi
                        }
                    }
                    NesneAsama::AnahtarSonrasi => {
                        if bosluk {
                            Adim::Tuketildi
                        } else if c == ':' {
                            *asama = NesneAsama::IkiNoktaSonrasi;
                            Adim::Tuketildi
                        } else {
                            return Err(hata());
                        }
                    }
                    NesneAsama::IkiNoktaSonrasi => {
                        if bosluk {
                            Adim::Tuketildi
                        } else {
                            itilecek = Some(Cerceve::Deger { dugum: alanlar[*aktif].dugum });
                            *asama = NesneAsama::DegerIcinde;
                            Adim::Yeniden
                        }
                    }
                    NesneAsama::DegerIcinde => unreachable!("deger cercevesi ustte olmali"),
                    NesneAsama::DegerSonrasi => {
                        if bosluk {
                            Adim::Tuketildi
                        } else if c == ',' && kalan_var {
                            // Yazilacak alan kalmadiysa virgul olu yola sokar;
                            // maskede hic acilmamasi icin burada da kapali.
                            *asama = NesneAsama::VirgulSonrasi;
                            Adim::Tuketildi
                        } else if c == '}' && zorunlular_tamam {
                            Adim::TuketildiVeBitti
                        } else {
                            return Err(hata());
                        }
                    }
                }
            }

            // --- dizi ---
            Cerceve::Dizi { dugum, sayac, asama } => {
                let Dugum::Dizi { eleman, en_az, en_cok } = &gramer.dugumler[*dugum] else {
                    unreachable!("dizi cercevesi dizi olmayan dugumle acilamaz")
                };
                let alt = en_az.unwrap_or(0);
                match *asama {
                    DiziAsama::Baslangic => {
                        if bosluk {
                            Adim::Tuketildi
                        } else if c == ']' && alt == 0 {
                            Adim::TuketildiVeBitti
                        } else if en_cok.is_some_and(|u| u == 0) {
                            return Err(hata());
                        } else {
                            itilecek = Some(Cerceve::Deger { dugum: *eleman });
                            *asama = DiziAsama::ElemanIcinde;
                            Adim::Yeniden
                        }
                    }
                    DiziAsama::ElemanIcinde => unreachable!("eleman cercevesi ustte olmali"),
                    DiziAsama::ElemanSonrasi => {
                        if bosluk {
                            Adim::Tuketildi
                        } else if c == ',' && en_cok.is_none_or(|u| *sayac < u) {
                            *asama = DiziAsama::ElemanOncesi;
                            Adim::Tuketildi
                        } else if c == ']' && *sayac >= alt {
                            Adim::TuketildiVeBitti
                        } else {
                            return Err(hata());
                        }
                    }
                    DiziAsama::ElemanOncesi => {
                        if bosluk {
                            Adim::Tuketildi
                        } else {
                            itilecek = Some(Cerceve::Deger { dugum: *eleman });
                            *asama = DiziAsama::ElemanIcinde;
                            Adim::Yeniden
                        }
                    }
                }
            }

            // --- serbest metin ---
            Cerceve::Metin { dugum, uzunluk, asama } => {
                let Dugum::Metin { en_cok_uzunluk } = &gramer.dugumler[*dugum] else {
                    unreachable!("metin cercevesi metin olmayan dugumle acilamaz")
                };
                let dolu = en_cok_uzunluk.is_some_and(|u| *uzunluk >= u);
                match *asama {
                    MetinAsama::Govde => {
                        if c == '"' {
                            Adim::TuketildiVeBitti
                        } else if dolu {
                            // Uzunluk sinirinda tek cikis kapanis tirnagi.
                            return Err(hata());
                        } else if c == '\\' {
                            *asama = MetinAsama::Kacis;
                            Adim::Tuketildi
                        } else if c.is_control() {
                            // JSON kontrol karakterini kacissiz yasaklar.
                            return Err(hata());
                        } else {
                            *uzunluk += 1;
                            Adim::Tuketildi
                        }
                    }
                    MetinAsama::Kacis => {
                        if c == 'u' {
                            *asama = MetinAsama::Onaltilik(4);
                            Adim::Tuketildi
                        } else if KACISLAR.contains(&c) {
                            // Kacis dizisi TEK karakter sayilir: uzunluk siniri
                            // uretilen metnin anlamli boyunu olcmeli.
                            *uzunluk += 1;
                            *asama = MetinAsama::Govde;
                            Adim::Tuketildi
                        } else {
                            return Err(hata());
                        }
                    }
                    MetinAsama::Onaltilik(kalan) => {
                        if !c.is_ascii_hexdigit() {
                            return Err(hata());
                        }
                        if kalan == 1 {
                            *uzunluk += 1;
                            *asama = MetinAsama::Govde;
                        } else {
                            *asama = MetinAsama::Onaltilik(kalan - 1);
                        }
                        Adim::Tuketildi
                    }
                }
            }

            // --- kapali deger kumesi ---
            Cerceve::Secenek { dugum, adaylar, ilerleme } => {
                let Dugum::Secenek { secenekler } = &gramer.dugumler[*dugum] else {
                    unreachable!("secenek cercevesi secenek olmayan dugumle acilamaz")
                };
                if c == '"' {
                    if adaylar.iter().any(|i| secenekler[*i].len() == *ilerleme) {
                        Adim::TuketildiVeBitti
                    } else {
                        return Err(hata());
                    }
                } else {
                    let kalanlar: Vec<usize> = adaylar
                        .iter()
                        .copied()
                        .filter(|i| secenekler[*i].get(*ilerleme) == Some(&c))
                        .collect();
                    if kalanlar.is_empty() {
                        return Err(hata());
                    }
                    *adaylar = kalanlar;
                    *ilerleme += 1;
                    Adim::Tuketildi
                }
            }

            // --- sayi ---
            Cerceve::Sayi { dugum, asama, tampon } => {
                let Dugum::Sayi { tam_mi, en_az, en_cok } = &gramer.dugumler[*dugum] else {
                    unreachable!("sayi cercevesi sayi olmayan dugumle acilamaz")
                };
                let ondalik = !*tam_mi;
                // Gecis tablosu + onek bazli aralik budamasi; ikisi de
                // `cerceve_izin` ile ORTAK, boylece izin kumesi otomatin
                // gercekten kabul ettiginden genis olamaz.
                let yeni = sayi_gecis(*asama, c, ondalik, *en_az).filter(|a| {
                    let mut aday = tampon.clone();
                    aday.push(c);
                    aralik_uygun(&aday, *a, *tam_mi, *en_az, *en_cok)
                });
                match yeni {
                    Some(a) => {
                        *asama = a;
                        tampon.push(c);
                        Adim::Tuketildi
                    }
                    None if asama.bitebilir() => {
                        let deger: f64 = tampon.parse().map_err(|_| {
                            GramerHatasi::AralikDisi(tampon.clone())
                        })?;
                        if en_az.is_some_and(|v| deger < v) || en_cok.is_some_and(|v| deger > v) {
                            return Err(GramerHatasi::AralikDisi(tampon.clone()));
                        }
                        Adim::BittiTuketilmedi
                    }
                    None => return Err(hata()),
                }
            }

            // --- true / false ---
            Cerceve::Sabit { adaylar, ilerleme } => {
                let kalanlar: Vec<usize> = adaylar
                    .iter()
                    .copied()
                    .filter(|i| SABITLER[*i].chars().nth(*ilerleme) == Some(c))
                    .collect();
                if kalanlar.is_empty() {
                    return Err(hata());
                }
                *ilerleme += 1;
                let i = *ilerleme;
                // "true"/"false" ortak onek tasimaz: tam eslesme kesin bitistir.
                if kalanlar.iter().any(|k| SABITLER[*k].chars().count() == i) {
                    Adim::TuketildiVeBitti
                } else {
                    *adaylar = kalanlar;
                    Adim::Tuketildi
                }
            }
        };

        if let Some(yeni) = itilecek {
            self.yigin.push(yeni);
        }
        Ok(adim)
    }

    // ---- izin kumesi ----

    /// `idx` cercevesinin izin verdigi karakterleri `k`ya ekler.
    /// Doner: bu cerceve tuketmeden bitebilir mi (ust cerceveye de bakilmali mi).
    fn cerceve_izin(&self, idx: usize, cocuk_bitti: bool, k: &mut IzinKumesi) -> bool {
        let gramer = &*self.gramer;
        match &self.yigin[idx] {
            Cerceve::Deger { dugum } => {
                k.bosluk_ac();
                self.deger_baslangici(*dugum, k);
                false
            }

            Cerceve::Nesne { dugum, gorulen, asama, adaylar, ilerleme, aktif } => {
                let Dugum::Nesne { alanlar } = &gramer.dugumler[*dugum] else {
                    return false;
                };
                let kalan_var = gorulen.iter().any(|g| !g);
                let zorunlular_tamam = alanlar
                    .iter()
                    .zip(gorulen.iter())
                    .all(|(a, g)| !a.zorunlu || *g);
                // Cocuk kapandiysa cerceve artik "deger sonrasi"ndadir.
                let etkin = if cocuk_bitti && *asama == NesneAsama::DegerIcinde {
                    NesneAsama::DegerSonrasi
                } else {
                    *asama
                };
                match etkin {
                    NesneAsama::AnahtarYaDaKapat | NesneAsama::VirgulSonrasi => {
                        k.bosluk_ac();
                        if etkin == NesneAsama::AnahtarYaDaKapat && zorunlular_tamam {
                            k.ekle('}');
                        }
                        if kalan_var {
                            k.ekle('"');
                        }
                    }
                    NesneAsama::AnahtarIcinde => {
                        for i in adaylar {
                            match alanlar[*i].ad_harfleri.get(*ilerleme) {
                                Some(c) => k.ekle(*c),
                                None => k.ekle('"'),
                            }
                        }
                    }
                    NesneAsama::AnahtarSonrasi => {
                        k.bosluk_ac();
                        k.ekle(':');
                    }
                    NesneAsama::IkiNoktaSonrasi => {
                        k.bosluk_ac();
                        self.deger_baslangici(alanlar[*aktif].dugum, k);
                    }
                    NesneAsama::DegerIcinde => {}
                    NesneAsama::DegerSonrasi => {
                        k.bosluk_ac();
                        if kalan_var {
                            k.ekle(',');
                        }
                        if zorunlular_tamam {
                            k.ekle('}');
                        }
                    }
                }
                false
            }

            Cerceve::Dizi { dugum, sayac, asama } => {
                let Dugum::Dizi { eleman, en_az, en_cok } = &gramer.dugumler[*dugum] else {
                    return false;
                };
                let alt = en_az.unwrap_or(0);
                let (etkin, adet) = if cocuk_bitti && *asama == DiziAsama::ElemanIcinde {
                    (DiziAsama::ElemanSonrasi, sayac + 1)
                } else {
                    (*asama, *sayac)
                };
                k.bosluk_ac();
                match etkin {
                    DiziAsama::Baslangic => {
                        if alt == 0 {
                            k.ekle(']');
                        }
                        if en_cok.is_none_or(|u| u > 0) {
                            self.deger_baslangici(*eleman, k);
                        }
                    }
                    DiziAsama::ElemanOncesi => self.deger_baslangici(*eleman, k),
                    DiziAsama::ElemanIcinde => {}
                    DiziAsama::ElemanSonrasi => {
                        if en_cok.is_none_or(|u| adet < u) {
                            k.ekle(',');
                        }
                        if adet >= alt {
                            k.ekle(']');
                        }
                    }
                }
                false
            }

            Cerceve::Metin { dugum, uzunluk, asama } => {
                let Dugum::Metin { en_cok_uzunluk } = &gramer.dugumler[*dugum] else {
                    return false;
                };
                match asama {
                    MetinAsama::Govde => {
                        k.ekle('"');
                        if en_cok_uzunluk.is_none_or(|u| *uzunluk < u) {
                            k.ekle('\\');
                            k.metin_govdesi_ac();
                        }
                    }
                    MetinAsama::Kacis => {
                        k.ekle_hepsi(KACISLAR);
                        k.ekle('u');
                    }
                    MetinAsama::Onaltilik(_) => {
                        k.ekle_hepsi(('0'..='9').chain('a'..='f').chain('A'..='F'));
                    }
                }
                false
            }

            Cerceve::Secenek { dugum, adaylar, ilerleme } => {
                let Dugum::Secenek { secenekler } = &gramer.dugumler[*dugum] else {
                    return false;
                };
                for i in adaylar {
                    match secenekler[*i].get(*ilerleme) {
                        Some(c) => k.ekle(*c),
                        None => k.ekle('"'),
                    }
                }
                false
            }

            Cerceve::Sayi { dugum, asama, tampon } => {
                let Dugum::Sayi { tam_mi, en_az, en_cok } = &gramer.dugumler[*dugum] else {
                    return false;
                };
                let ondalik = !*tam_mi;
                // Izin kumesi otomatin KENDISINE sorularak uretilir; ikinci bir
                // kural kopyasi tutulmaz (ayrisma riski yok). Aday sayisi 16,
                // maliyeti sabit.
                for c in SAYI_ADAYLARI {
                    if let Some(a) = sayi_gecis(*asama, c, ondalik, *en_az) {
                        let mut aday = tampon.clone();
                        aday.push(c);
                        if aralik_uygun(&aday, a, *tam_mi, *en_az, *en_cok) {
                            k.ekle(c);
                        }
                    }
                }
                // Sayi tuketmeden kapanabilir: ust cerceve de soz sahibi.
                // Ama once degerin gercekten aralikta oldugundan emin ol —
                // degilse burasi bitis noktasi degildir.
                asama.bitebilir()
                    && tampon.parse::<f64>().is_ok_and(|v| {
                        en_az.is_none_or(|x| v >= x) && en_cok.is_none_or(|x| v <= x)
                    })
            }

            Cerceve::Sabit { adaylar, ilerleme } => {
                for i in adaylar {
                    if let Some(c) = SABITLER[*i].chars().nth(*ilerleme) {
                        k.ekle(c);
                    }
                }
                false
            }
        }
    }

    /// Bir degerin baslayabilecegi karakterler.
    fn deger_baslangici(&self, dugum: usize, k: &mut IzinKumesi) {
        match &self.gramer.dugumler[dugum] {
            Dugum::Nesne { .. } => k.ekle('{'),
            Dugum::Dizi { .. } => k.ekle('['),
            Dugum::Metin { .. } | Dugum::Secenek { .. } => k.ekle('"'),
            // Sayinin ILK karakteri de aralik budamasindan gecer: `[1,50]`
            // araliginda `0` ile baslamak olu yoldur, hic acilmamali.
            Dugum::Sayi { tam_mi, en_az, en_cok } => {
                for c in SAYI_ADAYLARI {
                    if let Some(a) = sayi_gecis(SayiAsama::Basla, c, !*tam_mi, *en_az)
                        && aralik_uygun(&c.to_string(), a, *tam_mi, *en_az, *en_cok)
                    {
                        k.ekle(c);
                    }
                }
            }
            Dugum::Bool => k.ekle_hepsi(['t', 'f']),
        }
    }
}
