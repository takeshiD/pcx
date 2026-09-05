//! Bounded synchronous PCD v0.7 reader.

use std::{error, fmt, io, mem::size_of, sync::Arc};

use crate::core::point::{
    BatchError, PointBatch, PointColumn, PointDimensions, PointField, PointFieldSemantic,
    PointFrameMetadata, PointSchema, PrimitiveType, SchemaError, Timestamp, TimestampError,
};

const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_ASCII_TOKEN_BYTES: usize = 256;
const FIXED_MANAGED_OVERHEAD: usize = 64 * 1024;
const PROPORTIONAL_OVERHEAD_DIVISOR: usize = 8;
const DIRECTIVES: [&str; 10] = [
    "VERSION",
    "FIELDS",
    "SIZE",
    "TYPE",
    "COUNT",
    "WIDTH",
    "HEIGHT",
    "VIEWPOINT",
    "POINTS",
    "DATA",
];

/// Conservative managed-memory plan checked before point storage is allocated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadPlan {
    header_bytes: usize,
    point_data_bytes: usize,
    peak_managed_bytes: usize,
    memory_limit_bytes: usize,
}

impl ReadPlan {
    pub const fn header_bytes(self) -> usize {
        self.header_bytes
    }

    pub const fn point_data_bytes(self) -> usize {
        self.point_data_bytes
    }

    pub const fn peak_managed_bytes(self) -> usize {
        self.peak_managed_bytes
    }

    pub const fn memory_limit_bytes(self) -> usize {
        self.memory_limit_bytes
    }
}

/// A decoded static cloud and the memory bound used to admit it.
#[derive(Debug)]
pub struct ReadResult {
    points: PointBatch,
    plan: ReadPlan,
}

impl ReadResult {
    pub const fn points(&self) -> &PointBatch {
        &self.points
    }

    pub const fn plan(&self) -> ReadPlan {
        self.plan
    }

    pub fn into_points(self) -> PointBatch {
        self.points
    }
}

/// Read exactly one supported PCD file from a synchronous byte source.
///
/// The header is bounded by 64 KiB and held on the stack. The declared schema,
/// dimensions, and payload size are validated with checked arithmetic before
/// any point columns are allocated. PCD carries neither a timestamp nor a frame
/// identifier, so the common metadata uses zero/empty sentinel values.
pub fn read(input: &mut impl io::Read, memory_limit_bytes: usize) -> Result<ReadResult, ReadError> {
    let mut header_storage = [0_u8; MAX_HEADER_BYTES];
    let header_len = read_header(input, &mut header_storage)?;
    let header_text = std::str::from_utf8(&header_storage[..header_len])
        .map_err(|_| ReadError::Header("header is not ASCII/UTF-8".into()))?;
    if !header_text.is_ascii() {
        return Err(ReadError::Header("header contains non-ASCII bytes".into()));
    }
    let parsed = ParsedHeader::parse(header_text)?;
    let plan = parsed.plan(header_len, memory_limit_bytes)?;
    let built = parsed.build_schema()?;
    let dimensions =
        PointDimensions::new(parsed.width, parsed.height).map_err(ReadError::Dimensions)?;
    let mut columns = allocate_columns(&built.primitives, &built.counts, parsed.points)?;

    match parsed.encoding {
        PayloadEncoding::Ascii => read_ascii_payload(
            input,
            &built.primitives,
            &built.counts,
            &mut columns,
            parsed.points,
        )?,
        PayloadEncoding::Binary => read_binary_payload(
            input,
            &built.primitives,
            &built.counts,
            &mut columns,
            parsed.points,
        )?,
    }

    let metadata = Arc::new(PointFrameMetadata::new(
        Timestamp::new(0, 0).map_err(ReadError::Metadata)?,
        "",
        false,
    ));
    let points =
        PointBatch::new(built.schema, metadata, dimensions, columns).map_err(ReadError::Batch)?;
    Ok(ReadResult { points, plan })
}

fn read_header(input: &mut impl io::Read, storage: &mut [u8]) -> Result<usize, ReadError> {
    let mut len = 0_usize;
    let mut line_start = 0_usize;
    loop {
        if len == storage.len() {
            return Err(ReadError::HeaderTooLarge {
                maximum: storage.len(),
            });
        }
        let read = input
            .read(&mut storage[len..len + 1])
            .map_err(ReadError::Io)?;
        if read == 0 {
            return Err(ReadError::Header(
                "unexpected end of file before DATA".into(),
            ));
        }
        len += 1;
        if storage[len - 1] != b'\n' {
            continue;
        }
        let line = &storage[line_start..len - 1];
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let trimmed = trim_ascii(line);
        if trimmed == b"DATA"
            || (trimmed.starts_with(b"DATA") && trimmed.get(4).is_some_and(u8::is_ascii_whitespace))
        {
            return Ok(len);
        }
        line_start = len;
    }
}

fn trim_ascii(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

#[derive(Clone, Copy)]
enum PayloadEncoding {
    Ascii,
    Binary,
}

struct ParsedHeader<'a> {
    values: [&'a str; 10],
    width: usize,
    height: usize,
    points: usize,
    point_step: usize,
    field_count: usize,
    encoding: PayloadEncoding,
}

struct BuiltSchema {
    schema: Arc<PointSchema>,
    primitives: Vec<PrimitiveType>,
    counts: Vec<usize>,
}

impl<'a> ParsedHeader<'a> {
    fn parse(text: &'a str) -> Result<Self, ReadError> {
        let mut values = [""; 10];
        let mut directive_index = 0_usize;
        for raw_line in text.lines() {
            let line = raw_line.trim_ascii();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut tokens = line.split_ascii_whitespace();
            let key = tokens
                .next()
                .ok_or_else(|| ReadError::Header("empty directive".into()))?;
            let expected = DIRECTIVES.get(directive_index).ok_or_else(|| {
                ReadError::Header(format!("unexpected directive {key:?} after DATA"))
            })?;
            if key != *expected {
                return Err(ReadError::Header(format!(
                    "expected {expected} directive, found {key:?}"
                )));
            }
            let value_offset = line
                .find(char::is_whitespace)
                .ok_or_else(|| ReadError::Header(format!("{key} requires at least one value")))?;
            let value = line[value_offset..].trim_ascii();
            if value.is_empty() {
                return Err(ReadError::Header(format!(
                    "{key} requires at least one value"
                )));
            }
            values[directive_index] = value;
            directive_index += 1;
        }
        if directive_index != DIRECTIVES.len() {
            return Err(ReadError::Header(format!(
                "missing {} directive",
                DIRECTIVES[directive_index]
            )));
        }
        expect_tokens("VERSION", values[0], &["0.7"])?;
        let field_count = token_count(values[1]);
        if field_count == 0 {
            return Err(ReadError::Header("FIELDS must not be empty".into()));
        }
        for (name, value) in [
            ("SIZE", values[2]),
            ("TYPE", values[3]),
            ("COUNT", values[4]),
        ] {
            let actual = token_count(value);
            if actual != field_count {
                return Err(ReadError::Header(format!(
                    "{name} has {actual} values but FIELDS has {field_count}"
                )));
            }
        }
        let width = parse_single_usize("WIDTH", values[5])?;
        let height = parse_single_usize("HEIGHT", values[6])?;
        if height == 0 {
            return Err(ReadError::Header("HEIGHT must be positive".into()));
        }
        validate_viewpoint(values[7])?;
        let points = parse_single_usize("POINTS", values[8])?;
        let expected_points = width
            .checked_mul(height)
            .ok_or(ReadError::ArithmeticOverflow {
                context: "WIDTH * HEIGHT",
            })?;
        if points != expected_points {
            return Err(ReadError::PointCount {
                declared: points,
                width,
                height,
                expected: expected_points,
            });
        }
        let encoding = match single_token("DATA", values[9])? {
            "ascii" => PayloadEncoding::Ascii,
            "binary" => PayloadEncoding::Binary,
            "binary_compressed" => {
                return Err(ReadError::UnsupportedEncoding("binary_compressed".into()));
            }
            other => return Err(ReadError::UnsupportedEncoding(other.into())),
        };
        let point_step = parse_field_layout(values[2], values[3], values[4])?;
        Ok(Self {
            values,
            width,
            height,
            points,
            point_step,
            field_count,
            encoding,
        })
    }

    fn plan(&self, header_bytes: usize, memory_limit_bytes: usize) -> Result<ReadPlan, ReadError> {
        let point_data_bytes =
            self.points
                .checked_mul(self.point_step)
                .ok_or(ReadError::ArithmeticOverflow {
                    context: "POINTS * point_step",
                })?;
        let field_tables = self
            .field_count
            .checked_mul(
                size_of::<PointField>() + size_of::<PointColumn>() + 2 * size_of::<usize>(),
            )
            .ok_or(ReadError::ArithmeticOverflow {
                context: "field table size",
            })?;
        let subtotal = header_bytes
            .checked_add(point_data_bytes)
            .and_then(|value| value.checked_add(field_tables))
            .and_then(|value| value.checked_add(FIXED_MANAGED_OVERHEAD))
            .ok_or(ReadError::ArithmeticOverflow {
                context: "managed-memory plan",
            })?;
        let proportional = subtotal / PROPORTIONAL_OVERHEAD_DIVISOR;
        let peak_managed_bytes =
            subtotal
                .checked_add(proportional)
                .ok_or(ReadError::ArithmeticOverflow {
                    context: "managed-memory plan",
                })?;
        if peak_managed_bytes > memory_limit_bytes {
            return Err(ReadError::MemoryLimit {
                required: peak_managed_bytes,
                available: memory_limit_bytes,
            });
        }
        Ok(ReadPlan {
            header_bytes,
            point_data_bytes,
            peak_managed_bytes,
            memory_limit_bytes,
        })
    }

    fn build_schema(&self) -> Result<BuiltSchema, ReadError> {
        let names = self.values[1].split_ascii_whitespace();
        let sizes = self.values[2].split_ascii_whitespace();
        let types = self.values[3].split_ascii_whitespace();
        let counts_iter = self.values[4].split_ascii_whitespace();
        let mut fields = Vec::new();
        let mut primitives = Vec::new();
        let mut counts = Vec::new();
        fields
            .try_reserve_exact(self.field_count)
            .map_err(|_| ReadError::Allocation)?;
        primitives
            .try_reserve_exact(self.field_count)
            .map_err(|_| ReadError::Allocation)?;
        counts
            .try_reserve_exact(self.field_count)
            .map_err(|_| ReadError::Allocation)?;
        for (((name, size), kind), count) in names.zip(sizes).zip(types).zip(counts_iter) {
            if !name.bytes().all(|byte| byte.is_ascii_graphic()) {
                return Err(ReadError::Header(format!("invalid field name {name:?}")));
            }
            let size = parse_usize("SIZE", size)?;
            let count = parse_usize("COUNT", count)?;
            if count == 0 {
                return Err(ReadError::Header(format!("field {name:?} has zero COUNT")));
            }
            let primitive = primitive(kind, size).ok_or_else(|| ReadError::UnsupportedField {
                name: name.into(),
                field_type: kind.into(),
                size,
            })?;
            let field = PointField::new(name, primitive, count, field_semantic(name))
                .map_err(ReadError::Schema)?;
            fields.push(field);
            primitives.push(primitive);
            counts.push(count);
        }
        let schema = PointSchema::new(fields).map_err(ReadError::Schema)?;
        Ok(BuiltSchema {
            schema: Arc::new(schema),
            primitives,
            counts,
        })
    }
}

fn parse_field_layout(sizes: &str, types: &str, counts: &str) -> Result<usize, ReadError> {
    let mut point_step = 0_usize;
    for ((size, kind), count) in sizes
        .split_ascii_whitespace()
        .zip(types.split_ascii_whitespace())
        .zip(counts.split_ascii_whitespace())
    {
        let size = parse_usize("SIZE", size)?;
        let count = parse_usize("COUNT", count)?;
        if count == 0 {
            return Err(ReadError::Header("COUNT values must be positive".into()));
        }
        if primitive(kind, size).is_none() {
            return Err(ReadError::Header(format!(
                "unsupported TYPE/SIZE pair {kind}/{size}"
            )));
        }
        let width = size
            .checked_mul(count)
            .ok_or(ReadError::ArithmeticOverflow {
                context: "SIZE * COUNT",
            })?;
        point_step = point_step
            .checked_add(width)
            .ok_or(ReadError::ArithmeticOverflow {
                context: "packed point size",
            })?;
    }
    Ok(point_step)
}

fn primitive(kind: &str, size: usize) -> Option<PrimitiveType> {
    match (kind, size) {
        ("I", 1) => Some(PrimitiveType::I8),
        ("U", 1) => Some(PrimitiveType::U8),
        ("I", 2) => Some(PrimitiveType::I16),
        ("U", 2) => Some(PrimitiveType::U16),
        ("I", 4) => Some(PrimitiveType::I32),
        ("U", 4) => Some(PrimitiveType::U32),
        ("F", 4) => Some(PrimitiveType::F32),
        ("F", 8) => Some(PrimitiveType::F64),
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

fn token_count(value: &str) -> usize {
    value.split_ascii_whitespace().count()
}

fn expect_tokens(name: &str, value: &str, expected: &[&str]) -> Result<(), ReadError> {
    if value.split_ascii_whitespace().eq(expected.iter().copied()) {
        Ok(())
    } else {
        Err(ReadError::Header(format!("invalid {name} value {value:?}")))
    }
}

fn validate_viewpoint(value: &str) -> Result<(), ReadError> {
    let expected = [0.0_f64, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
    let mut tokens = value.split_ascii_whitespace();
    for expected_value in expected {
        let token = tokens.next().ok_or(ReadError::UnsupportedViewpoint)?;
        let actual = token
            .parse::<f64>()
            .map_err(|_| ReadError::UnsupportedViewpoint)?;
        if !actual.is_finite() || actual != expected_value {
            return Err(ReadError::UnsupportedViewpoint);
        }
    }
    if tokens.next().is_some() {
        return Err(ReadError::UnsupportedViewpoint);
    }
    Ok(())
}

fn single_token<'a>(name: &str, value: &'a str) -> Result<&'a str, ReadError> {
    let mut tokens = value.split_ascii_whitespace();
    let token = tokens
        .next()
        .ok_or_else(|| ReadError::Header(format!("{name} requires one value")))?;
    if tokens.next().is_some() {
        return Err(ReadError::Header(format!(
            "{name} requires exactly one value"
        )));
    }
    Ok(token)
}

fn parse_single_usize(name: &str, value: &str) -> Result<usize, ReadError> {
    parse_usize(name, single_token(name, value)?)
}

fn parse_usize(name: &str, token: &str) -> Result<usize, ReadError> {
    if token.is_empty() || !token.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ReadError::Header(format!(
            "{name} value {token:?} is not an unsigned decimal integer"
        )));
    }
    token.parse().map_err(|_| ReadError::ArithmeticOverflow {
        context: "header integer",
    })
}

fn allocate_columns(
    primitives: &[PrimitiveType],
    counts: &[usize],
    points: usize,
) -> Result<Vec<PointColumn>, ReadError> {
    let mut columns = Vec::new();
    columns
        .try_reserve_exact(primitives.len())
        .map_err(|_| ReadError::Allocation)?;
    for (&primitive, &count) in primitives.iter().zip(counts) {
        let len = points
            .checked_mul(count)
            .ok_or(ReadError::ArithmeticOverflow {
                context: "POINTS * COUNT",
            })?;
        macro_rules! column {
            ($variant:ident, $type:ty) => {{
                let mut values: Vec<$type> = Vec::new();
                values
                    .try_reserve_exact(len)
                    .map_err(|_| ReadError::Allocation)?;
                PointColumn::$variant(values)
            }};
        }
        columns.push(match primitive {
            PrimitiveType::I8 => column!(I8, i8),
            PrimitiveType::U8 => column!(U8, u8),
            PrimitiveType::I16 => column!(I16, i16),
            PrimitiveType::U16 => column!(U16, u16),
            PrimitiveType::I32 => column!(I32, i32),
            PrimitiveType::U32 => column!(U32, u32),
            PrimitiveType::F32 => column!(F32, f32),
            PrimitiveType::F64 => column!(F64, f64),
            PrimitiveType::I64 | PrimitiveType::U64 => {
                return Err(ReadError::Payload(
                    "PCD reader encountered an unsupported 64-bit integer field".into(),
                ));
            }
        });
    }
    Ok(columns)
}

fn read_ascii_payload(
    input: &mut impl io::Read,
    primitives: &[PrimitiveType],
    counts: &[usize],
    columns: &mut [PointColumn],
    points: usize,
) -> Result<(), ReadError> {
    let mut token = [0_u8; MAX_ASCII_TOKEN_BYTES];
    for point in 0..points {
        for (field, (&primitive, &count)) in primitives.iter().zip(counts).enumerate() {
            for element in 0..count {
                let value =
                    next_ascii_token(input, &mut token)?.ok_or(ReadError::PayloadTruncated {
                        point,
                        field,
                        element,
                    })?;
                push_ascii(&mut columns[field], primitive, value, point, field, element)?;
            }
        }
    }
    if let Some(extra) = next_ascii_token(input, &mut token)? {
        return Err(ReadError::TrailingPayload(format!(
            "extra ASCII value {extra:?}"
        )));
    }
    Ok(())
}

fn next_ascii_token<'a>(
    input: &mut impl io::Read,
    storage: &'a mut [u8],
) -> Result<Option<&'a str>, ReadError> {
    let mut len = 0_usize;
    let mut byte = [0_u8; 1];
    loop {
        let read = input.read(&mut byte).map_err(ReadError::Io)?;
        if read == 0 {
            break;
        }
        if byte[0].is_ascii_whitespace() {
            if len == 0 {
                continue;
            }
            break;
        }
        if !byte[0].is_ascii() {
            return Err(ReadError::Payload(
                "ASCII payload contains a non-ASCII byte".into(),
            ));
        }
        if len == storage.len() {
            return Err(ReadError::AsciiTokenTooLarge {
                maximum: storage.len(),
            });
        }
        storage[len] = byte[0];
        len += 1;
    }
    if len == 0 {
        return Ok(None);
    }
    let value = std::str::from_utf8(&storage[..len])
        .map_err(|_| ReadError::Payload("invalid ASCII token".into()))?;
    Ok(Some(value))
}

fn push_ascii(
    column: &mut PointColumn,
    primitive: PrimitiveType,
    token: &str,
    point: usize,
    field: usize,
    element: usize,
) -> Result<(), ReadError> {
    macro_rules! parse_push {
        ($values:expr, $type:ty) => {{
            let value = token
                .parse::<$type>()
                .map_err(|_| ReadError::InvalidValue {
                    token: token.into(),
                    point,
                    field,
                    element,
                    primitive,
                })?;
            $values.push(value);
        }};
    }
    match (primitive, column) {
        (PrimitiveType::I8, PointColumn::I8(values)) => parse_push!(values, i8),
        (PrimitiveType::U8, PointColumn::U8(values)) => parse_push!(values, u8),
        (PrimitiveType::I16, PointColumn::I16(values)) => parse_push!(values, i16),
        (PrimitiveType::U16, PointColumn::U16(values)) => parse_push!(values, u16),
        (PrimitiveType::I32, PointColumn::I32(values)) => parse_push!(values, i32),
        (PrimitiveType::U32, PointColumn::U32(values)) => parse_push!(values, u32),
        (PrimitiveType::F32, PointColumn::F32(values)) => parse_push!(values, f32),
        (PrimitiveType::F64, PointColumn::F64(values)) => parse_push!(values, f64),
        _ => {
            return Err(ReadError::Payload(
                "point column type does not match its PCD field".into(),
            ));
        }
    }
    Ok(())
}

fn read_binary_payload(
    input: &mut impl io::Read,
    primitives: &[PrimitiveType],
    counts: &[usize],
    columns: &mut [PointColumn],
    points: usize,
) -> Result<(), ReadError> {
    let mut bytes = [0_u8; 8];
    for point in 0..points {
        for (field, (&primitive, &count)) in primitives.iter().zip(counts).enumerate() {
            for element in 0..count {
                let width = primitive.size();
                input.read_exact(&mut bytes[..width]).map_err(|error| {
                    if error.kind() == io::ErrorKind::UnexpectedEof {
                        ReadError::PayloadTruncated {
                            point,
                            field,
                            element,
                        }
                    } else {
                        ReadError::Io(error)
                    }
                })?;
                push_binary(&mut columns[field], primitive, &bytes[..width])?;
            }
        }
    }
    let mut extra = [0_u8; 1];
    if input.read(&mut extra).map_err(ReadError::Io)? != 0 {
        return Err(ReadError::TrailingPayload(
            "binary payload exceeds declared size".into(),
        ));
    }
    Ok(())
}

fn push_binary(
    column: &mut PointColumn,
    primitive: PrimitiveType,
    bytes: &[u8],
) -> Result<(), ReadError> {
    macro_rules! value {
        ($type:ty) => {{
            let array: [u8; size_of::<$type>()] = bytes.try_into().map_err(|_| {
                ReadError::Payload("scalar byte width does not match its PCD field".into())
            })?;
            <$type>::from_le_bytes(array)
        }};
    }
    match (primitive, column) {
        (PrimitiveType::I8, PointColumn::I8(values)) => values.push(value!(i8)),
        (PrimitiveType::U8, PointColumn::U8(values)) => values.push(value!(u8)),
        (PrimitiveType::I16, PointColumn::I16(values)) => values.push(value!(i16)),
        (PrimitiveType::U16, PointColumn::U16(values)) => values.push(value!(u16)),
        (PrimitiveType::I32, PointColumn::I32(values)) => values.push(value!(i32)),
        (PrimitiveType::U32, PointColumn::U32(values)) => values.push(value!(u32)),
        (PrimitiveType::F32, PointColumn::F32(values)) => {
            values.push(f32::from_bits(value!(u32)));
        }
        (PrimitiveType::F64, PointColumn::F64(values)) => {
            values.push(f64::from_bits(value!(u64)));
        }
        _ => {
            return Err(ReadError::Payload(
                "point column type does not match its PCD field".into(),
            ));
        }
    }
    Ok(())
}

/// Malformed, unsupported, unplannable, or unreadable PCD input.
#[derive(Debug)]
pub enum ReadError {
    Io(io::Error),
    Header(String),
    HeaderTooLarge {
        maximum: usize,
    },
    UnsupportedEncoding(String),
    UnsupportedViewpoint,
    UnsupportedField {
        name: String,
        field_type: String,
        size: usize,
    },
    PointCount {
        declared: usize,
        width: usize,
        height: usize,
        expected: usize,
    },
    ArithmeticOverflow {
        context: &'static str,
    },
    MemoryLimit {
        required: usize,
        available: usize,
    },
    Allocation,
    Schema(SchemaError),
    Metadata(TimestampError),
    Dimensions(crate::core::point::LayoutError),
    Payload(String),
    PayloadTruncated {
        point: usize,
        field: usize,
        element: usize,
    },
    AsciiTokenTooLarge {
        maximum: usize,
    },
    InvalidValue {
        token: String,
        point: usize,
        field: usize,
        element: usize,
        primitive: PrimitiveType,
    },
    TrailingPayload(String),
    Batch(BatchError),
}

impl fmt::Display for ReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "failed to read PCD: {error}"),
            Self::Header(reason) => write!(formatter, "invalid PCD header: {reason}"),
            Self::HeaderTooLarge { maximum } => write!(formatter, "PCD header exceeds {maximum} bytes"),
            Self::UnsupportedEncoding(encoding) => write!(formatter, "unsupported PCD DATA encoding {encoding:?}"),
            Self::UnsupportedViewpoint => formatter.write_str("unsupported PCD VIEWPOINT: the common point model cannot preserve a non-default viewpoint"),
            Self::UnsupportedField { name, field_type, size } => write!(formatter, "unsupported PCD field {name:?}: TYPE {field_type} SIZE {size}"),
            Self::PointCount { declared, width, height, expected } => write!(formatter, "invalid PCD POINTS {declared}: WIDTH {width} * HEIGHT {height} = {expected}"),
            Self::ArithmeticOverflow { context } => write!(formatter, "PCD {context} overflows this platform's address space"),
            Self::MemoryLimit { required, available } => write!(formatter, "planned PCD managed-memory peak of {required} bytes exceeds the {available}-byte limit"),
            Self::Allocation => formatter.write_str("could not allocate planned PCD point storage"),
            Self::Schema(error) => write!(formatter, "invalid PCD field schema: {error}"),
            Self::Metadata(error) => write!(formatter, "invalid PCD metadata: {error}"),
            Self::Dimensions(error) => write!(formatter, "invalid PCD dimensions: {error}"),
            Self::Payload(reason) => write!(formatter, "invalid PCD payload: {reason}"),
            Self::PayloadTruncated { point, field, element } => write!(formatter, "truncated PCD payload at point {point}, field {field}, element {element}"),
            Self::AsciiTokenTooLarge { maximum } => write!(formatter, "PCD ASCII value exceeds {maximum} bytes"),
            Self::InvalidValue { token, point, field, element, primitive } => write!(formatter, "invalid {primitive:?} value {token:?} at point {point}, field {field}, element {element}"),
            Self::TrailingPayload(reason) => write!(formatter, "invalid PCD trailing payload: {reason}"),
            Self::Batch(error) => write!(formatter, "invalid decoded PCD point batch: {error}"),
        }
    }
}

impl error::Error for ReadError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Schema(error) => Some(error),
            Self::Metadata(error) => Some(error),
            Self::Dimensions(error) => Some(error),
            Self::Batch(error) => Some(error),
            _ => None,
        }
    }
}
