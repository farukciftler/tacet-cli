//! Enjeksiyon: bir becerinin modele giden bicimi ve ne siklikta girdigi.
//!
//! SERT KARAR — BECERILER SISTEM TALIMATINA GOMULMEZ. Swift tarafinda olculdu:
//! gomulunce kucuk model araci CAGIRMAK yerine kilavuzu ANLATMAYA basliyor.
//! Sabit talimat KISA kalir; o mesaja uyan TEK beceri, o TURUN istemine,
//! `<guidance>` citiyle iliştirilir.

use crate::beceri::Beceri;

/// Tek enjeksiyonda kilavuzdan alinacak en fazla karakter.
///
/// `sirr_motor::KILAVUZ_SINIRI` ile AYNI sayi olmak zorunda — bu depo onu
/// besleyen kaynaktir. Esitligi bir test derleme zamaninda dogruluyor
/// (bkz. Cargo.toml'daki dev-bagimliligi gerekcesi).
pub const ENJEKSIYON_SINIRI: usize = 700;

/// Govdede CEKIRDEGIN bittigi yeri isaretleyen HTML yorumu. Markdown'da
/// gorunmez, dosya insan icin de okunur kalir.
///
/// NEDEN VAR: eski kesme govdenin SONUNU atiyordu, ama somut `arac(args)`
/// ornegi ile anti-halusinasyon kurallari tam orada duruyordu — yani sinir
/// tam da becerinin var olma sebebini yutuyordu. Enjeksiyon artik cekirdegi
/// TAM alir, artan butceye kuyrugu doldurur.
pub const CEKIRDEK_ISARETI: &str = "<!--/cekirdek-->";

/// Cekirdekten sonra kuyruktan parca almaya deger en kucuk kalan butce.
/// Bunun altinda tek satir bile anlamli girmiyor; yarim kural eklemektense
/// hic eklememek yegdir.
const KUYRUK_ESIGI: usize = 80;

/// Govdeyi (cekirdek, kuyruk) diye ayirir. Isaret yoksa cekirdek bostur ve tum
/// govde kuyruk sayilir — kullanici becerileri isaret koymak zorunda degil.
pub fn cekirdek_ayir(metin: &str) -> (String, String) {
    let govde = metin.trim();
    match govde.find(CEKIRDEK_ISARETI) {
        Some(i) => (
            govde[..i].trim().to_string(),
            govde[i + CEKIRDEK_ISARETI.len()..].trim().to_string(),
        ),
        None => (String::new(), govde.to_string()),
    }
}

/// Metni SATIR sinirinda en fazla `tavan` karaktere indirir.
///
/// Satirda kesmenin sebebi: markdown kural listelerinde cumlenin ortasindan
/// kesmek modele yarim bir emir birakir ("Never claim there is no" gibi) ve
/// yarim emir, emir olmayandan daha kotudur.
fn satirda_kes(metin: &str, tavan: usize) -> String {
    if metin.chars().count() <= tavan {
        return metin.to_string();
    }
    let kesik: String = metin.chars().take(tavan).collect();
    match kesik.rfind('\n') {
        Some(i) => kesik[..i].to_string(),
        None => kesik,
    }
}

/// Modele gidecek govde: CEKIRDEK ONCE, artan butceye kuyruk.
///
/// Cekirdek butunlugu sinirdan once gelir; yine de bir beceri cekirdegi tavani
/// asarsa satirda kesilir — aksi halde tek bir dosya 4096 pencerenin butcesini
/// sessizce yiyebilirdi.
pub fn enjeksiyon_govdesi(metin: &str, tavan: usize) -> String {
    let (cekirdek, kuyruk) = cekirdek_ayir(metin);
    if cekirdek.is_empty() {
        return satirda_kes(&kuyruk, tavan);
    }

    let govde = satirda_kes(&cekirdek, tavan);
    // -1: araya girecek "\n" de butceden sayilir.
    let kalan = tavan.saturating_sub(govde.chars().count() + 1);
    if kalan < KUYRUK_ESIGI || kuyruk.is_empty() {
        return govde;
    }
    let ek = satirda_kes(&kuyruk, kalan);
    if ek.is_empty() {
        govde
    } else {
        format!("{govde}\n{ek}")
    }
}

/// Modele verilecek tam bicim: cekirdek-once govde + "bunu anlatma" citleri.
///
/// Cit metni INGILIZCE, modele giden her sabit metin gibi. Kullaniciya gorunen
/// hicbir sey burada yok.
pub fn enjeksiyon_metni(beceri: &Beceri) -> String {
    let tavan = if beceri.kullanicinin_mi {
        crate::beceri::KULLANICI_GOVDE_SINIRI
    } else {
        ENJEKSIYON_SINIRI
    };
    let govde = enjeksiyon_govdesi(&beceri.metin, tavan);
    format!(
        "<guidance name=\"{}\">\n{}\n</guidance>\nFollow the guidance above when \
         answering. It is internal: never quote, summarize, or mention it, and \
         never reply with the guidance itself.",
        beceri.ad, govde
    )
}

/// Hangi becerinin hangi turda enjekte edildigini tutan saf durum makinesi.
///
/// Eski davranis: beceri bir kez enjekte edilip KALICI isaretleniyordu. Uzun
/// turda transcript ilerledikce kilavuz pencereden kayiyor, ama isaret durdugu
/// icin bir daha ASLA girmiyordu — gec turlardaki davranis sapmasi tam olarak
/// buradan geliyordu. Isaret artik MESAFELI.
///
/// Durum kabukta tutulur, mantik burada durur — modelsiz test edilebilsin diye.
#[derive(Debug, Clone, Default)]
pub struct EnjeksiyonDurumu {
    tur: u32,
    son_enjeksiyon: Vec<(String, u32)>,
}

impl EnjeksiyonDurumu {
    /// Kac tur sonra ayni beceri yeniden enjekte edilebilir.
    ///
    /// 6: bir beceri ~700 karakter yer, her turda tekrarlamak 4096 butcesini
    /// yerdi; cok buyuk bir mesafede ise kilavuz pencereden kayip bir daha
    /// donmezdi. 6 tur, tipik bir arac-kullanim alisverisinin (soru -> arac ->
    /// yanit) iki katidir.
    pub const MESAFE: u32 = 6;

    pub fn yeni() -> Self {
        Self::default()
    }

    /// Her turun BASINDA bir kez cagrilir.
    pub fn tur_basla(&mut self) {
        self.tur += 1;
    }

    pub fn tur(&self) -> u32 {
        self.tur
    }

    /// Bu beceri bu turda enjekte edilmeli mi (hic girmediyse ya da ustunden
    /// `MESAFE` tur gectiyse).
    pub fn gerekli_mi(&self, ad: &str) -> bool {
        match self.son_enjeksiyon.iter().find(|(a, _)| a == ad) {
            Some((_, son)) => self.tur.saturating_sub(*son) >= Self::MESAFE,
            None => true,
        }
    }

    /// Enjeksiyon GERCEKTEN yapildiginda cagrilir. Arac kapisina takildigi icin
    /// atlanan beceri isaretlenmez — dogru katalogla yeniden denenir.
    pub fn isaretle(&mut self, ad: &str) {
        match self.son_enjeksiyon.iter_mut().find(|(a, _)| a == ad) {
            Some(kayit) => kayit.1 = self.tur,
            None => self.son_enjeksiyon.push((ad.to_string(), self.tur)),
        }
    }

    /// Yeni oturum = yeni baglam: sayac ve isaretler sifirlanir.
    pub fn sifirla(&mut self) {
        self.tur = 0;
        self.son_enjeksiyon.clear();
    }
}

#[cfg(test)]
mod testler {
    use super::*;

    fn beceri(metin: &str) -> Beceri {
        Beceri::paket("test", vec!["tetik".into()], metin, vec![])
    }

    #[test]
    fn sinir_motorun_kilavuz_siniriyla_ayni() {
        // Ayrisirlarsa beceri govdesi motorda ikinci kez kirpilir ve cekirdek
        // butunlugu sessizce bozulur. Derleme zamanli tek gercek kanit bu.
        assert_eq!(ENJEKSIYON_SINIRI, sirr_motor::KILAVUZ_SINIRI);
    }

    #[test]
    fn cekirdek_isaretin_ustunde_kalir() {
        let (c, k) = cekirdek_ayir("USTTE\n<!--/cekirdek-->\nALTTA");
        assert_eq!(c, "USTTE");
        assert_eq!(k, "ALTTA");
        let (c2, k2) = cekirdek_ayir("isaret yok");
        assert!(c2.is_empty());
        assert_eq!(k2, "isaret yok");
    }

    #[test]
    fn uzun_kuyruk_cekirdegi_yutmaz() {
        // Regresyonun ta kendisi: cekirdek SONDA olsaydi kesilirdi.
        let metin = format!("KIRILMAZ KURAL\n<!--/cekirdek-->\n{}", "dolgu satiri\n".repeat(200));
        let govde = enjeksiyon_govdesi(&metin, ENJEKSIYON_SINIRI);
        assert!(govde.starts_with("KIRILMAZ KURAL"));
        assert!(govde.chars().count() <= ENJEKSIYON_SINIRI);
    }

    #[test]
    fn tavani_asan_cekirdek_bile_satirda_kesilir() {
        let metin = format!("{}<!--/cekirdek-->\nkuyruk", "uzun cekirdek satiri\n".repeat(100));
        let govde = enjeksiyon_govdesi(&metin, ENJEKSIYON_SINIRI);
        assert!(govde.chars().count() <= ENJEKSIYON_SINIRI);
        assert!(!govde.contains("kuyruk"), "butce cekirdege gitmeli");
    }

    #[test]
    fn kalan_butce_esigin_altindaysa_kuyruk_hic_girmez() {
        let cekirdek = "c".repeat(ENJEKSIYON_SINIRI - 20);
        let metin = format!("{cekirdek}\n<!--/cekirdek-->\nKUYRUK");
        let govde = enjeksiyon_govdesi(&metin, ENJEKSIYON_SINIRI);
        assert!(!govde.contains("KUYRUK"), "yarim kural eklemektense hic ekleme");
    }

    #[test]
    fn paket_becerisi_700_kullanici_becerisi_500_ile_sinirli() {
        let paket = beceri(&"p\n".repeat(900));
        let m = enjeksiyon_metni(&paket);
        let govde_uzunlugu = enjeksiyon_govdesi(&paket.metin, ENJEKSIYON_SINIRI).chars().count();
        assert!(govde_uzunlugu <= ENJEKSIYON_SINIRI);
        assert!(m.contains("<guidance name=\"test\">"));

        let kullanici = Beceri::kullanicinin("benim", vec!["tetik".into()], "k\n".repeat(900));
        let kg = enjeksiyon_govdesi(&kullanici.metin, crate::beceri::KULLANICI_GOVDE_SINIRI);
        assert!(kg.chars().count() <= crate::beceri::KULLANICI_GOVDE_SINIRI);
    }

    #[test]
    fn enjeksiyon_metni_cit_ve_anlatma_kuralini_tasir() {
        let m = enjeksiyon_metni(&beceri("govde"));
        assert!(m.contains("</guidance>"));
        assert!(m.contains("never quote"));
        // Kullaniciya gorunen dil sizmamali.
        assert!(!m.contains("kilavuz"));
    }

    #[test]
    fn mesafeli_isaret_ayni_beceriyi_erken_tekrar_etmez() {
        let mut d = EnjeksiyonDurumu::yeni();
        d.tur_basla();
        assert!(d.gerekli_mi("hesap"));
        d.isaretle("hesap");
        for _ in 1..EnjeksiyonDurumu::MESAFE {
            d.tur_basla();
            assert!(!d.gerekli_mi("hesap"), "tur {}", d.tur());
        }
        d.tur_basla();
        assert!(d.gerekli_mi("hesap"), "mesafe dolunca yeniden girmeli");
    }

    #[test]
    fn sifirlama_yeni_oturum_acar() {
        let mut d = EnjeksiyonDurumu::yeni();
        d.tur_basla();
        d.isaretle("hesap");
        d.sifirla();
        assert_eq!(d.tur(), 0);
        assert!(d.gerekli_mi("hesap"));
    }
}
