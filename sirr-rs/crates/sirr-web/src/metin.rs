//! HTML → duz metin. BILINCLI OLARAK BASIT.
//!
//! Bu bir HTML ayristiricisi DEGIL ve olmaya calismamali. Amac tek: bir sayfayi
//! modelin okuyabilecegi kaba metne indirmek. Gercek bir DOM ayristiricisi
//! (html5ever + tendril + ...) hem buyuk bir bagimlilik agacidir hem de bize
//! ihtiyacimiz olmayan bir dogruluk verir — modelin gordugu metin zaten
//! kirpilacak ve ozetlenecek.
//!
//! NE YAPILIR: script/style/noscript govdeleri ATILIR (yoksa modele minified
//! JavaScript dokulur — hem anlamsiz hem token katili), etiketler soyulur,
//! yaygin varliklar cozulur, bosluk normallestirilir.
//!
//! NE YAPILMAZ: oznitelik yorumu, tablo yapisi, betik calistirma. Iyi
//! yapilandirilmamis sayfada cikti kotu olur; kabul edilen bedel bu.

/// `<script>`/`<style>`/`<noscript>` gibi govdesi metin OLMAYAN elemanlar.
/// Bunlarin ici soyulmakla kalmaz, tumden atilir.
const ATILAN_BLOKLAR: [&str; 4] = ["script", "style", "noscript", "svg"];

/// Metin akisinda satir sonu anlamina gelen etiketler — soyulunca kelimeler
/// birbirine yapismasin diye yerlerine bosluk konur.
const BLOK_ETIKETLERI: [&str; 12] =
    ["p", "br", "div", "li", "tr", "h1", "h2", "h3", "h4", "h5", "h6", "section"];

/// HTML govdesini duz metne cevirir.
pub fn metinlestir(html: &str) -> String {
    let govdesiz = bloklari_at(html);
    let soyulmus = etiketleri_soy(&govdesiz);
    bosluk_normallestir(&varliklari_coz(&soyulmus))
}

/// `<script>...</script>` gibi bloklari icerigiyle beraber siler.
///
/// Kapanis etiketi hic gelmezse (bozuk HTML) blogun sonuna kadar atilir:
/// yarim kalan bir `<script>`in icerigini metne katmak, tam da kacinmak
/// istedigimiz sey.
fn bloklari_at(html: &str) -> String {
    let mut cikti = String::with_capacity(html.len());
    let mut kalan = html;
    'dis: while let Some(i) = kalan.find('<') {
        cikti.push_str(&kalan[..i]);
        let govde = &kalan[i..];
        for ad in ATILAN_BLOKLAR {
            if acilis_mi(govde, ad) {
                kalan = blok_sonrasi(govde, ad);
                continue 'dis;
            }
        }
        cikti.push('<');
        kalan = &kalan[i + 1..];
    }
    cikti.push_str(kalan);
    cikti
}

/// `<ad` ACILIS etiketi mi — kapanis (`</ad`) sayilmaz.
///
/// Ayrim hayati: kapanisi da "blok basliyor" sayan bir surum, `</script>`
/// gordugunde blogu yeniden atlamaya calisir, imlec ilerlemez ve fonksiyon
/// SONSUZ DONGUYE girer. (Ilk surumde tam olarak bu oldu.)
fn acilis_mi(govde: &str, ad: &str) -> bool {
    !govde.starts_with("</") && etiket_basliyor(govde, ad)
}

/// Acilis etiketinden blogun SONRASINA gecer.
///
/// Kapanis etiketinin `>` isareti de yutulur: govdeyi kapanisin BASINDA
/// birakmak, cagiran donguye ayni kapanisi tekrar buldurur ve ilerleme durur.
/// Kapanis hic yoksa (bozuk HTML) sayfanin sonuna kadar atilir — yarim bir
/// `<script>`in icerigini metne katmak tam da kacindigimiz sey.
fn blok_sonrasi<'a>(govde: &'a str, ad: &str) -> &'a str {
    // `to_ascii_lowercase` yalniz ASCII baytlara dokunur, uzunluk degismez;
    // bu yuzden buradan cikan indis `govde` uzerinde gecerlidir.
    let kucuk = govde.to_ascii_lowercase();
    let Some(j) = kucuk.find(&format!("</{ad}")) else {
        return "";
    };
    match govde[j..].find('>') {
        Some(k) => &govde[j + k + 1..],
        None => "",
    }
}

/// `govde` `<ad` ya da `</ad` ile mi basliyor (buyuk/kucuk harf duyarsiz).
///
/// Ad sinirini kontrol eder: `<scriptish>` bir `script` etiketi DEGILDIR ve
/// naif bir `starts_with` onu da yutardi.
fn etiket_basliyor(govde: &str, ad: &str) -> bool {
    let g = govde.strip_prefix('<').unwrap_or(govde);
    let g = g.strip_prefix('/').unwrap_or(g);
    let Some(sonrasi) = g.get(..ad.len()) else { return false };
    if !sonrasi.eq_ignore_ascii_case(ad) {
        return false;
    }
    g[ad.len()..].chars().next().is_none_or(|c| !c.is_ascii_alphanumeric() && c != '-')
}

/// Etiketleri siler; blok etiketlerinin yerine bosluk birakir.
fn etiketleri_soy(html: &str) -> String {
    let mut cikti = String::with_capacity(html.len());
    let mut kalan = html;
    while let Some(i) = kalan.find('<') {
        cikti.push_str(&kalan[..i]);
        let govde = &kalan[i..];
        if BLOK_ETIKETLERI.iter().any(|ad| etiket_basliyor(govde, ad)) {
            cikti.push(' ');
        }
        // Kapanis `>` yoksa geri kalan tumuyle etiket sayilir ve atilir;
        // aksi halde `<` ile baslayan cop metne sizardi.
        kalan = match govde.find('>') {
            Some(j) => &govde[j + 1..],
            None => "",
        };
    }
    cikti.push_str(kalan);
    cikti
}

/// Yaygin HTML varliklarini cozer.
///
/// Tam varlik tablosu (2000+ giris) EKLENMEDI: gercek sayfalarda gorulen
/// avuc dolusu varlik bunlar, gerisi zaten sayisal kacislarla geliyor ve
/// asagida genel olarak cozuluyor.
fn varliklari_coz(metin: &str) -> String {
    let mut cikti = String::with_capacity(metin.len());
    let mut kalan = metin;
    while let Some(i) = kalan.find('&') {
        cikti.push_str(&kalan[..i]);
        let govde = &kalan[i..];
        // Noktali virgul uzakta ise bu bir varlik degil, duz bir `&` isareti.
        //
        // `char_indices` ile araniyor, `govde[..12]` ile DEGIL: cok baytli bir
        // karakterin ortasindan dilimlemek panik sebebidir ve girdi bize dis
        // dunyadan geliyor — orada panige acik tek bir satir birakilamaz.
        let son = govde.char_indices().take(12).find(|(_, c)| *c == ';').map(|(j, _)| j);
        match son.map(|j| (&govde[1..j], j)) {
            Some((ad, j)) => {
                cikti.push_str(&cozum(ad).unwrap_or_else(|| govde[..=j].to_string()));
                kalan = &govde[j + 1..];
            }
            None => {
                cikti.push('&');
                kalan = &govde[1..];
            }
        }
    }
    cikti.push_str(kalan);
    cikti
}

fn cozum(ad: &str) -> Option<String> {
    let s = match ad {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" | "#39" => "'",
        // Sert bosluk normal bosluga cevrilir; aksi halde asagidaki bosluk
        // normallestirmesi onu gormez ve metin sisik kalir.
        "nbsp" | "#160" => " ",
        "hellip" => "…",
        "mdash" => "—",
        "ndash" => "–",
        _ => return sayisal_cozum(ad),
    };
    Some(s.to_string())
}

/// `&#8217;` / `&#x2019;` bicimindeki sayisal kacislar.
fn sayisal_cozum(ad: &str) -> Option<String> {
    let govde = ad.strip_prefix('#')?;
    let kod = match govde.strip_prefix(['x', 'X']) {
        Some(onaltilik) => u32::from_str_radix(onaltilik, 16).ok()?,
        None => govde.parse::<u32>().ok()?,
    };
    char::from_u32(kod).map(String::from)
}

/// Ardisik bosluklari tek boslugu indirir, satir yapisini korumaz.
///
/// SATIR YAPISI BILINCLI OLARAK ATILIYOR: soyulmus HTML'de satir sonlari
/// kaynagin girintisinden gelir, anlamdan degil. Onlari korumak modele
/// sayfanin bicimlendirmesini degil, HTML yazarinin sekme aliskanligini
/// ogretirdi.
fn bosluk_normallestir(metin: &str) -> String {
    metin.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod testler {
    use super::*;

    #[test]
    fn etiketler_soyulur_ve_bosluk_normallesir() {
        let h = "<html>  <body><h1>Baslik</h1>\n\n<p>Bir   iki</p></body></html>";
        assert_eq!(metinlestir(h), "Baslik Bir iki");
    }

    #[test]
    fn script_ve_style_govdesi_tumden_atilir() {
        let h = "<p>once</p><script>var a = 1 < 2;</script><style>p{color:red}</style><p>sonra</p>";
        let m = metinlestir(h);
        assert_eq!(m, "once sonra");
        assert!(!m.contains("var a"), "JS metne sizmamali");
        assert!(!m.contains("color"), "CSS metne sizmamali");
    }

    #[test]
    fn kapanmayan_script_sonuna_kadar_atilir() {
        let m = metinlestir("<p>bas</p><script>gizli icerik ve devami");
        assert_eq!(m, "bas");
    }

    #[test]
    fn benzer_adli_etiket_yanlislikla_atilmaz() {
        // `<scriptish>` bir script blogu DEGIL — naif eslestirme burada patlar.
        let m = metinlestir("<scriptish>gorunur</scriptish>");
        assert_eq!(m, "gorunur");
    }

    #[test]
    fn varliklar_cozulur() {
        let m = metinlestir("<p>a&amp;b &lt;c&gt; &quot;d&quot; e&#39;f&nbsp;g &hellip;</p>");
        assert_eq!(m, "a&b <c> \"d\" e'f g …");
    }

    #[test]
    fn sayisal_varliklar_cozulur() {
        assert_eq!(metinlestir("<p>&#8217;&#x2019;</p>"), "’’");
    }

    #[test]
    fn varlik_olmayan_ampersand_korunur() {
        assert_eq!(metinlestir("<p>a & b, c &bilinmeyen; d</p>"), "a & b, c &bilinmeyen; d");
    }

    #[test]
    fn blok_etiketleri_kelimeleri_yapistirmaz() {
        // Bosluk konmasaydi "birikiuc" cikardi ve model tek kelime gorurdu.
        assert_eq!(metinlestir("<li>bir</li><li>iki</li><div>uc</div>"), "bir iki uc");
    }

    #[test]
    fn kapanmayan_etiket_metne_sizmaz() {
        assert_eq!(metinlestir("gorunur <p class=\"x"), "gorunur");
    }

    /// Bozuk girdinin SOZLESMESI: panik yok, `<...>` icerigi sizmaz.
    ///
    /// Basibos `>` isaretlerinin metinde kalmasi KABUL EDILEN davranistir.
    /// Onlari da temizlemek "her `>` gurultudur" gibi keyfi bir kural
    /// gerektirirdi; oysa duz metinde `>` mesru bir karakterdir (alinti,
    /// karsilastirma). Bu fonksiyon bir HTML ayristiricisi degil, kaba bir
    /// soyucudur — bozuk girdide kotu cikti vermesi bilincli bedeldir.
    #[test]
    fn bozuk_girdi_panik_yapmaz_ve_etiket_icerigi_sizmaz() {
        assert_eq!(metinlestir(""), "");
        assert_eq!(metinlestir("duz metin"), "duz metin");
        assert_eq!(metinlestir("&"), "&");
        assert_eq!(metinlestir("<<<>>>"), ">>");
        // Asil garanti: etiketin ICI disari cikmaz.
        assert_eq!(metinlestir("<a href=\"gizli\">gorunur</a>"), "gorunur");
        assert!(!metinlestir("<p onclick=\"kod()\">x</p>").contains("onclick"));
    }

    #[test]
    fn cok_baytli_icerik_bozulmaz() {
        assert_eq!(metinlestir("<p>çığır açan ölçüm</p>"), "çığır açan ölçüm");
    }

    /// GERILEME TESTI: ilk surumde `bloklari_at` kapanis etiketini yeniden
    /// "blok basliyor" sayiyor, imlec ilerlemiyor ve fonksiyon SONSUZ DONGUYE
    /// giriyordu (test kosumu %100 CPU'da asili kaldi, hata vermedi). Bu test
    /// o donguyu yakalar: dongu geri gelirse burada takilir.
    #[test]
    fn ardisik_atilan_bloklar_sonsuz_dongu_yapmaz() {
        let h = "<script>a</script><style>b</style><script>c</script><p>son</p>";
        assert_eq!(metinlestir(h), "son");
    }

    /// `&` sonrasi cok baytli karakter: naif `govde[..12]` dilimlemesi burada
    /// UTF-8 sinirini bolup PANIK yapardi. Girdi dis dunyadan geliyor.
    #[test]
    fn ampersand_sonrasi_cok_baytli_karakter_panik_yapmaz() {
        assert_eq!(metinlestir("<p>a &çığırğüö b</p>"), "a &çığırğüö b");
        assert_eq!(metinlestir("&ç"), "&ç");
    }
}
