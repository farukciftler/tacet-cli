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
pub const SISTEM_TALIMATI: &str = "Sen sirr'sin: tamamen cihaz uzerinde calisan bir asistansin. \
Veri cihazdan cikmaz. Bir arac gerekiyorsa YALNIZ su bicimde tek satir yaz: \
arac_adi({\"alan\":\"deger\"}). Arac gerekmiyorsa dogrudan kullanicinin dilinde cevap ver. \
Arac sonucunu kendi cumlene yerlestir; sonucu UYDURMA.";
