//! End-to-end CLI coverage for one-frame terminal rendering.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Arc,
    time::Duration,
};

use pcx_cli::{
    cli::try_run_render_from_with,
    terminal::{CapabilityQuery, DetectionContext, QueryResult},
};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/valid/pointcloud2.mcap")
}

fn render(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pcx"))
        .arg("render")
        .arg(fixture())
        .args(["--topic", "/lidar/points"])
        .args(arguments)
        .output()
        .expect("pcx should start")
}

#[derive(Default)]
struct FakeTerminal {
    environment: BTreeMap<String, OsString>,
}

impl FakeTerminal {
    fn interactive() -> Self {
        Self {
            environment: BTreeMap::from([("TERM".to_owned(), "xterm-256color".into())]),
        }
    }
}

impl DetectionContext for FakeTerminal {
    fn stdout_is_terminal(&self) -> bool {
        true
    }

    fn stdin_is_terminal(&self) -> bool {
        true
    }

    fn environment(&self, name: &str) -> Option<OsString> {
        self.environment.get(name).cloned()
    }
}

struct UnsupportedQuery;

impl CapabilityQuery for UnsupportedQuery {
    fn query(&self, _timeout: Duration) -> QueryResult {
        QueryResult::Unsupported
    }
}

#[test]
fn grammar_requires_exactly_one_selector_and_accepts_all_backends() {
    let path = fixture();
    let path = path.to_str().expect("fixture path should be UTF-8");
    for backend in ["auto", "unicode", "kitty", "sixel"] {
        let mut arguments = vec![
            "pcx",
            "render",
            path,
            "--topic",
            "/lidar/points",
            "--frame",
            "0",
            "--backend",
            backend,
        ];
        assert!(pcx_cli::cli::try_run_from(arguments.drain(..)).is_ok());
    }

    for arguments in [
        vec!["pcx", "render", "recording.mcap", "--topic", "/points"],
        vec![
            "pcx",
            "render",
            "recording.mcap",
            "--topic",
            "/points",
            "--frame",
            "0",
            "--at",
            "1s",
        ],
    ] {
        assert!(pcx_cli::cli::try_run_from(arguments).is_err());
    }
}

#[test]
fn redirected_auto_output_is_deterministic_plain_unicode() {
    let first = render(&[
        "--frame",
        "0",
        "--width",
        "8",
        "--height",
        "4",
        "--palette-limit",
        "0",
        "--payload-limit",
        "0",
    ]);
    let second = render(&["--at", "0ns", "--width", "8", "--height", "4"]);

    assert!(
        first.status.success(),
        "render failed: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(second.status.success());
    assert_eq!(first.stdout, second.stdout);
    assert!(first.stderr.is_empty());
    assert!(second.stderr.is_empty());
    assert!(!first.stdout.contains(&0x1b));
    let text = String::from_utf8(first.stdout).unwrap();
    assert!(text.chars().any(|glyph| matches!(glyph, '▀' | '▄' | '█')));
}

#[test]
fn explicit_control_backends_reject_redirected_stdout_before_output() {
    for backend in ["unicode", "kitty", "sixel"] {
        let output = render(&["--frame", "0", "--backend", backend]);
        assert_eq!(output.status.code(), Some(4));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("requires terminal stdout"));
    }
}

#[test]
fn explicit_sixel_dispatches_end_to_end_with_injected_tty_facts() {
    let path = fixture();
    let mut output = Vec::new();
    try_run_render_from_with(
        [
            "pcx",
            "render",
            path.to_str().expect("fixture path should be UTF-8"),
            "--topic",
            "/lidar/points",
            "--frame",
            "0",
            "--backend",
            "sixel",
            "--width",
            "8",
            "--height",
            "4",
        ],
        &mut output,
        &FakeTerminal::interactive(),
        Arc::new(UnsupportedQuery),
    )
    .expect("explicit Sixel should render");

    assert!(output.starts_with(b"\x1bP0;1;0q"));
    assert!(output.ends_with(b"\x1b\\"));
}

#[test]
fn resource_and_selection_errors_write_no_rendered_bytes() {
    let too_small = render(&["--frame", "0", "--memory-limit", "1"]);
    assert_eq!(too_small.status.code(), Some(6));
    assert!(too_small.stdout.is_empty());
    assert!(String::from_utf8_lossy(&too_small.stderr).contains("managed-memory peak"));

    let missing = render(&["--frame", "999", "--width", "8", "--height", "4"]);
    assert_eq!(missing.status.code(), Some(5));
    assert!(missing.stdout.is_empty());

    let zero_width = render(&["--frame", "0", "--width", "0"]);
    assert_eq!(zero_width.status.code(), Some(2));
    assert!(zero_width.stdout.is_empty());
}

#[test]
fn sixel_payload_limit_is_checked_before_escape_output() {
    let path = fixture();
    let mut output = Vec::new();
    let error = try_run_render_from_with(
        [
            "pcx",
            "render",
            path.to_str().expect("fixture path should be UTF-8"),
            "--topic",
            "/lidar/points",
            "--frame",
            "0",
            "--backend",
            "sixel",
            "--width",
            "8",
            "--height",
            "4",
            "--payload-limit",
            "1",
        ],
        &mut output,
        &FakeTerminal::interactive(),
        Arc::new(UnsupportedQuery),
    )
    .expect_err("tiny payload limit should fail");

    assert!(error.contains("payload requires"));
    assert!(output.is_empty());
}
