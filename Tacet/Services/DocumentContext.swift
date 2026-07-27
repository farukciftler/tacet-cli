//
//  BelgeBaglami.swift
//  Tacet
//
//  Belge bağlamı: sohbete paylaşılan (okuma/düzenleme için) aktif belge ve
//  üretilen dosyalar (QuickLook önizleme + paylaşım + Dosyalar'a kayıt).
//  Araçlar buraya erişir; UI buradan önizler. AracYurutucu ile aynı desen.
//

import Foundation
import Observation

/// Sohbete eklenmiş bir belge (kullanıcının paylaştığı).
struct AttachedDocument: Identifiable, Hashable {
    var id = UUID()
    var url: URL
    var name: String
    var format: DocumentFormat
}

@MainActor
@Observable
final class DocumentContext {
    /// Şu an sohbette aktif olan, okunabilir/düzenlenebilir belge.
    var activeDocument: AttachedDocument?
    /// Bu oturumda üretilen dosyalar (en yeni en sonda).
    private(set) var uretilenler: [URL] = []
    /// UI'nın QuickLook ile açacağı son üretilen/istenmiş dosya.
    var toPreview: URL?
    /// Tacet'in az önce ürettiği belge. Kullanıcı bir şey eklemese de "onu tablo
    /// olarak göster" / "bir satır ekle" gibi devam istekleri buna bağlanır.
    private(set) var sonUretilen: AttachedDocument?

    /// Araçların üzerinde çalışacağı belge: kullanıcının eklediği varsa o,
    /// yoksa bu sohbette en son üretilen. Devam isteklerini bağlamsız bırakmaz.
    var runnableDocument: AttachedDocument? { activeDocument ?? sonUretilen }

    /// Uygulama genelinde kullanılan koruma sınıfı: cihaz kilitliyken okunamaz,
    /// ama kilitliyken yeni dosya yazılabilsin diye `.complete` değil.
    nonisolated static let korumaSinifi = FileProtectionType.completeUnlessOpen

    /// Documents/Tacet'in YOLU. Saf hesap — diske dokunmaz, klasör yaratmaz.
    /// Okuma yolları (boyut/listeleme) bunu kullanır.
    nonisolated static func outputFolderPath() -> URL {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Tacet", isDirectory: true)
    }

    /// Klasör kurulumu süreç başına BİR KEZ. `static let` tembel ve iş parçacığı
    /// güvenli başlatılır; her boyut sorgusunda createDirectory/setResourceValues
    /// yeniden çalışmaz (Ayarlar ekranı bunu her çiziminde çağırıyordu).
    nonisolated private static let kokKurulumu: Void = {
        prepareFolder(outputFolderPath())
    }()

    /// Tacet çıktılarının yazıldığı klasör: Documents/Tacet.
    /// "Her şey bu cihazda kalır" vaadi burada koda dönüşüyor: klasör cihaz kilitliyken
    /// okunamayacak şekilde korunur ve iCloud/iTunes yedeğinden hariç tutulur.
    /// Yalnız YAZMA yolları çağırmalı — okuma için `outputFolderPath()` var.
    nonisolated static func outputFolder() -> URL {
        _ = kokKurulumu
        return outputFolderPath()
    }

    /// DEBUG test çıktılarının klasörü: Caches/tacet-test. Üretim klasöründen
    /// ayrıdır — test log'ları gerçek takvim/kişi yanıtları içerebilir, kullanıcının
    /// belgelerinin arasında durmamalı ve yedeğe/paylaşıma karışmamalı.
    nonisolated static func testKlasoru() -> URL {
        _ = testKurulumu
        return FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("tacet-test", isDirectory: true)
    }

    nonisolated private static let testKurulumu: Void = {
        let cache = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
        prepareFolder(cache.appendingPathComponent("tacet-test", isDirectory: true))
    }()

    /// Verilen yolu iCloud/iTunes yedeğinden çıkarır.
    nonisolated static func yedektenHaricTut(_ url: URL) {
        var target = url
        var degerler = URLResourceValues()
        degerler.isExcludedFromBackup = true
        try? target.setResourceValues(degerler)
    }

    /// Bir KLASÖRÜ (yoksa yaratarak) koruma sınıfıyla kurar ve yedekten hariç tutar.
    /// Tacet altında alt klasör açan her yer (ör. "Ekli/") bunu çağırmalı; böylece
    /// alt klasör de kök gibi korunur. Klasörün URL'ini döndürür.
    @discardableResult
    nonisolated static func prepareFolder(_ folder: URL) -> URL {
        try? FileManager.default.createDirectory(
            at: folder, withIntermediateDirectories: true,
            attributes: [.protectionKey: korumaSinifi])
        applyProtection(folder)
        return folder
    }

    /// Tacet kökü altında adlandırılmış bir alt klasör açar (ör. `altKlasor("Ekli")`).
    /// Koruma + yedekten hariç tutma uygulanmış olarak döner.
    @discardableResult
    nonisolated static func altKlasor(_ name: String) -> URL {
        prepareFolder(outputFolder().appendingPathComponent(name, isDirectory: true))
    }

    /// Tek bir dosyaya/klasöre koruma sınıfını + yedek hariç tutmayı uygular.
    /// Dosyalar için doğrudan çağırmaya gerek yok: `BelgeMotoru.yaz(...)` sarmalayıcısı
    /// yazma yolunun kendisinde uyguluyor (yeni motor eklendiğinde unutulamaz).
    nonisolated static func applyProtection(_ url: URL) {
        try? FileManager.default.setAttributes(
            [.protectionKey: korumaSinifi],
            ofItemAtPath: url.path)
        yedektenHaricTut(url)
    }

    func addDocument(url: URL) {
        let format = DocumentFormat(fileExtension: url.pathExtension) ?? .txt
        activeDocument = AttachedDocument(url: url, name: url.lastPathComponent, format: format)
    }

    func removeDocument() { activeDocument = nil }

    func outputAdded(_ url: URL) {
        Self.applyProtection(url)
        uretilenler.append(url)
        toPreview = url
        let format = DocumentFormat(fileExtension: url.pathExtension) ?? .txt
        sonUretilen = AttachedDocument(url: url, name: url.lastPathComponent, format: format)
    }

    /// Yeni sohbet: üretim geçmişi de silinir, yoksa yeni sohbet eski dosyayı okur.
    /// Yalnız bellekteki liste temizlenir — dosyalar diskte kalır, onları silmek
    /// kullanıcının kararı (bkz. `tumDosyalariSil()`).
    func uretimiUnut() {
        uretilenler.removeAll()
        sonUretilen = nil
    }

    // MARK: - Disk yönetimi (Ayarlar ekranı için)

    /// Documents/Tacet altındaki tüm dosyalar (üretilenler + kullanıcının eklediği
    /// kopyalar), en yeni en başta. Klasörler listelenmez, içleri gezilir.
    nonisolated static func filesOnDisk() -> [URL] {
        // Okuma yolu: klasörü yaratmaz/değiştirmez. Klasör henüz yoksa liste boştur.
        let root = outputFolderPath()
        guard let gezgin = FileManager.default.enumerator(
            at: root,
            includingPropertiesForKeys: [.isRegularFileKey, .contentModificationDateKey],
            options: [.skipsHiddenFiles]) else { return [] }

        var bulunan: [(url: URL, date: Date)] = []
        for item in gezgin {
            guard let url = item as? URL,
                  let ozellik = try? url.resourceValues(
                    forKeys: [.isRegularFileKey, .contentModificationDateKey]),
                  ozellik.isRegularFile == true else { continue }
            bulunan.append((url, ozellik.contentModificationDate ?? .distantPast))
        }
        return bulunan.sorted { $0.date > $1.date }.map(\.url)
    }

    /// Documents/Tacet altındaki tüm dosyaların toplam boyutu (bayt).
    nonisolated static func totalSize() -> Int64 {
        filesOnDisk().reduce(into: Int64(0)) { total, url in
            let ozellik = try? url.resourceValues(forKeys: [.fileSizeKey])
            total += Int64(ozellik?.fileSize ?? 0)
        }
    }

    /// `toplamBoyut()` ile aynı; Ayarlar ekranının beklediği ad.
    nonisolated static func outputSize() -> Int64 { totalSize() }

    /// `tumDosyalariSil()` ile aynı; Ayarlar ekranının beklediği ad.
    /// Hata fırlatmaz, sayı döndürmez — sessizce temizler.
    func deleteOutputs() { deleteAllFiles() }

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
        var counter = 0
        for item in gezgin {
            guard let lower = item as? URL,
                  let o = try? lower.resourceValues(forKeys: [.isRegularFileKey]),
                  o.isRegularFile == true else { continue }
            counter += 1
        }
        return counter
    }

    /// Documents/Tacet içeriğini tamamen siler ve bellekteki bağlamı da temizler.
    /// GERİ ALINAMAZ. Silinen gerçek dosya sayısını döndürür (alt klasörlerin
    /// içindekiler dahil). Ayarlar ekranı bunu onay sonrası çağırır.
    ///
    /// Kapsam güvenliği: yalnızca `Documents/Tacet`'in DOĞRUDAN içeriği silinir;
    /// kökün kendisi durur ve hiçbir koşulda daha yukarı bir dizine çıkılmaz.
    @discardableResult
    func deleteAllFiles() -> Int {
        let fm = FileManager.default
        let belgeler = fm.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let root = Self.outputFolderPath()
        // Kök gerçekten Documents/Tacet mi? Değilse hiçbir şey silme.
        guard root.lastPathComponent == "Tacet",
              root.deletingLastPathComponent().standardizedFileURL == belgeler.standardizedFileURL
        else { return 0 }

        var counter = 0
        // Tek geçiş: kökün doğrudan içeriğini gez, her öğeyi silmeden önce say, sonra sil.
        if let content = try? fm.contentsOfDirectory(
            at: root, includingPropertiesForKeys: [.isDirectoryKey], options: []) {
            for item in content {
                let adet = Self.dosyaSayisi(item)
                if (try? fm.removeItem(at: item)) != nil { counter += adet }
            }
        }

        // Silinen dosyayı gösteren bağlam kalmasın.
        activeDocument = nil
        toPreview = nil
        uretimiUnut()
        return counter
    }
}
