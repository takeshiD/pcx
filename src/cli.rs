//! Command-line grammar, presentation, and process-level behavior.

use std::{
    ffi::OsString,
    fs::File,
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
};

use clap::{Args, CommandFactory, Parser, Subcommand};

use crate::{
    core::{
        Error, ErrorCategory, ExecutionPlan, ExecutionReport, JobSpec, ManagedMemoryBound,
        SourceSpec,
    },
    mcap::{self, ProbeError, SourceOptions},
};

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
    long_about = "Inspect and reduce point-cloud recordings on edge Linux systems.\n\nThe current release can inspect MCAP container metadata with `pcx info`."
)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Show MCAP container metadata without decoding point frames.
    Info(InfoArgs),
}

#[derive(Debug, Args)]
struct InfoArgs {
    /// MCAP Source to inspect.
    #[arg(value_name = "INPUT.mcap")]
    input: PathBuf,

    /// Print versioned JSON instead of human-readable text.
    #[arg(long)]
    json: bool,
}

/// Parse command-line arguments without terminating the process.
pub fn try_run_from<I, T>(args: I) -> Result<(), clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    Cli::try_parse_from(args).map(|_| ())
}

#[derive(Debug)]
struct RunFailure {
    category: ErrorCategory,
    message: String,
    broken_pipe: bool,
}

impl RunFailure {
    fn core(error: Error) -> Self {
        Self {
            category: error.category(),
            message: error.to_string(),
            broken_pipe: false,
        }
    }

    fn output(error: io::Error) -> Self {
        Self {
            category: ErrorCategory::Io,
            message: format!("failed to write output: {error}"),
            broken_pipe: error.kind() == io::ErrorKind::BrokenPipe,
        }
    }
}

fn run(cli: Cli, output: &mut impl Write) -> Result<(), RunFailure> {
    match cli.command {
        Some(Command::Info(args)) => run_info(args, output),
        None => {
            Cli::command()
                .write_help(output)
                .map_err(RunFailure::output)?;
            writeln!(output).map_err(RunFailure::output)
        }
    }
}

fn run_info(args: InfoArgs, output: &mut impl Write) -> Result<(), RunFailure> {
    let source = SourceSpec::file(args.input.clone()).map_err(RunFailure::core)?;
    let options = SourceOptions::default();
    let peak_bytes = u64::try_from(options.read_chunk_bytes)
        .ok()
        .and_then(|read| {
            u64::try_from(options.max_record_bytes)
                .ok()
                .and_then(|record| read.checked_add(record))
        })
        .ok_or_else(|| RunFailure {
            category: ErrorCategory::Internal,
            message: "MCAP memory bound overflowed".to_owned(),
            broken_pipe: false,
        })?;
    let memory = ManagedMemoryBound::checked(peak_bytes, peak_bytes).map_err(RunFailure::core)?;
    let plan = ExecutionPlan::checked(JobSpec::info(source), memory);

    let file = File::open(&args.input).map_err(|source| RunFailure {
        category: ErrorCategory::Io,
        message: format!("failed to open Source '{}': {source}", args.input.display()),
        broken_pipe: false,
    })?;
    let info = mcap::inspect(file, options).map_err(probe_failure)?;
    let report = ExecutionReport::success(&plan, info);

    if args.json {
        serde_json::to_writer_pretty(&mut *output, &report).map_err(|error| RunFailure {
            category: if error.is_io() {
                ErrorCategory::Io
            } else {
                ErrorCategory::Internal
            },
            message: format!("failed to write JSON output: {error}"),
            broken_pipe: error.io_error_kind() == Some(io::ErrorKind::BrokenPipe),
        })?;
        writeln!(output).map_err(RunFailure::output)
    } else {
        writeln!(output, "{}", report.data()).map_err(RunFailure::output)
    }
}

fn probe_failure(error: ProbeError) -> RunFailure {
    let category = match error {
        ProbeError::Io { .. } => ErrorCategory::Io,
        ProbeError::Reader { .. } | ProbeError::Parse { .. } => ErrorCategory::InvalidData,
        ProbeError::InvalidOptions(_) => ErrorCategory::Internal,
    };
    RunFailure {
        category,
        message: error.to_string(),
        broken_pipe: false,
    }
}

/// Run the process entrypoint using the environment arguments.
pub fn main() -> ExitCode {
    let cli = match Cli::try_parse_from(std::env::args_os()) {
        Ok(cli) => cli,
        Err(error) => {
            let code = error.exit_code();
            let _ = error.print();
            return ExitCode::from(u8::try_from(code).unwrap_or(1));
        }
    };

    let stdout = io::stdout();
    let mut output = stdout.lock();
    match run(cli, &mut output) {
        Ok(()) => ExitStatus::Success.into(),
        Err(error) if error.broken_pipe => ExitStatus::Success.into(),
        Err(error) => {
            eprintln!("pcx: error: {}", error.message);
            ExitStatus::from(error.category).into()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashSet,
        io::{self, Write},
    };

    use clap::Parser;

    use super::{Cli, ExitStatus, run, try_run_from};
    use crate::core::ErrorCategory;

    #[test]
    fn accepts_the_program_name_without_product_commands() {
        assert!(try_run_from(["pcx"]).is_ok());
    }

    #[test]
    fn accepts_info_human_and_json_forms() {
        assert!(try_run_from(["pcx", "info", "recording.mcap"]).is_ok());
        assert!(try_run_from(["pcx", "info", "recording.mcap", "--json"]).is_ok());
    }

    #[test]
    fn rejects_unknown_arguments() {
        let error = try_run_from(["pcx", "--unknown"]).expect_err("argument must fail");
        assert_eq!(error.exit_code(), i32::from(ExitStatus::Usage.code()));
    }

    struct BrokenPipe;

    impl Write for BrokenPipe {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "downstream closed",
            ))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn broken_output_pipe_is_recognized_as_normal_pipeline_termination() {
        let cli = Cli::try_parse_from(["pcx"]).expect("arguments should parse");
        let error = run(cli, &mut BrokenPipe).expect_err("writer should reject output");

        assert!(error.broken_pipe);
        assert_eq!(error.category, ErrorCategory::Io);
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
