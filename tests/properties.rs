use std::sync::Arc;

use pcx_cli::{
    core::{
        ByteBound, Destination, FrameSelector, JobSpec, PipelineMemoryRequirements, Planner,
        SourceSpec,
        point::{
            Endianness, LayoutError, PointBatch, PointDimensions, PointField, PointFrameMetadata,
            PointLayout, PointSchema, PrimitiveType, SchemaError, Timestamp,
        },
    },
    pcd::{self, Encoding},
};
use proptest::{collection::btree_set, prelude::*};

fn primitive(index: u8) -> PrimitiveType {
    match index % 10 {
        0 => PrimitiveType::I8,
        1 => PrimitiveType::U8,
        2 => PrimitiveType::I16,
        3 => PrimitiveType::U16,
        4 => PrimitiveType::I32,
        5 => PrimitiveType::U32,
        6 => PrimitiveType::I64,
        7 => PrimitiveType::U64,
        8 => PrimitiveType::F32,
        _ => PrimitiveType::F64,
    }
}

fn pcd_primitive(index: u8) -> PrimitiveType {
    match index % 8 {
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

fn planner_accepts(retained_input: u64, limit: u64) -> bool {
    let job = JobSpec::extract(
        SourceSpec::file("recording.mcap").unwrap(),
        "/lidar/points",
        FrameSelector::Index(0),
        Destination::stdout(),
    )
    .unwrap();
    let requirements = PipelineMemoryRequirements::new(
        ByteBound::bounded(retained_input),
        ByteBound::bounded(0),
        ByteBound::bounded(0),
        ByteBound::bounded(0),
        ByteBound::bounded(0),
        ByteBound::bounded(0),
    );
    Planner::new().plan(job, requirements, limit).is_ok()
}

proptest! {
    #[test]
    fn point_dimensions_match_checked_size_arithmetic(width: usize, height: usize) {
        let actual = PointDimensions::new(width, height);
        if height == 0 {
            prop_assert_eq!(actual, Err(LayoutError::ZeroHeight));
        } else if let Some(point_count) = width.checked_mul(height) {
            let dimensions = actual.expect("a representable point count must be accepted");
            prop_assert_eq!(dimensions.width(), width);
            prop_assert_eq!(dimensions.height(), height);
            prop_assert_eq!(dimensions.point_count(), point_count);
        } else {
            prop_assert_eq!(
                actual,
                Err(LayoutError::PointCountOverflow { width, height })
            );
        }
    }

    #[test]
    fn valid_layout_extents_are_exact_and_padding_monotone(
        width in 0usize..=1_000_000,
        height in 1usize..=1_000,
        point_step in 1usize..=128,
        row_padding in 0usize..=128,
        data_offset in 0usize..=1_024,
    ) {
        let dimensions = PointDimensions::new(width, height).unwrap();
        let row_data_len = width.checked_mul(point_step).unwrap();
        let row_step = row_data_len.checked_add(row_padding).unwrap();
        let required = data_offset
            .checked_add(height.checked_mul(row_step).unwrap())
            .unwrap();
        let schema = Arc::new(PointSchema::new(Vec::new()).unwrap());
        let layout = PointLayout::new(
            schema,
            dimensions,
            Vec::new(),
            point_step,
            row_step,
            data_offset,
            Endianness::Little,
        ).unwrap();

        prop_assert_eq!(layout.required_source_len(), required);
        prop_assert_eq!(layout.row_step(), row_step);
        if row_data_len > 0 {
            let rejected = matches!(
                PointLayout::new(
                    Arc::new(PointSchema::new(Vec::new()).unwrap()),
                    dimensions,
                    Vec::new(),
                    point_step,
                    row_data_len - 1,
                    data_offset,
                    Endianness::Little,
                ),
                Err(LayoutError::RowStepTooSmall { minimum, .. }) if minimum == row_data_len
            );
            prop_assert!(rejected, "a row shorter than its point data must be rejected");
        }
    }

    #[test]
    fn planner_acceptance_is_monotone(
        peak: u64,
        limit: u64,
        extra_peak: u64,
        extra_limit: u64,
    ) {
        let accepted = planner_accepts(peak, limit);

        if accepted {
            let larger_limit = limit.saturating_add(extra_limit);
            let smaller_peak = peak.saturating_sub(extra_peak);
            prop_assert!(planner_accepts(peak, larger_limit));
            prop_assert!(planner_accepts(smaller_peak, limit));
        } else {
            let larger_peak = peak.saturating_add(extra_peak);
            let smaller_limit = limit.saturating_sub(extra_limit);
            prop_assert!(!planner_accepts(larger_peak, limit));
            prop_assert!(!planner_accepts(peak, smaller_limit));
        }
    }

    #[test]
    fn schemas_preserve_unique_order_and_reject_duplicates(
        definitions in btree_set(0u8..32, 1..=16),
    ) {
        let fields = definitions
            .iter()
            .map(|name| {
                PointField::new(
                    format!("field_{name}"),
                    primitive(*name),
                    usize::from(*name % 16 + 1),
                    None,
                ).unwrap()
            })
            .collect::<Vec<_>>();
        let schema = PointSchema::new(fields.clone()).unwrap();

        prop_assert_eq!(schema.fields(), fields.as_slice());
        for field in &fields {
            prop_assert_eq!(schema.field(field.name()), Some(field));
        }

        let duplicate = fields[0].clone();
        let duplicate_name = duplicate.name().to_owned();
        let mut invalid = fields;
        invalid.push(duplicate);
        prop_assert_eq!(
            PointSchema::new(invalid),
            Err(SchemaError::DuplicateFieldName { name: duplicate_name })
        );
    }

    #[test]
    fn pcd_header_columns_follow_the_validated_schema(
        definitions in btree_set(0u8..32, 1..=16),
        encoding in prop_oneof![Just(Encoding::Binary), Just(Encoding::Ascii)],
    ) {
        let fields = definitions
            .iter()
            .map(|name| {
                PointField::new(
                    format!("field_{name}"),
                    pcd_primitive(*name),
                    usize::from(*name % 16 + 1),
                    None,
                ).unwrap()
            })
            .collect::<Vec<_>>();
        let expected_names = fields
            .iter()
            .map(|field| field.name().to_owned())
            .collect::<Vec<_>>();
        let field_count = fields.len();
        let batch = PointBatch::new(
            Arc::new(PointSchema::new(fields).unwrap()),
            Arc::new(PointFrameMetadata::new(Timestamp::new(0, 0).unwrap(), "map", true)),
            PointDimensions::new(0, 1).unwrap(),
            definitions.iter().map(|primitive_index| match pcd_primitive(*primitive_index) {
                PrimitiveType::I8 => pcx_cli::core::point::PointColumn::I8(Vec::new()),
                PrimitiveType::U8 => pcx_cli::core::point::PointColumn::U8(Vec::new()),
                PrimitiveType::I16 => pcx_cli::core::point::PointColumn::I16(Vec::new()),
                PrimitiveType::U16 => pcx_cli::core::point::PointColumn::U16(Vec::new()),
                PrimitiveType::I32 => pcx_cli::core::point::PointColumn::I32(Vec::new()),
                PrimitiveType::U32 => pcx_cli::core::point::PointColumn::U32(Vec::new()),
                PrimitiveType::F32 => pcx_cli::core::point::PointColumn::F32(Vec::new()),
                PrimitiveType::F64 => pcx_cli::core::point::PointColumn::F64(Vec::new()),
                PrimitiveType::I64 | PrimitiveType::U64 => unreachable!(),
            }).collect(),
        ).unwrap();
        let mut output = Vec::new();
        pcd::write(&mut output, &batch, encoding).unwrap();
        let header = String::from_utf8(output).unwrap();
        let lines = header.lines().collect::<Vec<_>>();
        let names = lines.iter().find(|line| line.starts_with("FIELDS ")).unwrap()
            .split_whitespace().skip(1).collect::<Vec<_>>();

        prop_assert_eq!(names, expected_names);
        for keyword in ["SIZE ", "TYPE ", "COUNT "] {
            let columns = lines.iter().find(|line| line.starts_with(keyword)).unwrap()
                .split_whitespace().skip(1).count();
            prop_assert_eq!(columns, field_count);
        }
        prop_assert!(header.contains("WIDTH 0\nHEIGHT 1\n"));
        prop_assert!(header.contains("POINTS 0\n"));
    }
}
