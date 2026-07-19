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
//  Sözleşme dilden bağımsızdır (`dil` parametresi); v1'de tek motor JS'tir —
//  "python" istense de JS ile çözülür, dil bir uygulama ayrıntısıdır (§5.1).
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
    let description = "Runs a short script in a sandbox and returns its output. Call this for any calculation or transformation too complex for the hesapla tool (loops, dates, text processing, simulations). Write minimal code that PRINTS the final result. If the tool returns an error, fix the code and call it ONCE more."

    weak var raporlayici: (any AracRaporlayici)?
    /// Deneme sayacı — raporlayici deseniyle zayıf referans, döngü olmaz.
    weak var durum: KodDurumu?

    @Generable
    struct Arguments {
        @Guide(description: "The script. Keep it minimal; print the final result.")
        var kod: String
        @Guide(description: "js")
        var dil: String
    }

    /// Modele dönen çıktının tavanı — tam çıktı çipe gider (kod-spec §5.2).
    private static let modelCiktiTavani = 500

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

            // v1: tek motor JS — `dil` ne derse desin JSC çalışır (§5.1).
            let sonuc = await KodMotoru.calistir(arguments.kod)
            switch sonuc {
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
                return AracSonucu(
                    cipMetni: ilkDeneme ? Yerel.kodYenidenDeneniyor
                                        : Yerel.kodCalistirilamadi,
                    durum: .basarisiz(mesaj),
                    modeleDonen: ilkDeneme
                        ? "error: \(mesaj)"
                        : "error_final: give the user a short honest answer, do NOT retry",
                    hamCikti: mesaj
                )
            case .zamanAsimi:
                // Zaman aşımı da bir hatadır: 2. denemede o da error_final'e düşer.
                return AracSonucu(
                    cipMetni: Yerel.kodZamanAsimi,
                    durum: .basarisiz(Yerel.kodZamanAsimiNeden),
                    modeleDonen: denemeNo == 1
                        ? "error: timed out after 3 s"
                        : "error_final: give the user a short honest answer, do NOT retry",
                    hamCikti: Yerel.kodZamanAsimiNeden
                )
            }
        }
    }
}
