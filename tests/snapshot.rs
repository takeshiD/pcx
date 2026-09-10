//! End-to-end CLI coverage for one-frame PNG snapshots.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new(name: &str) -> Self {
        let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "pcx-snapshot-{name}-{}-{}",
            std::process::id(),
            NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("temporary directory should be created");
        Self(path)
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/valid/pointcloud2.mcap")
}

fn snapshot(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pcx"))
        .arg("snapshot")
        .arg(fixture())
        .args(["--topic", "/lidar/points", "--frame", "0"])
        .args(arguments)
        .output()
        .expect("pcx should start")
}

fn assert_no_temporary_output(directory: &TempDirectory) {
    assert!(
        fs::read_dir(&directory.0)
            .expect("temporary directory should be readable")
            .filter_map(Result::ok)
            .all(|entry| !entry.file_name().to_string_lossy().contains(".pcx.tmp.")),
        "temporary snapshot output remained"
    );
}

#[test]
fn grammar_requires_one_selector_and_an_explicit_output() {
    assert!(
        pcx_cli::cli::try_run_from([
            "pcx",
            "snapshot",
            "recording.mcap",
            "--topic",
            "/points",
            "--frame",
            "0",
            "--output",
            "frame.png",
        ])
        .is_ok()
    );
    assert!(
        pcx_cli::cli::try_run_from([
            "pcx",
            "snapshot",
            "recording.mcap",
            "--topic",
            "/points",
            "--output",
            "frame.png",
        ])
        .is_err()
    );
    assert!(
        pcx_cli::cli::try_run_from([
            "pcx",
            "snapshot",
            "recording.mcap",
            "--topic",
            "/points",
            "--frame",
            "0",
            "--at",
            "1s",
            "--output",
            "frame.png",
        ])
        .is_err()
    );
    assert!(
        pcx_cli::cli::try_run_from([
            "pcx",
            "snapshot",
            "recording.mcap",
            "--topic",
            "/points",
            "--frame",
            "0",
        ])
        .is_err()
    );
}

#[test]
fn writes_binary_png_to_explicit_stdout() {
    let first = snapshot(&["--output", "-", "--width", "8", "--height", "4"]);
    let second = snapshot(&["--output", "-", "--width", "8", "--height", "4"]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(first.stdout, second.stdout);
    assert!(first.stdout.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(first.stderr.is_empty());
}

#[test]
fn file_output_is_atomic_and_requires_force_to_replace() {
    let directory = TempDirectory::new("atomic");
    let destination = directory.join("frame.png");
    let destination_text = destination.to_str().unwrap();
    let first = snapshot(&[
        "--output",
        destination_text,
        "--width",
        "8",
        "--height",
        "4",
    ]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let original = fs::read(&destination).unwrap();
    assert!(original.starts_with(b"\x89PNG\r\n\x1a\n"));

    let refused = snapshot(&["--output", destination_text]);
    assert_eq!(refused.status.code(), Some(2));
    assert_eq!(fs::read(&destination).unwrap(), original);

    let replaced = snapshot(&["--output", destination_text, "--force"]);
    assert!(
        replaced.status.success(),
        "{}",
        String::from_utf8_lossy(&replaced.stderr)
    );
    assert_ne!(fs::read(&destination).unwrap(), original);
    assert_no_temporary_output(&directory);
}

#[test]
fn memory_preflight_fails_before_creating_output() {
    let directory = TempDirectory::new("memory");
    let destination = directory.join("frame.png");
    let output = snapshot(&[
        "--output",
        destination.to_str().unwrap(),
        "--memory-limit",
        "1",
    ]);
    assert_eq!(output.status.code(), Some(6));
    assert!(!destination.exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("managed-memory peak"));
    assert_no_temporary_output(&directory);
}
