#![no_main]

use std::sync::Arc;

use libfuzzer_sys::fuzz_target;
use pcx_cli::{
    core::point::{
        PointBatch, PointColumn, PointDimensions, PointField, PointFrameMetadata, PointSchema,
        PrimitiveType, Timestamp,
    },
    pcd::{self, Encoding},
};

fn primitive(selector: u8) -> PrimitiveType {
    match selector % 8 {
        0 => PrimitiveType::I8,
        1 => PrimitiveType::U8,
        2 => PrimitiveType::I16,
        3 => PrimitiveType::U16,
        4 => PrimitiveType::I32,
        5 => PrimitiveType::U32,
        6 => PrimitiveType::F32,
        _ => PrimitiveType::F64,
    }
}

fn empty_column(primitive: PrimitiveType) -> PointColumn {
    match primitive {
        PrimitiveType::I8 => PointColumn::I8(Vec::new()),
        PrimitiveType::U8 => PointColumn::U8(Vec::new()),
        PrimitiveType::I16 => PointColumn::I16(Vec::new()),
        PrimitiveType::U16 => PointColumn::U16(Vec::new()),
        PrimitiveType::I32 => PointColumn::I32(Vec::new()),
        PrimitiveType::U32 => PointColumn::U32(Vec::new()),
        PrimitiveType::F32 => PointColumn::F32(Vec::new()),
        PrimitiveType::F64 => PointColumn::F64(Vec::new()),
        PrimitiveType::I64 | PrimitiveType::U64 => unreachable!(),
    }
}

fuzz_target!(|data: &[u8]| {
    let mut fields = Vec::new();
    let mut columns = Vec::new();
    for (index, chunk) in data.chunks(2).take(32).enumerate() {
        let selector = chunk[0];
        let primitive = primitive(selector);
        let count = chunk.get(1).map_or(1, |count| usize::from(*count % 8) + 1);
        let field = PointField::new(format!("field_{index}_{selector}"), primitive, count, None)
            .expect("generated field is valid");
        fields.push(field);
        columns.push(empty_column(primitive));
    }
    if fields.is_empty() {
        fields.push(PointField::new("x", PrimitiveType::F32, 1, None).unwrap());
        columns.push(PointColumn::F32(Vec::new()));
    }

    let batch = PointBatch::new(
        Arc::new(PointSchema::new(fields).unwrap()),
        Arc::new(PointFrameMetadata::new(
            Timestamp::new(0, 0).unwrap(),
            "map",
            true,
        )),
        PointDimensions::new(0, 1).unwrap(),
        columns,
    )
    .unwrap();
    let encoding = if data.first().is_some_and(|byte| byte & 1 == 0) {
        Encoding::Binary
    } else {
        Encoding::Ascii
    };
    let mut output = Vec::new();
    pcd::write(&mut output, &batch, encoding).unwrap();
    std::hint::black_box(output);
});
