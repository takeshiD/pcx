//! Command-line entrypoint and process-level behavior.

use std::{ffi::OsString, process::ExitCode};

use clap::{CommandFactory, Parser};

use crate::core::ErrorCategory;

/// Process statuses assigned to structured core failure categories.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ExitStatus {
    Success = 0,
    Internal = 1,
    Usage = 2,
    InvalidData = 3,
    Unsupported = 4,
    NotFound = 5,
    Resource = 6,
    Io = 7,
    Interrupted = 130,
}

impl From<ErrorCategory> for ExitStatus {
    fn from(category: ErrorCategory) -> Self {
        match category {
            ErrorCategory::Usage => Self::Usage,
            ErrorCategory::InvalidData => Self::InvalidData,
            ErrorCategory::Unsupported => Self::Unsupported,
            ErrorCategory::NotFound => Self::NotFound,
            ErrorCategory::Resource => Self::Resource,
            ErrorCategory::Io => Self::Io,
            ErrorCategory::Interrupted => Self::Interrupted,
            ErrorCategory::Internal => Self::Internal,
        }
    }
}

impl From<ExitStatus> for ExitCode {
    fn from(status: ExitStatus) -> Self {
        Self::from(status as u8)
    }
}

impl ExitStatus {
    pub const fn code(self) -> u8 {
        self as u8
    }
}

/// Inspect and reduce point-cloud recordings where the data lives.
#[derive(Debug, Parser)]
#[command(
    name = "pcx",
    version,
    about = "Inspect and reduce point-cloud recordings on edge Linux systems",
    long_about = "Inspect and reduce point-cloud recordings on edge Linux systems.\n\npcx is under active development. The current foundation release exposes only help and version information; point-cloud commands are planned for v0.1."
)]
pub struct Cli {}

/// Parse command-line arguments without terminating the process.
pub fn try_run_from<I, T>(args: I) -> Result<(), clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    Cli::try_parse_from(args).map(|_| ())
}

/// Run the process entrypoint using the environment arguments.
pub fn main() -> ExitCode {
    match try_run_from(std::env::args_os()) {
        Ok(()) => {
            let mut command = Cli::command();
            if command.print_help().is_err() {
                return ExitCode::FAILURE;
            }
            println!();
            ExitCode::SUCCESS
        }
        Err(error) => {
            let code = error.exit_code();
            let _ = error.print();
            ExitCode::from(u8::try_from(code).unwrap_or(1))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{ExitStatus, try_run_from};
    use crate::core::ErrorCategory;

    #[test]
    fn accepts_the_program_name_without_product_commands() {
        assert!(try_run_from(["pcx"]).is_ok());
    }

    #[test]
    fn rejects_unknown_arguments() {
        let error = try_run_from(["pcx", "--unknown"]).expect_err("argument must fail");
        assert_eq!(error.exit_code(), i32::from(ExitStatus::Usage.code()));
    }

    #[test]
    fn every_error_category_has_a_distinct_nonzero_exit_status() {
        let mappings = [
            (ErrorCategory::Usage, 2),
            (ErrorCategory::InvalidData, 3),
            (ErrorCategory::Unsupported, 4),
            (ErrorCategory::NotFound, 5),
            (ErrorCategory::Resource, 6),
            (ErrorCategory::Io, 7),
            (ErrorCategory::Interrupted, 130),
            (ErrorCategory::Internal, 1),
        ];
        let mut assigned = HashSet::new();

        for (category, expected_code) in mappings {
            let actual_code = ExitStatus::from(category).code();
            assert_eq!(actual_code, expected_code);
            assert_ne!(actual_code, ExitStatus::Success.code());
            assert!(assigned.insert(actual_code), "duplicate exit status");
        }
    }
}
