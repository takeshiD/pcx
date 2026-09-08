//! Conservative terminal-backend selection and bounded raster encoders.
//!
//! Selection never renders or writes a terminal query. Encoders receive an
//! explicit output kind and stream deterministic bytes synchronously. A
//! rendering integration supplies a typed [`CapabilityQuery`] implementation;
//! the detector bounds that implementation and keeps process streams untouched.

mod sixel;

pub use sixel::{
    DEFAULT_SIXEL_LIMITS, SIXEL_ENCODER_MEMORY_BYTES, SixelError, SixelLimits, SixelPlan,
};

mod unicode;

pub use unicode::{
    TerminalCellDimensions, UnicodeColorPolicy, UnicodeOutputKind, UnicodeRenderError,
    UnicodeRenderPlan,
};

use std::{
    ffi::{OsStr, OsString},
    io::{self, IsTerminal, Write},
    num::NonZeroU32,
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

use crate::{
    core::{ByteBound, Cancellation, ErrorCategory},
    ops::Raster,
    terminal::kitty::{KittyEncoder, KittyError, KittyLimits, KittyPlan, KittyWriteOutcome},
};

pub mod kitty;

/// Maximum time automatic detection waits for an injected terminal query.
pub const DETECTION_TIMEOUT: Duration = Duration::from_millis(100);

/// A rendering backend requested by the caller.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BackendChoice {
    /// Detect conservatively and use the documented fallback order.
    #[default]
    Auto,
    /// Kitty's terminal graphics protocol.
    Kitty,
    /// Sixel terminal graphics protocol.
    Sixel,
    /// Cell-based Unicode rendering.
    Unicode,
    /// Plain output without terminal control sequences.
    Plain,
}

/// The selected rendering backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Backend {
    Kitty,
    Sixel,
    Unicode,
    Plain,
}

/// Encoder settings shared by terminal-rendering callers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalRenderOptions {
    unicode_color: UnicodeColorPolicy,
    kitty_image_id: NonZeroU32,
    kitty_limits: KittyLimits,
    sixel_limits: SixelLimits,
}

impl TerminalRenderOptions {
    pub const fn new(
        unicode_color: UnicodeColorPolicy,
        kitty_image_id: NonZeroU32,
        kitty_limits: KittyLimits,
        sixel_limits: SixelLimits,
    ) -> Self {
        Self {
            unicode_color,
            kitty_image_id,
            kitty_limits,
            sixel_limits,
        }
    }

    /// Encoder-owned memory required by the selected backend before projection.
    pub fn encoder_memory_bound(self, selection: Selection) -> ByteBound {
        match selection.backend() {
            Backend::Kitty => {
                KittyEncoder::new(self.kitty_image_id, self.kitty_limits).memory_bound()
            }
            Backend::Sixel => SixelPlan::encoder_memory_bound(),
            Backend::Unicode | Backend::Plain => ByteBound::bounded(0),
        }
    }

    /// Validate the selected backend and all raster-dependent output bounds.
    pub fn plan<'a>(
        self,
        selection: Selection,
        output_kind: UnicodeOutputKind,
        raster: &'a Raster,
    ) -> Result<TerminalRenderPlan<'a>, TerminalRenderError> {
        if output_kind == UnicodeOutputKind::NonTty
            && matches!(selection.backend(), Backend::Kitty | Backend::Sixel)
        {
            return Err(TerminalRenderError::NonTtyGraphics(selection.backend()));
        }

        let planned = match selection.backend() {
            Backend::Kitty => {
                let encoder = KittyEncoder::new(self.kitty_image_id, self.kitty_limits);
                let plan = encoder.plan(raster).map_err(TerminalRenderError::Kitty)?;
                PlannedBackend::Kitty { encoder, plan }
            }
            Backend::Sixel => PlannedBackend::Sixel(Box::new(
                SixelPlan::new(raster, self.sixel_limits).map_err(TerminalRenderError::Sixel)?,
            )),
            Backend::Unicode | Backend::Plain => PlannedBackend::Unicode(UnicodeRenderPlan::new(
                raster.dimensions(),
                self.unicode_color,
            )),
        };
        Ok(TerminalRenderPlan {
            selection,
            output_kind: if selection.backend() == Backend::Plain {
                UnicodeOutputKind::NonTty
            } else {
                output_kind
            },
            raster,
            planned,
        })
    }
}

impl Default for TerminalRenderOptions {
    fn default() -> Self {
        Self::new(
            UnicodeColorPolicy::TrueColor,
            NonZeroU32::MIN,
            KittyLimits::default(),
            DEFAULT_SIXEL_LIMITS,
        )
    }
}

#[derive(Debug)]
enum PlannedBackend<'a> {
    Unicode(UnicodeRenderPlan),
    Kitty {
        encoder: KittyEncoder,
        plan: KittyPlan,
    },
    Sixel(Box<SixelPlan<'a>>),
}

/// A raster whose selected encoder has completed bounded preflight.
#[derive(Debug)]
pub struct TerminalRenderPlan<'a> {
    selection: Selection,
    output_kind: UnicodeOutputKind,
    raster: &'a Raster,
    planned: PlannedBackend<'a>,
}

impl TerminalRenderPlan<'_> {
    pub const fn backend(&self) -> Backend {
        self.selection.backend()
    }

    /// Conservative or exact complete-output bound, depending on the encoder.
    pub fn output_bytes_bound(&self) -> Result<u64, TerminalRenderError> {
        match &self.planned {
            PlannedBackend::Unicode(plan) => plan
                .encoded_size_bound(self.output_kind)
                .map_err(TerminalRenderError::Unicode),
            PlannedBackend::Kitty { plan, .. } => u64::try_from(plan.output_bytes())
                .map_err(|_| TerminalRenderError::OutputSizeOverflow),
            PlannedBackend::Sixel(plan) => Ok(plan.encoded_bytes()),
        }
    }

    /// Synchronously dispatch the common raster to the preflighted backend.
    pub fn write(
        &self,
        writer: &mut impl Write,
        cancellation: &Cancellation,
    ) -> Result<(), TerminalRenderError> {
        match &self.planned {
            PlannedBackend::Unicode(plan) => plan
                .render_cancellable(self.raster, self.output_kind, writer, cancellation)
                .map_err(TerminalRenderError::Unicode),
            PlannedBackend::Kitty { encoder, .. } => match encoder
                .write(self.selection, self.raster, cancellation, writer)
                .map_err(TerminalRenderError::Kitty)?
            {
                KittyWriteOutcome::Rendered { .. } => Ok(()),
                KittyWriteOutcome::Fallback(_) | KittyWriteOutcome::Deleted => {
                    Err(TerminalRenderError::DispatchInvariant)
                }
            },
            PlannedBackend::Sixel(plan) => plan
                .write_selected(writer, cancellation, self.selection)
                .map_err(TerminalRenderError::Sixel),
        }
    }
}

/// Typed preflight or output failure from terminal raster dispatch.
#[derive(Debug)]
pub enum TerminalRenderError {
    NonTtyGraphics(Backend),
    OutputSizeOverflow,
    DispatchInvariant,
    Unicode(UnicodeRenderError),
    Kitty(KittyError),
    Sixel(SixelError),
}

impl TerminalRenderError {
    pub const fn category(&self) -> ErrorCategory {
        match self {
            Self::NonTtyGraphics(_) => ErrorCategory::Unsupported,
            Self::OutputSizeOverflow => ErrorCategory::Resource,
            Self::DispatchInvariant => ErrorCategory::Internal,
            Self::Unicode(error) => error.category(),
            Self::Kitty(error) => error.category(),
            Self::Sixel(error) => error.category(),
        }
    }
}

impl std::fmt::Display for TerminalRenderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonTtyGraphics(backend) => {
                write!(formatter, "{backend:?} graphics require terminal output")
            }
            Self::OutputSizeOverflow => formatter.write_str("terminal output size overflowed"),
            Self::DispatchInvariant => {
                formatter.write_str("preflighted terminal backend was not dispatched")
            }
            Self::Unicode(error) => error.fmt(formatter),
            Self::Kitty(error) => error.fmt(formatter),
            Self::Sixel(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for TerminalRenderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unicode(error) => Some(error),
            Self::Kitty(error) => Some(error),
            Self::Sixel(error) => Some(error),
            Self::NonTtyGraphics(_) | Self::OutputSizeOverflow | Self::DispatchInvariant => None,
        }
    }
}

impl Backend {
    /// Whether this backend may emit terminal control sequences.
    pub const fn emits_control_sequences(self) -> bool {
        matches!(self, Self::Kitty | Self::Sixel | Self::Unicode)
    }
}

/// Why automatic selection chose a backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionReason {
    Explicit,
    RedirectedStdout,
    NonInteractiveInput,
    MissingTerm,
    DumbTerm,
    RemoteSession,
    MultiplexerSession,
    QueryConfirmed,
    QueryUnsupported,
    QueryFailed,
    QueryTimedOut,
}

/// A backend selection and the observable policy reason behind it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Selection {
    backend: Backend,
    reason: SelectionReason,
    query_attempted: bool,
}

impl Selection {
    pub const fn backend(self) -> Backend {
        self.backend
    }

    pub const fn reason(self) -> SelectionReason {
        self.reason
    }

    pub const fn query_attempted(self) -> bool {
        self.query_attempted
    }
}

/// A rejected explicit selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionError {
    /// A control-sequence backend cannot target redirected stdout.
    RedirectedControlBackend(Backend),
}

impl std::fmt::Display for SelectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RedirectedControlBackend(backend) => write!(
                formatter,
                "explicit {backend:?} backend requires terminal stdout"
            ),
        }
    }
}

impl std::error::Error for SelectionError {}

/// Read-only process facts used by the policy.
///
/// Implementations must return values as data only. They are never copied into
/// a query or escape sequence, so hostile environment values cannot inject
/// terminal controls.
pub trait DetectionContext {
    fn stdout_is_terminal(&self) -> bool;
    fn stdin_is_terminal(&self) -> bool;
    fn environment(&self, name: &str) -> Option<OsString>;
}

/// The real process environment and standard-stream TTY state.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessContext;

impl DetectionContext for ProcessContext {
    fn stdout_is_terminal(&self) -> bool {
        io::stdout().is_terminal()
    }

    fn stdin_is_terminal(&self) -> bool {
        io::stdin().is_terminal()
    }

    fn environment(&self, name: &str) -> Option<OsString> {
        std::env::var_os(name)
    }
}

/// Typed result from a protocol-specific capability query.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryResult {
    Kitty,
    Sixel,
    Unsupported,
    Failed,
}

/// Injectable query seam used only on eligible interactive sessions.
///
/// The query runs on a detached worker because an uncooperative terminal or
/// implementation must not hang selection. Implementations should still apply
/// their own I/O deadline and bound response bytes so timed-out workers finish.
pub trait CapabilityQuery: Send + Sync + 'static {
    fn query(&self, timeout: Duration) -> QueryResult;
}

/// Select a terminal backend without rendering or writing process streams.
pub fn select_backend<C, Q>(
    choice: BackendChoice,
    context: &C,
    query: Arc<Q>,
) -> Result<Selection, SelectionError>
where
    C: DetectionContext,
    Q: CapabilityQuery,
{
    if let Some(backend) = explicit_backend(choice) {
        if backend.emits_control_sequences() && !context.stdout_is_terminal() {
            return Err(SelectionError::RedirectedControlBackend(backend));
        }
        return Ok(selection(backend, SelectionReason::Explicit, false));
    }

    if !context.stdout_is_terminal() {
        return Ok(selection(
            Backend::Plain,
            SelectionReason::RedirectedStdout,
            false,
        ));
    }
    if !context.stdin_is_terminal() {
        return Ok(selection(
            Backend::Unicode,
            SelectionReason::NonInteractiveInput,
            false,
        ));
    }

    let Some(term) = context.environment("TERM") else {
        return Ok(selection(
            Backend::Plain,
            SelectionReason::MissingTerm,
            false,
        ));
    };
    if term.is_empty() {
        return Ok(selection(
            Backend::Plain,
            SelectionReason::MissingTerm,
            false,
        ));
    }
    if term == OsStr::new("dumb") {
        return Ok(selection(Backend::Plain, SelectionReason::DumbTerm, false));
    }
    if environment_present(context, "SSH_CONNECTION")
        || environment_present(context, "SSH_CLIENT")
        || environment_present(context, "SSH_TTY")
    {
        return Ok(selection(
            Backend::Unicode,
            SelectionReason::RemoteSession,
            false,
        ));
    }
    if environment_present(context, "TMUX") || term_starts_with(&term, b"tmux-") {
        return Ok(selection(
            Backend::Unicode,
            SelectionReason::MultiplexerSession,
            false,
        ));
    }

    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let _ = sender.send(query.query(DETECTION_TIMEOUT));
    });
    let (backend, reason) = match receiver.recv_timeout(DETECTION_TIMEOUT) {
        Ok(QueryResult::Kitty) => (Backend::Kitty, SelectionReason::QueryConfirmed),
        Ok(QueryResult::Sixel) => (Backend::Sixel, SelectionReason::QueryConfirmed),
        Ok(QueryResult::Unsupported) => (Backend::Unicode, SelectionReason::QueryUnsupported),
        Ok(QueryResult::Failed) | Err(mpsc::RecvTimeoutError::Disconnected) => {
            (Backend::Unicode, SelectionReason::QueryFailed)
        }
        Err(mpsc::RecvTimeoutError::Timeout) => (Backend::Unicode, SelectionReason::QueryTimedOut),
    };
    Ok(selection(backend, reason, true))
}

const fn explicit_backend(choice: BackendChoice) -> Option<Backend> {
    match choice {
        BackendChoice::Auto => None,
        BackendChoice::Kitty => Some(Backend::Kitty),
        BackendChoice::Sixel => Some(Backend::Sixel),
        BackendChoice::Unicode => Some(Backend::Unicode),
        BackendChoice::Plain => Some(Backend::Plain),
    }
}

const fn selection(backend: Backend, reason: SelectionReason, query_attempted: bool) -> Selection {
    Selection {
        backend,
        reason,
        query_attempted,
    }
}

fn environment_present(context: &impl DetectionContext, name: &str) -> bool {
    context
        .environment(name)
        .is_some_and(|value| !value.is_empty())
}

#[cfg(unix)]
fn term_starts_with(term: &OsStr, prefix: &[u8]) -> bool {
    use std::os::unix::ffi::OsStrExt;
    term.as_bytes().starts_with(prefix)
}

#[cfg(not(unix))]
fn term_starts_with(term: &OsStr, prefix: &[u8]) -> bool {
    term.to_string_lossy().as_bytes().starts_with(prefix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::BTreeMap,
        sync::atomic::{AtomicUsize, Ordering},
        time::Instant,
    };

    use crate::{
        core::{
            LossPolicy, PointRepresentation,
            point::{
                PointBatch, PointColumn, PointDimensions, PointField, PointFieldSemantic,
                PointFrameMetadata, PointSchema, PrimitiveType, Timestamp,
            },
        },
        ops::{
            ColorPolicy, DepthPolicy, InvalidProjectionCoordinatePolicy, OrthographicView,
            Projection, RasterDimensions, Rgb8,
        },
    };

    fn dispatch_raster() -> Raster {
        let schema = Arc::new(
            PointSchema::new(vec![
                PointField::new("x", PrimitiveType::F64, 1, Some(PointFieldSemantic::X)).unwrap(),
                PointField::new("y", PrimitiveType::F64, 1, Some(PointFieldSemantic::Y)).unwrap(),
                PointField::new("z", PrimitiveType::F64, 1, Some(PointFieldSemantic::Z)).unwrap(),
            ])
            .unwrap(),
        );
        let dimensions = PointDimensions::new(1, 1).unwrap();
        let batch = PointBatch::new(
            Arc::clone(&schema),
            Arc::new(PointFrameMetadata::new(
                Timestamp::new(0, 0).unwrap(),
                "map",
                false,
            )),
            dimensions,
            vec![
                PointColumn::F64(vec![0.0]),
                PointColumn::F64(vec![0.0]),
                PointColumn::F64(vec![0.0]),
            ],
        )
        .unwrap();
        Projection::new(
            RasterDimensions::new(1, 1).unwrap(),
            OrthographicView::xy(),
            DepthPolicy::Nearest,
            InvalidProjectionCoordinatePolicy::Reject,
            ColorPolicy::Uniform(Rgb8([1, 2, 3])),
        )
        .plan(
            schema,
            dimensions,
            PointRepresentation::Columns,
            &LossPolicy::lossless(),
        )
        .unwrap()
        .execute_batch(&batch)
        .unwrap()
    }

    #[derive(Default)]
    struct FakeContext {
        stdout_tty: bool,
        stdin_tty: bool,
        environment: BTreeMap<String, OsString>,
    }

    impl FakeContext {
        fn interactive() -> Self {
            Self {
                stdout_tty: true,
                stdin_tty: true,
                environment: BTreeMap::from([("TERM".to_owned(), "xterm-256color".into())]),
            }
        }

        fn with(mut self, name: &str, value: impl Into<OsString>) -> Self {
            self.environment.insert(name.to_owned(), value.into());
            self
        }
    }

    impl DetectionContext for FakeContext {
        fn stdout_is_terminal(&self) -> bool {
            self.stdout_tty
        }
        fn stdin_is_terminal(&self) -> bool {
            self.stdin_tty
        }
        fn environment(&self, name: &str) -> Option<OsString> {
            self.environment.get(name).cloned()
        }
    }

    struct FakeQuery {
        result: QueryResult,
        calls: AtomicUsize,
        delay: Duration,
    }

    impl FakeQuery {
        fn returning(result: QueryResult) -> Arc<Self> {
            Arc::new(Self {
                result,
                calls: AtomicUsize::new(0),
                delay: Duration::ZERO,
            })
        }

        fn delayed(delay: Duration) -> Arc<Self> {
            Arc::new(Self {
                result: QueryResult::Kitty,
                calls: AtomicUsize::new(0),
                delay,
            })
        }
    }

    impl CapabilityQuery for FakeQuery {
        fn query(&self, _timeout: Duration) -> QueryResult {
            self.calls.fetch_add(1, Ordering::SeqCst);
            thread::sleep(self.delay);
            self.result
        }
    }

    #[test]
    fn explicit_selection_wins_without_querying() {
        let context = FakeContext::interactive().with("SSH_CONNECTION", "hostile\x1b_Gpayload");
        let query = FakeQuery::returning(QueryResult::Kitty);

        let selected =
            select_backend(BackendChoice::Unicode, &context, Arc::clone(&query)).unwrap();

        assert_eq!(
            selected,
            selection(Backend::Unicode, SelectionReason::Explicit, false)
        );
        assert_eq!(query.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn redirected_stdout_is_never_queried() {
        let context = FakeContext {
            stdin_tty: true,
            ..FakeContext::interactive()
        };
        let context = FakeContext {
            stdout_tty: false,
            ..context
        };
        let query = FakeQuery::returning(QueryResult::Kitty);

        let selected = select_backend(BackendChoice::Auto, &context, Arc::clone(&query)).unwrap();

        assert_eq!(selected.backend(), Backend::Plain);
        assert_eq!(selected.reason(), SelectionReason::RedirectedStdout);
        assert!(!selected.query_attempted());
        assert_eq!(query.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn explicit_control_backend_rejects_redirected_stdout() {
        let context = FakeContext::default();
        for (choice, backend) in [
            (BackendChoice::Kitty, Backend::Kitty),
            (BackendChoice::Sixel, Backend::Sixel),
            (BackendChoice::Unicode, Backend::Unicode),
        ] {
            let error = select_backend(choice, &context, FakeQuery::returning(QueryResult::Kitty))
                .unwrap_err();
            assert_eq!(error, SelectionError::RedirectedControlBackend(backend));
        }
    }

    #[test]
    fn ssh_tmux_and_missing_term_are_conservative_without_queries() {
        let cases = [
            (
                FakeContext::interactive().with("SSH_TTY", "/dev/pts/1"),
                Backend::Unicode,
                SelectionReason::RemoteSession,
            ),
            (
                FakeContext::interactive().with("TMUX", "/tmp/tmux,1,0"),
                Backend::Unicode,
                SelectionReason::MultiplexerSession,
            ),
            (
                FakeContext {
                    stdout_tty: true,
                    stdin_tty: true,
                    ..FakeContext::default()
                },
                Backend::Plain,
                SelectionReason::MissingTerm,
            ),
        ];
        for (context, backend, reason) in cases {
            let query = FakeQuery::returning(QueryResult::Kitty);
            let selected =
                select_backend(BackendChoice::Auto, &context, Arc::clone(&query)).unwrap();
            assert_eq!((selected.backend(), selected.reason()), (backend, reason));
            assert_eq!(query.calls.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn automatic_fallback_order_is_deterministic() {
        for (result, backend, reason) in [
            (
                QueryResult::Kitty,
                Backend::Kitty,
                SelectionReason::QueryConfirmed,
            ),
            (
                QueryResult::Sixel,
                Backend::Sixel,
                SelectionReason::QueryConfirmed,
            ),
            (
                QueryResult::Unsupported,
                Backend::Unicode,
                SelectionReason::QueryUnsupported,
            ),
            (
                QueryResult::Failed,
                Backend::Unicode,
                SelectionReason::QueryFailed,
            ),
        ] {
            let selected = select_backend(
                BackendChoice::Auto,
                &FakeContext::interactive(),
                FakeQuery::returning(result),
            )
            .unwrap();
            assert_eq!((selected.backend(), selected.reason()), (backend, reason));
        }
    }

    #[test]
    fn query_wait_is_strictly_bounded() {
        let started = Instant::now();
        let selected = select_backend(
            BackendChoice::Auto,
            &FakeContext::interactive(),
            FakeQuery::delayed(Duration::from_secs(2)),
        )
        .unwrap();

        assert_eq!(selected.backend(), Backend::Unicode);
        assert_eq!(selected.reason(), SelectionReason::QueryTimedOut);
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn dispatcher_preflights_and_routes_every_selected_backend() {
        let raster = dispatch_raster();
        for (backend, output_kind, prefix) in [
            (
                Backend::Unicode,
                UnicodeOutputKind::Tty,
                b"\x1b[".as_slice(),
            ),
            (Backend::Kitty, UnicodeOutputKind::Tty, b"\x1b_G".as_slice()),
            (Backend::Sixel, UnicodeOutputKind::Tty, b"\x1bP".as_slice()),
            (Backend::Plain, UnicodeOutputKind::NonTty, "▀".as_bytes()),
        ] {
            let selected = selection(backend, SelectionReason::Explicit, false);
            let plan = TerminalRenderOptions::default()
                .plan(selected, output_kind, &raster)
                .unwrap();
            assert_eq!(plan.backend(), backend);
            let bound = plan.output_bytes_bound().unwrap();
            let mut output = Vec::new();
            plan.write(&mut output, &Cancellation::default()).unwrap();
            assert!(output.starts_with(prefix));
            assert!(u64::try_from(output.len()).unwrap() <= bound);
            if backend == Backend::Plain {
                assert!(!output.contains(&0x1b));
            }
        }
    }

    #[test]
    fn dispatcher_rejects_non_tty_graphics_before_output_and_reports_cancellation() {
        let raster = dispatch_raster();
        for backend in [Backend::Kitty, Backend::Sixel] {
            let error = TerminalRenderOptions::default()
                .plan(
                    selection(backend, SelectionReason::QueryConfirmed, true),
                    UnicodeOutputKind::NonTty,
                    &raster,
                )
                .unwrap_err();
            assert!(
                matches!(error, TerminalRenderError::NonTtyGraphics(found) if found == backend)
            );
            assert_eq!(error.category(), ErrorCategory::Unsupported);
        }

        let plan = TerminalRenderOptions::default()
            .plan(
                selection(Backend::Unicode, SelectionReason::Explicit, false),
                UnicodeOutputKind::Tty,
                &raster,
            )
            .unwrap();
        let cancellation = Cancellation::default();
        cancellation.cancel();
        let mut output = Vec::new();
        let error = plan.write(&mut output, &cancellation).unwrap_err();
        assert_eq!(error.category(), ErrorCategory::Interrupted);
        assert!(output.is_empty());
    }
}
