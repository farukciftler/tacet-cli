//
//  VeriDeposu.swift
//  Tacet
//
//  Bağlam penceresini aşmadan büyük veri taşıma kanalı (spec §7.2, §7.3.2 akış 2).
//  Kaynak araç (ör. Takvim) topladığı yapılandırılmış veriyi buraya koyar ve modele
//  YALNIZCA kısa bir özet + referans ID döner. Üretim aracı (belge_olustur) veriyi
//  bu depodan çeker — toplu veri hiçbir zaman bağlam penceresine girmez. Dosya
//  boyutunu pencere değil cihaz sınırlar.
//

import Foundation
import Observation

@MainActor
@Observable
final class DataStore {
    /// Referans ID → tablo. Büyük tablolar burada yaşar, model bunları görmez.
    private var store: [String: Table] = [:]
    /// Tablo OLMAYAN büyük içerik (okunan belgenin düz gövdesi — P2-6).
    ///
    /// Ayrı bir sözlük tutulur çünkü düzyazıyı tek sütunluk bir `Tablo`ya
    /// sıkıştırmak, o ref'ten üretilen HER belgeyi (docx/pdf/md) tabloya
    /// çevirirdi: metni sessizce başka bir şeye dönüştüren yeni bir bozulma
    /// sınıfı. Tip ayrımı bunu baştan imkânsız kılar.
    private var metinDepo: [String: String] = [:]
    private var counter = 0

    // MARK: - Tavanlar (LRU)

    /// DEPO SOHBET BOYUNCA TEK YÖNLÜ BÜYÜYORDU. Okunan her belgenin tam gövdesi
    /// (belge başına yüz binlerce karakter) ve her araç tablosu burada kalıyor,
    /// yalnızca `temizle` boşaltıyordu; uzun bir sohbette bu, cihaz belleğini
    /// sessizce yiyen bir birikimdir.
    ///
    /// Tavan İKİ EKSENLİDİR: küçük ama yüzlerce kayıt da, iki tane devasa gövde
    /// de aynı derde yol açar; tek eksen ikincisini kaçırırdı.
    static let kayitTavani = 24
    static let karakterTavani = 800_000
    /// Düşürülen ref adlarından hatırlanacak en fazla sayı. Ad listesi (içerik
    /// değil) tutulur; "hiç var olmadı" ile "vardı, düştü" ayrımı bunun için.
    static let dusenTavani = 60

    /// Kullanım sırası — en ESKİ başta, en TAZE sonda. Düşürme baştan yer.
    /// Görünüm katmanı depo defterini izlemez; gözlem gürültüsü üretmesin.
    @ObservationIgnored private var order: [String] = []
    /// Ref başına karakter maliyeti — tavan hesabı her koyuşta tüm depoyu
    /// dolaşmasın diye saklanır.
    @ObservationIgnored private var boyutlar: [String: Int] = [:]
    @ObservationIgnored private var toplamKarakter = 0
    /// Tavan yüzünden DÜŞÜRÜLEN ref adları (en eski başta).
    @ObservationIgnored private var dusurulenler: [String] = []

    /// Tabloyu saklar, kısa bir referans ID döndürür. Model bu ID'yi taşır.
    func put(_ table: Table, tag: String) -> String {
        counter += 1
        let ref = "\(tag)-\(counter)"
        store[ref] = table
        deftereYaz(ref, size: Self.olcu(table))
        return ref
    }

    /// Düz metin gövdeyi saklar (aynı ref uzayı — ID'ler çakışmaz).
    func putText(_ text: String, tag: String) -> String {
        counter += 1
        let ref = "\(tag)-\(counter)"
        metinDepo[ref] = text
        deftereYaz(ref, size: text.count)
        return ref
    }

    /// Referansla tam tabloyu çeker (üretim aracı çağırır).
    func take(_ ref: String) -> Table? {
        let key = Self.normalize(ref)
        guard let table = store[key] else { return nil }
        refresh(key)
        return table
    }

    /// Referansla tam METİN gövdeyi çeker.
    func takeText(_ ref: String) -> String? {
        let key = Self.normalize(ref)
        guard let text = metinDepo[key] else { return nil }
        refresh(key)
        return text
    }

    // MARK: - Defter (LRU muhasebesi)

    /// Yeni kaydı sıranın SONUNA alır ve tavan aşıldıysa en eskileri düşürür.
    private func deftereYaz(_ ref: String, size: Int) {
        order.removeAll { $0 == ref }
        order.append(ref)
        toplamKarakter += size - (boyutlar[ref] ?? 0)
        boyutlar[ref] = size
        dusurulenler.removeAll { $0 == ref }
        prune()
    }

    /// Okunan ref en taze sayılır: bir belgeden art arda üretim yapan bir sohbette
    /// o belgenin, yalnızca yaşlı olduğu için altından çekilmesi anlamsız olurdu.
    private func refresh(_ ref: String) {
        guard order.last != ref, order.contains(ref) else { return }
        order.removeAll { $0 == ref }
        order.append(ref)
    }

    /// Tavan aşıldıkça EN ESKİ ref'i düşürür.
    ///
    /// EN YENİ KAYIT ASLA DÜŞMEZ (`sira.count > 1` koşulu): tek başına tavanı
    /// aşan bir gövde konur konmaz silinseydi, kaynak araç modele "koydum, ref
    /// şu" der, üretim aracı da o ref'i bulamazdı — 4096 bypass kanalının tam
    /// olarak önlemek için var olduğu sessiz veri kaybı.
    private func prune() {
        while order.count > 1,
              order.count > Self.kayitTavani || toplamKarakter > Self.karakterTavani {
            let eski = order.removeFirst()
            store[eski] = nil
            metinDepo[eski] = nil
            toplamKarakter -= boyutlar.removeValue(forKey: eski) ?? 0
            dusurulenler.append(eski)
        }
        if dusurulenler.count > Self.dusenTavani {
            dusurulenler.removeFirst(dusurulenler.count - Self.dusenTavani)
        }
    }

    /// Tablonun karakter maliyeti. Satır sayısına değil GERÇEK yüke bakılır:
    /// üç sütunlu 500 satır ile otuz sütunlu 500 satır aynı şey değil.
    private static func olcu(_ table: Table) -> Int {
        table.headers.reduce(0) { $0 + $1.count }
            + table.rows.reduce(0) { total, line in
                total + line.cells.reduce(0) { $0 + $1.count }
            }
    }

    /// Çözülemeyen bir ref için modele dönecek OLGU cümlesi (imperatif yönerge
    /// yok — P2-4 ile aynı duruş).
    ///
    /// İki hâl AYRI cümledir ve bu ayrım önemli: "hiç var olmadı" modelin ref'i
    /// uydurduğunu söyler, "vardı, yer açmak için düştü" ise veriyi yeniden
    /// toplamanın anlamlı olduğunu. Aynı cümleyi kullanmak, düşürülen bir ref'i
    /// modelin halüsinasyonu gibi gösterirdi.
    func refState(_ ref: String) -> String {
        let key = Self.normalize(ref)
        let eldekiler = refAnahtarlari
        let list = eldekiler.isEmpty ? "none" : eldekiler.joined(separator: ",")
        if dusurulenler.contains(key) {
            return "expired_data_ref: \"\(key)\" existed but was dropped to stay "
                + "within the device memory budget and is no longer available "
                + "(available: \(list))"
        }
        return "unknown_data_ref: \"\(key)\" (available: \(list))"
    }

    /// Ref herhangi bir kanalda çözülüyor mu? (P0-2 hata dalı bunu sorar.)
    func cozulurMu(_ ref: String) -> Bool {
        let key = Self.normalize(ref)
        return store[key] != nil || metinDepo[key] != nil
    }

    /// Model referansı çıplak vermez: araçlar modele "data_ref=takvim-1" diye
    /// döndüğü için model bu ANAHTAR-DEĞER ÇİFTİNİN TAMAMINI kaynakRef'e
    /// kopyalıyor. Eski eşleme yalnız tırnak/boşluk soyduğundan depo ıskalanıyor,
    /// ıska da (P0-2) sessiz boş belgeye dönüşüyordu. Önek soymak bu sınıfın
    /// tamamını kapatır; eşleşmeyen ref artık GERÇEKTEN yok demektir.
    static func normalize(_ raw: String) -> String {
        let cop = CharacterSet(charactersIn: " \t\n\r\"'`()[]{},;")
        var s = raw.trimmingCharacters(in: cop)
        // "data_ref=takvim-1", "kaynakRef: takvim-1", "ref = takvim-1" …
        for prefix in ["data_ref", "kaynakref", "kaynak_ref", "dataref", "ref"] {
            guard s.lowercased().hasPrefix(prefix) else { continue }
            let remaining = s.dropFirst(prefix.count).drop(while: { $0 == " " })
            // Ayırıcı ZORUNLU: yoksa "reference-1" gibi meşru bir ID kırpılırdı.
            guard let first = remaining.first, first == "=" || first == ":" else { continue }
            s = String(remaining.dropFirst()).trimmingCharacters(in: cop)
            break
        }
        return s
    }

    var bosMu: Bool { store.isEmpty && metinDepo.isEmpty }

    /// Yalnız anahtarlar — çözülemeyen ref hatasında modele "elde bunlar var"
    /// diye OLGU dönmek için (imperatif yönerge değil, mevcut adres listesi).
    var refAnahtarlari: [String] { (Array(store.keys) + Array(metinDepo.keys)).sorted() }

    /// Elde duran referanslar — "ref (etiket, N satır)" biçiminde.
    ///
    /// Profil değişimi oturumu YENİDEN KURAR ve transcript özete iner; bu sırada
    /// araçların döndürdüğü `kaynakRef`ler modelin bağlamından düşüyordu.
    /// Ölçülen sonuç: "namaz vakitleri" aranıp bulunduktan sonra "tablo yapsana"
    /// denince belge profiline geçiliyor, model elindeki veriyi göremiyor ve
    /// beceri dosyasındaki ÖRNEĞİ gerçek içerik sanıp alakasız bir dosya
    /// üretiyordu. Veri kaybolmuş değildi — yalnızca adresi unutulmuştu.
    var referanslar: [String] {
        let tablolar = store.keys.sorted().compactMap { ref -> String? in
            guard let t = store[ref] else { return nil }
            return "\(ref) (\(t.headers.joined(separator: "/")), \(t.rows.count) satır)"
        }
        let metinler = metinDepo.keys.sorted().compactMap { ref -> String? in
            guard let m = metinDepo[ref] else { return nil }
            return "\(ref) (metin, \(m.count) karakter)"
        }
        return tablolar + metinler
    }

    /// Yeni sohbete geçişte depoyu boşaltır.
    ///
    /// Sayaç KASITLI olarak sıfırlanmaz: monotonik artmaya devam eder. Sıfırlansaydı
    /// yeni sohbette yine "takvim-1" üretilirdi ve modelin bağlamında (ya da özet
    /// metninde) kalmış eski bir referans yepyeni bir tabloya çarpardı — kullanıcı
    /// başka bir sohbetin verisinden belge üretilmiş olurdu. Sessiz ve teşhisi zor
    /// bir veri karışması; ID'leri ucuza benzersiz tutmak bunu tümden ortadan kaldırır.
    func clear() {
        store.removeAll()
        metinDepo.removeAll()
        order.removeAll()
        boyutlar.removeAll()
        toplamKarakter = 0
        // Düşürülen ref listesi de gider: yeni sohbette "vardı ama düştü" demek
        // yanlış olurdu, o veri bu sohbete hiç ait değildi.
        dusurulenler.removeAll()
    }
}
