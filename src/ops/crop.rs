//! Deterministic frame-local axis-aligned cropping.
//!
//! Each axis uses a half-open interval: the minimum is inclusive and the
//! maximum is exclusive (`min <= coordinate < max`). Bounds must be finite and
//! strictly increasing. A point with a NaN or infinity in any coordinate is
//! excluded. Cropping is stable: retained points remain in input order.

use std::{error::Error as StdError, fmt, mem::size_of};

use crate::core::{
    Determinism, ElementCountRequirement, ErrorCategory, FidelityLoss, FieldRequirement,
    FieldSelector, InputCapabilities, LossPolicy, Materialization, MetadataEffect,
    OperatorBehavior, OperatorContract, OperatorInput, OperatorOutput, Ordering,
    OutputRepresentation, OutputSchema, Planner, PointCountEffect, PointRepresentation,
    PrimitiveRequirement, ScratchMemory, ValidatedOperatorPipeline, ValueEffect,
    point::{
        AccessError, BatchError, PointBatch, PointColumn, PointDimensions, PointField,
        PointFieldSemantic, PointFieldView, PointSchema, PointValue, PointView, PrimitiveType,
    },
};

const COORDINATES: [PointFieldSemantic; 3] = [
    PointFieldSemantic::X,
    PointFieldSemantic::Y,
    PointFieldSemantic::Z,
];

/// Finite half-open bounds for X, Y, and Z, respectively.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CropBounds {
    min: [f64; 3],
    max: [f64; 3],
}

impl CropBounds {
    /// Creates `[min, max)` bounds. Every value must be finite and `min < max`.
    pub fn new(min: [f64; 3], max: [f64; 3]) -> Result<Self, CropError> {
        for (axis, (&minimum, &maximum)) in min.iter().zip(&max).enumerate() {
            if !minimum.is_finite() || !maximum.is_finite() {
                return Err(CropError::NonFiniteBound { axis });
            }
            if minimum >= maximum {
                return Err(CropError::EmptyOrReversedBounds { axis });
            }
        }
        Ok(Self { min, max })
    }

    pub const fn min(self) -> [f64; 3] {
        self.min
    }

    pub const fn max(self) -> [f64; 3] {
        self.max
    }

    fn contains(self, coordinate: [f64; 3]) -> bool {
        coordinate.into_iter().enumerate().all(|(axis, value)| {
            value.is_finite() && self.min[axis] <= value && value < self.max[axis]
        })
    }
}

/// A stable, frame-local axis-aligned crop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisAlignedCrop {
    bounds: CropBounds,
}

impl AxisAlignedCrop {
    pub const fn new(bounds: CropBounds) -> Self {
        Self { bounds }
    }

    pub const fn bounds(self) -> CropBounds {
        self.bounds
    }

    /// Declares coordinate materialization, worst-case output, and one byte of
    /// selection scratch per input point before execution begins.
    pub fn contract() -> OperatorContract {
        OperatorContract::new(
            "axis-aligned-crop",
            OperatorInput::new(
                InputCapabilities::ViewOrColumns,
                COORDINATES.map(|semantic| {
                    FieldRequirement::new(
                        FieldSelector::semantic(semantic),
                        PrimitiveRequirement::one_of([PrimitiveType::F32, PrimitiveType::F64]),
                        ElementCountRequirement::Exactly(1),
                    )
                }),
                Materialization::fields(COORDINATES.map(FieldSelector::semantic)),
            ),
            OperatorOutput::new(
                OutputSchema::Preserve,
                OutputRepresentation::Columns,
                PointCountEffect::AtMostInput,
                MetadataEffect::Preserve,
                ValueEffect::Preserve,
            ),
            OperatorBehavior::new(
                [FidelityLoss::PointSelection],
                Ordering::Preserve,
                Determinism::Deterministic,
                ScratchMemory::per_input_point(0, 1),
            ),
        )
    }

    /// Validates schema, representation, loss authorization, materialization,
    /// worst-case output allocation, and scratch memory before execution.
    pub fn plan(
        self,
        input_schema: std::sync::Arc<PointSchema>,
        dimensions: PointDimensions,
        input_representation: PointRepresentation,
        loss_policy: &LossPolicy,
    ) -> crate::core::Result<CropPlan> {
        let pipeline = Planner::new().validate_operators(
            input_schema,
            dimensions,
            input_representation,
            &[Self::contract()],
            loss_policy,
        )?;
        Ok(CropPlan {
            crop: self,
            pipeline,
            input_representation,
            dimensions,
        })
    }

    fn apply(self, input: &PointView) -> Result<PointBatch, CropError> {
        let coordinates = coordinate_fields(input)?;
        let point_count = input.layout().dimensions().point_count();
        let mut retained = Vec::new();
        retained
            .try_reserve_exact(point_count)
            .map_err(|_| CropError::AllocationFailed {
                context: "selection scratch",
                requested: point_count,
            })?;

        for point in 0..point_count {
            let coordinate = [
                coordinate_value(coordinates[0], point)?,
                coordinate_value(coordinates[1], point)?,
                coordinate_value(coordinates[2], point)?,
            ];
            retained.push(u8::from(self.bounds.contains(coordinate)));
        }
        let retained_count = retained.iter().map(|&keep| usize::from(keep)).sum();

        let mut columns = Vec::new();
        columns
            .try_reserve_exact(input.schema().fields().len())
            .map_err(|_| CropError::AllocationFailed {
                context: "output column table",
                requested: input
                    .schema()
                    .fields()
                    .len()
                    .saturating_mul(size_of::<PointColumn>()),
            })?;
        for field in input.schema().fields() {
            let view = input
                .field(field.name())
                .expect("field comes from the input's validated schema");
            columns.push(filter_field(view, &retained, retained_count)?);
        }

        let dimensions = if retained_count == point_count {
            input.layout().dimensions()
        } else {
            PointDimensions::new(retained_count, 1)
                .expect("an unorganized point count always has valid dimensions")
        };
        PointBatch::new(
            input.shared_schema(),
            input.shared_metadata(),
            dimensions,
            columns,
        )
        .map_err(CropError::InvalidOutput)
    }

    fn apply_batch(self, input: &PointBatch) -> Result<PointBatch, CropError> {
        let coordinates = coordinate_columns(input)?;
        let point_count = input.dimensions().point_count();
        let mut retained = Vec::new();
        retained
            .try_reserve_exact(point_count)
            .map_err(|_| CropError::AllocationFailed {
                context: "selection scratch",
                requested: point_count,
            })?;

        for point in 0..point_count {
            let coordinate = [
                column_coordinate_value(coordinates[0], point),
                column_coordinate_value(coordinates[1], point),
                column_coordinate_value(coordinates[2], point),
            ];
            retained.push(u8::from(self.bounds.contains(coordinate)));
        }
        let retained_count = retained.iter().map(|&keep| usize::from(keep)).sum();

        let mut columns = Vec::new();
        columns
            .try_reserve_exact(input.columns().len())
            .map_err(|_| CropError::AllocationFailed {
                context: "output column table",
                requested: input
                    .columns()
                    .len()
                    .saturating_mul(size_of::<PointColumn>()),
            })?;
        for (field, column) in input.schema().fields().iter().zip(input.columns()) {
            columns.push(filter_column(
                column,
                field.count(),
                &retained,
                retained_count,
            )?);
        }

        let dimensions = output_dimensions(input.dimensions(), retained_count);
        PointBatch::new(
            input.shared_schema(),
            input.metadata_handle(),
            dimensions,
            columns,
        )
        .map_err(CropError::InvalidOutput)
    }
}

/// A crop that passed the shared schema, fidelity, and memory checks.
#[derive(Clone, Debug, PartialEq)]
pub struct CropPlan {
    crop: AxisAlignedCrop,
    pipeline: ValidatedOperatorPipeline,
    input_representation: PointRepresentation,
    dimensions: PointDimensions,
}

impl CropPlan {
    pub const fn pipeline(&self) -> &ValidatedOperatorPipeline {
        &self.pipeline
    }

    /// Executes this plan against the validated low-copy representation.
    pub fn execute_view(&self, input: &PointView) -> Result<PointBatch, CropError> {
        self.validate_input(
            input.schema(),
            input.layout().dimensions(),
            PointRepresentation::View,
        )?;
        self.crop.apply(input)
    }

    /// Executes this plan against an already materialized representation.
    pub fn execute_batch(&self, input: &PointBatch) -> Result<PointBatch, CropError> {
        self.validate_input(
            input.schema(),
            input.dimensions(),
            PointRepresentation::Columns,
        )?;
        self.crop.apply_batch(input)
    }

    fn validate_input(
        &self,
        schema: &PointSchema,
        dimensions: PointDimensions,
        representation: PointRepresentation,
    ) -> Result<(), CropError> {
        let stage = self
            .pipeline
            .stages()
            .first()
            .expect("a crop plan always contains one stage");
        if schema != stage.input_schema()
            || dimensions != self.dimensions
            || representation != self.input_representation
        {
            return Err(CropError::InputDoesNotMatchPlan);
        }
        Ok(())
    }
}

fn coordinate_fields(input: &PointView) -> Result<[PointFieldView<'_>; 3], CropError> {
    Ok([
        coordinate_field(input, PointFieldSemantic::X)?,
        coordinate_field(input, PointFieldSemantic::Y)?,
        coordinate_field(input, PointFieldSemantic::Z)?,
    ])
}

fn coordinate_field(
    input: &PointView,
    semantic: PointFieldSemantic,
) -> Result<PointFieldView<'_>, CropError> {
    let field = coordinate_schema_field(input.schema(), semantic)?;
    Ok(input
        .field(field.name())
        .expect("coordinate belongs to the validated input schema"))
}

fn coordinate_schema_field(
    schema: &PointSchema,
    semantic: PointFieldSemantic,
) -> Result<&PointField, CropError> {
    let mut matches = schema
        .fields()
        .iter()
        .filter(|field| field.semantic() == Some(semantic));
    let field = matches
        .next()
        .ok_or(CropError::MissingCoordinate { semantic })?;
    if matches.next().is_some() {
        return Err(CropError::AmbiguousCoordinate { semantic });
    }
    if field.count() != 1 {
        return Err(CropError::NonScalarCoordinate {
            name: field.name().to_owned(),
            count: field.count(),
        });
    }
    if !matches!(field.primitive(), PrimitiveType::F32 | PrimitiveType::F64) {
        return Err(CropError::UnsupportedCoordinateType {
            name: field.name().to_owned(),
            primitive: field.primitive(),
        });
    }
    Ok(field)
}

fn coordinate_value(field: PointFieldView<'_>, point: usize) -> Result<f64, CropError> {
    match field.value(point, 0).map_err(CropError::Access)? {
        PointValue::F32(value) => Ok(f64::from(value)),
        PointValue::F64(value) => Ok(value),
        _ => unreachable!("coordinate primitive was checked before point access"),
    }
}

fn coordinate_columns(input: &PointBatch) -> Result<[&PointColumn; 3], CropError> {
    Ok([
        coordinate_column(input, PointFieldSemantic::X)?,
        coordinate_column(input, PointFieldSemantic::Y)?,
        coordinate_column(input, PointFieldSemantic::Z)?,
    ])
}

fn coordinate_column(
    input: &PointBatch,
    semantic: PointFieldSemantic,
) -> Result<&PointColumn, CropError> {
    let field = coordinate_schema_field(input.schema(), semantic)?;
    Ok(input
        .column(field.name())
        .expect("coordinate column belongs to the validated input batch"))
}

fn column_coordinate_value(column: &PointColumn, point: usize) -> f64 {
    match column {
        PointColumn::F32(values) => f64::from(values[point]),
        PointColumn::F64(values) => values[point],
        _ => unreachable!("coordinate primitive was checked before point access"),
    }
}

fn filter_field(
    field: PointFieldView<'_>,
    retained: &[u8],
    retained_count: usize,
) -> Result<PointColumn, CropError> {
    let scalar_count = retained_count
        .checked_mul(field.field().count())
        .ok_or(CropError::SizeOverflow)?;

    macro_rules! filter_column {
        ($column:ident, $value:ident, $type:ty) => {{
            let mut output: Vec<$type> = Vec::new();
            output
                .try_reserve_exact(scalar_count)
                .map_err(|_| CropError::AllocationFailed {
                    context: "output field",
                    requested: scalar_count.saturating_mul(size_of::<$type>()),
                })?;
            for (point, &keep) in retained.iter().enumerate() {
                if keep != 0 {
                    for element in 0..field.field().count() {
                        let PointValue::$value(value) =
                            field.value(point, element).map_err(CropError::Access)?
                        else {
                            unreachable!("field access follows its validated primitive type")
                        };
                        output.push(value);
                    }
                }
            }
            PointColumn::$column(output)
        }};
    }

    Ok(match field.field().primitive() {
        PrimitiveType::I8 => filter_column!(I8, I8, i8),
        PrimitiveType::U8 => filter_column!(U8, U8, u8),
        PrimitiveType::I16 => filter_column!(I16, I16, i16),
        PrimitiveType::U16 => filter_column!(U16, U16, u16),
        PrimitiveType::I32 => filter_column!(I32, I32, i32),
        PrimitiveType::U32 => filter_column!(U32, U32, u32),
        PrimitiveType::I64 => filter_column!(I64, I64, i64),
        PrimitiveType::U64 => filter_column!(U64, U64, u64),
        PrimitiveType::F32 => filter_column!(F32, F32, f32),
        PrimitiveType::F64 => filter_column!(F64, F64, f64),
    })
}

fn filter_column(
    column: &PointColumn,
    elements: usize,
    retained: &[u8],
    retained_count: usize,
) -> Result<PointColumn, CropError> {
    let scalar_count = retained_count
        .checked_mul(elements)
        .ok_or(CropError::SizeOverflow)?;

    macro_rules! filter_values {
        ($values:expr, $variant:ident, $type:ty) => {{
            let mut output: Vec<$type> = Vec::new();
            output
                .try_reserve_exact(scalar_count)
                .map_err(|_| CropError::AllocationFailed {
                    context: "output field",
                    requested: scalar_count.saturating_mul(size_of::<$type>()),
                })?;
            for (point, &keep) in retained.iter().enumerate() {
                if keep != 0 {
                    let start = point * elements;
                    output.extend_from_slice(&$values[start..start + elements]);
                }
            }
            PointColumn::$variant(output)
        }};
    }

    Ok(match column {
        PointColumn::I8(values) => filter_values!(values, I8, i8),
        PointColumn::U8(values) => filter_values!(values, U8, u8),
        PointColumn::I16(values) => filter_values!(values, I16, i16),
        PointColumn::U16(values) => filter_values!(values, U16, u16),
        PointColumn::I32(values) => filter_values!(values, I32, i32),
        PointColumn::U32(values) => filter_values!(values, U32, u32),
        PointColumn::I64(values) => filter_values!(values, I64, i64),
        PointColumn::U64(values) => filter_values!(values, U64, u64),
        PointColumn::F32(values) => filter_values!(values, F32, f32),
        PointColumn::F64(values) => filter_values!(values, F64, f64),
    })
}

fn output_dimensions(input: PointDimensions, retained_count: usize) -> PointDimensions {
    if retained_count == input.point_count() {
        input
    } else {
        PointDimensions::new(retained_count, 1)
            .expect("an unorganized point count always has valid dimensions")
    }
}

/// A deterministic crop validation or execution failure.
#[derive(Clone, Debug, PartialEq)]
pub enum CropError {
    NonFiniteBound {
        axis: usize,
    },
    EmptyOrReversedBounds {
        axis: usize,
    },
    MissingCoordinate {
        semantic: PointFieldSemantic,
    },
    AmbiguousCoordinate {
        semantic: PointFieldSemantic,
    },
    NonScalarCoordinate {
        name: String,
        count: usize,
    },
    UnsupportedCoordinateType {
        name: String,
        primitive: PrimitiveType,
    },
    InputDoesNotMatchPlan,
    SizeOverflow,
    AllocationFailed {
        context: &'static str,
        requested: usize,
    },
    Access(AccessError),
    InvalidOutput(BatchError),
}

impl CropError {
    pub const fn category(&self) -> ErrorCategory {
        match self {
            Self::NonFiniteBound { .. } | Self::EmptyOrReversedBounds { .. } => {
                ErrorCategory::Usage
            }
            Self::MissingCoordinate { .. }
            | Self::AmbiguousCoordinate { .. }
            | Self::NonScalarCoordinate { .. }
            | Self::UnsupportedCoordinateType { .. }
            | Self::InputDoesNotMatchPlan => ErrorCategory::Unsupported,
            Self::SizeOverflow | Self::AllocationFailed { .. } => ErrorCategory::Resource,
            Self::Access(_) | Self::InvalidOutput(_) => ErrorCategory::Internal,
        }
    }
}

impl fmt::Display for CropError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteBound { axis } => {
                write!(formatter, "crop axis {axis} bounds must be finite")
            }
            Self::EmptyOrReversedBounds { axis } => write!(
                formatter,
                "crop axis {axis} minimum must be less than its maximum"
            ),
            Self::MissingCoordinate { semantic } => {
                write!(formatter, "crop coordinate {semantic:?} is missing")
            }
            Self::AmbiguousCoordinate { semantic } => {
                write!(formatter, "crop coordinate {semantic:?} is ambiguous")
            }
            Self::NonScalarCoordinate { name, count } => write!(
                formatter,
                "crop coordinate field {name:?} has {count} elements; expected one"
            ),
            Self::UnsupportedCoordinateType { name, primitive } => write!(
                formatter,
                "crop coordinate field {name:?} has unsupported primitive {primitive:?}; expected F32 or F64"
            ),
            Self::InputDoesNotMatchPlan => formatter.write_str(
                "crop input does not match its validated Point Schema, dimensions, and representation",
            ),
            Self::SizeOverflow => formatter.write_str("crop output size overflowed"),
            Self::AllocationFailed { context, requested } => write!(
                formatter,
                "crop {context} allocation of {requested} bytes failed"
            ),
            Self::Access(error) => write!(formatter, "crop field access failed: {error}"),
            Self::InvalidOutput(error) => {
                write!(formatter, "crop produced an invalid point batch: {error}")
            }
        }
    }
}

impl StdError for CropError {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use proptest::prelude::*;

    use super::*;
    use crate::{
        core::{
            LossPolicy, Planner, PointRepresentation,
            point::{
                Endianness, MaterializationBudget, PointField, PointFrameMetadata, PointLayout,
                PointSchema, Timestamp,
            },
        },
        ops::contract_tests::{ContractTestCase, assert_frame_local_contract},
    };

    type TestPoint = (f32, f64, f32, u32, [i16; 2]);

    fn metadata() -> Arc<PointFrameMetadata> {
        Arc::new(
            PointFrameMetadata::new(Timestamp::new(12, 34).unwrap(), "map", false)
                .with_container_times(56, 78),
        )
    }

    fn schema(x_primitive: PrimitiveType) -> Arc<PointSchema> {
        Arc::new(
            PointSchema::new(vec![
                PointField::new("x", x_primitive, 1, Some(PointFieldSemantic::X)).unwrap(),
                PointField::new("y", PrimitiveType::F64, 1, Some(PointFieldSemantic::Y)).unwrap(),
                PointField::new("z", PrimitiveType::F32, 1, Some(PointFieldSemantic::Z)).unwrap(),
                PointField::new("id", PrimitiveType::U32, 1, None).unwrap(),
                PointField::new("pair", PrimitiveType::I16, 2, None).unwrap(),
            ])
            .unwrap(),
        )
    }

    fn view(points: &[TestPoint], dimensions: PointDimensions) -> PointView {
        assert_eq!(points.len(), dimensions.point_count());
        let schema = schema(PrimitiveType::F32);
        let point_step = 24;
        let row_step = dimensions.width() * point_step;
        let layout = PointLayout::new(
            schema,
            dimensions,
            vec![0, 4, 12, 16, 20],
            point_step,
            row_step,
            0,
            Endianness::Little,
        )
        .unwrap();
        let mut bytes = Vec::with_capacity(points.len() * point_step);
        for &(x, y, z, id, pair) in points {
            bytes.extend_from_slice(&x.to_le_bytes());
            bytes.extend_from_slice(&y.to_le_bytes());
            bytes.extend_from_slice(&z.to_le_bytes());
            bytes.extend_from_slice(&id.to_le_bytes());
            bytes.extend_from_slice(&pair[0].to_le_bytes());
            bytes.extend_from_slice(&pair[1].to_le_bytes());
        }
        PointView::new(Arc::from(bytes), metadata(), layout).unwrap()
    }

    fn unit_crop() -> AxisAlignedCrop {
        AxisAlignedCrop::new(CropBounds::new([0.0; 3], [1.0; 3]).unwrap())
    }

    fn plan_view(crop: AxisAlignedCrop, input: &PointView) -> CropPlan {
        crop.plan(
            input.shared_schema(),
            input.layout().dimensions(),
            PointRepresentation::View,
            &LossPolicy::authorize([FidelityLoss::PointSelection]),
        )
        .unwrap()
    }

    #[test]
    fn bounds_are_finite_strictly_increasing_and_half_open() {
        assert_eq!(
            CropBounds::new([f64::NAN, 0.0, 0.0], [1.0; 3]),
            Err(CropError::NonFiniteBound { axis: 0 })
        );
        assert_eq!(
            CropBounds::new([0.0; 3], [1.0, f64::INFINITY, 1.0]),
            Err(CropError::NonFiniteBound { axis: 1 })
        );
        assert_eq!(
            CropBounds::new([0.0; 3], [1.0, 0.0, 1.0]),
            Err(CropError::EmptyOrReversedBounds { axis: 1 })
        );

        let bounds = CropBounds::new([0.0; 3], [1.0; 3]).unwrap();
        assert!(bounds.contains([0.0; 3]));
        assert!(bounds.contains([f64::from_bits((-0.0_f64).to_bits()), 0.5, 0.5]));
        assert!(!bounds.contains([1.0, 0.5, 0.5]));
    }

    #[test]
    fn crop_includes_lower_excludes_upper_and_discards_non_finite_coordinates() {
        let nan_payload = f32::from_bits(0x7fc0_1234);
        let points = [
            (0.0, 0.0, 0.0, 10, [1, 2]),
            (0.5, 0.5, 0.5, 11, [3, 4]),
            (1.0, 0.5, 0.5, 12, [5, 6]),
            (nan_payload, 0.5, 0.5, 13, [7, 8]),
            (0.5, f64::INFINITY, 0.5, 14, [9, 10]),
            (0.5, 0.5, f32::NEG_INFINITY, 15, [11, 12]),
        ];
        let input = view(&points, PointDimensions::new(3, 2).unwrap());
        let output = plan_view(unit_crop(), &input).execute_view(&input).unwrap();
        let materialized = input
            .materialize(MaterializationBudget::new(usize::MAX))
            .unwrap();
        let batch_plan = unit_crop()
            .plan(
                materialized.shared_schema(),
                materialized.dimensions(),
                PointRepresentation::Columns,
                &LossPolicy::authorize([FidelityLoss::PointSelection]),
            )
            .unwrap();
        let batch_output = batch_plan.execute_batch(&materialized).unwrap();

        assert_eq!(output.dimensions(), PointDimensions::new(2, 1).unwrap());
        assert_eq!(output.schema(), input.schema());
        assert_eq!(output.metadata(), input.metadata());
        assert!(std::ptr::eq(output.schema(), input.schema()));
        assert!(std::ptr::eq(output.metadata(), input.metadata()));
        assert_eq!(output.column("id"), Some(&PointColumn::U32(vec![10, 11])));
        assert_eq!(
            output.column("pair"),
            Some(&PointColumn::I16(vec![1, 2, 3, 4]))
        );
        assert_eq!(batch_output.columns(), output.columns());
        assert_eq!(batch_output.dimensions(), output.dimensions());
        assert_eq!(batch_output.schema(), output.schema());
        assert_eq!(batch_output.metadata(), output.metadata());
    }

    #[test]
    fn unchanged_point_count_preserves_organized_dimensions() {
        let points = [
            (0.0, 0.0, 0.0, 1, [1, 2]),
            (0.1, 0.2, 0.3, 2, [3, 4]),
            (0.4, 0.5, 0.6, 3, [5, 6]),
            (0.7, 0.8, 0.9, 4, [7, 8]),
        ];
        let input = view(&points, PointDimensions::new(2, 2).unwrap());
        let output = plan_view(unit_crop(), &input).execute_view(&input).unwrap();

        assert_eq!(output.dimensions(), PointDimensions::new(2, 2).unwrap());
    }

    #[test]
    fn unsupported_coordinate_type_is_explicit_at_planning_and_execution() {
        let rejected = schema(PrimitiveType::U32);
        let planning_error = Planner::new()
            .validate_operators(
                rejected,
                PointDimensions::new(1, 1).unwrap(),
                PointRepresentation::View,
                &[AxisAlignedCrop::contract()],
                &LossPolicy::authorize([FidelityLoss::PointSelection]),
            )
            .unwrap_err();
        assert_eq!(planning_error.category(), ErrorCategory::Unsupported);
        assert!(
            planning_error
                .message()
                .contains("unsupported primitive U32")
        );

        let schema = schema(PrimitiveType::U32);
        let layout = PointLayout::new(
            schema,
            PointDimensions::new(1, 1).unwrap(),
            vec![0, 4, 12, 16, 20],
            24,
            24,
            0,
            Endianness::Little,
        )
        .unwrap();
        let input = PointView::new(Arc::from(vec![0_u8; 24]), metadata(), layout).unwrap();
        let error = unit_crop().apply(&input).unwrap_err();
        assert!(matches!(
            error,
            CropError::UnsupportedCoordinateType {
                ref name,
                primitive: PrimitiveType::U32
            } if name == "x"
        ));
        assert!(error.to_string().contains("expected F32 or F64"));
    }

    #[test]
    fn crop_satisfies_the_shared_operator_contract() {
        let accepted = schema(PrimitiveType::F32);
        let rejected = Arc::new(
            PointSchema::new(vec![
                PointField::new("x", PrimitiveType::F32, 1, Some(PointFieldSemantic::X)).unwrap(),
                PointField::new("y", PrimitiveType::F64, 1, Some(PointFieldSemantic::Y)).unwrap(),
            ])
            .unwrap(),
        );
        assert_frame_local_contract(ContractTestCase {
            contract: AxisAlignedCrop::contract(),
            accepted_schema: accepted,
            rejected_schema: rejected,
            dimensions: PointDimensions::new(3, 2).unwrap(),
            input_representation: PointRepresentation::View,
            authorized_losses: &[FidelityLoss::PointSelection],
            expected_output_fields: &["x", "y", "z", "id", "pair"],
            expected_materialized_fields: &["x", "y", "z"],
            expected_scratch_bytes: 6,
            expected_output_representation: PointRepresentation::Columns,
            expected_point_count: PointCountEffect::AtMostInput,
            expected_ordering: Ordering::Preserve,
        });

        let plan = AxisAlignedCrop::new(CropBounds::new([0.0; 3], [1.0; 3]).unwrap())
            .plan(
                schema(PrimitiveType::F32),
                PointDimensions::new(3, 2).unwrap(),
                PointRepresentation::View,
                &LossPolicy::authorize([FidelityLoss::PointSelection]),
            )
            .unwrap();
        let column_header = size_of::<PointColumn>() as u64;
        assert_eq!(
            plan.pipeline().stages()[0].additional_materialization_bytes(),
            6 * (4 + 8 + 4) + 3 * column_header
        );
        assert_eq!(
            plan.pipeline().stages()[0].output_allocation_bytes(),
            6 * (4 + 8 + 4 + 4 + 4) + 5 * column_header
        );
        assert_eq!(plan.pipeline().peak_scratch_bytes(), 6);
    }

    proptest! {
        #[test]
        fn crop_matches_half_open_oracle_and_is_stable_and_deterministic(
            coordinates in prop::collection::vec((any::<f32>(), any::<f64>(), any::<f32>()), 0..80)
        ) {
            let points: Vec<TestPoint> = coordinates
                .iter()
                .enumerate()
                .map(|(index, &(x, y, z))| (x, y, z, index as u32, [index as i16, -(index as i16)]))
                .collect();
            let input = view(&points, PointDimensions::new(points.len(), 1).unwrap());
            let crop = AxisAlignedCrop::new(
                CropBounds::new([-100.0, -200.0, -300.0], [100.0, 200.0, 300.0]).unwrap()
            );
            let plan = plan_view(crop, &input);
            let first = plan.execute_view(&input).unwrap();
            let second = plan.execute_view(&input).unwrap();
            let expected: Vec<u32> = coordinates
                .iter()
                .enumerate()
                .filter(|entry| {
                    let &(x, y, z) = entry.1;
                    let coordinate = [f64::from(x), y, f64::from(z)];
                    coordinate.into_iter().all(f64::is_finite)
                        && -100.0 <= coordinate[0] && coordinate[0] < 100.0
                        && -200.0 <= coordinate[1] && coordinate[1] < 200.0
                        && -300.0 <= coordinate[2] && coordinate[2] < 300.0
                })
                .map(|(index, _)| index as u32)
                .collect();

            prop_assert_eq!(first.column("id"), Some(&PointColumn::U32(expected)));
            prop_assert_eq!(first.columns(), second.columns());
            prop_assert_eq!(first.dimensions(), second.dimensions());
            prop_assert_eq!(first.schema(), input.schema());
            prop_assert_eq!(first.metadata(), input.metadata());
            prop_assert_eq!(first.dimensions().height(), 1);
            prop_assert!(first.dimensions().point_count() <= input.layout().dimensions().point_count());
        }
    }
}
