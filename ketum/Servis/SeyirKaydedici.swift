//
//  SeyirKaydedici.swift
//  ketum
//
//  Seyir'in üreticisi (seyir-spec §5.2). Bir yanıt turu boyunca yaşar,
//  turun deterministik olaylarını ardışık adımlara çevirir.
//
//  SALT GÖZLEMCİ (seyir-spec §2.5, §6 kabul ölçütü): buradaki hiçbir metin
//  isteme/talimata enjekte edilmez, modelden hiçbir durum bildirimi istenmez.
//  Kaydedicinin varlığı ile yokluğu arasında model çıktısı bit düzeyinde
//  aynıdır; istem bütçesine maliyeti SIFIRDIR.
//
//  Adımlar ARDIŞIKTIR: yeni adım açılınca bir önceki kapanır. FoundationModels
//  araç çağrılarını tek tek verdiği için paralel adım kurgusuna gerek yoktur.
//

import Foundation
import SwiftData

@MainActor
@Observable
final class SeyirKaydedici {

    /// Turun şu ana kadarki adımları. Görünüm katmanı bunu izler.
    private(set) var adimlar: [SeyirAdimi] = []

    /// Tur hâlâ sürüyor mu (bitir/kes çağrılmadı).
    private(set) var suruyorMu: Bool = true

    init() {}

    // MARK: - Yaşam döngüsü

    /// Yeni adım açar, açık olan önceki adımı kapatır.
    /// `tur == .arac` ise metin yok sayılır — araç satırının metni tek doğruluk
    /// kaynağı olan `AracIzi`den okunur (seyir-spec §2.4).
    func basla(tur: SeyirTuru, metin: String = "") {
        guard suruyorMu else { return }
        sonuKapat()
        adimlar.append(SeyirAdimi(tur: tur, metin: metin))
    }

    /// Açık araç adımını `AracIzi`ye bağlar. Son adım araç değilse (ya da zaten
    /// bağlıysa) yeni bir araç adımı açar — çağıranın `basla(tur: .arac)` demeyi
    /// unutması sessiz veri kaybına yol açmasın diye toleranslıdır.
    func aracBagla(izID: UUID) {
        guard suruyorMu else { return }
        if let son = adimlar.last, son.tur == .arac, son.aracIziID == nil, son.bitis == nil {
            adimlar[adimlar.count - 1].aracIziID = izID
        } else {
            sonuKapat()
            adimlar.append(SeyirAdimi(tur: .arac, aracIziID: izID))
        }
    }

    /// Tur normal bitti: açık adım kapanır, kaydedici kapanır.
    func bitir() {
        guard suruyorMu else { return }
        sonuKapat()
        suruyorMu = false
    }

    /// Tur yarıda kesildi (iptal / sahne kesintisi). Açık adım kapanır ve
    /// sona kapalı bir `kesinti` adımı eklenir — sessiz kaybolma yoktur
    /// (seyir-spec §3.4). Böylece `adimlar.last?.tur == .kesinti` olur ve
    /// kesintiden önce ne yapıldığı da listede durur.
    func kes() {
        guard suruyorMu else { return }
        let acikVardi = adimlar.last.map { $0.bitis == nil } ?? false
        sonuKapat()
        if acikVardi || adimlar.isEmpty {
            let an = Date()
            adimlar.append(SeyirAdimi(tur: .kesinti,
                                      metin: SeyirKaydedici.yaridaKaldiMetni,
                                      baslangic: an,
                                      bitis: an))
        }
        suruyorMu = false
    }

    // MARK: - Kalıcılık

    /// Adım listesini mesaja yazar. Kaydedici SwiftData nesnesi TUTMAZ —
    /// mesaj yazma anında dışarıdan verilir; silinmiş ya da bağlamsız nesneye
    /// yazmak fatal error olacağı için önce kontrol edilir.
    func yaz(_ mesaj: Mesaj) {
        guard !mesaj.isDeleted, mesaj.modelContext != nil else { return }
        guard !adimlar.isEmpty else { return }
        mesaj.adimlar = adimlar
    }

    /// Yeni tur için temizler.
    func sifirla() {
        adimlar = []
        suruyorMu = true
    }

    // MARK: - İç

    /// Son adım açıksa şimdiyle kapatır. Süre negatif olamaz (`SeyirAdimi.sure`
    /// sıfıra kırpar), ama bitişi başlangıcın gerisine yazmamak için de kırpılır.
    private func sonuKapat() {
        guard let son = adimlar.last, son.bitis == nil else { return }
        adimlar[adimlar.count - 1].bitis = max(Date(), son.baslangic)
    }

    /// Kesinti adımının satır metni.
    static var yaridaKaldiMetni: String { String(localized: "yarıda kaldı") }
}
