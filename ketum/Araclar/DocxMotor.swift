//
//  DocxMotor.swift
//  ketum
//
//  WordprocessingML (.docx) yazma/okuma. Belge, OOXML zip paketidir; harici
//  paket ya da ağ olmadan ZipDeposu ile üretilir/okunur. Ağ yok.
//

import Foundation

/// XML özel karakterlerini kaçırır.
fileprivate func xmlKac(_ s: String) -> String {
    var r = s
    r = r.replacingOccurrences(of: "&", with: "&amp;")
    r = r.replacingOccurrences(of: "<", with: "&lt;")
    r = r.replacingOccurrences(of: ">", with: "&gt;")
    r = r.replacingOccurrences(of: "\"", with: "&quot;")
    r = r.replacingOccurrences(of: "'", with: "&apos;")
    return r
}

struct DocxMotor: BelgeMotoru {
    var bicim: BelgeBicimi { .docx }

    func yaz(dosyaAdi: String, baslik: String?, govde: String?, tablo: Tablo?, klasor: URL) throws -> URL {
        let url = hedefURL(dosyaAdi: dosyaAdi, klasor: klasor)

        var paragraflar = ""

        // baslik verilmişse ilk paragraf başlık (kalın) olsun.
        if let b = baslik, !b.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            paragraflar += baslikParagraf(b)
        }

        // govde markdown olabilir: satırlara böl, işle.
        if let g = govde {
            for satir in g.components(separatedBy: "\n") {
                let kirp = satir.trimmingCharacters(in: .whitespaces)
                if kirp.isEmpty { continue }        // boş satırları atla
                if kirp.hasPrefix("# ") {
                    let metin = String(kirp.dropFirst(2))
                    paragraflar += baslikParagraf(metin)
                } else {
                    paragraflar += normalParagraf(kirp)
                }
            }
        }

        // tablo verilmişse (nadir): her satırı "h1 | h2 | ..." biçiminde bir paragraf yap.
        if let t = tablo {
            if !t.basliklar.isEmpty {
                paragraflar += normalParagraf(t.basliklar.joined(separator: " | "))
            }
            for s in t.satirlar {
                paragraflar += normalParagraf(s.hucreler.joined(separator: " | "))
            }
        }

        // En az bir paragraf bulunsun (geçerli belge için).
        if paragraflar.isEmpty {
            paragraflar += normalParagraf("")
        }

        let documentXml = """
        <?xml version="1.0" encoding="UTF-8" standalone="yes"?>
        <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
        \(paragraflar)<w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
        </w:body></w:document>
        """

        let contentTypes = """
        <?xml version="1.0" encoding="UTF-8" standalone="yes"?>
        <Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
        <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
        <Default Extension="xml" ContentType="application/xml"/>
        <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
        </Types>
        """

        let rels = """
        <?xml version="1.0" encoding="UTF-8" standalone="yes"?>
        <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
        <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
        </Relationships>
        """

        let girisler = [
            ZipGiris(ad: "[Content_Types].xml", veri: Data(contentTypes.utf8)),
            ZipGiris(ad: "_rels/.rels", veri: Data(rels.utf8)),
            ZipGiris(ad: "word/document.xml", veri: Data(documentXml.utf8))
        ]

        let data = ZipDeposu.paketle(girisler)
        try data.write(to: url)
        return url
    }

    func oku(url: URL) throws -> BelgeIcerik {
        let zip = try Data(contentsOf: url)
        let parcalar = try ZipDeposu.ac(zip)
        guard let docData = parcalar["word/document.xml"] else {
            throw BelgeMotorHatasi.icerikYok
        }
        let ayirici = DocxAyirici()
        let metin = ayirici.ayristir(docData)
        return BelgeIcerik(metin: metin)
    }

    // MARK: - Paragraf üreticiler

    private func normalParagraf(_ metin: String) -> String {
        "<w:p><w:r><w:t xml:space=\"preserve\">\(xmlKac(metin))</w:t></w:r></w:p>\n"
    }

    private func baslikParagraf(_ metin: String) -> String {
        "<w:p><w:r><w:rPr><w:b/><w:sz w:val=\"28\"/></w:rPr><w:t xml:space=\"preserve\">\(xmlKac(metin))</w:t></w:r></w:p>\n"
    }
}

/// word/document.xml içindeki <w:t> metinlerini toplar, <w:p> sınırlarında ayırır.
fileprivate final class DocxAyirici: NSObject, XMLParserDelegate {
    private var paragraflar: [String] = []
    private var suanki = ""
    private var wtIcinde = false

    func ayristir(_ data: Data) -> String {
        let parser = XMLParser(data: data)
        parser.shouldProcessNamespaces = false
        parser.delegate = self
        parser.parse()
        return paragraflar.joined(separator: "\n")
    }

    func parser(_ parser: XMLParser, didStartElement elementName: String,
                namespaceURI: String?, qualifiedName qName: String?,
                attributes attributeDict: [String: String]) {
        if elementName == "w:p" {
            suanki = ""
        } else if elementName == "w:t" {
            wtIcinde = true
        }
    }

    func parser(_ parser: XMLParser, foundCharacters string: String) {
        if wtIcinde { suanki += string }
    }

    func parser(_ parser: XMLParser, didEndElement elementName: String,
                namespaceURI: String?, qualifiedName qName: String?) {
        if elementName == "w:t" {
            wtIcinde = false
        } else if elementName == "w:p" {
            paragraflar.append(suanki)
            suanki = ""
        }
    }
}
