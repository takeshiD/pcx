//! Conservative terminal-backend selection and bounded raster encoders.
//!
//! Selection never renders or writes a terminal query. Encoders receive an
//! explicit output kind and stream deterministic bytes synchronously. A
//! rendering integration supplies a typed [`CapabilityQuery`] implementation;
//! the detector bounds that implementation and keeps process streams untouched.

mod unicode;

pub use unicode::{
    TerminalCellDimensions, UnicodeColorPolicy, UnicodeOutputKind, UnicodeRenderError,
    UnicodeRenderPlan,
};

use std::{
    ffi::{OsStr, OsString},
    io::{self, IsTerminal},
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

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
    /// Cell-based Unicode rendering.
    Unicode,
    /// Plain output without terminal control sequences.
    Plain,
}

/// The selected rendering backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Backend {
    Kitty,
    Unicode,
    Plain,
}

impl Backend {
    /// Whether this backend may emit terminal control sequences.
    pub const fn emits_control_sequences(self) -> bool {
        matches!(self, Self::Kitty | Self::Unicode)
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
        let error = select_backend(
            BackendChoice::Kitty,
            &context,
            FakeQuery::returning(QueryResult::Kitty),
        )
        .unwrap_err();
        assert_eq!(
            error,
            SelectionError::RedirectedControlBackend(Backend::Kitty)
        );
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
}
