//
//  PdfMotor.swift
//  ketum
//
//  PDF motoru: baslik + govde'yi A4 sayfalara çizer (UIGraphicsPDFRenderer),
//  okurken PDFKit ile metni çıkarır. Ağ yok; çizim ana aktörde.
//

import Foundation
import UIKit
import PDFKit

/// PDF biçimi için yazma/okuma motoru.
struct PdfMotor: BelgeMotoru {
    var bicim: BelgeBicimi { .pdf }

    // A4 ölçüleri (punto) ve kenar boşluğu.
    private let sayfaGenislik: CGFloat = 595
    private let sayfaYukseklik: CGFloat = 842
    private let kenar: CGFloat = 48

    func yaz(dosyaAdi: String, baslik: String?, govde: String?, tablo: Tablo?, klasor: URL) throws -> URL {
        let url = hedefURL(dosyaAdi: dosyaAdi, klasor: klasor)

        let sayfa = CGRect(x: 0, y: 0, width: sayfaGenislik, height: sayfaYukseklik)
        let icerikGenislik = sayfaGenislik - kenar * 2
        let altSinir = sayfaYukseklik - kenar

        // Gövde metnini topla: markdown/düz metin + varsa tablo markdown'ı.
        var parcalar: [String] = []
        if let govde, !govde.isEmpty { parcalar.append(govde) }
        if let tablo {
            let mt = tablo.markdown
            if !mt.isEmpty { parcalar.append(mt) }
        }
        let tamGovde = parcalar.joined(separator: "\n\n")
        let satirlar = tamGovde.isEmpty ? [] : tamGovde.components(separatedBy: "\n")

        let renderer = UIGraphicsPDFRenderer(bounds: sayfa)
        let data = renderer.pdfData { ctx in
            ctx.beginPage()
            var y = kenar

            // Üst başlık (varsa) — 20pt bold Helvetica.
            if let baslik, !baslik.isEmpty {
                let bFont = UIFont(name: "Helvetica-Bold", size: 20) ?? UIFont.boldSystemFont(ofSize: 20)
                let bNitelik: [NSAttributedString.Key: Any] = [.font: bFont, .foregroundColor: UIColor.black]
                let cizilen = ciz(
                    metin: baslik, nitelik: bNitelik, genislik: icerikGenislik,
                    y: &y, altSinir: altSinir, ctx: ctx, sayfa: sayfa
                )
                y += cizilen + 16
            }

            // Gövde satırları — basit markdown yorumu ile.
            for ham in satirlar {
                let (metin, nitelik, ekBosluk) = satirBicimle(ham, genislik: icerikGenislik)
                if metin.isEmpty {
                    // Boş satır: paragraf aralığı.
                    y += 8
                    if y > altSinir { ctx.beginPage(); y = kenar }
                    continue
                }
                let cizilen = ciz(
                    metin: metin, nitelik: nitelik, genislik: icerikGenislik,
                    y: &y, altSinir: altSinir, ctx: ctx, sayfa: sayfa
                )
                y += cizilen + ekBosluk
            }
        }

        try data.write(to: url)
        return url
    }

    /// Bir metin bloğunu sözcük kaydırmalı çizer; gerektiğinde yeni sayfa açar.
    /// Çizilen toplam yüksekliği döndürür (son sayfadaki kısım için `y` güncellenir).
    private func ciz(
        metin: String,
        nitelik: [NSAttributedString.Key: Any],
        genislik: CGFloat,
        y: inout CGFloat,
        altSinir: CGFloat,
        ctx: UIGraphicsPDFRendererContext,
        sayfa: CGRect
    ) -> CGFloat {
        let attr = NSAttributedString(string: metin, attributes: nitelik)
        let sinir = CGSize(width: genislik, height: .greatestFiniteMagnitude)
        let kutu = attr.boundingRect(with: sinir, options: [.usesLineFragmentOrigin, .usesFontLeading], context: nil)
        let yukseklik = ceil(kutu.height)

        // Bir bloğun tamamı sayfaya sığmıyorsa ve sayfa başındaysak yine de çiz;
        // değilse yeni sayfaya geç.
        if y + yukseklik > altSinir && y > kenar {
            ctx.beginPage()
            y = kenar
        }

        let hedef = CGRect(x: kenar, y: y, width: genislik, height: yukseklik)
        attr.draw(with: hedef, options: [.usesLineFragmentOrigin, .usesFontLeading], context: nil)
        return yukseklik
    }

    /// Basit markdown: "# " başlık (bold, büyük), "- " madde "• ", diğerleri normal.
    private func satirBicimle(_ ham: String, genislik: CGFloat) -> (String, [NSAttributedString.Key: Any], CGFloat) {
        let paragraf = NSMutableParagraphStyle()
        paragraf.lineSpacing = 12 * 0.4          // 12pt gövde, 1.4 satır aralığı
        paragraf.lineBreakMode = .byWordWrapping

        let govdeFont = UIFont(name: "Helvetica", size: 12) ?? UIFont.systemFont(ofSize: 12)

        if ham.hasPrefix("# ") {
            let metin = String(ham.dropFirst(2))
            let font = UIFont(name: "Helvetica-Bold", size: 16) ?? UIFont.boldSystemFont(ofSize: 16)
            let nitelik: [NSAttributedString.Key: Any] = [
                .font: font, .foregroundColor: UIColor.black, .paragraphStyle: paragraf
            ]
            return (metin, nitelik, 8)
        }

        if ham.hasPrefix("- ") {
            let metin = "• " + ham.dropFirst(2)
            let nitelik: [NSAttributedString.Key: Any] = [
                .font: govdeFont, .foregroundColor: UIColor.black, .paragraphStyle: paragraf
            ]
            return (metin, nitelik, 2)
        }

        // Tablo satırları ve düz metin: gerektiğinde monospace ile hizala.
        let font: UIFont = ham.hasPrefix("|")
            ? (UIFont(name: "Menlo", size: 10) ?? UIFont.monospacedSystemFont(ofSize: 10, weight: .regular))
            : govdeFont
        let nitelik: [NSAttributedString.Key: Any] = [
            .font: font, .foregroundColor: UIColor.black, .paragraphStyle: paragraf
        ]
        return (ham, nitelik, 2)
    }

    func oku(url: URL) throws -> BelgeIcerik {
        guard let belge = PDFDocument(url: url) else {
            throw BelgeMotorHatasi.icerikYok
        }
        let metin = belge.string
        if let metin, !metin.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return BelgeIcerik(metin: metin)
        }
        // Taranmış PDF: metin katmanı yok.
        return BelgeIcerik(metin: "PDF metni çıkarılamadı (taranmış olabilir).")
    }
}
