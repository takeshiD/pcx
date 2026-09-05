//! Independent fixture oracle for the PCD writer.
//!
//! This intentionally does not call product parsing code. It decodes the
//! reviewed fixture headers and payloads using a small test-only implementation.

use std::collections::BTreeMap;

#[derive(Debug)]
struct Header<'a> {
    entries: BTreeMap<&'a str, Vec<&'a str>>,
    body: &'a [u8],
}

fn parse_header(bytes: &[u8]) -> Header<'_> {
    let data_line = bytes
        .windows(b"DATA ".len())
        .position(|window| window == b"DATA ")
        .expect("DATA entry");
    let body_offset = data_line
        + bytes[data_line..]
            .iter()
            .position(|byte| *byte == b'\n')
            .expect("DATA newline")
        + 1;
    let text = std::str::from_utf8(&bytes[..body_offset]).expect("ASCII header");
    let mut entries = BTreeMap::new();
    for line in text.lines().filter(|line| !line.starts_with('#')) {
        let mut tokens = line.split_ascii_whitespace();
        let key = tokens.next().expect("header key");
        assert!(entries.insert(key, tokens.collect()).is_none());
    }
    Header {
        entries,
        body: &bytes[body_offset..],
    }
}

fn assert_fixture_header(header: &Header<'_>, encoding: &str) {
    assert_eq!(header.entries["VERSION"], ["0.7"]);
    assert_eq!(
        header.entries["FIELDS"],
        ["x", "y", "z", "intensity", "ring"]
    );
    assert_eq!(header.entries["SIZE"], ["4", "4", "4", "2", "2"]);
    assert_eq!(header.entries["TYPE"], ["F", "F", "F", "U", "U"]);
    assert_eq!(header.entries["COUNT"], ["1", "1", "1", "1", "1"]);
    assert_eq!(header.entries["WIDTH"], ["2"]);
    assert_eq!(header.entries["HEIGHT"], ["1"]);
    assert_eq!(
        header.entries["VIEWPOINT"],
        ["0", "0", "0", "1", "0", "0", "0"]
    );
    assert_eq!(header.entries["POINTS"], ["2"]);
    assert_eq!(header.entries["DATA"], [encoding]);
}

fn u16_le(bytes: &[u8]) -> u16 {
    u16::from_le_bytes(bytes.try_into().unwrap())
}

fn f32_bits_le(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().unwrap())
}

#[test]
fn reviewed_binary_fixture_is_independently_decodable_with_exact_bits() {
    let fixture = include_bytes!("fixtures/valid/pointcloud2-binary.pcd");
    let header = parse_header(fixture);
    assert_fixture_header(&header, "binary");
    assert_eq!(header.body.len(), 32);

    let expected = [
        ([0x3f80_0000, 0xc020_0000, 0], [42, 7]),
        ([0x8000_0000, 0x7f80_0000, 0x7fc0_1234], [u16::MAX, 8]),
    ];
    for (point, (floats, integers)) in expected.iter().enumerate() {
        let record = &header.body[point * 16..(point + 1) * 16];
        assert_eq!(
            [
                f32_bits_le(&record[0..4]),
                f32_bits_le(&record[4..8]),
                f32_bits_le(&record[8..12]),
            ],
            *floats
        );
        assert_eq!(
            [u16_le(&record[12..14]), u16_le(&record[14..16])],
            *integers
        );
    }
}

#[test]
fn reviewed_ascii_fixture_is_independently_decodable_semantically() {
    let fixture = include_bytes!("fixtures/valid/pointcloud2-ascii.pcd");
    let header = parse_header(fixture);
    assert_fixture_header(&header, "ascii");
    let rows: Vec<Vec<&str>> = std::str::from_utf8(header.body)
        .unwrap()
        .lines()
        .map(|line| line.split_ascii_whitespace().collect())
        .collect();
    assert_eq!(
        rows,
        [
            ["1", "-2.5", "0", "42", "7"],
            ["-0", "inf", "nan", "65535", "8"]
        ]
    );

    let negative_zero: f32 = rows[1][0].parse().unwrap();
    let infinity: f32 = rows[1][1].parse().unwrap();
    let nan: f32 = rows[1][2].parse().unwrap();
    assert_eq!(negative_zero.to_bits(), 0x8000_0000);
    assert_eq!(infinity, f32::INFINITY);
    assert!(nan.is_nan());
}
