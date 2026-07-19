//
//  BelgeOlusturAraci.swift
//  ketum
//
//  Üretim aracı (spec §7.3, §7.3.2). Sohbet verisinden Excel/PDF/Word/Metin
//  dosyası üretir. Çıktı QuickLook önizleme + paylaşım + Dosyalar'a kayıt.
//  Yazma eylemi → yeşil çip. Ağ yok.
//

import Foundation
import FoundationModels

struct BelgeOlusturAraci: KetumAraci {
    let name = "belge_olustur"
    let description = "Creates an Excel/PDF/Word/Markdown file. Call this IMMEDIATELY when the user asks for a file, table, list, or report ('make an excel/pdf/word', in any language) — do not ask or narrate. Write markdown into 'icerik'; for a table write a markdown table (| … |). For device data (e.g. calendar) pass 'kaynakRef' instead of 'icerik'."

    weak var raporlayici: (any AracRaporlayici)?
    weak var baglam: BelgeBaglami?
    /// Büyük veri kanalı — kaynakRef ile toplu veri modelden geçmeden çekilir.
    weak var veriDeposu: VeriDeposu?

    @Generable
    struct Arguments {
        @Guide(description: "Dosya biçimi: 'excel', 'pdf', 'word', 'markdown' ya da 'metin'.")
        var bicim: String
        @Guide(description: "Uzantısız dosya adı, ör. 'temmuz-toplantilari'.")
        var dosyaAdi: String
        @Guide(description: "Belge başlığı (isteğe bağlı).")
        var baslik: String?
        @Guide(description: "Belge içeriği MARKDOWN olarak. Tablo gerekiyorsa markdown tablosu yaz: | Başlık1 | Başlık2 | satırı, altına | --- | --- |, sonra veri satırları. Excel bu tablodan üretilir.")
        var icerik: String?
        @Guide(description: "Başka bir aracın (ör. takvim) verdiği veri referansı. Verilirse tüm veri depodan çekilir; icerik'i doldurma.")
        var kaynakRef: String?
    }

    func call(arguments: Arguments) async -> String {
        let bicim = BelgeBicimi(kullaniciMetni: arguments.bicim)
        let girdi = "biçim: \(bicim.etiket), ad: \(arguments.dosyaAdi)"
            + (arguments.kaynakRef.map { ", ref: \($0)" } ?? "")
        return await cipliCalis(ikon: bicim.ikon,
                                calisiyorMetni: Yerel.belgeOlusturuluyor(bicim.etiket),
                                hamGirdi: girdi) {
            // Toplu veri kanalı: referans varsa tabloyu depodan çek (model bağlamından değil).
            var tablo: Tablo?
            var govde: String? = arguments.icerik
            if let ref = arguments.kaynakRef, let depoTablo = await veriDeposu?.al(ref) {
                tablo = depoTablo
                govde = nil
            } else if bicim.tabloYapisi, let ic = arguments.icerik,
                      let ayrilan = Tablo.markdownDan(ic) {
                // Excel isteniyor ve içerikte markdown tablo var → yapılandırılmış tabloya çevir.
                tablo = ayrilan
                govde = nil
            }
            let motor = BelgeMotorlari.motor(bicim)
            let url = try motor.yaz(dosyaAdi: arguments.dosyaAdi,
                                    baslik: arguments.baslik,
                                    govde: govde,
                                    tablo: tablo,
                                    klasor: BelgeBaglami.ciktiKlasoru())
            await baglam?.ciktiEklendi(url)
            return AracSonucu(
                cipMetni: Yerel.belgeOlusturuldu(bicim.etiket, url.lastPathComponent),
                durum: .yazildi,
                // Modele yalnızca olgu döner; UI yönergesi (önizle/paylaş) yazma — model papağanlar.
                modeleDonen: "file_created (\(bicim.etiket)): \(url.lastPathComponent)",
                hamCikti: url.path,
                dosyaYolu: url.path
            )
        }
    }
}
