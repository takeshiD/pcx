//! Ownership models for decoded point frames.
//!
//! [`PointView`] keeps validated interleaved point data in reference-counted
//! source storage. [`PointBatch`] owns typed, columnar values. The only
//! conversion between them is explicit, fallible, and bounded by a caller
//! supplied [`MaterializationBudget`].

use std::collections::HashSet;
use std::error::Error;
use std::fmt;
use std::mem::size_of;
use std::sync::Arc;

/// A primitive representation used by a point field.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PrimitiveType {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
}

impl PrimitiveType {
    /// Returns the encoded width of one scalar value.
    pub const fn size(self) -> usize {
        match self {
            Self::I8 | Self::U8 => 1,
            Self::I16 | Self::U16 => 2,
            Self::I32 | Self::U32 | Self::F32 => 4,
            Self::I64 | Self::U64 | Self::F64 => 8,
        }
    }
}

/// A known meaning attached to a field without replacing its source name.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PointFieldSemantic {
    X,
    Y,
    Z,
    Intensity,
    Ring,
    Timestamp,
}

/// One ordered field in a point schema.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PointField {
    name: String,
    primitive: PrimitiveType,
    count: usize,
    semantic: Option<PointFieldSemantic>,
}

impl PointField {
    /// Creates a field. `count` is the number of scalar elements per point.
    pub fn new(
        name: impl Into<String>,
        primitive: PrimitiveType,
        count: usize,
        semantic: Option<PointFieldSemantic>,
    ) -> Result<Self, SchemaError> {
        let name = name.into();
        if name.is_empty() {
            return Err(SchemaError::EmptyFieldName);
        }
        if count == 0 {
            return Err(SchemaError::ZeroFieldCount { name });
        }
        primitive
            .size()
            .checked_mul(count)
            .ok_or_else(|| SchemaError::FieldWidthOverflow { name: name.clone() })?;

        Ok(Self {
            name,
            primitive,
            count,
            semantic,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn primitive(&self) -> PrimitiveType {
        self.primitive
    }

    pub const fn count(&self) -> usize {
        self.count
    }

    pub const fn semantic(&self) -> Option<PointFieldSemantic> {
        self.semantic
    }

    fn encoded_width(&self) -> usize {
        // Checked by `PointField::new`; fields cannot be constructed otherwise.
        self.primitive.size() * self.count
    }
}

/// An immutable, ordered point schema.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PointSchema {
    fields: Box<[PointField]>,
}

impl PointSchema {
    /// Validates field-name uniqueness while preserving source field order.
    pub fn new(fields: Vec<PointField>) -> Result<Self, SchemaError> {
        let mut names = HashSet::with_capacity(fields.len());
        for field in &fields {
            if !names.insert(field.name.clone()) {
                return Err(SchemaError::DuplicateFieldName {
                    name: field.name.clone(),
                });
            }
        }
        Ok(Self {
            fields: fields.into_boxed_slice(),
        })
    }

    pub fn fields(&self) -> &[PointField] {
        &self.fields
    }

    pub fn field(&self, name: &str) -> Option<&PointField> {
        self.fields.iter().find(|field| field.name == name)
    }

    fn field_index(&self, name: &str) -> Option<usize> {
        self.fields.iter().position(|field| field.name == name)
    }
}

/// Semantic metadata which must survive a representation change.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PointFrameMetadata {
    sensor_timestamp: Timestamp,
    frame_id: String,
    is_dense: bool,
    log_time_ns: Option<u64>,
    publish_time_ns: Option<u64>,
}

impl PointFrameMetadata {
    pub fn new(sensor_timestamp: Timestamp, frame_id: impl Into<String>, is_dense: bool) -> Self {
        Self {
            sensor_timestamp,
            frame_id: frame_id.into(),
            is_dense,
            log_time_ns: None,
            publish_time_ns: None,
        }
    }

    /// Adds the timestamps of the containing record without replacing the
    /// sensor timestamp carried by the point frame itself.
    pub const fn with_container_times(mut self, log_time_ns: u64, publish_time_ns: u64) -> Self {
        self.log_time_ns = Some(log_time_ns);
        self.publish_time_ns = Some(publish_time_ns);
        self
    }

    pub const fn sensor_timestamp(&self) -> Timestamp {
        self.sensor_timestamp
    }

    pub fn frame_id(&self) -> &str {
        &self.frame_id
    }

    pub const fn is_dense(&self) -> bool {
        self.is_dense
    }

    pub const fn log_time_ns(&self) -> Option<u64> {
        self.log_time_ns
    }

    pub const fn publish_time_ns(&self) -> Option<u64> {
        self.publish_time_ns
    }
}

/// A ROS-compatible timestamp, kept independent of any ROS library.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Timestamp {
    seconds: i32,
    nanoseconds: u32,
}

impl Timestamp {
    pub fn new(seconds: i32, nanoseconds: u32) -> Result<Self, TimestampError> {
        if nanoseconds >= 1_000_000_000 {
            return Err(TimestampError { nanoseconds });
        }
        Ok(Self {
            seconds,
            nanoseconds,
        })
    }

    pub const fn seconds(self) -> i32 {
        self.seconds
    }

    pub const fn nanoseconds(self) -> u32 {
        self.nanoseconds
    }
}

/// Logical dimensions of a point frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PointDimensions {
    width: usize,
    height: usize,
    point_count: usize,
}

impl PointDimensions {
    pub fn new(width: usize, height: usize) -> Result<Self, LayoutError> {
        if height == 0 {
            return Err(LayoutError::ZeroHeight);
        }
        let point_count = width
            .checked_mul(height)
            .ok_or(LayoutError::PointCountOverflow { width, height })?;
        Ok(Self {
            width,
            height,
            point_count,
        })
    }

    pub const fn width(self) -> usize {
        self.width
    }

    pub const fn height(self) -> usize {
        self.height
    }

    pub const fn point_count(self) -> usize {
        self.point_count
    }

    pub const fn is_organized(self) -> bool {
        self.height > 1
    }
}

/// Byte order of scalar values in a view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Endianness {
    Little,
    Big,
}

/// A checked description of an interleaved point payload.
#[derive(Clone, Debug)]
pub struct PointLayout {
    schema: Arc<PointSchema>,
    dimensions: PointDimensions,
    field_offsets: Box<[usize]>,
    point_step: usize,
    row_step: usize,
    data_offset: usize,
    required_source_len: usize,
    endianness: Endianness,
}

impl PointLayout {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        schema: Arc<PointSchema>,
        dimensions: PointDimensions,
        field_offsets: Vec<usize>,
        point_step: usize,
        row_step: usize,
        data_offset: usize,
        endianness: Endianness,
    ) -> Result<Self, LayoutError> {
        if schema.fields.len() != field_offsets.len() {
            return Err(LayoutError::FieldOffsetCount {
                fields: schema.fields.len(),
                offsets: field_offsets.len(),
            });
        }
        if point_step == 0 && dimensions.point_count != 0 {
            return Err(LayoutError::ZeroPointStep);
        }

        let row_data_len = dimensions
            .width
            .checked_mul(point_step)
            .ok_or(LayoutError::RowDataLengthOverflow)?;
        if row_step < row_data_len {
            return Err(LayoutError::RowStepTooSmall {
                row_step,
                minimum: row_data_len,
            });
        }

        let mut occupied: Vec<(usize, usize, String)> = Vec::with_capacity(schema.fields.len());
        for (field, &offset) in schema.fields.iter().zip(&field_offsets) {
            let end = offset.checked_add(field.encoded_width()).ok_or_else(|| {
                LayoutError::FieldEndOverflow {
                    name: field.name.clone(),
                }
            })?;
            if end > point_step {
                return Err(LayoutError::FieldOutsidePoint {
                    name: field.name.clone(),
                    end,
                    point_step,
                });
            }
            if let Some((other_start, other_end, other_name)) = occupied
                .iter()
                .find(|(other_start, other_end, _)| offset < *other_end && *other_start < end)
            {
                return Err(LayoutError::OverlappingFields {
                    first: other_name.clone(),
                    first_range: (*other_start, *other_end),
                    second: field.name.clone(),
                    second_range: (offset, end),
                });
            }
            occupied.push((offset, end, field.name.clone()));
        }

        let payload_len = dimensions
            .height
            .checked_mul(row_step)
            .ok_or(LayoutError::PayloadExtentOverflow)?;
        let required_source_len = data_offset
            .checked_add(payload_len)
            .ok_or(LayoutError::PayloadExtentOverflow)?;

        Ok(Self {
            schema,
            dimensions,
            field_offsets: field_offsets.into_boxed_slice(),
            point_step,
            row_step,
            data_offset,
            required_source_len,
            endianness,
        })
    }

    pub fn schema(&self) -> &PointSchema {
        &self.schema
    }

    pub const fn dimensions(&self) -> PointDimensions {
        self.dimensions
    }

    pub fn field_offsets(&self) -> &[usize] {
        &self.field_offsets
    }

    pub const fn point_step(&self) -> usize {
        self.point_step
    }

    pub const fn row_step(&self) -> usize {
        self.row_step
    }

    pub const fn data_offset(&self) -> usize {
        self.data_offset
    }

    pub const fn required_source_len(&self) -> usize {
        self.required_source_len
    }

    pub const fn endianness(&self) -> Endianness {
        self.endianness
    }
}

/// A validated, low-copy view of interleaved point data.
#[derive(Clone, Debug)]
pub struct PointView {
    source: Arc<[u8]>,
    metadata: Arc<PointFrameMetadata>,
    layout: PointLayout,
}

impl PointView {
    /// Retains `source` after checking that the complete layout is accessible.
    pub fn new(
        source: Arc<[u8]>,
        metadata: Arc<PointFrameMetadata>,
        layout: PointLayout,
    ) -> Result<Self, ViewError> {
        if source.len() < layout.required_source_len {
            return Err(ViewError::SourceTooShort {
                required: layout.required_source_len,
                actual: source.len(),
            });
        }
        Ok(Self {
            source,
            metadata,
            layout,
        })
    }

    pub fn schema(&self) -> &PointSchema {
        self.layout.schema()
    }

    pub fn metadata(&self) -> &PointFrameMetadata {
        &self.metadata
    }

    pub const fn layout(&self) -> &PointLayout {
        &self.layout
    }

    pub fn source_len(&self) -> usize {
        self.source.len()
    }

    /// Attach the containing record's times without replacing the sensor time.
    pub fn with_container_times(mut self, log_time_ns: u64, publish_time_ns: u64) -> Self {
        self.metadata = Arc::new(
            self.metadata
                .as_ref()
                .clone()
                .with_container_times(log_time_ns, publish_time_ns),
        );
        self
    }

    /// Returns field access tied to this view's lifetime.
    pub fn field(&self, name: &str) -> Option<PointFieldView<'_>> {
        let index = self.layout.schema.field_index(name)?;
        Some(PointFieldView {
            source: &self.source,
            layout: &self.layout,
            field: &self.layout.schema.fields[index],
            field_offset: self.layout.field_offsets[index],
        })
    }

    /// Materializes every field into a typed owned column.
    ///
    /// The budget covers all additional heap requested for column values and
    /// the column table. The immutable schema and metadata are shared; source
    /// retention is already allocated and must be included separately by the
    /// caller when planning the whole pipeline's peak managed memory.
    pub fn materialize(
        &self,
        budget: MaterializationBudget,
    ) -> Result<PointBatch, MaterializeError> {
        let required = self.materialized_size()?;
        if required > budget.bytes {
            return Err(MaterializeError::BudgetExceeded {
                required,
                available: budget.bytes,
            });
        }

        let mut columns = Vec::new();
        columns
            .try_reserve_exact(self.schema().fields.len())
            .map_err(|_| MaterializeError::AllocationFailed {
                requested: required,
            })?;

        for field in &self.schema().fields {
            let view = self
                .field(field.name())
                .expect("field comes from this view's validated schema");
            columns.push(materialize_field(view)?);
        }

        // Construction cannot fail because the values were generated from this
        // exact schema and point count. Keep the check at the shared seam used
        // by future operators rather than duplicating the invariants here.
        PointBatch::new(
            Arc::clone(&self.layout.schema),
            Arc::clone(&self.metadata),
            self.layout.dimensions,
            columns,
        )
        .map_err(MaterializeError::InvalidBatch)
    }

    /// Returns the exact additional managed allocation requested by
    /// [`Self::materialize`].
    pub fn materialized_size(&self) -> Result<usize, MaterializeError> {
        let point_count = self.layout.dimensions.point_count;
        let values = self
            .schema()
            .fields
            .iter()
            .try_fold(0usize, |total, field| {
                let scalar_count = point_count
                    .checked_mul(field.count)
                    .ok_or(MaterializeError::SizeOverflow)?;
                let bytes = scalar_count
                    .checked_mul(field.primitive.size())
                    .ok_or(MaterializeError::SizeOverflow)?;
                total
                    .checked_add(bytes)
                    .ok_or(MaterializeError::SizeOverflow)
            })?;
        let table = self
            .schema()
            .fields
            .len()
            .checked_mul(size_of::<PointColumn>())
            .ok_or(MaterializeError::SizeOverflow)?;
        values
            .checked_add(table)
            .ok_or(MaterializeError::SizeOverflow)
    }
}

/// Borrowed access to one interleaved field.
///
/// The returned bytes cannot outlive the [`PointView`] that owns the source:
///
/// ```compile_fail
/// # use pcx_cli::core::point::{PointFieldView, PointView};
/// fn leak(view: &PointView) -> &'static [u8] {
///     let field: PointFieldView<'_> = view.field("x").unwrap();
///     field.raw(0, 0).unwrap()
/// }
/// ```
#[derive(Clone, Copy, Debug)]
pub struct PointFieldView<'a> {
    source: &'a [u8],
    layout: &'a PointLayout,
    field: &'a PointField,
    field_offset: usize,
}

impl<'a> PointFieldView<'a> {
    pub const fn field(self) -> &'a PointField {
        self.field
    }

    pub const fn point_count(self) -> usize {
        self.layout.dimensions.point_count
    }

    pub fn raw(self, point: usize, element: usize) -> Result<&'a [u8], AccessError> {
        if point >= self.point_count() {
            return Err(AccessError::PointOutOfBounds {
                index: point,
                point_count: self.point_count(),
            });
        }
        if element >= self.field.count {
            return Err(AccessError::ElementOutOfBounds {
                index: element,
                count: self.field.count,
            });
        }

        let row = point / self.layout.dimensions.width;
        let column = point % self.layout.dimensions.width;
        let scalar_size = self.field.primitive.size();
        // Every operation was proven in the layout constructor; point and
        // element bounds above keep these calculations within those maxima.
        let start = self.layout.data_offset
            + row * self.layout.row_step
            + column * self.layout.point_step
            + self.field_offset
            + element * scalar_size;
        Ok(&self.source[start..start + scalar_size])
    }

    pub fn value(self, point: usize, element: usize) -> Result<PointValue, AccessError> {
        let bytes = self.raw(point, element)?;
        Ok(decode_value(
            self.field.primitive,
            self.layout.endianness,
            bytes,
        ))
    }
}

/// An owned scalar returned by view access.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointValue {
    I8(i8),
    U8(u8),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    F32(f32),
    F64(f64),
}

/// An owned typed column. Multi-element fields are flattened point-major.
#[derive(Clone, Debug, PartialEq)]
pub enum PointColumn {
    I8(Vec<i8>),
    U8(Vec<u8>),
    I16(Vec<i16>),
    U16(Vec<u16>),
    I32(Vec<i32>),
    U32(Vec<u32>),
    I64(Vec<i64>),
    U64(Vec<u64>),
    F32(Vec<f32>),
    F64(Vec<f64>),
}

impl PointColumn {
    pub const fn primitive(&self) -> PrimitiveType {
        match self {
            Self::I8(_) => PrimitiveType::I8,
            Self::U8(_) => PrimitiveType::U8,
            Self::I16(_) => PrimitiveType::I16,
            Self::U16(_) => PrimitiveType::U16,
            Self::I32(_) => PrimitiveType::I32,
            Self::U32(_) => PrimitiveType::U32,
            Self::I64(_) => PrimitiveType::I64,
            Self::U64(_) => PrimitiveType::U64,
            Self::F32(_) => PrimitiveType::F32,
            Self::F64(_) => PrimitiveType::F64,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::I8(values) => values.len(),
            Self::U8(values) => values.len(),
            Self::I16(values) => values.len(),
            Self::U16(values) => values.len(),
            Self::I32(values) => values.len(),
            Self::U32(values) => values.len(),
            Self::I64(values) => values.len(),
            Self::U64(values) => values.len(),
            Self::F32(values) => values.len(),
            Self::F64(values) => values.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A point frame with schema-driven, owned typed columns.
#[derive(Clone, Debug)]
pub struct PointBatch {
    schema: Arc<PointSchema>,
    metadata: Arc<PointFrameMetadata>,
    dimensions: PointDimensions,
    columns: Box<[PointColumn]>,
}

impl PointBatch {
    /// Checks the one-column-per-field, type, and scalar-count invariants.
    pub fn new(
        schema: Arc<PointSchema>,
        metadata: Arc<PointFrameMetadata>,
        dimensions: PointDimensions,
        columns: Vec<PointColumn>,
    ) -> Result<Self, BatchError> {
        if schema.fields.len() != columns.len() {
            return Err(BatchError::ColumnCount {
                fields: schema.fields.len(),
                columns: columns.len(),
            });
        }
        for (field, column) in schema.fields.iter().zip(&columns) {
            if field.primitive != column.primitive() {
                return Err(BatchError::ColumnType {
                    name: field.name.clone(),
                    expected: field.primitive,
                    actual: column.primitive(),
                });
            }
            let expected = dimensions
                .point_count
                .checked_mul(field.count)
                .ok_or_else(|| BatchError::ColumnLengthOverflow {
                    name: field.name.clone(),
                })?;
            if column.len() != expected {
                return Err(BatchError::ColumnLength {
                    name: field.name.clone(),
                    expected,
                    actual: column.len(),
                });
            }
        }
        Ok(Self {
            schema,
            metadata,
            dimensions,
            columns: columns.into_boxed_slice(),
        })
    }

    pub fn schema(&self) -> &PointSchema {
        &self.schema
    }

    pub fn metadata(&self) -> &PointFrameMetadata {
        &self.metadata
    }

    pub const fn dimensions(&self) -> PointDimensions {
        self.dimensions
    }

    pub fn columns(&self) -> &[PointColumn] {
        &self.columns
    }

    pub fn column(&self, name: &str) -> Option<&PointColumn> {
        self.schema
            .field_index(name)
            .map(|index| &self.columns[index])
    }
}

/// Maximum additional managed bytes allowed for one view-to-batch conversion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MaterializationBudget {
    bytes: usize,
}

impl MaterializationBudget {
    pub const fn new(bytes: usize) -> Self {
        Self { bytes }
    }

    pub const fn bytes(self) -> usize {
        self.bytes
    }
}

fn materialize_field(view: PointFieldView<'_>) -> Result<PointColumn, MaterializeError> {
    let len = view
        .point_count()
        .checked_mul(view.field.count)
        .ok_or(MaterializeError::SizeOverflow)?;

    macro_rules! collect_column {
        ($variant:ident, $value_variant:ident, $type:ty) => {{
            let mut values: Vec<$type> = Vec::new();
            values
                .try_reserve_exact(len)
                .map_err(|_| MaterializeError::AllocationFailed {
                    requested: len.saturating_mul(size_of::<$type>()),
                })?;
            for point in 0..view.point_count() {
                for element in 0..view.field.count {
                    let PointValue::$value_variant(value) = view.value(point, element)? else {
                        unreachable!("field value follows its validated primitive type")
                    };
                    values.push(value);
                }
            }
            PointColumn::$variant(values)
        }};
    }

    Ok(match view.field.primitive {
        PrimitiveType::I8 => collect_column!(I8, I8, i8),
        PrimitiveType::U8 => collect_column!(U8, U8, u8),
        PrimitiveType::I16 => collect_column!(I16, I16, i16),
        PrimitiveType::U16 => collect_column!(U16, U16, u16),
        PrimitiveType::I32 => collect_column!(I32, I32, i32),
        PrimitiveType::U32 => collect_column!(U32, U32, u32),
        PrimitiveType::I64 => collect_column!(I64, I64, i64),
        PrimitiveType::U64 => collect_column!(U64, U64, u64),
        PrimitiveType::F32 => collect_column!(F32, F32, f32),
        PrimitiveType::F64 => collect_column!(F64, F64, f64),
    })
}

fn decode_value(primitive: PrimitiveType, endianness: Endianness, bytes: &[u8]) -> PointValue {
    macro_rules! integer {
        ($type:ty, $variant:ident) => {{
            let array: [u8; size_of::<$type>()] = bytes.try_into().expect("validated scalar width");
            let value = match endianness {
                Endianness::Little => <$type>::from_le_bytes(array),
                Endianness::Big => <$type>::from_be_bytes(array),
            };
            PointValue::$variant(value)
        }};
    }

    match primitive {
        PrimitiveType::I8 => PointValue::I8(bytes[0] as i8),
        PrimitiveType::U8 => PointValue::U8(bytes[0]),
        PrimitiveType::I16 => integer!(i16, I16),
        PrimitiveType::U16 => integer!(u16, U16),
        PrimitiveType::I32 => integer!(i32, I32),
        PrimitiveType::U32 => integer!(u32, U32),
        PrimitiveType::I64 => integer!(i64, I64),
        PrimitiveType::U64 => integer!(u64, U64),
        PrimitiveType::F32 => {
            let PointValue::U32(bits) = integer!(u32, U32) else {
                unreachable!()
            };
            PointValue::F32(f32::from_bits(bits))
        }
        PrimitiveType::F64 => {
            let PointValue::U64(bits) = integer!(u64, U64) else {
                unreachable!()
            };
            PointValue::F64(f64::from_bits(bits))
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SchemaError {
    EmptyFieldName,
    ZeroFieldCount { name: String },
    FieldWidthOverflow { name: String },
    DuplicateFieldName { name: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimestampError {
    nanoseconds: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LayoutError {
    ZeroHeight,
    PointCountOverflow {
        width: usize,
        height: usize,
    },
    FieldOffsetCount {
        fields: usize,
        offsets: usize,
    },
    ZeroPointStep,
    RowDataLengthOverflow,
    RowStepTooSmall {
        row_step: usize,
        minimum: usize,
    },
    FieldEndOverflow {
        name: String,
    },
    FieldOutsidePoint {
        name: String,
        end: usize,
        point_step: usize,
    },
    OverlappingFields {
        first: String,
        first_range: (usize, usize),
        second: String,
        second_range: (usize, usize),
    },
    PayloadExtentOverflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViewError {
    SourceTooShort { required: usize, actual: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessError {
    PointOutOfBounds { index: usize, point_count: usize },
    ElementOutOfBounds { index: usize, count: usize },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BatchError {
    ColumnCount {
        fields: usize,
        columns: usize,
    },
    ColumnType {
        name: String,
        expected: PrimitiveType,
        actual: PrimitiveType,
    },
    ColumnLengthOverflow {
        name: String,
    },
    ColumnLength {
        name: String,
        expected: usize,
        actual: usize,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MaterializeError {
    SizeOverflow,
    BudgetExceeded { required: usize, available: usize },
    AllocationFailed { requested: usize },
    Access(AccessError),
    InvalidBatch(BatchError),
}

macro_rules! debug_display {
    ($($type:ty),+ $(,)?) => {
        $(
            impl fmt::Display for $type {
                fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    write!(formatter, "{self:?}")
                }
            }

            impl Error for $type {}
        )+
    };
}

debug_display!(
    SchemaError,
    TimestampError,
    LayoutError,
    ViewError,
    AccessError,
    BatchError,
    MaterializeError
);

impl From<AccessError> for MaterializeError {
    fn from(error: AccessError) -> Self {
        Self::Access(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timestamp() -> Timestamp {
        Timestamp::new(-2, 345).unwrap()
    }

    fn metadata() -> Arc<PointFrameMetadata> {
        Arc::new(
            PointFrameMetadata::new(timestamp(), "lidar_front", false)
                .with_container_times(1_000, 900),
        )
    }

    fn xyz_schema() -> Arc<PointSchema> {
        Arc::new(
            PointSchema::new(vec![
                PointField::new("x", PrimitiveType::F32, 1, Some(PointFieldSemantic::X)).unwrap(),
                PointField::new(
                    "ring",
                    PrimitiveType::U16,
                    1,
                    Some(PointFieldSemantic::Ring),
                )
                .unwrap(),
                PointField::new("normal", PrimitiveType::I16, 2, None).unwrap(),
            ])
            .unwrap(),
        )
    }

    fn padded_big_endian_view() -> PointView {
        let schema = xyz_schema();
        let dimensions = PointDimensions::new(2, 2).unwrap();
        let layout = PointLayout::new(
            schema,
            dimensions,
            vec![0, 4, 6],
            10,
            24,
            3,
            Endianness::Big,
        )
        .unwrap();
        let mut source = vec![0xaa; layout.required_source_len()];
        let values = [
            (0x7fc0_1234_u32, 1_u16, [-1_i16, 2_i16]),
            (1.5_f32.to_bits(), 2, [3, 4]),
            ((-0.0_f32).to_bits(), 3, [5, 6]),
            (f32::INFINITY.to_bits(), 4, [7, 8]),
        ];
        for (point, (x, ring, normal)) in values.into_iter().enumerate() {
            let row = point / 2;
            let column = point % 2;
            let start = 3 + row * 24 + column * 10;
            source[start..start + 4].copy_from_slice(&x.to_be_bytes());
            source[start + 4..start + 6].copy_from_slice(&ring.to_be_bytes());
            source[start + 6..start + 8].copy_from_slice(&normal[0].to_be_bytes());
            source[start + 8..start + 10].copy_from_slice(&normal[1].to_be_bytes());
        }
        PointView::new(Arc::from(source), metadata(), layout).unwrap()
    }

    #[test]
    fn view_retains_source_bytes() {
        let source: Arc<[u8]> = Arc::from(vec![0_u8; 4]);
        let weak = Arc::downgrade(&source);
        let schema = Arc::new(
            PointSchema::new(vec![
                PointField::new("x", PrimitiveType::U32, 1, None).unwrap(),
            ])
            .unwrap(),
        );
        let layout = PointLayout::new(
            schema,
            PointDimensions::new(1, 1).unwrap(),
            vec![0],
            4,
            4,
            0,
            Endianness::Little,
        )
        .unwrap();
        let view = PointView::new(Arc::clone(&source), metadata(), layout).unwrap();
        drop(source);
        assert!(weak.upgrade().is_some());
        drop(view);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn layout_rejects_fields_outside_or_overlapping_a_point() {
        let schema = xyz_schema();
        let dimensions = PointDimensions::new(1, 1).unwrap();
        assert!(matches!(
            PointLayout::new(
                Arc::clone(&schema), dimensions, vec![0, 4, 7], 10, 10, 0, Endianness::Little
            ),
            Err(LayoutError::FieldOutsidePoint { name, .. }) if name == "normal"
        ));
        assert!(matches!(
            PointLayout::new(
                schema, dimensions, vec![0, 3, 6], 10, 10, 0, Endianness::Little
            ),
            Err(LayoutError::OverlappingFields { first, second, .. })
                if first == "x" && second == "ring"
        ));
    }

    #[test]
    fn view_rejects_a_truncated_source() {
        let schema = xyz_schema();
        let layout = PointLayout::new(
            schema,
            PointDimensions::new(2, 2).unwrap(),
            vec![0, 4, 6],
            10,
            24,
            3,
            Endianness::Little,
        )
        .unwrap();
        let required = layout.required_source_len();
        assert!(matches!(
            PointView::new(Arc::from(vec![0; required - 1]), metadata(), layout),
            Err(ViewError::SourceTooShort { required: needed, actual })
                if needed == required && actual == required - 1
        ));
    }

    #[test]
    fn borrowed_access_honors_rows_padding_counts_and_endianness() {
        let view = padded_big_endian_view();
        assert_eq!(
            view.field("ring").unwrap().value(2, 0),
            Ok(PointValue::U16(3))
        );
        assert_eq!(
            view.field("normal").unwrap().value(3, 1),
            Ok(PointValue::I16(8))
        );
        let PointValue::F32(nan) = view.field("x").unwrap().value(0, 0).unwrap() else {
            panic!("x must be f32")
        };
        assert_eq!(nan.to_bits(), 0x7fc0_1234);
    }

    #[test]
    fn materialization_is_explicit_and_budgeted() {
        let view = padded_big_endian_view();
        let required = view.materialized_size().unwrap();
        assert!(matches!(
            view.materialize(MaterializationBudget::new(required - 1)),
            Err(MaterializeError::BudgetExceeded { required: needed, available })
                if needed == required && available == required - 1
        ));

        let batch = view
            .materialize(MaterializationBudget::new(required))
            .unwrap();
        assert_eq!(batch.dimensions(), PointDimensions::new(2, 2).unwrap());
        assert_eq!(batch.metadata(), view.metadata());
        assert_eq!(batch.schema(), view.schema());
        assert_eq!(
            batch.column("ring"),
            Some(&PointColumn::U16(vec![1, 2, 3, 4]))
        );
        assert_eq!(
            batch.column("normal"),
            Some(&PointColumn::I16(vec![-1, 2, 3, 4, 5, 6, 7, 8]))
        );
        let Some(PointColumn::F32(x)) = batch.column("x") else {
            panic!("x column must be f32")
        };
        assert_eq!(
            x.iter().map(|value| value.to_bits()).collect::<Vec<_>>(),
            [
                0x7fc0_1234,
                1.5_f32.to_bits(),
                (-0.0_f32).to_bits(),
                f32::INFINITY.to_bits(),
            ]
        );
    }

    #[test]
    fn materialization_owns_each_supported_primitive_type() {
        let definitions = [
            ("i8", PrimitiveType::I8),
            ("u8", PrimitiveType::U8),
            ("i16", PrimitiveType::I16),
            ("u16", PrimitiveType::U16),
            ("i32", PrimitiveType::I32),
            ("u32", PrimitiveType::U32),
            ("i64", PrimitiveType::I64),
            ("u64", PrimitiveType::U64),
            ("f32", PrimitiveType::F32),
            ("f64", PrimitiveType::F64),
        ];
        let schema = Arc::new(
            PointSchema::new(
                definitions
                    .iter()
                    .map(|(name, primitive)| PointField::new(*name, *primitive, 1, None).unwrap())
                    .collect(),
            )
            .unwrap(),
        );
        let offsets = vec![0, 1, 2, 4, 6, 10, 14, 22, 30, 34];
        let point_step = 42;
        let layout = PointLayout::new(
            schema,
            PointDimensions::new(1, 1).unwrap(),
            offsets,
            point_step,
            point_step,
            0,
            Endianness::Little,
        )
        .unwrap();
        let mut source = Vec::with_capacity(point_step);
        source.push((-1_i8).to_le_bytes()[0]);
        source.push(2_u8);
        source.extend_from_slice(&(-3_i16).to_le_bytes());
        source.extend_from_slice(&4_u16.to_le_bytes());
        source.extend_from_slice(&(-5_i32).to_le_bytes());
        source.extend_from_slice(&6_u32.to_le_bytes());
        source.extend_from_slice(&(-7_i64).to_le_bytes());
        source.extend_from_slice(&8_u64.to_le_bytes());
        source.extend_from_slice(&f32::from_bits(0x7fc0_4321).to_le_bytes());
        source.extend_from_slice(&(-9.5_f64).to_le_bytes());
        let view = PointView::new(Arc::from(source), metadata(), layout).unwrap();
        let batch = view
            .materialize(MaterializationBudget::new(usize::MAX))
            .unwrap();

        assert_eq!(batch.column("i8"), Some(&PointColumn::I8(vec![-1])));
        assert_eq!(batch.column("u8"), Some(&PointColumn::U8(vec![2])));
        assert_eq!(batch.column("i16"), Some(&PointColumn::I16(vec![-3])));
        assert_eq!(batch.column("u16"), Some(&PointColumn::U16(vec![4])));
        assert_eq!(batch.column("i32"), Some(&PointColumn::I32(vec![-5])));
        assert_eq!(batch.column("u32"), Some(&PointColumn::U32(vec![6])));
        assert_eq!(batch.column("i64"), Some(&PointColumn::I64(vec![-7])));
        assert_eq!(batch.column("u64"), Some(&PointColumn::U64(vec![8])));
        let Some(PointColumn::F32(values)) = batch.column("f32") else {
            panic!("f32 column must be typed")
        };
        assert_eq!(values[0].to_bits(), 0x7fc0_4321);
        assert_eq!(batch.column("f64"), Some(&PointColumn::F64(vec![-9.5])));
    }

    #[test]
    fn batch_survives_the_source_view() {
        let view = padded_big_endian_view();
        let batch = view
            .materialize(MaterializationBudget::new(usize::MAX))
            .unwrap();
        drop(view);
        assert_eq!(batch.metadata().frame_id(), "lidar_front");
        assert_eq!(batch.metadata().sensor_timestamp(), timestamp());
        assert!(!batch.metadata().is_dense());
        assert_eq!(batch.metadata().log_time_ns(), Some(1_000));
        assert_eq!(batch.metadata().publish_time_ns(), Some(900));
        assert_eq!(
            batch.column("ring"),
            Some(&PointColumn::U16(vec![1, 2, 3, 4]))
        );
    }

    #[test]
    fn checked_batch_constructor_rejects_schema_mismatches() {
        let schema = Arc::new(
            PointSchema::new(vec![
                PointField::new("x", PrimitiveType::F32, 1, None).unwrap(),
            ])
            .unwrap(),
        );
        let dimensions = PointDimensions::new(2, 1).unwrap();
        assert!(matches!(
            PointBatch::new(
                Arc::clone(&schema), metadata(), dimensions, vec![PointColumn::U32(vec![0, 0])]
            ),
            Err(BatchError::ColumnType { name, .. }) if name == "x"
        ));
        assert!(matches!(
            PointBatch::new(schema, metadata(), dimensions, vec![PointColumn::F32(vec![0.0])]),
            Err(BatchError::ColumnLength { name, expected: 2, actual: 1 }) if name == "x"
        ));
    }

    #[test]
    fn schema_rejects_invalid_or_duplicate_fields() {
        assert_eq!(
            PointField::new("", PrimitiveType::U8, 1, None),
            Err(SchemaError::EmptyFieldName)
        );
        assert!(matches!(
            PointField::new("x", PrimitiveType::U8, 0, None),
            Err(SchemaError::ZeroFieldCount { name }) if name == "x"
        ));
        let x = PointField::new("x", PrimitiveType::F32, 1, None).unwrap();
        assert!(matches!(
            PointSchema::new(vec![x.clone(), x]),
            Err(SchemaError::DuplicateFieldName { name }) if name == "x"
        ));
    }

    #[test]
    fn timestamp_rejects_noncanonical_nanoseconds() {
        assert_eq!(
            Timestamp::new(0, 1_000_000_000),
            Err(TimestampError {
                nanoseconds: 1_000_000_000
            })
        );
    }
}
