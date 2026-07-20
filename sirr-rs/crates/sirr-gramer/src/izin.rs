//! `IzinKumesi` — "su an hangi karakterler gecerli" sorusunun cevabi.
//!
//! Kume neden duz bir `Set<char>` DEGIL: metin govdesinde izinli karakter
//! sayisi Unicode'un tamami eksi bir avuc kacis karakteridir. Bunu tek tek
//! saymak (a) imkansiza yakin, (b) her adimda milyonlarca eleman uretmek
//! demek olurdu. Bu yuzden kume iki parcali: sonlu bir karakter listesi +
//! "metin govdesi acik" bayragi. `icerir()` ikisini birlikte degerlendirir.

use std::collections::BTreeSet;

/// JSON'da yapisal konumlarda serbest birakilan bosluk karakterleri.
pub(crate) const BOSLUKLAR: [char; 4] = [' ', '\t', '\n', '\r'];

/// Bir sonraki adimda kabul edilebilir karakterler.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IzinKumesi {
    /// Sonlu, tek tek sayilabilen karakterler (yapisal isaretler, haneler,
    /// anahtar/enum onekleri). `BTreeSet`: cikti siralidir, testler ve
    /// eval karsilastirmalari kararli olsun.
    karakterler: BTreeSet<char>,
    /// Acikken: kontrol karakteri olmayan, `"` ve `\` disindaki HER karakter
    /// gecerlidir (bir JSON metninin govdesindeyiz).
    metin_govdesi: bool,
    /// Bosluk karakterleri serbest mi.
    bosluk: bool,
    /// Girdi tam burada bitebilir mi (gecerli, tamamlanmis JSON).
    bitebilir: bool,
}

impl IzinKumesi {
    pub(crate) fn ekle(&mut self, c: char) {
        self.karakterler.insert(c);
    }

    pub(crate) fn ekle_hepsi(&mut self, cs: impl IntoIterator<Item = char>) {
        self.karakterler.extend(cs);
    }

    pub(crate) fn metin_govdesi_ac(&mut self) {
        self.metin_govdesi = true;
    }

    pub(crate) fn bosluk_ac(&mut self) {
        self.bosluk = true;
    }

    pub(crate) fn bitebilir_ac(&mut self) {
        self.bitebilir = true;
    }

    /// Bu karakter su anda uretilebilir mi.
    pub fn icerir(&self, c: char) -> bool {
        if self.karakterler.contains(&c) {
            return true;
        }
        if self.bosluk && BOSLUKLAR.contains(&c) {
            return true;
        }
        // Metin govdesi: JSON kontrol karakterlerini kacissiz yasaklar.
        self.metin_govdesi && c != '"' && c != '\\' && !c.is_control()
    }

    /// Tek tek sayilabilen izinli karakterler (metin govdesi HARIC).
    ///
    /// Istem metni ve hata ayiklama icin; maskeleme bunu degil `icerir()`i
    /// kullanir, cunku metin govdesi burada gorunmez.
    pub fn karakterler(&self) -> impl Iterator<Item = char> + '_ {
        self.karakterler.iter().copied()
    }

    /// Serbest metin yaziliyor mu (govde acik mi).
    pub fn metin_govdesi_mi(&self) -> bool {
        self.metin_govdesi
    }

    pub fn bosluk_serbest_mi(&self) -> bool {
        self.bosluk
    }

    /// Uretim tam burada bitebilir mi.
    pub fn bitebilir(&self) -> bool {
        self.bitebilir
    }

    /// Hicbir sey uretilemiyor mu (olu dugum). Saglikli bir gramerde
    /// `bitebilir` olmadan bos kume olusmamali; testler bunu kollar.
    pub fn bos_mu(&self) -> bool {
        self.karakterler.is_empty() && !self.metin_govdesi && !self.bosluk
    }
}
