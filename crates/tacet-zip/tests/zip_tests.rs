//! tacet-zip tests.
//!
//! The emphasis is on two things: (1) round-trip correctness, (2) MALFORMED
//! INPUT DOES NOT PANIC. The second was the known weakness of the Swift version
//! (read16/read32 without bounds checks), which is why truncation/corruption/fuzz
//! tests dominate here.

use tacet_zip::{
    ARCHIVE_CAP, ENTRY_CAP, ZipEntry, ZipError, crc32, inflate, list, open, open_map,
    open_selected, pack,
};

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

// ------------------------------------------- list / open_selected (the new API)

/// Builds a zip container around ALREADY-ENCODED entry bodies.
///
/// WHY IT HAS TO EXIST: `pack` writes method 0 with `compressed_size ==
/// raw_size` and a correct CRC — by design (see writer.rs). Every claim the two
/// new functions make is about an archive that DOES NOT look like that: a
/// DEFLATE body, a size the header lies about, a mode bit saying "symlink".
/// None of those can be produced by the writer, so the container is assembled
/// here field by field. It is a test fixture, not a second writer: it validates
/// nothing on purpose.
struct RawEntry<'a> {
    name: &'a str,
    body: &'a [u8],
    method: u16,
    crc: u32,
    declared_size: u32,
    external_attributes: u32,
}

fn raw_zip(entries: &[RawEntry<'_>]) -> Vec<u8> {
    fn w16(out: &mut Vec<u8>, v: u16) {
        out.extend_from_slice(&v.to_le_bytes());
    }
    fn w32(out: &mut Vec<u8>, v: u32) {
        out.extend_from_slice(&v.to_le_bytes());
    }
    let mut body: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    for e in entries {
        let offset = body.len() as u32;
        let compressed = e.body.len() as u32;
        w32(&mut body, 0x0403_4b50);
        w16(&mut body, 20);
        w16(&mut body, 0);
        w16(&mut body, e.method);
        w16(&mut body, 0);
        w16(&mut body, 0);
        w32(&mut body, e.crc);
        w32(&mut body, compressed);
        w32(&mut body, e.declared_size);
        w16(&mut body, e.name.len() as u16);
        w16(&mut body, 0);
        body.extend_from_slice(e.name.as_bytes());
        body.extend_from_slice(e.body);

        w32(&mut central, 0x0201_4b50);
        w16(&mut central, 20);
        w16(&mut central, 20);
        w16(&mut central, 0);
        w16(&mut central, e.method);
        w16(&mut central, 0);
        w16(&mut central, 0);
        w32(&mut central, e.crc);
        w32(&mut central, compressed);
        w32(&mut central, e.declared_size);
        w16(&mut central, e.name.len() as u16);
        w16(&mut central, 0);
        w16(&mut central, 0);
        w16(&mut central, 0);
        w16(&mut central, 0);
        w32(&mut central, e.external_attributes);
        w32(&mut central, offset);
        central.extend_from_slice(e.name.as_bytes());
    }
    let central_offset = body.len() as u32;
    let central_size = central.len() as u32;
    let count = entries.len() as u16;
    let mut out = body;
    out.extend_from_slice(&central);
    w32(&mut out, 0x0605_4b50);
    w16(&mut out, 0);
    w16(&mut out, 0);
    w16(&mut out, count);
    w16(&mut out, count);
    w32(&mut out, central_size);
    w32(&mut out, central_offset);
    w16(&mut out, 0);
    out
}

#[test]
fn list_reports_the_directory_without_decoding_anything() {
    // THE CLAIM: `list` costs the length of the central directory, not the
    // length of the archive. It is measured the only way it can be — on an
    // archive whose DATA cannot be decoded at all. `open` fails on this one; if
    // `list` decoded, it would fail with it.
    let garbage = [0xFFu8; 8];
    let zip = raw_zip(&[RawEntry {
        name: "broken.bin",
        body: &garbage,
        method: 8,
        crc: 0,
        declared_size: u32::MAX,
        external_attributes: 0,
    }]);
    assert!(open(&zip).is_err(), "open must refuse this archive");

    let listed = list(&zip).expect("listing does not decode");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "broken.bin");
    assert_eq!(listed[0].method, 8);
    assert_eq!(listed[0].compressed_size, garbage.len());
    assert_eq!(
        listed[0].declared_size,
        u32::MAX as usize,
        "the DECLARED size is reported as declared — it is a claim, not a fact"
    );

    // MEASURED WHILE WRITING THIS TEST, AND IT IS WHY THE TOOL SIDE HAS A
    // DECLARED-VS-ACTUAL RULE OF ITS OWN: for a DEFLATE entry the reader ignores
    // `raw_size` entirely (see decode_entry_data — only the STORE branch
    // compares sizes), and a zero CRC is skipped. So an archive declaring 4 GiB
    // whose body really decodes to 8 MiB opens WITHOUT COMPLAINT. Nothing below
    // this line is a bug in the reader; it is the reason a caller may not treat
    // `declared_size` as a fact.
    let lying = raw_zip(&[RawEntry {
        name: "bomb.bin",
        body: BOMB_DEFLATE,
        method: 8,
        crc: 0,
        declared_size: u32::MAX,
        external_attributes: 0,
    }]);
    let opened = open(&lying).expect("the reader does not check the declared deflate size");
    assert_eq!(opened[0].data.len(), 8 * 1024 * 1024);
    assert_eq!(
        list(&lying).expect("listing")[0].declared_size,
        u32::MAX as usize
    );
}

#[test]
fn the_entry_cap_the_caller_passes_reaches_inflate() {
    // THE DEFECT THIS CLOSES: `decode_entry_data` used to hard-code
    // `inflate(raw, ENTRY_CAP)`, so a caller wanting a tighter ceiling could
    // only apply it AFTER 64 MiB had been inflated into memory. bomb.deflate is
    // 8144 bytes in and 8 MiB out; with a 64 KiB ceiling the refusal has to come
    // from inside inflate.
    let zip = raw_zip(&[RawEntry {
        name: "bomb.bin",
        body: BOMB_DEFLATE,
        method: 8,
        crc: 0,
        declared_size: 8 * 1024 * 1024,
        external_attributes: 0,
    }]);
    let names = vec!["bomb.bin".to_string()];
    let err = open_selected(&zip, &names, 64 * 1024, ARCHIVE_CAP).unwrap_err();
    assert!(
        matches!(err, ZipError::LimitExceeded(_)),
        "expected LimitExceeded, got {err:?}"
    );
    // The SAME archive with room to breathe decodes — so the refusal above is
    // the cap doing its job, not the fixture being broken.
    let ok = open_selected(&zip, &names, ENTRY_CAP, ARCHIVE_CAP).expect("decodes with room");
    assert_eq!(ok[0].data.len(), 8 * 1024 * 1024);
}

#[test]
fn open_selected_touches_only_the_entries_that_were_named() {
    // The unnamed entry is a bomb. If selection were done AFTER decoding — the
    // shape `open` + filter would give — this call could not succeed at all.
    let zip = raw_zip(&[
        RawEntry {
            name: "wanted.txt",
            body: b"hello",
            method: 0,
            crc: crc32(b"hello"),
            declared_size: 5,
            external_attributes: 0,
        },
        RawEntry {
            name: "bomb.bin",
            body: BOMB_DEFLATE,
            method: 8,
            crc: 0,
            declared_size: u32::MAX,
            external_attributes: 0,
        },
    ]);
    let got = open_selected(&zip, &["wanted.txt".to_string()], 1024, 1024).expect("only the named");
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].name, "wanted.txt");
    assert_eq!(got[0].data, b"hello".to_vec());
}

#[test]
fn open_selected_refuses_once_the_total_cap_is_passed() {
    let payload = vec![9u8; 100];
    let entries: Vec<RawEntry<'_>> = (0..3)
        .map(|_| RawEntry {
            name: "part.bin",
            body: &payload,
            method: 0,
            crc: crc32(&payload),
            declared_size: 100,
            external_attributes: 0,
        })
        .collect();
    let zip = raw_zip(&entries);
    let names = vec!["part.bin".to_string()];
    let err = open_selected(&zip, &names, ENTRY_CAP, 150).unwrap_err();
    assert!(
        matches!(err, ZipError::LimitExceeded(_)),
        "expected LimitExceeded, got {err:?}"
    );
    assert!(open_selected(&zip, &names, ENTRY_CAP, 300).is_ok());
}

#[test]
fn a_symlink_entry_is_visible_in_the_listing() {
    // 0o120777 << 16 is exactly what `zip` writes for a symbolic link; the body
    // of such an entry is the link TARGET, not file content. A caller that
    // materialises entries has no other way to tell.
    let zip = raw_zip(&[
        RawEntry {
            name: "link",
            body: b"/etc/passwd",
            method: 0,
            crc: crc32(b"/etc/passwd"),
            declared_size: 11,
            external_attributes: 0o120_777 << 16,
        },
        RawEntry {
            name: "plain.txt",
            body: b"x",
            method: 0,
            crc: crc32(b"x"),
            declared_size: 1,
            external_attributes: 0o100_644 << 16,
        },
    ]);
    let listed = list(&zip).expect("listing");
    assert!(listed[0].is_symlink(), "the link entry must be flagged");
    assert!(!listed[1].is_symlink(), "a regular file must not be");
    // A DOS/Windows writer leaves the high half empty; that must read as "not a
    // symlink", never as an accidental match.
    let dos = raw_zip(&[RawEntry {
        name: "plain.txt",
        body: b"x",
        method: 0,
        crc: crc32(b"x"),
        declared_size: 1,
        external_attributes: 0x20,
    }]);
    assert!(!list(&dos).expect("listing")[0].is_symlink());
}
