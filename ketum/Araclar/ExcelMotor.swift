//
//  ExcelMotor.swift
//  ketum
//
//  .xlsx (OOXML SpreadsheetML) yazma/okuma. Saf Swift; harici paket ya da ağ yok.
//  Paketleme ZipDeposu ile (STORE zip). Hücreler inlineStr; sharedStrings kullanmaz.
//  Yazarken tamamen sayısal kolonlar için altına =SUM formülü ekler (spec §7.3.2).
//

import Foundation

struct ExcelMotor: BelgeMotoru {
    var bicim: BelgeBicimi { .xlsx }

    // MARK: - Yazma

    func yaz(dosyaAdi: String, baslik: String?, govde: String?, tablo: Tablo?, klasor: URL) throws -> URL {
        // Kaynağı tabloya indir: tablo varsa doğrudan, yoksa govde'yi tek sütuna çevir.
        let basliklar: [String]
        let satirlar: [[String]]
        if let t = tablo, !t.basliklar.isEmpty {
            basliklar = t.basliklar
            satirlar = t.satirlar.map { $0.hucreler }
        } else {
            let metin = govde ?? baslik ?? ""
            let dilimler = metin.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
            basliklar = [baslik ?? "Metin"]
            satirlar = dilimler.isEmpty ? [[""]] : dilimler.map { [$0] }
        }

        let sheet = sheetXml(basliklar: basliklar, satirlar: satirlar)

        let girisler: [ZipGiris] = [
            ZipGiris(ad: "[Content_Types].xml", veri: Data(contentTypesXml.utf8)),
            ZipGiris(ad: "_rels/.rels", veri: Data(relsXml.utf8)),
            ZipGiris(ad: "xl/workbook.xml", veri: Data(workbookXml.utf8)),
            ZipGiris(ad: "xl/_rels/workbook.xml.rels", veri: Data(workbookRelsXml.utf8)),
            ZipGiris(ad: "xl/styles.xml", veri: Data(stylesXml.utf8)),
            ZipGiris(ad: "xl/worksheets/sheet1.xml", veri: Data(sheet.utf8)),
        ]

        let url = hedefURL(dosyaAdi: dosyaAdi, klasor: klasor)
        try ZipDeposu.paketle(girisler).write(to: url)
        return url
    }

    // MARK: - Okuma

    func oku(url: URL) throws -> BelgeIcerik {
        let zip = try Data(contentsOf: url)
        let girisler = try ZipDeposu.ac(zip)
        guard let sheetData = girisler["xl/worksheets/sheet1.xml"] else {
            throw BelgeMotorHatasi.icerikYok
        }

        // Varsa sharedStrings tablosunu indeksle (t="s" için).
        var paylasilan: [String] = []
        if let ssData = girisler["xl/sharedStrings.xml"] {
            let ssAyristirici = PaylasilanAyristirici()
            let p = XMLParser(data: ssData)
            p.delegate = ssAyristirici
            p.parse()
            paylasilan = ssAyristirici.degerler
        }

        let ayristirici = SayfaAyristirici(paylasilan: paylasilan)
        let parser = XMLParser(data: sheetData)
        parser.delegate = ayristirici
        parser.parse()

        let hamSatirlar = ayristirici.satirlar
        guard let ilk = hamSatirlar.first else { throw BelgeMotorHatasi.icerikYok }

        let basliklar = ilk
        let govdeSatirlar = hamSatirlar.dropFirst().map { Satir(hucreler: $0) }
        let tablo = Tablo(basliklar: basliklar, satirlar: Array(govdeSatirlar))
        return BelgeIcerik(metin: tablo.ozet, tablo: tablo)
    }

    // MARK: - Worksheet üretimi

    private func sheetXml(basliklar: [String], satirlar: [[String]]) -> String {
        let sutunSayisi = basliklar.count
        // Sayısal kolonları tespit et: veri satırlarında o kolonun tüm dolu hücreleri Double.
        var sayisalKolon = [Bool](repeating: false, count: sutunSayisi)
        for k in 0..<sutunSayisi {
            var enAzBir = false
            var hepsiSayi = true
            for s in satirlar {
                guard k < s.count else { continue }
                let h = s[k].trimmingCharacters(in: .whitespaces)
                if h.isEmpty { continue }
                enAzBir = true
                if Double(h) == nil { hepsiSayi = false; break }
            }
            sayisalKolon[k] = enAzBir && hepsiSayi
        }
        let sayisalVar = sayisalKolon.contains(true)

        var govde = ""

        // 1. satır: başlıklar (hepsi inlineStr).
        govde += "<row r=\"1\">"
        for (k, b) in basliklar.enumerated() {
            govde += inlineHucre(ref: "\(kolonHarfi(k))1", metin: b)
        }
        govde += "</row>"

        // Veri satırları.
        for (idx, s) in satirlar.enumerated() {
            let r = idx + 2
            govde += "<row r=\"\(r)\">"
            for k in 0..<sutunSayisi {
                let ref = "\(kolonHarfi(k))\(r)"
                let deger = k < s.count ? s[k] : ""
                let kirp = deger.trimmingCharacters(in: .whitespaces)
                if !kirp.isEmpty, Double(kirp) != nil {
                    govde += "<c r=\"\(ref)\"><v>\(kirp)</v></c>"
                } else {
                    govde += inlineHucre(ref: ref, metin: deger)
                }
            }
            govde += "</row>"
        }

        // Özet (Toplam) satırı: en az bir sayısal kolon varsa.
        if sayisalVar, !satirlar.isEmpty {
            let ilkVeri = 2
            let sonVeri = satirlar.count + 1
            let toplamSatir = sonVeri + 1
            govde += "<row r=\"\(toplamSatir)\">"
            for k in 0..<sutunSayisi {
                let ref = "\(kolonHarfi(k))\(toplamSatir)"
                if k == 0 {
                    govde += inlineHucre(ref: ref, metin: "Toplam")
                } else if sayisalKolon[k] {
                    let harf = kolonHarfi(k)
                    govde += "<c r=\"\(ref)\"><f>SUM(\(harf)\(ilkVeri):\(harf)\(sonVeri))</f></c>"
                } else {
                    govde += inlineHucre(ref: ref, metin: "")
                }
            }
            govde += "</row>"
        }

        return """
        <?xml version="1.0" encoding="UTF-8" standalone="yes"?>
        <worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>\(govde)</sheetData></worksheet>
        """
    }

    private func inlineHucre(ref: String, metin: String) -> String {
        "<c r=\"\(ref)\" t=\"inlineStr\"><is><t xml:space=\"preserve\">\(xmlKac(metin))</t></is></c>"
    }

    // MARK: - Sabit parçalar

    private let contentTypesXml = """
    <?xml version="1.0" encoding="UTF-8" standalone="yes"?>
    <Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
    <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
    <Default Extension="xml" ContentType="application/xml"/>
    <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
    <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
    <Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
    </Types>
    """

    private let relsXml = """
    <?xml version="1.0" encoding="UTF-8" standalone="yes"?>
    <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
    <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
    </Relationships>
    """

    private let workbookXml = """
    <?xml version="1.0" encoding="UTF-8" standalone="yes"?>
    <workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
    <sheets><sheet name="Sayfa1" sheetId="1" r:id="rId1"/></sheets></workbook>
    """

    private let workbookRelsXml = """
    <?xml version="1.0" encoding="UTF-8" standalone="yes"?>
    <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
    <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
    <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
    </Relationships>
    """

    private let stylesXml = """
    <?xml version="1.0" encoding="UTF-8" standalone="yes"?>
    <styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
    <fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts>
    <fills count="1"><fill><patternFill patternType="none"/></fill></fills>
    <borders count="1"><border/></borders>
    <cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
    <cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs>
    </styleSheet>
    """
}

// MARK: - Yardımcılar

/// Sütun indisini (0 tabanlı) harfe çevirir: 0→A, 25→Z, 26→AA.
fileprivate func kolonHarfi(_ index: Int) -> String {
    var n = index
    var s = ""
    repeat {
        let r = n % 26
        s = String(UnicodeScalar(UInt8(65 + r))) + s
        n = n / 26 - 1
    } while n >= 0
    return s
}

/// XML özel karakterlerini kaçırır.
fileprivate func xmlKac(_ metin: String) -> String {
    var s = metin
    s = s.replacingOccurrences(of: "&", with: "&amp;")
    s = s.replacingOccurrences(of: "<", with: "&lt;")
    s = s.replacingOccurrences(of: ">", with: "&gt;")
    s = s.replacingOccurrences(of: "\"", with: "&quot;")
    s = s.replacingOccurrences(of: "'", with: "&apos;")
    return s
}

// MARK: - Ayrıştırıcılar

/// sheet1.xml içindeki satır/hücre değerlerini toplar.
fileprivate final class SayfaAyristirici: NSObject, XMLParserDelegate {
    let paylasilan: [String]
    var satirlar: [[String]] = []

    private var aktifSatir: [String] = []
    private var aktifHucre = ""
    private var hucreTuru: String?
    private var topluyor = false

    init(paylasilan: [String]) {
        self.paylasilan = paylasilan
    }

    func parser(_ parser: XMLParser, didStartElement elementName: String,
                namespaceURI: String?, qualifiedName qName: String?,
                attributes attributeDict: [String: String]) {
        switch elementName {
        case "row":
            aktifSatir = []
        case "c":
            hucreTuru = attributeDict["t"]
            aktifHucre = ""
        case "t", "v":
            topluyor = true
        default:
            break
        }
    }

    func parser(_ parser: XMLParser, foundCharacters string: String) {
        if topluyor { aktifHucre += string }
    }

    func parser(_ parser: XMLParser, didEndElement elementName: String,
                namespaceURI: String?, qualifiedName qName: String?) {
        switch elementName {
        case "t", "v":
            topluyor = false
        case "c":
            if hucreTuru == "s", let i = Int(aktifHucre.trimmingCharacters(in: .whitespaces)),
               i >= 0, i < paylasilan.count {
                aktifSatir.append(paylasilan[i])
            } else {
                aktifSatir.append(aktifHucre)
            }
        case "row":
            satirlar.append(aktifSatir)
        default:
            break
        }
    }
}

/// sharedStrings.xml içindeki <si> değerlerini toplar (t="s" hücreler için).
fileprivate final class PaylasilanAyristirici: NSObject, XMLParserDelegate {
    var degerler: [String] = []
    private var aktif = ""
    private var siIcinde = false
    private var topluyor = false

    func parser(_ parser: XMLParser, didStartElement elementName: String,
                namespaceURI: String?, qualifiedName qName: String?,
                attributes attributeDict: [String: String]) {
        switch elementName {
        case "si":
            siIcinde = true
            aktif = ""
        case "t":
            topluyor = true
        default:
            break
        }
    }

    func parser(_ parser: XMLParser, foundCharacters string: String) {
        if siIcinde, topluyor { aktif += string }
    }

    func parser(_ parser: XMLParser, didEndElement elementName: String,
                namespaceURI: String?, qualifiedName qName: String?) {
        switch elementName {
        case "t":
            topluyor = false
        case "si":
            degerler.append(aktif)
            siIcinde = false
        default:
            break
        }
    }
}
