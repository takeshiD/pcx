use std::{collections::BTreeMap, io::Cursor, time::Duration};

use mcap::{
    records::MessageHeader,
    write::{WriteOptions, Writer},
};
use pcx_cli::{
    core::{ErrorCategory, FrameSelector},
    mcap::{SelectionError, Source, SourceOptions, select_topic_message},
};

struct TestMessage<'a> {
    channel: usize,
    log_time: u64,
    data: &'a [u8],
}

fn recording(topics: &[&str], messages: &[TestMessage<'_>]) -> Vec<u8> {
    let cursor = Cursor::new(Vec::new());
    let mut writer = Writer::with_options(
        cursor,
        WriteOptions::new()
            .profile("ros2")
            .library("pcx-selection-test")
            .compression(None),
    )
    .expect("writer should start");
    let schema = writer
        .add_schema("sensor_msgs/msg/PointCloud2", "ros2msg", b"schema")
        .expect("schema should be added");
    let channels: Vec<u16> = topics
        .iter()
        .map(|topic| {
            writer
                .add_channel(schema, topic, "cdr", &BTreeMap::new())
                .expect("channel should be added")
        })
        .collect();

    for (sequence, message) in messages.iter().enumerate() {
        writer
            .write_to_known_channel(
                &MessageHeader {
                    channel_id: channels[message.channel],
                    sequence: sequence as u32,
                    log_time: message.log_time,
                    publish_time: message.log_time,
                },
                message.data,
            )
            .expect("message should be written");
    }
    writer.finish().expect("writer should finish");
    writer.into_inner().into_inner()
}

fn source(bytes: Vec<u8>) -> Source<Cursor<Vec<u8>>> {
    Source::new(
        Cursor::new(bytes),
        SourceOptions {
            read_chunk_bytes: 7,
            max_record_bytes: 1024 * 1024,
        },
    )
    .expect("source should open")
}

#[test]
fn frame_index_counts_only_matching_topic_across_duplicate_channels() {
    let bytes = recording(
        &["/points", "/imu", "/points"],
        &[
            TestMessage {
                channel: 0,
                log_time: 100,
                data: b"points-0",
            },
            TestMessage {
                channel: 1,
                log_time: 101,
                data: b"imu",
            },
            TestMessage {
                channel: 2,
                log_time: 102,
                data: b"points-1",
            },
            TestMessage {
                channel: 0,
                log_time: 103,
                data: b"points-2",
            },
        ],
    );

    let selected = select_topic_message(&mut source(bytes), "/points", FrameSelector::Index(1))
        .expect("second matching Point Frame should exist");

    assert_eq!(selected.data(), b"points-1");
    assert_eq!(selected.sequence(), 2);
    assert_eq!(selected.log_time(), 102);
    assert_eq!(selected.publish_time(), 102);
}

#[test]
fn missing_topic_and_out_of_range_frame_are_distinct() {
    let bytes = recording(
        &["/points"],
        &[TestMessage {
            channel: 0,
            log_time: 100,
            data: b"points-0",
        }],
    );

    let missing_topic = select_topic_message(
        &mut source(bytes.clone()),
        "/missing",
        FrameSelector::Index(0),
    )
    .expect_err("unknown Topic should fail");
    let missing_frame =
        select_topic_message(&mut source(bytes), "/points", FrameSelector::Index(1))
            .expect_err("out-of-range Point Frame should fail");

    assert_eq!(missing_topic.category(), ErrorCategory::NotFound);
    assert_eq!(missing_frame.category(), ErrorCategory::NotFound);
    assert!(matches!(
        missing_topic,
        SelectionError::TopicNotFound { ref topic } if topic == "/missing"
    ));
    assert!(matches!(
        missing_frame,
        SelectionError::PointFrameNotFound {
            ref topic,
            selector: FrameSelector::Index(1),
        } if topic == "/points"
    ));
}

#[test]
fn time_selector_uses_recording_start_and_includes_the_boundary() {
    let bytes = recording(
        &["/other", "/points"],
        &[
            TestMessage {
                channel: 0,
                log_time: 100,
                data: b"recording-start",
            },
            TestMessage {
                channel: 1,
                log_time: 109,
                data: b"before",
            },
            TestMessage {
                channel: 1,
                log_time: 110,
                data: b"boundary",
            },
            TestMessage {
                channel: 1,
                log_time: 111,
                data: b"after",
            },
        ],
    );

    let selected = select_topic_message(
        &mut source(bytes),
        "/points",
        FrameSelector::At(Duration::from_nanos(10)),
    )
    .expect("boundary Point Frame should exist");

    assert_eq!(selected.data(), b"boundary");
    assert_eq!(selected.log_time(), 110);
}

#[test]
fn time_selector_returns_the_first_qualifying_message_in_mcap_order() {
    let bytes = recording(
        &["/points"],
        &[
            TestMessage {
                channel: 0,
                log_time: 100,
                data: b"start",
            },
            TestMessage {
                channel: 0,
                log_time: 120,
                data: b"first-qualifying",
            },
            TestMessage {
                channel: 0,
                log_time: 115,
                data: b"later-in-container",
            },
        ],
    );

    let selected = select_topic_message(
        &mut source(bytes),
        "/points",
        FrameSelector::At(Duration::from_nanos(10)),
    )
    .expect("qualifying Point Frame should exist");

    assert_eq!(selected.data(), b"first-qualifying");
    assert_eq!(selected.log_time(), 120);
}

#[test]
fn time_selector_reports_a_missing_point_frame_explicitly() {
    let bytes = recording(
        &["/points"],
        &[TestMessage {
            channel: 0,
            log_time: 100,
            data: b"only-frame",
        }],
    );

    let error = select_topic_message(
        &mut source(bytes),
        "/points",
        FrameSelector::At(Duration::from_nanos(1)),
    )
    .expect_err("no Point Frame reaches the requested time");

    assert!(matches!(
        error,
        SelectionError::PointFrameNotFound {
            ref topic,
            selector: FrameSelector::At(duration),
        } if topic == "/points" && duration == Duration::from_nanos(1)
    ));
}
