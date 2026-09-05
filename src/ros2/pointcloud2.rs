//! Strict decoding for ROS 2 `sensor_msgs/msg/PointCloud2`.
//!
//! The decoder accepts exactly the CDR shape of `PointCloud2`. It validates
//! the complete message and returns a low-copy [`PointView`] over the original
//! serialized bytes. Duplicate field names and overlapping field byte ranges
//! are rejected rather than assigned an order-dependent interpretation.

use std::{error, fmt, sync::Arc};

use crate::core::point::{
    Endianness, LayoutError, PointDimensions, PointField, PointFieldSemantic, PointFrameMetadata,
    PointLayout, PointSchema, PointView, PrimitiveType, SchemaError, Timestamp, TimestampError,
    ViewError,
};

use super::cdr::{self, Cursor};

/// A malformed or unsupported `sensor_msgs/msg/PointCloud2` message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    /// A CDR value could not be read. `member` identifies the message member.
    Cdr {
        member: String,
        source: cdr::Error,
    },
    InvalidTimestamp(TimestampError),
    UnsupportedDatatype {
        field: usize,
        name: String,
        datatype: u8,
    },
    InvalidField {
        field: usize,
        source: SchemaError,
    },
    InvalidSchema(SchemaError),
    InvalidDimensions(LayoutError),
    InvalidLayout(LayoutError),
    InvalidView(ViewError),
    PayloadLength {
        declared: usize,
        expected: usize,
        height: usize,
        row_step: usize,
    },
    NumericRange {
        member: String,
        value: u32,
    },
    InvalidDataOffset {
        offset: usize,
        length: usize,
    },
    TrailingData {
        offset: usize,
        remaining: usize,
    },
    Allocation {
        member: &'static str,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cdr { member, source } => {
                write!(formatter, "invalid PointCloud2.{member}: {source}")
            }
            Self::InvalidTimestamp(source) => {
                write!(formatter, "invalid PointCloud2.header.stamp: {source}")
            }
            Self::UnsupportedDatatype {
                field,
                name,
                datatype,
            } => write!(
                formatter,
                "invalid PointCloud2.fields[{field}] ({name:?}): unsupported PointField datatype {datatype}"
            ),
            Self::InvalidField { field, source } => {
                write!(formatter, "invalid PointCloud2.fields[{field}]: {source}")
            }
            Self::InvalidSchema(source) => {
                write!(formatter, "invalid PointCloud2.fields: {source}")
            }
            Self::InvalidDimensions(source) => {
                write!(formatter, "invalid PointCloud2 dimensions: {source}")
            }
            Self::InvalidLayout(source) => {
                write!(formatter, "invalid PointCloud2 layout: {source}")
            }
            Self::InvalidView(source) => {
                write!(formatter, "invalid PointCloud2 data extent: {source}")
            }
            Self::PayloadLength {
                declared,
                expected,
                height,
                row_step,
            } => write!(
                formatter,
                "invalid PointCloud2.data length {declared}: expected height {height} * row_step {row_step} = {expected} bytes"
            ),
            Self::NumericRange { member, value } => write!(
                formatter,
                "invalid PointCloud2.{member}: value {value} does not fit this platform's address space"
            ),
            Self::InvalidDataOffset { offset, length } => write!(
                formatter,
                "invalid PointCloud2.data extent: sequence ending at byte {offset} cannot contain {length} bytes"
            ),
            Self::TrailingData { offset, remaining } => write!(
                formatter,
                "invalid PointCloud2 message: {remaining} trailing bytes after byte {offset}"
            ),
            Self::Allocation { member } => {
                write!(formatter, "could not allocate PointCloud2.{member}")
            }
        }
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Cdr { source, .. } => Some(source),
            Self::InvalidTimestamp(source) => Some(source),
            Self::InvalidField { source, .. } | Self::InvalidSchema(source) => Some(source),
            Self::InvalidDimensions(source) | Self::InvalidLayout(source) => Some(source),
            Self::InvalidView(source) => Some(source),
            Self::UnsupportedDatatype { .. }
            | Self::PayloadLength { .. }
            | Self::NumericRange { .. }
            | Self::InvalidDataOffset { .. }
            | Self::TrailingData { .. }
            | Self::Allocation { .. } => None,
        }
    }
}

/// Decode and validate one encapsulated ROS 2 `PointCloud2` message.
///
/// `source` is retained by the returned view, so the point payload itself is
/// not copied. The decoder does not accept trailing bytes or a partial message.
pub fn decode(source: Arc<[u8]>) -> Result<PointView, Error> {
    let mut cursor = Cursor::new(&source).map_err(|source| Error::Cdr {
        member: "encapsulation".into(),
        source,
    })?;

    let seconds = read(&mut cursor, "header.stamp.sec", Cursor::read_i32)?;
    let nanoseconds = read(&mut cursor, "header.stamp.nanosec", Cursor::read_u32)?;
    let frame_id = copy_string(read(&mut cursor, "header.frame_id", Cursor::read_string)?)?;
    let height = as_usize(read(&mut cursor, "height", Cursor::read_u32)?, "height")?;
    let width = as_usize(read(&mut cursor, "width", Cursor::read_u32)?, "width")?;
    let field_count = as_usize(
        read(&mut cursor, "fields", Cursor::read_sequence_len)?,
        "fields",
    )?;

    let mut fields = Vec::new();
    let mut offsets = Vec::new();
    for field_index in 0..field_count {
        fields
            .try_reserve(1)
            .map_err(|_| Error::Allocation { member: "fields" })?;
        offsets.try_reserve(1).map_err(|_| Error::Allocation {
            member: "field offsets",
        })?;

        let name_member = format!("fields[{field_index}].name");
        let name = copy_string(read(&mut cursor, &name_member, Cursor::read_string)?)?;
        let offset_member = format!("fields[{field_index}].offset");
        let offset = as_usize(
            read(&mut cursor, &offset_member, Cursor::read_u32)?,
            &offset_member,
        )?;
        let datatype_member = format!("fields[{field_index}].datatype");
        let datatype = read(&mut cursor, &datatype_member, Cursor::read_u8)?;
        let count_member = format!("fields[{field_index}].count");
        let count = as_usize(
            read(&mut cursor, &count_member, Cursor::read_u32)?,
            &count_member,
        )?;
        let primitive = primitive_type(datatype).ok_or_else(|| Error::UnsupportedDatatype {
            field: field_index,
            name: name.clone(),
            datatype,
        })?;
        let semantic = field_semantic(&name);
        let field = PointField::new(name, primitive, count, semantic).map_err(|source| {
            Error::InvalidField {
                field: field_index,
                source,
            }
        })?;
        fields.push(field);
        offsets.push(offset);
    }

    let is_bigendian = read(&mut cursor, "is_bigendian", Cursor::read_bool)?;
    let point_step = as_usize(
        read(&mut cursor, "point_step", Cursor::read_u32)?,
        "point_step",
    )?;
    let row_step = as_usize(read(&mut cursor, "row_step", Cursor::read_u32)?, "row_step")?;
    let data = read(&mut cursor, "data", Cursor::read_byte_sequence)?;
    let data_offset = cursor
        .offset()
        .checked_sub(data.len())
        .ok_or(Error::InvalidDataOffset {
            offset: cursor.offset(),
            length: data.len(),
        })?;
    let is_dense = read(&mut cursor, "is_dense", Cursor::read_bool)?;

    if cursor.remaining() != 0 {
        return Err(Error::TrailingData {
            offset: cursor.offset(),
            remaining: cursor.remaining(),
        });
    }

    let timestamp = Timestamp::new(seconds, nanoseconds).map_err(Error::InvalidTimestamp)?;
    let dimensions = PointDimensions::new(width, height).map_err(Error::InvalidDimensions)?;
    let expected_payload_len = height
        .checked_mul(row_step)
        .ok_or(Error::InvalidLayout(LayoutError::PayloadExtentOverflow))?;
    if data.len() != expected_payload_len {
        return Err(Error::PayloadLength {
            declared: data.len(),
            expected: expected_payload_len,
            height,
            row_step,
        });
    }

    let schema = Arc::new(PointSchema::new(fields).map_err(Error::InvalidSchema)?);
    let metadata = Arc::new(PointFrameMetadata::new(timestamp, frame_id, is_dense));
    let endianness = if is_bigendian {
        Endianness::Big
    } else {
        Endianness::Little
    };
    let layout = PointLayout::new(
        schema,
        dimensions,
        offsets,
        point_step,
        row_step,
        data_offset,
        endianness,
    )
    .map_err(Error::InvalidLayout)?;
    PointView::new(source, metadata, layout).map_err(Error::InvalidView)
}

fn read<'a, T>(
    cursor: &mut Cursor<'a>,
    member: &str,
    operation: impl FnOnce(&mut Cursor<'a>) -> Result<T, cdr::Error>,
) -> Result<T, Error> {
    operation(cursor).map_err(|source| Error::Cdr {
        member: member.to_owned(),
        source,
    })
}

fn copy_string(value: &str) -> Result<String, Error> {
    let mut copy = String::new();
    copy.try_reserve_exact(value.len())
        .map_err(|_| Error::Allocation { member: "string" })?;
    copy.push_str(value);
    Ok(copy)
}

fn as_usize(value: u32, member: &str) -> Result<usize, Error> {
    usize::try_from(value).map_err(|_| Error::NumericRange {
        member: member.to_owned(),
        value,
    })
}

const fn primitive_type(datatype: u8) -> Option<PrimitiveType> {
    match datatype {
        1 => Some(PrimitiveType::I8),
        2 => Some(PrimitiveType::U8),
        3 => Some(PrimitiveType::I16),
        4 => Some(PrimitiveType::U16),
        5 => Some(PrimitiveType::I32),
        6 => Some(PrimitiveType::U32),
        7 => Some(PrimitiveType::F32),
        8 => Some(PrimitiveType::F64),
        _ => None,
    }
}

fn field_semantic(name: &str) -> Option<PointFieldSemantic> {
    match name {
        "x" => Some(PointFieldSemantic::X),
        "y" => Some(PointFieldSemantic::Y),
        "z" => Some(PointFieldSemantic::Z),
        "intensity" => Some(PointFieldSemantic::Intensity),
        "ring" => Some(PointFieldSemantic::Ring),
        "timestamp" => Some(PointFieldSemantic::Timestamp),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::point::{PointValue, PrimitiveType};

    const LITTLE: &[u8] =
        include_bytes!("../../tests/fixtures/valid/pointcloud2-little-endian.cdr");
    const BIG: &[u8] = include_bytes!("../../tests/fixtures/valid/pointcloud2-big-endian.cdr");
    const BAD_REPRESENTATION: &[u8] = include_bytes!(
        "../../tests/fixtures/malformed/cdr-representation-identifier-must-be-cdr.cdr"
    );
    const TRUNCATED_DATA: &[u8] = include_bytes!(
        "../../tests/fixtures/malformed/cdr-point-data-sequence-must-not-be-truncated.cdr"
    );
    const FIELD_OUTSIDE_POINT: &[u8] =
        include_bytes!("../../tests/fixtures/malformed/pointcloud2-field-must-fit-point-step.cdr");
    const ORGANIZED: &[u8] =
        include_bytes!("../../tests/fixtures/valid/pointcloud2-organized-row-padding.cdr");
    const VARIED_FIELDS: &[u8] =
        include_bytes!("../../tests/fixtures/valid/pointcloud2-reordered-fields-and-count.cdr");
    const DUPLICATE_FIELDS: &[u8] =
        include_bytes!("../../tests/fixtures/malformed/pointcloud2-field-names-must-be-unique.cdr");
    const OVERLAPPING_FIELDS: &[u8] = include_bytes!(
        "../../tests/fixtures/malformed/pointcloud2-field-ranges-must-not-overlap.cdr"
    );
    const ZERO_FIELD_COUNT: &[u8] = include_bytes!(
        "../../tests/fixtures/malformed/pointcloud2-field-count-must-be-positive.cdr"
    );
    const UNSUPPORTED_DATATYPE: &[u8] = include_bytes!(
        "../../tests/fixtures/malformed/pointcloud2-field-datatype-must-be-supported.cdr"
    );
    const SHORT_ROW: &[u8] =
        include_bytes!("../../tests/fixtures/malformed/pointcloud2-row-step-must-cover-row.cdr");
    const WRONG_PAYLOAD_LENGTH: &[u8] = include_bytes!(
        "../../tests/fixtures/malformed/pointcloud2-data-length-must-equal-height-times-row-step.cdr"
    );
    const BAD_TIMESTAMP: &[u8] = include_bytes!(
        "../../tests/fixtures/malformed/pointcloud2-timestamp-nanoseconds-must-be-canonical.cdr"
    );
    const ZERO_HEIGHT: &[u8] =
        include_bytes!("../../tests/fixtures/malformed/pointcloud2-height-must-be-positive.cdr");
    const ZERO_POINT_STEP: &[u8] = include_bytes!(
        "../../tests/fixtures/malformed/pointcloud2-point-step-must-be-positive.cdr"
    );

    fn decode_bytes(bytes: &[u8]) -> Result<PointView, Error> {
        decode(Arc::from(bytes))
    }

    #[test]
    fn decodes_committed_fixtures_in_both_byte_orders() {
        for (bytes, endianness) in [(LITTLE, Endianness::Little), (BIG, Endianness::Big)] {
            let view = decode_bytes(bytes).unwrap();
            assert_eq!(view.metadata().sensor_timestamp().seconds(), 1_700_000_000);
            assert_eq!(
                view.metadata().sensor_timestamp().nanoseconds(),
                123_456_789
            );
            assert_eq!(view.metadata().frame_id(), "map");
            assert!(!view.metadata().is_dense());
            assert_eq!(
                view.layout().dimensions(),
                PointDimensions::new(2, 1).unwrap()
            );
            assert_eq!(view.layout().point_step(), 16);
            assert_eq!(view.layout().row_step(), 32);
            assert_eq!(view.layout().endianness(), endianness);
            assert_eq!(view.schema().fields().len(), 5);
            assert_eq!(
                view.schema().field("x").unwrap().primitive(),
                PrimitiveType::F32
            );
            assert_eq!(
                view.schema().field("ring").unwrap().primitive(),
                PrimitiveType::U16
            );
            assert_eq!(
                view.field("x").unwrap().value(0, 0),
                Ok(PointValue::F32(1.0))
            );
            let PointValue::F32(nan) = view.field("z").unwrap().value(1, 0).unwrap() else {
                panic!("z must be float32")
            };
            assert_eq!(nan.to_bits(), 0x7fc0_1234);
            assert_eq!(
                view.field("ring").unwrap().value(1, 0),
                Ok(PointValue::U16(8))
            );
        }
    }

    #[test]
    fn committed_malformed_fixtures_report_member_context_without_panicking() {
        for (bytes, expected_context) in [
            (BAD_REPRESENTATION, "PointCloud2.encapsulation"),
            (TRUNCATED_DATA, "PointCloud2.data"),
            (FIELD_OUTSIDE_POINT, "PointCloud2 layout"),
        ] {
            let result = std::panic::catch_unwind(|| decode_bytes(bytes));
            let error = result
                .expect("malformed PointCloud2 must not panic")
                .unwrap_err();
            assert!(
                error.to_string().contains(expected_context),
                "{error:?} did not contain {expected_context:?}"
            );
        }
    }

    #[test]
    fn organized_rows_preserve_padding_and_point_indexing() {
        let view = decode_bytes(ORGANIZED).unwrap();
        assert_eq!(
            view.layout().dimensions(),
            PointDimensions::new(2, 2).unwrap()
        );
        assert_eq!(view.layout().point_step(), 4);
        assert_eq!(view.layout().row_step(), 12);
        assert_eq!(
            (0..4)
                .map(|point| view.field("x").unwrap().value(point, 0).unwrap())
                .collect::<Vec<_>>(),
            [
                PointValue::F32(1.0),
                PointValue::F32(2.0),
                PointValue::F32(3.0),
                PointValue::F32(4.0),
            ]
        );
    }

    #[test]
    fn preserves_reordered_fields_counts_and_every_ros_datatype() {
        let view = decode_bytes(VARIED_FIELDS).unwrap();
        let fields = view.schema().fields();
        assert_eq!(
            fields.iter().map(|field| field.name()).collect::<Vec<_>>(),
            [
                "returns", "i8", "u16", "i16", "u32", "i32", "normal", "time"
            ]
        );
        assert_eq!(
            fields
                .iter()
                .map(|field| field.primitive())
                .collect::<Vec<_>>(),
            [
                PrimitiveType::U8,
                PrimitiveType::I8,
                PrimitiveType::U16,
                PrimitiveType::I16,
                PrimitiveType::U32,
                PrimitiveType::I32,
                PrimitiveType::F32,
                PrimitiveType::F64,
            ]
        );
        assert_eq!(fields[0].count(), 2);
        assert_eq!(fields[6].count(), 3);
        assert_eq!(
            view.field("returns").unwrap().value(0, 1),
            Ok(PointValue::U8(2))
        );
        assert_eq!(
            view.field("i8").unwrap().value(0, 0),
            Ok(PointValue::I8(-3))
        );
        assert_eq!(
            view.field("normal").unwrap().value(0, 2),
            Ok(PointValue::F32(10.0))
        );
        assert_eq!(
            view.field("time").unwrap().value(0, 0),
            Ok(PointValue::F64(11.0))
        );
    }

    #[test]
    fn rejects_every_committed_pointcloud2_layout_violation() {
        for (bytes, expected) in [
            (DUPLICATE_FIELDS, "DuplicateFieldName"),
            (OVERLAPPING_FIELDS, "OverlappingFields"),
            (ZERO_FIELD_COUNT, "ZeroFieldCount"),
            (UNSUPPORTED_DATATYPE, "unsupported PointField datatype 9"),
            (SHORT_ROW, "RowStepTooSmall"),
            (WRONG_PAYLOAD_LENGTH, "expected height 2 * row_step 8 = 16"),
            (BAD_TIMESTAMP, "PointCloud2.header.stamp"),
            (ZERO_HEIGHT, "ZeroHeight"),
            (ZERO_POINT_STEP, "ZeroPointStep"),
        ] {
            let error = decode_bytes(bytes).unwrap_err();
            assert!(
                error.to_string().contains(expected),
                "{error:?} did not contain {expected:?}"
            );
        }
    }

    #[test]
    fn every_message_truncation_is_rejected_without_panicking() {
        for boundary in 0..LITTLE.len() {
            let result = std::panic::catch_unwind(|| decode_bytes(&LITTLE[..boundary]));
            assert!(result.is_ok(), "decoder panicked at byte {boundary}");
            assert!(
                result.unwrap().is_err(),
                "decoder accepted truncation at byte {boundary}"
            );
        }
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let mut bytes = LITTLE.to_vec();
        bytes.push(0);
        assert!(matches!(
            decode_bytes(&bytes),
            Err(Error::TrailingData { remaining: 1, .. })
        ));
    }

    #[test]
    fn an_impossible_field_sequence_length_fails_at_a_bounded_cdr_read() {
        let mut bytes = LITTLE.to_vec();
        // The fields sequence length begins at byte 28 in this reviewed fixture.
        bytes[28..32].copy_from_slice(&u32::MAX.to_le_bytes());
        let result = std::panic::catch_unwind(|| decode_bytes(&bytes));
        assert!(matches!(result, Ok(Err(Error::Cdr { .. }))));
    }
}
