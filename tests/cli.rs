use std::{
    collections::BTreeMap,
    fs::{self, File},
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use mcap::{
    records::MessageHeader,
    write::{WriteOptions, Writer},
};

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

struct TempSource(PathBuf);

impl TempSource {
    fn new(name: &str) -> Self {
        let id = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let path = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("pcx-{name}-{}-{id}.mcap", std::process::id()));
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempSource {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn write_recording(path: &Path, messages: &[(u64, &[u8])]) {
    let file = File::create(path).expect("fixture should be created");
    let options = WriteOptions::new()
        .profile("ros2")
        .library("pcx-cli-test")
        .compression(None);
    let mut writer = Writer::with_options(file, options).expect("writer should start");
    if !messages.is_empty() {
        let schema = writer
            .add_schema("sensor_msgs/msg/PointCloud2", "ros2msg", b"schema")
            .expect("schema should be added");
        let channel = writer
            .add_channel(schema, "/points", "cdr", &BTreeMap::new())
            .expect("channel should be added");
        for (sequence, (log_time, payload)) in messages.iter().enumerate() {
            writer
                .write_to_known_channel(
                    &MessageHeader {
                        channel_id: channel,
                        sequence: sequence as u32,
                        log_time: *log_time,
                        publish_time: *log_time,
                    },
                    payload,
                )
                .expect("message should be written");
        }
    }
    writer.finish().expect("writer should finish");
}

#[test]
fn help_identifies_the_product_and_its_status() {
    let output = Command::new(env!("CARGO_BIN_EXE_pcx"))
        .arg("--help")
        .output()
        .expect("pcx should start");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("help should be UTF-8");
    assert!(stdout.contains("Inspect and reduce point-cloud recordings"));
    assert!(stdout.contains("info"));
    assert!(output.stderr.is_empty());
}

#[test]
fn version_matches_the_package_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_pcx"))
        .arg("--version")
        .output()
        .expect("pcx should start");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("version should be UTF-8"),
        format!("pcx {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn info_help_labels_input_and_every_supported_option() {
    let output = Command::new(env!("CARGO_BIN_EXE_pcx"))
        .args(["info", "--help"])
        .output()
        .expect("pcx should start");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("help should be UTF-8");
    assert!(stdout.contains("Usage: pcx info [OPTIONS] <INPUT.mcap>"));
    assert!(stdout.contains("MCAP Source to inspect"));
    assert!(stdout.contains("--json"));
    assert!(stdout.contains("--help"));
}

#[test]
fn info_human_output_is_deterministic_and_stderr_free() {
    let source = TempSource::new("human");
    write_recording(source.path(), &[(140, b"later"), (100, b"earlier")]);
    let size = fs::metadata(source.path()).expect("fixture metadata").len();

    let first = Command::new(env!("CARGO_BIN_EXE_pcx"))
        .args(["info", source.path().to_str().expect("UTF-8 path")])
        .output()
        .expect("pcx should start");
    let second = Command::new(env!("CARGO_BIN_EXE_pcx"))
        .args(["info", source.path().to_str().expect("UTF-8 path")])
        .output()
        .expect("pcx should start");

    assert!(first.status.success());
    assert!(first.stderr.is_empty());
    assert_eq!(first.stdout, second.stdout);
    assert_eq!(
        String::from_utf8(first.stdout).expect("human output should be UTF-8"),
        format!(
            "Profile: ros2\nLibrary: pcx-cli-test\nSize: {size} bytes\nMessages: 2\nSchemas: 1\nChannels: 1\nMetadata records: 0\nStart log time: 100 ns\nEnd log time: 140 ns\nDuration: 40 ns\n"
        )
    );
}

#[test]
fn info_json_has_the_versioned_stable_envelope() {
    let source = TempSource::new("json");
    write_recording(source.path(), &[(7, b"payload")]);
    let size = fs::metadata(source.path()).expect("fixture metadata").len();

    let output = Command::new(env!("CARGO_BIN_EXE_pcx"))
        .args([
            "info",
            source.path().to_str().expect("UTF-8 path"),
            "--json",
        ])
        .output()
        .expect("pcx should start");
    let repeated = Command::new(env!("CARGO_BIN_EXE_pcx"))
        .args([
            "info",
            source.path().to_str().expect("UTF-8 path"),
            "--json",
        ])
        .output()
        .expect("pcx should start");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout, repeated.stdout);
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("JSON output should parse");
    assert_eq!(
        value,
        serde_json::json!({
            "schema_version": 1,
            "command": "info",
            "data": {
                "profile": "ros2",
                "library": "pcx-cli-test",
                "size_bytes": size,
                "message_count": 1,
                "schema_count": 1,
                "channel_count": 1,
                "metadata_count": 0,
                "start_log_time_ns": 7,
                "end_log_time_ns": 7,
                "duration_ns": 0
            }
        })
    );
}

#[test]
fn info_accepts_a_valid_empty_container() {
    let source = TempSource::new("empty");
    write_recording(source.path(), &[]);

    let output = Command::new(env!("CARGO_BIN_EXE_pcx"))
        .args([
            "info",
            source.path().to_str().expect("UTF-8 path"),
            "--json",
        ])
        .output()
        .expect("pcx should start");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("JSON output should parse");
    assert_eq!(value["data"]["message_count"], 0);
    assert_eq!(value["data"]["schema_count"], 0);
    assert_eq!(value["data"]["channel_count"], 0);
    assert_eq!(value["data"]["start_log_time_ns"], serde_json::Value::Null);
    assert_eq!(value["data"]["end_log_time_ns"], serde_json::Value::Null);
    assert_eq!(value["data"]["duration_ns"], serde_json::Value::Null);
}

#[test]
fn malformed_and_zero_byte_sources_fail_without_stdout_data() {
    for (name, bytes) in [("zero", Vec::new()), ("malformed", b"not mcap".to_vec())] {
        let source = TempSource::new(name);
        fs::write(source.path(), bytes).expect("fixture should be written");

        let output = Command::new(env!("CARGO_BIN_EXE_pcx"))
            .args(["info", source.path().to_str().expect("UTF-8 path")])
            .output()
            .expect("pcx should start");

        assert_eq!(output.status.code(), Some(3));
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8(output.stderr).expect("diagnostic should be UTF-8");
        assert!(stderr.starts_with("pcx: error: invalid MCAP"));
    }
}
