//
//  BeceriDeposu.swift
//  ketum
//
//  Beceri (skill) katmanı — Claude'un SKILL.md mantığı. Her aracın ayrıntılı
//  kullanım kılavuzu `Beceriler/*.md` içinde (frontmatter + gövde); kullanıcının
//  kendi yazdıkları SwiftData'da (KullaniciBecerisi). 4096 token penceresini
//  şişirmemek için "progressive disclosure": hepsi birden değil, yalnızca o anki
//  niyete uyan TEK beceri, o oturuma BİR KEZ enjekte edilir.
//

import Foundation

/// Bir beceri: ad, tetikleyici anahtar kelimeler ve kılavuz metni.
struct Beceri {
    let ad: String
    let tetikler: [String]
    let metin: String
    /// Bu kılavuzun EMRETTİĞİ araçların adları (frontmatter `araclar:`).
    /// Boşsa beceri araçtan bağımsızdır ve her profilde serbesttir.
    var araclar: [String] = []
    /// Kullanıcının kendi yazdığı mı — eşitlikte kullanıcınınki kazanır.
    var kullanicininMi: Bool = false
}

enum BeceriDeposu {
    /// Enjeksiyonda tek beceriden alınacak en fazla karakter. Paket becerileri
    /// insan referansı olarak daha uzun olabilir; modele giden kısım sınırlıdır.
    static let enjeksiyonSiniri = 700

    /// Gövdede ÇEKİRDEĞİN bittiği yeri işaretleyen HTML yorumu. Markdown'da
    /// görünmez, bu yüzden dosya insan için de okunur kalır.
    ///
    /// Neden var: eski kesme gövdenin SONUNU atıyordu, ama somut `tool(args)`
    /// örneği ile anti-halüsinasyon kuralları tam orada duruyordu — yani sınır
    /// tam da becerinin var oluş sebebini yutuyordu (kod.md'de 327 karakter,
    /// belge-olustur.md'de 729 karakter). Şimdi dosyalar "çekirdek-önce"
    /// yazılıyor: örnek + kırılmaz kurallar işaretin ÜSTÜNDE, insan referansı
    /// altında. Enjeksiyon çekirdeği TAM alır, artan bütçeye kuyruğu doldurur.
    static let cekirdekIsareti = "<!--/cekirdek-->"

    /// Çekirdekten sonra kuyruktan parça almaya değer en küçük kalan bütçe.
    /// Bunun altında tek satır bile anlamlı girmiyor; yarım kural eklemektense
    /// hiç eklememek yeğdir.
    private static let kuyrukEsigi = 80

    /// Bundle'daki .md becerileri (bir kez yüklenir, salt-okunur).
    static let paket: [Beceri] = yukle()

    /// Kullanıcının eklediği beceriler — UI kaydettikçe `kullaniciyiYenile` ile tazelenir.
    private(set) static var kullanici: [Beceri] = []

    /// Paket + kullanıcı; eşleşmede kullanıcınınki önce denenir.
    static var hepsi: [Beceri] { kullanici + paket }

    /// SwiftData'daki kullanıcı becerilerini depoya yansıtır (yalnızca aktif olanlar).
    static func kullaniciyiYenile(_ modeller: [KullaniciBecerisi]) {
        kullanici = modeller.compactMap { m in
            guard m.aktif, m.gecerliMi else { return nil }
            return Beceri(ad: m.ad, tetikler: m.tetikler, metin: m.govde, kullanicininMi: true)
        }
    }

    /// Ada göre beceri döndürür.
    static func beceri(ad: String) -> Beceri? {
        hepsi.first { $0.ad == ad }
    }

    /// Verilen adların becerilerini tek metinde birleştirir.
    static func birlestir(_ adlar: [String]) -> String {
        adlar.compactMap { beceri(ad: $0) }
            .map { "## \($0.ad)\n\($0.metin)" }
            .joined(separator: "\n\n")
    }

    /// Verilen mesaja en iyi uyan beceriyi döndürür (yoksa nil).
    ///
    /// Puan, eşleşen tetikleyicilerin UZUNLUKLARI toplamıdır — adet değil. Böylece
    /// özgül ifade genel kelimeyi yener: "bunu tablo olarak göster" cümlesinde
    /// belge-oku'nun "tablo olarak"ı, belge-olustur'un "tablo"sunu geçer. Adet
    /// sayılsaydı ikisi de 1 alır, sıra rastgele belirlerdi.
    /// Eşit puanda `hepsi` sırası gereği kullanıcının becerisi kazanır.
    ///
    /// `mevcutAraclar` verilirse (aktif profilin araç setindeki `tool.name`
    /// listesi) kılavuzun EMRETTİĞİ aracı bulundurmayan beceriler elenir —
    /// tek doğruluk kaynağı araç setidir, elle tutulan bir profil haritası
    /// değil. nil geçilirse eleme yapılmaz (test/önizleme yolu).
    static func eslesen(_ soru: String, mevcutAraclar: Set<String>? = nil) -> Beceri? {
        let s = soru.lowercased()
        var enIyi: (beceri: Beceri, skor: Int)?
        for b in hepsi {
            guard aracVarMi(b, mevcutAraclar) else { continue }
            let skor = b.tetikler.reduce(0) { $0 + (icerir(s, $1) ? $1.count : 0) }
            if skor > 0, skor > (enIyi?.skor ?? 0) {
                enIyi = (b, skor)
            }
        }
        return enIyi?.beceri
    }

    /// Becerinin bildirdiği TÜM araçlar oturumda var mı.
    ///
    /// Kapı "hepsi" üzerinden çünkü kılavuz iki adımlı bir akış anlatabiliyor
    /// (belge-duzenle: önce `belge_oku`, sonra `belge_duzenle`); yarısı eksikse
    /// kılavuz zaten uygulanamaz. Araç bildirmeyen beceri (hesap, kullanıcı
    /// becerileri) her sette serbesttir.
    static func aracVarMi(_ beceri: Beceri, _ mevcutAraclar: Set<String>?) -> Bool {
        guard let mevcut = mevcutAraclar, !beceri.araclar.isEmpty else { return true }
        return beceri.araclar.allSatisfy(mevcut.contains)
    }

    /// Tetikleyici aramasında SÖZCÜK BAŞI şartı arar (ham alt-dizgi değil).
    ///
    /// Ham `contains` kısa tetikleyicileri sık sözcüklerin İÇİNDE buluyordu:
    /// "alfabetik" içindeki "betik" kod becerisini, "yüzde"nin içindeki "yüz"ü
    /// hesap becerisini çağırıyordu; yanlış kılavuz enjekte edilince model o
    /// becerinin aracını zorlayıp şemaya uymayan bir çağrı üretiyordu. Türkçe
    /// SONA ek aldığı için yalnızca sözcük BAŞI sınırlanır — "satır" hâlâ
    /// "satırını" ile eşleşir, ki istenen budur.
    ///
    /// Boşluk kullanmayan yazılarda (CJK, Korece) sözcük sınırı diye bir şey
    /// yok; oradaki tetikleyiciler için ham alt-dizgiye düşülür, aksi hâlde
    /// hiç eşleşmezlerdi.
    /// Sözcük BAŞI yetmediği uzunluk sınırı. Bunun altındaki tetikleyiciler
    /// için sözcük SONU da aranır (tam sözcük eşleşmesi).
    ///
    /// Ölçülen hata: "ara" tetikleyicisi "**ara**lık"ı yakalıyordu, yani her
    /// Aralık ayı sorusu Spotlight arama becerisini çağırıyor, 4096 bütçesinden
    /// ~700 karakter yiyor ve modele alakasız kılavuz enjekte ediyordu. Aynı
    /// tuzak "bul" → "bulut/bulmaca/bulunduğu"da da var.
    ///
    /// Neden yalnız KISA tetikleyicilerde: Türkçe SONA ek alır, dolayısıyla
    /// uzun tetikleyicilerde ön-ek eşleşmesi ŞART ("satır" → "satırını",
    /// "çarp" → "çarparsak"). Ama kısa bir kök, kendisiyle akrabalığı olmayan
    /// bambaşka sözcüklerin de başında durabiliyor — orada gürültü üretiyor.
    ///
    /// Eşik 4: 3 harfli kökler (ara/bul/oku/dök/pdf) tam sözcük ister, 4+
    /// ön-ek eşleşmesini sürdürür. Eşiği 5 yapmak "çarp"ı kırardı.
    ///
    /// Asimetri bilinçli: yanlış beceri enjekte etmek modeli YANILTIR ve
    /// bütçeden ~700 karakter yer; beceriyi kaçırmak yalnızca fazladan
    /// kılavuzu kaybettirir — araç zaten oturumda durur. Kaçırmak ucuz,
    /// yanlış eşleşmek pahalı; o yüzden kısa köklerde katı davranılır.
    static let tamSozcukSiniri = 4

    private static func icerir(_ metin: String, _ tetik: String) -> Bool {
        guard let ilk = tetik.unicodeScalars.first, ilk.value < 0x0590 else {
            return metin.contains(tetik)   // CJK/Korece: sınır kavramı yok
        }
        // Boşluk içeren tetikleyiciler ("kaç satır") zaten öbek; kısalık kuralı
        // yalnız tek sözcüklük köklere uygulanır.
        let tamSozcukGerek = tetik.count < tamSozcukSiniri && !tetik.contains(" ")

        var alan = metin.startIndex..<metin.endIndex
        while let r = metin.range(of: tetik, range: alan) {
            let bastaMi = r.lowerBound == metin.startIndex
                || !metin[metin.index(before: r.lowerBound)].isLetter
                && !metin[metin.index(before: r.lowerBound)].isNumber
            if bastaMi {
                if !tamSozcukGerek { return true }
                // Kısa tetikleyici: sonrasında harf/rakam gelmemeli.
                let sonrasi = r.upperBound
                if sonrasi == metin.endIndex { return true }
                let sonraki = metin[sonrasi]
                if !sonraki.isLetter && !sonraki.isNumber { return true }
            }
            guard r.lowerBound < metin.endIndex else { break }
            alan = metin.index(after: r.lowerBound)..<metin.endIndex
        }
        return false
    }

    /// Metni satır sınırında en fazla `tavan` karaktere indirir (yarım kural kalmasın).
    private static func satirdaKes(_ metin: String, tavan: Int) -> String {
        guard metin.count > tavan else { return metin }
        let kesik = String(metin.prefix(tavan))
        guard let son = kesik.range(of: "\n", options: .backwards) else { return kesik }
        return String(kesik[..<son.lowerBound])
    }

    /// Gövdeyi (çekirdek, kuyruk) diye ayırır. İşaret yoksa çekirdek boştur ve
    /// tüm gövde kuyruk sayılır — kullanıcı becerileri işaret koymak zorunda değil.
    static func cekirdekAyir(_ metin: String) -> (cekirdek: String, kuyruk: String) {
        let govde = metin.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let r = govde.range(of: cekirdekIsareti) else { return ("", govde) }
        return (String(govde[..<r.lowerBound]).trimmingCharacters(in: .whitespacesAndNewlines),
                String(govde[r.upperBound...]).trimmingCharacters(in: .whitespacesAndNewlines))
    }

    /// Modele gidecek gövde: ÇEKİRDEK ÖNCE, artan bütçeye kuyruk.
    ///
    /// Çekirdek bütünlüğü sınırdan önce gelir; yine de bir beceri çekirdeği
    /// tavanı aşarsa satırda kesilir (aksi hâlde tek bir dosya 4096 pencerenin
    /// bütçesini sessizce yiyebilirdi).
    static func enjeksiyonGovdesi(_ metin: String) -> String {
        let (cekirdek, kuyruk) = cekirdekAyir(metin)
        guard !cekirdek.isEmpty else { return satirdaKes(kuyruk, tavan: enjeksiyonSiniri) }

        let govde = satirdaKes(cekirdek, tavan: enjeksiyonSiniri)
        let kalan = enjeksiyonSiniri - govde.count - 1   // -1: araya girecek "\n"
        guard kalan >= kuyrukEsigi, !kuyruk.isEmpty else { return govde }
        let ek = satirdaKes(kuyruk, tavan: kalan)
        return ek.isEmpty ? govde : govde + "\n" + ek
    }

    /// Modele verilecek biçim: çekirdek-önce gövde + "bunu anlatma" çitleri.
    static func enjeksiyonMetni(_ beceri: Beceri) -> String {
        let govde = enjeksiyonGovdesi(beceri.metin)
        return """
        <guidance name="\(beceri.ad)">
        \(govde)
        </guidance>
        Follow the guidance above when answering. It is internal: never quote, \
        summarize, or mention it, and never reply with the guidance itself.
        """
    }

    // MARK: - Yükleme

    private static func yukle() -> [Beceri] {
        let urller = Bundle.main.urls(forResourcesWithExtension: "md", subdirectory: nil) ?? []
        return urller.compactMap { ayristir($0) }
    }

    /// Frontmatter (--- ad: … / tetikler: … ---) + gövdeyi ayrıştırır.
    private static func ayristir(_ url: URL) -> Beceri? {
        guard let ham = try? String(contentsOf: url, encoding: .utf8) else { return nil }
        var ad = url.deletingPathExtension().lastPathComponent
        var tetikler: [String] = []
        var araclar: [String] = []
        var govde = ham

        let satirlar = ham.components(separatedBy: "\n")
        if satirlar.first == "---", let kapanis = satirlar.dropFirst().firstIndex(of: "---") {
            for satir in satirlar[1..<kapanis] {
                let parca = satir.split(separator: ":", maxSplits: 1).map {
                    $0.trimmingCharacters(in: .whitespaces)
                }
                guard parca.count == 2 else { continue }
                switch parca[0] {
                case "ad": ad = parca[1]
                case "tetikler":
                    tetikler = parca[1]
                        .split(separator: ",")
                        .map { $0.trimmingCharacters(in: .whitespaces).lowercased() }
                        .filter { !$0.isEmpty }
                case "araclar":
                    araclar = parca[1]
                        .split(separator: ",")
                        .map { $0.trimmingCharacters(in: .whitespaces) }
                        .filter { !$0.isEmpty }
                default: break
                }
            }
            govde = satirlar[(kapanis + 1)...].joined(separator: "\n")
                .trimmingCharacters(in: .whitespacesAndNewlines)
        }
        guard !tetikler.isEmpty else { return nil }
        return Beceri(ad: ad, tetikler: tetikler, metin: govde, araclar: araclar)
    }

    // MARK: - Yeniden enjeksiyon (mesafeli işaret)

    /// Hangi becerinin hangi turda enjekte edildiğini tutan saf durum makinesi.
    ///
    /// Eski davranış: beceri bir kez enjekte edilip kalıcı işaretleniyordu.
    /// Uzun turda transcript ilerledikçe kılavuz pencereden kayıyor, ama işaret
    /// durduğu için bir daha asla girmiyordu — geç turlarda davranış sapması
    /// tam da buradan geliyordu. İşaret artık MESAFELİ: aradan yeterince tur
    /// geçtiyse kılavuz yeniden yürürlüğe girer.
    ///
    /// Durum ModelServisi'nde tutulur, mantık burada durur — modelsiz test
    /// edilebilsin diye.
    struct EnjeksiyonDurumu {
        /// Kaç tur sonra aynı beceri yeniden enjekte edilebilir.
        ///
        /// 6: bir beceri ~700 karakter yer ve 4096 penceresinde her turda
        /// tekrarlamak bütçeyi yerdi; öte yandan çok büyük bir mesafede kılavuz
        /// pencereden kayıp bir daha dönmezdi. 6 tur, tipik bir araç-kullanım
        /// alışverişinin (soru → araç → yanıt) iki katıdır.
        static let mesafe = 6

        /// Şu ana kadar işlenen tur sayısı (1'den başlar).
        private(set) var tur = 0
        private var sonEnjeksiyon: [String: Int] = [:]

        /// Her turun BAŞINDA bir kez çağrılır.
        mutating func turBasla() { tur += 1 }

        /// Bu beceri bu turda enjekte edilmeli mi (hiç girmediyse ya da
        /// üstünden `mesafe` tur geçtiyse).
        func gerekliMi(_ ad: String) -> Bool {
            guard let son = sonEnjeksiyon[ad] else { return true }
            return tur - son >= Self.mesafe
        }

        /// Enjeksiyon GERÇEKTEN yapıldığında çağrılır. Profil uymadığı için
        /// atlanan beceri işaretlenmez — doğru profile geçilince yeniden denenir.
        mutating func isaretle(_ ad: String) { sonEnjeksiyon[ad] = tur }

        /// Yeni oturum = yeni bağlam: sayaç ve işaretler sıfırlanır.
        mutating func sifirla() {
            tur = 0
            sonEnjeksiyon.removeAll()
        }
    }
}
