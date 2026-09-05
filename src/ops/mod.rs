//! Frame-local point operators.
//!
//! Operators declare their behavior through the shared contracts in
//! [`crate::core`] before touching point values.

mod crop;
mod field_selection;
mod statistics;
mod voxel;

pub use crop::{AxisAlignedCrop, CropBounds, CropError, CropPlan};
pub use field_selection::{FieldSelection, FieldSelectionPlan, FieldSelectionResult, SchemaChange};
pub use statistics::{
    FieldStatistics, FiniteRange, NumericValue, STATISTICS_REPORT_SCHEMA_VERSION, StatisticsError,
    StatisticsReport, contract as statistics_contract, inspect as inspect_statistics,
};
pub use voxel::{
    InvalidCoordinatePolicy, VoxelGrid, VoxelGridBoundary, VoxelOutputOrdering,
    VoxelReductionReport, VoxelRepresentative,
};

#[cfg(test)]
pub(crate) mod contract_tests;
