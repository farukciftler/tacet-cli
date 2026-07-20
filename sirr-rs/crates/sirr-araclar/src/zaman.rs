//! Zaman araci — su anki tarih/saat + iki tarih arasi TAM gun farki.
//!
//! NEDEN "fark" AYRI BIR EYLEM: Swift tarafinda olculen hata. Takvim aritmetigi
//! yapabilen hicbir arac yokken model soruyu yanitsiz birakmiyordu, UYDURUYORDU
//! (19 Temmuz -> 2 Aralik arasina "6 gun" dedi). Yetenegi vermeden "kendin
//! hesaplama" demek, kendinden emin uydurma uretir. Hesap araci bunu cozemez:
//! yalniz rakam ve operator bilir, artik yili ve ay uzunluklarini bilmez.
//!
//! COZULEMEYEN ZAMAN = BASARISIZLIK. Swift'te `?? simdi()` sessiz geri dusumu
//! KALDIRILDI, cunku 9 dilin 8'inde yanlis etkinlik yaratiyordu: model "0 gun"
//! ya da bugunun tarihini gorup bunu CEVAP saniyordu. Burada da her cozumsuz
//! girdi acik bir hata olarak doner; hicbir yol sessizce "simdi"ye dusmez.
//!
//! SIFIR BAGIMLILIK: chrono EKLENMEDI. Ihtiyacimiz olan tek sey Gregoryen
//! takvim <-> gun sayisi donusumu; asagidaki iki fonksiyon (`gun_sayisina`,
//! `gunden_tarihe`) bunu artik yil kurallariyla birlikte ~20 satirda yapiyor.
//! Bir tarih kitapligi ugruna binlerce satirlik bagimlilik cekmek, "her sey
//! elde yazilir" kimligiyle celisirdi.
//!
//! SAAT DILIMI ACIK, TAHMIN EDILMEZ. std yerel saat dilimini vermez ve
//! /etc/localtime okumaya kalkmak platforma gore sessizce yaniliyor — bu da
//! yukaridaki "sessiz yanlis etkinlik" hatasinin ta kendisi olurdu. Bu yuzden
//! ofset cagiran tarafindan VERILIR (varsayilan UTC) ve ciktida `tz=` alaniyla
//! acikca yazilir; model yanlis dilimi gorup duzeltebilsin.

use crate::yonlendirici::sadelestir;
use serde_json::Value;
use sirr_cekirdek::{
    Alan, Arac, AracBaglami, AracDurumu, AracGelecegi, AracHatasi, AracSonucu, ArgSema,
    IzGuncelleme, kutula,
};
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Takvim aritmetigi
// ---------------------------------------------------------------------------

/// Gregoryen tarihten 1970-01-01'e gore gun sayisina (Howard Hinnant algoritmasi).
/// Artik yil kurallari bolme islemlerinin icinde; ayri bir `artik_mi` dali yok,
/// o yuzden 400 yillik istisna da bedavaya dogru cikiyor.
fn gun_sayisina(yil: i64, ay: u32, gun: u32) -> i64 {
    let y = if ay <= 2 { yil - 1 } else { yil };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = ay as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + gun as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// `gun_sayisina`nin tersi.
fn gunden_tarihe(gun_sayisi: i64) -> (i64, u32, u32) {
    let z = gun_sayisi + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let gun = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let ay = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if ay <= 2 { y + 1 } else { y }, ay, gun)
}

fn artik_mi(yil: i64) -> bool {
    (yil % 4 == 0 && yil % 100 != 0) || yil % 400 == 0
}

fn aydaki_gun(yil: i64, ay: u32) -> u32 {
    match ay {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if artik_mi(yil) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Ingilizce gun adlari. DIL-NOTR CIKTI: model bunu kullanicinin diline cevirir.
/// Ciktiyi Turkcelestirmek, cok dilli akista modelin metni papagan gibi
/// tekrarlamasina yol aciyordu.
const GUN_ADLARI: [&str; 7] =
    ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// Takvim ani (belirli bir saat diliminde okunan duvar saati).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TarihSaat {
    pub yil: i64,
    pub ay: u32,
    pub gun: u32,
    pub saat: u32,
    pub dakika: u32,
    pub saniye: u32,
}

impl TarihSaat {
    /// Gecersiz bilesen (13. ay, 31 Subat, 25. saat) `None` doner. Kirpma YOK:
    /// 31 Subat'i 28 Subat'a yuvarlamak, kullanicinin yazim hatasini sessizce
    /// baska bir tarihe cevirir — tam da kacindigimiz sessiz sapma.
    pub fn yeni(yil: i64, ay: u32, gun: u32, saat: u32, dakika: u32, saniye: u32) -> Option<Self> {
        if !(1..=12).contains(&ay) || gun == 0 || gun > aydaki_gun(yil, ay) {
            return None;
        }
        if saat > 23 || dakika > 59 || saniye > 59 {
            return None;
        }
        Some(Self { yil, ay, gun, saat, dakika, saniye })
    }

    pub fn epoch(&self) -> i64 {
        gun_sayisina(self.yil, self.ay, self.gun) * 86_400
            + self.saat as i64 * 3600
            + self.dakika as i64 * 60
            + self.saniye as i64
    }

    pub fn epochtan(saniye: i64) -> Self {
        let gun_sayisi = saniye.div_euclid(86_400);
        let kalan = saniye.rem_euclid(86_400);
        let (yil, ay, gun) = gunden_tarihe(gun_sayisi);
        Self {
            yil,
            ay,
            gun,
            saat: (kalan / 3600) as u32,
            dakika: ((kalan % 3600) / 60) as u32,
            saniye: (kalan % 60) as u32,
        }
    }

    /// Gun basi. Fark hesabinda SART: iki uc gun basina indirgenmezse saat
    /// farki sonucu bir gun kaydirir ("yarin" 0 gun gorunur).
    pub fn gun_basi(&self) -> Self {
        Self { saat: 0, dakika: 0, saniye: 0, ..*self }
    }

    pub fn gun_numarasi(&self) -> i64 {
        gun_sayisina(self.yil, self.ay, self.gun)
    }

    /// 0 = Pazar. 1970-01-01 Persembe oldugu icin +4 kaydirma.
    pub fn hafta_gunu(&self) -> u32 {
        (self.gun_numarasi() + 4).rem_euclid(7) as u32
    }

    pub fn hafta_gunu_adi(&self) -> &'static str {
        GUN_ADLARI[self.hafta_gunu() as usize]
    }

    pub fn iso_tarih(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.yil, self.ay, self.gun)
    }

    pub fn iso_saat(&self) -> String {
        format!("{:02}:{:02}", self.saat, self.dakika)
    }

    fn gun_ekle(&self, gun: i64) -> Self {
        let (yil, ay, g) = gunden_tarihe(self.gun_numarasi() + gun);
        Self { yil, ay, gun: g, ..*self }
    }
}

// ---------------------------------------------------------------------------
// ZamanCozucu
// ---------------------------------------------------------------------------

/// Cozulen an + metinde ACIK bir saat bilgisi olup olmadigi.
///
/// `saat_var` ayri tasiniyor cunku "yarin" ile "yarin 14:00" ayni tipte iki
/// farkli sey: birincisinde saat 00:00 bir VARSAYILAN, ikincisinde VERI.
/// Cagiran bunu ayirt edemezse gun-bazli hatirlaticiyi gece yarisi etkinligine
/// cevirir.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cozum {
    pub an: TarihSaat,
    pub saat_var: bool,
}

/// Serbest metinden tarih/saat cikaran sirali cozucu.
///
/// SIRA ONEMLI ve daralan kesinlige gore dizili: once tek anlami olan bicimler
/// (ISO), sonra dil-notr sayisal kaliplar, en sonda dile bagli kestirmeler.
/// Ters sirada olsaydi "2026-07-20" icindeki tek basina duran sayilar Turkce
/// kestirme dalinda saat sanilirdi.
pub struct ZamanCozucu;

impl ZamanCozucu {
    /// `simdi` disaridan verilir (test edilebilirlik + tek saat okuma noktasi).
    /// `simdi` cagiranin saat diliminde okunmus duvar saatidir; cozum de ayni
    /// dilimde doner.
    pub fn coz(ham: &str, simdi: TarihSaat) -> Option<Cozum> {
        Self::coz_ofsetli(ham, simdi, 0)
    }

    /// `yerel_ofset_dk`: `simdi`nin UTC'ye gore ofseti. YALNIZCA girdi kendi
    /// saat dilimini tasidiginda ("...T18:00+03:00") kullanilir; o ani cagiranin
    /// duvar saatine cevirmek icin gerekir. Ofsetsiz metinler zaten cagiranin
    /// diliminde yazilmis sayilir — en az sasirtan varsayim.
    pub fn coz_ofsetli(ham: &str, simdi: TarihSaat, yerel_ofset_dk: i64) -> Option<Cozum> {
        let metin = ham.trim();
        if metin.is_empty() {
            return None;
        }
        // 1) ISO 8601 ve dil-notr sayisal kaliplar (ikisi de ayni cozumleyici:
        //    fark yalnizca ayirac ve alan sirasi).
        if let Some(c) = mutlak_coz(metin, yerel_ofset_dk) {
            return Some(c);
        }
        // 2) Turkce goreli gun kestirmeleri ("yarin 14:00", "sali", "haftaya cuma").
        if let Some(c) = turkce_kestirme(metin, simdi) {
            return Some(c);
        }
        // 3) Turkce ay adli tarihler ("2 aralik 2026", "20 temmuz").
        if let Some(c) = turkce_ay(metin, simdi) {
            return Some(c);
        }
        // GERI DUSUM YOK. Buraya gelen girdi cozulememistir ve cagiran bunu
        // hata olarak gormek ZORUNDA.
        None
    }

    /// Metinde acik saat izi var mi ("18:00", "18.30", "6 pm")?
    pub fn saat_izi(metin: &str) -> bool {
        let sade = sadelestir(metin);
        if saat_ara(&sade).is_some() {
            return true;
        }
        ["am", "pm", "oo", "os"].iter().any(|ek| {
            sade.split(|c: char| !c.is_ascii_alphanumeric())
                .any(|p| p == *ek || (p.len() > ek.len() && p.ends_with(ek)))
        })
    }
}

/// "2026-07-20T18:00:00+03:00", "2026-07-20 18:00", "20.07.2026", "20/07/2026 09:30".
fn mutlak_coz(metin: &str, yerel_ofset_dk: i64) -> Option<Cozum> {
    let mut govde = metin;
    let mut dis_ofset_dk: Option<i64> = None;

    if let Some(kalan) = govde.strip_suffix('Z').or_else(|| govde.strip_suffix('z')) {
        dis_ofset_dk = Some(0);
        govde = kalan;
    } else if let Some((kalan, ofset)) = son_ofset(govde) {
        dis_ofset_dk = Some(ofset);
        govde = kalan;
    }

    let (tarih_kismi, saat_kismi) = if let Some(i) = govde.find(['T', 't']) {
        (&govde[..i], Some(govde[i + 1..].trim()))
    } else if let Some(i) = govde.find(' ') {
        (&govde[..i], Some(govde[i + 1..].trim()))
    } else {
        (govde, None)
    };

    let (yil, ay, gun) = tarih_parcasi(tarih_kismi.trim())?;

    let (saat, dakika, saniye, saat_var) = match saat_kismi {
        Some(s) if !s.is_empty() => {
            let (h, m, sn) = saat_parcasi(s)?;
            (h, m, sn, true)
        }
        // Saat yoksa dis ofset de anlamsizdir: "2026-07-20+03:00" gibi bir sey
        // gelirse tarih parcasi zaten bozulur, buraya dusmez.
        _ => (0, 0, 0, false),
    };

    let an = TarihSaat::yeni(yil, ay, gun, saat, dakika, saniye)?;
    // Girdi kendi saat dilimini tasiyorsa once mutlak ana cevir, sonra cagiranin
    // dilimine geri oku.
    let an = match dis_ofset_dk {
        Some(dis) if saat_var => TarihSaat::epochtan(an.epoch() - dis * 60 + yerel_ofset_dk * 60),
        _ => an,
    };
    Some(Cozum { an, saat_var })
}

/// Sondaki "+03:00" / "-0500" / "+03" ofsetini ayirir. Yalnizca 'T' ayraci olan
/// metinlerde aranir: "20-07-2026" gibi bir tarihin tiresini ofset sanmayalim.
fn son_ofset(s: &str) -> Option<(&str, i64)> {
    let t = s.rfind(['T', 't'])?;
    let p = s[t..].rfind(['+', '-'])?;
    let bas = t + p;
    let isaret: i64 = if s.as_bytes()[bas] == b'+' { 1 } else { -1 };
    let o = &s[bas + 1..];
    let (h, m) = if let Some((a, b)) = o.split_once(':') {
        (a, b)
    } else if o.len() == 4 {
        (&o[..2], &o[2..])
    } else if o.len() == 2 {
        (o, "0")
    } else {
        return None;
    };
    let h: i64 = h.parse().ok()?;
    let m: i64 = m.parse().ok()?;
    if h > 14 || m > 59 {
        return None;
    }
    Some((&s[..bas], isaret * (h * 60 + m)))
}

/// "yyyy-MM-dd", "yyyy/MM/dd", "dd.MM.yyyy", "dd/MM/yyyy".
///
/// GUN/AY SIRASI: dort haneli parca nerede duruyorsa yil odur; kalan iki parca
/// dd/MM okunur (Swift ile ayni karar). MM/dd tahmini EKLENMEDI: "03/04" iki
/// yorumda da gecerli bir tarih verir, yani yanlis yorum sessizce basarili
/// olur — cozulemeyen girdiden daha tehlikeli.
fn tarih_parcasi(s: &str) -> Option<(i64, u32, u32)> {
    let ayr = s.chars().find(|c| *c == '-' || *c == '/' || *c == '.')?;
    let p: Vec<&str> = s.split(ayr).collect();
    if p.len() != 3 || p.iter().any(|x| x.is_empty() || !x.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let (yil, ay, gun) = if p[0].len() == 4 {
        (p[0], p[1], p[2])
    } else if p[2].len() == 4 {
        (p[2], p[1], p[0])
    } else {
        return None;
    };
    Some((yil.parse().ok()?, ay.parse().ok()?, gun.parse().ok()?))
}

/// "HH:mm" veya "HH:mm:ss".
fn saat_parcasi(s: &str) -> Option<(u32, u32, u32)> {
    let p: Vec<&str> = s.split(':').collect();
    if !(2..=3).contains(&p.len())
        || p.iter().any(|x| x.is_empty() || !x.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let saniye = if p.len() == 3 { p[2].parse().ok()? } else { 0 };
    Some((p[0].parse().ok()?, p[1].parse().ok()?, saniye))
}

/// Turkce goreli gun + istege bagli saat. "bugun", "yarin", "obur gun", "dun",
/// ve hafta gunu adlari ("sali", "haftaya cuma").
fn turkce_kestirme(ham: &str, simdi: TarihSaat) -> Option<Cozum> {
    let metin = sadelestir(ham);

    let gun_ofseti = if metin.contains("obur gun") {
        Some(2)
    } else if metin.contains("yarin") {
        Some(1)
    } else if metin.contains("bugun") {
        Some(0)
    } else if metin.contains("dun") {
        Some(-1)
    } else {
        hafta_gunu_ofseti(&metin, simdi)
    }?;

    let hedef = simdi.gun_basi().gun_ekle(gun_ofseti);

    // Saat: once "18:00"/"18.30", yoksa gun ACIKCA belirtildigi icin tek basina
    // duran sayiyi guvenle saat sayabiliriz ("yarin 9").
    if let Some((h, m)) = saat_ara(&metin) {
        return TarihSaat::yeni(hedef.yil, hedef.ay, hedef.gun, h, m, 0)
            .map(|an| Cozum { an, saat_var: true });
    }
    if let Some(h) = yalin_saat(&metin) {
        return TarihSaat::yeni(hedef.yil, hedef.ay, hedef.gun, h, 0, 0)
            .map(|an| Cozum { an, saat_var: true });
    }
    Some(Cozum { an: hedef, saat_var: false })
}

/// Hafta gunu adindan gun ofseti.
///
/// "sali" = BUGUNDEN SONRAKI ilk sali (1..=7). Bugunu dahil etmiyoruz: gun ici
/// bir konusmada "sali" cogunlukla gelecegi kasteder, bugunu secmek etkinligi
/// gecmis bir saate koyma riski tasir. "haftaya/gelecek" oneki bir hafta ekler.
///
/// Uzun adlar once denenir: "pazartesi" ayni zamanda "pazar" icerir,
/// "cumartesi" ayni zamanda "cuma" icerir — kisa olan once bakilsa Pazartesi
/// sessizce Pazar olurdu.
fn hafta_gunu_ofseti(sade: &str, simdi: TarihSaat) -> Option<i64> {
    const GUNLER: [(&str, u32); 7] = [
        ("cumartesi", 6),
        ("pazartesi", 1),
        ("persembe", 4),
        ("carsamba", 3),
        ("cuma", 5),
        ("pazar", 0),
        ("sali", 2),
    ];
    let hedef = GUNLER.iter().find(|(ad, _)| sade.contains(ad)).map(|(_, n)| *n)?;
    let bugun = simdi.hafta_gunu() as i64;
    let mut ofset = (hedef as i64 - bugun).rem_euclid(7);
    if ofset == 0 {
        ofset = 7;
    }
    if sade.contains("haftaya") || sade.contains("gelecek") {
        ofset += 7;
    }
    Some(ofset)
}

/// Turkce ay adli tarih: "2 aralik 2026", "20 temmuz", "aralik 2".
///
/// Yil verilmemisse icinde bulundugumuz yil alinir; o tarih GECMISTE kaliyorsa
/// bir yil eklenir. Gerekce: bu arac cogunlukla ileriye donuk soruda cagriliyor
/// ("kac gun kaldi"); Aralik'ta "3 ocak" diyen kullanici gecen Ocak'i kastetmez.
fn turkce_ay(ham: &str, simdi: TarihSaat) -> Option<Cozum> {
    const AYLAR: [&str; 12] = [
        "ocak", "subat", "mart", "nisan", "mayis", "haziran", "temmuz", "agustos", "eylul",
        "ekim", "kasim", "aralik",
    ];
    let sade = sadelestir(ham);
    let ay = AYLAR.iter().position(|a| sade.contains(a)).map(|i| i as u32 + 1)?;

    let saat = saat_ara(&sade);
    // Saat bulunduysa onun rakamlari gun/yil sanilmasin diye metinden cikarilir.
    let temiz = match saat {
        Some((h, m)) => sade.replace(&format!("{h}:{m:02}"), " ").replace(&format!("{h}.{m:02}"), " "),
        None => sade.clone(),
    };

    let mut gun = None;
    let mut yil = None;
    for parca in temiz.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()) {
        match parca.len() {
            1 | 2 if gun.is_none() => gun = parca.parse::<u32>().ok(),
            4 if yil.is_none() => yil = parca.parse::<i64>().ok(),
            _ => {}
        }
    }
    let gun = gun?;

    let (an, saat_var) = match saat {
        Some((h, m)) => (TarihSaat::yeni(yil.unwrap_or(simdi.yil), ay, gun, h, m, 0)?, true),
        None => (TarihSaat::yeni(yil.unwrap_or(simdi.yil), ay, gun, 0, 0, 0)?, false),
    };
    if yil.is_none() && an.gun_numarasi() < simdi.gun_numarasi() {
        let ileri = TarihSaat::yeni(an.yil + 1, ay, gun, an.saat, an.dakika, 0)?;
        return Some(Cozum { an: ileri, saat_var });
    }
    Some(Cozum { an, saat_var })
}

/// "18:00" / "18.30" kalibini elle tarar (regex bagimliligi yok).
///
/// Tarih kaciran koruma: dakikanin ardindan yeniden ayirac+rakam geliyorsa
/// ("20.07.2026") bu bir tarihtir, saat degil — atlanir.
fn saat_ara(sade: &str) -> Option<(u32, u32)> {
    let b = sade.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let bas = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let uzunluk = i - bas;
        if uzunluk <= 2 && i < b.len() && (b[i] == b':' || b[i] == b'.') {
            let dk_bas = i + 1;
            let mut j = dk_bas;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let tarih_gibi = j < b.len() && (b[j] == b'.' || b[j] == b'/' || b[j] == b'-');
            if j - dk_bas == 2 && !tarih_gibi {
                let saat: u32 = sade[bas..bas + uzunluk].parse().ok()?;
                let dakika: u32 = sade[dk_bas..j].parse().ok()?;
                if saat <= 23 && dakika <= 59 {
                    return Some((saat, dakika));
                }
            }
            i = j;
        }
    }
    None
}

/// Tek basina duran 1-2 haneli sayi ("yarin 9"). Yalnizca gun ACIKCA
/// belirlendiginde cagrilir; aksi halde her sayiyi saat sanardi.
fn yalin_saat(sade: &str) -> Option<u32> {
    let b = sade.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let bas = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let onceki_ayrac = bas > 0 && matches!(b[bas - 1], b':' | b'.' | b'/' | b'-');
        let sonraki_ayrac = i < b.len() && matches!(b[i], b':' | b'.' | b'/' | b'-');
        if i - bas <= 2 && !onceki_ayrac && !sonraki_ayrac {
            let s: u32 = sade[bas..i].parse().ok()?;
            if s <= 23 {
                return Some(s);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Arac
// ---------------------------------------------------------------------------

/// Aracin donebilecegi bilgi turu.
///
/// SERBEST METIN DEGIL: Swift'te olculen hata, `tur.lowercased() == "fark"`
/// disindaki HER degerin sessizce "hepsi"ye dusmesiydi — "difference" yazan
/// model gun farki yerine saat/tarih aliyordu. Kapali kume, modelin altinci bir
/// deger uydurmasini gramer duzeyinde imkansiz kilar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tur {
    Saat,
    Tarih,
    Gun,
    Hepsi,
    Fark,
}

impl Tur {
    pub const HEPSI: [&'static str; 5] = ["saat", "tarih", "gun", "hepsi", "fark"];

    pub fn coz(ham: &str) -> Option<Self> {
        match ham {
            "saat" => Some(Tur::Saat),
            "tarih" => Some(Tur::Tarih),
            "gun" => Some(Tur::Gun),
            "hepsi" => Some(Tur::Hepsi),
            "fark" => Some(Tur::Fark),
            _ => None,
        }
    }
}

/// Su anki tarih/saat ve gun farki araci.
///
/// Durumsuz ve ag kullanmaz. `sabit_epoch` yalnizca test/eval icindir: gercek
/// saate bagli test belirlenimci olamaz.
pub struct ZamanAraci {
    ofset_dakika: i64,
    sabit_epoch: Option<i64>,
}

impl Default for ZamanAraci {
    fn default() -> Self {
        Self::yeni()
    }
}

impl ZamanAraci {
    /// Varsayilan dilim UTC. Bkz. dosya basi: dilim tahmin EDILMEZ.
    pub fn yeni() -> Self {
        Self { ofset_dakika: 0, sabit_epoch: None }
    }

    /// UTC'ye gore dakika cinsinden ofset (orn. Turkiye icin 180).
    pub fn ofset_dakika(mut self, dakika: i64) -> Self {
        self.ofset_dakika = dakika;
        self
    }

    /// Sabit "simdi" — eval ve birim testleri icin.
    pub fn sabit_epoch(mut self, epoch: i64) -> Self {
        self.sabit_epoch = Some(epoch);
        self
    }

    /// Cagiranin dilimindeki duvar saati.
    pub fn simdi(&self) -> TarihSaat {
        let epoch = self.sabit_epoch.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                // Sistem saati 1970 oncesine ayarliysa panik yerine epoch'a
                // duseriz: bir arac cagrisi tum akisi dusurmemeli.
                .unwrap_or(0)
        });
        TarihSaat::epochtan(epoch + self.ofset_dakika * 60)
    }

    fn tz_metni(&self) -> String {
        let isaret = if self.ofset_dakika < 0 { '-' } else { '+' };
        let m = self.ofset_dakika.abs();
        format!("UTC{isaret}{:02}:{:02}", m / 60, m % 60)
    }

    /// Dil-notr anlik bilgi. Model bunu kullanicinin diline cevirir.
    pub fn simdi_metni(&self, tur: Tur) -> String {
        let an = self.simdi();
        let (saat, tarih, gun, tz) =
            (an.iso_saat(), an.iso_tarih(), an.hafta_gunu_adi(), self.tz_metni());
        match tur {
            Tur::Saat => format!("time={saat} tz={tz}"),
            Tur::Tarih => format!("date={tarih}"),
            Tur::Gun => format!("weekday={gun}"),
            // Fark buraya gelmez (cagiran ayirir) ama sessiz bir `_` dali
            // birakmiyoruz: yeni bir varyant eklendiginde derleyici uyarsin.
            Tur::Hepsi | Tur::Fark => {
                format!("time={saat} date={tarih} weekday={gun} tz={tz}")
            }
        }
    }

    /// Bugun ile hedef arasindaki TAM gun sayisi. Iki uc da gun basina indirgenir.
    /// Gecmis tarihte NEGATIF doner — isaret bilincli korunur ki model yonu
    /// uydurmak zorunda kalmasin.
    pub fn fark_metni(&self, hedef_ham: &str) -> Result<String, String> {
        let simdi = self.simdi();
        // Dis ofsetli ISO girdisini yerel duvar saatine cevirebilmesi icin
        // cozucuye aracin ofseti bildirilir.
        let Some(cozum) = ZamanCozucu::coz_ofsetli(hedef_ham, simdi, self.ofset_dakika) else {
            // SESSIZCE BUGUNE DUSMEK YOK: model "0 gun" gorup bunu cevap sanar.
            return Err(format!(
                "error: unparsable_date \"{hedef_ham}\". Nothing was computed. \
                 Call the tool again with \"hedef\" as an ISO 8601 date, e.g. 2026-12-02."
            ));
        };
        let bugun = simdi.gun_basi();
        let hedef = cozum.an.gun_basi();
        Ok(format!(
            "from={} to={} days={}",
            bugun.iso_tarih(),
            hedef.iso_tarih(),
            hedef.gun_numarasi() - bugun.gun_numarasi()
        ))
    }
}

impl Arac for ZamanAraci {
    fn ad(&self) -> &str {
        "zaman"
    }

    fn aciklama(&self) -> &str {
        "Gives the current date/time/day of week, and counts days between today and another \
         date. Call this whenever the user asks for the current time/date OR asks how many \
         days/weeks until (or since) a date - in any language. NEVER compute a date difference \
         yourself: calendar arithmetic needs leap years and month lengths, so it must be \
         calculated here."
    }

    fn sema(&self) -> ArgSema {
        ArgSema::nesne(vec![
            Alan::yeni(
                "tur",
                ArgSema::secenek(Tur::HEPSI).aciklama(
                    "What to return: 'saat' (time), 'tarih' (date), 'gun' (day of week), \
                     'hepsi' (all), or 'fark' (days until/since the date given in 'hedef'). \
                     If unsure use 'hepsi'.",
                ),
            )
            .zorunlu(),
            Alan::yeni(
                "hedef",
                ArgSema::metin().aciklama(
                    "Only for tur='fark': the other date, exactly as the user wrote it \
                     (e.g. '2 aralik 2026', '2026-12-02', 'yarin'). Leave empty otherwise.",
                ),
            ),
        ])
    }

    /// Saat okumak kisisel veri okumak degildir; oturumu kirletmez.
    fn kirletir_mi(&self) -> bool {
        false
    }

    fn calistir<'a>(&'a self, args: Value, ctx: &'a mut AracBaglami) -> AracGelecegi<'a> {
        kutula(async move {
            let Some(tur_ham) = args.get("tur").and_then(|v| v.as_str()) else {
                return AracSonucu::basarisiz(&AracHatasi::EksikAlan("tur".into()));
            };
            let Some(tur) = Tur::coz(tur_ham) else {
                return AracSonucu::basarisiz(&AracHatasi::GecersizArguman(format!(
                    "bilinmeyen tur \"{tur_ham}\""
                )));
            };

            // "Saat kac" CIPSIZ kalir: onemsiz, akisi kalabaliklastirir.
            if tur != Tur::Fark {
                let cikti = self.simdi_metni(tur);
                return AracSonucu::okundu("", cikti.clone()).ham_cikti(cikti);
            }

            // "fark" CIP DUSURUR: bir SAYI uretiyor ve o sayi ayristirilmis bir
            // girdiye dayaniyor, yani tarih yanlis okunmus olabilir. Cip
            // detayinda "from=... to=... days=..." gorunur; kullanici yanlis
            // ayristirmayi yakalar. Dogrulanmasi gereken bir sayiyi gizlemek
            // "sirr yaptigini gizlemez" ilkesini delerdi.
            let hedef = args.get("hedef").and_then(|v| v.as_str()).unwrap_or("").trim();
            let iz = ctx.cip_baslat("calendar", "Günler sayılıyor");

            if hedef.is_empty() {
                let hata = AracHatasi::EksikAlan("hedef".into());
                ctx.cip_guncelle(
                    iz,
                    IzGuncelleme::durum(AracDurumu::Basarisiz(hata.kisa_hata()))
                        .metin("Tarih verilmedi"),
                );
                return AracSonucu::basarisiz(&hata);
            }

            match self.fark_metni(hedef) {
                Ok(cikti) => {
                    ctx.cip_guncelle(
                        iz,
                        IzGuncelleme::durum(AracDurumu::Okundu)
                            .metin("Gün farkı hesaplandı")
                            .ham_girdi(hedef)
                            .ham_cikti(cikti.clone()),
                    );
                    AracSonucu::okundu("Gün farkı hesaplandı", cikti.clone()).ham_cikti(cikti)
                }
                // BILINCLI OLARAK `basarisiz()` DEGIL: cekirdegin sabit
                // HATA_MODEL_METNI'i ic arizalar icin bilerek suskundur. Burada
                // ise durum kurtarilabilir bir GIRDI sorunu; modelin ne yapmasi
                // gerektigini (ISO 8601 ile tekrar cagir) ogrenmesi sart, yoksa
                // ayni cozumsuz metinle donup durur.
                Err(mesaj) => {
                    let cip = "Tarih anlaşılmadı";
                    ctx.cip_guncelle(
                        iz,
                        IzGuncelleme::durum(AracDurumu::Basarisiz(cip.into()))
                            .metin(cip)
                            .ham_girdi(hedef)
                            .ham_cikti(mesaj.clone()),
                    );
                    AracSonucu::yeni(cip, AracDurumu::Basarisiz(cip.into()), mesaj.clone())
                        .ham_cikti(mesaj)
                }
            }
        })
    }
}

#[cfg(test)]
mod testler {
    use super::*;
    use sirr_cekirdek::{BellekVeriDeposu, SessizRaporlayici};
    use std::sync::Arc;
    use std::task::{Context, Poll};

    /// 2026-07-20 12:00:00 UTC — bir Pazartesi.
    const SIMDI: i64 = 1_784_548_800;

    fn an(yil: i64, ay: u32, gun: u32, saat: u32, dakika: u32) -> TarihSaat {
        TarihSaat::yeni(yil, ay, gun, saat, dakika, 0).expect("gecerli an")
    }

    fn simdi() -> TarihSaat {
        an(2026, 7, 20, 12, 0)
    }

    /// tokio yok (ag/calistirici bagimliligi cekmiyoruz); `Waker::noop` ile
    /// minimal poll dongusu yeter — araclarimiz gercek bir uyandirmaya dayanmaz.
    fn calistir_bekle(gelecek: AracGelecegi<'_>) -> AracSonucu {
        let mut gelecek = gelecek;
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        loop {
            if let Poll::Ready(sonuc) = gelecek.as_mut().poll(&mut cx) {
                return sonuc;
            }
        }
    }

    fn baglam() -> AracBaglami {
        AracBaglami::yeni(Arc::new(BellekVeriDeposu::yeni()), ".", Arc::new(SessizRaporlayici))
    }

    #[test]
    fn simdi_pazartesi_ve_gidis_donus_tutarli() {
        let a = TarihSaat::epochtan(SIMDI);
        assert_eq!(a.iso_tarih(), "2026-07-20");
        assert_eq!(a.iso_saat(), "12:00");
        assert_eq!(a.hafta_gunu_adi(), "Monday");
        assert_eq!(a.epoch(), SIMDI);
        // Artik yil ve yuzyil sinirlari dahil genis bir aralikta gidis-donus.
        for gun in [-25_000_i64, -1, 0, 1, 19_000, 20_500, 40_000] {
            let e = gun * 86_400 + 3661;
            assert_eq!(TarihSaat::epochtan(e).epoch(), e, "gun={gun}");
        }
        assert!(artik_mi(2024) && !artik_mi(2100) && artik_mi(2000));
        assert_eq!(aydaki_gun(2024, 2), 29);
        assert_eq!(aydaki_gun(2026, 2), 28);
    }

    #[test]
    fn iso_8601_cozulur() {
        let c = ZamanCozucu::coz("2026-12-02T18:30:00", simdi()).expect("iso");
        assert_eq!(c.an, an(2026, 12, 2, 18, 30));
        assert!(c.saat_var);

        // Saatsiz ISO: saat VERI degil VARSAYILAN.
        let g = ZamanCozucu::coz("2026-12-02", simdi()).expect("iso tarih");
        assert_eq!(g.an, an(2026, 12, 2, 0, 0));
        assert!(!g.saat_var);

        // Z ve acik ofset: arac UTC'de oldugu icin +03:00 uc saat geri okunur.
        assert_eq!(ZamanCozucu::coz("2026-12-02T18:30:00Z", simdi()).unwrap().an.iso_saat(), "18:30");
        assert_eq!(
            ZamanCozucu::coz("2026-12-02T18:30:00+03:00", simdi()).unwrap().an.iso_saat(),
            "15:30"
        );
    }

    #[test]
    fn dil_notr_kaliplar_cozulur() {
        for (metin, beklenen, saatli) in [
            ("2026-12-02 18:30", an(2026, 12, 2, 18, 30), true),
            ("2026/12/02 18:30", an(2026, 12, 2, 18, 30), true),
            ("02.12.2026 18:30", an(2026, 12, 2, 18, 30), true),
            ("02/12/2026 09:05", an(2026, 12, 2, 9, 5), true),
            ("02.12.2026", an(2026, 12, 2, 0, 0), false),
            ("2026/12/02", an(2026, 12, 2, 0, 0), false),
        ] {
            let c = ZamanCozucu::coz(metin, simdi()).unwrap_or_else(|| panic!("{metin}"));
            assert_eq!(c.an, beklenen, "{metin}");
            assert_eq!(c.saat_var, saatli, "{metin}");
        }
        // Takvimde olmayan gun kirpilmaz, REDDEDILIR.
        assert!(ZamanCozucu::coz("2026-02-31", simdi()).is_none());
        assert!(ZamanCozucu::coz("2026-13-01", simdi()).is_none());
        // 2024 artik yil: 29 Subat gecerli, 2026'da degil.
        assert!(ZamanCozucu::coz("2024-02-29", simdi()).is_some());
        assert!(ZamanCozucu::coz("2026-02-29", simdi()).is_none());
    }

    #[test]
    fn turkce_kestirmeler_cozulur() {
        // 2026-07-20 Pazartesi.
        let c = ZamanCozucu::coz("yarın 14:00", simdi()).expect("yarin");
        assert_eq!(c.an, an(2026, 7, 21, 14, 0));
        assert!(c.saat_var);

        // Turkce karaktersiz yazim ayni sonucu vermeli.
        assert_eq!(ZamanCozucu::coz("yarin 14:00", simdi()).unwrap().an, c.an);

        // Gun acikca verildigi icin yalin sayi saat sayilir.
        assert_eq!(ZamanCozucu::coz("öbür gün 9", simdi()).unwrap().an, an(2026, 7, 22, 9, 0));

        // Saatsiz kestirme: gun basi + saat_var=false.
        let b = ZamanCozucu::coz("bugün", simdi()).expect("bugun");
        assert_eq!(b.an, an(2026, 7, 20, 0, 0));
        assert!(!b.saat_var);

        assert_eq!(ZamanCozucu::coz("dün", simdi()).unwrap().an, an(2026, 7, 19, 0, 0));
    }

    #[test]
    fn hafta_gunu_ileriye_atar() {
        // Bugun Pazartesi. "sali" -> ertesi gun.
        assert_eq!(ZamanCozucu::coz("salı 14:00", simdi()).unwrap().an, an(2026, 7, 21, 14, 0));
        // "pazartesi" bugune degil, GELECEK haftaya gider (bugun dahil degil).
        assert_eq!(ZamanCozucu::coz("pazartesi", simdi()).unwrap().an, an(2026, 7, 27, 0, 0));
        // Uzun ad kisa adi yutmamali: pazartesi != pazar, cumartesi != cuma.
        assert_eq!(ZamanCozucu::coz("pazar", simdi()).unwrap().an, an(2026, 7, 26, 0, 0));
        assert_eq!(ZamanCozucu::coz("cuma", simdi()).unwrap().an, an(2026, 7, 24, 0, 0));
        assert_eq!(ZamanCozucu::coz("cumartesi", simdi()).unwrap().an, an(2026, 7, 25, 0, 0));
        // "haftaya" bir hafta ekler.
        assert_eq!(ZamanCozucu::coz("haftaya salı", simdi()).unwrap().an, an(2026, 7, 28, 0, 0));
    }

    #[test]
    fn turkce_ay_adlari_cozulur() {
        assert_eq!(ZamanCozucu::coz("2 aralık 2026", simdi()).unwrap().an, an(2026, 12, 2, 0, 0));
        let saatli = ZamanCozucu::coz("2 aralık 2026 18:30", simdi()).expect("saatli");
        assert_eq!(saatli.an, an(2026, 12, 2, 18, 30));
        assert!(saatli.saat_var);
        // Yil yoksa icinde bulunulan yil; gecmisteyse bir sonraki yil.
        assert_eq!(ZamanCozucu::coz("20 aralık", simdi()).unwrap().an, an(2026, 12, 20, 0, 0));
        assert_eq!(ZamanCozucu::coz("3 ocak", simdi()).unwrap().an, an(2027, 1, 3, 0, 0));
    }

    #[test]
    fn cozulemeyen_zaman_sessizce_simdiye_dusmez() {
        for ham in ["", "   ", "lorem ipsum", "zzz", "kirmizi araba", "99/99/9999"] {
            assert!(ZamanCozucu::coz(ham, simdi()).is_none(), "cozulmemeliydi: {ham:?}");
        }
    }

    #[test]
    fn fark_artik_yili_ve_ay_uzunlugunu_dogru_sayar() {
        // Modelin uyduramayacagi noktalar: 29 Subat ve ay siniri.
        let arac = ZamanAraci::yeni().sabit_epoch(gun_sayisina(2024, 2, 28) * 86_400);
        assert_eq!(arac.fark_metni("2024-03-01").unwrap(), "from=2024-02-28 to=2024-03-01 days=2");

        let arac2 = ZamanAraci::yeni().sabit_epoch(gun_sayisina(2023, 2, 28) * 86_400);
        assert_eq!(arac2.fark_metni("2023-03-01").unwrap(), "from=2023-02-28 to=2023-03-01 days=1");

        // Swift'te modelin yanlis yanitladigi vaka: 19 Temmuz -> 2 Aralik.
        let arac3 = ZamanAraci::yeni().sabit_epoch(gun_sayisina(2026, 7, 19) * 86_400);
        assert_eq!(arac3.fark_metni("2 aralık 2026").unwrap(), "from=2026-07-19 to=2026-12-02 days=136");
    }

    #[test]
    fn fark_gecmiste_negatif_doner() {
        let arac = ZamanAraci::yeni().sabit_epoch(SIMDI);
        // Isaret korunur: model "gecti mi" sorusunu uydurmadan yanitlasin.
        assert_eq!(arac.fark_metni("2026-07-10").unwrap(), "from=2026-07-20 to=2026-07-10 days=-10");
        // Saat farki gunu KAYDIRMAMALI: 12:00'de sorulsa da yarin 1 gundur.
        assert!(arac.fark_metni("2026-07-21T01:00").unwrap().ends_with("days=1"));
    }

    #[test]
    fn fark_cozulemezse_yonlendirici_hata_doner() {
        let arac = ZamanAraci::yeni().sabit_epoch(SIMDI);
        let hata = arac.fark_metni("mavi kedi").expect_err("hata beklenir");
        assert!(hata.starts_with("error: unparsable_date"), "{hata}");
        // Modelin ne yapacagini bilmesi sart, yoksa ayni girdiyle doner durur.
        assert!(hata.contains("ISO 8601"), "{hata}");
        assert!(!hata.contains("days="), "gun sayisi uydurulmamali: {hata}");
    }

    #[test]
    fn arac_fark_calistirir_ve_cip_duser() {
        let arac = ZamanAraci::yeni().sabit_epoch(SIMDI);
        let mut ctx = baglam();
        let args = serde_json::json!({ "tur": "fark", "hedef": "2026-12-02" });
        let sonuc = calistir_bekle(arac.calistir(args, &mut ctx));
        assert_eq!(sonuc.durum, AracDurumu::Okundu);
        assert_eq!(sonuc.modele_donen, "from=2026-07-20 to=2026-12-02 days=135");
        assert!(!sonuc.cip_metni.is_empty(), "fark cip dusurmeli");
    }

    #[test]
    fn arac_cozulemeyen_tarihte_basarisiz_durum_doner() {
        let arac = ZamanAraci::yeni().sabit_epoch(SIMDI);
        let mut ctx = baglam();
        let args = serde_json::json!({ "tur": "fark", "hedef": "mavi kedi" });
        let sonuc = calistir_bekle(arac.calistir(args, &mut ctx));
        assert!(matches!(sonuc.durum, AracDurumu::Basarisiz(_)));
        assert!(sonuc.modele_donen.contains("unparsable_date"), "{}", sonuc.modele_donen);
        assert!(!sonuc.modele_donen.contains("days="));

        // Bos hedef de sessizce bugune dusmez.
        let bos = calistir_bekle(
            arac.calistir(serde_json::json!({ "tur": "fark", "hedef": "" }), &mut ctx),
        );
        assert!(matches!(bos.durum, AracDurumu::Basarisiz(_)));
    }

    #[test]
    fn arac_anlik_bilgiyi_dil_notr_verir() {
        let arac = ZamanAraci::yeni().sabit_epoch(SIMDI).ofset_dakika(180);
        assert_eq!(arac.simdi_metni(Tur::Tarih), "date=2026-07-20");
        assert_eq!(arac.simdi_metni(Tur::Gun), "weekday=Monday");
        assert_eq!(arac.simdi_metni(Tur::Saat), "time=15:00 tz=UTC+03:00");
        assert_eq!(
            arac.simdi_metni(Tur::Hepsi),
            "time=15:00 date=2026-07-20 weekday=Monday tz=UTC+03:00"
        );
        // Negatif ofset de dogru bicimlenir.
        assert!(ZamanAraci::yeni().ofset_dakika(-330).tz_metni() == "UTC-05:30");
    }

    #[test]
    fn arac_gecersiz_tur_ile_hata_doner() {
        let arac = ZamanAraci::yeni().sabit_epoch(SIMDI);
        let mut ctx = baglam();
        // "difference" gibi uydurma bir deger sessizce "hepsi"ye DUSMEMELI.
        let sonuc =
            calistir_bekle(arac.calistir(serde_json::json!({ "tur": "difference" }), &mut ctx));
        assert!(matches!(sonuc.durum, AracDurumu::Basarisiz(_)));
        let eksik = calistir_bekle(arac.calistir(serde_json::json!({}), &mut ctx));
        assert!(matches!(eksik.durum, AracDurumu::Basarisiz(_)));
    }

    #[test]
    fn sema_modeli_kapali_kumeye_zorlar() {
        let sema = ZamanAraci::yeni().sema();
        let js = sema.json_schema();
        assert_eq!(js["additionalProperties"], serde_json::json!(false));
        assert_eq!(js["required"], serde_json::json!(["tur"]));
        assert_eq!(js["properties"]["tur"]["enum"], serde_json::json!(Tur::HEPSI));
        assert!(sema.dogrula(&serde_json::json!({ "tur": "fark", "hedef": "x" })).is_ok());
        assert!(sema.dogrula(&serde_json::json!({ "tur": "difference" })).is_err());
        assert!(sema.dogrula(&serde_json::json!({ "hedef": "x" })).is_err());
    }

    #[test]
    fn saat_izi_ayirt_edilir() {
        assert!(ZamanCozucu::saat_izi("yarın 18:00"));
        assert!(ZamanCozucu::saat_izi("yarın 18.30"));
        assert!(ZamanCozucu::saat_izi("tomorrow 6 pm"));
        assert!(!ZamanCozucu::saat_izi("yarın"));
        // Tarih saat sanilmamali.
        assert!(!ZamanCozucu::saat_izi("20.07.2026"));
    }
}
