//
//  ZipDeposu.swift
//  ketum
//
//  Saf Swift zip paketleyici/açıcı. .xlsx ve .docx OOXML zip'idir; harici paket
//  ya da ağ kullanmadan cihazda üretmek/okumak için minimal ZIP uygulaması.
//  Yazma: STORE (sıkıştırmasız) — geçerli OOXML. Okuma: STORE + DEFLATE
//  (Apple Compression, ham deflate) — kullanıcı dosyaları çoğu zaman deflate'tir.
//

import Foundation
import Compression

struct ZipGiris {
    let ad: String     // zip içi yol, ör. "xl/workbook.xml"
    let veri: Data
}

enum ZipHatasi: LocalizedError {
    case bozuk(String)
    var errorDescription: String? { "Zip okunamadı" }
}

enum ZipDeposu {

    // MARK: - Yazma (STORE)

    static func paketle(_ girisler: [ZipGiris]) -> Data {
        var govde = Data()          // yerel başlıklar + veri
        var merkez = Data()         // merkezi dizin
        var sayac = 0

        for g in girisler {
            let adBytes = Array(g.ad.utf8)
            let crc = crc32(g.veri)
            let boyut = UInt32(g.veri.count)
            let ofset = UInt32(govde.count)

            // Yerel dosya başlığı
            var yerel = Data()
            yerel.le32(0x04034b50)          // imza
            yerel.le16(20)                  // gerekli sürüm
            yerel.le16(0)                   // bayrak
            yerel.le16(0)                   // yöntem: STORE
            yerel.le16(0); yerel.le16(0)    // zaman/tarih
            yerel.le32(crc)
            yerel.le32(boyut)               // sıkıştırılmış
            yerel.le32(boyut)               // sıkıştırılmamış
            yerel.le16(UInt16(adBytes.count))
            yerel.le16(0)                   // extra
            yerel.append(contentsOf: adBytes)
            govde.append(yerel)
            govde.append(g.veri)

            // Merkezi dizin kaydı
            merkez.le32(0x02014b50)
            merkez.le16(20); merkez.le16(20)
            merkez.le16(0); merkez.le16(0)
            merkez.le16(0); merkez.le16(0)
            merkez.le32(crc)
            merkez.le32(boyut); merkez.le32(boyut)
            merkez.le16(UInt16(adBytes.count))
            merkez.le16(0); merkez.le16(0)  // extra, yorum
            merkez.le16(0); merkez.le16(0)  // disk, iç öznitelik
            merkez.le32(0)                  // dış öznitelik
            merkez.le32(ofset)              // yerel başlık ofseti
            merkez.append(contentsOf: adBytes)
            sayac += 1
        }

        let merkezOfset = UInt32(govde.count)
        let merkezBoyut = UInt32(merkez.count)

        var sonuc = govde
        sonuc.append(merkez)
        // Merkezi dizin sonu kaydı (EOCD)
        sonuc.le32(0x06054b50)
        sonuc.le16(0); sonuc.le16(0)
        sonuc.le16(UInt16(sayac)); sonuc.le16(UInt16(sayac))
        sonuc.le32(merkezBoyut)
        sonuc.le32(merkezOfset)
        sonuc.le16(0)                       // yorum uzunluğu
        return sonuc
    }

    // MARK: - Okuma (STORE + DEFLATE)

    static func ac(_ zip: Data) throws -> [String: Data] {
        let bytes = [UInt8](zip)
        guard let eocd = eocdBul(bytes) else { throw ZipHatasi.bozuk("EOCD yok") }
        let kayitSayisi = Int(oku16(bytes, eocd + 10))
        var ofset = Int(oku32(bytes, eocd + 16))   // merkezi dizin başlangıcı

        var sonuc: [String: Data] = [:]
        for _ in 0..<kayitSayisi {
            guard ofset + 46 <= bytes.count, oku32(bytes, ofset) == 0x02014b50 else {
                throw ZipHatasi.bozuk("merkezi dizin kaydı")
            }
            let yontem = oku16(bytes, ofset + 10)
            let sikBoyut = Int(oku32(bytes, ofset + 20))
            let hamBoyut = Int(oku32(bytes, ofset + 24))
            let adLen = Int(oku16(bytes, ofset + 28))
            let extraLen = Int(oku16(bytes, ofset + 30))
            let yorumLen = Int(oku16(bytes, ofset + 32))
            let yerelOfset = Int(oku32(bytes, ofset + 42))
            let ad = String(decoding: bytes[(ofset + 46)..<(ofset + 46 + adLen)], as: UTF8.self)

            // Yerel başlıktan veri başlangıcını bul
            guard yerelOfset + 30 <= bytes.count, oku32(bytes, yerelOfset) == 0x04034b50 else {
                throw ZipHatasi.bozuk("yerel başlık")
            }
            let yAdLen = Int(oku16(bytes, yerelOfset + 26))
            let yExtraLen = Int(oku16(bytes, yerelOfset + 28))
            let veriBas = yerelOfset + 30 + yAdLen + yExtraLen
            guard veriBas + sikBoyut <= bytes.count else { throw ZipHatasi.bozuk("veri sınırı") }
            let ham = Data(bytes[veriBas..<(veriBas + sikBoyut)])

            let cozulen: Data
            if yontem == 0 {
                cozulen = ham
            } else if yontem == 8 {
                cozulen = inflate(ham, hamBoyut: hamBoyut) ?? Data()
            } else {
                cozulen = Data()
            }
            sonuc[ad] = cozulen
            ofset += 46 + adLen + extraLen + yorumLen
        }
        return sonuc
    }

    private static func eocdBul(_ b: [UInt8]) -> Int? {
        guard b.count >= 22 else { return nil }
        var i = b.count - 22
        let alt = max(0, b.count - 22 - 65_536)
        while i >= alt {
            if oku32(b, i) == 0x06054b50 { return i }
            i -= 1
        }
        return nil
    }

    /// Ham DEFLATE çözme (Apple Compression, COMPRESSION_ZLIB = ham deflate).
    private static func inflate(_ data: Data, hamBoyut: Int) -> Data? {
        let kapasite = max(hamBoyut, 1)
        var cikti = Data(count: kapasite)
        let yazilan = cikti.withUnsafeMutableBytes { (d: UnsafeMutableRawBufferPointer) -> Int in
            data.withUnsafeBytes { (s: UnsafeRawBufferPointer) -> Int in
                guard let db = d.baseAddress?.assumingMemoryBound(to: UInt8.self),
                      let sb = s.baseAddress?.assumingMemoryBound(to: UInt8.self) else { return 0 }
                return compression_decode_buffer(db, kapasite, sb, data.count, nil, COMPRESSION_ZLIB)
            }
        }
        guard yazilan > 0 else { return nil }
        return cikti.prefix(yazilan)
    }

    // MARK: - Küçük okuyucular

    private static func oku16(_ b: [UInt8], _ i: Int) -> UInt16 {
        UInt16(b[i]) | (UInt16(b[i + 1]) << 8)
    }
    private static func oku32(_ b: [UInt8], _ i: Int) -> UInt32 {
        UInt32(b[i]) | (UInt32(b[i + 1]) << 8) | (UInt32(b[i + 2]) << 16) | (UInt32(b[i + 3]) << 24)
    }

    // MARK: - CRC32

    private static let crcTablo: [UInt32] = {
        (0..<256).map { i -> UInt32 in
            var c = UInt32(i)
            for _ in 0..<8 { c = (c & 1) != 0 ? (0xEDB88320 ^ (c >> 1)) : (c >> 1) }
            return c
        }
    }()

    static func crc32(_ data: Data) -> UInt32 {
        var crc: UInt32 = 0xFFFFFFFF
        for b in data { crc = crcTablo[Int((crc ^ UInt32(b)) & 0xFF)] ^ (crc >> 8) }
        return crc ^ 0xFFFFFFFF
    }
}

private extension Data {
    mutating func le16(_ v: UInt16) {
        append(UInt8(v & 0xFF)); append(UInt8((v >> 8) & 0xFF))
    }
    mutating func le32(_ v: UInt32) {
        append(UInt8(v & 0xFF)); append(UInt8((v >> 8) & 0xFF))
        append(UInt8((v >> 16) & 0xFF)); append(UInt8((v >> 24) & 0xFF))
    }
}
