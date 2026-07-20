//! `TokenMaskesi` — dagarciktaki hangi tokenlarin su an uretilebilecegini soyler.
//!
//! SICAK YOL: bu fonksiyon uretilen HER token icin bir kez calisir. Saf yontem
//! (her token icin durumu klonlayip metni bastan besle) 32k'lik bir dagarcikta
//! adim basina ~150k karakter isler ve tokenlarin ortak oneklerini defalarca
//! yeniden hesaplar.
//!
//! Bu yuzden dagarcik BIR KEZ onek agacina (trie) konur ve maske agac uzerinde
//! derinlik-oncelikli gezinerek uretilir: bir onek gramerde reddedildigi anda
//! o dalin ALTINDAKI TUM tokenlar tek hamlede elenir. Pratikte gecerli onek
//! sayisi cok kucuktur (JSON'un o anki konumunda birkac yuz dal), yani is
//! dagarcik boyuyla degil gecerli dal sayisiyla olcuklenir.
//!
//! Tokenizer bagimliligi YOK: arayuz `&[String]`. Hangi tokenizer kullanilirsa
//! kullanilsin bu crate degismez.
//!
//! OLCUM (32k token, release, M-serisi; adim basina):
//!   yapisal konumlar (nesne basi / anahtar / enum) ....... 2-3 µs
//!   serbest metin govdesi ............................... ~0.95 ms
//! Serbest metin pahalidir cunku orada dagarcigin neredeyse TAMAMI gecerlidir
//! (~31k token), yani budanacak dal yoktur. 3B bir modelde adim butcesi
//! ~33ms oldugundan bu en kotu hal bile butcenin %3'u kadardir.

use crate::{GramerDurumu, IzinKumesi};

/// Onek agacinin bir dugumu.
#[derive(Debug, Default)]
struct TrieDugumu {
    /// (karakter, cocuk indeksi) — siralidir, ikili arama yapilabilir.
    cocuklar: Vec<(char, usize)>,
    /// Bu dugumde biten token id'leri. `Vec`: ayni metne sahip iki token
    /// bulunan dagarciklar var; ikisi de maskelenmeli.
    bitenler: Vec<usize>,
}

/// Dagarcik uzerine bir kez kurulan, tekrar tekrar kullanilan maske uretici.
#[derive(Debug)]
pub struct TokenMaskesi {
    dugumler: Vec<TrieDugumu>,
    dagarcik_boyu: usize,
    /// Bos metinli tokenlar (ozel/kontrol tokenlari). Gramer acisindan
    /// notrdurler; maskede daima kapali birakilir — EOS gibi ozel tokenlarin
    /// ne zaman serbest oldugu cagiranin karari (`GramerDurumu::bitti_mi`).
    bos_tokenlar: Vec<usize>,
}

impl TokenMaskesi {
    /// Dagarcigi onek agacina cevirir. Maliyet bir kezliktir.
    pub fn yeni(dagarcik: &[String]) -> Self {
        let mut dugumler = vec![TrieDugumu::default()];
        let mut bos_tokenlar = Vec::new();
        for (id, metin) in dagarcik.iter().enumerate() {
            if metin.is_empty() {
                bos_tokenlar.push(id);
                continue;
            }
            let mut su_an = 0usize;
            for c in metin.chars() {
                su_an = match dugumler[su_an].cocuklar.binary_search_by_key(&c, |(k, _)| *k) {
                    Ok(i) => dugumler[su_an].cocuklar[i].1,
                    Err(i) => {
                        dugumler.push(TrieDugumu::default());
                        let yeni = dugumler.len() - 1;
                        dugumler[su_an].cocuklar.insert(i, (c, yeni));
                        yeni
                    }
                };
            }
            dugumler[su_an].bitenler.push(id);
        }
        Self { dugumler, dagarcik_boyu: dagarcik.len(), bos_tokenlar }
    }

    /// Bos metinli (ozel) token id'leri; cagiran EOS kararini bunlarla verir.
    pub fn bos_tokenlar(&self) -> &[usize] {
        &self.bos_tokenlar
    }

    pub fn dagarcik_boyu(&self) -> usize {
        self.dagarcik_boyu
    }

    /// Verilen gramer durumunda uretilebilir token'lar icin `true`.
    pub fn maske(&self, durum: &GramerDurumu) -> Vec<bool> {
        self.maske_sonlandiricili(durum, None)
    }

    /// Gramere ek olarak bir SONLANDIRICI karakteri de tanyan maske.
    ///
    /// NEDEN GEREKLI (olcumle bulundu): gramer yalnizca ARGUMANLARI tanimlar;
    /// cagriyi kapatan `)` onun DISINDADIR. Ama belirtecleyici bu siniri
    /// tanimaz. Qwen2.5 dagarciginda `hesapla({"ifade": "12*8"})` dizgisinin
    /// DOGAL belirteclemesi `"})` (id 80154) ile biter — yani modelin uretmesi
    /// EN OLASI belirtec, tam da gramer siniri uzerine oturur.
    ///
    /// Sonlandiriciyi tanimadan yalnizca gramer icinde gezseydik o belirtec
    /// maskede KAPALI kalirdi; model gecerli cagriyi tek hamlede yazamaz,
    /// `"}` + `)` diye bolmek zorunda kalirdi. Daha kotusu, `ilerlet` ayni
    /// belirteci KABUL ediyordu: maske ile ilerletme ayrisir, yani kisit
    /// kendi kabul ettigi ciktiyi uretilemez kilardi.
    ///
    /// Cozum gezinin dogal parcasi: gramer KABUL durumundayken sonlandirici
    /// kenari da izlenir ve orada BITEN belirtecler acilir. Sonlandiricidan
    /// SONRASINA inilmez — cagri orada kapanir, devami artik gramerin
    /// konusu degil ve cagrinin ardina gevezelik iliştirilmesine izin verirdi.
    pub fn maske_sonlandiricili(
        &self,
        durum: &GramerDurumu,
        sonlandirici: Option<char>,
    ) -> Vec<bool> {
        let mut maske = vec![false; self.dagarcik_boyu];
        let izin = durum.izinli_onekler();
        self.gez(0, durum, &izin, sonlandirici, &mut maske);
        maske
    }

    /// Gramer su an kapanabiliyorsa, bu dugumden cikan sonlandirici kenarinda
    /// BITEN belirtecleri acar. Ayri fonksiyon: `gez` icinde iki yerden
    /// cagriliyor (kok ve her ilerleyen dal) ve kosul tek yerde dursun.
    fn sonlandiriciyi_ac(
        &self,
        dugum: usize,
        durum: &GramerDurumu,
        sonlandirici: Option<char>,
        maske: &mut [bool],
    ) {
        let Some(son) = sonlandirici else { return };
        if !durum.bitti_mi() {
            return;
        }
        let Ok(i) = self.dugumler[dugum].cocuklar.binary_search_by_key(&son, |(k, _)| *k) else {
            return;
        };
        let cocuk = self.dugumler[dugum].cocuklar[i].1;
        for id in &self.dugumler[cocuk].bitenler {
            maske[*id] = true;
        }
    }

    /// `izin` daima `durum`un izin kumesidir; parametre olarak gecirilir cunku
    /// notr dallarda durum degismez, dolayisiyla kume de degismez — yeniden
    /// hesaplamak dugum basina bir tahsis demekti.
    fn gez(
        &self,
        dugum: usize,
        durum: &GramerDurumu,
        izin: &IzinKumesi,
        sonlandirici: Option<char>,
        maske: &mut [bool],
    ) {
        // Bu dugumde gramer kapanabiliyorsa cagriyi bitiren belirtec de mesru.
        self.sonlandiriciyi_ac(dugum, durum, sonlandirici, maske);
        for (c, cocuk) in &self.dugumler[dugum].cocuklar {
            // Once ucuz kontrol: karakter bu konumda gecerli mi. Gecerliyse
            // durumu ilerlet — klon yalniz gecerli dallarda odenir.
            if !izin.icerir(*c) {
                continue;
            }
            // Notr karakter otomati ilerletmez: klonu hic odemeden ayni
            // durumla derine in. Serbest metin alanlarinda gezinin neredeyse
            // tamami bu daldir. Aksi halde durumu gercekten ilerlet — token
            // ancak gecis DOGRULANDIKTAN sonra maskede acilir; `izin` hizli
            // on eleme, otomat ise son sozdur.
            if durum.notr_mu(*c) {
                for id in &self.dugumler[*cocuk].bitenler {
                    maske[*id] = true;
                }
                self.gez(*cocuk, durum, izin, sonlandirici, maske);
                continue;
            }
            let Ok(yeni) = durum.dallan(*c) else { continue };
            for id in &self.dugumler[*cocuk].bitenler {
                maske[*id] = true;
            }
            let yeni_izin = yeni.izinli_onekler();
            self.gez(*cocuk, &yeni, &yeni_izin, sonlandirici, maske);
        }
    }
}
