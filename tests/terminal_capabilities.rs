use std::{
    collections::BTreeMap,
    ffi::OsString,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use pcx_cli::terminal::{
    Backend, BackendChoice, CapabilityQuery, DetectionContext, QueryResult, SelectionReason,
    select_backend,
};

struct Session {
    stdout_tty: bool,
    stdin_tty: bool,
    environment: BTreeMap<String, OsString>,
}

impl DetectionContext for Session {
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

struct RecordingQuery(AtomicUsize);

impl CapabilityQuery for RecordingQuery {
    fn query(&self, timeout: Duration) -> QueryResult {
        assert_eq!(timeout, pcx_cli::terminal::DETECTION_TIMEOUT);
        self.0.fetch_add(1, Ordering::SeqCst);
        QueryResult::Kitty
    }
}

#[test]
fn redirected_ssh_and_tmux_sessions_do_not_reach_the_query_boundary() {
    let sessions = [
        (
            false,
            true,
            [("TERM", "xterm-kitty")].as_slice(),
            Backend::Plain,
            SelectionReason::RedirectedStdout,
        ),
        (
            true,
            true,
            [("TERM", "xterm-kitty"), ("SSH_CONNECTION", "a b c d")].as_slice(),
            Backend::Unicode,
            SelectionReason::RemoteSession,
        ),
        (
            true,
            true,
            [("TERM", "screen-256color"), ("TMUX", "/tmp/tmux")].as_slice(),
            Backend::Unicode,
            SelectionReason::MultiplexerSession,
        ),
    ];

    for (stdout_tty, stdin_tty, values, expected, reason) in sessions {
        let query = Arc::new(RecordingQuery(AtomicUsize::new(0)));
        let environment = values
            .iter()
            .map(|(name, value)| ((*name).to_owned(), OsString::from(value)))
            .collect();
        let selected = select_backend(
            BackendChoice::Auto,
            &Session {
                stdout_tty,
                stdin_tty,
                environment,
            },
            Arc::clone(&query),
        )
        .unwrap();
        assert_eq!((selected.backend(), selected.reason()), (expected, reason));
        assert_eq!(query.0.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn redirected_process_output_contains_no_terminal_control_sequences() {
    let output = Command::new(env!("CARGO_BIN_EXE_pcx"))
        .arg("--help")
        .env("TERM", "xterm-kitty\x1b_Ginjected")
        .env("KITTY_WINDOW_ID", "1\x1b_Ginjected")
        .output()
        .expect("pcx should start");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(!output.stdout.contains(&0x1b));
}
