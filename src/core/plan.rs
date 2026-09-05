use std::sync::Arc;

use super::{
    Error, ErrorCategory, JobKind, JobSpec, LossPolicy, OperatorContract, PointRepresentation,
    Result, ValidatedOperatorPipeline,
    operator::validate_pipeline,
    point::{PointDimensions, PointSchema, PointView},
};

const FIXED_OVERHEAD_BYTES: u64 = 64 * 1024;
const PROPORTIONAL_OVERHEAD_DIVISOR: u64 = 8;

/// A byte requirement that must be bounded before execution can start.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ByteBound {
    Bounded(u64),
    Unplannable,
}

impl ByteBound {
    pub const fn bounded(bytes: u64) -> Self {
        Self::Bounded(bytes)
    }

    pub const fn unplannable() -> Self {
        Self::Unplannable
    }
}

/// Managed-memory requirements whose lifetimes may overlap during a pipeline.
///
/// Whole-process RSS, allocator metadata, shared libraries, memory mappings,
/// and the page cache are deliberately outside this contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PipelineMemoryRequirements {
    retained_input: ByteBound,
    materialization: ByteBound,
    operator_scratch: ByteBound,
    encoder_buffer: ByteBound,
    output_buffer: ByteBound,
    queued_output: ByteBound,
}

impl PipelineMemoryRequirements {
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        retained_input: ByteBound,
        materialization: ByteBound,
        operator_scratch: ByteBound,
        encoder_buffer: ByteBound,
        output_buffer: ByteBound,
        queued_output: ByteBound,
    ) -> Self {
        Self {
            retained_input,
            materialization,
            operator_scratch,
            encoder_buffer,
            output_buffer,
            queued_output,
        }
    }

    /// Derive the point-owned portions of the bound from a validated view.
    ///
    /// Stage owners still declare their own worst-case scratch and buffer
    /// requirements. Passing [`ByteBound::Unplannable`] refuses the job rather
    /// than guessing or weakening the memory contract.
    pub fn for_point_view(
        view: &PointView,
        materialize: bool,
        operator_scratch: ByteBound,
        encoder_buffer: ByteBound,
        output_buffer: ByteBound,
        queued_output: ByteBound,
    ) -> Result<Self> {
        let retained_input = u64::try_from(view.source_len()).map_err(|_| {
            resource_error("retained input length cannot be represented in the planner")
        })?;
        let materialization = if materialize {
            let bytes = view.materialized_size().map_err(|error| {
                resource_error(format!("materialization size cannot be proven: {error}"))
            })?;
            u64::try_from(bytes).map_err(|_| {
                resource_error("materialization size cannot be represented in the planner")
            })?
        } else {
            0
        };

        Ok(Self::new(
            ByteBound::Bounded(retained_input),
            ByteBound::Bounded(materialization),
            operator_scratch,
            encoder_buffer,
            output_buffer,
            queued_output,
        ))
    }

    /// Derive point-owned requirements from a validated operator pipeline.
    pub fn for_operator_pipeline(
        view: &PointView,
        operators: &ValidatedOperatorPipeline,
        encoder_buffer: ByteBound,
        output_buffer: ByteBound,
        queued_output: ByteBound,
    ) -> Result<Self> {
        let retained_input = u64::try_from(view.source_len()).map_err(|_| {
            resource_error("retained input length cannot be represented in the planner")
        })?;
        Ok(Self::new(
            ByteBound::Bounded(retained_input),
            ByteBound::Bounded(operators.materialization_bytes()),
            ByteBound::Bounded(operators.peak_scratch_bytes()),
            encoder_buffer,
            output_buffer,
            queued_output,
        ))
    }
}

/// Itemized conservative upper bound for memory managed by `pcx`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManagedMemoryBreakdown {
    retained_input_bytes: u64,
    materialization_bytes: u64,
    operator_scratch_bytes: u64,
    encoder_buffer_bytes: u64,
    output_buffer_bytes: u64,
    queued_output_bytes: u64,
    overhead_bytes: u64,
}

impl ManagedMemoryBreakdown {
    pub const fn retained_input_bytes(self) -> u64 {
        self.retained_input_bytes
    }

    pub const fn materialization_bytes(self) -> u64 {
        self.materialization_bytes
    }

    pub const fn operator_scratch_bytes(self) -> u64 {
        self.operator_scratch_bytes
    }

    pub const fn encoder_buffer_bytes(self) -> u64 {
        self.encoder_buffer_bytes
    }

    pub const fn output_buffer_bytes(self) -> u64 {
        self.output_buffer_bytes
    }

    pub const fn queued_output_bytes(self) -> u64 {
        self.queued_output_bytes
    }

    pub const fn overhead_bytes(self) -> u64 {
        self.overhead_bytes
    }
}

/// A proven upper bound for memory managed by `pcx` during one job.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManagedMemoryBound {
    breakdown: ManagedMemoryBreakdown,
    peak_bytes: u64,
    limit_bytes: u64,
}

impl ManagedMemoryBound {
    pub const fn breakdown(self) -> ManagedMemoryBreakdown {
        self.breakdown
    }

    pub const fn peak_bytes(self) -> u64 {
        self.peak_bytes
    }

    pub const fn limit_bytes(self) -> u64 {
        self.limit_bytes
    }
}

/// The explicit data plane selected for execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionMode {
    Container,
    SemanticPointFrame,
}

/// An executable job whose state and managed-memory bound passed preflight.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionPlan {
    job: JobSpec,
    mode: ExecutionMode,
    memory: ManagedMemoryBound,
}

impl ExecutionPlan {
    pub fn job(&self) -> &JobSpec {
        &self.job
    }

    pub const fn mode(&self) -> ExecutionMode {
        self.mode
    }

    pub const fn memory(&self) -> ManagedMemoryBound {
        self.memory
    }
}

/// Strict preflight planner for managed-memory requirements.
#[derive(Clone, Copy, Debug, Default)]
pub struct Planner;

impl Planner {
    pub const fn new() -> Self {
        Self
    }

    /// Validate a frame-local operator chain without executing it.
    pub fn validate_operators(
        self,
        input_schema: Arc<PointSchema>,
        dimensions: PointDimensions,
        input_representation: PointRepresentation,
        contracts: &[OperatorContract],
        loss_policy: &LossPolicy,
    ) -> Result<ValidatedOperatorPipeline> {
        validate_pipeline(
            input_schema,
            dimensions,
            input_representation,
            contracts,
            loss_policy,
        )
    }

    /// Prove a conservative peak before any executor or output sink is created.
    pub fn plan(
        self,
        job: JobSpec,
        requirements: PipelineMemoryRequirements,
        limit_bytes: u64,
    ) -> Result<ExecutionPlan> {
        let retained_input_bytes = bounded("retained input", requirements.retained_input)?;
        let materialization_bytes = bounded("materialization", requirements.materialization)?;
        let operator_scratch_bytes = bounded("operator scratch", requirements.operator_scratch)?;
        let encoder_buffer_bytes = bounded("encoder buffer", requirements.encoder_buffer)?;
        let output_buffer_bytes = bounded("output buffer", requirements.output_buffer)?;
        let queued_output_bytes = bounded("queued output", requirements.queued_output)?;

        let components = [
            retained_input_bytes,
            materialization_bytes,
            operator_scratch_bytes,
            encoder_buffer_bytes,
            output_buffer_bytes,
            queued_output_bytes,
        ];
        let subtotal = components.into_iter().try_fold(0_u64, checked_add)?;
        // Reserve 64 KiB plus a rounded-up 12.5% for managed control
        // structures and other bounded allocations not owned by one stage.
        let proportional_overhead = subtotal
            .checked_add(PROPORTIONAL_OVERHEAD_DIVISOR - 1)
            .ok_or_else(arithmetic_overflow)?
            / PROPORTIONAL_OVERHEAD_DIVISOR;
        let overhead_bytes = FIXED_OVERHEAD_BYTES
            .checked_add(proportional_overhead)
            .ok_or_else(arithmetic_overflow)?;
        let peak_bytes = subtotal
            .checked_add(overhead_bytes)
            .ok_or_else(arithmetic_overflow)?;

        if peak_bytes > limit_bytes {
            return Err(resource_error(format!(
                "planned managed-memory peak of {peak_bytes} bytes exceeds the {limit_bytes}-byte limit"
            )));
        }

        let mode = match job.kind() {
            JobKind::Info | JobKind::Topics | JobKind::Passthrough => ExecutionMode::Container,
            JobKind::Extract => ExecutionMode::SemanticPointFrame,
        };
        let breakdown = ManagedMemoryBreakdown {
            retained_input_bytes,
            materialization_bytes,
            operator_scratch_bytes,
            encoder_buffer_bytes,
            output_buffer_bytes,
            queued_output_bytes,
            overhead_bytes,
        };

        Ok(ExecutionPlan {
            job,
            mode,
            memory: ManagedMemoryBound {
                breakdown,
                peak_bytes,
                limit_bytes,
            },
        })
    }
}

fn bounded(name: &str, bound: ByteBound) -> Result<u64> {
    match bound {
        ByteBound::Bounded(bytes) => Ok(bytes),
        ByteBound::Unplannable => Err(resource_error(format!(
            "managed memory for {name} cannot be bounded before execution"
        ))),
    }
}

fn checked_add(total: u64, bytes: u64) -> Result<u64> {
    total.checked_add(bytes).ok_or_else(arithmetic_overflow)
}

fn arithmetic_overflow() -> Error {
    resource_error("managed-memory estimate overflowed")
}

fn resource_error(message: impl Into<String>) -> Error {
    Error::new(ErrorCategory::Resource, message)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{ByteBound, ExecutionMode, PipelineMemoryRequirements, Planner};
    use crate::core::{
        Destination, Determinism, ErrorCategory, FrameSelector, InputCapabilities, JobSpec,
        LossPolicy, Materialization, MetadataEffect, OperatorBehavior, OperatorContract,
        OperatorInput, OperatorOutput, Ordering, OutputRepresentation, OutputSchema,
        PointCountEffect, PointRepresentation, ScratchMemory, SourceSpec, ValueEffect,
        point::{
            Endianness, PointDimensions, PointField, PointFrameMetadata, PointLayout, PointSchema,
            PointView, PrimitiveType, Timestamp,
        },
    };

    fn requirements(values: [u64; 6]) -> PipelineMemoryRequirements {
        PipelineMemoryRequirements::new(
            ByteBound::bounded(values[0]),
            ByteBound::bounded(values[1]),
            ByteBound::bounded(values[2]),
            ByteBound::bounded(values[3]),
            ByteBound::bounded(values[4]),
            ByteBound::bounded(values[5]),
        )
    }

    fn extract_job(destination: Destination) -> JobSpec {
        JobSpec::extract(
            SourceSpec::file("recording.mcap").unwrap(),
            "/lidar/points",
            FrameSelector::Index(0),
            destination,
        )
        .unwrap()
    }

    fn point_view() -> PointView {
        let schema = Arc::new(
            PointSchema::new(vec![
                PointField::new("x", PrimitiveType::F32, 1, None).unwrap(),
                PointField::new("ring", PrimitiveType::U16, 1, None).unwrap(),
            ])
            .unwrap(),
        );
        let layout = PointLayout::new(
            schema,
            PointDimensions::new(2, 1).unwrap(),
            vec![0, 4],
            8,
            16,
            0,
            Endianness::Little,
        )
        .unwrap();
        PointView::new(
            Arc::from(vec![0_u8; 16]),
            Arc::new(PointFrameMetadata::new(
                Timestamp::new(1, 2).unwrap(),
                "lidar",
                true,
            )),
            layout,
        )
        .unwrap()
    }

    #[test]
    fn derives_retained_input_and_materialization_from_the_point_model() {
        let view = point_view();
        let materialized = u64::try_from(view.materialized_size().unwrap()).unwrap();
        let requirements = PipelineMemoryRequirements::for_point_view(
            &view,
            true,
            ByteBound::bounded(11),
            ByteBound::bounded(12),
            ByteBound::bounded(13),
            ByteBound::bounded(14),
        )
        .unwrap();
        let plan = Planner::new()
            .plan(extract_job(Destination::stdout()), requirements, u64::MAX)
            .unwrap();
        let breakdown = plan.memory().breakdown();

        assert_eq!(plan.mode(), ExecutionMode::SemanticPointFrame);
        assert_eq!(breakdown.retained_input_bytes(), 16);
        assert_eq!(breakdown.materialization_bytes(), materialized);
        assert_eq!(breakdown.operator_scratch_bytes(), 11);
        assert_eq!(breakdown.encoder_buffer_bytes(), 12);
        assert_eq!(breakdown.output_buffer_bytes(), 13);
        assert_eq!(breakdown.queued_output_bytes(), 14);
        assert!(breakdown.overhead_bytes() >= 64 * 1024);
    }

    #[test]
    fn validated_operator_bounds_feed_the_managed_memory_plan() {
        let view = point_view();
        let contract = OperatorContract::new(
            "view-inspection",
            OperatorInput::new(
                InputCapabilities::View,
                [],
                Materialization::fields([crate::core::FieldSelector::named("x")]),
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
                ScratchMemory::fixed(11),
            ),
        );
        let operators = Planner::new()
            .validate_operators(
                Arc::new(view.schema().clone()),
                view.layout().dimensions(),
                PointRepresentation::View,
                &[contract],
                &LossPolicy::lossless(),
            )
            .unwrap();
        let requirements = PipelineMemoryRequirements::for_operator_pipeline(
            &view,
            &operators,
            ByteBound::bounded(12),
            ByteBound::bounded(13),
            ByteBound::bounded(14),
        )
        .unwrap();
        let breakdown = Planner::new()
            .plan(extract_job(Destination::stdout()), requirements, u64::MAX)
            .unwrap()
            .memory()
            .breakdown();

        assert_eq!(
            breakdown.materialization_bytes(),
            operators.materialization_bytes()
        );
        assert_eq!(breakdown.operator_scratch_bytes(), 11);
    }

    #[test]
    fn accepts_the_exact_limit_and_refuses_one_byte_less() {
        let job = extract_job(Destination::stdout());
        let requirements = requirements([1, 2, 3, 4, 5, 6]);
        let initial = Planner::new()
            .plan(job.clone(), requirements, u64::MAX)
            .unwrap();
        let exact = initial.memory().peak_bytes();

        let plan = Planner::new()
            .plan(job.clone(), requirements, exact)
            .unwrap();
        assert_eq!(plan.memory().peak_bytes(), exact);
        assert_eq!(plan.memory().limit_bytes(), exact);

        let error = Planner::new()
            .plan(job, requirements, exact - 1)
            .expect_err("one byte below the peak must fail");
        assert_eq!(error.category(), ErrorCategory::Resource);
        assert!(error.message().contains("exceeds"));
    }

    #[test]
    fn estimate_is_monotonic_for_every_managed_component() {
        let job = extract_job(Destination::stdout());
        let baseline = Planner::new()
            .plan(job.clone(), requirements([10; 6]), u64::MAX)
            .unwrap()
            .memory()
            .peak_bytes();

        for index in 0..6 {
            let mut values = [10; 6];
            values[index] += 1;
            let increased = Planner::new()
                .plan(job.clone(), requirements(values), u64::MAX)
                .unwrap()
                .memory()
                .peak_bytes();
            assert!(increased > baseline, "component {index} was not monotonic");
        }
    }

    #[test]
    fn checked_arithmetic_refuses_an_overflow() {
        let error = Planner::new()
            .plan(
                extract_job(Destination::stdout()),
                requirements([u64::MAX, 1, 0, 0, 0, 0]),
                u64::MAX,
            )
            .expect_err("overflow must be unplannable");

        assert_eq!(error.category(), ErrorCategory::Resource);
        assert!(error.message().contains("overflowed"));
    }

    #[test]
    fn refuses_unplannable_or_excess_work_before_creating_file_output() {
        let path = std::env::temp_dir().join(format!(
            "pcx-planner-refusal-{}-must-not-exist.pcd",
            std::process::id()
        ));
        assert!(!path.exists(), "test output path unexpectedly exists");
        let job = extract_job(Destination::file(&path, false).unwrap());
        let unplannable_requirements = PipelineMemoryRequirements::new(
            ByteBound::bounded(1),
            ByteBound::bounded(2),
            ByteBound::unplannable(),
            ByteBound::bounded(4),
            ByteBound::bounded(5),
            ByteBound::bounded(6),
        );

        let error = Planner::new()
            .plan(job, unplannable_requirements, u64::MAX)
            .expect_err("unbounded scratch must be refused");

        assert_eq!(error.category(), ErrorCategory::Resource);
        assert!(error.message().contains("operator scratch"));
        assert!(!path.exists(), "preflight must not create output");

        let job = extract_job(Destination::file(&path, false).unwrap());
        let error = Planner::new()
            .plan(job, requirements([1, 2, 3, 4, 5, 6]), 1)
            .expect_err("work above the limit must be refused");

        assert_eq!(error.category(), ErrorCategory::Resource);
        assert!(error.message().contains("exceeds"));
        assert!(!path.exists(), "preflight must not create output");
    }
}
