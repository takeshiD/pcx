//! Format-independent contracts for planning and executing `pcx` jobs.
//!
//! Format adapters depend on this module. This module deliberately contains no
//! MCAP, ROS 2, PCD, or other concrete format knowledge.

mod error;
mod job;
mod plan;
mod report;

pub use error::{Error, ErrorCategory, Result};
pub use job::{Destination, FrameSelector, JobKind, JobSpec, SourceSpec};
pub use plan::{ExecutionMode, ExecutionPlan, ManagedMemoryBound};
pub use report::{ExecutionReport, MACHINE_REPORT_SCHEMA_VERSION};
