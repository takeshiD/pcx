//! Command-line grammar, presentation, and process-level behavior.

use std::{
    ffi::OsString,
    fs::File,
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
    time::Duration,
};

use clap::{ArgGroup, Args, CommandFactory, Parser, Subcommand, ValueEnum};

use crate::{
    core::{
        ByteBound, Cancellation, Destination, Error, ErrorCategory, ErrorReport, ExecutionPlan,
        ExecutionReport, FrameSelector, JobKind, JobSpec, PipelineMemoryRequirements, Planner,
        Result as CoreResult, SourceSpec, write_output,
    },
    mcap::{
        self, DiscoveredChannel, PassthroughCompression, PassthroughError, ProbeError,
        SelectionError, Source, SourceOptions, TopicDiscovery,
    },
    pcd::{self, Encoding},
    ros2,
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
    long_about = "Inspect and reduce point-cloud recordings on edge Linux systems.\n\nInspect MCAP metadata and Topics, extract one ROS 2 PointCloud2 frame to PCD, or copy one selected encoded message into a reduced MCAP."
)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

/// Build the implemented command grammar for generated help assets.
///
/// Shell completions and manual pages call this same constructor, so they
/// cannot advertise commands or options that are absent from the parser.
pub fn command() -> clap::Command {
    Cli::command()
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Show MCAP container metadata without decoding point frames.
    Info(InfoArgs),
    /// Discover Topics, MCAP Channels, Schemas, and message counts.
    Topics(TopicsArgs),
    /// Extract exactly one ROS 2 PointCloud2 Point Frame as PCD.
    Extract(ExtractArgs),
    /// Copy one selected encoded message into a faithful reduced MCAP.
    Passthrough(PassthroughArgs),
}

impl Cli {
    fn machine_command(&self) -> Option<JobKind> {
        match &self.command {
            Some(Command::Info(args)) if args.json => Some(JobKind::Info),
            Some(Command::Topics(args)) if args.json => Some(JobKind::Topics),
            _ => None,
        }
    }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum PcdEncoding {
    Binary,
    Ascii,
}

impl From<PcdEncoding> for Encoding {
    fn from(value: PcdEncoding) -> Self {
        match value {
            PcdEncoding::Binary => Self::Binary,
            PcdEncoding::Ascii => Self::Ascii,
        }
    }
}

#[derive(Debug, Args)]
#[command(group(
    ArgGroup::new("selector")
        .required(true)
        .multiple(false)
        .args(["frame", "at"])
))]
struct ExtractArgs {
    /// MCAP Source containing the Point Frame.
    #[arg(value_name = "INPUT.mcap")]
    input: PathBuf,

    /// Topic whose messages are counted as Point Frames.
    #[arg(long, value_name = "TOPIC")]
    topic: String,

    /// Zero-based Point Frame index after Topic selection.
    #[arg(long, value_name = "INDEX")]
    frame: Option<u64>,

    /// First Point Frame at or after this duration from recording start.
    #[arg(long, value_name = "DURATION", value_parser = parse_duration)]
    at: Option<Duration>,

    /// Output PCD path, or '-' for binary-safe stdout.
    #[arg(short, long, value_name = "PATH|-", required = true)]
    output: PathBuf,

    /// Replace an existing output file.
    #[arg(long)]
    force: bool,

    /// PCD payload representation.
    #[arg(long, value_enum, default_value = "binary")]
    encoding: PcdEncoding,

    /// Hard managed-memory limit in bytes.
    #[arg(long, value_name = "BYTES", default_value_t = 512 * 1024 * 1024_u64)]
    memory_limit: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum McapCompression {
    None,
    Zstd,
    Lz4,
}

impl From<McapCompression> for PassthroughCompression {
    fn from(value: McapCompression) -> Self {
        match value {
            McapCompression::None => Self::None,
            McapCompression::Zstd => Self::Zstd,
            McapCompression::Lz4 => Self::Lz4,
        }
    }
}

#[derive(Debug, Args)]
#[command(group(
    ArgGroup::new("selector")
        .required(true)
        .multiple(false)
        .args(["frame", "at"])
))]
struct PassthroughArgs {
    /// MCAP Source containing the encoded message.
    #[arg(value_name = "INPUT.mcap")]
    input: PathBuf,

    /// Topic whose encoded messages are selected.
    #[arg(long, value_name = "TOPIC")]
    topic: String,

    /// Zero-based message index after Topic selection.
    #[arg(long, value_name = "INDEX")]
    frame: Option<u64>,

    /// First message at or after this duration from recording start.
    #[arg(long, value_name = "DURATION", value_parser = parse_duration)]
    at: Option<Duration>,

    /// Output MCAP path, or '-' for binary-safe stdout.
    #[arg(short, long, value_name = "PATH|-", required = true)]
    output: PathBuf,

    /// Replace an existing output file.
    #[arg(long)]
    force: bool,

    /// Deterministic output chunk compression.
    #[arg(long, value_enum, default_value = "zstd")]
    compression: McapCompression,

    /// Hard managed-memory limit in bytes.
    #[arg(long, value_name = "BYTES", default_value_t = 512 * 1024 * 1024_u64)]
    memory_limit: u64,
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
        Some(Command::Extract(args)) => run_extract(args),
        Some(Command::Passthrough(args)) => run_passthrough(args),
        None => {
            command().write_help(output).map_err(RunFailure::output)?;
            writeln!(output).map_err(RunFailure::output)
        }
    }
}

fn run_passthrough(args: PassthroughArgs) -> Result<(), RunFailure> {
    let source_spec = SourceSpec::file(args.input.clone()).map_err(RunFailure::core)?;
    let destination = if args.output.as_os_str() == "-" {
        if args.force {
            return Err(RunFailure::core(Error::new(
                ErrorCategory::Usage,
                "--force is only valid for file output",
            )));
        }
        Destination::stdout()
    } else {
        Destination::file(args.output, args.force).map_err(RunFailure::core)?
    };
    let selector = match (args.frame, args.at) {
        (Some(index), None) => FrameSelector::Index(index),
        (None, Some(duration)) => FrameSelector::At(duration),
        _ => unreachable!("clap requires exactly one frame selector"),
    };
    let job = JobSpec::passthrough(source_spec, args.topic, selector, destination.clone())
        .map_err(RunFailure::core)?;
    let options = SourceOptions::default();
    plan_passthrough_job(job.clone(), options, args.memory_limit)?;

    let file = File::open(job.source().path()).map_err(|source| RunFailure {
        category: ErrorCategory::Io,
        message: format!(
            "failed to open Source '{}': {source}",
            job.source().path().display()
        ),
        broken_pipe: false,
    })?;
    let mut source = Source::new(file, options).map_err(probe_failure)?;
    let (topic, selector, _) = job.passthrough_selection().expect("passthrough selection");
    let passthrough =
        mcap::plan_passthrough(&mut source, topic, selector).map_err(passthrough_failure)?;

    let handler = InterruptHandler::install().map_err(|error| RunFailure {
        category: ErrorCategory::Internal,
        message: format!("could not install interrupt handler: {error}"),
        broken_pipe: false,
    })?;
    write_output(&destination, &handler.cancellation, |writer| {
        mcap::write_passthrough(&mut source, writer, &passthrough, args.compression.into())
    })
    .map_err(RunFailure::core)?;
    Ok(())
}

fn run_extract(args: ExtractArgs) -> Result<(), RunFailure> {
    let source_spec = SourceSpec::file(args.input.clone()).map_err(RunFailure::core)?;
    let destination = if args.output.as_os_str() == "-" {
        if args.force {
            return Err(RunFailure::core(Error::new(
                ErrorCategory::Usage,
                "--force is only valid for file output",
            )));
        }
        Destination::stdout()
    } else {
        Destination::file(args.output, args.force).map_err(RunFailure::core)?
    };
    let selector = match (args.frame, args.at) {
        (Some(index), None) => FrameSelector::Index(index),
        (None, Some(duration)) => FrameSelector::At(duration),
        _ => unreachable!("clap requires exactly one frame selector"),
    };
    let job = JobSpec::extract(source_spec, args.topic, selector, destination.clone())
        .map_err(RunFailure::core)?;
    let options = SourceOptions::default();

    // Prove the bounded MCAP read stage before opening the Source. A second,
    // tighter plan below accounts for the selected decoded view.
    plan_selection_job(job.clone(), options, args.memory_limit)?;

    let file = File::open(job.source().path()).map_err(|source| RunFailure {
        category: ErrorCategory::Io,
        message: format!(
            "failed to open Source '{}': {source}",
            job.source().path().display()
        ),
        broken_pipe: false,
    })?;
    let mut source = Source::new(file, options).map_err(probe_failure)?;
    let selected = mcap::select_topic_message(&mut source, job.extraction().unwrap().0, selector)
        .map_err(selection_failure)?;
    drop(source);

    if !selected.is_ros2_pointcloud2_candidate() {
        return Err(RunFailure::core(Error::new(
            ErrorCategory::Unsupported,
            format!(
                "selected MCAP Channel {} is not declared as ROS 2 PointCloud2 with ros2msg/CDR encoding",
                selected.channel_id()
            ),
        )));
    }

    let log_time = selected.log_time();
    let publish_time = selected.publish_time();
    let view = ros2::pointcloud2::decode(selected.into_data())
        .map_err(|error| RunFailure {
            category: ErrorCategory::InvalidData,
            message: error.to_string(),
            broken_pipe: false,
        })?
        .with_container_times(log_time, publish_time);
    pcd::validate(&view).map_err(pcd_failure)?;
    let requirements = PipelineMemoryRequirements::for_point_view(
        &view,
        false,
        ByteBound::bounded(0),
        ByteBound::bounded(0),
        ByteBound::bounded(0),
        ByteBound::bounded(0),
    )
    .map_err(RunFailure::core)?;
    Planner::new()
        .plan(job, requirements, args.memory_limit)
        .map_err(RunFailure::core)?;

    let handler = InterruptHandler::install().map_err(|error| RunFailure {
        category: ErrorCategory::Internal,
        message: format!("could not install interrupt handler: {error}"),
        broken_pipe: false,
    })?;
    write_output(&destination, &handler.cancellation, |writer| {
        pcd::write(&mut &mut *writer, &view, args.encoding.into()).map_err(io::Error::other)
    })
    .map_err(RunFailure::core)?;
    Ok(())
}

fn run_info(args: InfoArgs, output: &mut impl Write) -> Result<(), RunFailure> {
    let source = SourceSpec::file(args.input.clone()).map_err(RunFailure::core)?;
    let options = SourceOptions::default();
    let plan = plan_container_job(JobSpec::info(source), options, u64::MAX)?;

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

fn plan_container_job(
    job: JobSpec,
    options: SourceOptions,
    limit_bytes: u64,
) -> Result<ExecutionPlan, RunFailure> {
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
        .plan(job, requirements, limit_bytes)
        .map_err(RunFailure::core)
}

fn plan_selection_job(
    job: JobSpec,
    options: SourceOptions,
    limit_bytes: u64,
) -> Result<ExecutionPlan, RunFailure> {
    let retained_input_bytes = options.selection_retained_bytes().ok_or_else(|| {
        RunFailure::core(Error::new(
            ErrorCategory::Resource,
            "MCAP selection retained-input bound overflowed",
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
        .plan(job, requirements, limit_bytes)
        .map_err(RunFailure::core)
}

fn plan_passthrough_job(
    job: JobSpec,
    options: SourceOptions,
    limit_bytes: u64,
) -> Result<ExecutionPlan, RunFailure> {
    let retained_input_bytes = options.passthrough_retained_bytes().ok_or_else(|| {
        RunFailure::core(Error::new(
            ErrorCategory::Resource,
            "MCAP passthrough retained-input bound overflowed",
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
        .plan(job, requirements, limit_bytes)
        .map_err(RunFailure::core)
}

fn probe_failure(error: ProbeError) -> RunFailure {
    RunFailure {
        category: error.category(),
        message: error.to_string(),
        broken_pipe: false,
    }
}

fn selection_failure(error: SelectionError) -> RunFailure {
    RunFailure {
        category: error.category(),
        message: error.to_string(),
        broken_pipe: false,
    }
}

fn passthrough_failure(error: PassthroughError) -> RunFailure {
    RunFailure {
        category: error.category(),
        message: error.to_string(),
        broken_pipe: false,
    }
}

fn pcd_failure(error: pcd::Error) -> RunFailure {
    RunFailure {
        category: match error {
            pcd::Error::Io(_) => ErrorCategory::Io,
            pcd::Error::Access(_) => ErrorCategory::InvalidData,
            _ => ErrorCategory::Unsupported,
        },
        message: error.to_string(),
        broken_pipe: false,
    }
}

fn parse_duration(value: &str) -> Result<Duration, String> {
    let (number, scale, fractional_digits) = if let Some(number) = value.strip_suffix("ms") {
        (number, 1_000_000_u64, 6_usize)
    } else if let Some(number) = value.strip_suffix("us") {
        (number, 1_000_u64, 3_usize)
    } else if let Some(number) = value.strip_suffix("ns") {
        (number, 1_u64, 0_usize)
    } else if let Some(number) = value.strip_suffix('s') {
        (number, 1_000_000_000_u64, 9_usize)
    } else {
        return Err("duration must end in s, ms, us, or ns".to_owned());
    };
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > fractional_digits
    {
        return Err(format!("invalid duration {value:?}"));
    }
    let whole = whole
        .parse::<u64>()
        .map_err(|_| format!("duration {value:?} is too large"))?;
    let whole_nanos = whole
        .checked_mul(scale)
        .ok_or_else(|| format!("duration {value:?} is too large"))?;
    let fraction_nanos = if fraction.is_empty() {
        0
    } else {
        let parsed = fraction
            .parse::<u64>()
            .map_err(|_| format!("invalid duration {value:?}"))?;
        let padding = u32::try_from(fractional_digits - fraction.len()).unwrap();
        parsed
            .checked_mul(10_u64.pow(padding))
            .ok_or_else(|| format!("duration {value:?} is too large"))?
    };
    whole_nanos
        .checked_add(fraction_nanos)
        .map(Duration::from_nanos)
        .ok_or_else(|| format!("duration {value:?} is too large"))
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

    let machine_command = cli.machine_command();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    match run(cli, &mut output) {
        Ok(()) => ExitStatus::Success.into(),
        Err(error) if error.broken_pipe => ExitStatus::Success.into(),
        Err(error) => {
            if let Some(command) = machine_command {
                let report = ErrorReport::new(command, error.category, &error.message);
                let stderr = io::stderr();
                let mut diagnostic = stderr.lock();
                if serde_json::to_writer_pretty(&mut diagnostic, &report).is_ok() {
                    let _ = writeln!(diagnostic);
                } else {
                    eprintln!("pcx: error: {}", error.message);
                }
            } else {
                eprintln!("pcx: error: {}", error.message);
            }
            ExitStatus::from(error.category).into()
        }
    }
}

fn run_topics(args: TopicsArgs, output: &mut impl Write) -> Result<(), RunFailure> {
    let source = SourceSpec::file(args.input.clone()).map_err(RunFailure::core)?;
    let options = SourceOptions::default();
    let plan = plan_container_job(JobSpec::topics(source), options, u64::MAX)?;

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
    fn accepts_passthrough_with_exactly_one_selector() {
        assert!(
            try_run_from([
                "pcx",
                "passthrough",
                "recording.mcap",
                "--topic",
                "/points",
                "--frame",
                "0",
                "--output",
                "selected.mcap",
                "--compression",
                "lz4",
            ])
            .is_ok()
        );
        assert!(
            try_run_from([
                "pcx",
                "passthrough",
                "recording.mcap",
                "--topic",
                "/points",
                "--at",
                "10ms",
                "--output",
                "-",
            ])
            .is_ok()
        );
        assert!(
            try_run_from([
                "pcx",
                "passthrough",
                "recording.mcap",
                "--topic",
                "/points",
                "--frame",
                "0",
                "--at",
                "10ms",
                "--output",
                "-",
            ])
            .is_err()
        );
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
