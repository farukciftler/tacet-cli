//! tacet-zip tests.
//!
//! The emphasis is on two things: (1) round-trip correctness, (2) MALFORMED
//! INPUT DOES NOT PANIC. The second was the known weakness of the Swift version
//! (read16/read32 without bounds checks), which is why truncation/corruption/fuzz
//! tests dominate here.

use tacet_zip::{ARCHIVE_CAP, ENTRY_CAP, ZipEntry, ZipError, crc32, inflate, open, open_map, pack};

/// Real vectors produced by other tools; reading back what we wrote ourselves
/// does not prove the decoder is CORRECT, only that it is consistent.
const DYNAMIC_DEFLATE: &[u8] = include_bytes!("data/dynamic.deflate");
const DYNAMIC_RAW: &[u8] = include_bytes!("data/dynamic.raw");
const REAL_ZIP: &[u8] = include_bytes!("data/real.zip");
const BOMB_DEFLATE: &[u8] = include_bytes!("data/bomb.deflate");

// ---------------------------------------------------------------- CRC32

#[test]
fn crc32_known_vector() {
    // The standard IEEE CRC32 check value for "123456789".
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(crc32(b""), 0);
}

// ------------------------------------------------------- Write, then read

#[test]
fn round_trip_single_file() {
    let entries = vec![ZipEntry::new("xl/workbook.xml", b"<workbook/>".to_vec())];
    let zip = pack(&entries).expect("packing succeeds");
    let opened = open(&zip).expect("opening succeeds");
    assert_eq!(opened, entries);
}

#[test]
fn round_trip_multi_file_archive() {
    let entries: Vec<ZipEntry> = (0..25)
        .map(|i| ZipEntry::new(format!("part/{i}.xml"), format!("<v>{i}</v>").into_bytes()))
        .collect();
    let zip = pack(&entries).unwrap();
    let map = open_map(&zip).unwrap();
    assert_eq!(map.len(), 25);
    assert_eq!(map["part/7.xml"], b"<v>7</v>".to_vec());
}

#[test]
fn round_trip_empty_file_and_empty_archive() {
    // An entry with an empty body: the CRC becomes 0, which triggers the
    // reader's CRC-skipping path.
    let entries = vec![
        ZipEntry::new("empty.txt", Vec::new()),
        ZipEntry::new("full.txt", b"x".to_vec()),
    ];
    let zip = pack(&entries).unwrap();
    assert_eq!(open(&zip).unwrap(), entries);

    // No entries at all: a valid but empty archive.
    let empty_zip = pack(&[]).unwrap();
    assert_eq!(empty_zip.len(), 22, "there must be only the EOCD record");
    assert!(open(&empty_zip).unwrap().is_empty());
}

#[test]
fn round_trip_long_name_and_utf8() {
    let long_name = format!("{}/file.xml", "a".repeat(60_000));
    let entries = vec![
        ZipEntry::new(long_name.clone(), b"data".to_vec()),
        ZipEntry::new("documents/özet-günlük.xml", b"utf8 name".to_vec()),
    ];
    let zip = pack(&entries).unwrap();
    let map = open_map(&zip).unwrap();
    assert_eq!(map[&long_name], b"data".to_vec());
    assert_eq!(map["documents/özet-günlük.xml"], b"utf8 name".to_vec());
}

#[test]
fn round_trip_binary_data() {
    let body: Vec<u8> = (0..=255u8).cycle().take(100_000).collect();
    let entries = vec![ZipEntry::new("media/image1.png", body.clone())];
    let zip = pack(&entries).unwrap();
    assert_eq!(open(&zip).unwrap()[0].data, body);
}

// ------------------------------------------------------------- DEFLATE

#[test]
fn inflate_stored_block() {
    // zlib level 0 output: a kind 0 (stored) block.
    let input: &[u8] = &[
        0x01, 0x0b, 0x00, 0xf4, 0xff, 0x68, 0x65, 0x6c, 0x6c, 0x6f, 0x20, 0x77, 0x6f, 0x72, 0x6c,
        0x64,
    ];
    assert_eq!(inflate(input, ENTRY_CAP).unwrap(), b"hello world");
}

#[test]
fn inflate_fixed_huffman() {
    // zlib Z_FIXED output: a kind 1 block, contains a back-reference (distance 1).
    let input: &[u8] = &[0x4b, 0x4c, 0xc4, 0x07, 0x00];
    assert_eq!(inflate(input, ENTRY_CAP).unwrap(), b"a".repeat(30));
}

#[test]
fn inflate_dynamic_huffman() {
    let decoded = inflate(DYNAMIC_DEFLATE, ENTRY_CAP).unwrap();
    assert_eq!(decoded, DYNAMIC_RAW);
}

#[test]
fn inflate_the_deflate_entries_of_a_real_zip() {
    // A real archive produced with Python zipfile, using the DEFLATE method.
    let map = open_map(REAL_ZIP).expect("the real zip must open");
    assert_eq!(map.len(), 3);
    assert_eq!(
        map["[Content_Types].xml"],
        br#"<?xml version="1.0"?><Types/>"#.to_vec()
    );
    assert_eq!(
        map["xl/workbook.xml"],
        format!("<workbook>{}</workbook>", "<sheet/>".repeat(80)).into_bytes()
    );
    assert!(map["empty.txt"].is_empty());
}

// --------------------------------------------------------- Zip bomb / limits

#[test]
fn the_inflate_cap_stops_a_zip_bomb() {
    // 8 KiB of input -> 8 MiB of output. With a 64 KiB cap it must be rejected.
    let err = inflate(BOMB_DEFLATE, 64 * 1024).unwrap_err();
    assert!(
        matches!(err, ZipError::LimitExceeded(_)),
        "expected LimitExceeded, got {err:?}"
    );

    // With a sufficient cap the same input decodes without trouble.
    let full = inflate(BOMB_DEFLATE, ENTRY_CAP).unwrap();
    assert_eq!(full.len(), 8 * 1024 * 1024);
    assert!(full.iter().all(|&b| b == 0));
}

#[test]
fn the_cap_constants_are_sane() {
    // The single-entry cap cannot exceed the archive cap; if it did, the archive
    // limit would become meaningless.
    const { assert!(ENTRY_CAP <= ARCHIVE_CAP) };
}

// ---------------------------------------------------- Malformed input -> Err

#[test]
fn input_that_is_too_short_errors() {
    for length in 0..22 {
        let input = vec![0u8; length];
        assert!(
            open(&input).is_err(),
            "Err was expected for length {length}"
        );
    }
}

#[test]
fn a_corrupted_eocd_signature_errors() {
    let zip = pack(&[ZipEntry::new("a.txt", b"data".to_vec())]).unwrap();
    let mut broken = zip.clone();
    let n = broken.len();
    broken[n - 22] ^= 0xFF; // the first byte of the EOCD signature
    assert!(matches!(open(&broken), Err(ZipError::Malformed(_))));
}

#[test]
fn a_truncated_file_does_not_panic() {
    let zip = pack(&[
        ZipEntry::new("a.txt", b"first entry".to_vec()),
        ZipEntry::new("b.txt", b"second entry".to_vec()),
    ])
    .unwrap();
    // Every truncation point: either Ok (the whole file) or Err — but never a panic.
    for cut in 0..zip.len() {
        let _ = open(&zip[..cut]);
    }
    // Every version trimmed at the end is left without an EOCD, so it must be Err.
    assert!(open(&zip[..zip.len() - 1]).is_err());
}

#[test]
fn an_overflowing_central_directory_offset_errors() {
    let zip = pack(&[ZipEntry::new("a.txt", b"data".to_vec())]).unwrap();
    let mut broken = zip.clone();
    let n = broken.len();
    // EOCD + 16: the central directory offset. Point it far past the file length.
    broken[n - 6..n - 2].copy_from_slice(&0xFFFF_FF00u32.to_le_bytes());
    assert!(matches!(open(&broken), Err(ZipError::Malformed(_))));
}

#[test]
fn an_overflowing_local_header_offset_errors() {
    let zip = pack(&[ZipEntry::new("a.txt", b"data".to_vec())]).unwrap();
    let mut broken = zip.clone();
    // The central directory record comes right after the body; the local offset
    // field is at record+42.
    let central_start = u32::from_le_bytes(
        broken[broken.len() - 6..broken.len() - 2]
            .try_into()
            .unwrap(),
    ) as usize;
    broken[central_start + 42..central_start + 46].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
    assert!(open(&broken).is_err());
}

#[test]
fn an_inflated_record_count_errors() {
    let zip = pack(&[ZipEntry::new("a.txt", b"data".to_vec())]).unwrap();
    let mut broken = zip.clone();
    let n = broken.len();
    // EOCD + 10: the record count on this disk. Let us say 5000 instead of 1.
    broken[n - 12..n - 10].copy_from_slice(&5000u16.to_le_bytes());
    assert!(
        open(&broken).is_err(),
        "records that do not exist must give Err"
    );
}

#[test]
fn an_unsupported_method_errors() {
    let zip = pack(&[ZipEntry::new("a.txt", b"data".to_vec())]).unwrap();
    let mut broken = zip.clone();
    let central_start = u32::from_le_bytes(
        broken[broken.len() - 6..broken.len() - 2]
            .try_into()
            .unwrap(),
    ) as usize;
    // Central directory + 10: the method. 99 = AES, which we do not support.
    broken[central_start + 10..central_start + 12].copy_from_slice(&99u16.to_le_bytes());
    assert!(matches!(
        open(&broken),
        Err(ZipError::UnsupportedMethod(99))
    ));
}

#[test]
fn a_crc_mismatch_is_caught() {
    let zip = pack(&[ZipEntry::new("a.txt", b"data".to_vec())]).unwrap();
    let mut broken = zip.clone();
    // The body comes after a 30-byte local header + a 5-byte name: flip one byte.
    let body_start = 30 + 5;
    broken[body_start] ^= 0x01;
    assert!(matches!(open(&broken), Err(ZipError::CrcMismatch { .. })));
}

#[test]
fn a_malformed_deflate_stream_does_not_panic() {
    // Every byte of the dynamic Huffman vector is corrupted one at a time: all of
    // them are Ok or Err, none of them a panic. Building the Huffman table is the
    // most fragile place.
    for i in 0..DYNAMIC_DEFLATE.len() {
        let mut broken = DYNAMIC_DEFLATE.to_vec();
        broken[i] ^= 0xA5;
        let _ = inflate(&broken, ENTRY_CAP);
    }
    // Truncated streams must be rejected safely as well.
    for cut in 0..DYNAMIC_DEFLATE.len() {
        let _ = inflate(&DYNAMIC_DEFLATE[..cut], ENTRY_CAP);
    }
}

#[test]
fn fuzzing_with_random_bytes_does_not_panic() {
    // A dependency-free, reproducible pseudo-random generator (xorshift64*).
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };

    // 1) Entirely random buffers.
    for _ in 0..500 {
        let length = (next() % 512) as usize;
        let buffer: Vec<u8> = (0..length).map(|_| (next() & 0xFF) as u8).collect();
        let _ = open(&buffer);
        let _ = inflate(&buffer, ENTRY_CAP);
    }

    // 2) Randomly corrupted versions of a valid archive — the harder scenario,
    //    because most of the structure stays consistent and the reader descends
    //    deep.
    let clean = pack(&[
        ZipEntry::new("[Content_Types].xml", b"<Types/>".to_vec()),
        ZipEntry::new("xl/workbook.xml", vec![7u8; 300]),
    ])
    .unwrap();
    for _ in 0..2000 {
        let mut broken = clean.clone();
        let corruption_count = 1 + (next() % 4) as usize;
        for _ in 0..corruption_count {
            let slot = (next() as usize) % broken.len();
            broken[slot] ^= (next() & 0xFF) as u8;
        }
        let _ = open(&broken);
    }

    // 3) Corrupted versions of the real deflate-bearing archive.
    for _ in 0..1000 {
        let mut broken = REAL_ZIP.to_vec();
        let slot = (next() as usize) % broken.len();
        broken[slot] ^= (next() & 0xFF) as u8;
        let _ = open(&broken);
    }
}
