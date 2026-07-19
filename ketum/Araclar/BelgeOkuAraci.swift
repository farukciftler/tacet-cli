//
//  BelgeOkuAraci.swift
//  ketum
//
//  Algı aracı (spec §7.3). Kullanıcının sohbete paylaştığı belgeyi okur
//  (Excel/PDF/Word/Metin) ve içeriğini modele özet olarak döndürür. Okuma → gri çip.
//

import Foundation
import FoundationModels

struct BelgeOkuAraci: KetumAraci {
    let name = "belge_oku"
    let description = "Reads the document in play — the one attached to the chat, or the file you just created. Call this IMMEDIATELY when the user asks about it ('summarize', 'what's in it', 'show it as a table', in any language); read before describing. Never say you need an attachment before calling this."

    weak var raporlayici: (any AracRaporlayici)?
    weak var baglam: BelgeBaglami?

    @Generable
    struct Arguments {
        @Guide(description: "Optional: the topic or focus the user is interested in. If empty, the whole document is read.")
        var odak: String?
    }

    func call(arguments: Arguments) async -> String {
        let ekli = await baglam?.calisilabilirBelge
        guard let ekli else {
            return await cipliCalis(ikon: "doc", calisiyorMetni: Yerel.belgeAraniyor) {
                AracSonucu(cipMetni: Yerel.paylasilanYok,
                           durum: .okundu,
                           modeleDonen: "no_document_attached (ask the user to attach a document first)")
            }
        }
        return await cipliCalis(ikon: ekli.bicim.ikon,
                                calisiyorMetni: Yerel.belgeOkunuyor(ekli.bicim.etiket),
                                hamGirdi: ekli.ad) {
            let motor = BelgeMotorlari.motor(ekli.bicim)
            let icerik = try motor.oku(url: ekli.url)
            // Tablolu belgede modele MARKDOWN tablo döner: model bunu neredeyse
            // olduğu gibi aktarabilir ve sohbette gerçek tablo olarak çizilir.
            // Düz `ozet` (borusuz, 5 satırda kesik) modelin tabloyu yeniden
            // kurmasını gerektiriyordu; küçük model bunu yapmayıp "gösterildi" diyordu.
            let govde = icerik.tablo?.markdownKirpik(enFazlaSatir: 30) ?? icerik.metin
            let kirpik = govde.count > 1500 ? String(govde.prefix(1500)) + "…" : govde
            // Kullanıcının belgesi gerçekten okundu → oturum kirlenir (mcp §5.6).
            // Yukarıdaki "ekli belge yok" yolu buraya girmez: hiçbir veriye
            // dokunulmadığı için oturumu kirletmesi yanlış olurdu.
            return await kirletEgerBasarili(AracSonucu(
                cipMetni: Yerel.belgeOkundu(ekli.bicim.etiket, ekli.ad),
                durum: .okundu,
                modeleDonen: kirpik.isEmpty ? "Belge boş görünüyor." : kirpik,
                hamCikti: govde
            ))
        }
    }
}
