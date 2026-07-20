//! Tetikleyici eslestirmesinin TEK kurali.
//!
//! Burasi hem beceri hem hafiza katmaninin kullandigi ortak metin kuralidir.
//! Iki yere kopyalanmadi cunku kural bir SAYI degil bir DAVRANIS: kopyalar
//! ayrisirsa "ayni kural" iddiasi sessizce yalan olur ve kimse fark etmez —
//! hafiza notlari beceriden farkli eslesmeye baslar. Bu yuzden `sirr-hafiza`
//! bu modul icin `sirr-beceri`ye bagimlidir; yon keyfi degil, kuralin dogdugu
//! yer beceri katmanidir (olcum orada yapildi).

/// Sozcuk BASI eslesmenin yetmedigi uzunluk siniri; altindakiler TAM SOZCUK ister.
///
/// Olculen hata: "ara" tetikleyicisi "**ara**lik"i yakaliyor, her Aralik ayi
/// sorusu arama becerisini cagiriyor ve 4096 butcesinden ~700 karakter yiyordu.
/// Ayni tuzak "bul" -> "bulut/bulmaca"da da var.
///
/// Neden yalniz KISA tetikleyicilerde: Turkce SONA ek alir, uzun tetikleyicide
/// on-ek eslesmesi SART ("satir" -> "satirini", "carp" -> "carparsak"). Kisa bir
/// kok ise akrabasi olmayan sozcuklerin de basinda durabiliyor.
///
/// Esik 4: uc harfli kokler (ara/bul/oku/dok) tam sozcuk ister, 4+ on-eki surdurur.
/// Esigi 5 yapmak "carp"i kirardi.
///
/// Asimetri bilincli: YANLIS beceri enjekte etmek modeli yaniltir ve butceden
/// ~700 karakter yer; beceriyi KACIRMAK yalnizca fazladan kilavuzu kaybettirir —
/// arac zaten oturumda durur. Kacirmak ucuz, yanlis eslesmek pahali.
pub const TAM_SOZCUK_SINIRI: usize = 4;

/// Turkce'ye duyarli kucultme.
///
/// `str::to_lowercase` Unicode varsayilanini uygular: 'I' -> 'i' ve
/// 'İ' -> "i\u{307}" (i + birlesen nokta). Ikisi de Turkce'de YANLIS ve
/// eslestirmeyi bozar: "Istanbul" tetikleyici "istanbul" ile tutmali,
/// "İSTANBUL" ise iki kod noktasina dagilip hic tutmamali degil. Bu iki
/// harfi elle esliyoruz, gerisi Unicode'a birakiliyor.
pub fn kucult(metin: &str) -> String {
    let mut cikti = String::with_capacity(metin.len());
    for k in metin.chars() {
        match k {
            'I' => cikti.push('ı'),
            'İ' => cikti.push('i'),
            _ => {
                for kucuk in k.to_lowercase() {
                    cikti.push(kucuk);
                }
            }
        }
    }
    cikti
}

/// `metin` (ZATEN kucultulmus olmali) `tetik`i sozcuk sinirina uyarak iceriyor mu.
///
/// Ham alt-dizgi aramasi kisa tetikleyicileri sik sozcuklerin ICINDE buluyordu
/// ("alfabetik" icindeki "betik" kod becerisini cagiriyordu); yanlis kilavuz
/// enjekte edilince model o becerinin aracini zorluyor ve semaya uymayan cagri
/// uretiyordu. Bu yuzden sozcuk BASI sarti var.
pub fn icerir(metin: &str, tetik: &str) -> bool {
    let Some(ilk) = tetik.chars().next() else {
        return false;
    };
    // Bosluk kullanmayan yazilarda (CJK, Korece) sozcuk siniri diye bir sey yok;
    // oradaki tetikleyiciler ham alt-dizgiye duser, aksi halde HIC eslesmezlerdi.
    if (ilk as u32) >= 0x0590 {
        return metin.contains(tetik);
    }

    // char dilimi: girdi Turkce, cok baytli. Bayt indisiyle gezmek karakterin
    // ortasina dusme riski tasir.
    let m: Vec<char> = metin.chars().collect();
    let t: Vec<char> = tetik.chars().collect();
    if t.is_empty() || t.len() > m.len() {
        return false;
    }

    // Kisalik kurali yalniz TEK sozcukluk koklere uygulanir; bosluk iceren
    // tetikleyici ("kac satir") zaten obektir ve kendi ozguluugunu tasir.
    let tam_sozcuk_gerek = t.len() < TAM_SOZCUK_SINIRI && !t.contains(&' ');

    for i in 0..=(m.len() - t.len()) {
        if m[i..i + t.len()] != t[..] {
            continue;
        }
        let basta = i == 0 || !m[i - 1].is_alphanumeric();
        if !basta {
            continue;
        }
        if !tam_sozcuk_gerek {
            return true;
        }
        let son = i + t.len();
        if son == m.len() || !m[son].is_alphanumeric() {
            return true;
        }
    }
    false
}

/// Eslesen tetikleyicilerin UZUNLUK toplami — adet DEGIL.
///
/// Boylece ozgul ifade genel kelimeyi yener: "bunu tablo olarak goster"
/// cumlesinde belge-oku'nun "tablo olarak"i (13), belge-olustur'un
/// "tablo"sunu (5) gecer. Adet sayilsaydi ikisi de 1 alir ve BERABERLIKTE
/// sirayi rastgelelik belirlerdi — ayni cumleye iki farkli kilavuz.
pub fn puan(metin_kucuk: &str, tetikler: &[String]) -> usize {
    tetikler
        .iter()
        .filter(|t| icerir(metin_kucuk, t))
        .map(|t| t.chars().count())
        .sum()
}

#[cfg(test)]
mod testler {
    use super::*;

    #[test]
    fn turkce_kucultme_i_harflerini_dogru_esler() {
        assert_eq!(kucult("ISTANBUL"), "ıstanbul");
        assert_eq!(kucult("İstanbul"), "istanbul");
        assert_eq!(kucult("TABLO Olarak"), "tablo olarak");
    }

    #[test]
    fn kisa_kok_tam_sozcuk_ister() {
        // Olculen regresyon: "ara" tetikleyicisi "aralik"i yakaliyordu.
        assert!(!icerir("aralik ayinda ne var", "ara"));
        assert!(icerir("notlarimda ara", "ara"));
        assert!(!icerir("bulut nerede", "bul"));
        assert!(icerir("sunu bul", "bul"));
    }

    #[test]
    fn uzun_kok_on_ek_eslesmesini_surdurur() {
        // Turkce SONA ek alir; uzun kokte on-ek eslesmesi sart.
        assert!(icerir("satirini degistir", "satir"));
        assert!(icerir("carparsak ne olur", "carp"));
        // Ama sozcugun ORTASINDA duran kok tutmamali.
        assert!(!icerir("alfabetik siralama", "betik"));
    }

    #[test]
    fn obek_tetikleyici_bosluk_sartini_korur() {
        assert!(icerir("bunu tablo olarak goster", "tablo olarak"));
        assert!(!icerir("bunu tablo goster", "tablo olarak"));
    }

    #[test]
    fn puan_uzunluk_toplamidir_adet_degil() {
        let ozgul = vec!["tablo olarak".to_string()];
        let genel = vec!["tablo".to_string(), "goster".to_string()];
        let m = kucult("Bunu tablo olarak goster");
        // Adet sayilsaydi genel (2) ozgulu (1) yenerdi; uzunlukta ozgul kazanir
        // ancak burada genel de "goster" ile puan alir — asil sinav depo testinde.
        assert_eq!(puan(&m, &ozgul), 12);
        assert_eq!(puan(&m, &genel), 5 + 6);
    }

    #[test]
    fn eslesmeyen_tetik_sifir_puan_verir() {
        assert_eq!(puan("merhaba nasilsin", &["excel".to_string()]), 0);
        assert!(!icerir("merhaba", ""));
    }
}
