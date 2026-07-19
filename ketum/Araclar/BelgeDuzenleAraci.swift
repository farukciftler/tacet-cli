//
//  BelgeDuzenleAraci.swift
//  ketum
//
//  Üretim aracı — düzenleme. Sohbete paylaşılan belgeyi okuyup, modelin verdiği
//  yeni içerikle yeni bir sürüm olarak yazar (okuma-yardımlı yeniden üretim).
//  Orijinal korunur; "… (düzenlendi)" adıyla yeni dosya üretilir. Yazma → yeşil çip.
//
//  Akış: model önce belge_oku ile içeriği alır, sonra düzenlenmiş govde/tablo ile
//  bu aracı çağırır. Excel için yeniTablo, prose belgeler için yeniGovde verilir.
//

import Foundation
import FoundationModels

struct BelgeDuzenleAraci: KetumAraci {
    let name = "belge_duzenle"
    let description = "Edits the document in play — the one attached to the chat, or the file you just created — by writing a new version. Call this when the user asks to change it ('add this', 'delete that row', 'change the title', in any language). First call belge_oku to get the content, then pass the FULL edited content as 'yeniIcerik' (markdown; a markdown table for Excel files)."

    weak var raporlayici: (any AracRaporlayici)?
    weak var baglam: BelgeBaglami?

    @Generable
    struct Arguments {
        @Guide(description: "The FULL edited content as markdown (the whole document, not just the changed part). For an Excel document write a markdown table (| … |); for a text document write plain markdown.")
        var yeniIcerik: String
        @Guide(description: "Optional new title.")
        var baslik: String?
    }

    func call(arguments: Arguments) async -> String {
        let ekli = await baglam?.calisilabilirBelge
        guard let ekli else {
            return await cipliCalis(ikon: "doc", calisiyorMetni: Yerel.belgeAraniyor) {
                AracSonucu(cipMetni: Yerel.duzenlenecekYok,
                           durum: .okundu,
                           modeleDonen: "no_document_attached (ask the user to attach a document first)")
            }
        }
        return await cipliCalis(ikon: ekli.bicim.ikon,
                                calisiyorMetni: Yerel.belgeDuzenleniyor(ekli.bicim.etiket),
                                hamGirdi: ekli.ad) {
            let motor = BelgeMotorlari.motor(ekli.bicim)
            let taban = ekli.url.deletingPathExtension().lastPathComponent
            // Excel ise markdown tablosunu yapılandırılmış tabloya çevir; değilse düz metin.
            let tablo = ekli.bicim.tabloYapisi ? Tablo.markdownDan(arguments.yeniIcerik) : nil
            let url = try motor.yaz(dosyaAdi: "\(taban) (düzenlendi)",
                                    baslik: arguments.baslik,
                                    govde: tablo == nil ? arguments.yeniIcerik : nil,
                                    tablo: tablo,
                                    klasor: BelgeBaglami.ciktiKlasoru())
            await baglam?.ciktiEklendi(url)
            // Kullanıcının belgesi okunup yeni sürüm yazıldı (mcp §5.6).
            return await kirletEgerBasarili(AracSonucu(
                cipMetni: Yerel.belgeDuzenlendi(ekli.bicim.etiket, url.lastPathComponent),
                durum: .yazildi,
                modeleDonen: "file_edited: \(url.lastPathComponent)",
                hamCikti: url.path,
                dosyaYolu: url.path
            ))
        }
    }
}
