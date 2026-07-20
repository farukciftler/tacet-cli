//
//  KodCalistirAraci.swift
//  ketum
//
//  Kod çalıştırma aracı (kod-spec §5). Model küçük bir betik yazar, KodMotoru
//  sandbox'ta çalıştırır, yalnızca doğrulanmış çıktı sunulur. Yaz → çalıştır →
//  doğrula → sun; model sonucu iddia etmez, araç çalıştırır.
//
//  Deneme sayacı (kod-spec §5.4): tur başına en fazla 2 gerçek çalıştırma.
//  İkinci deneme de düşerse modele daha o an `error_final` döner; üçüncü
//  çağrı motoru HİÇ çalıştırmadan reddedilir (emniyet kemeri) — modelin
//  sayması beklenmez, ret araçtadır. Sayaç `KodDurumu`nda yaşar; entegratör
//  bunu AracYurutucu'nun tur kancasına bağlar (tur başına sıfırlanır).
//
//  Sözleşmede DİL ALANI YOKTUR; tek motor JS'tir — "python" istense de JS ile
//  çözülür, dil bir uygulama ayrıntısıdır (§5.1).
//

import Foundation
import FoundationModels

/// Tur başına deneme sayacı. Entegratör `yeniTur()`u AracYurutucu'nun tur
/// kancasına (`turKancasi`) bağlar — sıfırlama böylece kod-spec §5.4'ün
/// dediği yerde, AracYurutucu.yeniTur içinde ve TEK noktadan gerçekleşir.
@MainActor
final class KodDurumu {
    /// Bu turdaki gerçek çalıştırma sayısı.
    var deneme = 0
    /// Yeni tur — sayaç sıfırlanır (kod-spec §5.4).
    func yeniTur() { deneme = 0 }
}

struct KodCalistirAraci: KetumAraci {
    let name = "kod_calistir"
    let description = "Runs a short JavaScript in an isolated sandbox with NO network, NO filesystem and NO access to this device or any server. Call this for any calculation or transformation too complex for the hesapla tool (loops, dates, text processing, simulations). Date, JSON, Math, RegExp and Intl are available. Never call it to inspect servers, containers, ports, processes, disks, networks or the operating system — it cannot see them, and guessing their state is worse than saying you cannot check. The script MUST print its result with print(...) or console.log(...); a script that prints nothing returns an error. If the tool returns an error, fix the code and call it ONCE more."

    weak var raporlayici: (any AracRaporlayici)?
    /// Deneme sayacı — raporlayici deseniyle zayıf referans, döngü olmaz.
    weak var durum: KodDurumu?

    @Generable
    struct Arguments {
        @Guide(description: "The script. Keep it minimal; print the final result.")
        var kod: String
        // `dil` ALANI KALDIRILDI (P2-3). Ölü bir alandı: tek geçerli değeri
        // "js" idi, hiçbir dallanmada okunmuyordu, ama küçük modelin HER
        // çağrıda doldurması gereken bir decode slotu tüketiyordu. Motor
        // zaten tek: JS (§5.1) — dil bir uygulama ayrıntısı, sözleşme değil.
    }

    /// Modele dönen çıktının tavanı — tam çıktı çipe gider (kod-spec §5.2).
    private static let modelCiktiTavani = 500
    /// Hata metninin tavanı. Hata raporu artık kaynak satırı ve kısmi çıktı
    /// da taşıyor; 4096 bütçesini yemesin diye burada kırpılır (kod-spec §7).
    private static let modelHataTavani = 400

    /// Betik Python'a mı benziyor? ÖLÇÜM BULGUSU: küçük model kod vakalarının
    /// çoğunda Python yazıyor (`for i in range(2,101):`, `print(i, end=' ')`),
    /// motor JS olduğu için `SyntaxError` alıyor ve turu kaybediyor. Ham JS
    /// ayrıştırıcı mesajı ("Unexpected identifier 'i'") düzeltmek için YETMEZ:
    /// Python yazdığını sanan bir modele "beklenen '(' " demek yanlış onarıma
    /// götürür. Bu yüzden nedeni araç söyler — modelin kalan TEK denemesi
    /// boşa gitmesin.
    ///
    /// Saf ve statik: modelsiz test edilir.
    static func pythonMu(_ kod: String) -> Bool {
        // Bu belirteçlerin her biri JS'te ya sözdizimi hatası ya tanımsızdır;
        // geçerli bir JS betiğini yanlışlıkla Python sanma riski yoktur.
        // `print(` BİLEREK yok: motor `print`i JS'te de tanır (kod-spec §5.2).
        let pythonaOzgu = ["range(", "def ", "elif ", "end=", " True", " False", " None"]
        if pythonaOzgu.contains(where: kod.contains) { return true }
        // İKİ NOKTAYLA BİTEN BLOK BAŞLIĞI. JS'te `for`/`if`/`while` başlığı
        // parantezle açılır, gövde süslü ayraçla gelir; iki nokta üst üste ile
        // BİTMEZ. Ölçümde görülen `for i from 0 to 19: print(...)` tam olarak
        // buraya düşer — `range(` yok, ` in ` yok, ama JS de değil.
        // `{` varsa dokunma: `for (const k in o) { ... }` geçerli JS'tir ve
        // nesne değişmezleri de iki nokta taşır.
        guard !kod.contains("{") else { return false }
        let blokBasi = ["for ", "if ", "while ", "else", "try", "except"]
        return kod.contains(":") && blokBasi.contains(where: kod.contains)
    }

    /// Hata dönüşünü tek yerde kurar: son denemede `error_final` eklenir
    /// (kod-spec §5.4 adım 3) — ama NEDEN de söylenir, çünkü modelin
    /// kullanıcıya vereceği dürüst kısa cevap nedeni içermeli.
    private static func hataMetni(_ neden: String, sonDeneme: Bool) -> String {
        let kisa = String(neden.prefix(modelHataTavani))
        guard sonDeneme else { return kisa }
        return "\(kisa)\nerror_final: give the user a short honest answer, do NOT retry"
    }

    func call(arguments: Arguments) async -> String {
        await cipliCalis(ikon: "curlybraces",
                         calisiyorMetni: Yerel.kodCalisiyor,
                         hamGirdi: arguments.kod) {
            // Sayaç MainActor'da yaşar; artışı ret kararından ÖNCE yapılır.
            let denemeNo = await MainActor.run { [durum] () -> Int in
                // Fail-closed (kod-spec §5.4 — "ret ARAÇtadır"): durum
                // bağlanmamışsa tavan yok sayılmaz, çağrı redde düşer. Sessiz
                // sınırsız çalıştırma, unutulmuş bağlamanın en kötü arızasıdır;
                // gürültülü ret entegrasyon hatasını anında görünür kılar.
                guard let durum else { return 3 }
                durum.deneme += 1
                return durum.deneme
            }
            // 3. ve sonraki çağrılar motoru hiç görmez (kod-spec §5.4):
            // düzeltemediğini döngü kurtarmaz, pencereyi yer.
            guard denemeNo <= 2 else {
                return AracSonucu(
                    cipMetni: Yerel.kodDenemeSiniri,
                    durum: .basarisiz(Yerel.kodIkiDeneme),
                    modeleDonen: "error_final: give the user a short honest answer, do NOT retry"
                )
            }

            // v1: tek motor JS (§5.1).
            let sonuc = await KodMotoru.calistir(arguments.kod)
            switch sonuc {
            case .basarili(let cikti, _) where cikti.isEmpty:
                // ÇIKTISIZ BAŞARI BİR BAŞARI DEĞİLDİR. Eskiden burada
                // "ok (0 ms)\n" dönüyordu: model "çalıştı" görüp sonucu
                // UYDURUYORDU — ölçümde `console.log` kullanan her betik tam
                // olarak buraya düşüyordu. Artık hata sayılır ve model
                // basmaya yönlendirilir (kod-spec §2.1 "iddia yok").
                return AracSonucu(
                    cipMetni: denemeNo == 1 ? Yerel.kodYenidenDeneniyor
                                            : Yerel.kodCalistirilamadi,
                    durum: .basarisiz("no output"),
                    modeleDonen: Self.hataMetni(
                        "error: the script ran but printed nothing. Add print(...) for the value you need",
                        sonDeneme: denemeNo >= 2),
                    hamCikti: "no output"
                )
            case .basarili(let cikti, let ms):
                let kisa = String(cikti.prefix(Self.modelCiktiTavani))
                return AracSonucu(
                    cipMetni: Yerel.kodCalisti(ms),
                    durum: .okundu,
                    modeleDonen: "ok (\(ms) ms)\n\(kisa)",
                    hamCikti: cikti
                )
            case .hata(let mesaj):
                // Başarısızlık gizlenmez: 1. denemede çip yeniden denemeyi
                // söyler, 2. denemede dürüstçe düşer (kod-spec §6). Modele de
                // 2. denemede `error_final` döner (kod-spec §5.4 adım 3) —
                // description'daki "call it ONCE more" modeli boşa bir 3.
                // çağrıya itmesin; 3. çağrı reddi yalnız emniyet kemeridir.
                let ilkDeneme = denemeNo == 1
                // DİL TEŞHİSİ ÖNCE GELİR. Betik Python'sa ham JS ayrıştırıcı
                // mesajı ("Unexpected identifier 'i'. Expected '('") modeli
                // yanlış onarıma sürüklüyor: parantez ekliyor, dili
                // değiştirmiyor, ikinci deneme de düşüyor ve tur kayboluyor.
                // Nedeni açıkça söylemek kalan tek denemeyi işe yarar kılar.
                let neden = Self.pythonMu(arguments.kod)
                    ? "error: this script is Python, but the sandbox runs JavaScript ONLY. "
                      + "Rewrite the SAME logic in JavaScript: for (let i = 0; i < n; i++) {...}, "
                      + "console.log(x). Do not use range(), def, or indent blocks.\n\(mesaj)"
                    : "error: \(mesaj)"
                return AracSonucu(
                    cipMetni: ilkDeneme ? Yerel.kodYenidenDeneniyor
                                        : Yerel.kodCalistirilamadi,
                    durum: .basarisiz(mesaj),
                    // Motor artık tip+mesaj+satır no+HATALI SATIRIN METNİ ve
                    // hatadan önceki çıktıyı veriyor; hepsi modele geçer
                    // (ölçüm: "SyntaxError" tek başına düzeltme için yetmiyor).
                    modeleDonen: Self.hataMetni(neden, sonDeneme: !ilkDeneme),
                    hamCikti: mesaj
                )
            case .zamanAsimi:
                // Zaman aşımı da bir hatadır: 2. denemede o da error_final'e düşer.
                return AracSonucu(
                    cipMetni: Yerel.kodZamanAsimi,
                    durum: .basarisiz(Yerel.kodZamanAsimiNeden),
                    modeleDonen: Self.hataMetni(
                        "error: the script did not finish in time — it probably has an endless loop; bound every loop",
                        sonDeneme: denemeNo >= 2),
                    hamCikti: Yerel.kodZamanAsimiNeden
                )
            case .bellekAsimi:
                // Bellek tavanı: betik durduruldu, uygulama korundu. Model
                // burada döngüyü küçültmeli — süre sorunundan AYRI söylenir.
                return AracSonucu(
                    cipMetni: Yerel.kodCalistirilamadi,
                    durum: .basarisiz("memory limit"),
                    modeleDonen: Self.hataMetni(
                        "error: the script used too much memory — do not build huge arrays or strings; compute the result incrementally",
                        sonDeneme: denemeNo >= 2),
                    hamCikti: "memory limit"
                )
            }
        }
    }
}
