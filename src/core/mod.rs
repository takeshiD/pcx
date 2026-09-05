//! Format-independent contracts for planning and executing `pcx` jobs.
//!
//! Format adapters depend on this module. This module deliberately contains no
//! MCAP, ROS 2, PCD, or other concrete format knowledge.

mod error;
mod job;
mod operator;
mod plan;
pub mod point;
mod report;
mod sink;

pub use error::{Error, ErrorCategory, Result};
pub use job::{Destination, FrameSelector, JobKind, JobSpec, SourceSpec};
pub use operator::{
    Determinism, ElementCountRequirement, FidelityLoss, FieldRequirement, FieldSelector,
    InputCapabilities, LossPolicy, Materialization, MetadataEffect, OperatorBehavior,
    OperatorContract, OperatorInput, OperatorOutput, OperatorStagePlan, Ordering,
    OutputRepresentation, OutputSchema, PointCountEffect, PointRepresentation,
    PrimitiveRequirement, ScratchMemory, ValidatedOperatorPipeline, ValueEffect,
};
pub use plan::{
    ByteBound, ExecutionMode, ExecutionPlan, ManagedMemoryBound, ManagedMemoryBreakdown,
    PipelineMemoryRequirements, Planner,
};
pub use report::{ErrorReport, ExecutionReport, MACHINE_REPORT_SCHEMA_VERSION};
pub use sink::{Cancellation, OutputOutcome, write_output, write_stream};
