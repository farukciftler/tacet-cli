//
//  ZipStore.swift
//  Tacet
//
//  Pure-Swift zip packer/unpacker. .xlsx and .docx are OOXML zips; a minimal ZIP
//  implementation so they can be produced/read on device without an external
//  package or the network.
//  Writing: STORE (uncompressed) — valid OOXML. Reading: STORE + DEFLATE
//  (Apple Compression, raw deflate) — user files are deflate most of the time.
//

import Foundation
import Compression

struct ZipEntry {
    let name: String     // path inside the zip, e.g. "xl/workbook.xml"
    let data: Data
}

enum ZipError: LocalizedError {
    case malformed(String)
    /// USER-FACING TEXT. It goes through `String(localized:)` so the nine
    /// localisations each show their own; the xcstrings key is the English string.
    var errorDescription: String? { String(localized: "Couldn’t read the zip") }
}

enum ZipStore {

    // MARK: - Writing (STORE)

    static func package(_ entries: [ZipEntry]) -> Data {
        var body = Data()            // local headers + data
        var central = Data()         // central directory
        var counter = 0

        for entry in entries {
            let nameBytes = Array(entry.name.utf8)
            let crc = crc32(entry.data)
            let size = UInt32(entry.data.count)
            let offset = UInt32(body.count)

            // Local file header
            var local = Data()
            local.le32(0x04034b50)          // signature
            local.le16(20)                  // version needed
            local.le16(0)                   // flag
            local.le16(0)                   // method: STORE
            local.le16(0); local.le16(0)    // time/date
            local.le32(crc)
            local.le32(size)               // compressed
            local.le32(size)               // uncompressed
            local.le16(UInt16(nameBytes.count))
            local.le16(0)                   // extra
            local.append(contentsOf: nameBytes)
            body.append(local)
            body.append(entry.data)

            // Central directory record
            central.le32(0x02014b50)
            central.le16(20); central.le16(20)
            central.le16(0); central.le16(0)
            central.le16(0); central.le16(0)
            central.le32(crc)
            central.le32(size); central.le32(size)
            central.le16(UInt16(nameBytes.count))
            central.le16(0); central.le16(0)  // extra, comment
            central.le16(0); central.le16(0)  // disk, internal attribute
            central.le32(0)                  // external attribute
            central.le32(offset)              // local header offset
            central.append(contentsOf: nameBytes)
            counter += 1
        }

        let centralOffset = UInt32(body.count)
        let centralSize = UInt32(central.count)

        var outcome = body
        outcome.append(central)
        // End of central directory record (EOCD)
        outcome.le32(0x06054b50)
        outcome.le16(0); outcome.le16(0)
        outcome.le16(UInt16(counter)); outcome.le16(UInt16(counter))
        outcome.le32(centralSize)
        outcome.le32(centralOffset)
        outcome.le16(0)                       // comment length
        return outcome
    }

    // MARK: - Reading (STORE + DEFLATE)

    /// The inflated upper bound for a single entry — let us not trust the declared
    /// size and allocate 4 GB (zip bomb / corrupt field).
    /// 256 MB was generous for a single phone; no legitimate docx/xlsx entry reaches
    /// that size.
    private static let maxEntry = 64 * 1024 * 1024
    /// The inflated upper bound for the whole archive.
    private static let maxTotal = 512 * 1024 * 1024

    /// Opens the zip. THE INPUT COMES FROM THE USER: every length field can lie.
    /// That is why no slice/read is done without a bounds check; on a corrupt file
    /// `ZipError.malformed` is thrown instead of crashing.
    static func open(_ zip: Data) throws -> [String: Data] {
        let bytes = [UInt8](zip)
        guard let eocd = findEOCD(bytes) else { throw ZipError.malformed("no EOCD") }
        let recordCount = try need16(bytes, eocd + 10, "EOCD record count")
        var offset = try need32(bytes, eocd + 16, "EOCD central offset")
        guard offset <= bytes.count else { throw ZipError.malformed("central offset outside the file") }

        var outcome: [String: Data] = [:]
        var totalInflated = 0

        for _ in 0..<recordCount {
            guard bytes.count - offset >= 46 else { throw ZipError.malformed("central directory record truncated") }
            let signature = try need32(bytes, offset, "central directory signature")
            guard signature == 0x02014b50 else { throw ZipError.malformed("central directory record") }

            let method = try need16(bytes, offset + 10, "compression method")
            let compressedSize = try need32(bytes, offset + 20, "compressed size")
            let rawSize = try need32(bytes, offset + 24, "raw size")
            let nameLen = try need16(bytes, offset + 28, "name length")
            let extraLen = try need16(bytes, offset + 30, "extra length")
            let commentLen = try need16(bytes, offset + 32, "comment length")
            let localOffset = try need32(bytes, offset + 42, "local header offset")

            // Is the WHOLE record inside the file? If nameLen lies, the name slice would overflow.
            let recordSize = 46 + nameLen + extraLen + commentLen
            guard bytes.count - offset >= recordSize else { throw ZipError.malformed("record bound") }
            let name = String(decoding: bytes[(offset + 46)..<(offset + 46 + nameLen)], as: UTF8.self)
            let nextOffset = offset + recordSize

            // Find the data start from the local header.
            guard bytes.count - localOffset >= 30 else { throw ZipError.malformed("local header bound") }
            let localSignature = try need32(bytes, localOffset, "local header signature")
            guard localSignature == 0x04034b50 else { throw ZipError.malformed("local header") }
            let localNameLen = try need16(bytes, localOffset + 26, "local name length")
            let localExtraLen = try need16(bytes, localOffset + 28, "local extra length")
            let dataStart = localOffset + 30 + localNameLen + localExtraLen
            // Compare with subtraction: leave no room for an addition overflow.
            guard dataStart <= bytes.count, bytes.count - dataStart >= compressedSize else {
                throw ZipError.malformed("data bound")
            }
            guard rawSize <= maxEntry else { throw ZipError.malformed("entry too large: \(name)") }

            let inflated: Data
            switch method {
            case 0:
                inflated = Data(bytes[dataStart..<(dataStart + compressedSize)])
            case 8:
                if compressedSize == 0 {
                    inflated = Data()
                } else {
                    let raw = Data(bytes[dataStart..<(dataStart + compressedSize)])
                    guard let c = inflate(raw, rawSize: rawSize) else {
                        throw ZipError.malformed("deflate could not be inflated: \(name)")
                    }
                    inflated = c
                }
            default:
                // Unsupported method: putting empty data there would be a silent lie
                // (the caller sees "empty document" instead of "no content"). Skip the entry.
                offset = nextOffset
                continue
            }

            totalInflated += inflated.count
            guard totalInflated <= maxTotal else { throw ZipError.malformed("inflated content too large") }
            outcome[name] = inflated
            offset = nextOffset
        }
        return outcome
    }

    private static func findEOCD(_ b: [UInt8]) -> Int? {
        guard b.count >= 22 else { return nil }
        var i = b.count - 22
        let lower = max(0, b.count - 22 - 65_536)
        while i >= lower {
            // The signature is not enough: the comment length must agree with the end
            // of the file (random match).
            if read32(b, i) == 0x06054b50,
               let commentLen = read16(b, i + 20), i + 22 + Int(commentLen) <= b.count {
                return i
            }
            i -= 1
        }
        return nil
    }

    /// Raw DEFLATE inflation (Apple Compression, COMPRESSION_ZLIB = raw deflate).
    /// If `rawSize` is 0 (some writers put the size only in the data descriptor) the
    /// buffer is grown and the attempt is repeated.
    private static func inflate(_ data: Data, rawSize: Int) -> Data? {
        guard !data.isEmpty else { return Data() }
        // A DECLARED SIZE IS NOT AN ALLOCATION ORDER. When 64 bytes of garbage data
        // declared `rawSize = 256 MB`, a 256 MB buffer used to be allocated (measured)
        // — not a crash, but the road to jetsam. It starts from a sensible estimate
        // derived from the compressed size; if that is not enough the loop grows it
        // anyway, and the ceiling is still the declared size.
        let cap = rawSize > 0 ? min(rawSize, maxEntry) : maxEntry
        var capacity = min(cap, max(data.count * 8, 64 * 1024))
        for _ in 0..<8 {
            guard capacity > 0, capacity <= maxEntry else { return nil }
            var output = Data(count: capacity)
            let allocated = capacity
            let written = output.withUnsafeMutableBytes { (d: UnsafeMutableRawBufferPointer) -> Int in
                data.withUnsafeBytes { (s: UnsafeRawBufferPointer) -> Int in
                    guard let db = d.baseAddress?.assumingMemoryBound(to: UInt8.self),
                          let sb = s.baseAddress?.assumingMemoryBound(to: UInt8.self) else { return 0 }
                    return compression_decode_buffer(db, allocated, sb, data.count, nil, COMPRESSION_ZLIB)
                }
            }
            guard written > 0 else { return nil }
            // Hitting the end of the buffer means "it may have been truncated" → grow.
            // If we hit the ceiling there is no room to grow; what we have is the result.
            if written < allocated || allocated >= cap { return output.prefix(written) }
            capacity = min(capacity * 4, cap)
        }
        return nil
    }

    // MARK: - Small readers (bounds checked)

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

    private static let crcTable: [UInt32] = {
        (0..<256).map { i -> UInt32 in
            var c = UInt32(i)
            for _ in 0..<8 { c = (c & 1) != 0 ? (0xEDB88320 ^ (c >> 1)) : (c >> 1) }
            return c
        }
    }()

    static func crc32(_ data: Data) -> UInt32 {
        var crc: UInt32 = 0xFFFFFFFF
        for b in data { crc = crcTable[Int((crc ^ UInt32(b)) & 0xFF)] ^ (crc >> 8) }
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
