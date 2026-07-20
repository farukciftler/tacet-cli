//! `ArgSema` -> `Gramer` derlemesi.
//!
//! NEDEN AYRI BIR "DERLENMIS" BICIM: otomatin durumu (`GramerDurumu`) token
//! maskeleme sirasinda BINLERCE kez klonlanir — dagarcik agacinda her dal
//! denemesi bir klondur. Durum dogrudan `ArgSema` tutsaydi her klon derin bir
//! agac kopyasi olurdu. Bu yuzden sema bir kez arenaya (Vec<Dugum>) duzlestirilir,
//! durum yalnizca `usize` indeksler tasir; `Gramer` Arc ile paylasilir ve
//! klonlanmaz.
//!
//! Ikinci neden: metin karsilastirmalari char bazinda yapilir (anahtar ve enum
//! onek eslesmesi). char vektorlerini her adimda `str`den uretmek sicak yolda
//! israftir; derleme aninda bir kez uretilir.

use sirr_cekirdek::{Alan, ArgSema, SemaTipi};
use std::sync::Arc;

/// Derlenmis nesne alani.
#[derive(Debug, Clone)]
pub(crate) struct DerliAlan {
    /// Anahtar eslesmesi char bazinda ilerler; onceden uretilir.
    /// Alan adi yalniz bu bicimde saklanir: otomat hicbir yerde `str`
    /// karsilastirmasi yapmaz, ikinci bir kopya tutmak olu veri olurdu.
    pub(crate) ad_harfleri: Vec<char>,
    pub(crate) dugum: usize,
    pub(crate) zorunlu: bool,
}

/// Arenadaki tek bir sema dugumu.
#[derive(Debug, Clone)]
pub(crate) enum Dugum {
    Nesne {
        alanlar: Vec<DerliAlan>,
    },
    Dizi {
        eleman: usize,
        en_az: Option<usize>,
        en_cok: Option<usize>,
    },
    Metin {
        en_cok_uzunluk: Option<usize>,
    },
    Secenek {
        secenekler: Vec<Vec<char>>,
    },
    Sayi {
        tam_mi: bool,
        en_az: Option<f64>,
        en_cok: Option<f64>,
    },
    Bool,
}

/// Derlenmis, degismez gramer. Bir sema icin bir kez uretilir ve `Arc` ile
/// tum uretim boyunca paylasilir.
#[derive(Debug)]
pub struct Gramer {
    pub(crate) dugumler: Vec<Dugum>,
    pub(crate) kok: usize,
}

impl Gramer {
    /// Semayi arenaya duzlestirir.
    pub fn derle(sema: &ArgSema) -> Arc<Gramer> {
        let mut dugumler = Vec::new();
        let kok = ekle(&mut dugumler, sema);
        Arc::new(Gramer { dugumler, kok })
    }

    /// Bu gramer icin bastan bir uretim durumu acar.
    pub fn durum(self: &Arc<Self>) -> crate::GramerDurumu {
        crate::GramerDurumu::yeni(Arc::clone(self))
    }
}

fn ekle(arena: &mut Vec<Dugum>, sema: &ArgSema) -> usize {
    // Cocuklar once eklenir; dugumun kendisi sonra. Boylece indeksler
    // kararlidir ve arena tek yonlu (dongusuz) kalir — sema zaten agac.
    let dugum = match &sema.tip {
        SemaTipi::Nesne { alanlar } => {
            let derli: Vec<DerliAlan> = alanlar
                .iter()
                .map(|a: &Alan| DerliAlan {
                    ad_harfleri: a.ad.chars().collect(),
                    dugum: ekle(arena, &a.sema),
                    zorunlu: a.zorunlu,
                })
                .collect();
            Dugum::Nesne { alanlar: derli }
        }
        SemaTipi::Dizi { eleman, en_az, en_cok } => Dugum::Dizi {
            eleman: ekle(arena, eleman),
            en_az: *en_az,
            en_cok: *en_cok,
        },
        SemaTipi::Metin { en_cok_uzunluk } => Dugum::Metin { en_cok_uzunluk: *en_cok_uzunluk },
        // Secenek degerleri gramere HARFI HARFINE gomulur: model kume disina
        // cikamaz. Kacis gerektiren karakter (tirnak, ters bolu) iceren bir
        // secenek uretilemez hale gelir; bu bilincli bir daraltmadir —
        // enum degerleri kimlik/etiket olmalidir, serbest metin degil.
        SemaTipi::Secenek { secenekler } => Dugum::Secenek {
            secenekler: secenekler.iter().map(|s| s.chars().collect()).collect(),
        },
        SemaTipi::Sayi { tam_mi, en_az, en_cok } => Dugum::Sayi {
            tam_mi: *tam_mi,
            en_az: *en_az,
            en_cok: *en_cok,
        },
        SemaTipi::Bool => Dugum::Bool,
    };
    arena.push(dugum);
    arena.len() - 1
}
