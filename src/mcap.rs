//! Bounded synchronous access to MCAP container records.
//!
//! The adapter drives the official `mcap` crate's sans-I/O state machine from
//! a synchronous [`Read`] + [`Seek`] source. Probe records own any bytes they
//! expose, so advancing or dropping the source cannot invalidate them.

use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    io::{self, Read, Seek, SeekFrom},
    sync::Arc,
};

use ::mcap::{
    McapError, parse_record,
    records::Record,
    sans_io::linear_reader::{LinearReadEvent, LinearReader, LinearReaderOptions},
};

use crate::core::{ErrorCategory, FrameSelector};

const RECORD_ENVELOPE_BYTES: usize = 9;
const ROS2_POINTCLOUD2_SCHEMA: &str = "sensor_msgs/msg/PointCloud2";
const ROS2_SCHEMA_ENCODING: &str = "ros2msg";
const ROS2_MESSAGE_ENCODING: &str = "cdr";
const DECLARATION_INDEX_BYTES: u64 = u16::MAX as u64 + 1;

/// A Schema associated with a discovered MCAP Channel.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DiscoveredSchema {
    pub id: u16,
    pub name: String,
    pub encoding: String,
}

/// One discovered MCAP Channel and the Topic it represents to users.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DiscoveredChannel {
    pub channel_id: u16,
    pub topic: String,
    pub message_encoding: String,
    pub message_count: u64,
    pub schema: Option<DiscoveredSchema>,
    /// Whether declarations identify a ROS 2 PointCloud2 candidate.
    ///
    /// Discovery does not decode message payloads, so this is not a claim that
    /// the Channel's messages are valid or decodable PointCloud2 values.
    pub ros2_pointcloud2_candidate: bool,
}

/// Deterministically ordered Topic discovery results.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TopicDiscovery {
    channels: Vec<DiscoveredChannel>,
}

impl TopicDiscovery {
    pub fn channels(&self) -> &[DiscoveredChannel] {
        &self.channels
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SchemaDeclaration {
    name: String,
    encoding: String,
    data: Arc<[u8]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChannelDeclaration {
    schema_id: u16,
    topic: String,
    message_encoding: String,
    metadata: BTreeMap<String, String>,
}

/// Discover MCAP Channels, their Schemas, and message counts without decoding
/// message payloads.
pub fn discover_topics<R: Read + Seek>(
    input: R,
    options: SourceOptions,
) -> Result<TopicDiscovery, ProbeError> {
    let mut source = Source::new(input, options)?;
    let mut schemas = BTreeMap::<u16, SchemaDeclaration>::new();
    let mut channels = BTreeMap::<u16, ChannelDeclaration>::new();
    let mut message_counts = BTreeMap::<u16, u64>::new();

    while let Some(record) = source.next_probe()? {
        match record {
            ProbeRecord::Schema {
                id,
                name,
                encoding,
                data,
            } => insert_schema(
                &mut schemas,
                id,
                SchemaDeclaration {
                    name,
                    encoding,
                    data,
                },
            )?,
            ProbeRecord::Channel {
                id,
                schema_id,
                topic,
                message_encoding,
                metadata,
            } => insert_channel(
                &mut channels,
                id,
                ChannelDeclaration {
                    schema_id,
                    topic,
                    message_encoding,
                    metadata,
                },
            )?,
            ProbeRecord::Message { channel_id, .. } => {
                let count = message_counts.entry(channel_id).or_default();
                *count = count.checked_add(1).ok_or_else(|| ProbeError::Discovery {
                    message: format!("message count overflow for MCAP Channel {channel_id}"),
                })?;
            }
            ProbeRecord::Header { .. } | ProbeRecord::Metadata { .. } | ProbeRecord::DataEnd => {}
        }
    }

    for channel_id in message_counts.keys() {
        if !channels.contains_key(channel_id) {
            return Err(ProbeError::Discovery {
                message: format!("message references undefined MCAP Channel {channel_id}"),
            });
        }
    }

    let mut discovered = Vec::with_capacity(channels.len());
    for (channel_id, channel) in channels {
        let schema = match channel.schema_id {
            0 => None,
            schema_id => {
                let declaration = schemas
                    .get(&schema_id)
                    .ok_or_else(|| ProbeError::Discovery {
                        message: format!(
                            "MCAP Channel {channel_id} references undefined Schema {schema_id}"
                        ),
                    })?;
                Some(DiscoveredSchema {
                    id: schema_id,
                    name: declaration.name.clone(),
                    encoding: declaration.encoding.clone(),
                })
            }
        };
        let ros2_pointcloud2_candidate = schema.as_ref().is_some_and(|schema| {
            schema.name == ROS2_POINTCLOUD2_SCHEMA
                && schema.encoding == ROS2_SCHEMA_ENCODING
                && channel.message_encoding == ROS2_MESSAGE_ENCODING
        });
        discovered.push(DiscoveredChannel {
            channel_id,
            topic: channel.topic,
            message_encoding: channel.message_encoding,
            message_count: message_counts.get(&channel_id).copied().unwrap_or(0),
            schema,
            ros2_pointcloud2_candidate,
        });
    }
    discovered.sort_by(|left, right| {
        left.topic
            .cmp(&right.topic)
            .then(left.channel_id.cmp(&right.channel_id))
    });

    Ok(TopicDiscovery {
        channels: discovered,
    })
}

fn insert_schema(
    schemas: &mut BTreeMap<u16, SchemaDeclaration>,
    id: u16,
    declaration: SchemaDeclaration,
) -> Result<(), ProbeError> {
    match schemas.get(&id) {
        Some(existing) if existing != &declaration => Err(ProbeError::Discovery {
            message: format!("conflicting declarations for MCAP Schema {id}"),
        }),
        Some(_) => Ok(()),
        None => {
            schemas.insert(id, declaration);
            Ok(())
        }
    }
}

fn insert_channel(
    channels: &mut BTreeMap<u16, ChannelDeclaration>,
    id: u16,
    declaration: ChannelDeclaration,
) -> Result<(), ProbeError> {
    match channels.get(&id) {
        Some(existing) if existing != &declaration => Err(ProbeError::Discovery {
            message: format!("conflicting declarations for MCAP Channel {id}"),
        }),
        Some(_) => Ok(()),
        None => {
            channels.insert(id, declaration);
            Ok(())
        }
    }
}

/// Limits applied before bytes are admitted to the MCAP reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceOptions {
    /// Largest slice offered to the underlying `Read` implementation.
    pub read_chunk_bytes: usize,
    /// Largest emitted record body accepted.
    ///
    /// Compressed chunks may be larger in total because they are streamed, but
    /// each decompressed inner record is subject to this limit.
    pub max_record_bytes: usize,
}

impl Default for SourceOptions {
    fn default() -> Self {
        Self {
            read_chunk_bytes: 64 * 1024,
            max_record_bytes: 16 * 1024 * 1024,
        }
    }
}

impl SourceOptions {
    fn validate(self) -> Result<Self, ProbeError> {
        if self.read_chunk_bytes == 0 {
            return Err(ProbeError::InvalidOptions(
                "read_chunk_bytes must be greater than zero",
            ));
        }
        if self.max_record_bytes == 0 {
            return Err(ProbeError::InvalidOptions(
                "max_record_bytes must be greater than zero",
            ));
        }
        self.max_record_bytes
            .checked_add(RECORD_ENVELOPE_BYTES)
            .ok_or(ProbeError::InvalidOptions("max_record_bytes is too large"))?;
        Ok(self)
    }

    /// Conservative managed bytes retained while selecting one Topic message.
    pub fn selection_retained_bytes(self) -> Option<u64> {
        u64::try_from(self.read_chunk_bytes)
            .ok()?
            .checked_add(u64::try_from(self.max_record_bytes).ok()?)?
            .checked_add(DECLARATION_INDEX_BYTES)
    }
}

/// High-water marks observed while pulling records from a source.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ProbeStats {
    /// Bytes read from the source so far.
    pub bytes_read: u64,
    /// Largest buffer passed to a single `Read::read` call.
    pub max_read_chunk_bytes: usize,
    /// Largest parsed record body observed so far.
    pub max_record_bytes: usize,
    /// Largest logical content retained by one returned probe record.
    ///
    /// Records are returned one at a time. Bytes retained by callers that
    /// accumulate multiple records are outside the source's accounting.
    pub max_retained_bytes: usize,
    /// Number of records, including records not represented by [`ProbeRecord`].
    pub records_read: u64,
}

/// An incrementally discovered MCAP record relevant to source inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeRecord {
    Header {
        profile: String,
        library: String,
    },
    Metadata {
        name: String,
        metadata: BTreeMap<String, String>,
    },
    Schema {
        id: u16,
        name: String,
        encoding: String,
        data: Arc<[u8]>,
    },
    Channel {
        id: u16,
        schema_id: u16,
        topic: String,
        message_encoding: String,
        metadata: BTreeMap<String, String>,
    },
    Message {
        channel_id: u16,
        sequence: u32,
        log_time: u64,
        publish_time: u64,
        data: Arc<[u8]>,
    },
    DataEnd,
}

/// One encoded message selected from all MCAP Channels for a Topic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedMessage {
    channel_id: u16,
    sequence: u32,
    log_time: u64,
    publish_time: u64,
    ros2_pointcloud2_candidate: bool,
    data: Arc<[u8]>,
}

impl SelectedMessage {
    pub const fn channel_id(&self) -> u16 {
        self.channel_id
    }

    pub const fn sequence(&self) -> u32 {
        self.sequence
    }

    pub const fn log_time(&self) -> u64 {
        self.log_time
    }

    pub const fn publish_time(&self) -> u64 {
        self.publish_time
    }

    /// Whether this message's Channel and Schema declarations select the
    /// supported ROS 2 PointCloud2 CDR adapter.
    pub const fn is_ros2_pointcloud2_candidate(&self) -> bool {
        self.ros2_pointcloud2_candidate
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Transfer ownership of the encoded message bytes to a decoder.
    pub fn into_data(self) -> Arc<[u8]> {
        self.data
    }
}

/// A typed failure to read or select one message for a Topic.
#[derive(Debug)]
pub enum SelectionError {
    Probe(ProbeError),
    TopicNotFound {
        topic: String,
    },
    PointFrameNotFound {
        topic: String,
        selector: FrameSelector,
    },
}

impl SelectionError {
    /// Return the stable core failure category for process-level handling.
    pub const fn category(&self) -> ErrorCategory {
        match self {
            Self::Probe(error) => error.category(),
            Self::TopicNotFound { .. } | Self::PointFrameNotFound { .. } => ErrorCategory::NotFound,
        }
    }
}

impl fmt::Display for SelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Probe(error) => error.fmt(formatter),
            Self::TopicNotFound { topic } => write!(formatter, "Topic {topic:?} was not found"),
            Self::PointFrameNotFound { topic, selector } => {
                write!(
                    formatter,
                    "no Point Frame for Topic {topic:?} matched {selector:?}"
                )
            }
        }
    }
}

impl Error for SelectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Probe(error) => Some(error),
            Self::TopicNotFound { .. } | Self::PointFrameNotFound { .. } => None,
        }
    }
}

impl From<ProbeError> for SelectionError {
    fn from(error: ProbeError) -> Self {
        Self::Probe(error)
    }
}

impl ProbeRecord {
    /// Logical bytes held in this record's strings, maps, and byte payload.
    pub fn retained_bytes(&self) -> usize {
        fn map_bytes(map: &BTreeMap<String, String>) -> usize {
            map.iter().map(|(key, value)| key.len() + value.len()).sum()
        }

        match self {
            Self::Header { profile, library } => profile.len() + library.len(),
            Self::Metadata { name, metadata } => name.len() + map_bytes(metadata),
            Self::Schema {
                name,
                encoding,
                data,
                ..
            } => name.len() + encoding.len() + data.len(),
            Self::Channel {
                topic,
                message_encoding,
                metadata,
                ..
            } => topic.len() + message_encoding.len() + map_bytes(metadata),
            Self::Message { data, .. } => data.len(),
            Self::DataEnd => 0,
        }
    }
}

/// Deterministic container metadata produced without decoding message payloads.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct Info {
    pub profile: String,
    pub library: String,
    pub size_bytes: u64,
    pub message_count: u64,
    pub schema_count: u64,
    pub channel_count: u64,
    pub metadata_count: u64,
    pub start_log_time_ns: Option<u64>,
    pub end_log_time_ns: Option<u64>,
    pub duration_ns: Option<u64>,
}

impl fmt::Display for Info {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let profile = nonempty_or_none(&self.profile);
        let library = nonempty_or_none(&self.library);
        writeln!(formatter, "Profile: {profile}")?;
        writeln!(formatter, "Library: {library}")?;
        writeln!(formatter, "Size: {} bytes", self.size_bytes)?;
        writeln!(formatter, "Messages: {}", self.message_count)?;
        writeln!(formatter, "Schemas: {}", self.schema_count)?;
        writeln!(formatter, "Channels: {}", self.channel_count)?;
        writeln!(formatter, "Metadata records: {}", self.metadata_count)?;
        writeln!(
            formatter,
            "Start log time: {}",
            optional_nanoseconds(self.start_log_time_ns)
        )?;
        writeln!(
            formatter,
            "End log time: {}",
            optional_nanoseconds(self.end_log_time_ns)
        )?;
        write!(
            formatter,
            "Duration: {}",
            optional_nanoseconds(self.duration_ns)
        )
    }
}

fn nonempty_or_none(value: &str) -> &str {
    if value.is_empty() { "none" } else { value }
}

fn optional_nanoseconds(value: Option<u64>) -> String {
    value.map_or_else(|| "none".to_owned(), |value| format!("{value} ns"))
}

/// Inspect one MCAP Source using bounded synchronous reads.
pub fn inspect<R: Read + Seek>(input: R, options: SourceOptions) -> Result<Info, ProbeError> {
    let mut source = Source::new(input, options)?;
    let mut builder = InfoBuilder::default();

    while let Some(record) = source.next_probe()? {
        builder.observe(record);
    }

    Ok(builder.finish(source.stats().bytes_read))
}

#[derive(Default)]
struct InfoBuilder {
    profile: String,
    library: String,
    message_count: u64,
    schema_count: u64,
    channel_count: u64,
    metadata_count: u64,
    start_log_time_ns: Option<u64>,
    end_log_time_ns: Option<u64>,
    data_section_finished: bool,
}

impl InfoBuilder {
    fn observe(&mut self, record: ProbeRecord) {
        match record {
            ProbeRecord::Header { profile, library } => {
                self.profile = profile;
                self.library = library;
            }
            ProbeRecord::Message { log_time, .. } if !self.data_section_finished => {
                self.message_count = self.message_count.saturating_add(1);
                self.start_log_time_ns = Some(
                    self.start_log_time_ns
                        .map_or(log_time, |current| current.min(log_time)),
                );
                self.end_log_time_ns = Some(
                    self.end_log_time_ns
                        .map_or(log_time, |current| current.max(log_time)),
                );
            }
            ProbeRecord::Schema { .. } if !self.data_section_finished => {
                self.schema_count = self.schema_count.saturating_add(1);
            }
            ProbeRecord::Channel { .. } if !self.data_section_finished => {
                self.channel_count = self.channel_count.saturating_add(1);
            }
            ProbeRecord::Metadata { .. } if !self.data_section_finished => {
                self.metadata_count = self.metadata_count.saturating_add(1);
            }
            ProbeRecord::DataEnd => self.data_section_finished = true,
            _ => {}
        }
    }

    fn finish(self, size_bytes: u64) -> Info {
        let start = self.start_log_time_ns;
        let end = self.end_log_time_ns;
        let duration_ns = start.zip(end).map(|(start, end)| end.saturating_sub(start));

        Info {
            profile: self.profile,
            library: self.library,
            size_bytes,
            message_count: self.message_count,
            schema_count: self.schema_count,
            channel_count: self.channel_count,
            metadata_count: self.metadata_count,
            start_log_time_ns: start,
            end_log_time_ns: end,
            duration_ns,
        }
    }
}

/// Failure while driving or parsing the MCAP stream.
#[derive(Debug)]
pub enum ProbeError {
    InvalidOptions(&'static str),
    Io {
        offset: u64,
        source: io::Error,
    },
    Reader {
        offset: u64,
        record: u64,
        source: McapError,
    },
    Parse {
        offset: u64,
        record: u64,
        opcode: u8,
        source: McapError,
    },
    Discovery {
        message: String,
    },
}

impl fmt::Display for ProbeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOptions(message) => {
                write!(formatter, "invalid MCAP source options: {message}")
            }
            Self::Io { offset, source } => {
                write!(
                    formatter,
                    "failed to read MCAP at byte offset {offset}: {source}"
                )
            }
            Self::Reader {
                offset,
                record,
                source,
            } => write!(
                formatter,
                "invalid MCAP near byte offset {offset}, before record {record}: {source}"
            ),
            Self::Parse {
                offset,
                record,
                opcode,
                source,
            } => write!(
                formatter,
                "invalid MCAP record {record} (opcode 0x{opcode:02x}) near byte offset {offset}: {source}"
            ),
            Self::Discovery { message } => write!(formatter, "invalid MCAP: {message}"),
        }
    }
}

impl Error for ProbeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Reader { source, .. } | Self::Parse { source, .. } => Some(source),
            Self::InvalidOptions(_) | Self::Discovery { .. } => None,
        }
    }
}

impl ProbeError {
    /// Map adapter failures to the format-independent CLI error taxonomy.
    pub const fn category(&self) -> ErrorCategory {
        match self {
            Self::InvalidOptions(_) => ErrorCategory::Internal,
            Self::Io { .. } => ErrorCategory::Io,
            Self::Reader { .. } | Self::Parse { .. } | Self::Discovery { .. } => {
                ErrorCategory::InvalidData
            }
        }
    }
}

/// A synchronous pull source over the official MCAP sans-I/O reader.
pub struct Source<R> {
    input: R,
    reader: LinearReader,
    options: SourceOptions,
    start_offset: u64,
    offset: u64,
    stats: ProbeStats,
    finished: bool,
}

impl<R: Read + Seek> Source<R> {
    pub fn new(mut input: R, options: SourceOptions) -> Result<Self, ProbeError> {
        let options = options.validate()?;
        let offset = input
            .stream_position()
            .map_err(|source| ProbeError::Io { offset: 0, source })?;
        let reader_options = LinearReaderOptions::default()
            .with_check_finishes_after_end_magic(true)
            .with_validate_chunk_crcs(true)
            .with_validate_data_section_crc(true)
            .with_validate_summary_section_crc(true)
            .with_record_length_limit(options.max_record_bytes);

        Ok(Self {
            input,
            reader: LinearReader::new_with_options(reader_options),
            options,
            start_offset: offset,
            offset,
            stats: ProbeStats::default(),
            finished: false,
        })
    }

    pub fn stats(&self) -> ProbeStats {
        self.stats
    }

    pub fn into_inner(self) -> R {
        self.input
    }

    /// Pull the next metadata, schema, Channel, or message record.
    ///
    /// Structural records used only by the container are consumed internally.
    pub fn next_probe(&mut self) -> Result<Option<ProbeRecord>, ProbeError> {
        if self.finished {
            return Ok(None);
        }

        loop {
            let event = match self.reader.next_event() {
                Some(Ok(event)) => event,
                Some(Err(source)) => {
                    return Err(ProbeError::Reader {
                        offset: self.offset,
                        record: self.stats.records_read,
                        source,
                    });
                }
                None => {
                    self.finished = true;
                    return Ok(None);
                }
            };

            match event {
                LinearReadEvent::ReadRequest(requested) => self.read_more(requested)?,
                LinearReadEvent::Record { data, opcode } => {
                    self.stats.records_read += 1;
                    self.stats.max_record_bytes = self.stats.max_record_bytes.max(data.len());
                    let record_number = self.stats.records_read - 1;
                    let parsed =
                        parse_record(opcode, data).map_err(|source| ProbeError::Parse {
                            offset: self.offset,
                            record: record_number,
                            opcode,
                            source,
                        })?;
                    if let Some(record) = own_probe_record(parsed) {
                        self.stats.max_retained_bytes =
                            self.stats.max_retained_bytes.max(record.retained_bytes());
                        return Ok(Some(record));
                    }
                }
            }
        }
    }

    fn read_more(&mut self, requested: usize) -> Result<(), ProbeError> {
        let amount = requested.min(self.options.read_chunk_bytes);
        let read = self
            .input
            .read(self.reader.insert(amount))
            .map_err(|source| ProbeError::Io {
                offset: self.offset,
                source,
            })?;
        self.reader.notify_read(read);
        self.stats.max_read_chunk_bytes = self.stats.max_read_chunk_bytes.max(amount);
        self.stats.bytes_read = self.stats.bytes_read.saturating_add(read as u64);
        self.offset = self.offset.saturating_add(read as u64);
        Ok(())
    }

    fn restart(&mut self) -> Result<(), ProbeError> {
        self.input
            .seek(SeekFrom::Start(self.start_offset))
            .map_err(|source| ProbeError::Io {
                offset: self.offset,
                source,
            })?;
        self.reader = LinearReader::new_with_options(
            LinearReaderOptions::default()
                .with_check_finishes_after_end_magic(true)
                .with_validate_chunk_crcs(true)
                .with_validate_data_section_crc(true)
                .with_validate_summary_section_crc(true)
                .with_record_length_limit(self.options.max_record_bytes),
        );
        self.offset = self.start_offset;
        self.finished = false;
        Ok(())
    }
}

/// Select one encoded message after matching all MCAP Channels for `topic`.
///
/// Index selection is a single pull pass. Time selection first discovers the
/// recording-wide earliest log time, then restarts the seekable source and
/// returns the first matching message at or after the requested relative time.
/// Both passes retain only one probe record plus a fixed-size Channel-ID set.
pub fn select_topic_message<R: Read + Seek>(
    source: &mut Source<R>,
    topic: &str,
    selector: FrameSelector,
) -> Result<SelectedMessage, SelectionError> {
    source.restart()?;
    match selector {
        FrameSelector::Index(index) => select_by_index(source, topic, index, selector),
        FrameSelector::At(duration) => {
            let (topic_found, recording_start) = discover_recording_start(source, topic)?;
            if !topic_found {
                return Err(SelectionError::TopicNotFound {
                    topic: topic.to_owned(),
                });
            }
            let Some(relative_nanos) = u64::try_from(duration.as_nanos()).ok() else {
                return Err(point_frame_not_found(topic, selector));
            };
            let Some(threshold) =
                recording_start.and_then(|start| start.checked_add(relative_nanos))
            else {
                return Err(point_frame_not_found(topic, selector));
            };
            source.restart()?;
            select_at_or_after(source, topic, threshold, selector)
        }
    }
}

fn select_by_index<R: Read + Seek>(
    source: &mut Source<R>,
    topic: &str,
    index: u64,
    selector: FrameSelector,
) -> Result<SelectedMessage, SelectionError> {
    let mut declarations = vec![0_u8; usize::from(u16::MAX) + 1];
    let mut topic_found = false;
    let mut matching_index = 0_u64;
    while let Some(record) = source.next_probe()? {
        match record {
            ProbeRecord::Schema {
                id, name, encoding, ..
            } => {
                declarations[usize::from(id)] =
                    u8::from(name == ROS2_POINTCLOUD2_SCHEMA && encoding == ROS2_SCHEMA_ENCODING);
            }
            ProbeRecord::Channel {
                id,
                schema_id,
                topic: channel_topic,
                message_encoding,
                ..
            } => {
                let matches = channel_topic == topic;
                let schema_matches = declarations[usize::from(schema_id)] & 1 != 0;
                declarations[usize::from(id)] = (declarations[usize::from(id)] & 1)
                    | (u8::from(matches) << 1)
                    | (u8::from(
                        matches && schema_matches && message_encoding == ROS2_MESSAGE_ENCODING,
                    ) << 2);
                topic_found |= matches;
            }
            ProbeRecord::Message {
                channel_id,
                sequence,
                log_time,
                publish_time,
                data,
            } if declarations[usize::from(channel_id)] & 2 != 0 => {
                if matching_index == index {
                    return Ok(SelectedMessage {
                        channel_id,
                        sequence,
                        log_time,
                        publish_time,
                        ros2_pointcloud2_candidate: declarations[usize::from(channel_id)] & 4 != 0,
                        data,
                    });
                }
                matching_index = matching_index.saturating_add(1);
            }
            _ => {}
        }
    }
    if topic_found {
        Err(point_frame_not_found(topic, selector))
    } else {
        Err(SelectionError::TopicNotFound {
            topic: topic.to_owned(),
        })
    }
}

fn discover_recording_start<R: Read + Seek>(
    source: &mut Source<R>,
    topic: &str,
) -> Result<(bool, Option<u64>), ProbeError> {
    let mut topic_found = false;
    let mut recording_start: Option<u64> = None;
    while let Some(record) = source.next_probe()? {
        match record {
            ProbeRecord::Channel {
                topic: channel_topic,
                ..
            } => topic_found |= channel_topic == topic,
            ProbeRecord::Message { log_time, .. } => {
                recording_start =
                    Some(recording_start.map_or(log_time, |start| start.min(log_time)));
            }
            _ => {}
        }
    }
    Ok((topic_found, recording_start))
}

fn select_at_or_after<R: Read + Seek>(
    source: &mut Source<R>,
    topic: &str,
    threshold: u64,
    selector: FrameSelector,
) -> Result<SelectedMessage, SelectionError> {
    let mut declarations = vec![0_u8; usize::from(u16::MAX) + 1];
    while let Some(record) = source.next_probe()? {
        match record {
            ProbeRecord::Schema {
                id, name, encoding, ..
            } => {
                declarations[usize::from(id)] =
                    u8::from(name == ROS2_POINTCLOUD2_SCHEMA && encoding == ROS2_SCHEMA_ENCODING);
            }
            ProbeRecord::Channel {
                id,
                schema_id,
                topic: channel_topic,
                message_encoding,
                ..
            } => {
                let matches = channel_topic == topic;
                let schema_matches = declarations[usize::from(schema_id)] & 1 != 0;
                declarations[usize::from(id)] = (declarations[usize::from(id)] & 1)
                    | (u8::from(matches) << 1)
                    | (u8::from(
                        matches && schema_matches && message_encoding == ROS2_MESSAGE_ENCODING,
                    ) << 2);
            }
            ProbeRecord::Message {
                channel_id,
                sequence,
                log_time,
                publish_time,
                data,
            } if declarations[usize::from(channel_id)] & 2 != 0 && log_time >= threshold => {
                return Ok(SelectedMessage {
                    channel_id,
                    sequence,
                    log_time,
                    publish_time,
                    ros2_pointcloud2_candidate: declarations[usize::from(channel_id)] & 4 != 0,
                    data,
                });
            }
            _ => {}
        }
    }
    Err(point_frame_not_found(topic, selector))
}

fn point_frame_not_found(topic: &str, selector: FrameSelector) -> SelectionError {
    SelectionError::PointFrameNotFound {
        topic: topic.to_owned(),
        selector,
    }
}

fn own_probe_record(record: Record<'_>) -> Option<ProbeRecord> {
    match record {
        Record::Header(header) => Some(ProbeRecord::Header {
            profile: header.profile,
            library: header.library,
        }),
        Record::Metadata(metadata) => Some(ProbeRecord::Metadata {
            name: metadata.name,
            metadata: metadata.metadata,
        }),
        Record::Schema { header, data } => Some(ProbeRecord::Schema {
            id: header.id,
            name: header.name,
            encoding: header.encoding,
            data: Arc::from(data.as_ref()),
        }),
        Record::Channel(channel) => Some(ProbeRecord::Channel {
            id: channel.id,
            schema_id: channel.schema_id,
            topic: channel.topic,
            message_encoding: channel.message_encoding,
            metadata: channel.metadata,
        }),
        Record::Message { header, data } => Some(ProbeRecord::Message {
            channel_id: header.channel_id,
            sequence: header.sequence,
            log_time: header.log_time,
            publish_time: header.publish_time,
            data: Arc::from(data.as_ref()),
        }),
        Record::DataEnd(_) => Some(ProbeRecord::DataEnd),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        io::{Cursor, Read, Seek, SeekFrom},
        sync::Arc,
    };

    use ::mcap::{
        Compression,
        records::{MessageHeader, Metadata},
        write::{WriteOptions, Writer},
    };

    use super::{
        Info, ProbeError, ProbeRecord, SchemaDeclaration, Source, SourceOptions, discover_topics,
        insert_schema, inspect,
    };

    fn recording(compression: Option<Compression>, messages: &[&[u8]]) -> Vec<u8> {
        recording_with_summary(compression, messages, true)
    }

    fn recording_with_summary(
        compression: Option<Compression>,
        messages: &[&[u8]],
        emit_summary: bool,
    ) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let options = WriteOptions::new()
            .profile("ros2")
            .library("pcx-test")
            .compression(compression)
            .chunk_size(Some(32))
            .emit_summary_records(emit_summary)
            .emit_summary_offsets(emit_summary);
        let mut writer = Writer::with_options(cursor, options).expect("writer should start");
        let schema = writer
            .add_schema("sensor_msgs/msg/PointCloud2", "ros2msg", b"schema")
            .expect("schema should be added");
        let channel = writer
            .add_channel(schema, "/points", "cdr", &BTreeMap::new())
            .expect("channel should be added");
        writer
            .write_metadata(&Metadata {
                name: "robot".into(),
                metadata: BTreeMap::from([("site".into(), "lab".into())]),
            })
            .expect("metadata should be written");
        for (sequence, data) in messages.iter().enumerate() {
            writer
                .write_to_known_channel(
                    &MessageHeader {
                        channel_id: channel,
                        sequence: sequence as u32,
                        log_time: 100 + sequence as u64,
                        publish_time: 200 + sequence as u64,
                    },
                    data,
                )
                .expect("message should be written");
        }
        writer.finish().expect("writer should finish");
        writer.into_inner().into_inner()
    }

    fn empty_recording() -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let options = WriteOptions::new().profile("ros2").library("pcx-test");
        let mut writer = Writer::with_options(cursor, options).expect("writer should start");
        writer.finish().expect("writer should finish");
        writer.into_inner().into_inner()
    }

    fn probes(bytes: Vec<u8>, read_chunk_bytes: usize) -> Vec<ProbeRecord> {
        let mut source = Source::new(
            Cursor::new(bytes),
            SourceOptions {
                read_chunk_bytes,
                max_record_bytes: 1024 * 1024,
            },
        )
        .expect("source should open");
        let mut records = Vec::new();
        while let Some(record) = source.next_probe().expect("recording should parse") {
            records.push(record);
        }
        records
    }

    fn duplicate_topic_recording() -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = Writer::with_options(
            cursor,
            WriteOptions::new()
                .profile("ros2")
                .library("pcx-test")
                .compression(None),
        )
        .expect("writer should start");
        let point_schema_a = writer
            .add_schema("sensor_msgs/msg/PointCloud2", "ros2msg", b"schema-a")
            .expect("schema should be added");
        let point_schema_b = writer
            .add_schema("sensor_msgs/msg/PointCloud2", "ros2msg", b"schema-b")
            .expect("schema should be added");
        let other_schema = writer
            .add_schema("sensor_msgs/msg/Image", "ros2msg", b"schema-c")
            .expect("schema should be added");
        let later = writer
            .add_channel(point_schema_a, "/z", "cdr", &BTreeMap::new())
            .expect("channel should be added");
        let duplicate_a = writer
            .add_channel(point_schema_b, "/points", "cdr", &BTreeMap::new())
            .expect("channel should be added");
        let duplicate_b = writer
            .add_channel(other_schema, "/points", "cdr", &BTreeMap::new())
            .expect("channel should be added");
        for channel_id in [duplicate_b, duplicate_a, duplicate_a, later] {
            writer
                .write_to_known_channel(
                    &MessageHeader {
                        channel_id,
                        sequence: 0,
                        log_time: 0,
                        publish_time: 0,
                    },
                    b"payload",
                )
                .expect("message should be written");
        }
        writer.finish().expect("writer should finish");
        writer.into_inner().into_inner()
    }

    #[test]
    fn probes_metadata_schema_channel_and_messages_incrementally() {
        let records = probes(recording(None, &[b"one", b"two"]), 3);

        assert!(records.iter().any(|record| matches!(
            record,
            ProbeRecord::Header { profile, library }
                if profile == "ros2" && library == "pcx-test"
        )));
        assert!(records.iter().any(|record| matches!(
            record,
            ProbeRecord::Metadata { name, metadata }
                if name == "robot" && metadata.get("site").map(String::as_str) == Some("lab")
        )));
        assert!(records.iter().any(|record| matches!(
            record,
            ProbeRecord::Schema { name, data, .. }
                if name == "sensor_msgs/msg/PointCloud2" && data.as_ref() == b"schema"
        )));
        assert!(records.iter().any(|record| matches!(
            record,
            ProbeRecord::Channel { topic, message_encoding, .. }
                if topic == "/points" && message_encoding == "cdr"
        )));
        let payloads: Vec<&[u8]> = records
            .iter()
            .filter_map(|record| match record {
                ProbeRecord::Message { data, .. } => Some(data.as_ref()),
                _ => None,
            })
            .collect();
        assert_eq!(payloads, [b"one".as_slice(), b"two".as_slice()]);
    }

    #[test]
    fn info_reports_counts_and_deterministic_log_time_bounds() {
        let bytes = recording(None, &[b"one", b"two"]);
        let expected_size = bytes.len() as u64;

        let info = inspect(Cursor::new(bytes), SourceOptions::default())
            .expect("recording should inspect");

        assert_eq!(
            info,
            Info {
                profile: "ros2".into(),
                library: "pcx-test".into(),
                size_bytes: expected_size,
                message_count: 2,
                schema_count: 1,
                channel_count: 1,
                metadata_count: 1,
                start_log_time_ns: Some(100),
                end_log_time_ns: Some(101),
                duration_ns: Some(1),
            }
        );
        assert_eq!(
            info.to_string(),
            "Profile: ros2\nLibrary: pcx-test\nSize: ".to_owned()
                + &expected_size.to_string()
                + " bytes\nMessages: 2\nSchemas: 1\nChannels: 1\nMetadata records: 1\nStart log time: 100 ns\nEnd log time: 101 ns\nDuration: 1 ns"
        );
    }

    #[test]
    fn info_does_not_require_a_summary_section() {
        let bytes = recording_with_summary(None, &[b"one", b"two"], false);

        let info = inspect(Cursor::new(bytes), SourceOptions::default())
            .expect("recording without summary should inspect");

        assert_eq!(info.message_count, 2);
        assert_eq!(info.schema_count, 1);
        assert_eq!(info.channel_count, 1);
        assert_eq!(info.metadata_count, 1);
        assert_eq!(info.start_log_time_ns, Some(100));
        assert_eq!(info.end_log_time_ns, Some(101));
    }

    #[test]
    fn valid_empty_container_has_zero_counts_and_no_time_range() {
        let bytes = empty_recording();
        let expected_size = bytes.len() as u64;

        let info = inspect(Cursor::new(bytes), SourceOptions::default())
            .expect("empty container should inspect");

        assert_eq!(info.message_count, 0);
        assert_eq!(info.schema_count, 0);
        assert_eq!(info.channel_count, 0);
        assert_eq!(info.metadata_count, 0);
        assert_eq!(info.size_bytes, expected_size);
        assert_eq!(info.start_log_time_ns, None);
        assert_eq!(info.end_log_time_ns, None);
        assert_eq!(info.duration_ns, None);
    }

    #[test]
    fn empty_bytes_are_rejected_as_malformed_mcap() {
        let error = inspect(Cursor::new(Vec::new()), SourceOptions::default())
            .expect_err("zero-byte input must fail");

        assert!(matches!(error, ProbeError::Reader { .. }));
    }

    #[test]
    fn returned_message_owns_its_payload() {
        let bytes = recording(None, &[b"owned"]);
        let payload = {
            let records = probes(bytes, 2);
            records.into_iter().find_map(|record| match record {
                ProbeRecord::Message { data, .. } => Some(data),
                _ => None,
            })
        }
        .expect("message should be present");

        assert_eq!(payload.as_ref(), b"owned");
    }

    #[test]
    fn zstd_and_lz4_chunks_are_probed_with_the_same_feature_set() {
        for compression in [Compression::Zstd, Compression::Lz4] {
            let records = probes(recording(Some(compression), &[b"compressed"]), 5);
            assert!(records.iter().any(|record| matches!(
                record,
                ProbeRecord::Message { data, .. } if data.as_ref() == b"compressed"
            )));
        }
    }

    #[test]
    fn truncation_is_contextual_and_does_not_panic() {
        let mut bytes = recording(Some(Compression::Zstd), &[b"payload"]);
        bytes.truncate(bytes.len() - 12);
        let result = std::panic::catch_unwind(|| {
            let mut source = Source::new(Cursor::new(bytes), SourceOptions::default())
                .expect("source should open");
            loop {
                match source.next_probe() {
                    Ok(Some(_)) => {}
                    other => break other,
                }
            }
        });

        let error = result
            .expect("malformed input must not panic")
            .expect_err("truncation must fail");
        assert!(matches!(error, ProbeError::Reader { .. }));
        assert!(error.to_string().contains("byte offset"));
        assert!(error.to_string().contains("record"));
    }

    #[test]
    fn corrupt_magic_is_contextual_and_does_not_panic() {
        let mut bytes = recording(None, &[b"payload"]);
        bytes[0] = 0;
        let mut source =
            Source::new(Cursor::new(bytes), SourceOptions::default()).expect("source should open");

        let error = source.next_probe().expect_err("bad magic must fail");
        assert!(matches!(error, ProbeError::Reader { .. }));
        assert!(error.to_string().contains("byte offset"));
    }

    #[test]
    fn corrupt_record_body_is_contextual_and_does_not_panic() {
        let mut bytes = Vec::from(::mcap::MAGIC);
        bytes.push(::mcap::records::op::SCHEMA);
        bytes.extend_from_slice(&1_u64.to_le_bytes());
        bytes.push(0);
        let result = std::panic::catch_unwind(|| {
            let mut source = Source::new(Cursor::new(bytes), SourceOptions::default())
                .expect("source should open");
            source.next_probe()
        });

        let error = result
            .expect("malformed record must not panic")
            .expect_err("malformed record must fail");
        assert!(matches!(error, ProbeError::Parse { opcode: 3, .. }));
        assert!(error.to_string().contains("record 0"));
        assert!(error.to_string().contains("opcode 0x03"));
    }

    #[test]
    fn record_limit_is_enforced_before_record_body_allocation() {
        let mut bytes = Vec::from(::mcap::MAGIC);
        bytes.push(::mcap::records::op::SCHEMA);
        bytes.extend_from_slice(&100_u64.to_le_bytes());
        let mut source = Source::new(
            Cursor::new(bytes),
            SourceOptions {
                read_chunk_bytes: 4,
                max_record_bytes: 16,
            },
        )
        .expect("source should open");

        let error = source.next_probe().expect_err("oversized record must fail");
        assert!(matches!(error, ProbeError::Reader { .. }));
        assert!(error.to_string().contains("100"));
    }

    #[derive(Debug)]
    struct TrackingCursor {
        inner: Cursor<Vec<u8>>,
        largest_read_buffer: usize,
    }

    impl Read for TrackingCursor {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.largest_read_buffer = self.largest_read_buffer.max(buffer.len());
            self.inner.read(buffer)
        }
    }

    impl Seek for TrackingCursor {
        fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
            self.inner.seek(position)
        }
    }

    #[test]
    fn read_buffers_stay_within_the_configured_maximum() {
        let input = TrackingCursor {
            inner: Cursor::new(recording(
                Some(Compression::Lz4),
                &vec![b"0123456789".as_slice(); 100],
            )),
            largest_read_buffer: 0,
        };
        let mut source = Source::new(
            input,
            SourceOptions {
                read_chunk_bytes: 7,
                max_record_bytes: 1024,
            },
        )
        .expect("source should open");
        while source
            .next_probe()
            .expect("recording should parse")
            .is_some()
        {}
        let stats = source.stats();
        let input = source.into_inner();

        assert!(input.largest_read_buffer <= 7);
        assert!(stats.max_read_chunk_bytes <= 7);
        assert!(stats.max_record_bytes <= 1024);
        assert_eq!(stats.max_retained_bytes, 40);
        assert!(stats.max_retained_bytes <= 1024);
    }

    #[test]
    fn discovers_duplicate_topic_and_schema_names_as_distinct_channels() {
        let discovery = discover_topics(
            Cursor::new(duplicate_topic_recording()),
            SourceOptions::default(),
        )
        .expect("recording should be discovered");

        let channels = discovery.channels();
        assert_eq!(channels.len(), 3);
        assert_eq!(channels[0].topic, "/points");
        assert_eq!(channels[1].topic, "/points");
        assert!(channels[0].channel_id < channels[1].channel_id);
        assert_eq!(channels[0].message_count, 2);
        assert_eq!(channels[1].message_count, 1);
        assert!(channels[0].ros2_pointcloud2_candidate);
        assert!(!channels[1].ros2_pointcloud2_candidate);
        assert_eq!(channels[2].topic, "/z");
        assert_eq!(channels[2].message_count, 1);
    }

    #[test]
    fn discovers_an_empty_container_without_inventing_topics() {
        let cursor = Cursor::new(Vec::new());
        let mut writer = Writer::with_options(
            cursor,
            WriteOptions::new()
                .profile("ros2")
                .library("pcx-test")
                .compression(None),
        )
        .expect("writer should start");
        writer.finish().expect("writer should finish");
        let bytes = writer.into_inner().into_inner();

        let discovery = discover_topics(Cursor::new(bytes), SourceOptions::default())
            .expect("empty container should be valid");

        assert!(discovery.channels().is_empty());
    }

    #[test]
    fn topic_discovery_rejects_malformed_mcap_without_panicking() {
        let result = std::panic::catch_unwind(|| {
            discover_topics(
                Cursor::new(b"not an MCAP".to_vec()),
                SourceOptions::default(),
            )
        });

        let error = result
            .expect("malformed discovery must not panic")
            .expect_err("malformed MCAP must fail");
        assert!(matches!(error, ProbeError::Reader { .. }));
    }

    #[test]
    fn topic_discovery_rejects_conflicting_duplicate_schema_ids() {
        let mut schemas = BTreeMap::new();
        insert_schema(
            &mut schemas,
            7,
            SchemaDeclaration {
                name: "sensor_msgs/msg/PointCloud2".into(),
                encoding: "ros2msg".into(),
                data: Arc::from(b"first".as_slice()),
            },
        )
        .expect("first declaration should be accepted");

        let error = insert_schema(
            &mut schemas,
            7,
            SchemaDeclaration {
                name: "sensor_msgs/msg/PointCloud2".into(),
                encoding: "ros2msg".into(),
                data: Arc::from(b"conflict".as_slice()),
            },
        )
        .expect_err("conflicting declaration should fail");

        assert!(matches!(error, ProbeError::Discovery { .. }));
        assert!(error.to_string().contains("MCAP Schema 7"));
    }
}
