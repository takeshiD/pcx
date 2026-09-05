//! Shared assertions for every frame-local operator adapter.

use std::sync::Arc;

use crate::core::{
    Determinism, ErrorCategory, FidelityLoss, LossPolicy, MetadataEffect, OperatorContract,
    Ordering, Planner, PointCountEffect, PointRepresentation, ValidatedOperatorPipeline,
    ValueEffect,
    point::{PointDimensions, PointSchema},
};

/// Inputs and expected guarantees shared by field/crop/stats/voxel tests.
///
/// Operators which accept every valid schema omit `rejected_schema`.
pub(crate) struct ContractTestCase<'a> {
    pub contract: OperatorContract,
    pub accepted_schema: Arc<PointSchema>,
    pub rejected_schema: Option<Arc<PointSchema>>,
    pub dimensions: PointDimensions,
    pub input_representation: PointRepresentation,
    pub authorized_losses: &'a [FidelityLoss],
    pub expected_output_fields: &'a [&'a str],
    pub expected_materialized_fields: &'a [&'a str],
    pub expected_scratch_bytes: u64,
    pub expected_output_representation: PointRepresentation,
    pub expected_point_count: PointCountEffect,
    pub expected_ordering: Ordering,
}

/// Proves acceptance, schema rejection, loss gating, determinism, and bounds.
pub(crate) fn assert_frame_local_contract(case: ContractTestCase<'_>) {
    let validated = plan(
        &case,
        LossPolicy::authorize(case.authorized_losses.iter().copied()),
    )
    .expect("documented compatible schema and loss policy must plan");
    let stage = validated
        .stages()
        .first()
        .expect("one contract yields one stage");
    assert_eq!(
        stage
            .output_schema()
            .fields()
            .iter()
            .map(|field| field.name())
            .collect::<Vec<_>>(),
        case.expected_output_fields
    );
    assert_eq!(
        stage.materialized_fields(),
        case.expected_materialized_fields
    );
    assert_eq!(stage.scratch_bytes(), case.expected_scratch_bytes);
    assert_eq!(
        stage.output_representation(),
        case.expected_output_representation
    );
    assert_eq!(stage.point_count(), case.expected_point_count);
    assert_eq!(stage.ordering(), case.expected_ordering);
    assert_eq!(
        case.contract.behavior().determinism(),
        Determinism::Deterministic
    );
    assert_eq!(case.contract.output().metadata(), MetadataEffect::Preserve);
    assert_eq!(case.contract.output().values(), ValueEffect::Preserve);

    if let Some(rejected_schema) = &case.rejected_schema {
        let schema_error = Planner::new()
            .validate_operators(
                Arc::clone(rejected_schema),
                case.dimensions,
                case.input_representation,
                std::slice::from_ref(&case.contract),
                &LossPolicy::authorize(case.authorized_losses.iter().copied()),
            )
            .expect_err("documented incompatible schema must fail during planning");
        assert_eq!(schema_error.category(), ErrorCategory::Unsupported);
    }

    if !case.authorized_losses.is_empty() {
        let loss_error = plan(&case, LossPolicy::lossless())
            .expect_err("loss must require category-specific authorization");
        assert_eq!(loss_error.category(), ErrorCategory::Unsupported);
    }
}

fn plan(
    case: &ContractTestCase<'_>,
    loss_policy: LossPolicy,
) -> crate::core::Result<ValidatedOperatorPipeline> {
    Planner::new().validate_operators(
        Arc::clone(&case.accepted_schema),
        case.dimensions,
        case.input_representation,
        std::slice::from_ref(&case.contract),
        &loss_policy,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        ElementCountRequirement, FieldRequirement, FieldSelector, InputCapabilities,
        Materialization, OperatorBehavior, OperatorInput, OperatorOutput, OutputRepresentation,
        OutputSchema, PointCountEffect, PrimitiveRequirement, ScratchMemory, ValueEffect,
        point::{PointField, PointFieldSemantic, PrimitiveType},
    };

    fn schema(fields: Vec<PointField>) -> Arc<PointSchema> {
        Arc::new(PointSchema::new(fields).unwrap())
    }

    fn xyz_schema() -> Arc<PointSchema> {
        schema(vec![
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
    }

    fn crop_contract() -> OperatorContract {
        let coordinates = [
            PointFieldSemantic::X,
            PointFieldSemantic::Y,
            PointFieldSemantic::Z,
        ];
        OperatorContract::new(
            "crop",
            OperatorInput::new(
                InputCapabilities::ViewOrColumns,
                coordinates.into_iter().map(|semantic| {
                    FieldRequirement::scalar(
                        FieldSelector::semantic(semantic),
                        PrimitiveRequirement::Floating,
                    )
                }),
                Materialization::fields(coordinates.into_iter().map(FieldSelector::semantic)),
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
                ScratchMemory::per_input_point(16, 1),
            ),
        )
    }

    #[test]
    fn shared_assertion_covers_a_crop_shaped_contract() {
        assert_frame_local_contract(ContractTestCase {
            contract: crop_contract(),
            accepted_schema: xyz_schema(),
            rejected_schema: Some(schema(vec![
                PointField::new("x", PrimitiveType::F32, 1, Some(PointFieldSemantic::X)).unwrap(),
                PointField::new("y", PrimitiveType::F32, 1, Some(PointFieldSemantic::Y)).unwrap(),
            ])),
            dimensions: PointDimensions::new(10, 2).unwrap(),
            input_representation: PointRepresentation::View,
            authorized_losses: &[FidelityLoss::PointSelection],
            expected_output_fields: &["x", "y", "z", "ring"],
            expected_materialized_fields: &["x", "y", "z"],
            expected_scratch_bytes: 36,
            expected_output_representation: PointRepresentation::Columns,
            expected_point_count: PointCountEffect::AtMostInput,
            expected_ordering: Ordering::Preserve,
        });
    }

    #[test]
    fn pipeline_deduplicates_columns_and_resolves_schema_changes() {
        let inspect_x = OperatorContract::new(
            "inspect-x",
            OperatorInput::new(
                InputCapabilities::ViewOrColumns,
                [FieldRequirement::new(
                    FieldSelector::named("x"),
                    PrimitiveRequirement::Numeric,
                    ElementCountRequirement::Any,
                )],
                Materialization::fields([FieldSelector::named("x")]),
            ),
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
                ScratchMemory::fixed(8),
            ),
        );
        let project = OperatorContract::new(
            "field-selection",
            OperatorInput::new(
                InputCapabilities::ViewOrColumns,
                [],
                Materialization::fields([FieldSelector::named("x")]),
            ),
            OperatorOutput::new(
                OutputSchema::project([FieldSelector::named("x"), FieldSelector::named("ring")]),
                OutputRepresentation::Preserve,
                PointCountEffect::Preserve,
                MetadataEffect::Preserve,
                ValueEffect::Preserve,
            ),
            OperatorBehavior::new(
                [FidelityLoss::FieldSelection],
                Ordering::Preserve,
                Determinism::Deterministic,
                ScratchMemory::fixed(4),
            ),
        );
        let plan = Planner::new()
            .validate_operators(
                xyz_schema(),
                PointDimensions::new(2, 1).unwrap(),
                PointRepresentation::View,
                &[inspect_x, project],
                &LossPolicy::authorize([FidelityLoss::FieldSelection]),
            )
            .unwrap();

        assert_eq!(plan.stages()[0].materialized_fields(), &["x"]);
        assert!(plan.stages()[1].materialized_fields().is_empty());
        assert_eq!(plan.output_schema().fields().len(), 2);
        assert_eq!(plan.peak_scratch_bytes(), 8);
        assert_eq!(
            plan.materialization_bytes(),
            2 * u64::try_from(PrimitiveType::F32.size()).unwrap()
                + u64::try_from(std::mem::size_of::<crate::core::point::PointColumn>()).unwrap()
        );
    }

    #[test]
    fn planner_rejects_unbounded_nondeterministic_and_undeclared_loss() {
        let invalid = [
            OperatorContract::new(
                "unbounded",
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
                    ScratchMemory::Unplannable,
                ),
            ),
            OperatorContract::new(
                "nondeterministic",
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
                    Determinism::Unspecified,
                    ScratchMemory::fixed(0),
                ),
            ),
            OperatorContract::new(
                "hidden-loss",
                OperatorInput::new(InputCapabilities::View, [], Materialization::None),
                OperatorOutput::new(
                    OutputSchema::Preserve,
                    OutputRepresentation::Columns,
                    PointCountEffect::AtMostInput,
                    MetadataEffect::Preserve,
                    ValueEffect::Preserve,
                ),
                OperatorBehavior::new(
                    [],
                    Ordering::Preserve,
                    Determinism::Deterministic,
                    ScratchMemory::fixed(0),
                ),
            ),
        ];

        for contract in invalid {
            let error = Planner::new()
                .validate_operators(
                    xyz_schema(),
                    PointDimensions::new(1, 1).unwrap(),
                    PointRepresentation::View,
                    &[contract],
                    &LossPolicy::authorize([
                        FidelityLoss::PointSelection,
                        FidelityLoss::FieldSelection,
                    ]),
                )
                .unwrap_err();
            assert!(matches!(
                error.category(),
                ErrorCategory::Unsupported | ErrorCategory::Resource
            ));
        }
    }

    #[test]
    fn columns_only_input_triggers_full_materialization() {
        let contract = OperatorContract::new(
            "columns-only",
            OperatorInput::new(InputCapabilities::Columns, [], Materialization::None),
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
        );
        let plan = Planner::new()
            .validate_operators(
                xyz_schema(),
                PointDimensions::new(1, 1).unwrap(),
                PointRepresentation::View,
                &[contract],
                &LossPolicy::lossless(),
            )
            .unwrap();

        assert_eq!(
            plan.stages()[0].materialized_fields(),
            &["x", "y", "z", "ring"]
        );
        assert_eq!(plan.output_representation(), PointRepresentation::Columns);
    }
}
