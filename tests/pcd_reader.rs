use std::{io::Cursor, sync::Arc};

use pcx_cli::{
    core::point::{
        PointBatch, PointColumn, PointDimensions, PointField, PointFrameMetadata, PointSchema,
        PrimitiveType, Timestamp,
    },
    pcd::{self, Encoding, ReadError},
};

const BINARY: &[u8] = include_bytes!("fixtures/valid/pointcloud2-binary.pcd");
const ASCII: &[u8] = include_bytes!("fixtures/valid/pointcloud2-ascii.pcd");
const UNKNOWN_FIELDS: &[u8] =
    include_bytes!("fixtures/valid/pcd-organized-unknown-fields-ascii.pcd");

fn read_bytes(bytes: &[u8]) -> Result<pcd::ReadResult, ReadError> {
    pcd::read(&mut Cursor::new(bytes), usize::MAX)
}

#[test]
fn reads_reviewed_ascii_and_binary_goldens_into_the_common_model() {
    for (bytes, exact_nan_bits) in [(BINARY, true), (ASCII, false)] {
        let decoded = read_bytes(bytes).unwrap();
        let points = decoded.points();
        assert_eq!(points.dimensions(), PointDimensions::new(2, 1).unwrap());
        assert_eq!(
            points
                .schema()
                .fields()
                .iter()
                .map(|field| (field.name(), field.primitive(), field.count()))
                .collect::<Vec<_>>(),
            [
                ("x", PrimitiveType::F32, 1),
                ("y", PrimitiveType::F32, 1),
                ("z", PrimitiveType::F32, 1),
                ("intensity", PrimitiveType::U16, 1),
                ("ring", PrimitiveType::U16, 1),
            ]
        );
        assert_eq!(
            points.metadata().sensor_timestamp(),
            Timestamp::new(0, 0).unwrap()
        );
        assert_eq!(points.metadata().frame_id(), "");
        assert!(!points.metadata().is_dense());
        assert_eq!(points.column("x"), Some(&PointColumn::F32(vec![1.0, -0.0])));
        assert_eq!(
            points.column("y"),
            Some(&PointColumn::F32(vec![-2.5, f32::INFINITY]))
        );
        let PointColumn::F32(z) = points.column("z").unwrap() else {
            panic!("z fixture field must be float32")
        };
        assert_eq!(z[0].to_bits(), 0);
        assert!(z[1].is_nan());
        if exact_nan_bits {
            assert_eq!(z[1].to_bits(), 0x7fc0_1234);
        }
        assert_eq!(
            points.column("intensity"),
            Some(&PointColumn::U16(vec![42, u16::MAX]))
        );
        assert_eq!(points.column("ring"), Some(&PointColumn::U16(vec![7, 8])));
        assert_eq!(decoded.plan().point_data_bytes(), 32);
        assert!(decoded.plan().peak_managed_bytes() >= 32);
    }
}

#[test]
fn preserves_organized_dimensions_counts_and_unknown_representable_fields() {
    let decoded = read_bytes(UNKNOWN_FIELDS).unwrap();
    let points = decoded.points();
    assert_eq!(points.dimensions(), PointDimensions::new(2, 2).unwrap());
    assert_eq!(
        points
            .schema()
            .fields()
            .iter()
            .map(|field| (
                field.name(),
                field.primitive(),
                field.count(),
                field.semantic()
            ))
            .collect::<Vec<_>>(),
        [
            ("descriptor", PrimitiveType::I16, 2, None),
            ("quality", PrimitiveType::F64, 1, None),
            ("flag", PrimitiveType::U8, 1, None),
        ]
    );
    assert_eq!(
        points.column("descriptor"),
        Some(&PointColumn::I16(vec![1, 2, 3, 4, 5, 6, 7, 8]))
    );
    assert_eq!(
        points.column("flag"),
        Some(&PointColumn::U8(vec![1, 0, 1, 0]))
    );
    let PointColumn::F64(quality) = points.column("quality").unwrap() else {
        panic!("quality fixture field must be float64")
    };
    assert_eq!(quality[..3], [1.25, -0.0, f64::NEG_INFINITY]);
    assert!(quality[3].is_nan());
}

#[test]
fn malformed_fixtures_are_rejected_without_panicking() {
    let cases: &[(&[u8], &str)] = &[
        (
            include_bytes!("fixtures/malformed/pcd-points-must-equal-width-times-height.pcd"),
            "POINTS",
        ),
        (
            include_bytes!("fixtures/malformed/pcd-directives-must-be-ordered.pcd"),
            "expected FIELDS",
        ),
        (
            include_bytes!("fixtures/malformed/pcd-field-vectors-must-align.pcd"),
            "SIZE has",
        ),
        (
            include_bytes!("fixtures/malformed/pcd-field-type-size-must-be-supported.pcd"),
            "unsupported TYPE/SIZE",
        ),
        (
            include_bytes!("fixtures/malformed/pcd-field-count-must-be-positive.pcd"),
            "COUNT",
        ),
        (
            include_bytes!("fixtures/malformed/pcd-field-names-must-be-unique.pcd"),
            "DuplicateFieldName",
        ),
        (
            include_bytes!("fixtures/malformed/pcd-dimensions-must-not-overflow.pcd"),
            "overflows",
        ),
        (
            include_bytes!("fixtures/malformed/pcd-height-must-be-positive.pcd"),
            "HEIGHT",
        ),
        (
            include_bytes!("fixtures/malformed/pcd-viewpoint-must-be-preservable.pcd"),
            "VIEWPOINT",
        ),
        (
            include_bytes!("fixtures/malformed/pcd-compressed-must-be-rejected.pcd"),
            "binary_compressed",
        ),
        (
            include_bytes!("fixtures/malformed/pcd-ascii-payload-must-be-complete.pcd"),
            "truncated",
        ),
        (
            include_bytes!("fixtures/malformed/pcd-ascii-payload-must-not-have-extra-values.pcd"),
            "extra ASCII",
        ),
        (
            include_bytes!("fixtures/malformed/pcd-binary-payload-must-be-exact.pcd"),
            "truncated",
        ),
        (
            include_bytes!("fixtures/malformed/pcd-binary-payload-must-not-have-extra-bytes.pcd"),
            "exceeds declared size",
        ),
    ];
    for &(bytes, expected) in cases {
        let result = std::panic::catch_unwind(|| read_bytes(bytes));
        let error = result.expect("malformed PCD must not panic").unwrap_err();
        assert!(
            error.to_string().contains(expected),
            "{error:?} did not contain {expected:?}"
        );
    }
}

#[test]
fn every_golden_truncation_is_rejected_without_panicking() {
    for (bytes, significant_len) in [
        (BINARY, BINARY.len()),
        (
            ASCII,
            ASCII
                .iter()
                .rposition(|byte| !byte.is_ascii_whitespace())
                .unwrap()
                + 1,
        ),
        (
            UNKNOWN_FIELDS,
            UNKNOWN_FIELDS
                .iter()
                .rposition(|byte| !byte.is_ascii_whitespace())
                .unwrap()
                + 1,
        ),
    ] {
        for boundary in 0..significant_len {
            let result = std::panic::catch_unwind(|| read_bytes(&bytes[..boundary]));
            assert!(result.is_ok(), "reader panicked at byte {boundary}");
            assert!(
                result.unwrap().is_err(),
                "reader accepted truncation at byte {boundary}"
            );
        }
    }
}

#[test]
fn memory_is_planned_and_refused_before_payload_consumption() {
    let data_offset = BINARY
        .windows(b"DATA binary\n".len())
        .position(|window| window == b"DATA binary\n")
        .unwrap()
        + b"DATA binary\n".len();
    let mut input = Cursor::new(BINARY);
    assert!(matches!(
        pcd::read(&mut input, 1),
        Err(ReadError::MemoryLimit { .. })
    ));
    assert_eq!(input.position(), u64::try_from(data_offset).unwrap());

    let admitted = read_bytes(BINARY).unwrap().plan().peak_managed_bytes();
    assert!(pcd::read(&mut Cursor::new(BINARY), admitted).is_ok());
    assert!(matches!(
        pcd::read(&mut Cursor::new(BINARY), admitted - 1),
        Err(ReadError::MemoryLimit { .. })
    ));
}

#[test]
fn header_and_ascii_token_buffers_are_hard_bounded() {
    let oversized_header = vec![b'#'; 64 * 1024 + 1];
    assert!(matches!(
        read_bytes(&oversized_header),
        Err(ReadError::HeaderTooLarge { maximum: 65_536 })
    ));

    let mut oversized_token = b"VERSION 0.7\nFIELDS x\nSIZE 4\nTYPE F\nCOUNT 1\nWIDTH 1\nHEIGHT 1\nVIEWPOINT 0 0 0 1 0 0 0\nPOINTS 1\nDATA ascii\n".to_vec();
    oversized_token.extend([b'1'; 257]);
    assert!(matches!(
        read_bytes(&oversized_token),
        Err(ReadError::AsciiTokenTooLarge { maximum: 256 })
    ));
}

fn invariant_batch() -> PointBatch {
    let schema = Arc::new(
        PointSchema::new(vec![
            PointField::new("i8", PrimitiveType::I8, 1, None).unwrap(),
            PointField::new("u8_pair", PrimitiveType::U8, 2, None).unwrap(),
            PointField::new("i16", PrimitiveType::I16, 1, None).unwrap(),
            PointField::new("u16", PrimitiveType::U16, 1, None).unwrap(),
            PointField::new("i32", PrimitiveType::I32, 1, None).unwrap(),
            PointField::new("u32", PrimitiveType::U32, 1, None).unwrap(),
            PointField::new("f32", PrimitiveType::F32, 1, None).unwrap(),
            PointField::new("f64", PrimitiveType::F64, 1, None).unwrap(),
        ])
        .unwrap(),
    );
    PointBatch::new(
        schema,
        Arc::new(PointFrameMetadata::new(
            Timestamp::new(0, 0).unwrap(),
            "",
            false,
        )),
        PointDimensions::new(1, 2).unwrap(),
        vec![
            PointColumn::I8(vec![-1, i8::MIN]),
            PointColumn::U8(vec![1, 2, 3, u8::MAX]),
            PointColumn::I16(vec![-3, i16::MIN]),
            PointColumn::U16(vec![4, u16::MAX]),
            PointColumn::I32(vec![-5, i32::MIN]),
            PointColumn::U32(vec![6, u32::MAX]),
            PointColumn::F32(vec![-0.0, f32::INFINITY]),
            PointColumn::F64(vec![1.25, f64::NEG_INFINITY]),
        ],
    )
    .unwrap()
}

#[test]
fn writer_reader_invariant_holds_for_every_supported_primitive_and_encoding() {
    let expected = invariant_batch();
    for encoding in [Encoding::Binary, Encoding::Ascii] {
        let mut encoded = Vec::new();
        pcd::write(&mut encoded, &expected, encoding).unwrap();
        let actual = read_bytes(&encoded).unwrap().into_points();
        assert_eq!(actual.schema(), expected.schema());
        assert_eq!(actual.dimensions(), expected.dimensions());
        assert_eq!(actual.columns(), expected.columns());
    }
}

#[test]
fn arbitrary_byte_inputs_do_not_panic() {
    for length in 0..=512 {
        let bytes = (0..length)
            .map(|index| (index * 131 + length * 17) as u8)
            .collect::<Vec<_>>();
        assert!(std::panic::catch_unwind(|| read_bytes(&bytes)).is_ok());
    }
}
