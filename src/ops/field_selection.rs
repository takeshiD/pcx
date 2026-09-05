//! Explicit, ordered Point Field selection.

use std::sync::Arc;

use crate::core::{
    Determinism, ElementCountRequirement, Error, ErrorCategory, FidelityLoss, FieldRequirement,
    FieldSelector, InputCapabilities, LossPolicy, Materialization, MetadataEffect,
    OperatorBehavior, OperatorContract, OperatorInput, OperatorOutput, Ordering,
    OutputRepresentation, OutputSchema, Planner, PointCountEffect, PointRepresentation,
    PrimitiveRequirement, Result, ScratchMemory, ValidatedOperatorPipeline, ValueEffect,
    point::{PointBatch, PointDimensions, PointFieldSemantic, PointSchema},
};

/// A validated request to retain Point Fields in the given order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldSelection {
    names: Box<[String]>,
    allow_coordinate_removal: bool,
}

impl FieldSelection {
    /// Creates a selection without consulting a particular Point Schema.
    pub fn new<I, N>(names: I) -> Result<Self>
    where
        I: IntoIterator<Item = N>,
        N: Into<String>,
    {
        let names: Vec<String> = names.into_iter().map(Into::into).collect();
        if names.is_empty() {
            return Err(Error::new(
                ErrorCategory::Usage,
                "field selection requires at least one Point Field name",
            ));
        }
        for (index, name) in names.iter().enumerate() {
            if name.is_empty() {
                return Err(Error::new(
                    ErrorCategory::Usage,
                    "selected Point Field name must not be empty",
                ));
            }
            if names[..index].contains(name) {
                return Err(Error::new(
                    ErrorCategory::Usage,
                    format!("duplicate selected Point Field name {name:?}"),
                ));
            }
        }
        Ok(Self {
            names: names.into_boxed_slice(),
            allow_coordinate_removal: false,
        })
    }

    /// Requests coordinate removal, which still needs its own loss policy.
    pub const fn allow_coordinate_removal(mut self) -> Self {
        self.allow_coordinate_removal = true;
        self
    }

    /// Declares this frame-local operator to the shared pipeline planner.
    pub fn contract(&self) -> OperatorContract {
        let selected = self.names.iter().cloned().map(FieldSelector::named);
        let coordinates = [
            PointFieldSemantic::X,
            PointFieldSemantic::Y,
            PointFieldSemantic::Z,
        ]
        .into_iter()
        .map(|semantic| {
            FieldRequirement::scalar(
                FieldSelector::semantic(semantic),
                PrimitiveRequirement::Numeric,
            )
        });
        let mut losses = vec![FidelityLoss::FieldSelection];
        if self.allow_coordinate_removal {
            losses.push(FidelityLoss::CoordinateFieldSelection);
        }
        OperatorContract::new(
            "field-selection",
            OperatorInput::new(
                InputCapabilities::Columns,
                selected
                    .clone()
                    .map(|selector| {
                        FieldRequirement::new(
                            selector,
                            PrimitiveRequirement::Any,
                            ElementCountRequirement::Any,
                        )
                    })
                    .chain(coordinates),
                Materialization::None,
            ),
            OperatorOutput::new(
                OutputSchema::project(selected),
                OutputRepresentation::Columns,
                PointCountEffect::Preserve,
                MetadataEffect::Preserve,
                ValueEffect::Preserve,
            ),
            OperatorBehavior::new(
                losses,
                Ordering::Preserve,
                Determinism::Deterministic,
                ScratchMemory::fixed(0),
            ),
        )
    }

    /// Resolves the requested names and records the complete schema change.
    pub fn plan(
        &self,
        input_schema: Arc<PointSchema>,
        dimensions: PointDimensions,
        loss_policy: &LossPolicy,
    ) -> Result<FieldSelectionPlan> {
        let pipeline = Planner::new().validate_operators(
            Arc::clone(&input_schema),
            dimensions,
            PointRepresentation::Columns,
            &[self.contract()],
            loss_policy,
        )?;
        for semantic in [
            PointFieldSemantic::X,
            PointFieldSemantic::Y,
            PointFieldSemantic::Z,
        ] {
            let coordinate = input_schema
                .fields()
                .iter()
                .find(|field| field.semantic() == Some(semantic))
                .expect("coordinate requirement was validated by the shared planner");
            if !self.allow_coordinate_removal
                && !self.names.iter().any(|name| name == coordinate.name())
            {
                return Err(Error::new(
                    ErrorCategory::Unsupported,
                    format!(
                        "operator \"field-selection\": required coordinate Point Field {:?} would be removed",
                        coordinate.name()
                    ),
                ));
            }
        }
        let output_schema = pipeline
            .stages()
            .first()
            .expect("a FieldSelection plan always contains one stage")
            .output_schema_handle();
        let change = SchemaChange::new(input_schema, output_schema);
        Ok(FieldSelectionPlan {
            pipeline,
            change,
            dimensions,
        })
    }
}

/// Complete, deterministic account of one Point Schema projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemaChange {
    input: Arc<PointSchema>,
    output: Arc<PointSchema>,
}

impl SchemaChange {
    fn new(input: Arc<PointSchema>, output: Arc<PointSchema>) -> Self {
        Self { input, output }
    }

    pub fn input_schema(&self) -> &PointSchema {
        &self.input
    }

    pub fn output_schema(&self) -> &PointSchema {
        &self.output
    }

    pub fn input_fields(&self) -> impl ExactSizeIterator<Item = &str> {
        self.input.fields().iter().map(|field| field.name())
    }

    pub fn output_fields(&self) -> impl ExactSizeIterator<Item = &str> {
        self.output.fields().iter().map(|field| field.name())
    }

    pub fn removed_fields(&self) -> impl Iterator<Item = &str> {
        self.input_fields()
            .filter(|name| self.output.field(name).is_none())
    }
}

/// A field selection that passed schema, fidelity, and memory-contract checks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldSelectionPlan {
    pipeline: ValidatedOperatorPipeline,
    change: SchemaChange,
    dimensions: PointDimensions,
}

impl FieldSelectionPlan {
    pub const fn pipeline(&self) -> &ValidatedOperatorPipeline {
        &self.pipeline
    }

    pub const fn schema_change(&self) -> &SchemaChange {
        &self.change
    }

    /// Applies the already-validated projection to one Point Frame.
    pub fn execute(&self, input: &PointBatch) -> Result<FieldSelectionResult> {
        let stage = self
            .pipeline
            .stages()
            .first()
            .expect("a FieldSelection plan always contains one stage");
        if input.schema() != stage.input_schema() || input.dimensions() != self.dimensions {
            return Err(Error::new(
                ErrorCategory::Unsupported,
                "field selection input does not match its validated Point Schema and dimensions",
            ));
        }

        let columns = self
            .change
            .output_fields()
            .map(|name| {
                input
                    .column(name)
                    .expect("the plan resolved every selected Point Field")
                    .clone()
            })
            .collect();
        let output = PointBatch::new(
            stage.output_schema_handle(),
            input.metadata_handle(),
            input.dimensions(),
            columns,
        )
        .map_err(|error| {
            Error::new(
                ErrorCategory::Internal,
                format!("planned field selection produced an invalid Point Batch: {error}"),
            )
        })?;
        Ok(FieldSelectionResult {
            frame: output,
            change: self.change.clone(),
        })
    }
}

/// Selected Point Frame paired with its mandatory schema-change report.
#[derive(Clone, Debug)]
pub struct FieldSelectionResult {
    frame: PointBatch,
    change: SchemaChange,
}

impl FieldSelectionResult {
    pub const fn frame(&self) -> &PointBatch {
        &self.frame
    }

    pub const fn schema_change(&self) -> &SchemaChange {
        &self.change
    }

    pub fn into_frame(self) -> PointBatch {
        self.frame
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, sync::Arc};

    use crate::{
        core::point::{
            PointBatch, PointColumn, PointDimensions, PointField, PointFieldSemantic,
            PointFrameMetadata, PointSchema, PrimitiveType, Timestamp,
        },
        core::{ErrorCategory, FidelityLoss, LossPolicy, PointRepresentation},
        ops::contract_tests::{ContractTestCase, assert_frame_local_contract},
    };
    use proptest::prelude::*;

    use super::FieldSelection;

    #[test]
    fn duplicate_requested_names_fail_clearly() {
        let error = FieldSelection::new(["x", "y", "x"])
            .expect_err("a Point Field may be selected only once");

        assert_eq!(error.category(), ErrorCategory::Usage);
        assert!(error.message().contains("duplicate"));
        assert!(error.message().contains("x"));
    }

    #[test]
    fn empty_selection_and_empty_names_are_usage_errors() {
        let empty = FieldSelection::new(Vec::<String>::new()).unwrap_err();
        assert_eq!(empty.category(), ErrorCategory::Usage);
        assert!(empty.message().contains("at least one"));

        let empty_name = FieldSelection::new(["x", ""]).unwrap_err();
        assert_eq!(empty_name.category(), ErrorCategory::Usage);
        assert!(empty_name.message().contains("must not be empty"));
    }

    fn schema(fields: Vec<PointField>) -> Arc<PointSchema> {
        Arc::new(PointSchema::new(fields).unwrap())
    }

    fn xyz_fields() -> Vec<PointField> {
        vec![
            PointField::new("x", PrimitiveType::F32, 1, Some(PointFieldSemantic::X)).unwrap(),
            PointField::new("y", PrimitiveType::F32, 1, Some(PointFieldSemantic::Y)).unwrap(),
            PointField::new("z", PrimitiveType::F32, 1, Some(PointFieldSemantic::Z)).unwrap(),
        ]
    }

    #[test]
    fn field_selection_satisfies_the_shared_frame_local_contract() {
        let mut accepted_fields = xyz_fields();
        accepted_fields.push(PointField::new("intensity", PrimitiveType::U16, 1, None).unwrap());
        let selection = FieldSelection::new(["z", "x", "y"]).unwrap();

        assert_frame_local_contract(ContractTestCase {
            contract: selection.contract(),
            accepted_schema: schema(accepted_fields),
            rejected_schema: schema({
                let mut fields = xyz_fields();
                fields.pop();
                fields
            }),
            dimensions: PointDimensions::new(8, 2).unwrap(),
            input_representation: PointRepresentation::Columns,
            authorized_losses: &[FidelityLoss::FieldSelection],
            expected_output_fields: &["z", "x", "y"],
            expected_materialized_fields: &[],
            expected_scratch_bytes: 0,
            expected_output_representation: PointRepresentation::Columns,
            expected_point_count: crate::core::PointCountEffect::Preserve,
            expected_ordering: crate::core::Ordering::Preserve,
        });

        selection
            .plan(
                schema({
                    let mut fields = xyz_fields();
                    fields.push(PointField::new("intensity", PrimitiveType::U16, 1, None).unwrap());
                    fields
                }),
                PointDimensions::new(8, 2).unwrap(),
                &LossPolicy::authorize([FidelityLoss::FieldSelection]),
            )
            .expect("all coordinate Point Fields remain selected");
    }

    #[test]
    fn a_missing_requested_name_identifies_the_field() {
        let error = FieldSelection::new(["x", "y", "z", "intensity"])
            .unwrap()
            .plan(
                schema(xyz_fields()),
                PointDimensions::new(1, 1).unwrap(),
                &LossPolicy::authorize([FidelityLoss::FieldSelection]),
            )
            .unwrap_err();

        assert_eq!(error.category(), ErrorCategory::Unsupported);
        assert!(error.message().contains("intensity"));
        assert!(error.message().contains("missing"));
    }

    #[test]
    fn coordinate_fields_require_a_separate_explicit_loss_policy() {
        let mut fields = xyz_fields();
        fields.push(PointField::new("intensity", PrimitiveType::U16, 1, None).unwrap());
        let schema = schema(fields);
        let selection = FieldSelection::new(["intensity"])
            .unwrap()
            .allow_coordinate_removal();

        let error = selection
            .plan(
                Arc::clone(&schema),
                PointDimensions::new(1, 1).unwrap(),
                &LossPolicy::authorize([FidelityLoss::FieldSelection]),
            )
            .expect_err("coordinate loss needs its own authorization");
        assert_eq!(error.category(), ErrorCategory::Unsupported);
        assert!(error.message().contains("CoordinateFieldSelection"));

        selection
            .plan(
                schema,
                PointDimensions::new(1, 1).unwrap(),
                &LossPolicy::authorize([
                    FidelityLoss::FieldSelection,
                    FidelityLoss::CoordinateFieldSelection,
                ]),
            )
            .expect("both category-specific losses are authorized");
    }

    #[test]
    fn execution_preserves_values_and_metadata_and_reports_the_whole_schema_change() {
        let mut fields = xyz_fields();
        fields.push(PointField::new("intensity", PrimitiveType::U16, 1, None).unwrap());
        let schema = schema(fields);
        let metadata = Arc::new(
            PointFrameMetadata::new(Timestamp::new(7, 8).unwrap(), "lidar", false)
                .with_container_times(90, 80),
        );
        let dimensions = PointDimensions::new(2, 1).unwrap();
        let batch = PointBatch::new(
            Arc::clone(&schema),
            metadata,
            dimensions,
            vec![
                PointColumn::F32(vec![1.0, 2.0]),
                PointColumn::F32(vec![3.0, 4.0]),
                PointColumn::F32(vec![5.0, 6.0]),
                PointColumn::U16(vec![7, 8]),
            ],
        )
        .unwrap();
        let plan = FieldSelection::new(["z", "x", "y"])
            .unwrap()
            .plan(
                schema,
                dimensions,
                &LossPolicy::authorize([FidelityLoss::FieldSelection]),
            )
            .unwrap();

        let result = plan.execute(&batch).unwrap();

        assert_eq!(
            result
                .frame()
                .schema()
                .fields()
                .iter()
                .map(|field| field.name())
                .collect::<Vec<_>>(),
            ["z", "x", "y"]
        );
        assert_eq!(
            result.frame().columns(),
            &[
                PointColumn::F32(vec![5.0, 6.0]),
                PointColumn::F32(vec![1.0, 2.0]),
                PointColumn::F32(vec![3.0, 4.0]),
            ]
        );
        assert_eq!(result.frame().metadata(), batch.metadata());
        assert_eq!(result.frame().dimensions(), dimensions);
        assert_eq!(
            result.schema_change().input_fields().collect::<Vec<_>>(),
            ["x", "y", "z", "intensity"]
        );
        assert_eq!(
            result.schema_change().output_fields().collect::<Vec<_>>(),
            ["z", "x", "y"]
        );
        assert_eq!(
            result.schema_change().removed_fields().collect::<Vec<_>>(),
            ["intensity"]
        );
        assert!(plan.pipeline().materialization_bytes() > 0);
        assert_eq!(plan.pipeline().peak_scratch_bytes(), 0);
    }

    proptest! {
        #[test]
        fn selected_and_reported_removed_fields_partition_the_input_without_value_loss(
            mask in 0_u8..16,
            reverse_extras in any::<bool>(),
        ) {
            let mut fields = xyz_fields();
            fields.push(PointField::new("intensity", PrimitiveType::U8, 1, None).unwrap());
            fields.push(PointField::new("ring", PrimitiveType::U8, 1, None).unwrap());
            fields.push(PointField::new("timestamp", PrimitiveType::U8, 1, None).unwrap());
            fields.push(PointField::new("normal", PrimitiveType::U8, 2, None).unwrap());
            let schema = schema(fields);
            let dimensions = PointDimensions::new(2, 1).unwrap();
            let batch = PointBatch::new(
                Arc::clone(&schema),
                Arc::new(PointFrameMetadata::new(
                    Timestamp::new(0, 0).unwrap(),
                    "frame",
                    true,
                )),
                dimensions,
                vec![
                    PointColumn::F32(vec![1.0, 2.0]),
                    PointColumn::F32(vec![3.0, 4.0]),
                    PointColumn::F32(vec![5.0, 6.0]),
                    PointColumn::U8(vec![10, 11]),
                    PointColumn::U8(vec![20, 21]),
                    PointColumn::U8(vec![30, 31]),
                    PointColumn::U8(vec![40, 41, 42, 43]),
                ],
            ).unwrap();
            let mut extras: Vec<&str> = ["intensity", "ring", "timestamp", "normal"]
                .into_iter()
                .enumerate()
                .filter_map(|(index, name)| (mask & (1 << index) != 0).then_some(name))
                .collect();
            if reverse_extras {
                extras.reverse();
            }
            let requested: Vec<&str> = ["z", "x", "y"]
                .into_iter()
                .chain(extras)
                .collect();
            let plan = FieldSelection::new(requested.iter().copied())
                .unwrap()
                .plan(
                    schema,
                    dimensions,
                    &LossPolicy::authorize([FidelityLoss::FieldSelection]),
                )
                .unwrap();
            let result = plan.execute(&batch).unwrap();

            prop_assert_eq!(
                result.schema_change().output_fields().collect::<Vec<_>>(),
                requested.as_slice()
            );
            let reported: BTreeSet<&str> = result
                .schema_change()
                .output_fields()
                .chain(result.schema_change().removed_fields())
                .collect();
            prop_assert_eq!(reported, BTreeSet::from([
                "x", "y", "z", "intensity", "ring", "timestamp", "normal"
            ]));
            for removed in result.schema_change().removed_fields() {
                prop_assert!(!requested.contains(&removed));
            }
            for name in &requested {
                prop_assert_eq!(result.frame().column(name), batch.column(name));
            }
            let expected_output_bytes = requested.len() * std::mem::size_of::<PointColumn>()
                + requested.iter().map(|name| {
                    let field = batch.schema().field(name).unwrap();
                    field.primitive().size() * field.count() * dimensions.point_count()
                }).sum::<usize>();
            prop_assert_eq!(
                plan.pipeline().stages()[0].output_allocation_bytes(),
                u64::try_from(expected_output_bytes).unwrap()
            );
        }
    }
}
