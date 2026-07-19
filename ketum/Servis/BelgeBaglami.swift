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

    /// sirr çıktılarının yazıldığı klasör: Documents/sirr.
    nonisolated static func ciktiKlasoru() -> URL {
        let belgeler = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let klasor = belgeler.appendingPathComponent("sirr", isDirectory: true)
        try? FileManager.default.createDirectory(at: klasor, withIntermediateDirectories: true)
        return klasor
    }

    func belgeEkle(url: URL) {
        let bicim = BelgeBicimi(uzanti: url.pathExtension) ?? .txt
        aktifBelge = EkliBelge(url: url, ad: url.lastPathComponent, bicim: bicim)
    }

    func belgeKaldir() { aktifBelge = nil }

    func ciktiEklendi(_ url: URL) {
        uretilenler.append(url)
        onizlenecek = url
        let bicim = BelgeBicimi(uzanti: url.pathExtension) ?? .txt
        sonUretilen = EkliBelge(url: url, ad: url.lastPathComponent, bicim: bicim)
    }

    /// Yeni sohbet: üretim geçmişi de silinir, yoksa yeni sohbet eski dosyayı okur.
    func uretimiUnut() {
        uretilenler.removeAll()
        sonUretilen = nil
    }
}
