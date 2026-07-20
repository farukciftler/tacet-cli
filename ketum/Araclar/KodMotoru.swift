//
//  KodMotoru.swift
//  ketum
//
//  JavaScriptCore sandbox (kod-spec §5.3). Modelin yazdığı küçük betiği
//  çalıştırır, çıktıyı yakalar. Sandbox mutlaktır: JSContext'e dosya, ağ ya da
//  cihaz verisi taşıyan HİÇBİR köprü verilmez. `print`/`console` saf JS
//  önekiyle sağlanır; tek yerel köprü `__g` bekçisidir ve yalnızca "süre/bellek
//  doldu mu" sorusuna Bool döner (§ Bekçi notu).
//
//  ÖLÇÜLMÜŞ GEREKÇELER (bu dosyadaki her mekanizma bir ölçümün cevabıdır):
//
//  1. JSC bağlamı KENDİ `console` nesnesiyle gelir (log/warn/error/…) ve o
//     nesne sistem günlüğüne yazar. Yakalanmadığı sürece `console.log('x')`
//     hatasız çalışır ama çıktı BOŞ döner — model "ok" görüp sonucu uydurur;
//     üstelik kullanıcı verisi cihaz günlüğüne sızar. Bu yüzden `console`
//     önekte saf JS ile EZİLİR ve `print`e bağlanır.
//  2. `String(nesne)` → "[object Object]". Modelin bastığı nesne/dizi işe
//     yaramaz hale geliyordu; `__s` JSON'a çevirir.
//  3. Sonsuz döngü: JSC'de kooperatif iptal YOKTUR, dolayısıyla dış gözcü
//     iş parçacığı terk etmekten başka bir şey yapamaz — terk edilen iş
//     parçacığı bir çekirdeği SONSUZA DEK yer. Çözüm: döngü koşullarına saf
//     JS sayaç `__c()` enjekte edilir, sayaç 256 turda bir yerel bekçiyi
//     yoklar ve süre/bellek dolduysa JS istisnası atar. Böylece iptal GERÇEK
//     olur; iş parçacığı biter.
//  4. Bellek: `while(true){a.push(new Array(1e5))}` ölçümde 3 sn'lik dış
//     zaman aşımı dolmadan ~2 GB yerleşik / ~12 GB tepe ayak izine ulaştı —
//     iOS'ta bu jetsam, yani UYGULAMANIN ÖLÜMÜ demektir. Dış gözcü buna
//     yetişemez. Bekçi bu yüzden süreyle birlikte ayak izi ARTIŞINI da
//     ölçer ve tavanı aşınca betiği durdurur.
//
//  Her çağrı taze JSVirtualMachine + JSContext (sızıntı birikmez). Dış zaman
//  aşımı 3 sn korunur: bekçi ezilmiş ya da enjeksiyon atlanmışsa son emniyet
//  kemeridir. Çıktı 10.000 karakterde kırpılır; kırpma JS İÇİNDE yapılır ki
//  dev çıktı Swift tarafına hiç köprülenmesin.
//

import Foundation
import JavaScriptCore

/// Bir kod çalıştırmasının sonucu (kod-spec §5.2/§5.3).
enum KodSonucu {
    /// Betik hatasız bitti — yakalanan çıktı + geçen süre (milisaniye).
    /// `cikti` BOŞ ise betik hiçbir şey basmamıştır: çağıran bunu başarı
    /// gibi sunmamalıdır (modelin uydurma yaptığı asıl delik burasıdır).
    case basarili(cikti: String, ms: Int)
    /// Betik istisna attı — tip + mesaj + satır + hatalı satırın METNİ.
    case hata(String)
    /// Süre doldu; betik bekçi tarafından durduruldu ya da bağlam terk edildi.
    case zamanAsimi
    /// Bellek tavanı aşıldı; betik durduruldu (uygulama korundu).
    case bellekAsimi
}

/// JSC sandbox motoru. Durum tutmaz — her çağrı bağımsızdır.
enum KodMotoru {
    /// Çıktı tavanı: aşan kırpılır (kod-spec §5.3).
    static let ciktiTavani = 10_000
    /// Dış zaman aşımı (saniye) — bekçi çalışmazsa devreye giren emniyet kemeri.
    static let zamanAsimiSuresi: TimeInterval = 3
    /// Bekçinin uyguladığı iç süre tavanı. Dış tavandan KISA olmalı ki
    /// kooperatif durdurma kazansın (iş parçacığı gerçekten bitsin).
    /// 2,7 sn ölçüyle seçildi: OtoTest sonsuz döngünün ≥2,5 sn sürmesini
    /// bekliyor (2,5 koysaydık pay 1,5 ms kalırdı), dış 3 sn'ye de 300 ms
    /// kalıyor — geri sarma ölçümde ~1 ms sürüyor, kooperatif yol hep kazanır.
    static let bekciSuresi: TimeInterval = 2.7
    /// Betiğin ekleyebileceği ayak izi tavanı (bayt). Aşılırsa durdurulur.
    /// 256 MB: ağır ama meşru bir hesabın üstünde, jetsam eşiğinin altında.
    static let bellekTavani: UInt64 = 256 << 20
    /// Terk edilmiş iş parçacığı tavanı. Bekçi sayesinde bu yola pratikte
    /// düşülmez; ezilmiş bekçi senaryosunda birikim yine de sınırlıdır.
    static let terkTavani = 3

    /// Bekçinin attığı istisnaların imzası — hata metninden AYIRT edilir.
    private static let sureImzasi = "__ketum_sure"
    private static let bellekImzasi = "__ketum_bellek"

    /// Saf JS önek. Yerel köprü DEĞİLDİR (tek istisna `__g`, aşağıda).
    /// Kullanıcı kodundan ÖNCE ve AYRI bir betik olarak değerlendirilir —
    /// böylece kullanıcı kodunun satır numaraları 1'den başlar.
    private static var onek: String {
        """
        var __out=[],__len=0,__lim=\(ciktiTavani),__kirp=false,__i=0;
        function __s(v){
          var t=typeof v;
          if(t==='string')return v;
          if(v===null)return 'null';
          if(v===undefined)return 'undefined';
          if(t==='number'||t==='boolean'||t==='bigint'||t==='symbol')return String(v);
          if(t==='function')return '[Function'+(v.name?': '+v.name:'')+']';
          if(v instanceof Error)return v.name+': '+v.message;
          if(v instanceof Date)return isNaN(v)?'Invalid Date':v.toISOString();
          try{var j=JSON.stringify(v);return j===undefined?String(v):j}catch(e){return String(v)}
        }
        function __yaz(a){
          if(__len>=__lim){__kirp=true;return}
          var s='';
          for(var i=0;i<a.length;i++){if(i)s+=' ';s+=__s(a[i])}
          if(s.length>__lim-__len){s=s.slice(0,__lim-__len);__kirp=true}
          __len+=s.length+1;__out.push(s)
        }
        function print(){__yaz(arguments)}
        (function(){
          var n=function(){};
          globalThis.console={log:print,info:print,warn:print,error:print,debug:print,
            trace:print,dir:print,dirxml:print,table:print,group:print,groupCollapsed:print,
            groupEnd:n,time:n,timeEnd:n,timeLog:n,count:n,countReset:n,clear:n,
            assert:function(c){if(!c){var a=[].slice.call(arguments,1);a.unshift('Assertion failed:');__yaz(a)}}};
        })();
        """
    }

    /// Betiği sandbox'ta çalıştırır. Ayrı bir Thread'de değerlendirir ve
    /// semaforla bekler; bekçi süreyi/belleği kendi içinden uygular, dış
    /// gözcü yalnız emniyet kemeridir.
    static func calistir(_ kod: String) async -> KodSonucu {
        guard TerkSayaci.paylasilan.deger < terkTavani else {
            return .hata("engine unavailable: too many runs had to be abandoned this session")
        }
        return await withCheckedContinuation { devam in
            let semafor = DispatchSemaphore(value: 0)
            let kutu = SonucKutusu()
            let isci = Thread {
                if kutu.yaz(degerlendir(kod)) { TerkSayaci.paylasilan.azalt() }
                semafor.signal()
            }
            isci.name = "ketum.kodmotoru"
            // .utility: terk edilirse UI ile aynı öncelik bandında dönmesin.
            isci.qualityOfService = .utility
            isci.start()
            DispatchQueue.global(qos: .utility).async {
                if semafor.wait(timeout: .now() + zamanAsimiSuresi) == .timedOut {
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

        // MARK: Bekçi — tek yerel köprü.
        // Sandbox gerekçesi: bu blok DOSYA, AĞ ve CİHAZ VERİSİ görmez; girdi
        // almaz, veri döndürmez. Yaptığı tek iş, süre/bellek tavanı aşıldıysa
        // JS istisnası atmaktır. `Date.now()`un zaten verdiğinden fazla hiçbir
        // yetenek eklemez — sandbox yüzeyi genişlemez, DARALIR (kaçış yolu
        // olan sonsuz döngü kapanır).
        let bitis = DispatchTime.now().uptimeNanoseconds
            &+ UInt64(bekciSuresi * 1_000_000_000)
        let baslangicAyakIzi = ayakIzi()
        let bekci: @convention(block) () -> Bool = {
            guard let aktif = JSContext.current() else { return true }
            if DispatchTime.now().uptimeNanoseconds > bitis {
                aktif.exception = JSValue(newErrorFromMessage: sureImzasi, in: aktif)
                return true
            }
            let simdi = ayakIzi()
            if simdi > baslangicAyakIzi, simdi &- baslangicAyakIzi > bellekTavani {
                aktif.exception = JSValue(newErrorFromMessage: bellekImzasi, in: aktif)
            }
            return true
        }
        baglam.setObject(bekci, forKeyedSubscript: "__g" as NSString)

        // İstisna yakalama: ilk hata + satır/sütun saklanır.
        var hataMesaji: String?
        var hataSatiri = 0
        baglam.exceptionHandler = { _, istisna in
            guard hataMesaji == nil else { return }
            hataMesaji = ilkSatir(istisna?.toString() ?? "unknown error")
            hataSatiri = Int(istisna?.objectForKeyedSubscript("line")?.toInt32() ?? 0)
        }

        baglam.evaluateScript(onek)
        // Bekçi çağrıları döngü koşullarına enjekte edilir. Enjeksiyon
        // güvenli yapılamıyorsa (belirsiz sözdizimi) kod OLDUĞU GİBİ
        // çalışır — dış zaman aşımı yine korur.
        let calisan = BekciEnjeksiyonu.uygula(kod)
        let baslangic = DispatchTime.now()
        var sonDeger = baglam.evaluateScript(calisan)

        // ÜST DÜZEY `return` KURTARMASI (ölçüm: kod kategorisinin en sık hatası).
        // Küçük model betiği sık sık bir fonksiyon gövdesi gibi yazıp sonucu
        // `return` ile veriyor; global kapsamda bu SyntaxError'dır ve tur
        // "yapamadım"a düşüyordu. Kod bir IIFE'ye sarılıp BİR KEZ daha denenir.
        //
        // Neden koşulsuz sarmalamıyoruz: sarmalama son-ifade değerini yutar
        // (`6*7` → undefined), oysa print'siz betiğin son değeri bilerek çıktı
        // sayılıyor. Bu yüzden yalnız BU sözdizimi hatasında devreye girer;
        // başarılı ve diğer hatalı yollar bit düzeyinde aynı kalır.
        if let ham = hataMesaji, Self.ustDuzeyReturnMu(ham) {
            hataMesaji = nil
            hataSatiri = 0
            // Sözdizimi hatası hiçbir şey çalıştırmadığı için yan etki yok;
            // yine de çıktı tamponu temizlenir ki kısmi çıktı sızmasın.
            baglam.evaluateScript("__out.length=0;__kirp=false")
            sonDeger = baglam.evaluateScript("(function(){\n\(calisan)\n})()")
        }
        let ms = Int((DispatchTime.now().uptimeNanoseconds &- baslangic.uptimeNanoseconds) / 1_000_000)

        // Çıktı, hata olsa DA okunur: modele "nereye kadar geldiğini"
        // göstermek ikinci denemenin isabetini belirgin biçimde artırır.
        var cikti = ciktiyiTopla(baglam)

        if let ham = hataMesaji {
            if ham.contains(sureImzasi) { return .zamanAsimi }
            if ham.contains(bellekImzasi) { return .bellekAsimi }
            return .hata(hataRaporu(ham, satir: hataSatiri, kaynak: kod, kismiCikti: cikti))
        }

        // print hiç çağrılmadıysa son ifadenin değeri çıktı sayılır
        // (OtoTest: `print(6*7)` → "42"; `6*7` de "42" vermeli).
        if cikti.isEmpty, let sonDeger, !sonDeger.isUndefined, !sonDeger.isNull {
            let kisalt = baglam.evaluateScript(
                "(function(v){try{return __s(v).slice(0,\(ciktiTavani + 1))}catch(e){return ''}})")
            cikti = kisalt?.call(withArguments: [sonDeger])?.toString() ?? ""
            if cikti.count > ciktiTavani {
                cikti = String(cikti.prefix(ciktiTavani)) + "\n" + Yerel.kodCiktiKirpildi
            }
        }
        return .basarili(cikti: cikti, ms: ms)
    }

    /// Hata metni "global kapsamda return" sözdizimi hatası mı?
    ///
    /// JSC bunu "Return statements are only valid inside functions." diye
    /// bildirir. Eşleşme DAR tutulur: yalnız `return` + `function` geçen
    /// sözdizimi hataları sarmalamayı tetikler, yoksa gerçek bir kullanıcı
    /// hatası sessizce ikinci kez çalıştırılmış olurdu.
    private static func ustDuzeyReturnMu(_ ham: String) -> Bool {
        let h = ham.lowercased()
        return h.contains("return") && h.contains("function")
    }

    /// `__out` içeriğini köprüler. Kırpma JS İÇİNDE yapılır: tavan üstü veri
    /// Swift tarafına hiç kopyalanmaz (aksi halde `print('x'.repeat(5e8))`
    /// kırpmadan ÖNCE yüzlerce MB'ı köprüden geçirirdi — jetsam/OOM).
    private static func ciktiyiTopla(_ baglam: JSContext) -> String {
        let ham = baglam.evaluateScript(
            "(function(){try{return __out.join('\\n').slice(0,\(ciktiTavani + 1))}catch(e){return ''}})()")
        var cikti = ham?.toString() ?? ""
        // JSValue nil/undefined ise toString() "undefined" verir — çıktı sayılmaz.
        if cikti == "undefined" { cikti = "" }
        let kirpildi = baglam.evaluateScript("__kirp===true")?.toBool() ?? false
        if cikti.count > ciktiTavani {
            cikti = String(cikti.prefix(ciktiTavani))
        }
        if kirpildi, !cikti.isEmpty {
            cikti += "\n" + Yerel.kodCiktiKirpildi
        }
        return cikti
    }

    /// Modele dönen hata raporu. ÖLÇÜLEN DERS: "SyntaxError" tek başına 3B
    /// modele hiçbir şey söylemiyor; hatalı satırın METNİ verilince düzeltme
    /// hedefe kilitleniyor. Rapor üç parçadır: tip+mesaj, satır no + o satırın
    /// kendisi, ve hatadan önce basılmış çıktı.
    private static func hataRaporu(_ mesaj: String, satir: Int,
                                   kaynak: String, kismiCikti: String) -> String {
        var parcalar = [mesaj]
        let satirlar = kaynak.components(separatedBy: "\n")
        if satir > 0, satir <= satirlar.count {
            let metin = satirlar[satir - 1].trimmingCharacters(in: .whitespaces)
            if metin.isEmpty {
                parcalar.append("at line \(satir)")
            } else {
                parcalar.append("at line \(satir): \(String(metin.prefix(160)))")
            }
        }
        if !kismiCikti.isEmpty {
            parcalar.append("output before the error:\n\(String(kismiCikti.prefix(300)))")
        }
        return parcalar.joined(separator: "\n")
    }

    /// Süreç ayak izi (jetsam'in baktığı ölçü). Okunamazsa 0 döner ve bellek
    /// bekçisi sessizce devre dışı kalır — süre bekçisi çalışmaya devam eder.
    private static func ayakIzi() -> UInt64 {
        var bilgi = task_vm_info_data_t()
        var say = mach_msg_type_number_t(
            MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<integer_t>.size)
        let sonuc = withUnsafeMutablePointer(to: &bilgi) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(say)) {
                task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &say)
            }
        }
        return sonuc == KERN_SUCCESS ? UInt64(bilgi.phys_footprint) : 0
    }

    /// Hata metninin yalnızca ilk satırı modele döner (kod-spec §5.2).
    private static func ilkSatir(_ metin: String) -> String {
        metin.split(separator: "\n", maxSplits: 1,
                    omittingEmptySubsequences: false).first.map(String.init) ?? metin
    }
}

// MARK: - Bekçi enjeksiyonu

/// Döngü koşullarına saf JS bekçi çağrısı (`__c()`) yerleştirir.
///
/// NEDEN GEREKLİ: JSC'de çalışan bir betiği dışarıdan durdurmanın herkese
/// açık bir yolu yoktur (`JSContextGroupSetExecutionTimeLimit` özel API'dir
/// ve App Store riski taşır — bilinçle KULLANILMADI). Sonsuz döngü, dış
/// gözcü zaman aşımı bildirse bile bir çekirdeği sonsuza dek yakar.
/// Koşula enjekte edilen `__c()` her turda çalışır ve iptali GERÇEK yapar.
///
/// NEDEN GÖVDEYE DEĞİL KOŞULA: gövde enjeksiyonu süslü parantezsiz döngüleri
/// (`while(x) y();`) bozar ve satır numaralarını kaydırır. Koşula virgül/`&&`
/// ile girmek her zaman geçerli sözdizimidir ve TEK SATIRDA kalır — hata
/// satır numaraları bozulmaz.
///
/// GÜVENLİK: dizge, şablon dizgesi, düzenli ifade ve yorum içindeki
/// `for`/`while` sözcükleri lexer ile ELENİR. Lexer tuhaf bir durumda
/// biterse (kapanmamış dizge/yorum) enjeksiyon tamamen atlanır — çalışan
/// kodu bozmaktansa dış zaman aşımına güvenilir.
enum BekciEnjeksiyonu {
    /// Her turda çalışan sayaç. ÖLÇÜLDÜ: bunu bir JS fonksiyonuna (`__c()`)
    /// sarmak 3e7 turluk sıkı döngüyü 222 ms → 886 ms yapıyordu; satır içi
    /// ifade 556 ms'de kalıyor (%35 daha ucuz). Yerel bekçi 256 turda bir
    /// yoklanır — köprü geçişi sıcak yolda değildir.
    private static let sayac = "((++__i&255)||__g())"

    static func uygula(_ kod: String) -> String {
        let k = Array(kod)
        guard let maske = kodMaskesi(k) else { return kod }

        // Enjeksiyon noktaları sondan başa uygulanır ki indeksler kaymasın.
        var eklemeler: [(indeks: Int, metin: String)] = []
        var i = 0
        while i < k.count {
            guard maske[i], let uzunluk = anahtarSozcuk(k, i, maske) else { i += 1; continue }
            // Anahtar sözcükten sonraki ilk '(' bulunur.
            var j = i + uzunluk
            while j < k.count, maske[j], k[j] == " " || k[j] == "\n" || k[j] == "\t" || k[j] == "\r" { j += 1 }
            guard j < k.count, maske[j], k[j] == "(" else { i += uzunluk; continue }
            guard let kapanis = eslesenParantez(k, j, maske) else { i += uzunluk; continue }

            let sozcuk = String(k[i..<(i + uzunluk)])
            if sozcuk == "while" {
                // while (KOŞUL) → while (SAYAÇ, (KOŞUL))
                eklemeler.append((j + 1, "\(sayac),("))
                eklemeler.append((kapanis, ")"))
            } else {
                // for: yalnızca klasik üç parçalı biçim. for-of/for-in'de
                // koşul yoktur; onlara DOKUNULMAZ (sonlu yineleyiciler).
                guard let noktaliVirguller = ustDuzeyNoktaliVirgul(k, j, kapanis, maske),
                      noktaliVirguller.count == 2 else { i += uzunluk; continue }
                let kosulBas = noktaliVirguller[0] + 1
                let kosulSon = noktaliVirguller[1]
                let bos = (kosulBas..<kosulSon).allSatisfy { k[$0] == " " || k[$0] == "\t" }
                if bos {
                    // for(;;) → for(;SAYAÇ,true;) — koşul yerine doğru gerekir.
                    eklemeler.append((kosulBas, "\(sayac),true"))
                } else {
                    eklemeler.append((kosulBas, "\(sayac),("))
                    eklemeler.append((kosulSon, ")"))
                }
            }
            i = kapanis + 1
        }
        guard !eklemeler.isEmpty else { return kod }

        var sonuc = k
        for ekleme in eklemeler.sorted(by: { $0.indeks > $1.indeks }) {
            sonuc.insert(contentsOf: Array(ekleme.metin), at: ekleme.indeks)
        }
        return String(sonuc)
    }

    /// `i` konumunda gerçek bir `while`/`for` anahtar sözcüğü varsa uzunluğunu
    /// döner. Önündeki karakter tanımlayıcı parçasıysa (`.for`, `xfor`) elenir.
    private static func anahtarSozcuk(_ k: [Character], _ i: Int, _ maske: [Bool]) -> Int? {
        for sozcuk in ["while", "for"] {
            let n = sozcuk.count
            guard i + n <= k.count, String(k[i..<(i + n)]) == sozcuk else { continue }
            if i > 0, tanimlayiciParcasi(k[i - 1]) || k[i - 1] == "." { return nil }
            if i + n < k.count, tanimlayiciParcasi(k[i + n]) { return nil }
            return n
        }
        return nil
    }

    private static func tanimlayiciParcasi(_ c: Character) -> Bool {
        c.isLetter || c.isNumber || c == "_" || c == "$"
    }

    /// `acilis` konumundaki '(' ile eşleşen ')' indeksini döner.
    private static func eslesenParantez(_ k: [Character], _ acilis: Int, _ maske: [Bool]) -> Int? {
        var derinlik = 0
        var i = acilis
        while i < k.count {
            if maske[i] {
                if k[i] == "(" { derinlik += 1 }
                else if k[i] == ")" {
                    derinlik -= 1
                    if derinlik == 0 { return i }
                }
            }
            i += 1
        }
        return nil
    }

    /// `for(...)` içindeki ÜST DÜZEY (parantez/köşeli/süslü içinde olmayan)
    /// noktalı virgül indeksleri.
    private static func ustDuzeyNoktaliVirgul(_ k: [Character], _ acilis: Int,
                                              _ kapanis: Int, _ maske: [Bool]) -> [Int]? {
        var derinlik = 0
        var bulunan: [Int] = []
        for i in (acilis + 1)..<kapanis where maske[i] {
            switch k[i] {
            case "(", "[", "{": derinlik += 1
            case ")", "]", "}": derinlik -= 1
            case ";" where derinlik == 0:
                bulunan.append(i)
                if bulunan.count > 2 { return nil }
            default: break
            }
        }
        return bulunan
    }

    /// Her karakter için "bu KOD mu?" maskesi. Dizge/şablon/yorum/regex
    /// içindekiler `false` olur. Lexer beklenmedik durumda biterse `nil`
    /// döner ve çağıran enjeksiyonu tamamen atlar.
    private static func kodMaskesi(_ k: [Character]) -> [Bool]? {
        var maske = [Bool](repeating: true, count: k.count)
        // Şablon dizgesi `${}` iç içe geçebilir; derinlik yığınla izlenir.
        var sablonYigini: [Int] = []
        var suslulukDerinligi = 0
        var i = 0
        // Regex/bölme ayrımı için son anlamlı kod karakteri.
        var sonAnlamli: Character?

        while i < k.count {
            let c = k[i]
            if c == "'" || c == "\"" {
                let tirnak = c
                maske[i] = false
                i += 1
                while i < k.count {
                    maske[i] = false
                    if k[i] == "\\" { if i + 1 < k.count { maske[i + 1] = false }; i += 2; continue }
                    if k[i] == tirnak { i += 1; break }
                    if k[i] == "\n" { return nil } // kapanmamış dizge
                    i += 1
                }
                sonAnlamli = "\""
                continue
            }
            if c == "`" {
                maske[i] = false
                i += 1
                var kapandi = false
                while i < k.count {
                    if k[i] == "\\" { maske[i] = false; if i + 1 < k.count { maske[i + 1] = false }; i += 2; continue }
                    if k[i] == "`" { maske[i] = false; i += 1; kapandi = true; break }
                    if k[i] == "$", i + 1 < k.count, k[i + 1] == "{" {
                        // `${` içi KOD'dur; süslü kapanana kadar normal lexing.
                        maske[i] = false; maske[i + 1] = true
                        sablonYigini.append(suslulukDerinligi)
                        suslulukDerinligi += 1
                        i += 2
                        // İç kod normal döngüde işlensin diye dışarı çıkılır.
                        break
                    }
                    maske[i] = false
                    i += 1
                }
                if kapandi || i >= k.count {
                    if !kapandi { return nil }
                    sonAnlamli = "\""
                }
                continue
            }
            if c == "/", i + 1 < k.count, k[i + 1] == "/" {
                while i < k.count, k[i] != "\n" { maske[i] = false; i += 1 }
                continue
            }
            if c == "/", i + 1 < k.count, k[i + 1] == "*" {
                maske[i] = false; maske[i + 1] = false
                i += 2
                var kapandi = false
                while i + 1 < k.count {
                    if k[i] == "*", k[i + 1] == "/" { maske[i] = false; maske[i + 1] = false; i += 2; kapandi = true; break }
                    maske[i] = false; i += 1
                }
                if !kapandi { return nil }
                continue
            }
            if c == "/", regexBaslangici(sonAnlamli) {
                maske[i] = false
                i += 1
                while i < k.count {
                    maske[i] = false
                    if k[i] == "\\" { if i + 1 < k.count { maske[i + 1] = false }; i += 2; continue }
                    if k[i] == "[" { // karakter sınıfı: '/' kaçmadan geçebilir
                        while i < k.count, k[i] != "]" {
                            maske[i] = false
                            if k[i] == "\\" { if i + 1 < k.count { maske[i + 1] = false }; i += 1 }
                            i += 1
                        }
                        if i < k.count { maske[i] = false }
                        i += 1
                        continue
                    }
                    if k[i] == "/" { i += 1; break }
                    if k[i] == "\n" { return nil }
                    i += 1
                }
                while i < k.count, k[i].isLetter { maske[i] = false; i += 1 } // bayraklar
                sonAnlamli = ")"
                continue
            }
            if c == "{" { suslulukDerinligi += 1 }
            if c == "}" {
                suslulukDerinligi -= 1
                if let beklenen = sablonYigini.last, suslulukDerinligi == beklenen {
                    // `${...}` kapandı; şablon dizgesinin kalanına dönülür.
                    sablonYigini.removeLast()
                    maske[i] = false
                    i += 1
                    var kapandi = false
                    while i < k.count {
                        if k[i] == "\\" { maske[i] = false; if i + 1 < k.count { maske[i + 1] = false }; i += 2; continue }
                        if k[i] == "`" { maske[i] = false; i += 1; kapandi = true; break }
                        if k[i] == "$", i + 1 < k.count, k[i + 1] == "{" {
                            maske[i] = false; maske[i + 1] = true
                            sablonYigini.append(suslulukDerinligi)
                            suslulukDerinligi += 1
                            i += 2
                            kapandi = true
                            break
                        }
                        maske[i] = false
                        i += 1
                    }
                    if !kapandi { return nil }
                    sonAnlamli = "\""
                    continue
                }
            }
            if !c.isWhitespace { sonAnlamli = c }
            i += 1
        }
        guard sablonYigini.isEmpty else { return nil }
        return maske
    }

    /// `/` bir düzenli ifade mi yoksa bölme mi? Standart sezgisel: önündeki
    /// anlamlı karakter bir DEĞER sonuysa bölmedir, değilse regex'tir.
    private static func regexBaslangici(_ sonAnlamli: Character?) -> Bool {
        guard let s = sonAnlamli else { return true }
        if s.isLetter || s.isNumber || s == "_" || s == "$" { return false }
        if s == ")" || s == "]" { return false }
        return true
    }
}

// MARK: - İş parçacığı arası taşıma

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
