//! LAS/LAZ interoperability tests with PDAL-produced fixtures and independent
//! value expectations (the adapter does not generate its own oracle inputs).

use pcx_cli::core::point::{PointBatch, PointColumn, PointFrameMetadata, Timestamp};
use pcx_cli::core::{FidelityLoss, LossPolicy};
use pcx_cli::las::{Encoding, ReadLimits, Reader, WriteLimits, Writer};
use std::io::{Cursor, Read, Seek};
use std::sync::Arc;

const LAS: &[u8] = include_bytes!("fixtures/valid/las-pdal.las");
const LAZ: &[u8] = include_bytes!("fixtures/valid/las-pdal.laz");

fn limits(points: usize) -> ReadLimits {
    ReadLimits::new(points, 2 * 1024 * 1024).unwrap()
}

fn assert_oracle(batch: &PointBatch, point: usize) {
    let f64s = |name| match batch.column(name).unwrap() {
        PointColumn::F64(values) => values.as_slice(),
        _ => panic!("{name} is not f64"),
    };
    let f32s = |name| match batch.column(name).unwrap() {
        PointColumn::F32(values) => values.as_slice(),
        _ => panic!("{name} is not f32"),
    };
    let u16s = |name| match batch.column(name).unwrap() {
        PointColumn::U16(values) => values.as_slice(),
        _ => panic!("{name} is not u16"),
    };
    let u8s = |name| match batch.column(name).unwrap() {
        PointColumn::U8(values) => values.as_slice(),
        _ => panic!("{name} is not u8"),
    };

    let expected = [
        (1000.25, 2000.50, -5.75, 42, 2, 1, 0, 0, 0, 1, 123456.5),
        (1001.00, 2001.25, -6.00, 65535, 9, 0, 1, 1, 1, 2, 123457.5),
    ][point];
    assert_eq!(f64s("x")[0], expected.0);
    assert_eq!(f64s("y")[0], expected.1);
    assert_eq!(f64s("z")[0], expected.2);
    assert_eq!(u16s("intensity")[0], expected.3);
    assert_eq!(u8s("return_number")[0], [1, 2][point]);
    assert_eq!(u8s("number_of_returns")[0], 2);
    assert_eq!(u8s("scan_direction")[0], 0);
    assert_eq!(u8s("edge_of_flight_line")[0], 0);
    assert_eq!(u8s("classification")[0], expected.4);
    assert_eq!(u8s("synthetic")[0], expected.5);
    assert_eq!(u8s("key_point")[0], expected.6);
    assert_eq!(u8s("withheld")[0], expected.7);
    assert_eq!(u8s("overlap")[0], expected.8);
    assert_eq!(u8s("scanner_channel")[0], expected.9);
    assert_eq!(
        f32s("scan_angle")[0],
        [-3.0, f32::from(667_i16) * 0.006][point]
    );
    assert_eq!(u8s("user_data")[0], [9, 10][point]);
    assert_eq!(u16s("point_source_id")[0], [77, 78][point]);
    assert_eq!(f64s("gps_time")[0], expected.10);
    assert_eq!(u16s("red")[0], [100, 65535][point]);
    assert_eq!(u16s("green")[0], [200, 0][point]);
    assert_eq!(u16s("blue")[0], [300, 32768][point]);
    assert_eq!(u16s("nir")[0], [400, 500][point]);
    let extra = u8s("las_extra_bytes");
    assert_eq!(
        extra,
        [21.5_f32.to_le_bytes(), (-10.25_f32).to_le_bytes()][point]
    );
}

fn read_fixture(bytes: &'static [u8]) -> (Arc<pcx_cli::las::SpatialMetadata>, Vec<PointBatch>) {
    let mut reader = Reader::new(Cursor::new(bytes), limits(1)).unwrap();
    assert_eq!(reader.metadata().scale(), [0.01, 0.01, 0.01]);
    assert_eq!(reader.metadata().offset(), [1000.0, 2000.0, -10.0]);
    assert!(
        reader
            .metadata()
            .crs_records()
            .any(|record| record.data.windows(4).any(|window| window == b"4978"))
    );
    assert!(reader.managed_peak_bytes() <= 2 * 1024 * 1024);
    let metadata = Arc::new(reader.metadata().clone());
    let mut batches = Vec::new();
    while let Some(batch) = reader.next_batch().unwrap() {
        batches.push(batch);
    }
    assert_eq!(batches.len(), 2, "one-point bound must split the fixture");
    assert_oracle(&batches[0], 0);
    assert_oracle(&batches[1], 1);
    (metadata, batches)
}

#[test]
fn reads_pdal_las_through_the_common_schema() {
    read_fixture(LAS);
}

#[test]
fn reads_pdal_laz_with_the_same_mapping_and_bound() {
    read_fixture(LAZ);
}

#[test]
fn lossless_batches_round_trip_to_las_and_laz() {
    for source in [LAS, LAZ] {
        let (metadata, batches) = read_fixture(source);
        for encoding in [Encoding::Las, Encoding::Laz] {
            let mut writer = Writer::new(
                Cursor::new(Vec::new()),
                Arc::clone(&metadata),
                encoding,
                WriteLimits::new(2, 2 * 1024 * 1024),
            )
            .unwrap();
            assert!(writer.managed_peak_bytes() <= 2 * 1024 * 1024);
            for batch in &batches {
                writer.write_batch(batch, &LossPolicy::lossless()).unwrap();
            }
            let bytes = writer.finish().unwrap().into_inner();
            let mut reader = Reader::new(Cursor::new(bytes), limits(2)).unwrap();
            let batch = reader.next_batch().unwrap().unwrap();
            assert_eq!(batch.dimensions().point_count(), 2);
            for name in ["x", "y", "z", "classification", "las_extra_bytes"] {
                let expected = batches
                    .iter()
                    .map(|batch| batch.column(name).unwrap())
                    .collect::<Vec<_>>();
                match batch.column(name).unwrap() {
                    PointColumn::F64(values) => assert_eq!(values.len(), expected.len()),
                    PointColumn::U8(values) if name == "las_extra_bytes" => {
                        assert_eq!(values.len(), 8)
                    }
                    PointColumn::U8(values) => assert_eq!(values.len(), expected.len()),
                    _ => panic!("unexpected oracle column"),
                }
            }
            assert_eq!(reader.metadata().scale(), metadata.scale());
            assert_eq!(reader.metadata().offset(), metadata.offset());
            assert_eq!(
                reader.metadata().crs_records().count(),
                metadata.crs_records().count()
            );
        }
    }
}

#[test]
fn coordinate_quantization_requires_explicit_representation_loss() {
    let (metadata, mut batches) = read_fixture(LAS);
    let original = batches.remove(0);
    let mut columns = original.columns().to_vec();
    let PointColumn::F64(x) = &mut columns[0] else {
        panic!("x column")
    };
    x[0] += 0.001;
    let changed = PointBatch::new(
        Arc::new(original.schema().clone()),
        Arc::new(original.metadata().clone()),
        original.dimensions(),
        columns,
    )
    .unwrap();

    let mut lossless = Writer::new(
        Cursor::new(Vec::new()),
        Arc::clone(&metadata),
        Encoding::Las,
        WriteLimits::new(1, 2 * 1024 * 1024),
    )
    .unwrap();
    let error = lossless
        .write_batch(&changed, &LossPolicy::lossless())
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("authorize representation loss explicitly")
    );

    let mut lossy = Writer::new(
        Cursor::new(Vec::new()),
        metadata,
        Encoding::Las,
        WriteLimits::new(1, 2 * 1024 * 1024),
    )
    .unwrap();
    lossy
        .write_batch(
            &changed,
            &LossPolicy::authorize([FidelityLoss::Representation]),
        )
        .unwrap();
    let _ = lossy.finish().unwrap();
}

#[test]
fn point_frame_metadata_loss_requires_explicit_authorization() {
    let (metadata, batches) = read_fixture(LAS);
    let original = &batches[0];
    let framed = PointBatch::new(
        Arc::new(original.schema().clone()),
        Arc::new(PointFrameMetadata::new(
            Timestamp::new(1, 2).unwrap(),
            "map",
            true,
        )),
        original.dimensions(),
        original.columns().to_vec(),
    )
    .unwrap();
    let mut writer = Writer::new(
        Cursor::new(Vec::new()),
        metadata,
        Encoding::Las,
        WriteLimits::new(1, 2 * 1024 * 1024),
    )
    .unwrap();
    assert!(
        writer
            .write_batch(&framed, &LossPolicy::lossless())
            .unwrap_err()
            .to_string()
            .contains("metadata loss")
    );
    writer
        .write_batch(&framed, &LossPolicy::authorize([FidelityLoss::Metadata]))
        .unwrap();
}

#[test]
fn malformed_and_unplannable_inputs_fail_without_a_batch() {
    for bytes in [b"not a LAS file".as_slice(), &LAS[..200]] {
        assert!(Reader::new(Cursor::new(bytes.to_vec()), limits(1)).is_err());
    }
    let error = Reader::new(Cursor::new(LAZ), ReadLimits::new(50_000, 1).unwrap())
        .err()
        .expect("one byte cannot satisfy preflight");
    assert!(error.to_string().contains("managed-memory peak"));
    assert!(ReadLimits::new(0, usize::MAX).is_err());

    let mut cursor = Cursor::new(LAZ);
    let _ = las::Header::new(&mut cursor).unwrap();
    let point_data_start = cursor.stream_position().unwrap();
    let mut offset = [0_u8; 8];
    cursor.read_exact(&mut offset).unwrap();
    let chunk_table = usize::try_from(i64::from_le_bytes(offset)).unwrap();
    assert!(chunk_table > usize::try_from(point_data_start).unwrap());
    let mut malicious_table = LAZ.to_vec();
    malicious_table[chunk_table + 4..chunk_table + 8].copy_from_slice(&u32::MAX.to_le_bytes());
    let error = Reader::new(Cursor::new(malicious_table), limits(1))
        .err()
        .expect("unbounded LAZ chunk table must fail preflight");
    assert!(error.to_string().contains("chunk count"));

    let (metadata, batches) = read_fixture(LAS);
    let mut bounded = Writer::new(
        Cursor::new(Vec::new()),
        Arc::clone(&metadata),
        Encoding::Laz,
        WriteLimits::new(0, 2 * 1024 * 1024),
    )
    .unwrap();
    assert!(
        bounded
            .write_batch(&batches[0], &LossPolicy::lossless())
            .unwrap_err()
            .to_string()
            .contains("point bound")
    );
    assert!(
        Writer::new(
            Cursor::new(Vec::new()),
            metadata,
            Encoding::Laz,
            WriteLimits::new(u64::MAX, 1),
        )
        .is_err()
    );
}
