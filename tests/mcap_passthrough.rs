//! MCAP passthrough fidelity tests using the official `mcap` crate as oracle.

use std::{
    borrow::Cow,
    collections::BTreeMap,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use mcap::{
    Attachment, MessageStream, Summary,
    read::ChunkFlattener,
    records::{MessageHeader, Metadata, Record},
    write::{WriteOptions, Writer},
};

static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new(name: &str) -> Self {
        let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "pcx-passthrough-{name}-{}-{}",
            std::process::id(),
            NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("temporary directory should be created");
        Self(path)
    }

    fn join(&self, name: impl AsRef<Path>) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn recording() -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    {
        let mut writer = Writer::with_options(
            &mut output,
            WriteOptions::new()
                .profile("ros2")
                .library("independent-fixture")
                .compression(None),
        )
        .expect("writer should start");
        writer
            .add_schema_with_id(42, "custom/Encoded", "opaque", b"schema-bytes")
            .expect("schema should be added");
        writer
            .add_channel_with_id(
                7,
                42,
                "/selected",
                "application/octet-stream",
                &BTreeMap::from([("frame".into(), "map".into())]),
            )
            .expect("selected Channel should be added");
        writer
            .add_channel_with_id(
                8,
                0,
                "/other",
                "bytes",
                &BTreeMap::from([("drop".into(), "message-only".into())]),
            )
            .expect("other Channel should be added");
        for (channel_id, sequence, log_time, publish_time, data) in [
            (8, 1, 90, 91, b"other".as_slice()),
            (7, 2, 100, 101, b"first".as_slice()),
            (7, 3, 110, 112, b"selected-payload\0\xff".as_slice()),
        ] {
            writer
                .write_to_known_channel(
                    &MessageHeader {
                        channel_id,
                        sequence,
                        log_time,
                        publish_time,
                    },
                    data,
                )
                .expect("message should be added");
        }
        writer
            .write_metadata(&Metadata {
                name: "calibration".into(),
                metadata: BTreeMap::from([("serial".into(), "synthetic-1".into())]),
            })
            .expect("metadata should be added");
        writer
            .attach(&Attachment {
                log_time: 105,
                create_time: 80,
                name: "notes.bin".into(),
                media_type: "application/octet-stream".into(),
                data: Cow::Borrowed(b"attachment-data"),
            })
            .expect("attachment should be added");
        writer
            .write_private_record(0x80, b"private-extension", Default::default())
            .expect("private record should be added");
        writer.finish().expect("writer should finish");
    }
    output.into_inner()
}

fn run_passthrough(source: &Path, destination: &Path, compression: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pcx"))
        .args([
            "passthrough",
            source.to_str().unwrap(),
            "--topic",
            "/selected",
            "--frame",
            "1",
            "--output",
            destination.to_str().unwrap(),
            "--compression",
            compression,
        ])
        .output()
        .expect("pcx should start")
}

#[test]
fn preserves_selected_encoded_semantics_and_global_records() {
    let directory = TempDirectory::new("fidelity");
    let source = directory.join("source.mcap");
    let destination = directory.join("selected.mcap");
    fs::write(&source, recording()).expect("source should be written");

    let result = run_passthrough(&source, &destination, "none");
    assert!(
        result.status.success(),
        "passthrough failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stdout.is_empty());
    assert!(result.stderr.is_empty());

    let output = fs::read(destination).expect("atomic output should exist");
    let messages = MessageStream::new(&output)
        .expect("official MCAP reader should open output")
        .collect::<Result<Vec<_>, _>>()
        .expect("official MCAP reader should decode messages");
    assert_eq!(messages.len(), 1);
    let message = &messages[0];
    assert_eq!(message.channel.id, 7);
    assert_eq!(message.channel.topic, "/selected");
    assert_eq!(message.channel.message_encoding, "application/octet-stream");
    assert_eq!(message.channel.metadata["frame"], "map");
    let schema = message.channel.schema.as_ref().expect("selected Schema");
    assert_eq!(schema.id, 42);
    assert_eq!(schema.name, "custom/Encoded");
    assert_eq!(schema.encoding, "opaque");
    assert_eq!(schema.data.as_ref(), b"schema-bytes");
    assert_eq!(message.sequence, 3);
    assert_eq!(message.log_time, 110);
    assert_eq!(message.publish_time, 112);
    assert_eq!(message.data.as_ref(), b"selected-payload\0\xff");

    let summary = Summary::read(&output)
        .expect("official summary reader should succeed")
        .expect("output should contain a summary");
    assert_eq!(summary.channels.len(), 1);
    assert_eq!(summary.schemas.len(), 1);
    assert!(summary.attachment_indexes.is_empty());
    assert!(summary.metadata_indexes.is_empty());

    let mut attachment = None;
    let mut metadata = None;
    let mut private = None;
    for record in ChunkFlattener::new(&output).expect("official record reader should open output") {
        match record.expect("record should parse") {
            Record::Attachment { header, data, .. } => {
                attachment = Some((header, data.into_owned()));
            }
            Record::Metadata(value) => metadata = Some(value),
            Record::Unknown { opcode, data } => {
                private = Some((opcode, data.into_owned()));
            }
            Record::DataEnd(_) => break,
            _ => {}
        }
    }
    let (attachment_header, attachment_data) = attachment.expect("attachment should be preserved");
    assert_eq!(attachment_header.log_time, 105);
    assert_eq!(attachment_header.create_time, 80);
    assert_eq!(attachment_header.name, "notes.bin");
    assert_eq!(attachment_header.media_type, "application/octet-stream");
    assert_eq!(attachment_data, b"attachment-data");
    let metadata = metadata.expect("metadata should be preserved");
    assert_eq!(metadata.name, "calibration");
    assert_eq!(metadata.metadata["serial"], "synthetic-1");
    let private = private.expect("private record should be preserved");
    assert_eq!(private, (0x80, b"private-extension".to_vec()));
}

#[test]
fn zstd_and_lz4_output_are_byte_deterministic() {
    let directory = TempDirectory::new("determinism");
    let source = directory.join("source.mcap");
    fs::write(&source, recording()).expect("source should be written");

    for compression in ["zstd", "lz4"] {
        let first = directory.join(format!("{compression}-first.mcap"));
        let second = directory.join(format!("{compression}-second.mcap"));
        assert!(
            run_passthrough(&source, &first, compression)
                .status
                .success()
        );
        assert!(
            run_passthrough(&source, &second, compression)
                .status
                .success()
        );
        assert_eq!(
            fs::read(first).unwrap(),
            fs::read(second).unwrap(),
            "{compression} output changed for identical input and options"
        );
    }
}

#[test]
fn unknown_future_standard_record_is_refused_without_output() {
    let directory = TempDirectory::new("unknown-standard");
    let source = directory.join("source.mcap");
    let destination = directory.join("selected.mcap");
    let mut bytes = recording();
    let data_end = bytes
        .windows(9)
        .position(|window| window[0] == 0x0f && window[1..9] == 4_u64.to_le_bytes())
        .expect("DataEnd record");
    let mut unknown = vec![0x10];
    unknown.extend(3_u64.to_le_bytes());
    unknown.extend(b"new");
    bytes.splice(data_end..data_end, unknown);
    fs::write(&source, bytes).expect("source should be written");

    let result = run_passthrough(&source, &destination, "none");
    assert_eq!(result.status.code(), Some(4));
    assert!(result.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("cannot faithfully preserve unknown reserved MCAP record opcode 0x10")
    );
    assert!(!destination.exists());
}

#[test]
fn malformed_source_and_memory_refusal_leave_no_partial_output() {
    let directory = TempDirectory::new("failures");
    let malformed = directory.join("malformed.mcap");
    let malformed_output = directory.join("malformed-output.mcap");
    fs::write(&malformed, b"not an MCAP").unwrap();
    let malformed_result = run_passthrough(&malformed, &malformed_output, "none");
    assert_eq!(malformed_result.status.code(), Some(3));
    assert!(!malformed_output.exists());

    let source = directory.join("source.mcap");
    let limited_output = directory.join("limited-output.mcap");
    fs::write(&source, recording()).unwrap();
    let limited = Command::new(env!("CARGO_BIN_EXE_pcx"))
        .args([
            "passthrough",
            source.to_str().unwrap(),
            "--topic",
            "/selected",
            "--frame",
            "1",
            "--output",
            limited_output.to_str().unwrap(),
            "--memory-limit",
            "1",
        ])
        .output()
        .expect("pcx should start");
    assert_eq!(limited.status.code(), Some(6));
    assert!(!limited_output.exists());
    assert!(
        fs::read_dir(&directory.0)
            .unwrap()
            .filter_map(Result::ok)
            .all(|entry| !entry.file_name().to_string_lossy().contains(".pcx.tmp."))
    );
}
