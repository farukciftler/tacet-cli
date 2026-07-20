//! sirr-zip testleri.
//!
//! Vurgu iki yerde: (1) gidis-donus dogrulugu, (2) BOZUK GIRDI PANIK YAPMAZ.
//! Ikincisi Swift surumunun bilinen zayifligiydi (sinir kontrolsuz oku16/oku32),
//! bu yuzden burada kesme/bozma/fuzz testleri agirlikta.

use sirr_zip::{ARSIV_TAVANI, GIRIS_TAVANI, ZipGiris, ZipHatasi, ac, ac_harita, crc32, inflate, paketle};

/// Baska araclarla uretilmis gercek vektorler; kendi yazdigimizi kendi
/// okumamiz cozucunun DOGRU oldugunu kanitlamaz, yalniz tutarli oldugunu.
const DINAMIK_DEFLATE: &[u8] = include_bytes!("veri/dinamik.deflate");
const DINAMIK_HAM: &[u8] = include_bytes!("veri/dinamik.ham");
const GERCEK_ZIP: &[u8] = include_bytes!("veri/gercek.zip");
const BOMBA_DEFLATE: &[u8] = include_bytes!("veri/bomba.deflate");

// ---------------------------------------------------------------- CRC32

#[test]
fn crc32_bilinen_vektor() {
    // "123456789" icin IEEE CRC32 standart kontrol degeri.
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(crc32(b""), 0);
}

// ------------------------------------------------------- Yaz-sonra-oku

#[test]
fn gidis_donus_tek_dosya() {
    let girisler = vec![ZipGiris::yeni("xl/workbook.xml", b"<workbook/>".to_vec())];
    let zip = paketle(&girisler).expect("paketleme basarili");
    let acilan = ac(&zip).expect("acma basarili");
    assert_eq!(acilan, girisler);
}

#[test]
fn gidis_donus_cok_dosyali_arsiv() {
    let girisler: Vec<ZipGiris> = (0..25)
        .map(|i| ZipGiris::yeni(format!("parca/{i}.xml"), format!("<v>{i}</v>").into_bytes()))
        .collect();
    let zip = paketle(&girisler).unwrap();
    let harita = ac_harita(&zip).unwrap();
    assert_eq!(harita.len(), 25);
    assert_eq!(harita["parca/7.xml"], b"<v>7</v>".to_vec());
}

#[test]
fn gidis_donus_bos_dosya_ve_bos_arsiv() {
    // Bos govdeli giris: CRC 0 olur, okuyucunun CRC atlama yolu tetiklenir.
    let girisler = vec![
        ZipGiris::yeni("bos.txt", Vec::new()),
        ZipGiris::yeni("dolu.txt", b"x".to_vec()),
    ];
    let zip = paketle(&girisler).unwrap();
    assert_eq!(ac(&zip).unwrap(), girisler);

    // Hic giris yok: gecerli ama bos bir arsiv.
    let bos_zip = paketle(&[]).unwrap();
    assert_eq!(bos_zip.len(), 22, "yalniz EOCD kaydi olmali");
    assert!(ac(&bos_zip).unwrap().is_empty());
}

#[test]
fn gidis_donus_uzun_ad_ve_utf8() {
    let uzun_ad = format!("{}/dosya.xml", "a".repeat(60_000));
    let girisler = vec![
        ZipGiris::yeni(uzun_ad.clone(), b"veri".to_vec()),
        ZipGiris::yeni("belgeler/ozet-gunluk.xml", b"turkce ad".to_vec()),
    ];
    let zip = paketle(&girisler).unwrap();
    let harita = ac_harita(&zip).unwrap();
    assert_eq!(harita[&uzun_ad], b"veri".to_vec());
    assert_eq!(harita["belgeler/ozet-gunluk.xml"], b"turkce ad".to_vec());
}

#[test]
fn gidis_donus_ikili_veri() {
    let govde: Vec<u8> = (0..=255u8).cycle().take(100_000).collect();
    let girisler = vec![ZipGiris::yeni("media/image1.png", govde.clone())];
    let zip = paketle(&girisler).unwrap();
    assert_eq!(ac(&zip).unwrap()[0].veri, govde);
}

// ------------------------------------------------------------- DEFLATE

#[test]
fn inflate_saklanmis_blok() {
    // zlib level 0 ciktisi: tur 0 (stored) blok.
    let girdi: &[u8] = &[
        0x01, 0x0d, 0x00, 0xf2, 0xff, 0x6d, 0x65, 0x72, 0x68, 0x61, 0x62, 0x61, 0x20, 0x64, 0x75,
        0x6e, 0x79, 0x61,
    ];
    assert_eq!(inflate(girdi, GIRIS_TAVANI).unwrap(), b"merhaba dunya");
}

#[test]
fn inflate_sabit_huffman() {
    // zlib Z_FIXED ciktisi: tur 1 blok, geriye referans (mesafe 1) iceriyor.
    let girdi: &[u8] = &[0x4b, 0x4c, 0xc4, 0x07, 0x00];
    assert_eq!(inflate(girdi, GIRIS_TAVANI).unwrap(), b"a".repeat(30));
}

#[test]
fn inflate_dinamik_huffman() {
    let cozulen = inflate(DINAMIK_DEFLATE, GIRIS_TAVANI).unwrap();
    assert_eq!(cozulen, DINAMIK_HAM);
}

#[test]
fn inflate_gercek_zip_deflate_girisleri() {
    // Python zipfile ile uretilmis, DEFLATE yontemli gercek bir arsiv.
    let harita = ac_harita(GERCEK_ZIP).expect("gercek zip acilmali");
    assert_eq!(harita.len(), 3);
    assert_eq!(
        harita["[Content_Types].xml"],
        br#"<?xml version="1.0"?><Types/>"#.to_vec()
    );
    assert_eq!(
        harita["xl/workbook.xml"],
        format!("<workbook>{}</workbook>", "<sheet/>".repeat(80)).into_bytes()
    );
    assert!(harita["bos.txt"].is_empty());
}

// --------------------------------------------------------- Zip bomb / sinir

#[test]
fn inflate_tavani_zip_bombu_durdurur() {
    // 8 KiB girdi -> 8 MiB cikti. Tavan 64 KiB verilince reddedilmeli.
    let hata = inflate(BOMBA_DEFLATE, 64 * 1024).unwrap_err();
    assert!(matches!(hata, ZipHatasi::SinirAsimi(_)), "beklenen SinirAsimi, gelen {hata:?}");

    // Tavan yeterliyse ayni girdi sorunsuz cozulur.
    let tam = inflate(BOMBA_DEFLATE, GIRIS_TAVANI).unwrap();
    assert_eq!(tam.len(), 8 * 1024 * 1024);
    assert!(tam.iter().all(|&b| b == 0));
}

#[test]
fn tavan_sabitleri_makul() {
    // Tek giris tavani arsiv tavanini asamaz; asarsa arsiv siniri anlamsizlasir.
    const { assert!(GIRIS_TAVANI <= ARSIV_TAVANI) };
}

// ---------------------------------------------------- Bozuk girdi -> Err

#[test]
fn cok_kisa_girdi_hata_verir() {
    for uzunluk in 0..22 {
        let girdi = vec![0u8; uzunluk];
        assert!(ac(&girdi).is_err(), "uzunluk {uzunluk} icin Err bekleniyordu");
    }
}

#[test]
fn eocd_imzasi_bozulunca_hata() {
    let zip = paketle(&[ZipGiris::yeni("a.txt", b"veri".to_vec())]).unwrap();
    let mut bozuk = zip.clone();
    let n = bozuk.len();
    bozuk[n - 22] ^= 0xFF; // EOCD imzasinin ilk bayti
    assert!(matches!(ac(&bozuk), Err(ZipHatasi::Bozuk(_))));
}

#[test]
fn kesik_dosya_panik_yapmaz() {
    let zip = paketle(&[
        ZipGiris::yeni("a.txt", b"birinci giris".to_vec()),
        ZipGiris::yeni("b.txt", b"ikinci giris".to_vec()),
    ])
    .unwrap();
    // Her kesme noktasi: ya Ok (tam dosya) ya Err — ama asla panik.
    for kes in 0..zip.len() {
        let _ = ac(&zip[..kes]);
    }
    // Sonu kirpilmis her surum EOCD'siz kalir, dolayisiyla Err olmali.
    assert!(ac(&zip[..zip.len() - 1]).is_err());
}

#[test]
fn tasan_merkezi_dizin_ofseti_hata() {
    let zip = paketle(&[ZipGiris::yeni("a.txt", b"veri".to_vec())]).unwrap();
    let mut bozuk = zip.clone();
    let n = bozuk.len();
    // EOCD + 16: merkezi dizin ofseti. Dosya boyunun cok otesini gosterelim.
    bozuk[n - 6..n - 2].copy_from_slice(&0xFFFF_FF00u32.to_le_bytes());
    assert!(matches!(ac(&bozuk), Err(ZipHatasi::Bozuk(_))));
}

#[test]
fn tasan_yerel_baslik_ofseti_hata() {
    let zip = paketle(&[ZipGiris::yeni("a.txt", b"veri".to_vec())]).unwrap();
    let mut bozuk = zip.clone();
    // Merkezi dizin kaydi govdeden hemen sonra; yerel ofset alani kayit+42.
    let merkez_bas = u32::from_le_bytes(bozuk[bozuk.len() - 6..bozuk.len() - 2].try_into().unwrap())
        as usize;
    bozuk[merkez_bas + 42..merkez_bas + 46].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
    assert!(ac(&bozuk).is_err());
}

#[test]
fn sisirilmis_kayit_sayisi_hata() {
    let zip = paketle(&[ZipGiris::yeni("a.txt", b"veri".to_vec())]).unwrap();
    let mut bozuk = zip.clone();
    let n = bozuk.len();
    // EOCD + 10: bu diskteki kayit sayisi. 1 yerine 5000 diyelim.
    bozuk[n - 12..n - 10].copy_from_slice(&5000u16.to_le_bytes());
    assert!(ac(&bozuk).is_err(), "olmayan kayitlar Err vermeli");
}

#[test]
fn desteklenmeyen_yontem_hata() {
    let zip = paketle(&[ZipGiris::yeni("a.txt", b"veri".to_vec())]).unwrap();
    let mut bozuk = zip.clone();
    let merkez_bas = u32::from_le_bytes(bozuk[bozuk.len() - 6..bozuk.len() - 2].try_into().unwrap())
        as usize;
    // Merkezi dizin + 10: yontem. 99 = AES, desteklemiyoruz.
    bozuk[merkez_bas + 10..merkez_bas + 12].copy_from_slice(&99u16.to_le_bytes());
    assert!(matches!(
        ac(&bozuk),
        Err(ZipHatasi::DesteklenmeyenYontem(99))
    ));
}

#[test]
fn crc_uyusmazligi_yakalanir() {
    let zip = paketle(&[ZipGiris::yeni("a.txt", b"veri".to_vec())]).unwrap();
    let mut bozuk = zip.clone();
    // Yerel baslik 30 bayt + ad 5 bayt sonrasi govde: tek bayti degistir.
    let govde_bas = 30 + 5;
    bozuk[govde_bas] ^= 0x01;
    assert!(matches!(ac(&bozuk), Err(ZipHatasi::CrcUyusmadi { .. })));
}

#[test]
fn bozuk_deflate_akisi_panik_yapmaz() {
    // Dinamik Huffman vektorunun her bayti tek tek bozulur: hepsi Ok ya da Err,
    // hicbiri panik degil. Huffman tablosu insasi en kirilgan yer.
    for i in 0..DINAMIK_DEFLATE.len() {
        let mut bozuk = DINAMIK_DEFLATE.to_vec();
        bozuk[i] ^= 0xA5;
        let _ = inflate(&bozuk, GIRIS_TAVANI);
    }
    // Kesik akislar da guvenle reddedilmeli.
    for kes in 0..DINAMIK_DEFLATE.len() {
        let _ = inflate(&DINAMIK_DEFLATE[..kes], GIRIS_TAVANI);
    }
}

#[test]
fn fuzz_rastgele_baytlar_panik_yapmaz() {
    // Bagimliliksiz, yinelenebilir sozde-rastgele uretec (xorshift64*).
    let mut durum: u64 = 0x2545_F491_4F6C_DD1D;
    let mut sonraki = move || {
        durum ^= durum << 13;
        durum ^= durum >> 7;
        durum ^= durum << 17;
        durum
    };

    // 1) Tamamen rastgele tamponlar.
    for _ in 0..500 {
        let uzunluk = (sonraki() % 512) as usize;
        let tampon: Vec<u8> = (0..uzunluk).map(|_| (sonraki() & 0xFF) as u8).collect();
        let _ = ac(&tampon);
        let _ = inflate(&tampon, GIRIS_TAVANI);
    }

    // 2) Gecerli bir arsivin rastgele bozulmus surumleri — daha zorlu senaryo,
    //    cunku yapinin cogu tutarli kalir ve okuyucu derinlere iner.
    let temiz = paketle(&[
        ZipGiris::yeni("[Content_Types].xml", b"<Types/>".to_vec()),
        ZipGiris::yeni("xl/workbook.xml", vec![7u8; 300]),
    ])
    .unwrap();
    for _ in 0..2000 {
        let mut bozuk = temiz.clone();
        let bozma_adedi = 1 + (sonraki() % 4) as usize;
        for _ in 0..bozma_adedi {
            let yer = (sonraki() as usize) % bozuk.len();
            bozuk[yer] ^= (sonraki() & 0xFF) as u8;
        }
        let _ = ac(&bozuk);
    }

    // 3) Gercek deflate'li arsivin bozulmus surumleri.
    for _ in 0..1000 {
        let mut bozuk = GERCEK_ZIP.to_vec();
        let yer = (sonraki() as usize) % bozuk.len();
        bozuk[yer] ^= (sonraki() & 0xFF) as u8;
        let _ = ac(&bozuk);
    }
}
