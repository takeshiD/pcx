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
        ByteBound, Cancellation, Error, ErrorCategory, ExecutionPlan, ExecutionReport, JobSpec,
        PipelineMemoryRequirements, Planner, Result as CoreResult, SourceSpec,
    },
    mcap::{self, DiscoveredChannel, ProbeError, SourceOptions, TopicDiscovery},
};

struct InterruptHandler {
    cancellation: Cancellation,
    signal_id: signal_hook::SigId,
}

impl InterruptHandler {
    fn install() -> std::io::Result<Self> {
        let cancellation = Cancellation::default();
        let signal_id =
            signal_hook::flag::register(signal_hook::consts::SIGINT, cancellation.signal_flag())?;
        Ok(Self {
            cancellation,
            signal_id,
        })
    }
}

impl Drop for InterruptHandler {
    fn drop(&mut self) {
        signal_hook::low_level::unregister(self.signal_id);
    }
}

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

/// Run synchronous product work with SIGINT mapped to cancellation and exit 130.
///
/// Product commands keep their format-specific work outside this process seam;
/// this function only owns signal setup, typed exit mapping, and stderr diagnostics.
pub fn run_interruptibly<F>(operation: F) -> ExitStatus
where
    F: FnOnce(Cancellation) -> CoreResult<()>,
{
    let handler = match InterruptHandler::install() {
        Ok(handler) => handler,
        Err(error) => {
            eprintln!("pcx: could not install interrupt handler: {error}");
            return ExitStatus::Internal;
        }
    };

    match operation(handler.cancellation.clone()) {
        Ok(()) => ExitStatus::Success,
        Err(error) => report_error(&error),
    }
}

fn report_error(error: &Error) -> ExitStatus {
    eprintln!("pcx: {error}");
    ExitStatus::from(error.category())
}

/// Inspect and reduce point-cloud recordings where the data lives.
#[derive(Debug, Parser)]
#[command(
    name = "pcx",
    version,
    about = "Inspect and reduce point-cloud recordings on edge Linux systems",
    long_about = "Inspect and reduce point-cloud recordings on edge Linux systems.\n\nThe current release can inspect MCAP container metadata with `pcx info` and discover Topics, Channels, Schemas, and message counts with `pcx topics`."
)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Show MCAP container metadata without decoding point frames.
    Info(InfoArgs),
    /// Discover Topics, MCAP Channels, Schemas, and message counts.
    Topics(TopicsArgs),
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

#[derive(Debug, Args)]
struct TopicsArgs {
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
        Some(Command::Topics(args)) => run_topics(args, output),
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
    let plan = plan_container_job(JobSpec::info(source), options)?;

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

fn plan_container_job(job: JobSpec, options: SourceOptions) -> Result<ExecutionPlan, RunFailure> {
    let retained_input_bytes = u64::try_from(options.read_chunk_bytes)
        .ok()
        .and_then(|read| {
            u64::try_from(options.max_record_bytes)
                .ok()
                .and_then(|record| read.checked_add(record))
        })
        .ok_or_else(|| {
            RunFailure::core(Error::new(
                ErrorCategory::Resource,
                "MCAP retained-input bound overflowed",
            ))
        })?;
    let requirements = PipelineMemoryRequirements::new(
        ByteBound::bounded(retained_input_bytes),
        ByteBound::bounded(0),
        ByteBound::bounded(0),
        ByteBound::bounded(0),
        ByteBound::bounded(0),
        ByteBound::bounded(0),
    );
    Planner::new()
        .plan(job, requirements, u64::MAX)
        .map_err(RunFailure::core)
}

fn probe_failure(error: ProbeError) -> RunFailure {
    RunFailure {
        category: error.category(),
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

fn run_topics(args: TopicsArgs, output: &mut impl Write) -> Result<(), RunFailure> {
    let source = SourceSpec::file(args.input.clone()).map_err(RunFailure::core)?;
    let options = SourceOptions::default();
    let plan = plan_container_job(JobSpec::topics(source), options)?;

    let file = File::open(&args.input).map_err(|source| RunFailure {
        category: ErrorCategory::Io,
        message: format!("failed to open Source '{}': {source}", args.input.display()),
        broken_pipe: false,
    })?;
    let discovery = mcap::discover_topics(file, options).map_err(probe_failure)?;
    let report = ExecutionReport::success(&plan, discovery);

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
        render_topics_human(report.data(), output).map_err(RunFailure::output)
    }
}

fn render_topics_human(discovery: &TopicDiscovery, output: &mut impl Write) -> io::Result<()> {
    if discovery.channels().is_empty() {
        return writeln!(output, "No Topics found.");
    }
    for (index, channel) in discovery.channels().iter().enumerate() {
        if index > 0 {
            writeln!(output)?;
        }
        render_channel(channel, output)?;
    }
    writeln!(
        output,
        "\nPointCloud2 candidate status is based on declared Schema and encoding metadata; message payloads were not decoded."
    )
}

fn render_channel(channel: &DiscoveredChannel, output: &mut impl Write) -> io::Result<()> {
    writeln!(output, "Topic: {}", channel.topic)?;
    writeln!(output, "  MCAP Channel ID: {}", channel.channel_id)?;
    writeln!(output, "  Messages: {}", channel.message_count)?;
    writeln!(output, "  Message encoding: {}", channel.message_encoding)?;
    match &channel.schema {
        Some(schema) => {
            writeln!(output, "  MCAP Schema ID: {}", schema.id)?;
            writeln!(output, "  Schema: {}", schema.name)?;
            writeln!(output, "  Schema encoding: {}", schema.encoding)?;
        }
        None => {
            writeln!(output, "  MCAP Schema: none")?;
        }
    }
    writeln!(
        output,
        "  ROS 2 PointCloud2 candidate: {}",
        if channel.ros2_pointcloud2_candidate {
            "yes"
        } else {
            "no"
        }
    )
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
    fn accepts_topics_human_and_json_forms() {
        assert!(try_run_from(["pcx", "topics", "recording.mcap"]).is_ok());
        assert!(try_run_from(["pcx", "topics", "recording.mcap", "--json"]).is_ok());
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
