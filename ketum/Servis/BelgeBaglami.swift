//
//  BelgeBaglami.swift
//  ketum
//
//  Belge bağlamı: sohbete paylaşılan (okuma/düzenleme için) aktif belge ve
//  üretilen dosyalar (QuickLook önizleme + paylaşım + Dosyalar'a kayıt).
//  Araçlar buraya erişir; UI buradan önizler. AracYurutucu ile aynı desen.
//

import Foundation
import Observation

/// Sohbete eklenmiş bir belge (kullanıcının paylaştığı).
struct EkliBelge: Identifiable, Hashable {
    var id = UUID()
    var url: URL
    var ad: String
    var bicim: BelgeBicimi
}

@MainActor
@Observable
final class BelgeBaglami {
    /// Şu an sohbette aktif olan, okunabilir/düzenlenebilir belge.
    var aktifBelge: EkliBelge?
    /// Bu oturumda üretilen dosyalar (en yeni en sonda).
    private(set) var uretilenler: [URL] = []
    /// UI'nın QuickLook ile açacağı son üretilen/istenmiş dosya.
    var onizlenecek: URL?
    /// sirr'in az önce ürettiği belge. Kullanıcı bir şey eklemese de "onu tablo
    /// olarak göster" / "bir satır ekle" gibi devam istekleri buna bağlanır.
    private(set) var sonUretilen: EkliBelge?

    /// Araçların üzerinde çalışacağı belge: kullanıcının eklediği varsa o,
    /// yoksa bu sohbette en son üretilen. Devam isteklerini bağlamsız bırakmaz.
    var calisilabilirBelge: EkliBelge? { aktifBelge ?? sonUretilen }

    /// Uygulama genelinde kullanılan koruma sınıfı: cihaz kilitliyken okunamaz,
    /// ama kilitliyken yeni dosya yazılabilsin diye `.complete` değil.
    nonisolated static let korumaSinifi = FileProtectionType.completeUnlessOpen

    /// Documents/sirr'in YOLU. Saf hesap — diske dokunmaz, klasör yaratmaz.
    /// Okuma yolları (boyut/listeleme) bunu kullanır.
    nonisolated static func ciktiKlasoruYolu() -> URL {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("sirr", isDirectory: true)
    }

    /// Klasör kurulumu süreç başına BİR KEZ. `static let` tembel ve iş parçacığı
    /// güvenli başlatılır; her boyut sorgusunda createDirectory/setResourceValues
    /// yeniden çalışmaz (Ayarlar ekranı bunu her çiziminde çağırıyordu).
    nonisolated private static let kokKurulumu: Void = {
        klasorHazirla(ciktiKlasoruYolu())
    }()

    /// sirr çıktılarının yazıldığı klasör: Documents/sirr.
    /// "Her şey bu cihazda kalır" vaadi burada koda dönüşüyor: klasör cihaz kilitliyken
    /// okunamayacak şekilde korunur ve iCloud/iTunes yedeğinden hariç tutulur.
    /// Yalnız YAZMA yolları çağırmalı — okuma için `ciktiKlasoruYolu()` var.
    nonisolated static func ciktiKlasoru() -> URL {
        _ = kokKurulumu
        return ciktiKlasoruYolu()
    }

    /// DEBUG test çıktılarının klasörü: Caches/sirr-test. Üretim klasöründen
    /// ayrıdır — test log'ları gerçek takvim/kişi yanıtları içerebilir, kullanıcının
    /// belgelerinin arasında durmamalı ve yedeğe/paylaşıma karışmamalı.
    nonisolated static func testKlasoru() -> URL {
        _ = testKurulumu
        return FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("sirr-test", isDirectory: true)
    }

    nonisolated private static let testKurulumu: Void = {
        let onbellek = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
        klasorHazirla(onbellek.appendingPathComponent("sirr-test", isDirectory: true))
    }()

    /// Verilen yolu iCloud/iTunes yedeğinden çıkarır.
    nonisolated static func yedektenHaricTut(_ url: URL) {
        var hedef = url
        var degerler = URLResourceValues()
        degerler.isExcludedFromBackup = true
        try? hedef.setResourceValues(degerler)
    }

    /// Bir KLASÖRÜ (yoksa yaratarak) koruma sınıfıyla kurar ve yedekten hariç tutar.
    /// sirr altında alt klasör açan her yer (ör. "Ekli/") bunu çağırmalı; böylece
    /// alt klasör de kök gibi korunur. Klasörün URL'ini döndürür.
    @discardableResult
    nonisolated static func klasorHazirla(_ klasor: URL) -> URL {
        try? FileManager.default.createDirectory(
            at: klasor, withIntermediateDirectories: true,
            attributes: [.protectionKey: korumaSinifi])
        korumayiUygula(klasor)
        return klasor
    }

    /// sirr kökü altında adlandırılmış bir alt klasör açar (ör. `altKlasor("Ekli")`).
    /// Koruma + yedekten hariç tutma uygulanmış olarak döner.
    @discardableResult
    nonisolated static func altKlasor(_ ad: String) -> URL {
        klasorHazirla(ciktiKlasoru().appendingPathComponent(ad, isDirectory: true))
    }

    /// Tek bir dosyaya/klasöre koruma sınıfını + yedek hariç tutmayı uygular.
    /// Dosyalar için doğrudan çağırmaya gerek yok: `BelgeMotoru.yaz(...)` sarmalayıcısı
    /// yazma yolunun kendisinde uyguluyor (yeni motor eklendiğinde unutulamaz).
    nonisolated static func korumayiUygula(_ url: URL) {
        try? FileManager.default.setAttributes(
            [.protectionKey: korumaSinifi],
            ofItemAtPath: url.path)
        yedektenHaricTut(url)
    }

    func belgeEkle(url: URL) {
        let bicim = BelgeBicimi(uzanti: url.pathExtension) ?? .txt
        aktifBelge = EkliBelge(url: url, ad: url.lastPathComponent, bicim: bicim)
    }

    func belgeKaldir() { aktifBelge = nil }

    func ciktiEklendi(_ url: URL) {
        Self.korumayiUygula(url)
        uretilenler.append(url)
        onizlenecek = url
        let bicim = BelgeBicimi(uzanti: url.pathExtension) ?? .txt
        sonUretilen = EkliBelge(url: url, ad: url.lastPathComponent, bicim: bicim)
    }

    /// Yeni sohbet: üretim geçmişi de silinir, yoksa yeni sohbet eski dosyayı okur.
    /// Yalnız bellekteki liste temizlenir — dosyalar diskte kalır, onları silmek
    /// kullanıcının kararı (bkz. `tumDosyalariSil()`).
    func uretimiUnut() {
        uretilenler.removeAll()
        sonUretilen = nil
    }

    // MARK: - Disk yönetimi (Ayarlar ekranı için)

    /// Documents/sirr altındaki tüm dosyalar (üretilenler + kullanıcının eklediği
    /// kopyalar), en yeni en başta. Klasörler listelenmez, içleri gezilir.
    nonisolated static func disktekiDosyalar() -> [URL] {
        // Okuma yolu: klasörü yaratmaz/değiştirmez. Klasör henüz yoksa liste boştur.
        let kok = ciktiKlasoruYolu()
        guard let gezgin = FileManager.default.enumerator(
            at: kok,
            includingPropertiesForKeys: [.isRegularFileKey, .contentModificationDateKey],
            options: [.skipsHiddenFiles]) else { return [] }

        var bulunan: [(url: URL, tarih: Date)] = []
        for oge in gezgin {
            guard let url = oge as? URL,
                  let ozellik = try? url.resourceValues(
                    forKeys: [.isRegularFileKey, .contentModificationDateKey]),
                  ozellik.isRegularFile == true else { continue }
            bulunan.append((url, ozellik.contentModificationDate ?? .distantPast))
        }
        return bulunan.sorted { $0.tarih > $1.tarih }.map(\.url)
    }

    /// Documents/sirr altındaki tüm dosyaların toplam boyutu (bayt).
    nonisolated static func toplamBoyut() -> Int64 {
        disktekiDosyalar().reduce(into: Int64(0)) { toplam, url in
            let ozellik = try? url.resourceValues(forKeys: [.fileSizeKey])
            toplam += Int64(ozellik?.fileSize ?? 0)
        }
    }

    /// `toplamBoyut()` ile aynı; Ayarlar ekranının beklediği ad.
    nonisolated static func ciktiBoyutu() -> Int64 { toplamBoyut() }

    /// `tumDosyalariSil()` ile aynı; Ayarlar ekranının beklediği ad.
    /// Hata fırlatmaz, sayı döndürmez — sessizce temizler.
    func ciktilariSil() { tumDosyalariSil() }

    /// Tek bir üretilmiş dosyayı siler.
    nonisolated static func dosyaSil(_ url: URL) {
        try? FileManager.default.removeItem(at: url)
    }

    /// Bir yolun altındaki düz dosya sayısı (yolun kendisi dosyaysa 1).
    /// Silmeden ÖNCE çağrılır — silinen gerçek dosya sayısını verir.
    nonisolated private static func dosyaSayisi(_ url: URL) -> Int {
        let ozellik = try? url.resourceValues(forKeys: [.isDirectoryKey])
        guard ozellik?.isDirectory == true else { return 1 }
        guard let gezgin = FileManager.default.enumerator(
            at: url, includingPropertiesForKeys: [.isRegularFileKey]) else { return 0 }
        var sayac = 0
        for oge in gezgin {
            guard let alt = oge as? URL,
                  let o = try? alt.resourceValues(forKeys: [.isRegularFileKey]),
                  o.isRegularFile == true else { continue }
            sayac += 1
        }
        return sayac
    }

    /// Documents/sirr içeriğini tamamen siler ve bellekteki bağlamı da temizler.
    /// GERİ ALINAMAZ. Silinen gerçek dosya sayısını döndürür (alt klasörlerin
    /// içindekiler dahil). Ayarlar ekranı bunu onay sonrası çağırır.
    ///
    /// Kapsam güvenliği: yalnızca `Documents/sirr`'in DOĞRUDAN içeriği silinir;
    /// kökün kendisi durur ve hiçbir koşulda daha yukarı bir dizine çıkılmaz.
    @discardableResult
    func tumDosyalariSil() -> Int {
        let fm = FileManager.default
        let belgeler = fm.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let kok = Self.ciktiKlasoruYolu()
        // Kök gerçekten Documents/sirr mi? Değilse hiçbir şey silme.
        guard kok.lastPathComponent == "sirr",
              kok.deletingLastPathComponent().standardizedFileURL == belgeler.standardizedFileURL
        else { return 0 }

        var sayac = 0
        // Tek geçiş: kökün doğrudan içeriğini gez, her öğeyi silmeden önce say, sonra sil.
        if let icerik = try? fm.contentsOfDirectory(
            at: kok, includingPropertiesForKeys: [.isDirectoryKey], options: []) {
            for oge in icerik {
                let adet = Self.dosyaSayisi(oge)
                if (try? fm.removeItem(at: oge)) != nil { sayac += adet }
            }
        }

        // Silinen dosyayı gösteren bağlam kalmasın.
        aktifBelge = nil
        onizlenecek = nil
        uretimiUnut()
        return sayac
    }
}
