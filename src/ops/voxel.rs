//! Deterministic, frame-local voxel-grid reduction.
//!
//! Cells are half-open intervals anchored at an explicit origin: cell `i` on
//! an axis is `[origin + i * size, origin + (i + 1) * size)`. The first input
//! point in each occupied cell is retained, and retained points stay in input
//! order. Non-finite coordinates are either dropped or reject the frame,
//! according to [`InvalidCoordinatePolicy`]. A finite coordinate whose cell
//! index cannot be represented as `i64` always rejects the frame.

use std::mem::size_of;

use crate::core::{
    Determinism, ElementCountRequirement, Error, ErrorCategory, FidelityLoss, FieldRequirement,
    FieldSelector, InputCapabilities, Materialization, MetadataEffect, OperatorBehavior,
    OperatorContract, OperatorInput, OperatorOutput, Ordering, OutputRepresentation, OutputSchema,
    PointCountEffect, PrimitiveRequirement, Result, ScratchMemory, ValueEffect,
    point::{PointBatch, PointColumn, PointDimensions, PointFieldSemantic, PrimitiveType},
};

const CANDIDATE_BYTES: u64 = size_of::<CellCandidate>() as u64;
const REPRESENTATIVE_INDEX_BYTES: u64 = size_of::<usize>() as u64;
const SCRATCH_BYTES_PER_INPUT_POINT: u64 = CANDIDATE_BYTES + REPRESENTATIVE_INDEX_BYTES;

/// Treatment of points containing NaN or infinity in x, y, or z.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidCoordinatePolicy {
    Drop,
    Reject,
}

/// Boundary convention for voxel cells.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VoxelGridBoundary {
    /// Lower faces are inclusive and upper faces belong to the next cell.
    HalfOpen,
}

/// Choice of the point retained from an occupied cell.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VoxelRepresentative {
    FirstPoint,
}

/// Ordering of retained representatives.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VoxelOutputOrdering {
    InputOrder,
}

/// Schema and point-count effects disclosed after reduction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VoxelReductionReport {
    input_points: usize,
    output_points: usize,
    schema_preserved: bool,
    output_is_unorganized: bool,
}

impl VoxelReductionReport {
    pub const fn input_points(self) -> usize {
        self.input_points
    }

    pub const fn output_points(self) -> usize {
        self.output_points
    }

    pub const fn schema_preserved(self) -> bool {
        self.schema_preserved
    }

    pub const fn output_is_unorganized(self) -> bool {
        self.output_is_unorganized
    }
}

/// A validated voxel-grid configuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoxelGrid {
    origin: [f64; 3],
    size: [f64; 3],
    boundary: VoxelGridBoundary,
    representative: VoxelRepresentative,
    invalid_coordinates: InvalidCoordinatePolicy,
    ordering: VoxelOutputOrdering,
}

impl VoxelGrid {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        origin: [f64; 3],
        size: [f64; 3],
        boundary: VoxelGridBoundary,
        representative: VoxelRepresentative,
        invalid_coordinates: InvalidCoordinatePolicy,
        ordering: VoxelOutputOrdering,
    ) -> Result<Self> {
        if origin.into_iter().any(|value| !value.is_finite()) {
            return Err(Error::new(
                ErrorCategory::Usage,
                "voxel origin coordinates must be finite",
            ));
        }
        if size
            .into_iter()
            .any(|value| !value.is_finite() || value <= 0.0)
        {
            return Err(Error::new(
                ErrorCategory::Usage,
                "voxel sizes must be finite and greater than zero",
            ));
        }
        Ok(Self {
            origin,
            size,
            boundary,
            representative,
            invalid_coordinates,
            ordering,
        })
    }

    pub const fn origin(self) -> [f64; 3] {
        self.origin
    }

    pub const fn size(self) -> [f64; 3] {
        self.size
    }

    pub const fn boundary(self) -> VoxelGridBoundary {
        self.boundary
    }

    pub const fn representative(self) -> VoxelRepresentative {
        self.representative
    }

    pub const fn invalid_coordinates(self) -> InvalidCoordinatePolicy {
        self.invalid_coordinates
    }

    pub const fn ordering(self) -> VoxelOutputOrdering {
        self.ordering
    }

    /// Resolve a coordinate to its integer cell, or `None` when invalid
    /// coordinates are configured to be dropped.
    pub fn cell(self, coordinates: [f64; 3]) -> Result<Option<[i64; 3]>> {
        if coordinates.into_iter().any(|value| !value.is_finite()) {
            return match self.invalid_coordinates {
                InvalidCoordinatePolicy::Drop => Ok(None),
                InvalidCoordinatePolicy::Reject => Err(Error::new(
                    ErrorCategory::InvalidData,
                    "voxel reduction encountered a non-finite coordinate",
                )),
            };
        }

        let mut cell = [0_i64; 3];
        for axis in 0..3 {
            let index = ((coordinates[axis] - self.origin[axis]) / self.size[axis]).floor();
            if !index.is_finite() || index < i64::MIN as f64 || index >= -(i64::MIN as f64) {
                return Err(Error::new(
                    ErrorCategory::InvalidData,
                    "voxel cell index is outside the supported i64 range",
                ));
            }
            cell[axis] = index as i64;
        }
        Ok(Some(cell))
    }

    /// Declare the schema, fidelity, ordering, and memory effects to the
    /// shared frame-local operator planner.
    pub fn contract(self) -> OperatorContract {
        let coordinates = [
            PointFieldSemantic::X,
            PointFieldSemantic::Y,
            PointFieldSemantic::Z,
        ];
        OperatorContract::new(
            "voxel-grid",
            OperatorInput::new(
                InputCapabilities::Columns,
                coordinates.into_iter().map(|semantic| {
                    FieldRequirement::new(
                        FieldSelector::semantic(semantic),
                        PrimitiveRequirement::Floating,
                        ElementCountRequirement::Exactly(1),
                    )
                }),
                Materialization::AllFields,
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
                ScratchMemory::per_input_point(0, SCRATCH_BYTES_PER_INPUT_POINT),
            ),
        )
    }

    /// Reduce one materialized Point Frame.
    pub fn reduce(self, input: &PointBatch) -> Result<(PointBatch, VoxelReductionReport)> {
        let coordinate_columns = coordinate_column_indices(input)?;
        let input_points = input.dimensions().point_count();
        let mut candidates = Vec::new();
        candidates.try_reserve_exact(input_points).map_err(|_| {
            resource_error("voxel candidate table allocation failed within its planned bound")
        })?;

        for point in 0..input_points {
            let coordinates = coordinate_columns.map(|column| coordinate(input, column, point));
            if let Some(cell) = self.cell(coordinates)? {
                candidates.push(CellCandidate { cell, point });
            }
        }
        candidates.sort_unstable();

        let mut representatives = Vec::new();
        representatives
            .try_reserve_exact(candidates.len())
            .map_err(|_| {
                resource_error(
                    "voxel representative index allocation failed within its planned bound",
                )
            })?;
        let mut last_cell = None;
        for candidate in candidates {
            if last_cell != Some(candidate.cell) {
                representatives.push(candidate.point);
                last_cell = Some(candidate.cell);
            }
        }
        representatives.sort_unstable();

        let mut columns = Vec::new();
        columns
            .try_reserve_exact(input.columns().len())
            .map_err(|_| resource_error("voxel output column table allocation failed"))?;
        for (field, column) in input.schema().fields().iter().zip(input.columns()) {
            columns.push(select_column(column, field.count(), &representatives)?);
        }
        let output_points = representatives.len();
        let output = PointBatch::new(
            input.shared_schema(),
            input.metadata_handle(),
            PointDimensions::new(output_points, 1).map_err(|error| {
                Error::new(
                    ErrorCategory::Internal,
                    format!("voxel output dimensions violated an invariant: {error}"),
                )
            })?,
            columns,
        )
        .map_err(|error| {
            Error::new(
                ErrorCategory::Internal,
                format!("voxel output batch violated an invariant: {error}"),
            )
        })?;
        Ok((
            output,
            VoxelReductionReport {
                input_points,
                output_points,
                schema_preserved: true,
                output_is_unorganized: true,
            },
        ))
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct CellCandidate {
    cell: [i64; 3],
    point: usize,
}

fn coordinate_column_indices(input: &PointBatch) -> Result<[usize; 3]> {
    Ok([
        coordinate_column_index(input, PointFieldSemantic::X)?,
        coordinate_column_index(input, PointFieldSemantic::Y)?,
        coordinate_column_index(input, PointFieldSemantic::Z)?,
    ])
}

fn coordinate_column_index(input: &PointBatch, semantic: PointFieldSemantic) -> Result<usize> {
    let mut matches = input
        .schema()
        .fields()
        .iter()
        .enumerate()
        .filter(|(_, field)| field.semantic() == Some(semantic));
    let (index, field) = matches.next().ok_or_else(|| {
        unsupported_error(format!(
            "voxel input is missing coordinate semantic {semantic:?}"
        ))
    })?;
    if matches.next().is_some() {
        return Err(unsupported_error(format!(
            "voxel input coordinate semantic {semantic:?} is ambiguous"
        )));
    }
    if field.count() != 1 || !matches!(field.primitive(), PrimitiveType::F32 | PrimitiveType::F64) {
        return Err(unsupported_error(format!(
            "voxel coordinate field {:?} must be scalar f32 or f64",
            field.name()
        )));
    }
    Ok(index)
}

fn coordinate(input: &PointBatch, column: usize, point: usize) -> f64 {
    match &input.columns()[column] {
        PointColumn::F32(values) => f64::from(values[point]),
        PointColumn::F64(values) => values[point],
        _ => unreachable!("coordinate columns were validated"),
    }
}

fn select_column(column: &PointColumn, count: usize, points: &[usize]) -> Result<PointColumn> {
    macro_rules! select {
        ($values:expr, $variant:ident) => {{
            let values = $values;
            let len = points
                .len()
                .checked_mul(count)
                .ok_or_else(|| resource_error("voxel output scalar count overflowed"))?;
            let mut selected = Vec::new();
            selected
                .try_reserve_exact(len)
                .map_err(|_| resource_error("voxel output column allocation failed"))?;
            for &point in points {
                let start = point
                    .checked_mul(count)
                    .ok_or_else(|| resource_error("voxel source scalar index overflowed"))?;
                selected.extend_from_slice(&values[start..start + count]);
            }
            PointColumn::$variant(selected)
        }};
    }
    Ok(match column {
        PointColumn::I8(values) => select!(values, I8),
        PointColumn::U8(values) => select!(values, U8),
        PointColumn::I16(values) => select!(values, I16),
        PointColumn::U16(values) => select!(values, U16),
        PointColumn::I32(values) => select!(values, I32),
        PointColumn::U32(values) => select!(values, U32),
        PointColumn::I64(values) => select!(values, I64),
        PointColumn::U64(values) => select!(values, U64),
        PointColumn::F32(values) => select!(values, F32),
        PointColumn::F64(values) => select!(values, F64),
    })
}

fn resource_error(message: impl Into<String>) -> Error {
    Error::new(ErrorCategory::Resource, message)
}

fn unsupported_error(message: impl Into<String>) -> Error {
    Error::new(ErrorCategory::Unsupported, message)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use std::sync::Arc;

    use super::{
        InvalidCoordinatePolicy, SCRATCH_BYTES_PER_INPUT_POINT, VoxelGrid, VoxelGridBoundary,
        VoxelOutputOrdering, VoxelRepresentative,
    };
    use crate::core::point::{
        PointBatch, PointColumn, PointDimensions, PointField, PointFieldSemantic,
        PointFrameMetadata, PointSchema, PrimitiveType, Timestamp,
    };
    use crate::core::{FidelityLoss, Ordering, PointCountEffect, PointRepresentation};
    use crate::ops::contract_tests::{ContractTestCase, assert_frame_local_contract};

    fn grid(invalid: InvalidCoordinatePolicy) -> VoxelGrid {
        VoxelGrid::new(
            [0.0; 3],
            [1.0; 3],
            VoxelGridBoundary::HalfOpen,
            VoxelRepresentative::FirstPoint,
            invalid,
            VoxelOutputOrdering::InputOrder,
        )
        .unwrap()
    }

    fn batch(coordinates: &[[f32; 3]], rings: &[u16]) -> PointBatch {
        batch_with_dimensions(
            coordinates,
            rings,
            PointDimensions::new(coordinates.len(), 1).unwrap(),
        )
    }

    fn batch_with_dimensions(
        coordinates: &[[f32; 3]],
        rings: &[u16],
        dimensions: PointDimensions,
    ) -> PointBatch {
        let schema = Arc::new(
            PointSchema::new(vec![
                PointField::new("x", PrimitiveType::F32, 1, Some(PointFieldSemantic::X)).unwrap(),
                PointField::new("y", PrimitiveType::F32, 1, Some(PointFieldSemantic::Y)).unwrap(),
                PointField::new("z", PrimitiveType::F32, 1, Some(PointFieldSemantic::Z)).unwrap(),
                PointField::new(
                    "ring",
                    PrimitiveType::U16,
                    1,
                    Some(PointFieldSemantic::Ring),
                )
                .unwrap(),
            ])
            .unwrap(),
        );
        PointBatch::new(
            schema,
            Arc::new(
                PointFrameMetadata::new(Timestamp::new(3, 4).unwrap(), "lidar", false)
                    .with_container_times(5, 6),
            ),
            dimensions,
            vec![
                PointColumn::F32(coordinates.iter().map(|p| p[0]).collect()),
                PointColumn::F32(coordinates.iter().map(|p| p[1]).collect()),
                PointColumn::F32(coordinates.iter().map(|p| p[2]).collect()),
                PointColumn::U16(rings.to_vec()),
            ],
        )
        .unwrap()
    }

    fn assert_same_batch(left: &PointBatch, right: &PointBatch) {
        assert_eq!(left.schema(), right.schema());
        assert_eq!(left.metadata(), right.metadata());
        assert_eq!(left.dimensions(), right.dimensions());
        assert_eq!(left.columns(), right.columns());
    }

    #[test]
    fn half_open_cells_are_anchored_at_the_explicit_origin() {
        let grid = VoxelGrid::new(
            [10.0, -2.0, 0.5],
            [2.0, 2.0, 2.0],
            VoxelGridBoundary::HalfOpen,
            VoxelRepresentative::FirstPoint,
            InvalidCoordinatePolicy::Reject,
            VoxelOutputOrdering::InputOrder,
        )
        .unwrap();

        assert_eq!(grid.cell([10.0, -2.0, 0.5]).unwrap(), Some([0, 0, 0]));
        assert_eq!(grid.cell([11.999, -0.001, 2.499]).unwrap(), Some([0, 0, 0]));
        assert_eq!(grid.cell([12.0, 0.0, 2.5]).unwrap(), Some([1, 1, 1]));
        assert_eq!(
            grid.cell([9.999, -2.001, 0.499]).unwrap(),
            Some([-1, -1, -1])
        );
    }

    #[test]
    fn reduction_keeps_first_representatives_in_input_order_and_preserves_the_frame() {
        let input = batch_with_dimensions(
            &[
                [1.1, 0.0, 0.0],
                [0.2, 0.0, 0.0],
                [1.9, 0.0, 0.0],
                [-0.1, 0.0, 0.0],
                [0.8, 0.0, 0.0],
                [1.2, 0.0, 0.0],
            ],
            &[10, 20, 30, 40, 50, 60],
            PointDimensions::new(3, 2).unwrap(),
        );

        let (output, report) = grid(InvalidCoordinatePolicy::Reject)
            .reduce(&input)
            .unwrap();

        assert_eq!(output.dimensions(), PointDimensions::new(3, 1).unwrap());
        assert_eq!(
            output.column("x"),
            Some(&PointColumn::F32(vec![1.1, 0.2, -0.1]))
        );
        assert_eq!(
            output.column("ring"),
            Some(&PointColumn::U16(vec![10, 20, 40]))
        );
        assert_eq!(output.schema(), input.schema());
        assert_eq!(output.metadata(), input.metadata());
        assert_eq!(report.input_points(), 6);
        assert_eq!(report.output_points(), 3);
        assert!(report.schema_preserved());
        assert!(report.output_is_unorganized());
    }

    #[test]
    fn invalid_coordinates_follow_the_configured_policy() {
        let input = batch(&[[0.0, 0.0, 0.0], [f32::NAN, 1.0, 1.0]], &[1, 2]);

        let (output, report) = grid(InvalidCoordinatePolicy::Drop).reduce(&input).unwrap();
        assert_eq!(output.column("ring"), Some(&PointColumn::U16(vec![1])));
        assert_eq!(report.output_points(), 1);

        let error = grid(InvalidCoordinatePolicy::Reject)
            .reduce(&input)
            .unwrap_err();
        assert_eq!(error.category(), crate::core::ErrorCategory::InvalidData);
    }

    #[test]
    fn configuration_and_cell_index_ranges_are_validated() {
        let error = VoxelGrid::new(
            [f64::NAN, 0.0, 0.0],
            [1.0; 3],
            VoxelGridBoundary::HalfOpen,
            VoxelRepresentative::FirstPoint,
            InvalidCoordinatePolicy::Drop,
            VoxelOutputOrdering::InputOrder,
        )
        .unwrap_err();
        assert_eq!(error.category(), crate::core::ErrorCategory::Usage);

        let error = VoxelGrid::new(
            [0.0; 3],
            [1.0, 0.0, 1.0],
            VoxelGridBoundary::HalfOpen,
            VoxelRepresentative::FirstPoint,
            InvalidCoordinatePolicy::Drop,
            VoxelOutputOrdering::InputOrder,
        )
        .unwrap_err();
        assert_eq!(error.category(), crate::core::ErrorCategory::Usage);

        let error = grid(InvalidCoordinatePolicy::Drop)
            .cell([f64::MAX, 0.0, 0.0])
            .unwrap_err();
        assert_eq!(error.category(), crate::core::ErrorCategory::InvalidData);
    }

    #[test]
    fn voxel_contract_passes_the_shared_frame_local_assertions() {
        let accepted = Arc::new(batch(&[[0.0; 3]; 4], &[1, 2, 3, 4]).schema().clone());
        let rejected = Arc::new(
            PointSchema::new(vec![
                PointField::new("x", PrimitiveType::F32, 1, Some(PointFieldSemantic::X)).unwrap(),
                PointField::new("y", PrimitiveType::F32, 1, Some(PointFieldSemantic::Y)).unwrap(),
            ])
            .unwrap(),
        );
        assert_frame_local_contract(ContractTestCase {
            contract: grid(InvalidCoordinatePolicy::Reject).contract(),
            accepted_schema: accepted,
            rejected_schema: Some(rejected),
            dimensions: PointDimensions::new(2, 2).unwrap(),
            input_representation: PointRepresentation::View,
            authorized_losses: &[FidelityLoss::PointSelection],
            expected_output_fields: &["x", "y", "z", "ring"],
            expected_materialized_fields: &["x", "y", "z", "ring"],
            expected_scratch_bytes: 4 * SCRATCH_BYTES_PER_INPUT_POINT,
            expected_output_representation: PointRepresentation::Columns,
            expected_point_count: PointCountEffect::AtMostInput,
            expected_ordering: Ordering::Preserve,
        });
    }

    proptest! {
        #[test]
        fn every_integer_face_obeys_the_half_open_boundary(cell in -1_000_000_i64..1_000_000) {
            let grid = grid(InvalidCoordinatePolicy::Reject);
            let face = cell as f64;
            prop_assert_eq!(grid.cell([face; 3]).unwrap(), Some([cell; 3]));
            prop_assert_eq!(
                grid.cell([face - 0.25; 3]).unwrap(),
                Some([cell - 1; 3]),
            );
            prop_assert_eq!(
                grid.cell([face + 0.75; 3]).unwrap(),
                Some([cell; 3]),
            );
        }

        #[test]
        fn reduction_is_deterministic(
            coordinates in prop::collection::vec(
                (-1000_i16..1000, -1000_i16..1000, -1000_i16..1000),
                0..128,
            ),
        ) {
            let coordinates: Vec<[f32; 3]> = coordinates
                .into_iter()
                .map(|(x, y, z)| [f32::from(x) / 4.0, f32::from(y) / 4.0, f32::from(z) / 4.0])
                .collect();
            let rings: Vec<u16> = (0..coordinates.len()).map(|index| index as u16).collect();
            let input = batch(&coordinates, &rings);
            let grid = grid(InvalidCoordinatePolicy::Reject);

            let (first, first_report) = grid.reduce(&input).unwrap();
            let (second, second_report) = grid.reduce(&input).unwrap();

            assert_same_batch(&first, &second);
            prop_assert_eq!(first_report, second_report);
        }
    }
}
