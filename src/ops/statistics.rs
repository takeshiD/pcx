use std::{error::Error, fmt};

use serde::{Serialize, Serializer};

use crate::core::{
    Determinism, InputCapabilities, Materialization, MetadataEffect, OperatorBehavior,
    OperatorContract, OperatorInput, OperatorOutput, Ordering, OutputRepresentation, OutputSchema,
    PointCountEffect, ScratchMemory, ValueEffect,
    point::{AccessError, PointFieldView, PointValue, PointView, PrimitiveType},
};

/// Current schema for a machine-readable frame statistics report.
pub const STATISTICS_REPORT_SCHEMA_VERSION: u32 = 1;

/// Statistics for exactly one Point Frame.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StatisticsReport {
    schema_version: u32,
    point_count: u64,
    fields: Box<[FieldStatistics]>,
}

impl StatisticsReport {
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub const fn point_count(&self) -> u64 {
        self.point_count
    }

    pub fn fields(&self) -> &[FieldStatistics] {
        &self.fields
    }
}

/// Counts and finite range for every scalar in one Point Field.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FieldStatistics {
    name: String,
    #[serde(serialize_with = "serialize_primitive")]
    primitive: PrimitiveType,
    element_count: u64,
    scalar_count: u64,
    finite_count: u64,
    nan_count: u64,
    infinity_count: u64,
    positive_infinity_count: u64,
    negative_infinity_count: u64,
    finite_range: Option<FiniteRange>,
}

impl FieldStatistics {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn primitive(&self) -> PrimitiveType {
        self.primitive
    }

    pub const fn element_count(&self) -> u64 {
        self.element_count
    }

    pub const fn scalar_count(&self) -> u64 {
        self.scalar_count
    }

    pub const fn finite_count(&self) -> u64 {
        self.finite_count
    }

    pub const fn nan_count(&self) -> u64 {
        self.nan_count
    }

    pub const fn infinity_count(&self) -> u64 {
        self.infinity_count
    }

    pub const fn positive_infinity_count(&self) -> u64 {
        self.positive_infinity_count
    }

    pub const fn negative_infinity_count(&self) -> u64 {
        self.negative_infinity_count
    }

    pub const fn finite_range(&self) -> Option<&FiniteRange> {
        self.finite_range.as_ref()
    }
}

/// Inclusive range over only the finite values of one Point Field.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FiniteRange {
    minimum: NumericValue,
    maximum: NumericValue,
}

impl FiniteRange {
    pub const fn minimum(&self) -> NumericValue {
        self.minimum
    }

    pub const fn maximum(&self) -> NumericValue {
        self.maximum
    }
}

/// An exact integer or finite floating-point report value.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum NumericValue {
    I64(i64),
    U64(u64),
    F64(f64),
}

/// Failure to represent counts, reserve report storage, or read a validated view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatisticsError {
    CountOverflow,
    AllocationFailed,
    Access(AccessError),
}

impl fmt::Display for StatisticsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl Error for StatisticsError {}

impl From<AccessError> for StatisticsError {
    fn from(error: AccessError) -> Self {
        Self::Access(error)
    }
}

/// Declare lossless view-based inspection with no materialization or scratch.
pub fn contract() -> OperatorContract {
    OperatorContract::new(
        "statistics",
        OperatorInput::new(InputCapabilities::View, [], Materialization::None),
        OperatorOutput::new(
            OutputSchema::Preserve,
            OutputRepresentation::Preserve,
            PointCountEffect::Preserve,
            MetadataEffect::Preserve,
            ValueEffect::Preserve,
        ),
        OperatorBehavior::new(
            [],
            Ordering::Preserve,
            Determinism::Deterministic,
            ScratchMemory::fixed(0),
        ),
    )
}

/// Inspect one Point Frame without materializing or changing its point data.
pub fn inspect(view: &PointView) -> Result<StatisticsReport, StatisticsError> {
    let point_count = u64::try_from(view.layout().dimensions().point_count())
        .map_err(|_| StatisticsError::CountOverflow)?;
    let mut fields = Vec::new();
    fields
        .try_reserve_exact(view.schema().fields().len())
        .map_err(|_| StatisticsError::AllocationFailed)?;

    for field in view.schema().fields() {
        let field_view = view
            .field(field.name())
            .expect("field belongs to this view's validated schema");
        fields.push(inspect_field(field_view)?);
    }

    Ok(StatisticsReport {
        schema_version: STATISTICS_REPORT_SCHEMA_VERSION,
        point_count,
        fields: fields.into_boxed_slice(),
    })
}

fn inspect_field(view: PointFieldView<'_>) -> Result<FieldStatistics, StatisticsError> {
    let field = view.field();
    let element_count = u64::try_from(field.count()).map_err(|_| StatisticsError::CountOverflow)?;
    let point_count =
        u64::try_from(view.point_count()).map_err(|_| StatisticsError::CountOverflow)?;
    let scalar_count = point_count
        .checked_mul(element_count)
        .ok_or(StatisticsError::CountOverflow)?;

    let mut counts = Counts::default();
    let finite_range = match field.primitive() {
        PrimitiveType::I8 => signed_range(view, &mut counts, |value| match value {
            PointValue::I8(value) => i64::from(value),
            _ => unreachable!(),
        })?,
        PrimitiveType::I16 => signed_range(view, &mut counts, |value| match value {
            PointValue::I16(value) => i64::from(value),
            _ => unreachable!(),
        })?,
        PrimitiveType::I32 => signed_range(view, &mut counts, |value| match value {
            PointValue::I32(value) => i64::from(value),
            _ => unreachable!(),
        })?,
        PrimitiveType::I64 => signed_range(view, &mut counts, |value| match value {
            PointValue::I64(value) => value,
            _ => unreachable!(),
        })?,
        PrimitiveType::U8 => unsigned_range(view, &mut counts, |value| match value {
            PointValue::U8(value) => u64::from(value),
            _ => unreachable!(),
        })?,
        PrimitiveType::U16 => unsigned_range(view, &mut counts, |value| match value {
            PointValue::U16(value) => u64::from(value),
            _ => unreachable!(),
        })?,
        PrimitiveType::U32 => unsigned_range(view, &mut counts, |value| match value {
            PointValue::U32(value) => u64::from(value),
            _ => unreachable!(),
        })?,
        PrimitiveType::U64 => unsigned_range(view, &mut counts, |value| match value {
            PointValue::U64(value) => value,
            _ => unreachable!(),
        })?,
        PrimitiveType::F32 => float_range(view, &mut counts, |value| match value {
            PointValue::F32(value) => f64::from(value),
            _ => unreachable!(),
        })?,
        PrimitiveType::F64 => float_range(view, &mut counts, |value| match value {
            PointValue::F64(value) => value,
            _ => unreachable!(),
        })?,
    };
    let infinity_count = counts
        .positive_infinity
        .checked_add(counts.negative_infinity)
        .ok_or(StatisticsError::CountOverflow)?;

    Ok(FieldStatistics {
        name: field.name().to_owned(),
        primitive: field.primitive(),
        element_count,
        scalar_count,
        finite_count: counts.finite,
        nan_count: counts.nan,
        infinity_count,
        positive_infinity_count: counts.positive_infinity,
        negative_infinity_count: counts.negative_infinity,
        finite_range,
    })
}

#[derive(Default)]
struct Counts {
    finite: u64,
    nan: u64,
    positive_infinity: u64,
    negative_infinity: u64,
}

fn increment(count: &mut u64) -> Result<(), StatisticsError> {
    *count = count.checked_add(1).ok_or(StatisticsError::CountOverflow)?;
    Ok(())
}

fn visit_values(
    view: PointFieldView<'_>,
    mut visit: impl FnMut(PointValue) -> Result<(), StatisticsError>,
) -> Result<(), StatisticsError> {
    for point in 0..view.point_count() {
        for element in 0..view.field().count() {
            visit(view.value(point, element)?)?;
        }
    }
    Ok(())
}

fn signed_range(
    view: PointFieldView<'_>,
    counts: &mut Counts,
    convert: impl Fn(PointValue) -> i64,
) -> Result<Option<FiniteRange>, StatisticsError> {
    let mut range: Option<(i64, i64)> = None;
    visit_values(view, |value| {
        let value = convert(value);
        increment(&mut counts.finite)?;
        range = Some(match range {
            Some((minimum, maximum)) => (minimum.min(value), maximum.max(value)),
            None => (value, value),
        });
        Ok(())
    })?;
    Ok(range.map(|(minimum, maximum)| FiniteRange {
        minimum: NumericValue::I64(minimum),
        maximum: NumericValue::I64(maximum),
    }))
}

fn unsigned_range(
    view: PointFieldView<'_>,
    counts: &mut Counts,
    convert: impl Fn(PointValue) -> u64,
) -> Result<Option<FiniteRange>, StatisticsError> {
    let mut range: Option<(u64, u64)> = None;
    visit_values(view, |value| {
        let value = convert(value);
        increment(&mut counts.finite)?;
        range = Some(match range {
            Some((minimum, maximum)) => (minimum.min(value), maximum.max(value)),
            None => (value, value),
        });
        Ok(())
    })?;
    Ok(range.map(|(minimum, maximum)| FiniteRange {
        minimum: NumericValue::U64(minimum),
        maximum: NumericValue::U64(maximum),
    }))
}

fn float_range(
    view: PointFieldView<'_>,
    counts: &mut Counts,
    convert: impl Fn(PointValue) -> f64,
) -> Result<Option<FiniteRange>, StatisticsError> {
    let mut range: Option<(f64, f64)> = None;
    visit_values(view, |value| {
        let value = convert(value);
        if value.is_nan() {
            increment(&mut counts.nan)?;
        } else if value == f64::INFINITY {
            increment(&mut counts.positive_infinity)?;
        } else if value == f64::NEG_INFINITY {
            increment(&mut counts.negative_infinity)?;
        } else {
            increment(&mut counts.finite)?;
            let value = if value == 0.0 { 0.0 } else { value };
            range = Some(match range {
                Some((minimum, maximum)) => (minimum.min(value), maximum.max(value)),
                None => (value, value),
            });
        }
        Ok(())
    })?;
    Ok(range.map(|(minimum, maximum)| FiniteRange {
        minimum: NumericValue::F64(minimum),
        maximum: NumericValue::F64(maximum),
    }))
}

fn primitive_name(primitive: PrimitiveType) -> &'static str {
    match primitive {
        PrimitiveType::I8 => "i8",
        PrimitiveType::U8 => "u8",
        PrimitiveType::I16 => "i16",
        PrimitiveType::U16 => "u16",
        PrimitiveType::I32 => "i32",
        PrimitiveType::U32 => "u32",
        PrimitiveType::I64 => "i64",
        PrimitiveType::U64 => "u64",
        PrimitiveType::F32 => "f32",
        PrimitiveType::F64 => "f64",
    }
}

fn serialize_primitive<S>(primitive: &PrimitiveType, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(primitive_name(*primitive))
}

impl fmt::Display for NumericValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::I64(value) => value.fmt(formatter),
            Self::U64(value) => value.fmt(formatter),
            Self::F64(value) => value.fmt(formatter),
        }
    }
}

impl fmt::Display for StatisticsReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "points: {}", self.point_count)?;
        for field in &self.fields {
            write!(
                formatter,
                "\nfield {} {}[{}]: scalars={} finite={} nan={} infinity={} (+{}/-{}) range=",
                field.name,
                primitive_name(field.primitive),
                field.element_count,
                field.scalar_count,
                field.finite_count,
                field.nan_count,
                field.infinity_count,
                field.positive_infinity_count,
                field.negative_infinity_count,
            )?;
            match &field.finite_range {
                Some(range) => write!(formatter, "[{}, {}]", range.minimum, range.maximum)?,
                None => formatter.write_str("empty")?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use proptest::prelude::*;
    use serde_json::json;

    use super::*;
    use crate::{
        core::{
            LossPolicy, Planner, PointCountEffect, PointRepresentation,
            point::{
                Endianness, PointDimensions, PointField, PointFrameMetadata, PointLayout,
                PointSchema, PrimitiveType, Timestamp,
            },
        },
        ops::contract_tests::{ContractTestCase, assert_frame_local_contract},
    };

    fn metadata(frame: &str) -> Arc<PointFrameMetadata> {
        Arc::new(PointFrameMetadata::new(
            Timestamp::new(0, 0).unwrap(),
            frame,
            false,
        ))
    }

    fn f64_view(frame: &str, values: &[f64]) -> crate::core::point::PointView {
        let schema = Arc::new(
            PointSchema::new(vec![
                PointField::new("x", PrimitiveType::F64, 1, None).unwrap(),
            ])
            .unwrap(),
        );
        let dimensions = PointDimensions::new(values.len(), 1).unwrap();
        let layout = PointLayout::new(
            schema,
            dimensions,
            vec![0],
            8,
            values.len() * 8,
            0,
            Endianness::Little,
        )
        .unwrap();
        let source = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        crate::core::point::PointView::new(Arc::from(source), metadata(frame), layout).unwrap()
    }

    #[test]
    fn mixed_values_have_exact_matching_human_and_json_statistics() {
        let view = f64_view(
            "lidar",
            &[3.5, f64::NAN, f64::INFINITY, -2.0, f64::NEG_INFINITY, -0.0],
        );

        let report = inspect(&view).unwrap();

        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            json!({
                "schema_version": 1,
                "point_count": 6,
                "fields": [{
                    "name": "x",
                    "primitive": "f64",
                    "element_count": 1,
                    "scalar_count": 6,
                    "finite_count": 3,
                    "nan_count": 1,
                    "infinity_count": 2,
                    "positive_infinity_count": 1,
                    "negative_infinity_count": 1,
                    "finite_range": {"minimum": -2.0, "maximum": 3.5}
                }]
            })
        );
        assert_eq!(
            report.to_string(),
            "points: 6\nfield x f64[1]: scalars=6 finite=3 nan=1 infinity=2 (+1/-1) range=[-2, 3.5]"
        );
    }

    #[test]
    fn empty_frame_has_zero_counts_and_no_finite_range() {
        let report = inspect(&f64_view("empty", &[])).unwrap();
        let field = &report.fields()[0];

        assert_eq!(report.point_count(), 0);
        assert_eq!(field.scalar_count(), 0);
        assert_eq!(field.finite_count(), 0);
        assert_eq!(field.nan_count(), 0);
        assert_eq!(field.infinity_count(), 0);
        assert_eq!(field.finite_range(), None);
        assert!(report.to_string().ends_with("range=empty"));
    }

    #[test]
    fn report_is_frame_local() {
        let first = inspect(&f64_view("first", &[1.0, 2.0])).unwrap();
        let second = inspect(&f64_view("second", &[100.0])).unwrap();

        assert_eq!(first.point_count(), 2);
        assert_eq!(
            first.fields()[0].finite_range().unwrap().maximum(),
            NumericValue::F64(2.0)
        );
        assert_eq!(second.point_count(), 1);
        assert_eq!(
            second.fields()[0].finite_range().unwrap().minimum(),
            NumericValue::F64(100.0)
        );
    }

    #[test]
    fn statistics_contract_preserves_data_with_no_materialization_or_scratch() {
        let accepted = Arc::new(
            PointSchema::new(vec![
                PointField::new("samples", PrimitiveType::I16, 2, None).unwrap(),
            ])
            .unwrap(),
        );
        let contract = contract();
        assert_frame_local_contract(ContractTestCase {
            contract: contract.clone(),
            accepted_schema: Arc::clone(&accepted),
            rejected_schema: None,
            dimensions: PointDimensions::new(4, 1).unwrap(),
            input_representation: PointRepresentation::View,
            authorized_losses: &[],
            expected_output_fields: &["samples"],
            expected_materialized_fields: &[],
            expected_scratch_bytes: 0,
            expected_output_representation: PointRepresentation::View,
            expected_point_count: PointCountEffect::Preserve,
            expected_ordering: crate::core::Ordering::Preserve,
        });
        let plan = Planner::new()
            .validate_operators(
                accepted,
                PointDimensions::new(4, 1).unwrap(),
                PointRepresentation::View,
                &[contract],
                &LossPolicy::lossless(),
            )
            .unwrap();
        assert_eq!(plan.materialization_bytes(), 0);
        assert_eq!(plan.peak_scratch_bytes(), 0);
    }

    #[test]
    fn integer_extrema_and_multi_element_fields_remain_exact() {
        let schema = Arc::new(
            PointSchema::new(vec![
                PointField::new("signed", PrimitiveType::I64, 2, None).unwrap(),
                PointField::new("unsigned", PrimitiveType::U64, 1, None).unwrap(),
            ])
            .unwrap(),
        );
        let dimensions = PointDimensions::new(1, 1).unwrap();
        let layout = PointLayout::new(
            schema,
            dimensions,
            vec![0, 16],
            24,
            24,
            0,
            Endianness::Little,
        )
        .unwrap();
        let mut source = Vec::new();
        source.extend_from_slice(&i64::MIN.to_le_bytes());
        source.extend_from_slice(&i64::MAX.to_le_bytes());
        source.extend_from_slice(&u64::MAX.to_le_bytes());
        let original = source.clone();
        let view = crate::core::point::PointView::new(
            Arc::from(source),
            metadata("integer-extrema"),
            layout,
        )
        .unwrap();

        let report = inspect(&view).unwrap();

        assert_eq!(report.fields()[0].element_count(), 2);
        assert_eq!(report.fields()[0].scalar_count(), 2);
        assert_eq!(
            report.fields()[0].finite_range(),
            Some(&FiniteRange {
                minimum: NumericValue::I64(i64::MIN),
                maximum: NumericValue::I64(i64::MAX),
            })
        );
        assert_eq!(
            report.fields()[1].finite_range(),
            Some(&FiniteRange {
                minimum: NumericValue::U64(u64::MAX),
                maximum: NumericValue::U64(u64::MAX),
            })
        );
        assert_eq!(
            view.field("signed").unwrap().raw(0, 0).unwrap(),
            &original[..8]
        );
        assert_eq!(
            view.field("unsigned").unwrap().raw(0, 0).unwrap(),
            &original[16..]
        );
    }

    #[test]
    fn every_primitive_type_is_counted_and_ranged() {
        let definitions = [
            ("i8", PrimitiveType::I8, vec![i8::MIN as u8]),
            ("u8", PrimitiveType::U8, vec![u8::MAX]),
            ("i16", PrimitiveType::I16, i16::MIN.to_le_bytes().to_vec()),
            ("u16", PrimitiveType::U16, u16::MAX.to_le_bytes().to_vec()),
            ("i32", PrimitiveType::I32, i32::MIN.to_le_bytes().to_vec()),
            ("u32", PrimitiveType::U32, u32::MAX.to_le_bytes().to_vec()),
            ("i64", PrimitiveType::I64, i64::MIN.to_le_bytes().to_vec()),
            ("u64", PrimitiveType::U64, u64::MAX.to_le_bytes().to_vec()),
            (
                "f32",
                PrimitiveType::F32,
                (-1.25_f32).to_le_bytes().to_vec(),
            ),
            ("f64", PrimitiveType::F64, 2.5_f64.to_le_bytes().to_vec()),
        ];
        let schema = Arc::new(
            PointSchema::new(
                definitions
                    .iter()
                    .map(|(name, primitive, _)| {
                        PointField::new(*name, *primitive, 1, None).unwrap()
                    })
                    .collect(),
            )
            .unwrap(),
        );
        let mut source = Vec::new();
        let mut offsets = Vec::new();
        for (_, _, bytes) in &definitions {
            offsets.push(source.len());
            source.extend_from_slice(bytes);
        }
        let point_step = source.len();
        let layout = PointLayout::new(
            schema,
            PointDimensions::new(1, 1).unwrap(),
            offsets,
            point_step,
            point_step,
            0,
            Endianness::Little,
        )
        .unwrap();
        let view = crate::core::point::PointView::new(
            Arc::from(source),
            metadata("all-primitives"),
            layout,
        )
        .unwrap();

        let report = inspect(&view).unwrap();

        assert_eq!(report.fields().len(), definitions.len());
        assert!(report.fields().iter().all(|field| {
            field.scalar_count() == 1 && field.finite_count() == 1 && field.finite_range().is_some()
        }));
    }

    proptest! {
        #[test]
        fn finite_range_and_counts_do_not_depend_on_value_order(mut values in prop::collection::vec(any::<f64>(), 0..128)) {
            let before = inspect(&f64_view("property", &values)).unwrap();
            values.reverse();
            let after = inspect(&f64_view("property", &values)).unwrap();
            prop_assert_eq!(before, after);
        }
    }
}
