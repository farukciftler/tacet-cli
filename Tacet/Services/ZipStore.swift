//
//  ZipDeposu.swift
//  Tacet
//
//  Saf Swift zip paketleyici/açıcı. .xlsx ve .docx OOXML zip'idir; harici paket
//  ya da ağ kullanmadan cihazda üretmek/okumak için minimal ZIP uygulaması.
//  Yazma: STORE (sıkıştırmasız) — geçerli OOXML. Okuma: STORE + DEFLATE
//  (Apple Compression, ham deflate) — kullanıcı dosyaları çoğu zaman deflate'tir.
//

import Foundation
import Compression

struct ZipEntry {
    let name: String     // zip içi yol, ör. "xl/workbook.xml"
    let data: Data
}

enum ZipError: LocalizedError {
    case malformed(String)
    var errorDescription: String? { "Zip okunamadı" }
}

enum ZipStore {

    // MARK: - Yazma (STORE)

    static func package(_ girisler: [ZipEntry]) -> Data {
        var body = Data()          // yerel başlıklar + veri
        var central = Data()         // merkezi dizin
        var counter = 0

        for g in girisler {
            let adBytes = Array(g.name.utf8)
            let crc = crc32(g.data)
            let size = UInt32(g.data.count)
            let offset = UInt32(body.count)

            // Local file header
            var local = Data()
            local.le32(0x04034b50)          // imza
            local.le16(20)                  // gerekli sürüm
            local.le16(0)                   // bayrak
            local.le16(0)                   // yöntem: STORE
            local.le16(0); local.le16(0)    // zaman/tarih
            local.le32(crc)
            local.le32(size)               // sıkıştırılmış
            local.le32(size)               // sıkıştırılmamış
            local.le16(UInt16(adBytes.count))
            local.le16(0)                   // extra
            local.append(contentsOf: adBytes)
            body.append(local)
            body.append(g.data)

            // Merkezi dizin kaydı
            central.le32(0x02014b50)
            central.le16(20); central.le16(20)
            central.le16(0); central.le16(0)
            central.le16(0); central.le16(0)
            central.le32(crc)
            central.le32(size); central.le32(size)
            central.le16(UInt16(adBytes.count))
            central.le16(0); central.le16(0)  // extra, yorum
            central.le16(0); central.le16(0)  // disk, iç öznitelik
            central.le32(0)                  // dış öznitelik
            central.le32(offset)              // yerel başlık ofseti
            central.append(contentsOf: adBytes)
            counter += 1
        }

        let merkezOfset = UInt32(body.count)
        let merkezBoyut = UInt32(central.count)

        var outcome = body
        outcome.append(central)
        // Merkezi dizin sonu kaydı (EOCD)
        outcome.le32(0x06054b50)
        outcome.le16(0); outcome.le16(0)
        outcome.le16(UInt16(counter)); outcome.le16(UInt16(counter))
        outcome.le32(merkezBoyut)
        outcome.le32(merkezOfset)
        outcome.le16(0)                       // yorum uzunluğu
        return outcome
    }

    // MARK: - Okuma (STORE + DEFLATE)

    /// Tek parçanın açılmış üst sınırı — beyan edilen boyuta güvenip 4 GB
    /// ayırmayalım (zip bombası / bozuk alan).
    /// 256 MB tek bir telefon için cömertti; hiçbir docx/xlsx parçası bu
    /// büyüklüğe meşru olarak ulaşmaz.
    private static let enBuyukParca = 64 * 1024 * 1024
    /// Tüm arşivin açılmış üst sınırı.
    private static let enBuyukToplam = 512 * 1024 * 1024

    /// Zip'i açar. GİRDİ KULLANICIDAN GELİR: her uzunluk alanı yalan söyleyebilir.
    /// Bu yüzden hiçbir dilim/okuma sınır denetimsiz yapılmaz; bozuk dosyada
    /// çökmek yerine `ZipHatasi.bozuk` fırlatılır.
    static func open(_ zip: Data) throws -> [String: Data] {
        let bytes = [UInt8](zip)
        guard let eocd = eocdBul(bytes) else { throw ZipError.malformed("EOCD yok") }
        let kayitSayisi = try need16(bytes, eocd + 10, "EOCD kayıt sayısı")
        var offset = try need32(bytes, eocd + 16, "EOCD merkez ofseti")
        guard offset <= bytes.count else { throw ZipError.malformed("merkez ofseti dosya dışında") }

        var outcome: [String: Data] = [:]
        var toplamAcilan = 0

        for _ in 0..<kayitSayisi {
            guard bytes.count - offset >= 46 else { throw ZipError.malformed("merkezi dizin kaydı kesik") }
            let signature = try need32(bytes, offset, "merkezi dizin imzası")
            guard signature == 0x02014b50 else { throw ZipError.malformed("merkezi dizin kaydı") }

            let yontem = try need16(bytes, offset + 10, "sıkıştırma yöntemi")
            let sikBoyut = try need32(bytes, offset + 20, "sıkıştırılmış boyut")
            let hamBoyut = try need32(bytes, offset + 24, "ham boyut")
            let adLen = try need16(bytes, offset + 28, "ad uzunluğu")
            let extraLen = try need16(bytes, offset + 30, "extra uzunluğu")
            let yorumLen = try need16(bytes, offset + 32, "yorum uzunluğu")
            let yerelOfset = try need32(bytes, offset + 42, "yerel başlık ofseti")

            // Kaydın TAMAMI dosyanın içinde mi? adLen yalan söylerse ad dilimi taşardı.
            let kayitBoyut = 46 + adLen + extraLen + yorumLen
            guard bytes.count - offset >= kayitBoyut else { throw ZipError.malformed("kayıt sınırı") }
            let name = String(decoding: bytes[(offset + 46)..<(offset + 46 + adLen)], as: UTF8.self)
            let sonrakiOfset = offset + kayitBoyut

            // Find the data start from the local header.
            guard bytes.count - yerelOfset >= 30 else { throw ZipError.malformed("yerel başlık sınırı") }
            let yerelImza = try need32(bytes, yerelOfset, "yerel başlık imzası")
            guard yerelImza == 0x04034b50 else { throw ZipError.malformed("yerel başlık") }
            let yAdLen = try need16(bytes, yerelOfset + 26, "yerel ad uzunluğu")
            let yExtraLen = try need16(bytes, yerelOfset + 28, "yerel extra uzunluğu")
            let veriBas = yerelOfset + 30 + yAdLen + yExtraLen
            // Çıkarma ile karşılaştır: toplama taşmasına yer bırakma.
            guard veriBas <= bytes.count, bytes.count - veriBas >= sikBoyut else {
                throw ZipError.malformed("veri sınırı")
            }
            guard hamBoyut <= enBuyukParca else { throw ZipError.malformed("parça çok büyük: \(name)") }

            let cozulen: Data
            switch yontem {
            case 0:
                cozulen = Data(bytes[veriBas..<(veriBas + sikBoyut)])
            case 8:
                if sikBoyut == 0 {
                    cozulen = Data()
                } else {
                    let raw = Data(bytes[veriBas..<(veriBas + sikBoyut)])
                    guard let c = inflate(raw, hamBoyut: hamBoyut) else {
                        throw ZipError.malformed("deflate çözülemedi: \(name)")
                    }
                    cozulen = c
                }
            default:
                // Desteklenmeyen yöntem: boş veri koymak sessiz yalan olur
                // (çağıran "içerik yok" yerine "boş belge" görür). Parçayı atla.
                offset = sonrakiOfset
                continue
            }

            toplamAcilan += cozulen.count
            guard toplamAcilan <= enBuyukToplam else { throw ZipError.malformed("açılan içerik çok büyük") }
            outcome[name] = cozulen
            offset = sonrakiOfset
        }
        return outcome
    }

    private static func eocdBul(_ b: [UInt8]) -> Int? {
        guard b.count >= 22 else { return nil }
        var i = b.count - 22
        let lower = max(0, b.count - 22 - 65_536)
        while i >= lower {
            // İmza yetmez: yorum uzunluğu dosya sonuyla tutmalı (rastgele eşleşme).
            if read32(b, i) == 0x06054b50,
               let yorumLen = read16(b, i + 20), i + 22 + Int(yorumLen) <= b.count {
                return i
            }
            i -= 1
        }
        return nil
    }

    /// Ham DEFLATE çözme (Apple Compression, COMPRESSION_ZLIB = ham deflate).
    /// `hamBoyut` 0 ise (bazı yazıcılar boyutu yalnız data descriptor'a koyar)
    /// tampon büyütülerek yeniden denenir.
    private static func inflate(_ data: Data, hamBoyut: Int) -> Data? {
        guard !data.isEmpty else { return Data() }
        // BEYAN EDİLEN BOYUT AYIRMA EMRİ DEĞİLDİR. 64 baytlık çöp veri
        // `hamBoyut = 256 MB` bildirdiğinde eskiden 256 MB'lık tampon
        // ayrılıyordu (ölçüldü) — çökme değil, jetsam yolu. Sıkıştırılmış
        // boyuttan türeyen makul bir tahminle başlanır; yetmezse döngü zaten
        // büyütür ve tavan yine beyan edilen boyuttur.
        let cap = hamBoyut > 0 ? min(hamBoyut, enBuyukParca) : enBuyukParca
        var kapasite = min(cap, max(data.count * 8, 64 * 1024))
        for _ in 0..<8 {
            guard kapasite > 0, kapasite <= enBuyukParca else { return nil }
            var output = Data(count: kapasite)
            let ayrilan = kapasite
            let yazilan = output.withUnsafeMutableBytes { (d: UnsafeMutableRawBufferPointer) -> Int in
                data.withUnsafeBytes { (s: UnsafeRawBufferPointer) -> Int in
                    guard let db = d.baseAddress?.assumingMemoryBound(to: UInt8.self),
                          let sb = s.baseAddress?.assumingMemoryBound(to: UInt8.self) else { return 0 }
                    return compression_decode_buffer(db, ayrilan, sb, data.count, nil, COMPRESSION_ZLIB)
                }
            }
            guard yazilan > 0 else { return nil }
            // Tamponun sonuna dayanmak "kesilmiş olabilir" demektir → büyüt.
            // Tavana dayandıysak büyüyecek yer yok; elde olan sonuçtur.
            if yazilan < ayrilan || ayrilan >= cap { return output.prefix(yazilan) }
            kapasite = min(kapasite * 4, cap)
        }
        return nil
    }

    // MARK: - Küçük okuyucular (sınır denetimli)

    private static func need16(_ b: [UInt8], _ i: Int, _ cause: String) throws -> Int {
        guard let v = read16(b, i) else { throw ZipError.malformed(cause) }
        return Int(v)
    }
    private static func need32(_ b: [UInt8], _ i: Int, _ cause: String) throws -> Int {
        guard let v = read32(b, i) else { throw ZipError.malformed(cause) }
        return Int(v)
    }

    private static func read16(_ b: [UInt8], _ i: Int) -> UInt16? {
        guard i >= 0, b.count - i >= 2 else { return nil }
        return UInt16(b[i]) | (UInt16(b[i + 1]) << 8)
    }
    private static func read32(_ b: [UInt8], _ i: Int) -> UInt32? {
        guard i >= 0, b.count - i >= 4 else { return nil }
        return UInt32(b[i]) | (UInt32(b[i + 1]) << 8) | (UInt32(b[i + 2]) << 16) | (UInt32(b[i + 3]) << 24)
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
