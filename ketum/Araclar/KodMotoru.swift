//
//  KodMotoru.swift
//  ketum
//
//  JavaScriptCore sandbox (kod-spec §5.3). Modelin yazdığı küçük betiği
//  çalıştırır, çıktıyı yakalar. Sandbox mutlaktır: JSContext'e HİÇBİR yerel
//  köprü verilmez (`setObject` YOK) — dosya, ağ ve cihaz verisi fiziksel
//  olarak erişilemez. `print` bile saf JS önekiyle sağlanır.
//
//  Her çağrı taze JSVirtualMachine + JSContext (sızıntı birikmez). Zaman
//  aşımı 3 sn: ayrı iş parçacığında (.utility — UI bandıyla yarışmaz)
//  değerlendirilir, süre dolunca bağlam terk edilir (JSC'de kooperatif iptal
//  yoktur — bağlam çöpe gider). Terk edilen iş parçacıkları sayılır; tavana
//  ulaşılırsa motor oturum boyu kapanır — sonsuz döngüler birikip çekirdek
//  yiyemez. Çıktı 10.000 karakterde kırpılır ve kırpıldığı söylenir; kırpma
//  JS içinde yapılır ki dev çıktı Swift tarafına hiç köprülenmesin.
//

import Foundation
import JavaScriptCore

/// Bir kod çalıştırmasının sonucu (kod-spec §5.2/§5.3).
enum KodSonucu {
    /// Betik hatasız bitti — yakalanan çıktı + geçen süre (milisaniye).
    case basarili(cikti: String, ms: Int)
    /// Betik istisna attı — ilk satır + varsa satır numarası.
    case hata(String)
    /// 3 sn doldu; bağlam terk edildi.
    case zamanAsimi
}

/// JSC sandbox motoru. Durum tutmaz — her çağrı bağımsızdır.
enum KodMotoru {
    /// Çıktı tavanı: aşan kırpılır (kod-spec §5.3).
    static let ciktiTavani = 10_000
    /// Zaman aşımı süresi (saniye).
    static let zamanAsimiSuresi: TimeInterval = 3
    /// Terk edilmiş iş parçacığı tavanı: her terk kalıcı olarak bir çekirdeği
    /// meşgul edebilir (kooperatif iptal yok); tavana ulaşılınca motor oturum
    /// boyu `engine unavailable` döner — birikim sınırlıdır.
    static let terkTavani = 3

    /// `print` saf JS ile tanımlanır — yerel köprü DEĞİLDİR. Kullanıcı
    /// kodundan önce değerlendirilir; kod bittikten sonra `__out` okunur.
    private static let onek =
        "let __out=[];function print(...a){__out.push(a.map(String).join(' '))}"

    /// Betiği sandbox'ta çalıştırır. Ayrı bir Thread'de değerlendirir ve
    /// semaforla bekler; süre dolarsa iş parçacığı (ve JSC bağlamı) terk
    /// edilir — sonuç `.zamanAsimi` olur, sessiz donma yoktur.
    static func calistir(_ kod: String) async -> KodSonucu {
        // Tavan aşıldıysa motor hiç kurulmaz: her yeni sonsuz döngü kalıcı bir
        // dönen iş parçacığı daha eklerdi (pil/ısı) — birikime izin verilmez.
        guard TerkSayaci.paylasilan.deger < terkTavani else {
            return .hata("engine unavailable")
        }
        return await withCheckedContinuation { devam in
            let semafor = DispatchSemaphore(value: 0)
            let kutu = SonucKutusu()
            let isci = Thread {
                // Terk edilmiş iş parçacığı geç de olsa biterse sayaçtan düşülür.
                if kutu.yaz(degerlendir(kod)) { TerkSayaci.paylasilan.azalt() }
                semafor.signal()
            }
            isci.name = "ketum.kodmotoru"
            // .utility: terk edilirse UI ile aynı öncelik bandında dönmesin;
            // normal betik 3 sn tavanında bu bantta da rahat biter.
            isci.qualityOfService = .utility
            isci.start()
            // Bekleme ayrı kuyrukta — çağıranın (Swift concurrency havuzu)
            // iş parçacığı 3 sn kilitlenmesin.
            DispatchQueue.global(qos: .utility).async {
                if semafor.wait(timeout: .now() + zamanAsimiSuresi) == .timedOut {
                    // Bağlam terk edilir; isci sonsuz döngüdeyse kendi
                    // kaderine bırakılır (kod-spec §5.3 — kooperatif iptal yok).
                    // Sonuç tam bu anda yazılmışsa terk sayılmaz (kilit sıralar).
                    if !kutu.terkEt() { TerkSayaci.paylasilan.artir() }
                    devam.resume(returning: .zamanAsimi)
                } else {
                    devam.resume(returning: kutu.oku() ?? .hata("no result"))
                }
            }
        }
    }

    /// Gerçek değerlendirme — isci iş parçacığında, eşzamanlı çalışır.
    private static func degerlendir(_ kod: String) -> KodSonucu {
        let vm = JSVirtualMachine()
        guard let baglam = JSContext(virtualMachine: vm) else {
            return .hata("engine unavailable")
        }
        // İstisna yakalama: ilk hata + satır numarası saklanır.
        var hata: String?
        baglam.exceptionHandler = { _, istisna in
            guard hata == nil else { return }
            let mesaj = ilkSatir(istisna?.toString() ?? "unknown error")
            let satir = istisna?.objectForKeyedSubscript("line")?.toInt32() ?? 0
            hata = satir > 0 ? "\(mesaj) (line \(satir))" : mesaj
        }

        // Önek (saf JS print) → kullanıcı kodu → __out okuması.
        baglam.evaluateScript(onek)
        let baslangic = DispatchTime.now()
        let sonDeger = baglam.evaluateScript(kod)
        let ms = Int((DispatchTime.now().uptimeNanoseconds &- baslangic.uptimeNanoseconds) / 1_000_000)

        if let hata { return .hata(hata) }

        // Kırpma JS İÇİNDE başlar: köprüye (JSValue→Swift String) tavan üstü
        // veri hiç sokulmaz — `print('x'.repeat(5e8))` aksi halde kırpmadan
        // ÖNCE yüzlerce MB'ı Swift tarafına kopyalardı (jetsam/OOM). Tavan+1
        // alınır ki aşağıdaki Swift kırpması aşımı görüp notu ekleyebilsin.
        var cikti = baglam.evaluateScript(
            "__out.join('\\n').slice(0, \(ciktiTavani + 1))")?.toString() ?? ""
        if hata != nil { cikti = "" } // __out okuması bile düştüyse çıktıya güvenilmez
        // print hiç çağrılmadıysa son ifadenin değeri çıktı sayılır
        // (OtoTest: `print(6*7)` → "42"; `6*7` de "42" vermeli).
        if cikti.isEmpty, let sonDeger, !sonDeger.isUndefined, !sonDeger.isNull {
            // Son değer de köprülenmeden önce JS tarafında sınırlanır.
            let kisalt = baglam.evaluateScript(
                "(function(v){try{return String(v).slice(0,\(ciktiTavani + 1))}catch(e){return ''}})")
            cikti = kisalt?.call(withArguments: [sonDeger])?.toString() ?? ""
        }
        if cikti.count > ciktiTavani {
            cikti = String(cikti.prefix(ciktiTavani))
                + "\n" + Yerel.kodCiktiKirpildi
        }
        return .basarili(cikti: cikti, ms: ms)
    }

    /// Hata metninin yalnızca ilk satırı modele döner (kod-spec §5.2).
    private static func ilkSatir(_ metin: String) -> String {
        metin.split(separator: "\n", maxSplits: 1,
                    omittingEmptySubsequences: false).first.map(String.init) ?? metin
    }
}

/// İki iş parçacığı arasında sonuç taşıyan kilitli kutu. Zaman aşımında
/// isci sonucu geç yazabilir — kilit sayesinde bu yazma zararsızdır ve
/// terk/bitiş yarışını tek sıraya dizer: ikinci gelen, ilkini görür.
private final class SonucKutusu: @unchecked Sendable {
    private let kilit = NSLock()
    private var sonuc: KodSonucu?
    private var terkEdildi = false

    /// Sonucu yazar; iş parçacığı bu andan önce terk edildiyse true döner
    /// (terk sayacından düşülmesi gerekir — artık dönmüyor).
    func yaz(_ s: KodSonucu) -> Bool {
        kilit.lock(); defer { kilit.unlock() }
        sonuc = s
        return terkEdildi
    }

    /// Terk işareti koyar; sonuç bu andan önce yazıldıysa true döner
    /// (iş parçacığı zaten bitmiştir, terk SAYILMAZ).
    func terkEt() -> Bool {
        kilit.lock(); defer { kilit.unlock() }
        terkEdildi = true
        return sonuc != nil
    }

    func oku() -> KodSonucu? {
        kilit.lock(); defer { kilit.unlock() }
        return sonuc
    }
}

/// Terk edilen iş parçacığı sayacı — süreç genelinde tekil, kilitli.
/// Artış zaman aşımı anında, düşüş terk edilmiş işin geç bitişinde.
private final class TerkSayaci: @unchecked Sendable {
    static let paylasilan = TerkSayaci()
    private let kilit = NSLock()
    private var sayi = 0

    var deger: Int {
        kilit.lock(); defer { kilit.unlock() }
        return sayi
    }
    func artir() { kilit.lock(); sayi += 1; kilit.unlock() }
    func azalt() { kilit.lock(); sayi -= 1; kilit.unlock() }
}
