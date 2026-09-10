//! PNG adapter coverage through its public raster-encoding seam.

use std::sync::Arc;

use pcx_cli::{
    core::point::{
        Endianness, PointDimensions, PointField, PointFieldSemantic, PointFrameMetadata,
        PointLayout, PointSchema, PointView, PrimitiveType, Timestamp,
    },
    core::{Cancellation, LossPolicy, PointRepresentation},
    ops::{
        ColorPolicy, DepthPolicy, IntensityRange, InvalidProjectionCoordinatePolicy,
        OrthographicView, Projection, Raster, RasterDimensions, Rgb8,
    },
    png::PngPlan,
};

fn raster() -> Raster {
    let schema = Arc::new(
        PointSchema::new(vec![
            PointField::new("x", PrimitiveType::F64, 1, Some(PointFieldSemantic::X)).unwrap(),
            PointField::new("y", PrimitiveType::F64, 1, Some(PointFieldSemantic::Y)).unwrap(),
            PointField::new("z", PrimitiveType::F64, 1, Some(PointFieldSemantic::Z)).unwrap(),
            PointField::new(
                "intensity",
                PrimitiveType::F32,
                1,
                Some(PointFieldSemantic::Intensity),
            )
            .unwrap(),
        ])
        .unwrap(),
    );
    let points = [(-1.0_f64, 0.0_f64, 0.0_f64, 0.0_f32), (1.0, 0.0, 0.0, 1.0)];
    let mut source = Vec::new();
    for (x, y, z, intensity) in points {
        source.extend_from_slice(&x.to_le_bytes());
        source.extend_from_slice(&y.to_le_bytes());
        source.extend_from_slice(&z.to_le_bytes());
        source.extend_from_slice(&intensity.to_le_bytes());
    }
    let dimensions = PointDimensions::new(points.len(), 1).unwrap();
    let view = PointView::new(
        Arc::from(source),
        Arc::new(PointFrameMetadata::new(
            Timestamp::new(1, 2).unwrap(),
            "map",
            false,
        )),
        PointLayout::new(
            Arc::clone(&schema),
            dimensions,
            vec![0, 8, 16, 24],
            28,
            56,
            0,
            Endianness::Little,
        )
        .unwrap(),
    )
    .unwrap();
    Projection::new(
        RasterDimensions::new(3, 1).unwrap(),
        OrthographicView::xy(),
        DepthPolicy::Nearest,
        InvalidProjectionCoordinatePolicy::Drop,
        ColorPolicy::Intensity {
            range: IntensityRange::new(0.0, 1.0).unwrap(),
            invalid: Rgb8([255, 0, 255]),
        },
    )
    .plan(
        schema,
        dimensions,
        PointRepresentation::View,
        &LossPolicy::lossless(),
    )
    .unwrap()
    .execute_view(&view)
    .unwrap()
}

fn chunks(bytes: &[u8]) -> Vec<(&[u8; 4], &[u8])> {
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    let mut offset = 8;
    let mut chunks = Vec::new();
    while offset < bytes.len() {
        let length = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        let kind = bytes[offset + 4..offset + 8].try_into().unwrap();
        let data = &bytes[offset + 8..offset + 8 + length];
        chunks.push((kind, data));
        offset += 12 + length;
    }
    assert_eq!(offset, bytes.len());
    chunks
}

fn stored_zlib_payload(idat: &[u8]) -> Vec<u8> {
    assert_eq!(&idat[..2], &[0x78, 0x01]);
    let mut offset = 2;
    let mut output = Vec::new();
    loop {
        let header = idat[offset];
        offset += 1;
        assert_eq!(header & 0x06, 0, "encoder must use stored DEFLATE blocks");
        let final_block = header & 1 == 1;
        let length = u16::from_le_bytes(idat[offset..offset + 2].try_into().unwrap());
        let complement = u16::from_le_bytes(idat[offset + 2..offset + 4].try_into().unwrap());
        assert_eq!(length ^ complement, u16::MAX);
        offset += 4;
        let end = offset + usize::from(length);
        output.extend_from_slice(&idat[offset..end]);
        offset = end;
        if final_block {
            break;
        }
    }
    assert_eq!(offset + 4, idat.len(), "zlib stream ends with Adler-32");
    output
}

#[test]
fn writes_deterministic_rgba8_with_transparent_empty_pixels() {
    let raster = raster();
    let plan = PngPlan::new(&raster).unwrap();
    let mut first = Vec::new();
    plan.write(&mut first, &Cancellation::default()).unwrap();
    let mut second = Vec::new();
    plan.write(&mut second, &Cancellation::default()).unwrap();
    assert_eq!(first, second);
    assert_eq!(u64::try_from(first.len()).unwrap(), plan.output_bytes());

    let chunks = chunks(&first);
    assert_eq!(chunks.first().unwrap().0, b"IHDR");
    assert_eq!(
        chunks.first().unwrap().1,
        &[0, 0, 0, 3, 0, 0, 0, 1, 8, 6, 0, 0, 0]
    );
    assert_eq!(*chunks.last().unwrap(), (b"IEND", &[][..]));
    let compressed: Vec<u8> = chunks
        .iter()
        .filter(|(kind, _)| *kind == b"IDAT")
        .flat_map(|(_, data)| data.iter().copied())
        .collect();
    assert_eq!(
        stored_zlib_payload(&compressed),
        [
            0, // PNG filter: None
            0, 0, 0, 255, // occupied black
            0, 0, 0, 0, // empty and transparent
            255, 255, 255, 255, // occupied white
        ]
    );
}

#[test]
fn cancellation_before_encoding_writes_nothing() {
    let raster = raster();
    let plan = PngPlan::new(&raster).unwrap();
    let cancellation = Cancellation::default();
    cancellation.cancel();
    let mut output = Vec::new();
    assert!(plan.write(&mut output, &cancellation).is_err());
    assert!(output.is_empty());
}
