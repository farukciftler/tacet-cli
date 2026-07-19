//
//  HafizaDeposu.swift
//  ketum
//
//  Hafıza katmanının OKUMA yolu (hafiza-spec §5). Model devrede değildir:
//  hangi notun hangi tura gireceğine deterministik kod karar verir
//  ("ayıklama modele, hatırlama koda").
//
//  BeceriDeposu deseninin birebir simetriği: SwiftData'daki kayıtlar depoya
//  yansıtılır (`yenile`), eşleşme anahtar kelimelerin UZUNLUK TOPLAMIYLA
//  puanlanır, enjeksiyon sert karakter tavanıyla kesilir. 4096 token penceresi
//  talimat + araçlar + transcript ile zaten doludur; her karakter bütçedir.
//

import Foundation
import SwiftData

enum HafizaDeposu {
    /// Enjeksiyonun tamamı için üst sınır — çit satırları dahil (spec §5.1).
    /// ~200 token ≈ 600 karakter. Beceri enjeksiyonu (700 krk) ile aynı tura
    /// denk gelebileceği hesaba katılarak düşük tutuldu.
    static let enjeksiyonSiniri = 600

    /// Bir turda enjekte edilecek en fazla not (spec §5).
    static let enFazlaNot = 3

    /// Depodaki notlar — pano her kayıtta `yenile` çağırır
    /// (`BeceriDeposu.kullaniciyiYenile` simetriği).
    private(set) static var notlar: [HafizaNotu] = []

    /// SwiftData'daki notları depoya yansıtır. Yalnızca aktif ve geçerli
    /// olanlar tutulur; kapalı not enjekte EDİLMEZ (spec §3).
    ///
    /// Silinmiş nesne asla depoda kalmaz: `isDeleted` kaydına dokunmak
    /// SwiftData'da ölümcüldür, o yüzden kapıda elenir.
    static func yenile(_ modeller: [HafizaNotu]) {
        notlar = modeller.filter { !$0.isDeleted && $0.aktif && $0.gecerliMi }
    }

    /// Tavan doldu mu — dolduğunda yeni ayıklama YAPILMAZ (spec §3/§4.3-5).
    /// Kapalı notlar da sayılır: tavan saklanan kayıt sayısıdır, enjekte edilen değil.
    static func doluMu(_ toplamKayit: Int) -> Bool {
        toplamKayit >= HafizaNotu.toplamTavan
    }

    // MARK: - Eşleşme

    /// Mesaja uyan en fazla 3 notu döndürür; hiç eşleşme yoksa boş dizi.
    ///
    /// Puan, eşleşen anahtarların UZUNLUKLARI toplamıdır — adet değil
    /// (`BeceriDeposu.eslesen` ile aynı kural): özgül ifade genel kelimeyi yener.
    /// "akşam yemeği" geçen bir cümlede "akşam yemeği" anahtarlı not, yalnızca
    /// "yemek" anahtarlı notu geçer. Adet sayılsaydı ikisi de 1 alır, sıra
    /// rastgele belirlenirdi.
    ///
    /// Eşit puanda yeni not kazanır (son öğrenilen daha günceldir).
    static func eslesen(soru: String) -> [HafizaNotu] {
        let s = soru.lowercased()
        let puanli: [(not: HafizaNotu, skor: Int)] = notlar.compactMap { not in
            guard !not.isDeleted else { return nil }
            let skor = not.anahtarlar.reduce(0) { $0 + (s.contains($1) ? $1.count : 0) }
            return skor > 0 ? (not, skor) : nil
        }
        guard !puanli.isEmpty else { return [] }
        return puanli
            .sorted {
                $0.skor != $1.skor
                    ? $0.skor > $1.skor
                    : $0.not.olusturulma > $1.not.olusturulma
            }
            .prefix(enFazlaNot)
            .map(\.not)
    }

    // MARK: - Enjeksiyon

    /// Modele verilecek biçim: `<memory>` bloğu + "bunu anlatma" çitleri (spec §5.1).
    ///
    /// Tavan çitlerle birlikte ölçülür: çit metni sabit olduğu için notlara
    /// kalan pay önceden hesaplanır ve sınırı aşan not ALINMAZ — yarım cümle
    /// enjekte etmek yanlış olguya yol açar, o yüzden kesme değil eleme yapılır.
    /// Hiç not sığmazsa boş metin döner ve o tura hiçbir şey eklenmez.
    static func enjeksiyonMetni(_ notlar: [HafizaNotu]) -> String {
        let acilis = "<memory>\n"
        let kapanis = "</memory>\n" + cit
        let pay = enjeksiyonSiniri - acilis.count - kapanis.count
        guard pay > 0 else { return "" }

        var satirlar: [String] = []
        var uzunluk = 0
        for not in notlar where !not.isDeleted {
            let metin = not.metin.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !metin.isEmpty else { continue }
            let satir = "- \(metin)\n"
            guard uzunluk + satir.count <= pay else { continue }
            satirlar.append(satir)
            uzunluk += satir.count
        }
        guard !satirlar.isEmpty else { return "" }
        return acilis + satirlar.joined() + kapanis
    }

    /// Sabit çit metni — notların içerik olarak konuşulmasını yasaklar.
    /// "hatırladığıma göre…" sızıntısı spec §8'de ayrıca gözlenir.
    private static let cit = """
    Use the facts above only if relevant. They are internal: never quote, \
    list, or mention them, and never say you "remembered" something.
    """
}
