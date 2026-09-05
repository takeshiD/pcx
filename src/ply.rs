//! Faithful scalar-vertex subset of the PLY 1.0 point-cloud format.
//!
//! The adapter accepts ASCII, `binary_little_endian`, and
//! `binary_big_endian` files containing exactly one `vertex` element. Every
//! vertex property must be scalar and use one of `char`, `uchar`, `short`,
//! `ushort`, `int`, `uint`, `float`, or `double`. Property names, primitive
//! types, values, and source order are preserved through the common schema.
//! Lists, other elements (including faces), 64-bit integers, vector fields,
//! and organized dimensions are rejected instead of being discarded or
//! flattened.
//!
//! Reading is deliberately two-phase. [`Reader::new`] consumes only a bounded
//! header and exposes a [`ReadPlan`] with the exact column allocation. Point
//! data is not materialized until [`Reader::read`] receives a sufficient
//! caller-controlled budget. All I/O is synchronous and uses a fixed-size
//! buffered reader; no complete encoded file is buffered.

use crate::core::{
    ByteBound, PipelineMemoryRequirements,
    point::{
        AccessError, MaterializationBudget, PointBatch, PointColumn, PointDimensions, PointField,
        PointFieldSemantic, PointFrameMetadata, PointSchema, PointValue, PointView, PrimitiveType,
        SchemaError, Timestamp,
    },
};
use std::{
    error, fmt,
    io::{self, BufRead, BufReader, Read, Write},
    mem::size_of,
    sync::Arc,
};

/// Maximum accepted PLY header size, including line endings.
pub const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_HEADER_LINE_BYTES: usize = 4 * 1024;
const MAX_ASCII_TOKEN_BYTES: usize = 256;
const IO_BUFFER_BYTES: usize = 8 * 1024;

/// PLY payload representation supported by this adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Encoding {
    Ascii,
    BinaryLittleEndian,
    BinaryBigEndian,
}

impl Encoding {
    const fn keyword(self) -> &'static str {
        match self {
            Self::Ascii => "ascii",
            Self::BinaryLittleEndian => "binary_little_endian",
            Self::BinaryBigEndian => "binary_big_endian",
        }
    }
}

/// Header-derived proof available before any point values are allocated.
#[derive(Clone, Debug)]
pub struct ReadPlan {
    encoding: Encoding,
    schema: Arc<PointSchema>,
    vertex_count: usize,
    record_bytes: usize,
    materialization_bytes: usize,
    schema_storage_bytes: usize,
}

impl ReadPlan {
    pub const fn encoding(&self) -> Encoding {
        self.encoding
    }

    pub fn schema(&self) -> &PointSchema {
        &self.schema
    }

    pub const fn vertex_count(&self) -> usize {
        self.vertex_count
    }

    /// Bytes in one packed binary vertex record.
    pub const fn record_bytes(&self) -> usize {
        self.record_bytes
    }

    /// Exact additional allocation used for column values and their table.
    pub const fn materialization_bytes(&self) -> usize {
        self.materialization_bytes
    }

    /// Conservative inputs for the common managed-memory planner.
    ///
    /// The retained-input component covers the fixed I/O buffer plus the
    /// maximum bounded header/schema storage. PLY decoding has no hidden
    /// operator, encoder, output, or queue allocation.
    pub fn pipeline_memory_requirements(&self) -> Result<PipelineMemoryRequirements, Error> {
        let retained = MAX_HEADER_BYTES
            .checked_add(IO_BUFFER_BYTES)
            .and_then(|bytes| bytes.checked_add(self.schema_storage_bytes))
            .ok_or(Error::SizeOverflow)?;
        let retained = u64::try_from(retained).map_err(|_| Error::SizeOverflow)?;
        let materialization =
            u64::try_from(self.materialization_bytes).map_err(|_| Error::SizeOverflow)?;
        Ok(PipelineMemoryRequirements::new(
            ByteBound::bounded(retained),
            ByteBound::bounded(materialization),
            ByteBound::bounded(0),
            ByteBound::bounded(0),
            ByteBound::bounded(0),
            ByteBound::bounded(0),
        ))
    }
}

/// A bounded synchronous PLY reader positioned immediately after its header.
pub struct Reader<R> {
    input: BufReader<R>,
    plan: ReadPlan,
}

impl<R: Read> Reader<R> {
    /// Parse and validate a bounded header without allocating point columns.
    pub fn new(input: R) -> Result<Self, Error> {
        let mut input = BufReader::with_capacity(IO_BUFFER_BYTES, input);
        let plan = parse_header(&mut input)?;
        Ok(Self { input, plan })
    }

    pub const fn plan(&self) -> &ReadPlan {
        &self.plan
    }

    /// Materialize the planned vertex values through the common point schema.
    ///
    /// The budget is checked before any column allocation. PLY has no frame
    /// metadata or organized shape, so reads use the documented static-cloud
    /// defaults: timestamp zero, empty frame id, `is_dense = false`, width
    /// equal to the vertex count, and height one.
    pub fn read(mut self, budget: MaterializationBudget) -> Result<PointBatch, Error> {
        if self.plan.materialization_bytes > budget.bytes() {
            return Err(Error::BudgetExceeded {
                required: self.plan.materialization_bytes,
                available: budget.bytes(),
            });
        }

        let mut columns = allocate_columns(&self.plan)?;
        match self.plan.encoding {
            Encoding::Ascii => read_ascii(&mut self.input, &self.plan, &mut columns)?,
            Encoding::BinaryLittleEndian => {
                read_binary(&mut self.input, &self.plan, &mut columns, ByteOrder::Little)?
            }
            Encoding::BinaryBigEndian => {
                read_binary(&mut self.input, &self.plan, &mut columns, ByteOrder::Big)?
            }
        }

        let dimensions = PointDimensions::new(self.plan.vertex_count, 1)
            .map_err(|error| Error::InvalidDimensions(error.to_string()))?;
        let metadata = Arc::new(PointFrameMetadata::new(
            Timestamp::new(0, 0).expect("zero is a canonical timestamp"),
            "",
            false,
        ));
        PointBatch::new(Arc::clone(&self.plan.schema), metadata, dimensions, columns)
            .map_err(|error| Error::InvalidBatch(error.to_string()))
    }
}

/// Either common point representation accepted by the writer.
#[derive(Clone, Copy, Debug)]
pub enum PointDataRef<'a> {
    View(&'a PointView),
    Batch(&'a PointBatch),
}

impl<'a> From<&'a PointView> for PointDataRef<'a> {
    fn from(value: &'a PointView) -> Self {
        Self::View(value)
    }
}

impl<'a> From<&'a PointBatch> for PointDataRef<'a> {
    fn from(value: &'a PointBatch) -> Self {
        Self::Batch(value)
    }
}

impl<'a> PointDataRef<'a> {
    fn schema(self) -> &'a PointSchema {
        match self {
            Self::View(view) => view.schema(),
            Self::Batch(batch) => batch.schema(),
        }
    }

    fn dimensions(self) -> PointDimensions {
        match self {
            Self::View(view) => view.layout().dimensions(),
            Self::Batch(batch) => batch.dimensions(),
        }
    }

    fn metadata(self) -> &'a PointFrameMetadata {
        match self {
            Self::View(view) => view.metadata(),
            Self::Batch(batch) => batch.metadata(),
        }
    }

    fn value(self, field: usize, point: usize) -> Result<PointValue, Error> {
        match self {
            Self::View(view) => view
                .field(view.schema().fields()[field].name())
                .expect("field belongs to the validated view schema")
                .value(point, 0)
                .map_err(Error::Access),
            Self::Batch(batch) => Ok(batch_value(&batch.columns()[field], point)),
        }
    }
}

/// Validate and write one complete PLY file.
///
/// Compatibility, including every ASCII value, is checked before the first
/// byte is written. Binary floats preserve their exact bit patterns. ASCII
/// rejects NaN and infinity because PLY 1.0 does not define portable spellings
/// for them; finite values use shortest round-tripping decimal formatting.
pub fn write<'a>(
    output: &mut impl Write,
    points: impl Into<PointDataRef<'a>>,
    encoding: Encoding,
) -> Result<(), Error> {
    let points = points.into();
    validate_points(points)?;
    if encoding == Encoding::Ascii {
        validate_ascii_values(points)?;
    }
    write_header(
        output,
        points.schema(),
        points.dimensions().point_count(),
        encoding,
    )?;
    match encoding {
        Encoding::Ascii => write_ascii(output, points),
        Encoding::BinaryLittleEndian => write_binary(output, points, ByteOrder::Little),
        Encoding::BinaryBigEndian => write_binary(output, points, ByteOrder::Big),
    }
}

/// Check that a common point representation maps faithfully to the PLY subset.
pub fn validate<'a>(points: impl Into<PointDataRef<'a>>) -> Result<(), Error> {
    validate_points(points.into())
}

fn validate_points(points: PointDataRef<'_>) -> Result<(), Error> {
    let dimensions = points.dimensions();
    if dimensions.height() != 1 {
        return Err(Error::OrganizedCloud {
            width: dimensions.width(),
            height: dimensions.height(),
        });
    }
    let metadata = points.metadata();
    if metadata.sensor_timestamp() != Timestamp::new(0, 0).expect("canonical zero timestamp")
        || !metadata.frame_id().is_empty()
        || metadata.is_dense()
        || metadata.log_time_ns().is_some()
        || metadata.publish_time_ns().is_some()
    {
        return Err(Error::UnrepresentableMetadata);
    }
    let schema = points.schema();
    if schema.fields().is_empty() {
        return Err(Error::EmptySchema);
    }
    for field in schema.fields() {
        if !valid_name(field.name()) {
            return Err(Error::UnrepresentablePropertyName {
                name: field.name().to_owned(),
            });
        }
        if field.count() != 1 {
            return Err(Error::ListLikeField {
                name: field.name().to_owned(),
                count: field.count(),
            });
        }
        if matches!(field.primitive(), PrimitiveType::I64 | PrimitiveType::U64) {
            return Err(Error::UnsupportedPrimitive {
                name: field.name().to_owned(),
                primitive: field.primitive(),
            });
        }
        if field.semantic() != semantic_for_name(field.name()) {
            return Err(Error::UnrepresentableSemantic {
                name: field.name().to_owned(),
                semantic: field.semantic(),
            });
        }
    }
    Ok(())
}

fn validate_ascii_values(points: PointDataRef<'_>) -> Result<(), Error> {
    for point in 0..points.dimensions().point_count() {
        for (field_index, field) in points.schema().fields().iter().enumerate() {
            match points.value(field_index, point)? {
                PointValue::F32(value) if !value.is_finite() => {
                    return Err(Error::NonFiniteAscii {
                        name: field.name().to_owned(),
                        point,
                    });
                }
                PointValue::F64(value) if !value.is_finite() => {
                    return Err(Error::NonFiniteAscii {
                        name: field.name().to_owned(),
                        point,
                    });
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn parse_header(input: &mut impl BufRead) -> Result<ReadPlan, Error> {
    let mut total = 0usize;
    let first = read_header_line(input, &mut total)?.ok_or(Error::MissingMagic)?;
    if first != b"ply" {
        return Err(Error::MissingMagic);
    }

    let mut encoding = None;
    let mut vertex_count = None;
    let mut fields = Vec::new();
    let mut in_vertex = false;
    loop {
        let line = read_header_line(input, &mut total)?.ok_or(Error::MissingEndHeader)?;
        if !line.is_ascii() {
            return Err(Error::NonAsciiHeader);
        }
        let text = std::str::from_utf8(&line).expect("ASCII is valid UTF-8");
        let tokens: Vec<&str> = text.split_ascii_whitespace().collect();
        let Some(&directive) = tokens.first() else {
            return Err(Error::InvalidHeader("blank header line".to_owned()));
        };
        match directive {
            "comment" | "obj_info" => {}
            "format" => {
                if encoding.is_some() || tokens.len() != 3 || tokens[2] != "1.0" {
                    return Err(Error::InvalidFormat(text.to_owned()));
                }
                encoding = Some(match tokens[1] {
                    "ascii" => Encoding::Ascii,
                    "binary_little_endian" => Encoding::BinaryLittleEndian,
                    "binary_big_endian" => Encoding::BinaryBigEndian,
                    _ => return Err(Error::InvalidFormat(text.to_owned())),
                });
            }
            "element" => {
                if encoding.is_none() || tokens.len() != 3 {
                    return Err(Error::InvalidHeader(text.to_owned()));
                }
                if tokens[1] != "vertex" || vertex_count.is_some() {
                    return Err(Error::UnsupportedElement(tokens[1].to_owned()));
                }
                let count = tokens[2]
                    .parse::<usize>()
                    .map_err(|_| Error::InvalidVertexCount(tokens[2].to_owned()))?;
                vertex_count = Some(count);
                in_vertex = true;
            }
            "property" => {
                if !in_vertex {
                    return Err(Error::InvalidHeader(text.to_owned()));
                }
                if tokens.get(1) == Some(&"list") {
                    return Err(Error::ListProperty(text.to_owned()));
                }
                if tokens.len() != 3 || !valid_name(tokens[2]) {
                    return Err(Error::InvalidProperty(text.to_owned()));
                }
                let primitive = primitive_from_keyword(tokens[1])
                    .ok_or_else(|| Error::UnsupportedPropertyType(tokens[1].to_owned()))?;
                fields.push(
                    PointField::new(tokens[2], primitive, 1, semantic_for_name(tokens[2]))
                        .map_err(Error::InvalidSchema)?,
                );
            }
            "end_header" if tokens.len() == 1 => break,
            _ => return Err(Error::InvalidHeader(text.to_owned())),
        }
    }

    let encoding = encoding.ok_or_else(|| Error::InvalidHeader("missing format".to_owned()))?;
    let vertex_count =
        vertex_count.ok_or_else(|| Error::InvalidHeader("missing vertex element".to_owned()))?;
    if fields.is_empty() {
        return Err(Error::EmptySchema);
    }
    let schema_storage_bytes = fields
        .len()
        .checked_mul(size_of::<PointField>())
        .and_then(|table| {
            fields
                .iter()
                .try_fold(table, |total, field| total.checked_add(field.name().len()))
        })
        .ok_or(Error::SizeOverflow)?;
    let schema = Arc::new(PointSchema::new(fields).map_err(Error::InvalidSchema)?);
    let record_bytes = schema
        .fields()
        .iter()
        .try_fold(0usize, |total, field| {
            total.checked_add(field.primitive().size())
        })
        .ok_or(Error::SizeOverflow)?;
    record_bytes
        .checked_mul(vertex_count)
        .ok_or(Error::SizeOverflow)?;
    let value_bytes = schema
        .fields()
        .iter()
        .try_fold(0usize, |total, field| {
            vertex_count
                .checked_mul(field.primitive().size())
                .and_then(|bytes| total.checked_add(bytes))
        })
        .ok_or(Error::SizeOverflow)?;
    let table_bytes = schema
        .fields()
        .len()
        .checked_mul(size_of::<PointColumn>())
        .ok_or(Error::SizeOverflow)?;
    let materialization_bytes = value_bytes
        .checked_add(table_bytes)
        .ok_or(Error::SizeOverflow)?;
    Ok(ReadPlan {
        encoding,
        schema,
        vertex_count,
        record_bytes,
        materialization_bytes,
        schema_storage_bytes,
    })
}

fn read_header_line(input: &mut impl BufRead, total: &mut usize) -> Result<Option<Vec<u8>>, Error> {
    let mut line = Vec::new();
    loop {
        let available = input.fill_buf()?;
        if available.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            return Err(Error::InvalidHeader("unterminated header line".to_owned()));
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        *total = total.checked_add(take).ok_or(Error::SizeOverflow)?;
        if *total > MAX_HEADER_BYTES || line.len().saturating_add(take) > MAX_HEADER_LINE_BYTES {
            return Err(Error::HeaderTooLarge);
        }
        line.extend_from_slice(&available[..take]);
        input.consume(take);
        if line.last() == Some(&b'\n') {
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return Ok(Some(line));
        }
    }
}

fn allocate_columns(plan: &ReadPlan) -> Result<Vec<PointColumn>, Error> {
    let mut columns = Vec::new();
    columns
        .try_reserve_exact(plan.schema.fields().len())
        .map_err(|_| Error::AllocationFailed(plan.materialization_bytes))?;
    for field in plan.schema.fields() {
        macro_rules! column {
            ($variant:ident, $type:ty) => {{
                let mut values: Vec<$type> = Vec::new();
                values
                    .try_reserve_exact(plan.vertex_count)
                    .map_err(|_| Error::AllocationFailed(plan.materialization_bytes))?;
                PointColumn::$variant(values)
            }};
        }
        columns.push(match field.primitive() {
            PrimitiveType::I8 => column!(I8, i8),
            PrimitiveType::U8 => column!(U8, u8),
            PrimitiveType::I16 => column!(I16, i16),
            PrimitiveType::U16 => column!(U16, u16),
            PrimitiveType::I32 => column!(I32, i32),
            PrimitiveType::U32 => column!(U32, u32),
            PrimitiveType::F32 => column!(F32, f32),
            PrimitiveType::F64 => column!(F64, f64),
            PrimitiveType::I64 | PrimitiveType::U64 => unreachable!("header rejected type"),
        });
    }
    Ok(columns)
}

fn read_ascii(
    input: &mut impl BufRead,
    plan: &ReadPlan,
    columns: &mut [PointColumn],
) -> Result<(), Error> {
    for point in 0..plan.vertex_count {
        for (field_index, field) in plan.schema.fields().iter().enumerate() {
            let token = next_ascii_token(input)?.ok_or(Error::TruncatedPayload {
                point,
                field: field.name().to_owned(),
            })?;
            let text = std::str::from_utf8(&token).map_err(|_| Error::InvalidAsciiValue {
                point,
                field: field.name().to_owned(),
                value: "non-ASCII token".to_owned(),
            })?;
            let value = parse_ascii_value(field.primitive(), text).ok_or_else(|| {
                Error::InvalidAsciiValue {
                    point,
                    field: field.name().to_owned(),
                    value: text.to_owned(),
                }
            })?;
            if matches!(value, PointValue::F32(value) if !value.is_finite())
                || matches!(value, PointValue::F64(value) if !value.is_finite())
            {
                return Err(Error::NonFiniteAscii {
                    name: field.name().to_owned(),
                    point,
                });
            }
            push_value(&mut columns[field_index], value);
        }
    }
    if next_ascii_token(input)?.is_some() {
        return Err(Error::TrailingData);
    }
    Ok(())
}

fn next_ascii_token(input: &mut impl BufRead) -> Result<Option<Vec<u8>>, Error> {
    let mut token = Vec::new();
    loop {
        let available = input.fill_buf()?;
        if available.is_empty() {
            return Ok((!token.is_empty()).then_some(token));
        }
        let mut consumed = 0;
        while consumed < available.len() {
            let byte = available[consumed];
            consumed += 1;
            if byte.is_ascii_whitespace() {
                if !token.is_empty() {
                    input.consume(consumed);
                    return Ok(Some(token));
                }
            } else {
                if token.len() == MAX_ASCII_TOKEN_BYTES {
                    return Err(Error::AsciiTokenTooLarge);
                }
                token.push(byte);
            }
        }
        input.consume(consumed);
    }
}

#[derive(Clone, Copy)]
enum ByteOrder {
    Little,
    Big,
}

fn read_binary(
    input: &mut impl Read,
    plan: &ReadPlan,
    columns: &mut [PointColumn],
    order: ByteOrder,
) -> Result<(), Error> {
    for point in 0..plan.vertex_count {
        for (field_index, field) in plan.schema.fields().iter().enumerate() {
            let value = read_binary_value(input, field.primitive(), order).map_err(|error| {
                if error.kind() == io::ErrorKind::UnexpectedEof {
                    Error::TruncatedPayload {
                        point,
                        field: field.name().to_owned(),
                    }
                } else {
                    Error::Io(error)
                }
            })?;
            push_value(&mut columns[field_index], value);
        }
    }
    let mut trailing = [0];
    if input.read(&mut trailing)? != 0 {
        return Err(Error::TrailingData);
    }
    Ok(())
}

fn read_binary_value(
    input: &mut impl Read,
    primitive: PrimitiveType,
    order: ByteOrder,
) -> io::Result<PointValue> {
    macro_rules! scalar {
        ($type:ty, $variant:ident) => {{
            let mut bytes = [0; size_of::<$type>()];
            input.read_exact(&mut bytes)?;
            let value = match order {
                ByteOrder::Little => <$type>::from_le_bytes(bytes),
                ByteOrder::Big => <$type>::from_be_bytes(bytes),
            };
            PointValue::$variant(value)
        }};
    }
    Ok(match primitive {
        PrimitiveType::I8 => {
            let mut bytes = [0];
            input.read_exact(&mut bytes)?;
            PointValue::I8(bytes[0] as i8)
        }
        PrimitiveType::U8 => {
            let mut bytes = [0];
            input.read_exact(&mut bytes)?;
            PointValue::U8(bytes[0])
        }
        PrimitiveType::I16 => scalar!(i16, I16),
        PrimitiveType::U16 => scalar!(u16, U16),
        PrimitiveType::I32 => scalar!(i32, I32),
        PrimitiveType::U32 => scalar!(u32, U32),
        PrimitiveType::F32 => {
            let PointValue::U32(bits) = scalar!(u32, U32) else {
                unreachable!()
            };
            PointValue::F32(f32::from_bits(bits))
        }
        PrimitiveType::F64 => {
            let PointValue::U64(bits) = scalar!(u64, U64) else {
                unreachable!()
            };
            PointValue::F64(f64::from_bits(bits))
        }
        PrimitiveType::I64 | PrimitiveType::U64 => unreachable!("header rejected type"),
    })
}

fn parse_ascii_value(primitive: PrimitiveType, value: &str) -> Option<PointValue> {
    macro_rules! parse {
        ($type:ty, $variant:ident) => {
            value.parse::<$type>().ok().map(PointValue::$variant)
        };
    }
    match primitive {
        PrimitiveType::I8 => parse!(i8, I8),
        PrimitiveType::U8 => parse!(u8, U8),
        PrimitiveType::I16 => parse!(i16, I16),
        PrimitiveType::U16 => parse!(u16, U16),
        PrimitiveType::I32 => parse!(i32, I32),
        PrimitiveType::U32 => parse!(u32, U32),
        PrimitiveType::F32 => parse!(f32, F32),
        PrimitiveType::F64 => parse!(f64, F64),
        PrimitiveType::I64 | PrimitiveType::U64 => None,
    }
}

fn push_value(column: &mut PointColumn, value: PointValue) {
    macro_rules! push {
        ($column:ident, $value:ident, $variant:ident) => {
            if let (PointColumn::$variant(values), PointValue::$variant(value)) = ($column, $value)
            {
                values.push(value);
            } else {
                unreachable!("schema and value type agree")
            }
        };
    }
    match value {
        PointValue::I8(_) => push!(column, value, I8),
        PointValue::U8(_) => push!(column, value, U8),
        PointValue::I16(_) => push!(column, value, I16),
        PointValue::U16(_) => push!(column, value, U16),
        PointValue::I32(_) => push!(column, value, I32),
        PointValue::U32(_) => push!(column, value, U32),
        PointValue::F32(_) => push!(column, value, F32),
        PointValue::F64(_) => push!(column, value, F64),
        PointValue::I64(_) | PointValue::U64(_) => unreachable!("unsupported PLY primitive"),
    }
}

fn write_header(
    output: &mut impl Write,
    schema: &PointSchema,
    points: usize,
    encoding: Encoding,
) -> Result<(), Error> {
    writeln!(output, "ply")?;
    writeln!(output, "format {} 1.0", encoding.keyword())?;
    writeln!(output, "comment generated by pcx")?;
    writeln!(output, "element vertex {points}")?;
    for field in schema.fields() {
        writeln!(
            output,
            "property {} {}",
            primitive_keyword(field.primitive()),
            field.name()
        )?;
    }
    writeln!(output, "end_header")?;
    Ok(())
}

fn write_ascii(output: &mut impl Write, points: PointDataRef<'_>) -> Result<(), Error> {
    for point in 0..points.dimensions().point_count() {
        for field in 0..points.schema().fields().len() {
            if field != 0 {
                output.write_all(b" ")?;
            }
            write!(output, "{}", DisplayValue(points.value(field, point)?))?;
        }
        output.write_all(b"\n")?;
    }
    Ok(())
}

struct DisplayValue(PointValue);
impl fmt::Display for DisplayValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            PointValue::I8(v) => write!(formatter, "{v}"),
            PointValue::U8(v) => write!(formatter, "{v}"),
            PointValue::I16(v) => write!(formatter, "{v}"),
            PointValue::U16(v) => write!(formatter, "{v}"),
            PointValue::I32(v) => write!(formatter, "{v}"),
            PointValue::U32(v) => write!(formatter, "{v}"),
            PointValue::F32(v) => write!(formatter, "{v}"),
            PointValue::F64(v) => write!(formatter, "{v}"),
            PointValue::I64(_) | PointValue::U64(_) => unreachable!("unsupported PLY primitive"),
        }
    }
}

fn write_binary(
    output: &mut impl Write,
    points: PointDataRef<'_>,
    order: ByteOrder,
) -> Result<(), Error> {
    for point in 0..points.dimensions().point_count() {
        for field in 0..points.schema().fields().len() {
            write_binary_value(output, points.value(field, point)?, order)?;
        }
    }
    Ok(())
}

fn write_binary_value(
    output: &mut impl Write,
    value: PointValue,
    order: ByteOrder,
) -> Result<(), Error> {
    macro_rules! bytes {
        ($value:expr) => {
            match order {
                ByteOrder::Little => $value.to_le_bytes(),
                ByteOrder::Big => $value.to_be_bytes(),
            }
        };
    }
    match value {
        PointValue::I8(v) => output.write_all(&[v as u8])?,
        PointValue::U8(v) => output.write_all(&[v])?,
        PointValue::I16(v) => output.write_all(&bytes!(v))?,
        PointValue::U16(v) => output.write_all(&bytes!(v))?,
        PointValue::I32(v) => output.write_all(&bytes!(v))?,
        PointValue::U32(v) => output.write_all(&bytes!(v))?,
        PointValue::F32(v) => output.write_all(&bytes!(v.to_bits()))?,
        PointValue::F64(v) => output.write_all(&bytes!(v.to_bits()))?,
        PointValue::I64(_) | PointValue::U64(_) => unreachable!("unsupported PLY primitive"),
    }
    Ok(())
}

fn batch_value(column: &PointColumn, point: usize) -> PointValue {
    match column {
        PointColumn::I8(v) => PointValue::I8(v[point]),
        PointColumn::U8(v) => PointValue::U8(v[point]),
        PointColumn::I16(v) => PointValue::I16(v[point]),
        PointColumn::U16(v) => PointValue::U16(v[point]),
        PointColumn::I32(v) => PointValue::I32(v[point]),
        PointColumn::U32(v) => PointValue::U32(v[point]),
        PointColumn::I64(v) => PointValue::I64(v[point]),
        PointColumn::U64(v) => PointValue::U64(v[point]),
        PointColumn::F32(v) => PointValue::F32(v[point]),
        PointColumn::F64(v) => PointValue::F64(v[point]),
    }
}

const fn primitive_keyword(primitive: PrimitiveType) -> &'static str {
    match primitive {
        PrimitiveType::I8 => "char",
        PrimitiveType::U8 => "uchar",
        PrimitiveType::I16 => "short",
        PrimitiveType::U16 => "ushort",
        PrimitiveType::I32 => "int",
        PrimitiveType::U32 => "uint",
        PrimitiveType::F32 => "float",
        PrimitiveType::F64 => "double",
        PrimitiveType::I64 | PrimitiveType::U64 => unreachable!(),
    }
}

fn primitive_from_keyword(keyword: &str) -> Option<PrimitiveType> {
    match keyword {
        "char" | "int8" => Some(PrimitiveType::I8),
        "uchar" | "uint8" => Some(PrimitiveType::U8),
        "short" | "int16" => Some(PrimitiveType::I16),
        "ushort" | "uint16" => Some(PrimitiveType::U16),
        "int" | "int32" => Some(PrimitiveType::I32),
        "uint" | "uint32" => Some(PrimitiveType::U32),
        "float" | "float32" => Some(PrimitiveType::F32),
        "double" | "float64" => Some(PrimitiveType::F64),
        _ => None,
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !byte.is_ascii_whitespace())
}

fn semantic_for_name(name: &str) -> Option<PointFieldSemantic> {
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

/// PLY parsing, fidelity, resource, or I/O failure.
#[derive(Debug)]
pub enum Error {
    MissingMagic,
    MissingEndHeader,
    HeaderTooLarge,
    NonAsciiHeader,
    InvalidFormat(String),
    InvalidHeader(String),
    InvalidVertexCount(String),
    InvalidProperty(String),
    ListProperty(String),
    UnsupportedElement(String),
    UnsupportedPropertyType(String),
    EmptySchema,
    InvalidSchema(SchemaError),
    SizeOverflow,
    BudgetExceeded {
        required: usize,
        available: usize,
    },
    AllocationFailed(usize),
    TruncatedPayload {
        point: usize,
        field: String,
    },
    InvalidAsciiValue {
        point: usize,
        field: String,
        value: String,
    },
    AsciiTokenTooLarge,
    TrailingData,
    OrganizedCloud {
        width: usize,
        height: usize,
    },
    UnrepresentablePropertyName {
        name: String,
    },
    ListLikeField {
        name: String,
        count: usize,
    },
    UnsupportedPrimitive {
        name: String,
        primitive: PrimitiveType,
    },
    UnrepresentableSemantic {
        name: String,
        semantic: Option<PointFieldSemantic>,
    },
    NonFiniteAscii {
        name: String,
        point: usize,
    },
    UnrepresentableMetadata,
    Access(AccessError),
    InvalidDimensions(String),
    InvalidBatch(String),
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingMagic => f.write_str("PLY input must begin with the exact line 'ply'"),
            Self::MissingEndHeader => f.write_str("PLY header is missing end_header"),
            Self::HeaderTooLarge => {
                write!(f, "PLY header exceeds the {MAX_HEADER_BYTES}-byte bound")
            }
            Self::NonAsciiHeader => f.write_str("PLY header must be ASCII"),
            Self::InvalidFormat(line) => {
                write!(f, "unsupported or malformed PLY format declaration: {line}")
            }
            Self::InvalidHeader(line) => write!(f, "invalid PLY header line: {line}"),
            Self::InvalidVertexCount(value) => write!(f, "invalid PLY vertex count: {value}"),
            Self::InvalidProperty(line) => write!(f, "invalid PLY property declaration: {line}"),
            Self::ListProperty(line) => write!(f, "PLY list properties are not supported: {line}"),
            Self::UnsupportedElement(name) => write!(
                f,
                "PLY element {name:?} cannot be represented as a point field schema"
            ),
            Self::UnsupportedPropertyType(name) => {
                write!(f, "unsupported PLY scalar property type {name:?}")
            }
            Self::EmptySchema => f.write_str("PLY requires at least one scalar vertex property"),
            Self::InvalidSchema(error) => write!(f, "invalid PLY vertex schema: {error}"),
            Self::SizeOverflow => f.write_str("PLY size calculation overflowed"),
            Self::BudgetExceeded {
                required,
                available,
            } => write!(
                f,
                "PLY materialization requires {required} bytes but only {available} bytes are available"
            ),
            Self::AllocationFailed(bytes) => write!(
                f,
                "failed to allocate the planned {bytes}-byte PLY materialization"
            ),
            Self::TruncatedPayload { point, field } => {
                write!(f, "PLY payload ended at vertex {point}, property {field:?}")
            }
            Self::InvalidAsciiValue {
                point,
                field,
                value,
            } => write!(
                f,
                "invalid ASCII PLY value {value:?} at vertex {point}, property {field:?}"
            ),
            Self::AsciiTokenTooLarge => write!(
                f,
                "ASCII PLY token exceeds the {MAX_ASCII_TOKEN_BYTES}-byte bound"
            ),
            Self::TrailingData => f.write_str("PLY payload has data after the declared vertices"),
            Self::OrganizedCloud { width, height } => write!(
                f,
                "organized cloud {width}x{height} cannot be represented losslessly by a PLY vertex element"
            ),
            Self::UnrepresentablePropertyName { name } => write!(
                f,
                "point field {name:?} is not a valid ASCII PLY property token"
            ),
            Self::ListLikeField { name, count } => write!(
                f,
                "point field {name:?} has count {count}; PLY list properties are outside the faithful subset"
            ),
            Self::UnsupportedPrimitive { name, primitive } => write!(
                f,
                "point field {name:?} uses {primitive:?}, which PLY 1.0 cannot represent losslessly"
            ),
            Self::UnrepresentableSemantic { name, semantic } => write!(
                f,
                "point field {name:?} has semantic {semantic:?}, which cannot be reconstructed from its PLY property name"
            ),
            Self::NonFiniteAscii { name, point } => write!(
                f,
                "non-finite value at vertex {point}, property {name:?} is not portable ASCII PLY"
            ),
            Self::UnrepresentableMetadata => f.write_str(
                "PLY cannot represent Point Frame timestamps, frame identity, density, or container times",
            ),
            Self::Access(error) => write!(f, "validated point access failed: {error}"),
            Self::InvalidDimensions(error) => write!(f, "invalid PLY point dimensions: {error}"),
            Self::InvalidBatch(error) => write!(f, "invalid materialized PLY batch: {error}"),
            Self::Io(error) => write!(f, "PLY I/O failed: {error}"),
        }
    }
}

impl error::Error for Error {}
impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
