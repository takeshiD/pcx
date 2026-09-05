use std::{path::PathBuf, time::Duration};

use super::{Error, ErrorCategory, Result};

/// The command-level behavior requested by a job.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Info,
    Topics,
    Extract,
}

/// A validated local Source description.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceSpec {
    path: PathBuf,
}

impl SourceSpec {
    /// Describe a file Source without opening it.
    pub fn file(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if path.as_os_str().is_empty() {
            return Err(Error::new(
                ErrorCategory::Usage,
                "source path must not be empty",
            ));
        }
        Ok(Self { path })
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

/// Exactly one way to select a Point Frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameSelector {
    /// A zero-based index among messages matching the selected Topic.
    Index(u64),
    /// The first Point Frame at or after this duration from recording start.
    At(Duration),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum DestinationKind {
    Stdout,
    File { path: PathBuf, force: bool },
}

/// A validated output destination for an extraction job.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Destination(DestinationKind);

impl Destination {
    /// Select standard output. No file-only state can accompany this choice.
    pub const fn stdout() -> Self {
        Self(DestinationKind::Stdout)
    }

    /// Describe file output without creating or replacing the destination.
    pub fn file(path: impl Into<PathBuf>, force: bool) -> Result<Self> {
        let path = path.into();
        if path.as_os_str().is_empty() {
            return Err(Error::new(
                ErrorCategory::Usage,
                "destination path must not be empty",
            ));
        }
        Ok(Self(DestinationKind::File { path, force }))
    }

    pub const fn is_stdout(&self) -> bool {
        matches!(&self.0, DestinationKind::Stdout)
    }

    pub fn file_path(&self) -> Option<&std::path::Path> {
        match &self.0 {
            DestinationKind::Stdout => None,
            DestinationKind::File { path, .. } => Some(path),
        }
    }

    pub const fn force(&self) -> bool {
        match &self.0 {
            DestinationKind::Stdout => false,
            DestinationKind::File { force, .. } => *force,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Operation {
    Info,
    Topics,
    Extract {
        topic: String,
        selector: FrameSelector,
        destination: Destination,
    },
}

/// A validated, format-independent description of requested behavior.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobSpec {
    source: SourceSpec,
    operation: Operation,
}

impl JobSpec {
    pub fn info(source: SourceSpec) -> Self {
        Self {
            source,
            operation: Operation::Info,
        }
    }

    pub fn topics(source: SourceSpec) -> Self {
        Self {
            source,
            operation: Operation::Topics,
        }
    }

    pub fn extract(
        source: SourceSpec,
        topic: impl Into<String>,
        selector: FrameSelector,
        destination: Destination,
    ) -> Result<Self> {
        let topic = topic.into();
        if topic.trim().is_empty() {
            return Err(Error::new(ErrorCategory::Usage, "Topic must not be empty"));
        }

        Ok(Self {
            source,
            operation: Operation::Extract {
                topic,
                selector,
                destination,
            },
        })
    }

    pub const fn kind(&self) -> JobKind {
        match self.operation {
            Operation::Info => JobKind::Info,
            Operation::Topics => JobKind::Topics,
            Operation::Extract { .. } => JobKind::Extract,
        }
    }

    pub fn source(&self) -> &SourceSpec {
        &self.source
    }

    pub fn extraction(&self) -> Option<(&str, FrameSelector, &Destination)> {
        match &self.operation {
            Operation::Extract {
                topic,
                selector,
                destination,
            } => Some((topic, *selector, destination)),
            Operation::Info | Operation::Topics => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{path::Path, time::Duration};

    use super::{Destination, FrameSelector, JobKind, JobSpec, SourceSpec};
    use crate::core::ErrorCategory;

    #[test]
    fn constructs_each_valid_job_without_format_knowledge() {
        let source = SourceSpec::file("recording.bin").expect("valid source");
        let info = JobSpec::info(source.clone());
        let topics = JobSpec::topics(source.clone());
        let extract = JobSpec::extract(
            source,
            "/lidar/points",
            FrameSelector::Index(0),
            Destination::stdout(),
        )
        .expect("valid extraction");

        assert_eq!(info.kind(), JobKind::Info);
        assert_eq!(topics.kind(), JobKind::Topics);
        assert_eq!(extract.kind(), JobKind::Extract);
        assert_eq!(extract.source().path(), Path::new("recording.bin"));
        assert_eq!(
            extract.extraction(),
            Some((
                "/lidar/points",
                FrameSelector::Index(0),
                &Destination::stdout()
            ))
        );
    }

    #[test]
    fn selector_is_one_checked_state_instead_of_two_optional_values() {
        let source = SourceSpec::file("recording.bin").expect("valid source");
        let by_time = JobSpec::extract(
            source,
            "/lidar/points",
            FrameSelector::At(Duration::from_millis(250)),
            Destination::file("frame.pcd", true).expect("valid destination"),
        )
        .expect("valid extraction");

        let (_, selector, destination) = by_time.extraction().expect("extraction state");
        assert_eq!(selector, FrameSelector::At(Duration::from_millis(250)));
        assert_eq!(destination.file_path(), Some(Path::new("frame.pcd")));
        assert!(destination.force());
    }

    #[test]
    fn rejects_empty_source_destination_and_topic_at_construction() {
        let source_error = SourceSpec::file("").expect_err("empty source must fail");
        let destination_error =
            Destination::file("", false).expect_err("empty destination must fail");
        let topic_error = JobSpec::extract(
            SourceSpec::file("recording.bin").expect("valid source"),
            "  ",
            FrameSelector::Index(0),
            Destination::stdout(),
        )
        .expect_err("empty Topic must fail");

        assert_eq!(source_error.category(), ErrorCategory::Usage);
        assert_eq!(destination_error.category(), ErrorCategory::Usage);
        assert_eq!(topic_error.category(), ErrorCategory::Usage);
    }
}
