use super::{Error, ErrorCategory, JobKind, JobSpec, Result};

/// A proven upper bound for memory managed by `pcx` during one job.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManagedMemoryBound {
    peak_bytes: u64,
    limit_bytes: u64,
}

impl ManagedMemoryBound {
    /// Check a planner's conservative peak against the configured hard limit.
    pub fn checked(peak_bytes: u64, limit_bytes: u64) -> Result<Self> {
        if peak_bytes > limit_bytes {
            return Err(Error::new(
                ErrorCategory::Resource,
                format!(
                    "planned managed-memory peak of {peak_bytes} bytes exceeds the {limit_bytes}-byte limit"
                ),
            ));
        }

        Ok(Self {
            peak_bytes,
            limit_bytes,
        })
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

/// An executable job whose basic state and managed-memory bound are checked.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionPlan {
    job: JobSpec,
    mode: ExecutionMode,
    memory: ManagedMemoryBound,
}

impl ExecutionPlan {
    /// Seal a validated job and a proven memory bound into an execution plan.
    ///
    /// Resource-estimate calculation belongs to the Planner; this boundary only
    /// accepts an already checked bound and makes the selected data plane explicit.
    pub fn checked(job: JobSpec, memory: ManagedMemoryBound) -> Self {
        let mode = match job.kind() {
            JobKind::Info | JobKind::Topics => ExecutionMode::Container,
            JobKind::Extract => ExecutionMode::SemanticPointFrame,
        };

        Self { job, mode, memory }
    }

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

#[cfg(test)]
mod tests {
    use super::{ExecutionMode, ExecutionPlan, ManagedMemoryBound};
    use crate::core::{Destination, ErrorCategory, FrameSelector, JobSpec, SourceSpec};

    #[test]
    fn accepts_a_peak_within_the_hard_limit() {
        let bound = ManagedMemoryBound::checked(512, 1_024).expect("bound should fit");
        assert_eq!(bound.peak_bytes(), 512);
        assert_eq!(bound.limit_bytes(), 1_024);
    }

    #[test]
    fn rejects_a_peak_above_the_hard_limit_before_a_plan_exists() {
        let error = ManagedMemoryBound::checked(1_025, 1_024).expect_err("peak must fail");
        assert_eq!(error.category(), ErrorCategory::Resource);
    }

    #[test]
    fn makes_the_selected_data_plane_explicit() {
        let source = SourceSpec::file("recording.bin").expect("valid source");
        let memory = ManagedMemoryBound::checked(128, 128).expect("valid bound");
        let info = ExecutionPlan::checked(JobSpec::info(source.clone()), memory);
        let extract = ExecutionPlan::checked(
            JobSpec::extract(
                source,
                "/lidar/points",
                FrameSelector::Index(0),
                Destination::stdout(),
            )
            .expect("valid extraction"),
            memory,
        );

        assert_eq!(info.mode(), ExecutionMode::Container);
        assert_eq!(extract.mode(), ExecutionMode::SemanticPointFrame);
        assert_eq!(extract.memory(), memory);
    }
}
