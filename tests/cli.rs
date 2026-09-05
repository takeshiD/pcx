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

fn write_topics_recording(path: &Path) {
    let file = File::create(path).expect("fixture should be created");
    let mut writer = Writer::with_options(
        file,
        WriteOptions::new()
            .profile("ros2")
            .library("pcx-cli-test")
            .compression(None),
    )
    .expect("writer should start");
    let point_schema = writer
        .add_schema("sensor_msgs/msg/PointCloud2", "ros2msg", b"point schema")
        .expect("schema should be added");
    let image_schema = writer
        .add_schema("sensor_msgs/msg/Image", "ros2msg", b"image schema")
        .expect("schema should be added");
    let later = writer
        .add_channel(point_schema, "/z", "cdr", &BTreeMap::new())
        .expect("channel should be added");
    let duplicate_candidate = writer
        .add_channel(point_schema, "/points", "cdr", &BTreeMap::new())
        .expect("channel should be added");
    let duplicate_other = writer
        .add_channel(image_schema, "/points", "cdr", &BTreeMap::new())
        .expect("channel should be added");
    for (sequence, channel_id) in [
        duplicate_other,
        duplicate_candidate,
        duplicate_candidate,
        later,
    ]
    .into_iter()
    .enumerate()
    {
        writer
            .write_to_known_channel(
                &MessageHeader {
                    channel_id,
                    sequence: sequence as u32,
                    log_time: sequence as u64,
                    publish_time: sequence as u64,
                },
                b"not decoded by discovery",
            )
            .expect("message should be written");
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
    assert!(stdout.contains("topics"));
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
fn topics_help_labels_input_and_every_supported_option() {
    let output = Command::new(env!("CARGO_BIN_EXE_pcx"))
        .args(["topics", "--help"])
        .output()
        .expect("pcx should start");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("help should be UTF-8");
    assert!(stdout.contains("Usage: pcx topics [OPTIONS] <INPUT.mcap>"));
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

#[test]
fn topics_json_is_versioned_deterministic_and_preserves_duplicate_names() {
    let source = TempSource::new("topics-json");
    write_topics_recording(source.path());
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_pcx"))
            .args([
                "topics",
                source.path().to_str().expect("UTF-8 path"),
                "--json",
            ])
            .output()
            .expect("pcx should start")
    };

    let first = run();
    let second = run();

    assert!(first.status.success());
    assert!(first.stderr.is_empty());
    assert_eq!(first.stdout, second.stdout);
    assert!(second.stderr.is_empty());
    let report: serde_json::Value =
        serde_json::from_slice(&first.stdout).expect("stdout should be JSON");
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["command"], "topics");
    let channels = report["data"]["channels"]
        .as_array()
        .expect("channels should be an array");
    assert_eq!(channels.len(), 3);
    assert_eq!(channels[0]["topic"], "/points");
    assert_eq!(channels[1]["topic"], "/points");
    assert_eq!(channels[2]["topic"], "/z");
    assert_eq!(channels[0]["message_count"], 2);
    assert_eq!(channels[1]["message_count"], 1);
    assert_eq!(channels[0]["message_encoding"], "cdr");
    assert_eq!(channels[0]["schema"]["name"], "sensor_msgs/msg/PointCloud2");
    assert_eq!(channels[0]["schema"]["encoding"], "ros2msg");
    assert_eq!(channels[0]["ros2_pointcloud2_candidate"], true);
    assert_eq!(channels[1]["ros2_pointcloud2_candidate"], false);
}

#[test]
fn topics_human_output_uses_topic_and_mcap_channel_terms_without_decode_claims() {
    let source = TempSource::new("topics-human");
    write_topics_recording(source.path());
    let output = Command::new(env!("CARGO_BIN_EXE_pcx"))
        .args(["topics", source.path().to_str().expect("UTF-8 path")])
        .output()
        .expect("pcx should start");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("output should be UTF-8");
    assert!(stdout.starts_with("Topic: /points\n  MCAP Channel ID:"));
    assert_eq!(stdout.matches("Topic: /points").count(), 2);
    assert!(stdout.contains("ROS 2 PointCloud2 candidate: yes"));
    assert!(stdout.contains("message payloads were not decoded"));
}

#[test]
fn topics_reports_an_empty_container_only_on_stdout() {
    let source = TempSource::new("topics-empty");
    write_recording(source.path(), &[]);
    let output = Command::new(env!("CARGO_BIN_EXE_pcx"))
        .args(["topics", source.path().to_str().expect("UTF-8 path")])
        .output()
        .expect("pcx should start");

    assert!(output.status.success());
    assert_eq!(output.stdout, b"No Topics found.\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn topics_rejects_malformed_input_only_on_stderr() {
    let source = TempSource::new("topics-malformed");
    fs::write(source.path(), b"not an MCAP").expect("fixture should be written");
    let output = Command::new(env!("CARGO_BIN_EXE_pcx"))
        .args([
            "topics",
            source.path().to_str().expect("UTF-8 path"),
            "--json",
        ])
        .output()
        .expect("pcx should start");

    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    let error: serde_json::Value =
        serde_json::from_slice(&output.stderr).expect("diagnostic should be JSON");
    assert_eq!(error["schema_version"], 1);
    assert_eq!(error["command"], "topics");
    assert_eq!(error["error"]["category"], "invalid_data");
    assert!(
        error["error"]["message"]
            .as_str()
            .is_some_and(|message| message.starts_with("invalid MCAP"))
    );
}
