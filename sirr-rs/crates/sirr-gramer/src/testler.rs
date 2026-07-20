//! Bu crate'in degeri testleriyle olculur: gramer yanlissa model sessizce
//! bozuk JSON uretir ve hata arac katmaninda, cok gec ortaya cikar.
//!
//! Testlerin ortak omurgasi: kabul edilen her metin serde_json ile de
//! cozumlenebilmeli (yani gramer JSON'un DISINA cikmiyor), reddedilen her
//! metin ya JSON degil ya da semaya uymuyor (yani gramer semadan GEVSEK degil).

use super::*;
use sirr_cekirdek::{Alan, ArgSema};
use std::sync::Arc;

fn ornek_sema() -> ArgSema {
    ArgSema::nesne(vec![
        Alan::yeni("sorgu", ArgSema::metin()).zorunlu(),
        Alan::yeni("kapsam", ArgSema::secenek(["tumu", "yakin"])),
        Alan::yeni("adet", ArgSema::tamsayi().aralik(Some(1.0), Some(50.0))),
        Alan::yeni("derin", ArgSema::bool()),
    ])
}

fn gramer(sema: &ArgSema) -> Arc<Gramer> {
    Gramer::derle(sema)
}

/// Metni bastan sona besler ve kapanip kapanmadigini soyler.
fn kabul(sema: &ArgSema, metin: &str) -> bool {
    let g = gramer(sema);
    let mut d = g.durum();
    d.ilerlet(metin).is_ok() && d.bitti_mi()
}

/// Kabul edilen metin gercekten gecerli JSON mu — gramerin JSON'dan
/// sapmadiginin bagimsiz taniği.
fn kabul_ve_json(sema: &ArgSema, metin: &str) -> bool {
    kabul(sema, metin) && serde_json::from_str::<serde_json::Value>(metin).is_ok()
}

// ---------------------------------------------------------------- temel kabul

#[test]
fn gecerli_nesne_kabul_edilir() {
    let s = ornek_sema();
    assert!(kabul_ve_json(&s, r#"{"sorgu":"rapor"}"#));
    assert!(kabul_ve_json(&s, r#"{"sorgu":"rapor","kapsam":"yakin"}"#));
    assert!(kabul_ve_json(
        &s,
        r#"{"sorgu":"a","kapsam":"tumu","adet":12,"derin":true}"#
    ));
}

#[test]
fn bosluklar_yapisal_konumlarda_serbest() {
    let s = ornek_sema();
    assert!(kabul_ve_json(&s, "{ \"sorgu\" : \"a\" , \"adet\" : 3 }"));
    assert!(kabul_ve_json(&s, "{\n  \"sorgu\": \"a\"\n}"));
}

#[test]
fn alan_sirasi_serbest_ama_uydurma_alan_yok() {
    let s = ornek_sema();
    // Sira sema sirasina bagli degil; sozlesme kume, liste degil.
    assert!(kabul_ve_json(&s, r#"{"adet":5,"sorgu":"a"}"#));
    // Semada olmayan anahtarin ILK harfi bile uretilemez.
    assert!(!kabul(&s, r#"{"sorgu":"a","baska":1}"#));
    assert!(!kabul(&s, r#"{"x":1}"#));
}

#[test]
fn zorunlu_alan_atlanamaz() {
    let s = ornek_sema();
    assert!(!kabul(&s, "{}"));
    assert!(!kabul(&s, r#"{"kapsam":"tumu"}"#));
    // Zorunlu alan yazilmadan '}' hic acilmaz: kapatma karakteri izinli degil.
    let g = gramer(&s);
    let mut d = g.durum();
    d.ilerlet(r#"{"kapsam":"tumu""#).unwrap();
    assert!(!d.izinli_onekler().icerir('}'));
    assert!(d.izinli_onekler().icerir(','));
}

#[test]
fn ayni_anahtar_iki_kez_uretilemez() {
    let s = ornek_sema();
    assert!(!kabul(&s, r#"{"sorgu":"a","sorgu":"b"}"#));
}

#[test]
fn son_alandan_sonra_virgul_acilmaz() {
    // Tek alanli sema: alan yazildiktan sonra ',' olu yola sokar.
    let s = ArgSema::nesne(vec![Alan::yeni("a", ArgSema::bool()).zorunlu()]);
    let g = gramer(&s);
    let mut d = g.durum();
    d.ilerlet(r#"{"a":true"#).unwrap();
    let izin = d.izinli_onekler();
    assert!(izin.icerir('}'));
    assert!(!izin.icerir(','));
    assert!(!kabul(&s, r#"{"a":true,}"#));
}

// ---------------------------------------------------------------- enum

#[test]
fn enum_disi_deger_uretilemez() {
    let s = ornek_sema();
    assert!(!kabul(&s, r#"{"sorgu":"a","kapsam":"uzak"}"#));
    assert!(!kabul(&s, r#"{"sorgu":"a","kapsam":"tumusey"}"#));
    // Yarim birakilan enum da kapanmaz.
    assert!(!kabul(&s, r#"{"sorgu":"a","kapsam":"tum"}"#));
}

#[test]
fn enum_yazarken_yalniz_sonraki_harfler_izinli() {
    let s = ornek_sema();
    let g = gramer(&s);
    let mut d = g.durum();
    d.ilerlet(r#"{"sorgu":"a","kapsam":""#).unwrap();
    let izin = d.izinli_onekler();
    assert!(izin.icerir('t') && izin.icerir('y'));
    assert!(!izin.icerir('u'));
    assert!(!izin.metin_govdesi_mi(), "enum serbest metin degildir");
    d.ilerlet("tum").unwrap();
    let izin = d.izinli_onekler();
    assert_eq!(izin.karakterler().collect::<Vec<_>>(), vec!['u']);
}

// ---------------------------------------------------------------- metin

#[test]
fn kacisli_string_dogru_ele_alinir() {
    let s = ArgSema::nesne(vec![Alan::yeni("m", ArgSema::metin()).zorunlu()]);
    assert!(kabul_ve_json(&s, r#"{"m":"tirnak: \" ve ters bolu: \\"}"#));
    assert!(kabul_ve_json(&s, r#"{"m":"satir\nsonu\tsekme"}"#));
    // Gecersiz kacis harfi.
    assert!(!kabul(&s, r#"{"m":"\q"}"#));
    // Kacissiz kontrol karakteri JSON'da yasak.
    assert!(!kabul(&s, "{\"m\":\"a\nb\"}"));
}

#[test]
fn unicode_kacisi_dort_onaltilik_hane_ister() {
    let s = ArgSema::nesne(vec![Alan::yeni("m", ArgSema::metin()).zorunlu()]);
    assert!(kabul_ve_json(&s, r#"{"m":"ç �"}"#));
    assert!(!kabul(&s, r#"{"m":"\u00e"}"#));
    assert!(!kabul(&s, r#"{"m":"\u00zz"}"#));

    let g = gramer(&s);
    let mut d = g.durum();
    d.ilerlet(r#"{"m":"\u00"#).unwrap();
    let izin = d.izinli_onekler();
    assert!(izin.icerir('a') && izin.icerir('F') && izin.icerir('9'));
    assert!(!izin.icerir('g') && !izin.icerir('"'));
}

#[test]
fn cok_baytli_unicode_govdede_serbest() {
    let s = ArgSema::nesne(vec![Alan::yeni("m", ArgSema::metin()).zorunlu()]);
    assert!(kabul_ve_json(&s, r#"{"m":"cok gizli — sirr ✓ 世界"}"#));
}

#[test]
fn metin_uzunluk_siniri_kapanmaya_zorlar() {
    let s = ArgSema::nesne(vec![
        Alan::yeni("m", ArgSema::metin()).zorunlu(),
    ]);
    let sinirli = ArgSema::nesne(vec![
        Alan::yeni(
            "m",
            ArgSema {
                tip: sirr_cekirdek::SemaTipi::Metin { en_cok_uzunluk: Some(3) },
                aciklama: None,
            },
        )
        .zorunlu(),
    ]);
    assert!(kabul_ve_json(&s, r#"{"m":"abcdef"}"#));
    assert!(kabul_ve_json(&sinirli, r#"{"m":"abc"}"#));
    assert!(!kabul(&sinirli, r#"{"m":"abcd"}"#));
    // Sinira gelindiginde tek cikis kapanis tirnagidir.
    let g = gramer(&sinirli);
    let mut d = g.durum();
    d.ilerlet(r#"{"m":"abc"#).unwrap();
    let izin = d.izinli_onekler();
    assert!(!izin.metin_govdesi_mi());
    assert_eq!(izin.karakterler().collect::<Vec<_>>(), vec!['"']);
}

// ---------------------------------------------------------------- sayi

#[test]
fn sayi_grameri_json_kurallarina_uyar() {
    let s = ArgSema::nesne(vec![Alan::yeni("n", ArgSema::sayi()).zorunlu()]);
    for iyi in [
        "0", "-0", "12", "-12", "1.5", "-1.5", "0.5", "1e3", "1E+3", "2.5e-4",
    ] {
        assert!(kabul_ve_json(&s, &format!(r#"{{"n":{iyi}}}"#)), "kabul: {iyi}");
    }
    for kotu in ["01", "1.", ".5", "+1", "1e", "1e+", "--1", "1.2.3", "0x1"] {
        assert!(!kabul(&s, &format!(r#"{{"n":{kotu}}}"#)), "red: {kotu}");
    }
}

#[test]
fn tamsayi_ondalik_ve_us_uretemez() {
    let s = ArgSema::nesne(vec![Alan::yeni("n", ArgSema::tamsayi()).zorunlu()]);
    assert!(kabul_ve_json(&s, r#"{"n":42}"#));
    assert!(!kabul(&s, r#"{"n":4.2}"#));
    assert!(!kabul(&s, r#"{"n":4e2}"#));
    let g = gramer(&s);
    let mut d = g.durum();
    d.ilerlet(r#"{"n":4"#).unwrap();
    let izin = d.izinli_onekler();
    assert!(!izin.icerir('.') && !izin.icerir('e'));
    // Sayi tuketmeden kapanabildigi icin ust cercevenin '}' izni gorunur.
    assert!(izin.icerir('}'));
}

#[test]
fn negatif_olmayan_aralikta_eksi_isareti_hic_acilmaz() {
    let s = ArgSema::nesne(vec![
        Alan::yeni("n", ArgSema::tamsayi().aralik(Some(1.0), Some(50.0))).zorunlu(),
    ]);
    let g = gramer(&s);
    let mut d = g.durum();
    d.ilerlet(r#"{"n":"#).unwrap();
    assert!(!d.izinli_onekler().icerir('-'));
    assert!(!kabul(&s, r#"{"n":-5}"#));
    assert!(!kabul(&s, r#"{"n":99}"#));
    assert!(kabul_ve_json(&s, r#"{"n":50}"#));
}

// ---------------------------------------------------------------- bool

#[test]
fn bool_yalniz_true_false_uretir() {
    let s = ArgSema::nesne(vec![Alan::yeni("b", ArgSema::bool()).zorunlu()]);
    assert!(kabul_ve_json(&s, r#"{"b":true}"#));
    assert!(kabul_ve_json(&s, r#"{"b":false}"#));
    assert!(!kabul(&s, r#"{"b":True}"#));
    assert!(!kabul(&s, r#"{"b":1}"#));
    assert!(!kabul(&s, r#"{"b":"true"}"#));
    let g = gramer(&s);
    let mut d = g.durum();
    d.ilerlet(r#"{"b":"#).unwrap();
    assert_eq!(d.izinli_onekler().karakterler().collect::<Vec<_>>(), vec!['f', 't']);
}

// ---------------------------------------------------------------- ic ice yapi

#[test]
fn ic_ice_nesne_ve_dizi() {
    let s = ArgSema::nesne(vec![
        Alan::yeni(
            "hedef",
            ArgSema::nesne(vec![
                Alan::yeni("yol", ArgSema::metin()).zorunlu(),
                Alan::yeni("satir", ArgSema::tamsayi()),
            ]),
        )
        .zorunlu(),
        Alan::yeni("etiketler", ArgSema::dizi(ArgSema::metin())),
    ]);
    assert!(kabul_ve_json(&s, r#"{"hedef":{"yol":"a.txt"}}"#));
    assert!(kabul_ve_json(
        &s,
        r#"{"hedef":{"yol":"a.txt","satir":3},"etiketler":["x","y"]}"#
    ));
    assert!(kabul_ve_json(&s, r#"{"hedef":{"yol":"a"},"etiketler":[]}"#));
    // Ic nesnenin zorunlu alani da atlanamaz.
    assert!(!kabul(&s, r#"{"hedef":{}}"#));
    // Dizi elemanlarinin tipi sabittir.
    assert!(!kabul(&s, r#"{"hedef":{"yol":"a"},"etiketler":[1]}"#));
}

#[test]
fn dizi_uzunluk_sinirlari_zorlanir() {
    let s = ArgSema::nesne(vec![
        Alan::yeni("d", ArgSema::dizi(ArgSema::tamsayi()).uzunluk(Some(2), Some(3)))
            .zorunlu(),
    ]);
    assert!(kabul_ve_json(&s, r#"{"d":[1,2]}"#));
    assert!(kabul_ve_json(&s, r#"{"d":[1,2,3]}"#));
    assert!(!kabul(&s, r#"{"d":[]}"#));
    assert!(!kabul(&s, r#"{"d":[1]}"#));
    assert!(!kabul(&s, r#"{"d":[1,2,3,4]}"#));

    let g = gramer(&s);
    let mut d = g.durum();
    d.ilerlet(r#"{"d":[1,2,3"#).unwrap();
    let izin = d.izinli_onekler();
    assert!(izin.icerir(']'));
    assert!(!izin.icerir(','), "ust sinirda virgul acilmamali");
}

#[test]
fn ic_ice_dizi_ve_nesne_karisik() {
    let s = ArgSema::nesne(vec![
        Alan::yeni(
            "kayitlar",
            ArgSema::dizi(ArgSema::nesne(vec![
                Alan::yeni("ad", ArgSema::metin()).zorunlu(),
                Alan::yeni("tur", ArgSema::secenek(["dosya", "dizin"])).zorunlu(),
            ])),
        )
        .zorunlu(),
    ]);
    assert!(kabul_ve_json(
        &s,
        r#"{"kayitlar":[{"ad":"a","tur":"dosya"},{"ad":"b","tur":"dizin"}]}"#
    ));
    assert!(!kabul(&s, r#"{"kayitlar":[{"ad":"a"}]}"#));
    assert!(!kabul(&s, r#"{"kayitlar":[{"ad":"a","tur":"link"}]}"#));
}

#[test]
fn bos_sema_yalniz_bos_nesne_kabul_eder() {
    let s = ArgSema::bos();
    assert!(kabul_ve_json(&s, "{}"));
    assert!(kabul_ve_json(&s, "{ }"));
    assert!(!kabul(&s, r#"{"a":1}"#));
}

// ---------------------------------------------------------------- durum akisi

#[test]
fn ilerlet_hatada_durumu_bozmaz() {
    let s = ornek_sema();
    let g = gramer(&s);
    let mut d = g.durum();
    d.ilerlet(r#"{"sorgu":"#).unwrap();
    let hata = d.ilerlet("42").unwrap_err();
    assert!(matches!(hata, GramerHatasi::BeklenmeyenKarakter { .. }));
    // Atomiklik: reddedilen parca durumu kirletmedi, dogru deger hala yazilabilir.
    d.ilerlet(r#""rapor"}"#).unwrap();
    assert!(d.bitti_mi());
}

#[test]
fn kapandiktan_sonra_fazladan_girdi_reddedilir() {
    let s = ornek_sema();
    let g = gramer(&s);
    let mut d = g.durum();
    d.ilerlet(r#"{"sorgu":"a"}"#).unwrap();
    assert!(d.bitti_mi());
    assert!(d.bitir().is_ok());
    assert!(d.izinli_onekler().bitebilir());
    // Model "yapayim mi?" diye anlatmaya baslarsa ilk harfte durur.
    assert!(matches!(
        d.ilerlet("Simdi").unwrap_err(),
        GramerHatasi::FazladanGirdi { .. }
    ));
    // Sondaki bosluk zararsiz.
    d.ilerlet("  \n").unwrap();
}

#[test]
fn yarim_kalan_yapi_bitmis_sayilmaz() {
    let s = ornek_sema();
    let g = gramer(&s);
    let mut d = g.durum();
    d.ilerlet(r#"{"sorgu":"a"#).unwrap();
    assert!(!d.bitti_mi());
    assert_eq!(d.bitir().unwrap_err(), GramerHatasi::Tamamlanmadi);
}

#[test]
fn izin_kumesi_hicbir_zaman_olu_kalmaz() {
    // Gecerli bir uretim boyunca her adimda ya bir karakter uretilebilmeli
    // ya da uretim bitebilmeli; aksi halde model kilitlenir.
    let s = ornek_sema();
    let g = gramer(&s);
    let mut d = g.durum();
    for c in r#"{"sorgu":"aç","adet":7,"derin":false}"#.chars() {
        let izin = d.izinli_onekler();
        assert!(!izin.bos_mu() || izin.bitebilir(), "olu dugum: {izin:?}");
        assert!(izin.icerir(c), "'{c}' izinli olmaliydi: {izin:?}");
        d = d.dallan(c).unwrap();
    }
    assert!(d.bitti_mi());
}

#[test]
fn sayi_araliginda_olu_onek_hic_acilmaz() {
    // [10,20]: '2' yazildiktan sonra yalniz '0' devam edebilir (20), cunku
    // 21..29 aralik disi ve tek basina 2 de aralik disi.
    let s = ArgSema::nesne(vec![
        Alan::yeni("n", ArgSema::tamsayi().aralik(Some(10.0), Some(20.0))).zorunlu(),
    ]);
    let g = gramer(&s);
    let mut d = g.durum();
    d.ilerlet(r#"{"n":"#).unwrap();
    // Tek haneli hicbir sayi araliga giremez ama 1 ve 2 onek olarak yasar.
    assert_eq!(d.izinli_onekler().karakterler().collect::<Vec<_>>(), vec!['1', '2']);

    let mut iki = d.dallan('2').unwrap();
    let izin = iki.izinli_onekler();
    assert_eq!(izin.karakterler().collect::<Vec<_>>(), vec!['0']);
    assert!(!izin.bitebilir(), "2 tek basina aralik disi, kapanamaz");

    let bir = d.dallan('1').unwrap();
    // 1 de tek basina aralik disi: '}' acilmamali, ama 10..19 icin haneler acik.
    assert!(!bir.izinli_onekler().icerir('}'));
    assert!(bir.izinli_onekler().icerir('9'));

    iki.ilerlet("0}").unwrap();
    assert!(iki.bitti_mi());
    assert!(!kabul(&s, r#"{"n":21}"#));
    assert!(!kabul(&s, r#"{"n":5}"#));
    assert!(kabul_ve_json(&s, r#"{"n":20}"#));
}

#[test]
fn hicbir_ulasilabilir_durum_kilitlenmez() {
    // EN ONEMLI OZELLIK: kisitli uretimde olu bir durum modeli kilitler —
    // hicbir token uretilemez, uretim de bitemez. Ulasilabilir durum uzayini
    // (sinirli derinlikte) gezip her dugumde "ya bir cikis var ya bitebilir"
    // onermesini dogrular.
    let s = ornek_sema();
    let g = gramer(&s);
    let mut butce = 40_000usize;
    gez(&g.durum(), 0, &mut butce);

    fn gez(d: &GramerDurumu, derinlik: usize, butce: &mut usize) {
        let izin = d.izinli_onekler();
        assert!(
            !izin.bos_mu() || izin.bitebilir(),
            "kilitli durum (derinlik {derinlik}): {izin:?}"
        );
        if derinlik >= 12 || *butce == 0 {
            return;
        }
        // Serbest metin govdesi sonsuz dallanir; temsili birkac karakter yeter.
        let mut adaylar: Vec<char> = izin.karakterler().collect();
        if izin.metin_govdesi_mi() {
            adaylar.extend(['a', 'ç', '✓']);
        }
        for c in adaylar {
            if *butce == 0 {
                return;
            }
            *butce -= 1;
            if let Ok(yeni) = d.dallan(c) {
                gez(&yeni, derinlik + 1, butce);
            }
        }
    }
}

// ---------------------------------------------------------------- maske

fn dagarcik() -> Vec<String> {
    [
        "", "{", "}", "\"", ":", ",", "[", "]", "sorgu", "kapsam", "adet", "derin", "tumu",
        "yakin", "true", "false", "rapor", "0", "1", "12", "99", "Simdi", "uzak", "\"sorgu\"",
        "\":", "{\"", "baska", "-",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn izinliler(maske: &[bool], d: &[String]) -> Vec<String> {
    maske
        .iter()
        .enumerate()
        .filter(|(_, v)| **v)
        .map(|(i, _)| d[i].clone())
        .collect()
}

#[test]
fn maske_yalniz_gecerli_tokenlari_birakir() {
    let s = ornek_sema();
    let g = gramer(&s);
    let d = g.durum();
    let dag = dagarcik();
    let m = TokenMaskesi::yeni(&dag);
    let acik = izinliler(&m.maske(&d), &dag);
    // Bastayken yalniz '{' ile baslayan tokenlar acik olabilir.
    assert!(acik.contains(&"{".to_string()));
    assert!(acik.contains(&"{\"".to_string()));
    assert!(!acik.contains(&"Simdi".to_string()));
    assert!(!acik.contains(&"sorgu".to_string()));
    assert!(!acik.contains(&"}".to_string()));
    // Bos metinli ozel tokenlar hicbir zaman maskede acilmaz.
    assert!(!acik.iter().any(|t| t.is_empty()));
    assert_eq!(m.bos_tokenlar(), &[0]);
    assert_eq!(m.dagarcik_boyu(), dag.len());
}

#[test]
fn maske_anahtar_konumunda_yalniz_sema_alanlarini_acar() {
    let s = ornek_sema();
    let g = gramer(&s);
    let mut d = g.durum();
    d.ilerlet("{\"").unwrap();
    let dag = dagarcik();
    let acik = izinliler(&d.maske(&dag), &dag);
    for beklenen in ["sorgu", "kapsam", "adet", "derin"] {
        assert!(acik.contains(&beklenen.to_string()), "{beklenen} acik olmali");
    }
    assert!(!acik.contains(&"baska".to_string()));
    assert!(!acik.contains(&"tumu".to_string()));
}

#[test]
fn maske_enum_konumunda_kume_disini_kapatir() {
    let s = ornek_sema();
    let g = gramer(&s);
    let mut d = g.durum();
    d.ilerlet(r#"{"sorgu":"a","kapsam":""#).unwrap();
    let dag = dagarcik();
    let acik = izinliler(&d.maske(&dag), &dag);
    assert!(acik.contains(&"tumu".to_string()));
    assert!(acik.contains(&"yakin".to_string()));
    assert!(!acik.contains(&"uzak".to_string()));
    assert!(!acik.contains(&"rapor".to_string()));
}

#[test]
fn maske_araligi_ve_tipi_gozetir() {
    let s = ornek_sema();
    let g = gramer(&s);
    let mut d = g.durum();
    d.ilerlet(r#"{"sorgu":"a","adet":"#).unwrap();
    let dag = dagarcik();
    let acik = izinliler(&d.maske(&dag), &dag);
    assert!(acik.contains(&"12".to_string()));
    assert!(acik.contains(&"1".to_string()));
    // 99 aralik disi (1..50), 0 alt sinirin altinda, '-' negatif olamaz.
    assert!(!acik.contains(&"99".to_string()));
    assert!(!acik.contains(&"0".to_string()));
    assert!(!acik.contains(&"-".to_string()));
    assert!(!acik.contains(&"true".to_string()));
}

#[test]
fn maske_cok_karakterli_tokeni_bastan_sona_dogrular() {
    let s = ornek_sema();
    let g = gramer(&s);
    let mut d = g.durum();
    d.ilerlet("{").unwrap();
    let dag = dagarcik();
    let acik = izinliler(&d.maske(&dag), &dag);
    // Coklu karakter iceren token ancak TAMAMI gecerliyse acilir.
    assert!(acik.contains(&"\"sorgu\"".to_string()));
    assert!(acik.contains(&"\"".to_string()));
    assert!(!acik.contains(&"\":".to_string()));
}

#[test]
fn maske_bitis_konumunda_hicbir_seyi_acmaz() {
    let s = ornek_sema();
    let g = gramer(&s);
    let mut d = g.durum();
    d.ilerlet(r#"{"sorgu":"a"}"#).unwrap();
    let dag = dagarcik();
    // Bittiginde hicbir metin tokeni gecerli degil; durdurma karari (EOS)
    // cagiranin isi, gramer yalniz "bitebilir" der.
    assert!(izinliler(&d.maske(&dag), &dag).is_empty());
    assert!(d.izinli_onekler().bitebilir());
}

#[test]
fn maske_onbellegi_ile_onbelleksiz_ayni_sonucu_verir() {
    let s = ornek_sema();
    let g = gramer(&s);
    let mut d = g.durum();
    d.ilerlet(r#"{"sorgu":"#).unwrap();
    let dag = dagarcik();
    let m = TokenMaskesi::yeni(&dag);
    assert_eq!(m.maske(&d), d.maske(&dag));
}

#[test]
fn maskeyle_yurutulen_uretim_daima_gecerli_json_verir() {
    // Butunsel test: her adimda maskeden ILK acik tokeni sec. Gramer dogruysa
    // hangi acgozlu secim yapilirsa yapilsin cikti gecerli JSON olmali.
    //
    // Sema bilerek serbest METIN icermiyor: serbest metin alani gramerce
    // sonsuza kadar uzatilabilir (her karakter gecerlidir), yani "ne secersen
    // sec kapanir" onermesi orada gecmez — kapanma karari modele aittir.
    let s = ArgSema::nesne(vec![
        Alan::yeni("kapsam", ArgSema::secenek(["tumu", "yakin"])).zorunlu(),
        Alan::yeni("adet", ArgSema::tamsayi().aralik(Some(1.0), Some(50.0))).zorunlu(),
        Alan::yeni("derin", ArgSema::bool()).zorunlu(),
    ]);
    let g = gramer(&s);
    let mut d = g.durum();
    let dag = dagarcik();
    let m = TokenMaskesi::yeni(&dag);
    let mut cikti = String::new();
    for _ in 0..64 {
        let maske = m.maske(&d);
        let Some(i) = maske.iter().position(|v| *v) else { break };
        cikti.push_str(&dag[i]);
        d.ilerlet(&dag[i]).unwrap();
    }
    assert!(d.bitti_mi(), "uretim kapanmadi: {cikti}");
    let deger: serde_json::Value = serde_json::from_str(&cikti).unwrap();
    // Uretilen sey semadan da gecmeli: gramer ile dogrulama ayni sozlesmede.
    assert!(s.dogrula(&deger).is_ok(), "sema reddetti: {cikti}");
}
