use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

use pcx_cli::{
    cli::{ExitStatus as PcxExitStatus, run_interruptibly},
    core::{Destination, Error, ErrorCategory, write_output, write_stream},
};

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn test_directory(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "pcx-cli-sink-{name}-{}-{}",
        std::process::id(),
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&path).expect("create test directory");
    path
}

fn temporary_output_exists(directory: &Path) -> bool {
    fs::read_dir(directory)
        .expect("read test directory")
        .filter_map(Result::ok)
        .any(|entry| entry.file_name().to_string_lossy().contains(".pcx.tmp."))
}

fn wait_for_temporary_output(directory: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !temporary_output_exists(directory) {
        assert!(
            Instant::now() < deadline,
            "child did not create temporary output"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn send_sigint(pid: u32) -> ExitStatus {
    Command::new("kill")
        .args(["-INT", &pid.to_string()])
        .status()
        .expect("send SIGINT")
}

#[test]
fn sigint_removes_partial_file_reports_on_stderr_and_exits_130() {
    let directory = test_directory("sigint");
    let destination = directory.join("frame.pcd");
    let child = Command::new(std::env::current_exe().expect("test executable"))
        .args(["--ignored", "--exact", "interrupt_driver", "--nocapture"])
        .env("PCX_SINK_INTERRUPT_DRIVER", &destination)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start interrupt driver");

    wait_for_temporary_output(&directory);
    assert!(send_sigint(child.id()).success());
    let output = child.wait_with_output().expect("wait for interrupt driver");

    assert_eq!(output.status.code(), Some(130));
    assert!(!destination.exists(), "partial destination was committed");
    assert!(
        !temporary_output_exists(&directory),
        "temporary output remained"
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("pcx:"),
        "diagnostic leaked to stdout"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("pcx: operation interrupted"),
        "missing stderr interruption diagnostic: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::remove_dir_all(directory).expect("remove test directory");
}

#[test]
fn process_stream_keeps_binary_stdout_separate_from_diagnostics() {
    let output = Command::new(std::env::current_exe().expect("test executable"))
        .args(["--ignored", "--exact", "stream_driver", "--nocapture"])
        .env("PCX_SINK_STREAM_DRIVER", "1")
        .output()
        .expect("run stream driver");

    assert_eq!(output.status.code(), Some(PcxExitStatus::Io.code().into()));
    assert!(
        output
            .stdout
            .windows(BINARY_PAYLOAD.len())
            .any(|window| window == BINARY_PAYLOAD),
        "binary payload was not preserved"
    );
    assert!(!output.stdout.windows(4).any(|window| window == b"pcx:"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("pcx: injected diagnostic"));
}

const BINARY_PAYLOAD: &[u8] = b"pcx-binary:\0\xff\xfe\n";

#[test]
#[ignore = "subprocess driver"]
fn interrupt_driver() {
    let Some(path) = std::env::var_os("PCX_SINK_INTERRUPT_DRIVER") else {
        return;
    };
    let status = run_interruptibly(|cancellation| {
        let destination = Destination::file(path, false)?;
        write_output(&destination, &cancellation, |writer| {
            loop {
                writer.write_all(&[0x5a; 4096])?;
                thread::sleep(Duration::from_millis(2));
            }
        })?;
        Ok(())
    });
    std::process::exit(status.code().into());
}

#[test]
#[ignore = "subprocess driver"]
fn stream_driver() {
    if std::env::var_os("PCX_SINK_STREAM_DRIVER").is_none() {
        return;
    }
    let status = run_interruptibly(|cancellation| {
        write_stream(std::io::stdout().lock(), &cancellation, |writer| {
            writer.write_all(BINARY_PAYLOAD)
        })?;
        Err(Error::new(ErrorCategory::Io, "injected diagnostic"))
    });
    std::process::exit(status.code().into());
}
