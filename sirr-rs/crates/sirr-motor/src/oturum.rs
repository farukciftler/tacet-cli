//! Oturum sabitleri — arac dongusunun MIMARI ayarlari.
//!
//! NEDEN BURADA, EVAL'DE DEGIL: bu iki sabit uretim davranisidir, olcum ayari
//! degil. Onceden `sirr-eval` icinde duruyorlardi ve `sirr-cli` onlari test
//! altyapisindan cekiyordu; yani uretim ikilisi eval crate'ine bagimliydi ve
//! "kac tur donulur" sorusunun cevabi bir test dosyasinda oynuyordu. Motor
//! katmani her iki tarafin da ortak atasi oldugu icin dogru ev burasi:
//! uygulama ve eval AYNI sabiti gorur, kimse digerine bagimli olmaz.

/// Bir kullanici mesajinda en fazla kac model turu kosulur.
///
/// Dort: en uzun mesru zincir "oku -> olustur -> cevap" (3 tur). Dorduncu tur
/// hata kurtarmaya pay birakir; besincisi bir donguye girildiginin isareti
/// olurdu ve sonsuza kadar kosmak hicbir seyi olcmez.
pub const EN_FAZLA_TUR: usize = 4;

/// Sabit oturum talimati. Eval'in ve uygulamanin AYNI metni gormesi sart:
/// farkli istemle olculen bir davranis uygulamayi baglamaz.
///
/// YER TUTUCU YOK — GERCEK MODELDE IKI KEZ OLCULDU.
///
/// 1. Metin once bicimi `arac_adi({"alan":"deger"})` diye gosteriyordu.
///    Qwen2.5-3B bunu kalip olarak degil YAZILACAK METIN olarak aldi: "Saat
///    kac?" sorusuna harfiyen `arac_adi({"tur":"saat"})` uretti.
/// 2. Yer tutucu `ad(...)` diye kisaltilinca sorun TASINDI, cozulmedi:
///    "Merhaba" gibi arac gerektirmeyen bir mesaja `ad({"alan":"Merhaba"})`
///    uretildi — yani kalip, arac cagrisini hak etmeyen yerde bile cagri
///    tetikledi.
///
/// 3. Yer tutucu kaldirilinca kopyalama BUYUK HARFLI EMIRLERE kaydi: talimatta
///    "arac CAGIRMA" yazinca model selamlamaya "... Uygun Bir Arac CAGIRMA;
///    Kendi Dilinde Merhaba" diye cevap verdi. Yani vurgu icin kullanilan
///    buyuk harf, modelce ICERIK sanildi.
///
/// 4. GERCEK arac uzerinden verilen ornek de kopyalandi — Gemma3-4B ile
///    olculdu. Talimatta `Ornek: hesapla({"ifade":"12*8"})` yazarken
///    "Tesekkurler, iyi gunler." mesajina "Tesekkurler, size de iyi gunler!
///    Hesapla({"ifade":"12*8"})." diye cevap verdi: selamlamayi dogru yapip
///    ornegi HARFIYEN, uydurma argumanlariyla birlikte arkasina yapistirdi.
///    Yani 1. ve 2. maddedeki ariza yer tutucuya ozgu degilmis; SOMUT ORNEGIN
///    KENDISINE ozguymus. Qwen3-4B ayni istemde kopyalamiyordu, bu yuzden
///    ariza tek modelle olculdugunde gorunmuyordu.
///
/// Cikarilan ders: bu boyuttaki bir model SOYUT KALIBI SOMUT ORNEKTEN
/// AYIRAMAZ; istemde gorunen her `xxx(...)` bir kopyalama adayidir — yer
/// tutucu olsun, gercek arac olsun — her BUYUK HARFLI kelime de oyle. Bu
/// yuzden talimatta artik HIC cagri ornegi yok: bicim yalnizca sozle tarif
/// edilir. Ornegin bosalttigi yeri `<araclar>` listesi zaten dolduruyor —
/// arac tarifi kisa imza bicimine gectiginden (`hesapla(ifade: metin,
/// basamak?: tamsayi)`) listenin kendisi cagri seklini gosteriyor, ustelik
/// kopyalanacak somut bir ARGUMAN DEGERI icermeden. Cagri iki savunmayla
/// korunuyor (gramer + katalog kapisi), ama istem modeli en bastan dogru yere
/// itmeli — bosa giden tur de bir maliyet.
pub const SISTEM_TALIMATI: &str = "Sen sirr'sin: tamamen cihaz uzerinde calisan bir asistansin. \
Veri cihazdan cikmaz. Bir arac gerekiyorsa satira once o aracin <araclar> listesindeki \
adini yaz, hemen ardindan parantez ac, argumanlarini tek bir JSON nesnesi olarak ver, \
parantezi kapat ve satiri orada bitir. Arac adini yazmadan dogrudan JSON yazma; \
adi olmayan bir cagri gecersizdir. Ornek: hesapla({\"ifade\":\"12*8\"}). \
Arguman adlari o aracin listedeki imzasinda yazilidir. \
Yalniz listede bulunan adlari kullan ve bir araci yalnizca o mesaj icin gerekiyorsa cagir. \
Selamlasma ve sohbet gibi arac gerektirmeyen mesajlara dogrudan kullanicinin dilinde, \
kendi cumlelerinle cevap ver. \
Istenen bilgi bir <tool_response> blogunda sana verilmisse artik yeni bir arac cagirma; \
o blogun icindeki degeri kendi cumlene yerlestirip kullaniciya Turkce, kisa ve dogrudan \
cevap ver. Arac cagrisini ya da onun JSON'unu cevap diye tekrarlama; sonucu uydurma.";
