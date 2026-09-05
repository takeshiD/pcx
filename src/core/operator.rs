//! Declarative contracts for frame-local point operators.
//!
//! Concrete operators live in `crate::ops`. This module is the planning seam:
//! it resolves their declarations against an input schema without executing an
//! operator or touching point values.

use std::{collections::BTreeSet, mem::size_of, sync::Arc};

use super::{
    Error, ErrorCategory, Result,
    point::{
        PointColumn, PointDimensions, PointField, PointFieldSemantic, PointSchema, PrimitiveType,
    },
};

/// The point representation an operator can consume directly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputCapabilities {
    View,
    Columns,
    ViewOrColumns,
}

impl InputCapabilities {
    const fn accepts(self, representation: PointRepresentation) -> bool {
        matches!(
            (self, representation),
            (Self::View, PointRepresentation::View)
                | (Self::Columns, PointRepresentation::Columns)
                | (Self::ViewOrColumns, _)
        )
    }
}

/// A representation known to be available between planned stages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PointRepresentation {
    View,
    Columns,
}

/// Selects one field without relying on its position in the schema.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FieldSelector {
    Name(String),
    Semantic(PointFieldSemantic),
}

impl FieldSelector {
    pub fn named(name: impl Into<String>) -> Self {
        Self::Name(name.into())
    }

    pub const fn semantic(semantic: PointFieldSemantic) -> Self {
        Self::Semantic(semantic)
    }
}

/// Primitive representations accepted for a required field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PrimitiveRequirement {
    Any,
    Numeric,
    Floating,
    OneOf(Box<[PrimitiveType]>),
}

impl PrimitiveRequirement {
    pub fn one_of(primitives: impl IntoIterator<Item = PrimitiveType>) -> Self {
        Self::OneOf(primitives.into_iter().collect())
    }

    fn accepts(&self, primitive: PrimitiveType) -> bool {
        match self {
            Self::Any | Self::Numeric => true,
            Self::Floating => matches!(primitive, PrimitiveType::F32 | PrimitiveType::F64),
            Self::OneOf(primitives) => primitives.contains(&primitive),
        }
    }
}

/// Scalar-count requirement for one field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ElementCountRequirement {
    Any,
    Exactly(usize),
}

/// A field that must exist before an operator may run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldRequirement {
    selector: FieldSelector,
    primitives: PrimitiveRequirement,
    elements: ElementCountRequirement,
}

impl FieldRequirement {
    pub fn new(
        selector: FieldSelector,
        primitives: PrimitiveRequirement,
        elements: ElementCountRequirement,
    ) -> Self {
        Self {
            selector,
            primitives,
            elements,
        }
    }

    pub fn scalar(selector: FieldSelector, primitives: PrimitiveRequirement) -> Self {
        Self::new(selector, primitives, ElementCountRequirement::Exactly(1))
    }

    pub fn selector(&self) -> &FieldSelector {
        &self.selector
    }
}

/// Columns that must be decoded for an operator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Materialization {
    None,
    Fields(Box<[FieldSelector]>),
    AllFields,
}

impl Materialization {
    pub fn fields(selectors: impl IntoIterator<Item = FieldSelector>) -> Self {
        Self::Fields(selectors.into_iter().collect())
    }
}

/// Schema transformation performed by an operator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OutputSchema {
    Preserve,
    Project(Box<[FieldSelector]>),
}

impl OutputSchema {
    pub fn project(selectors: impl IntoIterator<Item = FieldSelector>) -> Self {
        Self::Project(selectors.into_iter().collect())
    }
}

/// Representation produced for the next stage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputRepresentation {
    Preserve,
    Columns,
}

/// Upper-bound relationship between input and output point counts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PointCountEffect {
    Preserve,
    AtMostInput,
}

/// Effect on temporal and coordinate-frame metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetadataEffect {
    Preserve,
    MayDiscard,
}

/// Effect on retained point values and their primitive representation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueEffect {
    Preserve,
    MayChangeRepresentation,
}

/// Point ordering guarantee made by an operator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ordering {
    Preserve,
    StableReorder,
}

/// Repeatability guarantee made by an operator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Determinism {
    Deterministic,
    Unspecified,
}

/// Category-specific semantic or representation loss.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum FidelityLoss {
    FieldSelection,
    /// Removal of one or more coordinate Point Fields.
    CoordinateFieldSelection,
    PointSelection,
    Representation,
    Metadata,
    Order,
}

/// Explicit authorizations supplied to planning.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LossPolicy {
    authorized: BTreeSet<FidelityLoss>,
}

impl LossPolicy {
    pub fn lossless() -> Self {
        Self::default()
    }

    pub fn authorize(losses: impl IntoIterator<Item = FidelityLoss>) -> Self {
        Self {
            authorized: losses.into_iter().collect(),
        }
    }

    pub fn allows(&self, loss: FidelityLoss) -> bool {
        self.authorized.contains(&loss)
    }
}

/// Conservative operator-owned scratch formula.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScratchMemory {
    Bounded {
        fixed_bytes: u64,
        bytes_per_input_point: u64,
    },
    Unplannable,
}

impl ScratchMemory {
    pub const fn fixed(bytes: u64) -> Self {
        Self::Bounded {
            fixed_bytes: bytes,
            bytes_per_input_point: 0,
        }
    }

    pub const fn per_input_point(fixed_bytes: u64, bytes_per_input_point: u64) -> Self {
        Self::Bounded {
            fixed_bytes,
            bytes_per_input_point,
        }
    }
}

/// Everything an operator requires from its input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorInput {
    capabilities: InputCapabilities,
    schema: Box<[FieldRequirement]>,
    materialization: Materialization,
}

impl OperatorInput {
    pub fn new(
        capabilities: InputCapabilities,
        schema: impl IntoIterator<Item = FieldRequirement>,
        materialization: Materialization,
    ) -> Self {
        Self {
            capabilities,
            schema: schema.into_iter().collect(),
            materialization,
        }
    }

    pub const fn capabilities(&self) -> InputCapabilities {
        self.capabilities
    }

    pub fn schema_requirements(&self) -> &[FieldRequirement] {
        &self.schema
    }

    pub const fn materialization(&self) -> &Materialization {
        &self.materialization
    }
}

/// Everything an operator guarantees about its output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorOutput {
    schema: OutputSchema,
    representation: OutputRepresentation,
    point_count: PointCountEffect,
    metadata: MetadataEffect,
    values: ValueEffect,
}

impl OperatorOutput {
    pub const fn new(
        schema: OutputSchema,
        representation: OutputRepresentation,
        point_count: PointCountEffect,
        metadata: MetadataEffect,
        values: ValueEffect,
    ) -> Self {
        Self {
            schema,
            representation,
            point_count,
            metadata,
            values,
        }
    }

    pub const fn schema(&self) -> &OutputSchema {
        &self.schema
    }

    pub const fn representation(&self) -> OutputRepresentation {
        self.representation
    }

    pub const fn point_count(&self) -> PointCountEffect {
        self.point_count
    }

    pub const fn metadata(&self) -> MetadataEffect {
        self.metadata
    }

    pub const fn values(&self) -> ValueEffect {
        self.values
    }
}

/// Fidelity, ordering, determinism, and resource behavior.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorBehavior {
    losses: BTreeSet<FidelityLoss>,
    ordering: Ordering,
    determinism: Determinism,
    scratch: ScratchMemory,
}

impl OperatorBehavior {
    pub fn new(
        losses: impl IntoIterator<Item = FidelityLoss>,
        ordering: Ordering,
        determinism: Determinism,
        scratch: ScratchMemory,
    ) -> Self {
        Self {
            losses: losses.into_iter().collect(),
            ordering,
            determinism,
            scratch,
        }
    }

    pub fn losses(&self) -> impl Iterator<Item = FidelityLoss> + '_ {
        self.losses.iter().copied()
    }

    pub const fn ordering(&self) -> Ordering {
        self.ordering
    }

    pub const fn determinism(&self) -> Determinism {
        self.determinism
    }

    pub const fn scratch(&self) -> ScratchMemory {
        self.scratch
    }
}

/// Complete declaration for one frame-local operator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorContract {
    name: String,
    input: OperatorInput,
    output: OperatorOutput,
    behavior: OperatorBehavior,
}

impl OperatorContract {
    pub fn new(
        name: impl Into<String>,
        input: OperatorInput,
        output: OperatorOutput,
        behavior: OperatorBehavior,
    ) -> Self {
        Self {
            name: name.into(),
            input,
            output,
            behavior,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn input(&self) -> &OperatorInput {
        &self.input
    }

    pub const fn output(&self) -> &OperatorOutput {
        &self.output
    }

    pub const fn behavior(&self) -> &OperatorBehavior {
        &self.behavior
    }
}

/// Planner output for one validated operator stage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorStagePlan {
    name: String,
    input_schema: Arc<PointSchema>,
    output_schema: Arc<PointSchema>,
    input_representation: PointRepresentation,
    output_representation: PointRepresentation,
    materialized_fields: Box<[String]>,
    additional_materialization_bytes: u64,
    output_allocation_bytes: u64,
    scratch_bytes: u64,
    point_count: PointCountEffect,
    ordering: Ordering,
}

impl OperatorStagePlan {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn input_schema(&self) -> &PointSchema {
        &self.input_schema
    }

    pub fn output_schema(&self) -> &PointSchema {
        &self.output_schema
    }

    pub(crate) fn output_schema_handle(&self) -> Arc<PointSchema> {
        Arc::clone(&self.output_schema)
    }

    pub const fn input_representation(&self) -> PointRepresentation {
        self.input_representation
    }

    pub const fn output_representation(&self) -> PointRepresentation {
        self.output_representation
    }

    pub fn materialized_fields(&self) -> &[String] {
        &self.materialized_fields
    }

    pub const fn additional_materialization_bytes(&self) -> u64 {
        self.additional_materialization_bytes
    }

    pub const fn output_allocation_bytes(&self) -> u64 {
        self.output_allocation_bytes
    }

    pub const fn scratch_bytes(&self) -> u64 {
        self.scratch_bytes
    }

    pub const fn point_count(&self) -> PointCountEffect {
        self.point_count
    }

    pub const fn ordering(&self) -> Ordering {
        self.ordering
    }
}

/// A fully validated, frame-local operator pipeline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedOperatorPipeline {
    stages: Box<[OperatorStagePlan]>,
    output_schema: Arc<PointSchema>,
    output_representation: PointRepresentation,
    materialization_bytes: u64,
    peak_scratch_bytes: u64,
    output_is_unorganized: bool,
}

impl ValidatedOperatorPipeline {
    pub fn stages(&self) -> &[OperatorStagePlan] {
        &self.stages
    }

    pub fn output_schema(&self) -> &PointSchema {
        &self.output_schema
    }

    pub const fn output_representation(&self) -> PointRepresentation {
        self.output_representation
    }

    pub const fn materialization_bytes(&self) -> u64 {
        self.materialization_bytes
    }

    pub const fn peak_scratch_bytes(&self) -> u64 {
        self.peak_scratch_bytes
    }

    pub const fn output_is_unorganized(&self) -> bool {
        self.output_is_unorganized
    }
}

pub(crate) fn validate_pipeline(
    input_schema: Arc<PointSchema>,
    dimensions: PointDimensions,
    input_representation: PointRepresentation,
    contracts: &[OperatorContract],
    loss_policy: &LossPolicy,
) -> Result<ValidatedOperatorPipeline> {
    let mut schema = input_schema;
    let mut representation = input_representation;
    let mut materialized = BTreeSet::new();
    let mut materialization_bytes = 0_u64;
    let mut peak_scratch_bytes = 0_u64;
    let mut output_is_unorganized = !dimensions.is_organized();
    let mut stages = Vec::new();
    stages
        .try_reserve_exact(contracts.len())
        .map_err(|_| resource_error("operator stage table allocation failed before execution"))?;

    for contract in contracts {
        if contract.name.is_empty() {
            return Err(contract_error(
                "<unnamed>",
                "operator name must not be empty",
            ));
        }
        validate_requirements(contract, &schema)?;
        validate_behavior(contract, loss_policy)?;

        let materializes_from_view = representation == PointRepresentation::View;
        let mut stage_representation = representation;
        let mut selected = materialization_fields(contract, &schema)?;
        if !contract.input.capabilities.accepts(stage_representation) {
            match (stage_representation, contract.input.capabilities) {
                (PointRepresentation::View, InputCapabilities::Columns) => {
                    selected = schema
                        .fields()
                        .iter()
                        .map(|field| field.name().to_owned())
                        .collect();
                    stage_representation = PointRepresentation::Columns;
                }
                _ => {
                    return Err(contract_error(
                        &contract.name,
                        "cannot consume the representation produced by the preceding stage",
                    ));
                }
            }
        }

        let mut additional_fields = Vec::new();
        let mut additional_bytes = 0_u64;
        if materializes_from_view {
            for name in selected {
                if materialized.insert(name.clone()) {
                    let field = schema.field(&name).expect("resolved from current schema");
                    additional_bytes = checked_add(
                        additional_bytes,
                        materialized_field_bytes(field, dimensions.point_count())?,
                    )?;
                    additional_fields.push(name);
                }
            }
            if !additional_fields.is_empty() {
                additional_bytes = checked_add(
                    additional_bytes,
                    u64::try_from(additional_fields.len())
                        .ok()
                        .and_then(|count| count.checked_mul(size_of::<PointColumn>() as u64))
                        .ok_or_else(|| resource_error("materialized column table overflowed"))?,
                )?;
                materialization_bytes = checked_add(materialization_bytes, additional_bytes)?;
            }
        }

        let scratch_bytes = scratch_bound(contract, dimensions.point_count())?;
        peak_scratch_bytes = peak_scratch_bytes.max(scratch_bytes);
        let output_schema = resolve_output_schema(contract, &schema)?;
        let output_allocation_bytes =
            if contract.output.representation == OutputRepresentation::Columns {
                point_batch_bytes(&output_schema, dimensions.point_count())?
            } else {
                0
            };
        materialization_bytes = checked_add(materialization_bytes, output_allocation_bytes)?;
        let output_representation = match contract.output.representation {
            OutputRepresentation::Preserve => stage_representation,
            OutputRepresentation::Columns => PointRepresentation::Columns,
        };
        if contract.output.point_count == PointCountEffect::AtMostInput {
            output_is_unorganized = true;
        }

        stages.push(OperatorStagePlan {
            name: contract.name.clone(),
            input_schema: Arc::clone(&schema),
            output_schema: Arc::clone(&output_schema),
            input_representation: stage_representation,
            output_representation,
            materialized_fields: additional_fields.into_boxed_slice(),
            additional_materialization_bytes: additional_bytes,
            output_allocation_bytes,
            scratch_bytes,
            point_count: contract.output.point_count,
            ordering: contract.behavior.ordering,
        });
        schema = output_schema;
        representation = output_representation;
        materialized.retain(|name| schema.field(name).is_some());
    }

    Ok(ValidatedOperatorPipeline {
        stages: stages.into_boxed_slice(),
        output_schema: schema,
        output_representation: representation,
        materialization_bytes,
        peak_scratch_bytes,
        output_is_unorganized,
    })
}

fn validate_requirements(contract: &OperatorContract, schema: &PointSchema) -> Result<()> {
    for requirement in &contract.input.schema {
        let field = resolve_field(&requirement.selector, schema).map_err(|message| {
            contract_error(
                &contract.name,
                format!("schema requirement failed: {message}"),
            )
        })?;
        if !requirement.primitives.accepts(field.primitive()) {
            return Err(contract_error(
                &contract.name,
                format!(
                    "field {:?} has unsupported primitive {:?}",
                    field.name(),
                    field.primitive()
                ),
            ));
        }
        if let ElementCountRequirement::Exactly(expected) = requirement.elements
            && field.count() != expected
        {
            return Err(contract_error(
                &contract.name,
                format!(
                    "field {:?} has {} elements; expected {expected}",
                    field.name(),
                    field.count()
                ),
            ));
        }
    }
    Ok(())
}

fn validate_behavior(contract: &OperatorContract, policy: &LossPolicy) -> Result<()> {
    if contract.behavior.determinism != Determinism::Deterministic {
        return Err(contract_error(
            &contract.name,
            "does not guarantee deterministic output",
        ));
    }

    let implied = [
        (
            matches!(contract.output.schema, OutputSchema::Project(_)),
            FidelityLoss::FieldSelection,
        ),
        (
            contract.output.point_count == PointCountEffect::AtMostInput,
            FidelityLoss::PointSelection,
        ),
        (
            contract.output.metadata == MetadataEffect::MayDiscard,
            FidelityLoss::Metadata,
        ),
        (
            contract.output.values == ValueEffect::MayChangeRepresentation,
            FidelityLoss::Representation,
        ),
        (
            contract.behavior.ordering == Ordering::StableReorder,
            FidelityLoss::Order,
        ),
    ];
    for (applies, loss) in implied {
        if applies && !contract.behavior.losses.contains(&loss) {
            return Err(contract_error(
                &contract.name,
                format!("output implies undeclared {loss:?} loss"),
            ));
        }
    }
    for loss in &contract.behavior.losses {
        if !policy.allows(*loss) {
            return Err(contract_error(
                &contract.name,
                format!("requires explicit {loss:?} loss authorization"),
            ));
        }
    }
    Ok(())
}

fn materialization_fields(
    contract: &OperatorContract,
    schema: &PointSchema,
) -> Result<Vec<String>> {
    let selectors: Vec<FieldSelector> = match &contract.input.materialization {
        Materialization::None => return Ok(Vec::new()),
        Materialization::Fields(selectors) => selectors.to_vec(),
        Materialization::AllFields => schema
            .fields()
            .iter()
            .map(|field| FieldSelector::named(field.name()))
            .collect(),
    };
    selectors
        .iter()
        .map(|selector| {
            resolve_field(selector, schema)
                .map(|field| field.name().to_owned())
                .map_err(|message| {
                    contract_error(
                        &contract.name,
                        format!("materialization requirement failed: {message}"),
                    )
                })
        })
        .collect()
}

fn resolve_output_schema(
    contract: &OperatorContract,
    input: &Arc<PointSchema>,
) -> Result<Arc<PointSchema>> {
    let OutputSchema::Project(selectors) = &contract.output.schema else {
        return Ok(Arc::clone(input));
    };
    let mut names = BTreeSet::new();
    let mut fields = Vec::new();
    fields.try_reserve_exact(selectors.len()).map_err(|_| {
        resource_error(format!(
            "operator {:?} output schema allocation failed",
            contract.name
        ))
    })?;
    for selector in selectors {
        let field = resolve_field(selector, input).map_err(|message| {
            contract_error(
                &contract.name,
                format!("output schema projection failed: {message}"),
            )
        })?;
        if !names.insert(field.name()) {
            return Err(contract_error(
                &contract.name,
                format!("output schema repeats field {:?}", field.name()),
            ));
        }
        fields.push(field.clone());
    }
    PointSchema::new(fields)
        .map(Arc::new)
        .map_err(|error| contract_error(&contract.name, format!("invalid output schema: {error}")))
}

fn resolve_field<'a>(
    selector: &FieldSelector,
    schema: &'a PointSchema,
) -> std::result::Result<&'a PointField, String> {
    match selector {
        FieldSelector::Name(name) => schema
            .field(name)
            .ok_or_else(|| format!("field {name:?} is missing")),
        FieldSelector::Semantic(semantic) => {
            let mut matches = schema
                .fields()
                .iter()
                .filter(|field| field.semantic() == Some(*semantic));
            let field = matches
                .next()
                .ok_or_else(|| format!("field with semantic {semantic:?} is missing"))?;
            if matches.next().is_some() {
                return Err(format!("field semantic {semantic:?} is ambiguous"));
            }
            Ok(field)
        }
    }
}

fn scratch_bound(contract: &OperatorContract, point_count: usize) -> Result<u64> {
    match contract.behavior.scratch {
        ScratchMemory::Bounded {
            fixed_bytes,
            bytes_per_input_point,
        } => {
            let points = u64::try_from(point_count)
                .map_err(|_| resource_error("point count cannot be represented by the planner"))?;
            fixed_bytes
                .checked_add(bytes_per_input_point.checked_mul(points).ok_or_else(|| {
                    resource_error(format!(
                        "operator {:?} scratch bound overflowed",
                        contract.name
                    ))
                })?)
                .ok_or_else(|| {
                    resource_error(format!(
                        "operator {:?} scratch bound overflowed",
                        contract.name
                    ))
                })
        }
        ScratchMemory::Unplannable => Err(resource_error(format!(
            "operator {:?} scratch cannot be bounded before execution",
            contract.name
        ))),
    }
}

fn materialized_field_bytes(field: &PointField, point_count: usize) -> Result<u64> {
    let scalars = point_count
        .checked_mul(field.count())
        .ok_or_else(|| resource_error("materialized scalar count overflowed"))?;
    let bytes = scalars
        .checked_mul(field.primitive().size())
        .ok_or_else(|| resource_error("materialized field size overflowed"))?;
    u64::try_from(bytes)
        .map_err(|_| resource_error("materialized field size cannot be represented by the planner"))
}

fn point_batch_bytes(schema: &PointSchema, point_count: usize) -> Result<u64> {
    let values = schema.fields().iter().try_fold(0_u64, |total, field| {
        checked_add(total, materialized_field_bytes(field, point_count)?)
    })?;
    let table = u64::try_from(schema.fields().len())
        .ok()
        .and_then(|count| count.checked_mul(size_of::<PointColumn>() as u64))
        .ok_or_else(|| resource_error("output column table overflowed"))?;
    checked_add(values, table)
}

fn checked_add(total: u64, bytes: u64) -> Result<u64> {
    total
        .checked_add(bytes)
        .ok_or_else(|| resource_error("operator pipeline memory estimate overflowed"))
}

fn contract_error(name: &str, message: impl Into<String>) -> Error {
    Error::new(
        ErrorCategory::Unsupported,
        format!("operator {name:?}: {}", message.into()),
    )
}

fn resource_error(message: impl Into<String>) -> Error {
    Error::new(ErrorCategory::Resource, message)
}
