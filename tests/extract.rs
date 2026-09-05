//! End-to-end extraction tests with an independent PCD oracle.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use mcap::{
    records::MessageHeader,
    write::{WriteOptions, Writer},
};

static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new(name: &str) -> Self {
        let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "pcx-extract-{name}-{}-{}",
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

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn extract_to(source: &Path, destination: &Path, extra: &[&str]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pcx"));
    command.args([
        "extract",
        source.to_str().unwrap(),
        "--topic",
        "/lidar/points",
        "--frame",
        "0",
        "--output",
        destination.to_str().unwrap(),
    ]);
    command.args(extra);
    command.output().expect("pcx should start")
}

fn assert_no_temporary_output(directory: &TempDirectory) {
    assert!(
        fs::read_dir(&directory.0)
            .expect("temporary directory should be readable")
            .filter_map(Result::ok)
            .all(|entry| !entry.file_name().to_string_lossy().contains(".pcx.tmp.")),
        "temporary extraction output remained"
    );
}

fn write_pointcloud2_recording(
    path: &Path,
    schema_name: &str,
    schema_encoding: &str,
    message_encoding: &str,
    payload: &[u8],
) {
    let file = fs::File::create(path).expect("test MCAP should be created");
    let mut writer = Writer::with_options(
        file,
        WriteOptions::new()
            .profile("ros2")
            .library("pcx-extract-test")
            .compression(None),
    )
    .expect("MCAP writer should start");
    let schema = writer
        .add_schema(schema_name, schema_encoding, b"test schema")
        .expect("schema should be added");
    let channel = writer
        .add_channel(schema, "/lidar/points", message_encoding, &BTreeMap::new())
        .expect("Channel should be added");
    writer
        .write_to_known_channel(
            &MessageHeader {
                channel_id: channel,
                sequence: 0,
                log_time: 100,
                publish_time: 100,
            },
            payload,
        )
        .expect("message should be added");
    writer.finish().expect("MCAP should finish");
}

#[derive(Debug)]
struct Pcd<'a> {
    header: BTreeMap<&'a str, Vec<&'a str>>,
    body: &'a [u8],
}

fn parse_pcd(bytes: &[u8]) -> Pcd<'_> {
    let data_line = bytes
        .windows(b"DATA ".len())
        .position(|window| window == b"DATA ")
        .expect("PCD DATA entry");
    let body_offset = data_line
        + bytes[data_line..]
            .iter()
            .position(|byte| *byte == b'\n')
            .expect("PCD DATA newline")
        + 1;
    let text = std::str::from_utf8(&bytes[..body_offset]).expect("ASCII PCD header");
    let mut header = BTreeMap::new();
    for line in text.lines().filter(|line| !line.starts_with('#')) {
        let mut tokens = line.split_ascii_whitespace();
        let key = tokens.next().expect("PCD header key");
        assert!(header.insert(key, tokens.collect()).is_none());
    }
    Pcd {
        header,
        body: &bytes[body_offset..],
    }
}

fn assert_binary_pointcloud2(pcd: &Pcd<'_>) {
    assert_eq!(pcd.header["FIELDS"], ["x", "y", "z", "intensity", "ring"]);
    assert_eq!(pcd.header["SIZE"], ["4", "4", "4", "2", "2"]);
    assert_eq!(pcd.header["TYPE"], ["F", "F", "F", "U", "U"]);
    assert_eq!(pcd.header["COUNT"], ["1", "1", "1", "1", "1"]);
    assert_eq!(pcd.header["WIDTH"], ["2"]);
    assert_eq!(pcd.header["HEIGHT"], ["1"]);
    assert_eq!(pcd.header["POINTS"], ["2"]);
    assert_eq!(pcd.header["DATA"], ["binary"]);
    assert_eq!(pcd.body.len(), 32);

    let expected = [
        ([0x3f80_0000, 0xc020_0000, 0], [42, 7]),
        ([0x8000_0000, 0x7f80_0000, 0x7fc0_1234], [u16::MAX, 8]),
    ];
    for (point, (floats, integers)) in expected.iter().enumerate() {
        let record = &pcd.body[point * 16..(point + 1) * 16];
        assert_eq!(
            [
                u32::from_le_bytes(record[0..4].try_into().unwrap()),
                u32::from_le_bytes(record[4..8].try_into().unwrap()),
                u32::from_le_bytes(record[8..12].try_into().unwrap()),
            ],
            *floats
        );
        assert_eq!(
            [
                u16::from_le_bytes(record[12..14].try_into().unwrap()),
                u16::from_le_bytes(record[14..16].try_into().unwrap()),
            ],
            *integers
        );
    }
}

fn assert_ascii_pointcloud2(pcd: &Pcd<'_>) {
    assert_eq!(pcd.header["FIELDS"], ["x", "y", "z", "intensity", "ring"]);
    assert_eq!(pcd.header["SIZE"], ["4", "4", "4", "2", "2"]);
    assert_eq!(pcd.header["TYPE"], ["F", "F", "F", "U", "U"]);
    assert_eq!(pcd.header["COUNT"], ["1", "1", "1", "1", "1"]);
    assert_eq!(pcd.header["WIDTH"], ["2"]);
    assert_eq!(pcd.header["HEIGHT"], ["1"]);
    assert_eq!(pcd.header["POINTS"], ["2"]);
    assert_eq!(pcd.header["DATA"], ["ascii"]);
    let rows: Vec<Vec<&str>> = std::str::from_utf8(pcd.body)
        .expect("ASCII PCD body")
        .lines()
        .map(|line| line.split_ascii_whitespace().collect())
        .collect();
    assert_eq!(
        rows,
        [
            ["1", "-2.5", "0", "42", "7"],
            ["-0", "inf", "nan", "65535", "8"]
        ]
    );
    assert_eq!(rows[1][0].parse::<f32>().unwrap().to_bits(), 0x8000_0000);
    assert_eq!(rows[1][1].parse::<f32>().unwrap(), f32::INFINITY);
    assert!(rows[1][2].parse::<f32>().unwrap().is_nan());
}

#[test]
fn extracts_one_binary_point_frame_to_an_atomic_file() {
    let directory = TempDirectory::new("binary-file");
    let destination = directory.join("frame.pcd");
    let output = Command::new(env!("CARGO_BIN_EXE_pcx"))
        .args([
            "extract",
            fixture("valid/pointcloud2.mcap").to_str().unwrap(),
            "--topic",
            "/lidar/points",
            "--frame",
            "0",
            "--output",
            destination.to_str().unwrap(),
        ])
        .output()
        .expect("pcx should start");

    assert!(
        output.status.success(),
        "extraction failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert_binary_pointcloud2(&parse_pcd(
        &fs::read(destination).expect("committed PCD output"),
    ));
}

#[test]
fn extracts_ascii_to_explicit_stdout_without_diagnostics() {
    let output = Command::new(env!("CARGO_BIN_EXE_pcx"))
        .args([
            "extract",
            fixture("valid/pointcloud2.mcap").to_str().unwrap(),
            "--topic",
            "/lidar/points",
            "--frame",
            "0",
            "--output",
            "-",
            "--encoding",
            "ascii",
        ])
        .output()
        .expect("pcx should start");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_ascii_pointcloud2(&parse_pcd(&output.stdout));
}

#[test]
fn extracts_binary_to_explicit_stdout_byte_for_byte() {
    let output = Command::new(env!("CARGO_BIN_EXE_pcx"))
        .args([
            "extract",
            fixture("valid/pointcloud2.mcap").to_str().unwrap(),
            "--topic",
            "/lidar/points",
            "--frame",
            "0",
            "--output",
            "-",
        ])
        .output()
        .expect("pcx should start");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_binary_pointcloud2(&parse_pcd(&output.stdout));
}

#[test]
fn missing_topic_and_frame_have_not_found_status_without_output() {
    let directory = TempDirectory::new("not-found");
    let source = fixture("valid/pointcloud2.mcap");

    for (topic, frame, name, diagnostic) in [
        (
            "/missing",
            "0",
            "missing-topic.pcd",
            "Topic \"/missing\" was not found",
        ),
        (
            "/lidar/points",
            "1",
            "missing-frame.pcd",
            "no Point Frame for Topic \"/lidar/points\" matched Index(1)",
        ),
    ] {
        let destination = directory.join(name);
        let output = Command::new(env!("CARGO_BIN_EXE_pcx"))
            .args([
                "extract",
                source.to_str().unwrap(),
                "--topic",
                topic,
                "--frame",
                frame,
                "--output",
                destination.to_str().unwrap(),
            ])
            .output()
            .expect("pcx should start");
        assert_eq!(output.status.code(), Some(5));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains(diagnostic));
        assert!(!destination.exists());
    }
    assert_no_temporary_output(&directory);
}

#[test]
fn malformed_mcap_fails_as_invalid_data_without_partial_output() {
    let directory = TempDirectory::new("malformed");
    let destination = directory.join("frame.pcd");
    let output = extract_to(
        &fixture("malformed/mcap-leading-magic-must-match.mcap"),
        &destination,
        &[],
    );

    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid MCAP"));
    assert!(!destination.exists());
    assert_no_temporary_output(&directory);
}

#[test]
fn memory_refusal_happens_before_destination_creation() {
    let directory = TempDirectory::new("memory-refusal");
    let destination = directory.join("frame.pcd");
    let output = extract_to(
        &fixture("valid/pointcloud2.mcap"),
        &destination,
        &["--memory-limit", "1"],
    );

    assert_eq!(output.status.code(), Some(6));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("managed-memory peak"));
    assert!(!destination.exists());
    assert_no_temporary_output(&directory);
}

#[test]
fn existing_destination_requires_force_and_forced_output_is_atomic() {
    let directory = TempDirectory::new("force");
    let destination = directory.join("frame.pcd");
    fs::write(&destination, b"original").expect("existing destination should be seeded");

    let refusal = extract_to(&fixture("valid/pointcloud2.mcap"), &destination, &[]);
    assert_eq!(refusal.status.code(), Some(2));
    assert!(refusal.stdout.is_empty());
    assert!(String::from_utf8_lossy(&refusal.stderr).contains("pass --force"));
    assert_eq!(fs::read(&destination).unwrap(), b"original");
    assert_no_temporary_output(&directory);

    let replacement = extract_to(
        &fixture("valid/pointcloud2.mcap"),
        &destination,
        &["--force"],
    );
    assert!(replacement.status.success());
    assert!(replacement.stdout.is_empty());
    assert!(replacement.stderr.is_empty());
    assert_binary_pointcloud2(&parse_pcd(&fs::read(&destination).unwrap()));
    assert_no_temporary_output(&directory);
}

#[test]
fn extract_grammar_requires_topic_selector_and_explicit_sink() {
    let help = Command::new(env!("CARGO_BIN_EXE_pcx"))
        .args(["extract", "--help"])
        .output()
        .expect("pcx should start");
    assert!(help.status.success());
    assert!(help.stderr.is_empty());
    let help = String::from_utf8(help.stdout).unwrap();
    for expected in [
        "--topic <TOPIC>",
        "--frame <INDEX>",
        "--at <DURATION>",
        "--output <PATH|->",
        "--force",
        "--encoding <ENCODING>",
        "--memory-limit <BYTES>",
    ] {
        assert!(
            help.contains(expected),
            "missing {expected:?} from help:\n{help}"
        );
    }

    for args in [
        vec!["extract", "input.mcap", "--frame", "0", "-o", "-"],
        vec!["extract", "input.mcap", "--topic", "/points", "-o", "-"],
        vec![
            "extract",
            "input.mcap",
            "--topic",
            "/points",
            "--frame",
            "0",
        ],
        vec![
            "extract",
            "input.mcap",
            "--topic",
            "/points",
            "--frame",
            "0",
            "--at",
            "0s",
            "-o",
            "-",
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_pcx"))
            .args(args)
            .output()
            .expect("pcx should start");
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }

    let force_stdout = Command::new(env!("CARGO_BIN_EXE_pcx"))
        .args([
            "extract",
            fixture("valid/pointcloud2.mcap").to_str().unwrap(),
            "--topic",
            "/lidar/points",
            "--frame",
            "0",
            "--output",
            "-",
            "--force",
        ])
        .output()
        .expect("pcx should start");
    assert_eq!(force_stdout.status.code(), Some(2));
    assert!(force_stdout.stdout.is_empty());
    assert!(String::from_utf8_lossy(&force_stdout.stderr).contains("only valid for file output"));
}

#[test]
fn duration_selector_uses_recording_relative_time() {
    let output = Command::new(env!("CARGO_BIN_EXE_pcx"))
        .args([
            "extract",
            fixture("valid/pointcloud2.mcap").to_str().unwrap(),
            "--topic",
            "/lidar/points",
            "--at",
            "0.000s",
            "--output",
            "-",
        ])
        .output()
        .expect("pcx should start");

    assert!(
        output.status.success(),
        "duration selection failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert_binary_pointcloud2(&parse_pcd(&output.stdout));
}

#[test]
fn malformed_pointcloud2_payload_fails_before_output_creation() {
    let directory = TempDirectory::new("malformed-pointcloud2");
    let source = directory.join("malformed.mcap");
    let destination = directory.join("frame.pcd");
    let payload = fs::read(fixture(
        "malformed/pointcloud2-field-must-fit-point-step.cdr",
    ))
    .unwrap();
    write_pointcloud2_recording(
        &source,
        "sensor_msgs/msg/PointCloud2",
        "ros2msg",
        "cdr",
        &payload,
    );

    let output = extract_to(&source, &destination, &[]);
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid PointCloud2 layout"));
    assert!(!destination.exists());
    assert_no_temporary_output(&directory);
}

#[test]
fn declared_non_pointcloud2_channel_is_rejected_before_decoding() {
    let directory = TempDirectory::new("wrong-schema");
    let source = directory.join("wrong-schema.mcap");
    let destination = directory.join("frame.pcd");
    let payload = fs::read(fixture("valid/pointcloud2-little-endian.cdr")).unwrap();
    write_pointcloud2_recording(&source, "sensor_msgs/msg/Image", "ros2msg", "cdr", &payload);

    let output = extract_to(&source, &destination, &[]);
    assert_eq!(output.status.code(), Some(4));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not declared as ROS 2 PointCloud2"));
    assert!(!destination.exists());
    assert_no_temporary_output(&directory);
}
