use serde::Serialize;

use super::{ErrorCategory, ExecutionPlan, JobKind};

/// Current schema for machine-readable execution reports.
pub const MACHINE_REPORT_SCHEMA_VERSION: u32 = 1;

/// Versioned machine-readable envelope returned when a JSON command fails.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ErrorReport {
    schema_version: u32,
    command: JobKind,
    error: ErrorData,
}

/// Stable failure fields for machine-readable command diagnostics.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ErrorData {
    category: ErrorCategory,
    message: String,
}

impl ErrorReport {
    pub fn new(command: JobKind, category: ErrorCategory, message: impl Into<String>) -> Self {
        Self {
            schema_version: MACHINE_REPORT_SCHEMA_VERSION,
            command,
            error: ErrorData {
                category,
                message: message.into(),
            },
        }
    }
}

/// Versioned machine-readable envelope returned after successful execution.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExecutionReport<T> {
    schema_version: u32,
    command: JobKind,
    data: T,
}

impl<T> ExecutionReport<T> {
    /// Build the success envelope for a checked execution plan.
    pub fn success(plan: &ExecutionPlan, data: T) -> Self {
        Self {
            schema_version: MACHINE_REPORT_SCHEMA_VERSION,
            command: plan.job().kind(),
            data,
        }
    }

    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub const fn command(&self) -> JobKind {
        self.command
    }

    pub fn data(&self) -> &T {
        &self.data
    }

    pub fn into_data(self) -> T {
        self.data
    }
}

#[cfg(test)]
mod tests {
    use serde::Serialize;
    use serde_json::json;

    use super::{ErrorReport, ExecutionReport, MACHINE_REPORT_SCHEMA_VERSION};
    use crate::core::{
        ByteBound, ErrorCategory, JobKind, JobSpec, PipelineMemoryRequirements, Planner, SourceSpec,
    };

    #[derive(Debug, Eq, PartialEq, Serialize)]
    struct Summary {
        bytes_written: u64,
    }

    #[test]
    fn successful_machine_report_has_a_versioned_envelope() {
        let job = JobSpec::info(SourceSpec::file("recording.bin").expect("valid source"));
        let requirements = PipelineMemoryRequirements::new(
            ByteBound::bounded(1),
            ByteBound::bounded(1),
            ByteBound::bounded(1),
            ByteBound::bounded(1),
            ByteBound::bounded(1),
            ByteBound::bounded(1),
        );
        let plan = Planner::new()
            .plan(job, requirements, 128 * 1024)
            .expect("valid execution plan");
        let report = ExecutionReport::success(&plan, Summary { bytes_written: 42 });

        assert_eq!(report.schema_version(), MACHINE_REPORT_SCHEMA_VERSION);
        assert_eq!(report.command(), JobKind::Info);
        assert_eq!(report.data().bytes_written, 42);
        assert_eq!(
            serde_json::to_value(&report).expect("report should serialize"),
            json!({
                "schema_version": 1,
                "command": "info",
                "data": { "bytes_written": 42 }
            })
        );
    }

    #[test]
    fn error_report_has_the_same_versioned_command_context() {
        let report = ErrorReport::new(JobKind::Topics, ErrorCategory::InvalidData, "bad source");

        assert_eq!(
            serde_json::to_value(&report).expect("report should serialize"),
            json!({
                "schema_version": 1,
                "command": "topics",
                "error": {
                    "category": "invalid_data",
                    "message": "bad source"
                }
            })
        );
    }
}
