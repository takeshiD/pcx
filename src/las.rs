//! Bounded synchronous LAS and LAZ adapters for the common point schema.
//!
//! Coordinates are decoded as semantic `f64` fields. Their integer LAS
//! representation is described by [`SpatialMetadata::scale`] and
//! [`SpatialMetadata::offset`]. Extra Bytes remain one ordered `u8` field so
//! their bytes and the descriptor VLRs can be preserved without guessing at
//! vendor-specific meanings.

use crate::core::point::{
    PointBatch, PointColumn, PointDimensions, PointField, PointFieldSemantic, PointFrameMetadata,
    PointSchema, PrimitiveType, Timestamp,
};
use crate::core::{FidelityLoss, LossPolicy};
use las::point::{Classification, Format, ScanDirection};
use las::raw::point::Waveform;
use las::{Builder, Color, Header, Point};
use std::error;
use std::fmt;
use std::io::{Read, Seek, Write};
use std::mem::size_of;
use std::num::NonZeroUsize;
use std::sync::Arc;

/// Default maximum number of decoded points retained by one read call.
pub const DEFAULT_MAX_POINTS_PER_BATCH: usize = 50_000;

/// LAS payload encoding selected independently from a path extension.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Encoding {
    Las,
    Laz,
}

/// Hard bounds for adapter-owned input and output buffers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadLimits {
    pub max_points_per_batch: NonZeroUsize,
    pub memory_limit_bytes: usize,
}

/// Hard bounds for writer-owned codec state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WriteLimits {
    pub max_points: u64,
    pub memory_limit_bytes: usize,
}

impl WriteLimits {
    pub const fn new(max_points: u64, memory_limit_bytes: usize) -> Self {
        Self {
            max_points,
            memory_limit_bytes,
        }
    }
}

impl ReadLimits {
    pub fn new(max_points_per_batch: usize, memory_limit_bytes: usize) -> Result<Self, Error> {
        let max_points_per_batch =
            NonZeroUsize::new(max_points_per_batch).ok_or(Error::ZeroBatchPointLimit)?;
        Ok(Self {
            max_points_per_batch,
            memory_limit_bytes,
        })
    }
}

/// Coordinate transforms and format records which accompany a Static Cloud.
///
/// Keeping the complete header preserves CRS VLRs/EVLRs, Extra Bytes
/// descriptors, GUID, creation metadata, GPS time interpretation, and unknown
/// records when a cloud is read and written again.
#[derive(Clone, Debug, PartialEq)]
pub struct SpatialMetadata {
    header: Header,
}

impl SpatialMetadata {
    pub fn from_header(header: Header) -> Self {
        Self { header }
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn scale(&self) -> [f64; 3] {
        let transforms = self.header.transforms();
        [transforms.x.scale, transforms.y.scale, transforms.z.scale]
    }

    pub fn offset(&self) -> [f64; 3] {
        let transforms = self.header.transforms();
        [
            transforms.x.offset,
            transforms.y.offset,
            transforms.z.offset,
        ]
    }

    /// Returns WKT and GeoTIFF CRS records without interpreting their bytes.
    pub fn crs_records(&self) -> impl Iterator<Item = &las::Vlr> {
        self.header.all_vlrs().filter(|vlr| vlr.is_crs())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FieldKind {
    X,
    Y,
    Z,
    Intensity,
    ReturnNumber,
    NumberOfReturns,
    ScanDirection,
    EdgeOfFlightLine,
    Classification,
    Synthetic,
    KeyPoint,
    Withheld,
    Overlap,
    ScannerChannel,
    ScanAngle,
    UserData,
    PointSourceId,
    GpsTime,
    Red,
    Green,
    Blue,
    Nir,
    WavePacketDescriptorIndex,
    WaveformDataOffset,
    WaveformPacketSize,
    ReturnPointWaveformLocation,
    WaveformXt,
    WaveformYt,
    WaveformZt,
    ExtraBytes,
}

#[derive(Clone, Debug)]
struct SchemaMapping {
    schema: Arc<PointSchema>,
    fields: Box<[FieldKind]>,
    bytes_per_point: usize,
}

impl SchemaMapping {
    fn for_format(format: &Format) -> Result<Self, Error> {
        let mut definitions = vec![
            (
                FieldKind::X,
                "x",
                PrimitiveType::F64,
                1,
                Some(PointFieldSemantic::X),
            ),
            (
                FieldKind::Y,
                "y",
                PrimitiveType::F64,
                1,
                Some(PointFieldSemantic::Y),
            ),
            (
                FieldKind::Z,
                "z",
                PrimitiveType::F64,
                1,
                Some(PointFieldSemantic::Z),
            ),
            (
                FieldKind::Intensity,
                "intensity",
                PrimitiveType::U16,
                1,
                Some(PointFieldSemantic::Intensity),
            ),
            (
                FieldKind::ReturnNumber,
                "return_number",
                PrimitiveType::U8,
                1,
                None,
            ),
            (
                FieldKind::NumberOfReturns,
                "number_of_returns",
                PrimitiveType::U8,
                1,
                None,
            ),
            (
                FieldKind::ScanDirection,
                "scan_direction",
                PrimitiveType::U8,
                1,
                None,
            ),
            (
                FieldKind::EdgeOfFlightLine,
                "edge_of_flight_line",
                PrimitiveType::U8,
                1,
                None,
            ),
            (
                FieldKind::Classification,
                "classification",
                PrimitiveType::U8,
                1,
                Some(PointFieldSemantic::Classification),
            ),
            (
                FieldKind::Synthetic,
                "synthetic",
                PrimitiveType::U8,
                1,
                None,
            ),
            (FieldKind::KeyPoint, "key_point", PrimitiveType::U8, 1, None),
            (FieldKind::Withheld, "withheld", PrimitiveType::U8, 1, None),
            (FieldKind::Overlap, "overlap", PrimitiveType::U8, 1, None),
            (
                FieldKind::ScannerChannel,
                "scanner_channel",
                PrimitiveType::U8,
                1,
                None,
            ),
            (
                FieldKind::ScanAngle,
                "scan_angle",
                PrimitiveType::F32,
                1,
                None,
            ),
            (FieldKind::UserData, "user_data", PrimitiveType::U8, 1, None),
            (
                FieldKind::PointSourceId,
                "point_source_id",
                PrimitiveType::U16,
                1,
                None,
            ),
        ];
        if format.has_gps_time {
            definitions.push((
                FieldKind::GpsTime,
                "gps_time",
                PrimitiveType::F64,
                1,
                Some(PointFieldSemantic::Timestamp),
            ));
        }
        if format.has_color {
            definitions.extend([
                (FieldKind::Red, "red", PrimitiveType::U16, 1, None),
                (FieldKind::Green, "green", PrimitiveType::U16, 1, None),
                (FieldKind::Blue, "blue", PrimitiveType::U16, 1, None),
            ]);
        }
        if format.has_nir {
            definitions.push((FieldKind::Nir, "nir", PrimitiveType::U16, 1, None));
        }
        if format.has_waveform {
            definitions.extend([
                (
                    FieldKind::WavePacketDescriptorIndex,
                    "wave_packet_descriptor_index",
                    PrimitiveType::U8,
                    1,
                    None,
                ),
                (
                    FieldKind::WaveformDataOffset,
                    "waveform_data_offset",
                    PrimitiveType::U64,
                    1,
                    None,
                ),
                (
                    FieldKind::WaveformPacketSize,
                    "waveform_packet_size",
                    PrimitiveType::U32,
                    1,
                    None,
                ),
                (
                    FieldKind::ReturnPointWaveformLocation,
                    "return_point_waveform_location",
                    PrimitiveType::F32,
                    1,
                    None,
                ),
                (
                    FieldKind::WaveformXt,
                    "waveform_x_t",
                    PrimitiveType::F32,
                    1,
                    None,
                ),
                (
                    FieldKind::WaveformYt,
                    "waveform_y_t",
                    PrimitiveType::F32,
                    1,
                    None,
                ),
                (
                    FieldKind::WaveformZt,
                    "waveform_z_t",
                    PrimitiveType::F32,
                    1,
                    None,
                ),
            ]);
        }
        if format.extra_bytes > 0 {
            definitions.push((
                FieldKind::ExtraBytes,
                "las_extra_bytes",
                PrimitiveType::U8,
                usize::from(format.extra_bytes),
                None,
            ));
        }

        let bytes_per_point = definitions.iter().try_fold(0usize, |total, definition| {
            total
                .checked_add(
                    definition
                        .2
                        .size()
                        .checked_mul(definition.3)
                        .ok_or(Error::MemoryEstimateOverflow)?,
                )
                .ok_or(Error::MemoryEstimateOverflow)
        })?;
        let fields = definitions
            .iter()
            .map(|definition| definition.0)
            .collect::<Vec<_>>();
        let schema = definitions
            .into_iter()
            .map(|(_, name, primitive, count, semantic)| {
                PointField::new(name, primitive, count, semantic)
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(Error::Schema)?;
        Ok(Self {
            schema: Arc::new(PointSchema::new(schema).map_err(Error::Schema)?),
            fields: fields.into_boxed_slice(),
            bytes_per_point,
        })
    }
}

/// A synchronous reader which returns one bounded common-schema batch at a time.
pub struct Reader {
    inner: las::Reader,
    metadata: Arc<SpatialMetadata>,
    mapping: SchemaMapping,
    limits: ReadLimits,
    managed_peak_bytes: usize,
}

impl Reader {
    pub fn new<R>(mut input: R, limits: ReadLimits) -> Result<Self, Error>
    where
        R: Read + Seek + Send + Sync + 'static,
    {
        let header = Header::new(&mut input).map_err(Error::Las)?;
        let mapping = SchemaMapping::for_format(header.point_format())?;
        let laz_chunk_table_bytes = laz_chunk_table_bytes(&mut input, &header)?;
        let managed_peak_bytes = managed_peak(&header, &mapping, limits, laz_chunk_table_bytes)?;
        if managed_peak_bytes > limits.memory_limit_bytes {
            return Err(Error::MemoryLimitExceeded {
                required: managed_peak_bytes,
                limit: limits.memory_limit_bytes,
            });
        }
        input.rewind().map_err(Error::Io)?;
        let inner = las::Reader::new(input).map_err(Error::Las)?;
        let metadata = Arc::new(SpatialMetadata::from_header(header));
        Ok(Self {
            inner,
            metadata,
            mapping,
            limits,
            managed_peak_bytes,
        })
    }

    pub fn schema(&self) -> &PointSchema {
        &self.mapping.schema
    }

    pub fn metadata(&self) -> &SpatialMetadata {
        &self.metadata
    }

    pub const fn managed_peak_bytes(&self) -> usize {
        self.managed_peak_bytes
    }

    /// Returns `None` only after all declared points have been consumed.
    pub fn next_batch(&mut self) -> Result<Option<PointBatch>, Error> {
        let point_data = self
            .inner
            .read_points(self.limits.max_points_per_batch.get() as u64)
            .map_err(Error::Las)?;
        if point_data.is_empty() {
            return Ok(None);
        }
        let point_count = point_data.len();
        let mut columns = self
            .mapping
            .fields
            .iter()
            .map(|kind| empty_column(*kind, point_count, self.inner.header().point_format()))
            .collect::<Vec<_>>();
        let mut dense = true;
        for point in point_data.points() {
            let point = point.map_err(Error::Las)?;
            dense &= point.x.is_finite() && point.y.is_finite() && point.z.is_finite();
            for (kind, column) in self.mapping.fields.iter().zip(&mut columns) {
                push_point_value(*kind, column, &point);
            }
        }
        let dimensions = PointDimensions::new(point_count, 1).map_err(Error::Layout)?;
        let metadata = Arc::new(PointFrameMetadata::new(
            Timestamp::new(0, 0).expect("zero is a canonical timestamp"),
            "",
            dense,
        ));
        PointBatch::new(
            Arc::clone(&self.mapping.schema),
            metadata,
            dimensions,
            columns,
        )
        .map(Some)
        .map_err(Error::Batch)
    }
}

/// A synchronous LAS/LAZ writer. LAZ uses the serial codec and its fixed
/// 50,000-point chunk, while this adapter stages only one raw point at a time.
pub struct Writer<W: Write + Seek + Send + Sync + 'static> {
    inner: las::Writer<W>,
    mapping: SchemaMapping,
    metadata: Arc<SpatialMetadata>,
    max_points: u64,
    points_written: u64,
    managed_peak_bytes: usize,
}

impl<W: Write + Seek + Send + Sync + 'static> Writer<W> {
    pub fn new(
        output: W,
        metadata: Arc<SpatialMetadata>,
        encoding: Encoding,
        limits: WriteLimits,
    ) -> Result<Self, Error> {
        let mut builder = Builder::from(metadata.header.clone());
        builder.vlrs.retain(|vlr| !las::laz::is_laszip_vlr(vlr));
        builder.evlrs.retain(|vlr| !las::laz::is_laszip_vlr(vlr));
        builder.point_format.is_compressed = encoding == Encoding::Laz;
        let mapping = SchemaMapping::for_format(&builder.point_format)?;
        let managed_peak_bytes = writer_managed_peak(&builder, encoding, limits.max_points)?;
        if managed_peak_bytes > limits.memory_limit_bytes {
            return Err(Error::MemoryLimitExceeded {
                required: managed_peak_bytes,
                limit: limits.memory_limit_bytes,
            });
        }
        let header = builder.into_header().map_err(Error::Las)?;
        let inner = las::Writer::new(output, header).map_err(Error::Las)?;
        Ok(Self {
            inner,
            mapping,
            metadata,
            max_points: limits.max_points,
            points_written: 0,
            managed_peak_bytes,
        })
    }

    pub const fn managed_peak_bytes(&self) -> usize {
        self.managed_peak_bytes
    }

    pub fn write_batch(
        &mut self,
        batch: &PointBatch,
        loss_policy: &LossPolicy,
    ) -> Result<(), Error> {
        if batch.schema() != self.mapping.schema.as_ref() {
            return Err(Error::SchemaMismatch);
        }
        validate_batch_metadata(batch, loss_policy)?;
        let batch_points = u64::try_from(batch.dimensions().point_count())
            .map_err(|_| Error::MemoryEstimateOverflow)?;
        let next_count = self
            .points_written
            .checked_add(batch_points)
            .ok_or(Error::MemoryEstimateOverflow)?;
        if next_count > self.max_points {
            return Err(Error::PointLimitExceeded {
                attempted: next_count,
                limit: self.max_points,
            });
        }
        for point_index in 0..batch.dimensions().point_count() {
            let point = point_from_batch(batch, &self.mapping, point_index)?;
            validate_coordinate_quantization(
                &point,
                self.metadata.header.transforms(),
                loss_policy,
            )?;
            validate_scan_angle_quantization(
                point.scan_angle,
                self.metadata.header.point_format(),
                loss_policy,
            )?;
            self.inner.write_point(point).map_err(Error::Las)?;
        }
        self.points_written = next_count;
        Ok(())
    }

    pub fn finish(mut self) -> Result<W, Error> {
        self.inner.close().map_err(Error::Las)?;
        self.inner.into_inner().map_err(Error::Las)
    }
}

fn validate_batch_metadata(batch: &PointBatch, policy: &LossPolicy) -> Result<(), Error> {
    let metadata = batch.metadata();
    let timestamp = metadata.sensor_timestamp();
    let loses_metadata = batch.dimensions().height() != 1
        || timestamp.seconds() != 0
        || timestamp.nanoseconds() != 0
        || !metadata.frame_id().is_empty()
        || metadata.log_time_ns().is_some()
        || metadata.publish_time_ns().is_some();
    if loses_metadata && !policy.allows(FidelityLoss::Metadata) {
        return Err(Error::UnrepresentableFrameMetadata);
    }
    Ok(())
}

fn writer_managed_peak(
    builder: &Builder,
    encoding: Encoding,
    max_points: u64,
) -> Result<usize, Error> {
    let record_bytes = builder
        .vlrs
        .iter()
        .try_fold(0usize, |total, vlr| total.checked_add(vlr.len(false)))
        .and_then(|bytes| {
            builder
                .evlrs
                .iter()
                .try_fold(bytes, |total, vlr| total.checked_add(vlr.len(true)))
        })
        .and_then(|bytes| bytes.checked_add(builder.padding.len()))
        .and_then(|bytes| bytes.checked_add(builder.vlr_padding.len()))
        .and_then(|bytes| bytes.checked_add(builder.point_padding.len()))
        .and_then(|bytes| bytes.checked_add(builder.system_identifier.len()))
        .and_then(|bytes| bytes.checked_add(builder.generating_software.len()))
        .ok_or(Error::MemoryEstimateOverflow)?;
    let chunk_table_bytes = if encoding == Encoding::Laz {
        let chunks = max_points
            .checked_add(49_999)
            .ok_or(Error::MemoryEstimateOverflow)?
            / 50_000;
        usize::try_from(chunks)
            .ok()
            .and_then(|chunks| chunks.checked_mul(2 * size_of::<u64>()))
            .ok_or(Error::MemoryEstimateOverflow)?
    } else {
        0
    };
    record_bytes
        .checked_mul(2)
        .and_then(|bytes| bytes.checked_add(chunk_table_bytes))
        .and_then(|bytes| bytes.checked_add(usize::from(builder.point_format.len())))
        .and_then(|bytes| bytes.checked_add(4096))
        .ok_or(Error::MemoryEstimateOverflow)
}

fn managed_peak(
    header: &Header,
    mapping: &SchemaMapping,
    limits: ReadLimits,
    laz_chunk_table_bytes: usize,
) -> Result<usize, Error> {
    let points = limits.max_points_per_batch.get();
    let raw = points
        .checked_mul(usize::from(header.point_format().len()))
        .ok_or(Error::MemoryEstimateOverflow)?;
    let columns = points
        .checked_mul(mapping.bytes_per_point)
        .and_then(|bytes| bytes.checked_add(mapping.fields.len() * size_of::<PointColumn>()))
        .ok_or(Error::MemoryEstimateOverflow)?;
    let vlr_bytes = header
        .vlrs()
        .iter()
        .try_fold(0usize, |total, vlr| total.checked_add(vlr.len(false)))
        .ok_or(Error::MemoryEstimateOverflow)?;
    let evlr_bytes = header
        .evlrs()
        .iter()
        .try_fold(0usize, |total, vlr| total.checked_add(vlr.len(true)))
        .ok_or(Error::MemoryEstimateOverflow)?;
    let record_bytes = vlr_bytes
        .checked_add(evlr_bytes)
        .and_then(|bytes| bytes.checked_add(header.padding().len()))
        .and_then(|bytes| bytes.checked_add(header.vlr_padding().len()))
        .and_then(|bytes| bytes.checked_add(header.point_padding().len()))
        .and_then(|bytes| bytes.checked_add(header.system_identifier().len()))
        .and_then(|bytes| bytes.checked_add(header.generating_software().len()))
        .ok_or(Error::MemoryEstimateOverflow)?;
    raw.checked_add(columns)
        .and_then(|bytes| bytes.checked_add(record_bytes.saturating_mul(2)))
        .and_then(|bytes| bytes.checked_add(laz_chunk_table_bytes))
        .and_then(|bytes| bytes.checked_add(4096))
        .ok_or(Error::MemoryEstimateOverflow)
}

fn laz_chunk_table_bytes(input: &mut (impl Read + Seek), header: &Header) -> Result<usize, Error> {
    if !header.point_format().is_compressed {
        return Ok(0);
    }
    let point_data_start = input.stream_position().map_err(Error::Io)?;
    let mut offset = [0_u8; 8];
    input.read_exact(&mut offset).map_err(Error::Io)?;
    let mut chunk_table_offset = i64::from_le_bytes(offset);
    if chunk_table_offset <= point_data_start as i64 {
        input.seek(std::io::SeekFrom::End(-8)).map_err(Error::Io)?;
        input.read_exact(&mut offset).map_err(Error::Io)?;
        chunk_table_offset = i64::from_le_bytes(offset);
    }
    let chunk_table_offset = u64::try_from(chunk_table_offset)
        .map_err(|_| Error::MalformedChunkTable("negative chunk-table offset"))?;
    if chunk_table_offset <= point_data_start {
        return Err(Error::MalformedChunkTable(
            "chunk-table offset precedes point data",
        ));
    }
    input
        .seek(std::io::SeekFrom::Start(chunk_table_offset))
        .map_err(Error::Io)?;
    let mut table_header = [0_u8; 8];
    input.read_exact(&mut table_header).map_err(Error::Io)?;
    let chunks = u32::from_le_bytes(table_header[4..8].try_into().expect("four-byte slice"));
    let chunks = usize::try_from(chunks).map_err(|_| Error::MemoryEstimateOverflow)?;
    let declared_points =
        usize::try_from(header.number_of_points()).map_err(|_| Error::MemoryEstimateOverflow)?;
    if chunks > declared_points.max(1) {
        return Err(Error::MalformedChunkTable(
            "chunk count exceeds the declared point count",
        ));
    }
    chunks
        .checked_mul(2 * size_of::<u64>())
        .ok_or(Error::MemoryEstimateOverflow)
}

fn empty_column(kind: FieldKind, points: usize, format: &Format) -> PointColumn {
    let count = if kind == FieldKind::ExtraBytes {
        usize::from(format.extra_bytes)
    } else {
        1
    };
    let capacity = points
        .checked_mul(count)
        .expect("preflight checked column capacity");
    match kind {
        FieldKind::X | FieldKind::Y | FieldKind::Z | FieldKind::GpsTime => {
            PointColumn::F64(Vec::with_capacity(capacity))
        }
        FieldKind::Intensity
        | FieldKind::PointSourceId
        | FieldKind::Red
        | FieldKind::Green
        | FieldKind::Blue
        | FieldKind::Nir => PointColumn::U16(Vec::with_capacity(capacity)),
        FieldKind::ScanAngle
        | FieldKind::ReturnPointWaveformLocation
        | FieldKind::WaveformXt
        | FieldKind::WaveformYt
        | FieldKind::WaveformZt => PointColumn::F32(Vec::with_capacity(capacity)),
        FieldKind::WaveformDataOffset => PointColumn::U64(Vec::with_capacity(capacity)),
        FieldKind::WaveformPacketSize => PointColumn::U32(Vec::with_capacity(capacity)),
        _ => PointColumn::U8(Vec::with_capacity(capacity)),
    }
}

fn push_point_value(kind: FieldKind, column: &mut PointColumn, point: &Point) {
    match (kind, column) {
        (FieldKind::X, PointColumn::F64(values)) => values.push(point.x),
        (FieldKind::Y, PointColumn::F64(values)) => values.push(point.y),
        (FieldKind::Z, PointColumn::F64(values)) => values.push(point.z),
        (FieldKind::Intensity, PointColumn::U16(values)) => values.push(point.intensity),
        (FieldKind::ReturnNumber, PointColumn::U8(values)) => values.push(point.return_number),
        (FieldKind::NumberOfReturns, PointColumn::U8(values)) => {
            values.push(point.number_of_returns)
        }
        (FieldKind::ScanDirection, PointColumn::U8(values)) => {
            values.push(u8::from(point.scan_direction == ScanDirection::LeftToRight))
        }
        (FieldKind::EdgeOfFlightLine, PointColumn::U8(values)) => {
            values.push(u8::from(point.is_edge_of_flight_line))
        }
        (FieldKind::Classification, PointColumn::U8(values)) => {
            values.push(point.classification.into())
        }
        (FieldKind::Synthetic, PointColumn::U8(values)) => {
            values.push(u8::from(point.is_synthetic))
        }
        (FieldKind::KeyPoint, PointColumn::U8(values)) => values.push(u8::from(point.is_key_point)),
        (FieldKind::Withheld, PointColumn::U8(values)) => values.push(u8::from(point.is_withheld)),
        (FieldKind::Overlap, PointColumn::U8(values)) => values.push(u8::from(point.is_overlap)),
        (FieldKind::ScannerChannel, PointColumn::U8(values)) => values.push(point.scanner_channel),
        (FieldKind::ScanAngle, PointColumn::F32(values)) => values.push(point.scan_angle),
        (FieldKind::UserData, PointColumn::U8(values)) => values.push(point.user_data),
        (FieldKind::PointSourceId, PointColumn::U16(values)) => values.push(point.point_source_id),
        (FieldKind::GpsTime, PointColumn::F64(values)) => {
            values.push(point.gps_time.expect("format requires GPS time"))
        }
        (FieldKind::Red, PointColumn::U16(values)) => {
            values.push(point.color.expect("format requires color").red)
        }
        (FieldKind::Green, PointColumn::U16(values)) => {
            values.push(point.color.expect("format requires color").green)
        }
        (FieldKind::Blue, PointColumn::U16(values)) => {
            values.push(point.color.expect("format requires color").blue)
        }
        (FieldKind::Nir, PointColumn::U16(values)) => {
            values.push(point.nir.expect("format requires NIR"))
        }
        (FieldKind::WavePacketDescriptorIndex, PointColumn::U8(values)) => values.push(
            point
                .waveform
                .expect("format requires waveform")
                .wave_packet_descriptor_index,
        ),
        (FieldKind::WaveformDataOffset, PointColumn::U64(values)) => values.push(
            point
                .waveform
                .expect("format requires waveform")
                .byte_offset_to_waveform_data,
        ),
        (FieldKind::WaveformPacketSize, PointColumn::U32(values)) => values.push(
            point
                .waveform
                .expect("format requires waveform")
                .waveform_packet_size_in_bytes,
        ),
        (FieldKind::ReturnPointWaveformLocation, PointColumn::F32(values)) => values.push(
            point
                .waveform
                .expect("format requires waveform")
                .return_point_waveform_location,
        ),
        (FieldKind::WaveformXt, PointColumn::F32(values)) => {
            values.push(point.waveform.expect("format requires waveform").x_t)
        }
        (FieldKind::WaveformYt, PointColumn::F32(values)) => {
            values.push(point.waveform.expect("format requires waveform").y_t)
        }
        (FieldKind::WaveformZt, PointColumn::F32(values)) => {
            values.push(point.waveform.expect("format requires waveform").z_t)
        }
        (FieldKind::ExtraBytes, PointColumn::U8(values)) => {
            values.extend_from_slice(&point.extra_bytes)
        }
        _ => unreachable!("column was built from the same field mapping"),
    }
}

fn point_from_batch(
    batch: &PointBatch,
    mapping: &SchemaMapping,
    index: usize,
) -> Result<Point, Error> {
    let mut point = Point::default();
    let mut color = None;
    let mut waveform = None;
    for (kind, column) in mapping.fields.iter().zip(batch.columns()) {
        match kind {
            FieldKind::X => point.x = f64_at(column, index)?,
            FieldKind::Y => point.y = f64_at(column, index)?,
            FieldKind::Z => point.z = f64_at(column, index)?,
            FieldKind::Intensity => point.intensity = u16_at(column, index)?,
            FieldKind::ReturnNumber => point.return_number = u8_at(column, index)?,
            FieldKind::NumberOfReturns => point.number_of_returns = u8_at(column, index)?,
            FieldKind::ScanDirection => {
                point.scan_direction = if bool_at(column, index)? {
                    ScanDirection::LeftToRight
                } else {
                    ScanDirection::RightToLeft
                }
            }
            FieldKind::EdgeOfFlightLine => point.is_edge_of_flight_line = bool_at(column, index)?,
            FieldKind::Classification => {
                point.classification =
                    Classification::new(u8_at(column, index)?).map_err(Error::Las)?
            }
            FieldKind::Synthetic => point.is_synthetic = bool_at(column, index)?,
            FieldKind::KeyPoint => point.is_key_point = bool_at(column, index)?,
            FieldKind::Withheld => point.is_withheld = bool_at(column, index)?,
            FieldKind::Overlap => point.is_overlap = bool_at(column, index)?,
            FieldKind::ScannerChannel => point.scanner_channel = u8_at(column, index)?,
            FieldKind::ScanAngle => point.scan_angle = f32_at(column, index)?,
            FieldKind::UserData => point.user_data = u8_at(column, index)?,
            FieldKind::PointSourceId => point.point_source_id = u16_at(column, index)?,
            FieldKind::GpsTime => point.gps_time = Some(f64_at(column, index)?),
            FieldKind::Red => color.get_or_insert_with(Color::default).red = u16_at(column, index)?,
            FieldKind::Green => {
                color.get_or_insert_with(Color::default).green = u16_at(column, index)?
            }
            FieldKind::Blue => {
                color.get_or_insert_with(Color::default).blue = u16_at(column, index)?
            }
            FieldKind::Nir => point.nir = Some(u16_at(column, index)?),
            FieldKind::WavePacketDescriptorIndex => {
                waveform
                    .get_or_insert_with(Waveform::default)
                    .wave_packet_descriptor_index = u8_at(column, index)?
            }
            FieldKind::WaveformDataOffset => {
                waveform
                    .get_or_insert_with(Waveform::default)
                    .byte_offset_to_waveform_data = u64_at(column, index)?
            }
            FieldKind::WaveformPacketSize => {
                waveform
                    .get_or_insert_with(Waveform::default)
                    .waveform_packet_size_in_bytes = u32_at(column, index)?
            }
            FieldKind::ReturnPointWaveformLocation => {
                waveform
                    .get_or_insert_with(Waveform::default)
                    .return_point_waveform_location = f32_at(column, index)?
            }
            FieldKind::WaveformXt => {
                waveform.get_or_insert_with(Waveform::default).x_t = f32_at(column, index)?
            }
            FieldKind::WaveformYt => {
                waveform.get_or_insert_with(Waveform::default).y_t = f32_at(column, index)?
            }
            FieldKind::WaveformZt => {
                waveform.get_or_insert_with(Waveform::default).z_t = f32_at(column, index)?
            }
            FieldKind::ExtraBytes => {
                let PointColumn::U8(values) = column else {
                    return Err(Error::SchemaMismatch);
                };
                let count = mapping.schema.fields()[mapping
                    .fields
                    .iter()
                    .position(|candidate| candidate == kind)
                    .expect("mapped field")]
                .count();
                let start = index
                    .checked_mul(count)
                    .ok_or(Error::MemoryEstimateOverflow)?;
                point
                    .extra_bytes
                    .extend_from_slice(&values[start..start + count]);
            }
        }
    }
    point.color = color;
    point.waveform = waveform;
    Ok(point)
}

fn validate_coordinate_quantization(
    point: &Point,
    transforms: &las::Vector<las::Transform>,
    policy: &LossPolicy,
) -> Result<(), Error> {
    for (axis, value, transform) in [
        ("x", point.x, transforms.x),
        ("y", point.y, transforms.y),
        ("z", point.z, transforms.z),
    ] {
        let integer = transform.inverse(value).map_err(Error::Las)?;
        let represented = transform.direct(integer);
        if represented.to_bits() != value.to_bits() && !policy.allows(FidelityLoss::Representation)
        {
            return Err(Error::CoordinateQuantization {
                axis,
                value,
                represented,
            });
        }
    }
    Ok(())
}

fn validate_scan_angle_quantization(
    value: f32,
    format: &Format,
    policy: &LossPolicy,
) -> Result<(), Error> {
    let represented = if format.is_extended {
        let raw = (value / 0.006) as i16;
        f32::from(raw) * 0.006
    } else {
        value.round().clamp(f32::from(i8::MIN), f32::from(i8::MAX))
    };
    if represented.to_bits() != value.to_bits() && !policy.allows(FidelityLoss::Representation) {
        return Err(Error::ScanAngleQuantization { value, represented });
    }
    Ok(())
}

macro_rules! typed_at {
    ($name:ident, $variant:ident, $type:ty) => {
        fn $name(column: &PointColumn, index: usize) -> Result<$type, Error> {
            match column {
                PointColumn::$variant(values) => {
                    values.get(index).copied().ok_or(Error::SchemaMismatch)
                }
                _ => Err(Error::SchemaMismatch),
            }
        }
    };
}
typed_at!(u8_at, U8, u8);
typed_at!(u16_at, U16, u16);
typed_at!(u32_at, U32, u32);
typed_at!(u64_at, U64, u64);
typed_at!(f32_at, F32, f32);
typed_at!(f64_at, F64, f64);

fn bool_at(column: &PointColumn, index: usize) -> Result<bool, Error> {
    match u8_at(column, index)? {
        0 => Ok(false),
        1 => Ok(true),
        value => Err(Error::InvalidBoolean(value)),
    }
}

/// LAS/LAZ parsing, mapping, fidelity, and resource failures.
#[derive(Debug)]
pub enum Error {
    Las(las::Error),
    Io(std::io::Error),
    Schema(crate::core::point::SchemaError),
    Layout(crate::core::point::LayoutError),
    Batch(crate::core::point::BatchError),
    ZeroBatchPointLimit,
    MemoryEstimateOverflow,
    MemoryLimitExceeded {
        required: usize,
        limit: usize,
    },
    SchemaMismatch,
    UnrepresentableFrameMetadata,
    PointLimitExceeded {
        attempted: u64,
        limit: u64,
    },
    InvalidBoolean(u8),
    MalformedChunkTable(&'static str),
    CoordinateQuantization {
        axis: &'static str,
        value: f64,
        represented: f64,
    },
    ScanAngleQuantization {
        value: f32,
        represented: f32,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Las(error) => write!(formatter, "LAS/LAZ error: {error}"),
            Self::Io(error) => write!(formatter, "LAS/LAZ I/O error: {error}"),
            Self::Schema(error) => write!(formatter, "invalid LAS point schema: {error}"),
            Self::Layout(error) => write!(formatter, "invalid LAS point dimensions: {error}"),
            Self::Batch(error) => write!(formatter, "invalid LAS point batch: {error}"),
            Self::ZeroBatchPointLimit => {
                formatter.write_str("LAS batch point limit must be positive")
            }
            Self::MemoryEstimateOverflow => {
                formatter.write_str("LAS managed-memory estimate overflowed")
            }
            Self::MemoryLimitExceeded { required, limit } => write!(
                formatter,
                "LAS managed-memory peak of {required} bytes exceeds the {limit}-byte limit"
            ),
            Self::SchemaMismatch => formatter
                .write_str("point schema does not exactly match the LAS point format mapping"),
            Self::UnrepresentableFrameMetadata => formatter.write_str(
                "LAS cannot represent organized dimensions or Point Frame identity/timestamps; authorize metadata loss explicitly",
            ),
            Self::PointLimitExceeded { attempted, limit } => write!(
                formatter,
                "LAS writer point count {attempted} exceeds the declared {limit}-point bound"
            ),
            Self::InvalidBoolean(value) => {
                write!(formatter, "LAS flag field must be 0 or 1, got {value}")
            }
            Self::MalformedChunkTable(reason) => {
                write!(formatter, "malformed LAZ chunk table: {reason}")
            }
            Self::CoordinateQuantization {
                axis,
                value,
                represented,
            } => write!(
                formatter,
                "LAS {axis} coordinate {value} would be quantized to {represented}; authorize representation loss explicitly"
            ),
            Self::ScanAngleQuantization { value, represented } => write!(
                formatter,
                "LAS scan angle {value} would be quantized to {represented}; authorize representation loss explicitly"
            ),
        }
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Las(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Schema(error) => Some(error),
            Self::Layout(error) => Some(error),
            Self::Batch(error) => Some(error),
            _ => None,
        }
    }
}
