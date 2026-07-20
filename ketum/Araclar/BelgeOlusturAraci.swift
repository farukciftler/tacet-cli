//
//  BelgeOlusturAraci.swift
//  ketum
//
//  Üretim aracı (spec §7.3, §7.3.2; kod-spec §4). Sohbet verisinden
//  Excel/PDF/Word/Metin dosyası ya da tek dosyalık HTML sayfası üretir.
//  HTML çıktı SayfaDogrulayici ile ekran dışı doğrulanır; geçmeyen sayfa
//  sunulmaz, dosya silinir. Çıktı QuickLook önizleme + paylaşım +
//  Dosyalar'a kayıt. Yazma eylemi → yeşil çip. Ağ yok.
//

import Foundation
import FoundationModels

struct BelgeOlusturAraci: KetumAraci {
    let name = "belge_olustur"
    let description = "Creates an Excel/PDF/Word/Markdown file or an HTML page. Call this IMMEDIATELY when the user asks for a file, table, list, report or web page ('make an excel/pdf/word/site', in any language) — do not ask or narrate. Write markdown into 'icerik'; for a table write a markdown table (| … |). For device data (e.g. calendar) pass 'kaynakRef' instead of 'icerik'."

    weak var raporlayici: (any AracRaporlayici)?
    weak var baglam: BelgeBaglami?
    /// Büyük veri kanalı — kaynakRef ile toplu veri modelden geçmeden çekilir.
    weak var veriDeposu: VeriDeposu?

    /// Biçim artık serbest metin DEĞİL. Eskiden `BelgeBicimi(kullaniciMetni:)`
    /// bulanık `.contains` ile çözüyordu ve eşleşmeyen her değer sessizce `.txt`
    /// oluyordu: kullanıcı "excel" isteyip .txt alıyordu. Enum ile kısıtlı
    /// çözümleme (constrained decoding) geçersiz değeri ÜRETİLEMEZ yapar.
    @Generable
    enum Bicim: String, Equatable, CaseIterable {
        case excel
        case pdf
        case word
        case markdown
        case metin
        case html

        var belgeBicimi: BelgeBicimi {
            switch self {
            case .excel:    return .xlsx
            case .pdf:      return .pdf
            case .word:     return .docx
            case .markdown: return .md
            case .metin:    return .txt
            case .html:     return .html
            }
        }
    }

    @Generable
    struct Arguments {
        @Guide(description: "File format: 'excel' (spreadsheet), 'pdf', 'word', 'markdown', 'metin' (plain text) or 'html' (single-page website).")
        var bicim: Bicim
        @Guide(description: "File name without extension, e.g. 'july-meetings'.")
        var dosyaAdi: String
        @Guide(description: "Document title (optional).")
        var baslik: String?
        @Guide(description: "Document body as MARKDOWN. If a table is needed, write a markdown table: a | Header1 | Header2 | row, then | --- | --- |, then the data rows. Excel files are built from that table.")
        var icerik: String?
        @Guide(description: "Data reference returned by another tool (e.g. the calendar tool). If given, the full data is pulled from the store — leave 'icerik' empty.")
        var kaynakRef: String?
    }

    func call(arguments: Arguments) async -> String {
        let bicim = arguments.bicim.belgeBicimi
        let girdi = "biçim: \(bicim.etiket), ad: \(arguments.dosyaAdi)"
            + (arguments.kaynakRef.map { ", ref: \($0)" } ?? "")
        return await cipliCalis(ikon: bicim.ikon,
                                calisiyorMetni: Yerel.belgeOlusturuluyor(bicim.etiket),
                                hamGirdi: girdi) {
            // Toplu veri kanalı: referans varsa tabloyu depodan çek (model bağlamından değil).
            var tablo: Tablo?
            var govde: String? = arguments.icerik
            let refHam = arguments.kaynakRef?.trimmingCharacters(in: .whitespacesAndNewlines)
            if let ref = refHam, !ref.isEmpty {
                // REF VARSA REF BAĞLAYICIDIR (P0-2). Çözülemeyen ref eskiden
                // sessizce `icerik`e düşüyordu; `icerik` de boş olduğu için
                // BOŞ dosya yazılıp "file_created" raporlanıyordu — kullanıcı
                // dolu sandığı dosyayı taşıyordu. En tehlikeli hata sınıfı
                // buydu. Artık dosya HİÇ yazılmaz, açık hata döner.
                if let depoTablo = await veriDeposu?.al(ref) {
                    tablo = depoTablo
                    govde = nil
                } else if let depoMetin = await veriDeposu?.alMetin(ref) {
                    // belge_oku'nun offload ettiği düz gövde (P2-6). Excel
                    // isteniyorsa içindeki markdown tabloyu yapılandır.
                    if bicim.tabloYapisi, let ayrilan = Tablo.markdownDan(depoMetin) {
                        tablo = ayrilan
                        govde = nil
                    } else {
                        tablo = nil
                        govde = depoMetin
                    }
                } else {
                    let eldekiler = await veriDeposu?.refAnahtarlari ?? []
                    let liste = eldekiler.isEmpty ? "none" : eldekiler.joined(separator: ",")
                    return AracSonucu(
                        cipMetni: Self.kaynakBulunamadi,
                        durum: .basarisiz(Self.kaynakBulunamadi),
                        // Yalnız olgu: hangi ref istendi, elde hangileri var,
                        // ne oldu. İmperatif yönerge yok (P2-4).
                        modeleDonen: "unknown_data_ref: \"\(ref)\" (available: \(liste)); no file was created",
                        hamCikti: "kaynakRef=\(ref)"
                    )
                }
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
            // HTML doğrulaması (kod-spec §4.3): sayfa ekran dışı WKWebView'de
            // yüklenir; yüklenmeyen ya da betik hatası veren sayfa kullanıcıya
            // SUNULMAZ — dosya silinir, modele kısa neden döner (beceri kılavuzu
            // modele içeriği sadeleştirip BİR kez daha denemeyi söyler).
            if bicim == .html {
                let dogrulama = await SayfaDogrulayici.dogrula(url: url)
                if !dogrulama.gecti {
                    try? FileManager.default.removeItem(at: url)
                    let neden = dogrulama.neden ?? Yerel.sayfaDogrulanamadi
                    return AracSonucu(
                        cipMetni: Yerel.sayfaDogrulanamadi,
                        durum: .basarisiz(neden),
                        // Yapısal kanal `durum: .basarisiz(neden)`; modele yalnız
                        // olgu döner. "Simplify … try ONCE more" gibi imperatif
                        // yönerge kaldırıldı (P2-4): araç modele emir vermez,
                        // yeniden deneme yönergesi beceri dosyasının işidir.
                        modeleDonen: "verification_failed: the page did not load cleanly; the file was discarded",
                        hamCikti: neden
                    )
                }
            }
            await baglam?.ciktiEklendi(url)
            // Cihazda bir dosya oluştu; içeriği kullanıcının verisidir (mcp §5.6).
            return await kirletEgerBasarili(AracSonucu(
                cipMetni: Yerel.belgeOlusturuldu(bicim.etiket, url.lastPathComponent),
                durum: .yazildi,
                // Modele yalnızca olgu döner; UI yönergesi (önizle/paylaş) yazma — model papağanlar.
                modeleDonen: "file_created (\(bicim.etiket)): \(url.lastPathComponent)",
                hamCikti: url.path,
                dosyaYolu: url.path
            ))
        }
    }

    // Not: Yerel.swift bu fazda başka bir ajanın dosyası; yeni anahtar burada
    // String(localized:) ile tanımlı — String Catalog'a otomatik girer.
    static var kaynakBulunamadi: String { String(localized: "Kaynak veri bulunamadı") }
}
