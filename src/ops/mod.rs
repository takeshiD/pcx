//! Frame-local point operators.
//!
//! Operators declare their behavior through the shared contracts in
//! [`crate::core`] before touching point values.

mod crop;
mod field_selection;

pub use crop::{AxisAlignedCrop, CropBounds, CropError, CropPlan};
pub use field_selection::{FieldSelection, FieldSelectionPlan, FieldSelectionResult, SchemaChange};

#[cfg(test)]
pub(crate) mod contract_tests;
