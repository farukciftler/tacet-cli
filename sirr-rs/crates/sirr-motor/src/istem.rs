//! Istem kurulumu — modele giden metnin TEK uretim yeri.
//!
//! Istem parcalanmis halde tutulur (sistem / arac tarifi / gecmis / kilavuz /
//! soru) ve ancak `metin()` cagrisinda birlestirilir. Gerekce: baglam butcesi
//! asildiginda kirpma politikasi hangi parcanin feda edilebilecegini bilmek
//! zorunda. Istem tek bir `String` olsaydi kirpma "sondan kes"ten ibaret
//! kalir, ilk kurban her zaman sorunun kendisi olurdu.
//!
//! BECERI KARARI (Swift tarafindan devraliniyor, olcumle alinmis): kilavuz
//! metni SISTEM TALIMATINA GOMULMEZ. Olcum, kucuk cihaz-ustu modelin fazla
//! sabit talimat altinda araci CAGIRMAK yerine ne yapacagini ANLATMAYA
//! basladigini gosterdi. Bunun yerine o mesaja uyan TEK beceri, o turun
//! istemine `<guidance>` citiyle ve `KILAVUZ_SINIRI` karakterle sinirli
//! olarak iliştirilir. Kilavuzun sorunun HEMEN ONUNDE durmasi da bilincli:
//! kucuk modelde son bloklarin agirligi en yuksektir.

use sirr_cekirdek::AracKatalogu;

/// Istemin tel bicimi — parcalarin hangi citlerle birlestirilecegi.
///
/// NEDEN MOTOR BILDIRIR, CLI SECMEZ: sablon modelin egitildigi bicimdir, bir
/// kullanici tercihi degil. Elle secilebilir olsaydi yanlis sec­mek sessizce
/// bozuk cikti uretirdi — talimat-ayarli bir model kendi citlerini gormeyince
/// rolleri kaybeder ve gevezelige duser. Bu yuzden `MotorSaglayici::sablon`
/// bunu SOYLER; cagri yeri yalnizca uyar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sablon {
    /// Etiketli duz metin. `SahteMotor` ve testler icin: okunabilir ve
    /// belirtecleyici gerektirmez.
    #[default]
    Duz,
    /// ChatML — Qwen2.5/Qwen3 ailesinin egitildigi bicim:
    /// `<|im_start|>rol\n...<|im_end|>`. Kapanis `<|im_end|>` ayni zamanda
    /// bitis belirtecidir, yani uretimin nerede duracagini da bu bicim tanimlar.
    ChatML,
    /// Gemma — `<start_of_turn>rol\n...<end_of_turn>`.
    ///
    /// ChatML'DEN TEK BIR ETIKET DEGISIKLIGI DEGILDIR, iki yapisal farki var
    /// ve ikisi de sessiz bozulma kaynagi:
    ///
    /// 1. **SISTEM ROLU YOKTUR.** Gemma sohbet sablonunda yalniz `user` ve
    ///    `model` turleri vardir. `<start_of_turn>system` yazmak modelin hic
    ///    gormedigi bir dizgidir; egitim dagiliminin disina cikar ve talimat
    ///    siradan metne dondugu icin arac tarifi baglayiciligini yitirir.
    ///    Bu yuzden sistem talimati + arac tarifi + hafiza ILK KULLANICI
    ///    TURUNUN basina gomulur.
    /// 2. **ASISTAN ROLUNUN ADI `model`dir**, `assistant` degil.
    ///
    /// Bitis belirteci `<end_of_turn>`; `bitis_belirtecleri_bul` onu zaten
    /// ariyor ve Gemma dagarciginda ozel isaretli oldugu icin buluyor.
    Gemma,
}

/// Tek enjeksiyonda kilavuzdan alinacak en fazla karakter.
///
/// Swift tarafiyla ayni sayi (700). Daha buyugu 4096 pencerede gecmisi
/// yiyor, daha kucugu somut `arac(args)` ornegini ve anti-halusinasyon
/// kurallarini kesiyordu — kilavuzun var olma sebebi tam da o kisim.
pub const KILAVUZ_SINIRI: usize = 700;

/// Bir konusma turunun kaynagi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rol {
    Kullanici,
    Asistan,
    /// Arac sonucunun modele donen KISA metni. Toplu veri burada olmaz
    /// (bkz. VeriDeposu bypass kanali).
    Arac,
}

impl Rol {
    fn etiket(self) -> &'static str {
        match self {
            Rol::Kullanici => "Kullanici",
            Rol::Asistan => "Asistan",
            Rol::Arac => "Arac",
        }
    }

    /// ChatML rol adi. ASCII ve INGILIZCE olmasi zorunlu: bunlar modelin
    /// egitim verisinde gectigi dizgilerdir, bizim sozlugumuzun parcasi degil.
    /// Turkcelestirmek (`kullanici`) modelin tanidigi capayi kirardi.
    ///
    /// ARAC ROLU DE `user`dir — `tool` DEGIL. Qwen3'un GGUF ustverisindeki
    /// resmi sablonu dogrulandi: `message.role == "tool"` dalinda yazilan cit
    /// `<|im_start|>user` olup govde `<tool_response>...</tool_response>` ile
    /// sarilir. Onceden burada `tool` yaziliyordu; model egitimde hic gormedigi
    /// bir rol adi gorup arac sonucunu baglamsiz metin sanip araci tekrar
    /// cagiriyordu.
    fn chatml(self) -> &'static str {
        match self {
            Rol::Kullanici | Rol::Arac => "user",
            Rol::Asistan => "assistant",
        }
    }

    /// Gemma rol adi. Gemma sablonunda YALNIZ iki tur vardir: `user` ve
    /// `model`. Arac ciktisi da `user` olarak yazilir — cunku ucuncu bir rol
    /// adi uydurmak modelin hic gormedigi bir cit olurdu; arac sonucu modele
    /// DISARIDAN gelen bir bilgidir, yani kullanici tarafina aittir.
    fn gemma(self) -> &'static str {
        match self {
            Rol::Kullanici | Rol::Arac => "user",
            Rol::Asistan => "model",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Tur {
    pub rol: Rol,
    pub metin: String,
}

impl Tur {
    pub fn kullanici(metin: impl Into<String>) -> Self {
        Self { rol: Rol::Kullanici, metin: metin.into() }
    }
    pub fn asistan(metin: impl Into<String>) -> Self {
        Self { rol: Rol::Asistan, metin: metin.into() }
    }
    pub fn arac(metin: impl Into<String>) -> Self {
        Self { rol: Rol::Arac, metin: metin.into() }
    }

    fn yaz(&self, hedef: &mut String) {
        hedef.push_str(self.rol.etiket());
        hedef.push_str(": ");
        hedef.push_str(&self.metin);
        hedef.push('\n');
    }
}

/// Modele gonderilecek istemin parcali hali.
#[derive(Debug, Clone, Default)]
pub struct Istem {
    /// Sabit oturum talimati. KIRPILMAZ — kimlik, dil capasi ve arac
    /// cagirma sozlesmesi burada; kirpilirsa model ne oldugunu unutur.
    pub sistem: String,
    /// Katalogdan turetilmis arac tarifi. Kirpilmaz: eksik tarif, modelin
    /// var olmayan bir imza uydurmasi demektir.
    pub araclar: String,
    /// Kalici hafizadan bu mesaja uyan notlar, `<hafiza>` citli. Sistem
    /// blogunda durur — kimlik/tercih olgusu bir konusma turu degil, sabit
    /// baglamdir; gecmis gibi kirpilirsa "kullanici vejetaryen" olgusu ucuncu
    /// turda kaybolur. Kabuk `sirr_hafiza` deposundan mesaja gore doldurur.
    pub hafiza: Option<String>,
    /// Eski turlar. Butce baskisinda ILK feda edilen budur.
    pub gecmis: Vec<Tur>,
    /// O mesaja uyan TEK beceri kilavuzu, `<guidance>` citli. Sorunun hemen onunde.
    pub kilavuz: Option<String>,
    /// Bu turun kullanici sorusu. Kirpilir ama ASLA dusurulmez.
    ///
    /// BOS BIRAKILABILIR ve bunun ozel bir anlami vardir: "sorulacak yeni bir
    /// sey yok, gecmisin BITTIGI yerden devam et". Arac dongusunun ikinci ve
    /// sonraki turlari boyledir — soru zaten gecmiste, arac cagrisinin ONUNDE
    /// duruyor. Bos birakilmazsa soru arac sonucunun ARDINA ikinci kez yazilir
    /// ve model onu cevaplanmamis yeni bir istek sanip araci yeniden cagirir.
    pub soru: String,
}

impl Istem {
    pub fn yeni(sistem: impl Into<String>, soru: impl Into<String>) -> Self {
        Self { sistem: sistem.into(), soru: soru.into(), ..Default::default() }
    }

    /// Arac tarifini KATALOGDAN turetir — elle yazilmis ikinci bir liste yok.
    ///
    /// Sira katalogun `Vec` sirasidir: ayni katalog her calistirmada
    /// bit-birebir ayni istemi uretsin, eval sonuclari karsilastirilabilir olsun.
    pub fn araclarla(mut self, katalog: &AracKatalogu) -> Self {
        let mut m = String::new();
        for arac in katalog.araclar() {
            // KISA IMZA, tam JSON Schema DEGIL (bkz. `ArgSema::kisa_imza`):
            // argumanlari gramer zorluyor, sema iki yerde tutulmuyor.
            m.push_str("- ");
            m.push_str(arac.ad());
            m.push('(');
            m.push_str(&arac.sema().kisa_imza());
            m.push_str(") — ");
            m.push_str(arac.aciklama().trim());
            m.push('\n');
        }
        self.araclar = m;
        self
    }

    /// Hafiza notlarini isteme ekler. Bos/bosluk metin YOK sayilir: bos bir
    /// `<hafiza>` citi modele "hafizan var ama bos" diye anlamsiz bir sinyal
    /// verirdi. Butce zaten `sirr_hafiza::ENJEKSIYON_SINIRI` ile depoda kesildi;
    /// burada tekrar kesmiyoruz ki iki farkli tavan sessizce catismasin.
    pub fn hafizayla(mut self, notlar: impl AsRef<str>) -> Self {
        let n = notlar.as_ref().trim();
        self.hafiza = (!n.is_empty()).then(|| n.to_string());
        self
    }

    /// Kilavuzu `KILAVUZ_SINIRI` karakterle SINIRLAYARAK ekler.
    ///
    /// URETIME BAGLI (kabuk fazi): `sirr-cli` her turda `BeceriDeposu::eslesen`
    /// ile mesaja uyan TEK beceriyi secip `enjeksiyon_metni` ciktisini buraya
    /// veriyor. 700 siniri artik yalniz bir birim testini degil, gercek uretim
    /// yolunu da baglar. Bagli oldugu yer: `Istem::yeni(...).araclarla(...)
    /// .kilavuzla(secilen_beceri)`.
    ///
    /// Kesme BASTAN alir (sondan degil): beceri dosyalari "cekirdek-once"
    /// yazilir — somut cagri ornegi ve kirilmaz kurallar en ustte durur.
    /// Sondan kesmek tam da o cekirdegi birakip insan referansini atardi;
    /// bastan almak tersini yapar.
    pub fn kilavuzla(mut self, kilavuz: impl AsRef<str>) -> Self {
        let k = kilavuz.as_ref();
        let kirpilmis: String = k.chars().take(KILAVUZ_SINIRI).collect();
        self.kilavuz = (!kirpilmis.trim().is_empty()).then_some(kirpilmis);
        self
    }

    pub fn gecmisle(mut self, turler: impl IntoIterator<Item = Tur>) -> Self {
        self.gecmis = turler.into_iter().collect();
        self
    }

    /// Parcalari tek metne birlestirir. Sira sabittir ve anlamlidir:
    /// talimat -> araclar -> gecmis -> kilavuz -> soru. Soru EN SONDA kalir.
    ///
    /// Duz bicim. Sablonu motor bildiriyorsa `metin_sablonlu` kullanilir;
    /// bu metot onun `Sablon::Duz` dalidir ve baglam butcesi olcumu
    /// (`TokenSayaci`) ile CLI'nin `--goster-istem` ciktisinin dayanagidir.
    pub fn metin(&self) -> String {
        self.metin_sablonlu(Sablon::Duz)
    }

    /// Istemi VERILEN sablonun tel biciminde uretir.
    ///
    /// Parca sirasi sablondan BAGIMSIZ olarak aynidir; degisen yalnizca citler.
    /// Boylece kirpma politikasi (hangi parca once feda edilir) tek yerde kalir
    /// ve sablon eklemek onu etkilemez.
    pub fn metin_sablonlu(&self, sablon: Sablon) -> String {
        match sablon {
            Sablon::Duz => self.duz_metin(),
            Sablon::ChatML => self.chatml_metin(),
            Sablon::Gemma => self.gemma_metin(),
        }
    }

    /// Gemma sablonu. Parca SIRASI ChatML ile aynidir (kirpma politikasi tek
    /// yerde kalsin diye); degisen, sistem blogunun NEREYE gittigi.
    ///
    /// Sistem talimati ayri bir tur DEGIL, ilk kullanici turunun basi. Gemma'nin
    /// sistem rolu yok (bkz. `Sablon::Gemma`); ayri tur yazmak modeli egitim
    /// dagiliminin disina cikarirdi.
    fn gemma_metin(&self) -> String {
        let mut m = String::with_capacity(self.kaba_uzunluk());

        // Sistem blogu ilk kullanici turunun ICINE gomulur.
        m.push_str("<start_of_turn>user\n");
        m.push_str(self.sistem.trim());
        if !self.araclar.trim().is_empty() {
            m.push_str("\n\n<araclar>\n");
            m.push_str(self.araclar.trim_end());
            m.push_str("\n</araclar>");
        }
        if let Some(h) = &self.hafiza {
            m.push_str("\n\n<hafiza>\n");
            m.push_str(h.trim());
            m.push_str("\n</hafiza>");
        }

        // Gecmis + soru TEK bir blok listesine cevrilir, sonra ARDISIK AYNI
        // ROLLER BIRLESTIRILIR.
        //
        // NEDEN BIRLESTIRME SART: Gemma sablonu donusumlu (user/model) olmak
        // zorundadir ve arac sonucu da `user` rolune duser (bkz. `Rol::gemma`).
        // Birlestirme olmasaydi "arac sonucu (user) + soru (user)" art arda iki
        // `user` turu uretirdi — modelin egitim dagiliminda olmayan bir dizilim.
        // Sistem blogu da ilk `user` turunun basi oldugu icin bu listeye dahil.
        let mut bloklar: Vec<(&'static str, String)> = Vec::with_capacity(self.gecmis.len() + 1);
        for t in &self.gecmis {
            let govde = if t.rol == Rol::Arac {
                // `<tool_response>` Gemma'da OZEL BELIRTEC DEGIL, duz metindir.
                // Yine de yaziliyor: amac ozel belirtec degil, arac sonucunun
                // nerede baslayip bittigini ISARETLEMEK.
                format!("<tool_response>\n{}\n</tool_response>", t.metin.trim())
            } else {
                t.metin.trim().to_string()
            };
            bloklar.push((t.rol.gemma(), govde));
        }
        // Soru bossa kapanis kullanici blogu eklenmez (bkz. `soru` alani).
        if !self.soru.trim().is_empty() {
            let mut soru = String::new();
            self.gemma_soruyu_yaz(&mut soru);
            bloklar.push(("user", soru));
        }

        // Ilk blok `user` ise sistem blogunun devami olarak ayni ture yapisir.
        let mut acik: Option<&'static str> = Some("user");
        for (rol, govde) in bloklar {
            match acik {
                Some(a) if a == rol => m.push_str("\n\n"),
                _ => {
                    if acik.is_some() {
                        m.push_str("<end_of_turn>\n");
                    }
                    m.push_str("<start_of_turn>");
                    m.push_str(rol);
                    m.push('\n');
                    acik = Some(rol);
                }
            }
            m.push_str(&govde);
        }
        m.push_str("<end_of_turn>\n");

        // Uretim capasi.
        m.push_str("<start_of_turn>model\n");
        m
    }

    /// Kilavuz + soru. Kilavuz sorunun HEMEN ONUNDE durur (bkz. dosya basi
    /// BECERI KARARI); iki cagri yerinde ayni sirayi tekrar yazmamak icin ayri.
    fn gemma_soruyu_yaz(&self, m: &mut String) {
        if let Some(k) = &self.kilavuz {
            m.push_str("<guidance>\n");
            m.push_str(k.trim());
            m.push_str("\n</guidance>\n");
        }
        m.push_str(self.soru.trim());
    }

    /// ChatML: sistem talimati ve arac tarifi TEK sistem turunda birlesir.
    ///
    /// NEDEN BIRLESIK: arac tarifi sabit bir sozlesmedir, konusma turu degil.
    /// Ayri bir tur olarak yazilsaydi gecmisin icine karisir ve butce baskisi
    /// altinda kirpma onu "eski tur" sanip atabilirdi — oysa tarif kirpilmaz
    /// olmak zorunda (bkz. `araclar` alani).
    fn chatml_metin(&self) -> String {
        let mut m = String::with_capacity(self.kaba_uzunluk());

        m.push_str("<|im_start|>system\n");
        m.push_str(self.sistem.trim());
        if !self.araclar.trim().is_empty() {
            m.push_str("\n\n<araclar>\n");
            m.push_str(self.araclar.trim_end());
            m.push_str("\n</araclar>");
        }
        // Hafiza sistem blogunda: sabit kullanici baglami, konusma turu degil.
        if let Some(h) = &self.hafiza {
            m.push_str("\n\n<hafiza>\n");
            m.push_str(h.trim());
            m.push_str("\n</hafiza>");
        }
        m.push_str("<|im_end|>\n");

        // ARAC SONUCLARI `<tool_response>` ile sarilir ve ARDISIK olanlar TEK
        // `user` turunda birlesir — Qwen3'un resmi sablonundaki davranis.
        // Birlestirme bosuna bir ayrinti degil: her arac sonucu icin ayri bir
        // user turu acmak, gecmiste art arda gelen kullanici turleri uretir ve
        // modelin gordugu "son soru" kayar.
        let mut i = 0;
        while i < self.gecmis.len() {
            let t = &self.gecmis[i];
            if t.rol == Rol::Arac {
                m.push_str("<|im_start|>user");
                while i < self.gecmis.len() && self.gecmis[i].rol == Rol::Arac {
                    m.push_str("\n<tool_response>\n");
                    m.push_str(self.gecmis[i].metin.trim());
                    m.push_str("\n</tool_response>");
                    i += 1;
                }
                m.push_str("<|im_end|>\n");
                continue;
            }
            m.push_str("<|im_start|>");
            m.push_str(t.rol.chatml());
            m.push('\n');
            m.push_str(t.metin.trim());
            m.push_str("<|im_end|>\n");
            i += 1;
        }

        // Kilavuz sorunun ICINDE, hemen onunde durur — ayri bir tur olsaydi
        // araya rol siniri girer ve kucuk modelde son blogun agirlik ustunlugu
        // (bkz. dosya basi BECERI KARARI) kaybolurdu.
        // SORU BOSSA kapanis kullanici turu HIC ACILMAZ (bkz. `soru` alani):
        // gecmis zaten bir `<tool_response>` user turuyle bitiyor ve model
        // dogrudan oradan devam etmeli.
        if !self.soru.trim().is_empty() {
            m.push_str("<|im_start|>user\n");
            if let Some(k) = &self.kilavuz {
                m.push_str("<guidance>\n");
                m.push_str(k.trim());
                m.push_str("\n</guidance>\n");
            }
            m.push_str(self.soru.trim());
            m.push_str("<|im_end|>\n");
        }

        // Uretim capasi: model buradan itibaren yazar. Sonunda `\n` YOKTUR —
        // ChatML'de rol satirindan sonraki tek satir sonu zaten burada verildi.
        m.push_str("<|im_start|>assistant\n");
        m
    }

    fn duz_metin(&self) -> String {
        let mut m = String::with_capacity(self.kaba_uzunluk());
        m.push_str("<sistem>\n");
        m.push_str(self.sistem.trim());
        m.push_str("\n</sistem>\n");

        if !self.araclar.trim().is_empty() {
            m.push_str("<araclar>\n");
            m.push_str(self.araclar.trim_end());
            m.push_str("\n</araclar>\n");
        }

        if let Some(h) = &self.hafiza {
            m.push_str("<hafiza>\n");
            m.push_str(h.trim());
            m.push_str("\n</hafiza>\n");
        }

        if !self.gecmis.is_empty() {
            m.push_str("<gecmis>\n");
            for t in &self.gecmis {
                t.yaz(&mut m);
            }
            m.push_str("</gecmis>\n");
        }

        if let Some(k) = &self.kilavuz {
            m.push_str("<guidance>\n");
            m.push_str(k.trim());
            m.push_str("\n</guidance>\n");
        }

        // Soru bossa yalniz uretim capasi yazilir (bkz. `soru` alani).
        if !self.soru.trim().is_empty() {
            m.push_str("Kullanici: ");
            m.push_str(self.soru.trim());
            m.push('\n');
        }
        m.push_str("Asistan: ");
        m
    }

    fn kaba_uzunluk(&self) -> usize {
        self.sistem.len()
            + self.araclar.len()
            + self.soru.len()
            + self.hafiza.as_ref().map_or(0, String::len)
            + self.kilavuz.as_ref().map_or(0, String::len)
            + self.gecmis.iter().map(|t| t.metin.len() + 12).sum::<usize>()
            + 64
    }
}
