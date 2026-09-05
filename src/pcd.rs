//! Deterministic PCD v0.7 encoding for validated point frames.
//!
//! The writer emits ordered, packed point records derived from the core schema.
//! Source padding and byte order are representation details and are not copied.

use crate::core::point::{
    AccessError, PointBatch, PointColumn, PointDimensions, PointSchema, PointValue, PointView,
    PrimitiveType,
};
use std::{error, fmt, io};

const PREAMBLE: &str = "# .PCD v0.7 - Point Cloud Data file format\nVERSION 0.7\n";
const DEFAULT_VIEWPOINT: &str = "0 0 0 1 0 0 0";

/// PCD payload representation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Encoding {
    Binary,
    Ascii,
}

impl Encoding {
    const fn keyword(self) -> &'static str {
        match self {
            Self::Binary => "binary",
            Self::Ascii => "ascii",
        }
    }
}

/// Either validated core representation accepted by the PCD writer.
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

    fn value(self, field: usize, point: usize, element: usize) -> Result<PointValue, Error> {
        match self {
            Self::View(view) => view
                .field(view.schema().fields()[field].name())
                .expect("field belongs to the validated view schema")
                .value(point, element)
                .map_err(Error::Access),
            Self::Batch(batch) => Ok(batch_value(
                &batch.columns()[field],
                point,
                element,
                batch.schema().fields()[field].count(),
            )),
        }
    }
}

/// Writes one complete PCD file to `output`.
///
/// Schema compatibility is checked before the first byte is written. Binary
/// scalars are always little-endian, making output independent of source and
/// host byte order. ASCII floats use Rust's shortest round-tripping decimal;
/// signed zero is retained and non-finite values are spelled `nan`, `inf`, and
/// `-inf` as expected by PCL. An ASCII NaN retains its semantic value but, as
/// PCD text cannot carry it, not its payload/sign bits.
pub fn write<'a>(
    output: &mut impl io::Write,
    points: impl Into<PointDataRef<'a>>,
    encoding: Encoding,
) -> Result<(), Error> {
    let points = points.into();
    validate_schema(points.schema())?;
    write_header(output, points.schema(), points.dimensions(), encoding)?;

    match encoding {
        Encoding::Binary => write_binary(output, points),
        Encoding::Ascii => write_ascii(output, points),
    }
}

/// Check that a point representation can be written losslessly as PCD.
///
/// Callers use this during preflight, before creating an output sink. The
/// writer repeats the check so direct callers retain the same guarantee.
pub fn validate<'a>(points: impl Into<PointDataRef<'a>>) -> Result<(), Error> {
    validate_schema(points.into().schema())
}

fn validate_schema(schema: &PointSchema) -> Result<(), Error> {
    if schema.fields().is_empty() {
        return Err(Error::EmptySchema);
    }
    for field in schema.fields() {
        if !field.name().bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(Error::UnrepresentableFieldName {
                name: field.name().to_owned(),
            });
        }
        if matches!(field.primitive(), PrimitiveType::I64 | PrimitiveType::U64) {
            return Err(Error::UnsupportedPrimitive {
                name: field.name().to_owned(),
                primitive: field.primitive(),
            });
        }
    }
    Ok(())
}

fn write_header(
    output: &mut impl io::Write,
    schema: &PointSchema,
    dimensions: PointDimensions,
    encoding: Encoding,
) -> Result<(), Error> {
    output.write_all(PREAMBLE.as_bytes())?;
    write!(output, "FIELDS")?;
    for field in schema.fields() {
        write!(output, " {}", field.name())?;
    }
    write!(output, "\nSIZE")?;
    for field in schema.fields() {
        write!(output, " {}", field.primitive().size())?;
    }
    write!(output, "\nTYPE")?;
    for field in schema.fields() {
        write!(output, " {}", pcd_type(field.primitive()))?;
    }
    write!(output, "\nCOUNT")?;
    for field in schema.fields() {
        write!(output, " {}", field.count())?;
    }
    write!(
        output,
        "\nWIDTH {}\nHEIGHT {}\nVIEWPOINT {DEFAULT_VIEWPOINT}\nPOINTS {}\nDATA {}\n",
        dimensions.width(),
        dimensions.height(),
        dimensions.point_count(),
        encoding.keyword(),
    )?;
    Ok(())
}

const fn pcd_type(primitive: PrimitiveType) -> char {
    match primitive {
        PrimitiveType::I8 | PrimitiveType::I16 | PrimitiveType::I32 | PrimitiveType::I64 => 'I',
        PrimitiveType::U8 | PrimitiveType::U16 | PrimitiveType::U32 | PrimitiveType::U64 => 'U',
        PrimitiveType::F32 | PrimitiveType::F64 => 'F',
    }
}

fn write_binary(output: &mut impl io::Write, points: PointDataRef<'_>) -> Result<(), Error> {
    for point in 0..points.dimensions().point_count() {
        for (field_index, field) in points.schema().fields().iter().enumerate() {
            for element in 0..field.count() {
                write_binary_value(output, points.value(field_index, point, element)?)?;
            }
        }
    }
    Ok(())
}

fn write_binary_value(output: &mut impl io::Write, value: PointValue) -> Result<(), Error> {
    match value {
        PointValue::I8(value) => output.write_all(&[value as u8])?,
        PointValue::U8(value) => output.write_all(&[value])?,
        PointValue::I16(value) => output.write_all(&value.to_le_bytes())?,
        PointValue::U16(value) => output.write_all(&value.to_le_bytes())?,
        PointValue::I32(value) => output.write_all(&value.to_le_bytes())?,
        PointValue::U32(value) => output.write_all(&value.to_le_bytes())?,
        PointValue::F32(value) => output.write_all(&value.to_bits().to_le_bytes())?,
        PointValue::F64(value) => output.write_all(&value.to_bits().to_le_bytes())?,
        PointValue::I64(_) | PointValue::U64(_) => {
            unreachable!("64-bit integers are rejected before output")
        }
    }
    Ok(())
}

fn write_ascii(output: &mut impl io::Write, points: PointDataRef<'_>) -> Result<(), Error> {
    for point in 0..points.dimensions().point_count() {
        let mut first = true;
        for (field_index, field) in points.schema().fields().iter().enumerate() {
            for element in 0..field.count() {
                if !first {
                    output.write_all(b" ")?;
                }
                first = false;
                write_ascii_value(output, points.value(field_index, point, element)?)?;
            }
        }
        output.write_all(b"\n")?;
    }
    Ok(())
}

fn write_ascii_value(output: &mut impl io::Write, value: PointValue) -> Result<(), Error> {
    match value {
        PointValue::I8(value) => write!(output, "{value}")?,
        PointValue::U8(value) => write!(output, "{value}")?,
        PointValue::I16(value) => write!(output, "{value}")?,
        PointValue::U16(value) => write!(output, "{value}")?,
        PointValue::I32(value) => write!(output, "{value}")?,
        PointValue::U32(value) => write!(output, "{value}")?,
        PointValue::F32(value) => write_float(output, value)?,
        PointValue::F64(value) => write_float(output, value)?,
        PointValue::I64(_) | PointValue::U64(_) => {
            unreachable!("64-bit integers are rejected before output")
        }
    }
    Ok(())
}

trait PcdFloat: fmt::Display + Copy {
    fn is_nan(self) -> bool;
    fn is_infinite(self) -> bool;
    fn is_sign_negative(self) -> bool;
}

impl PcdFloat for f32 {
    fn is_nan(self) -> bool {
        self.is_nan()
    }
    fn is_infinite(self) -> bool {
        self.is_infinite()
    }
    fn is_sign_negative(self) -> bool {
        self.is_sign_negative()
    }
}

impl PcdFloat for f64 {
    fn is_nan(self) -> bool {
        self.is_nan()
    }
    fn is_infinite(self) -> bool {
        self.is_infinite()
    }
    fn is_sign_negative(self) -> bool {
        self.is_sign_negative()
    }
}

fn write_float(output: &mut impl io::Write, value: impl PcdFloat) -> Result<(), Error> {
    if value.is_nan() {
        output.write_all(b"nan")?;
    } else if value.is_infinite() {
        output.write_all(if value.is_sign_negative() {
            b"-inf"
        } else {
            b"inf"
        })?;
    } else {
        write!(output, "{value}")?;
    }
    Ok(())
}

fn batch_value(column: &PointColumn, point: usize, element: usize, count: usize) -> PointValue {
    let index = point * count + element;
    match column {
        PointColumn::I8(values) => PointValue::I8(values[index]),
        PointColumn::U8(values) => PointValue::U8(values[index]),
        PointColumn::I16(values) => PointValue::I16(values[index]),
        PointColumn::U16(values) => PointValue::U16(values[index]),
        PointColumn::I32(values) => PointValue::I32(values[index]),
        PointColumn::U32(values) => PointValue::U32(values[index]),
        PointColumn::I64(values) => PointValue::I64(values[index]),
        PointColumn::U64(values) => PointValue::U64(values[index]),
        PointColumn::F32(values) => PointValue::F32(values[index]),
        PointColumn::F64(values) => PointValue::F64(values[index]),
    }
}

/// Failure to represent or write a point frame as PCD.
#[derive(Debug)]
pub enum Error {
    EmptySchema,
    UnrepresentableFieldName {
        name: String,
    },
    UnsupportedPrimitive {
        name: String,
        primitive: PrimitiveType,
    },
    Access(AccessError),
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySchema => formatter.write_str("PCD requires at least one point field"),
            Self::UnrepresentableFieldName { name } => write!(
                formatter,
                "point field {name:?} cannot be represented in an ASCII PCD header"
            ),
            Self::UnsupportedPrimitive { name, primitive } => write!(
                formatter,
                "point field {name:?} uses {primitive:?}, which PCD v0.7 cannot represent losslessly"
            ),
            Self::Access(error) => write!(formatter, "validated point access failed: {error}"),
            Self::Io(error) => write!(formatter, "failed to write PCD: {error}"),
        }
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Access(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::point::{PointField, PointFrameMetadata, PointLayout, PointSchema, Timestamp};
    use std::sync::Arc;

    fn fixture_view() -> PointView {
        let schema = Arc::new(
            PointSchema::new(vec![
                PointField::new("x", PrimitiveType::F32, 1, None).unwrap(),
                PointField::new("y", PrimitiveType::F32, 1, None).unwrap(),
                PointField::new("z", PrimitiveType::F32, 1, None).unwrap(),
                PointField::new("intensity", PrimitiveType::U16, 1, None).unwrap(),
                PointField::new("ring", PrimitiveType::U16, 1, None).unwrap(),
            ])
            .unwrap(),
        );
        let dimensions = PointDimensions::new(2, 1).unwrap();
        let mut source = Vec::new();
        for (x, y, z, intensity, ring) in [
            (1.0_f32, -2.5_f32, 0.0_f32, 42_u16, 7_u16),
            (
                -0.0,
                f32::INFINITY,
                f32::from_bits(0x7fc0_1234),
                u16::MAX,
                8,
            ),
        ] {
            source.extend(x.to_be_bytes());
            source.extend(y.to_be_bytes());
            source.extend(z.to_be_bytes());
            source.extend(intensity.to_be_bytes());
            source.extend(ring.to_be_bytes());
        }
        let layout = PointLayout::new(
            schema,
            dimensions,
            vec![0, 4, 8, 12, 14],
            16,
            32,
            0,
            crate::core::point::Endianness::Big,
        )
        .unwrap();
        let metadata = Arc::new(PointFrameMetadata::new(
            Timestamp::new(1_700_000_000, 123_456_789).unwrap(),
            "map",
            false,
        ));
        PointView::new(source.into(), metadata, layout).unwrap()
    }

    #[test]
    fn binary_and_ascii_match_reviewed_goldens() {
        let view = fixture_view();
        for (encoding, expected) in [
            (
                Encoding::Binary,
                include_bytes!("../tests/fixtures/valid/pointcloud2-binary.pcd").as_slice(),
            ),
            (
                Encoding::Ascii,
                include_bytes!("../tests/fixtures/valid/pointcloud2-ascii.pcd").as_slice(),
            ),
        ] {
            let mut actual = Vec::new();
            write(&mut actual, &view, encoding).unwrap();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn preserves_organized_dimensions_arbitrary_fields_and_counts() {
        let schema = Arc::new(
            PointSchema::new(vec![
                PointField::new("descriptor", PrimitiveType::I16, 2, None).unwrap(),
                PointField::new("quality", PrimitiveType::F64, 1, None).unwrap(),
            ])
            .unwrap(),
        );
        let batch = PointBatch::new(
            schema,
            Arc::new(PointFrameMetadata::new(
                Timestamp::new(0, 0).unwrap(),
                "camera",
                true,
            )),
            PointDimensions::new(2, 2).unwrap(),
            vec![
                PointColumn::I16(vec![1, 2, 3, 4, 5, 6, 7, 8]),
                PointColumn::F64(vec![1.25, -0.0, f64::NEG_INFINITY, f64::NAN]),
            ],
        )
        .unwrap();
        let mut output = Vec::new();
        write(&mut output, &batch, Encoding::Ascii).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains(
            "FIELDS descriptor quality\nSIZE 2 8\nTYPE I F\nCOUNT 2 1\nWIDTH 2\nHEIGHT 2\n"
        ));
        assert!(text.ends_with("1 2 1.25\n3 4 -0\n5 6 -inf\n7 8 nan\n"));
    }

    #[test]
    fn rejects_lossy_schemas_before_writing() {
        for primitive in [PrimitiveType::I64, PrimitiveType::U64] {
            let schema = Arc::new(
                PointSchema::new(vec![PointField::new("clock", primitive, 1, None).unwrap()])
                    .unwrap(),
            );
            let column = match primitive {
                PrimitiveType::I64 => PointColumn::I64(vec![1]),
                PrimitiveType::U64 => PointColumn::U64(vec![1]),
                _ => unreachable!(),
            };
            let batch = PointBatch::new(
                schema,
                Arc::new(PointFrameMetadata::new(
                    Timestamp::new(0, 0).unwrap(),
                    "map",
                    true,
                )),
                PointDimensions::new(1, 1).unwrap(),
                vec![column],
            )
            .unwrap();
            let mut output = Vec::new();
            assert!(matches!(
                write(&mut output, &batch, Encoding::Binary),
                Err(Error::UnsupportedPrimitive { .. })
            ));
            assert!(output.is_empty());
        }
    }

    #[test]
    fn rejects_names_that_cannot_be_header_tokens_before_writing() {
        let schema = Arc::new(
            PointSchema::new(vec![
                PointField::new("not a token", PrimitiveType::U8, 1, None).unwrap(),
            ])
            .unwrap(),
        );
        let batch = PointBatch::new(
            schema,
            Arc::new(PointFrameMetadata::new(
                Timestamp::new(0, 0).unwrap(),
                "map",
                true,
            )),
            PointDimensions::new(1, 1).unwrap(),
            vec![PointColumn::U8(vec![0])],
        )
        .unwrap();
        let mut output = Vec::new();
        assert!(matches!(
            write(&mut output, &batch, Encoding::Ascii),
            Err(Error::UnrepresentableFieldName { .. })
        ));
        assert!(output.is_empty());
    }

    #[test]
    fn writes_every_lossless_pcd_primitive_deterministically() {
        let schema = Arc::new(
            PointSchema::new(vec![
                PointField::new("i8", PrimitiveType::I8, 1, None).unwrap(),
                PointField::new("u8", PrimitiveType::U8, 1, None).unwrap(),
                PointField::new("i16", PrimitiveType::I16, 1, None).unwrap(),
                PointField::new("u16", PrimitiveType::U16, 1, None).unwrap(),
                PointField::new("i32", PrimitiveType::I32, 1, None).unwrap(),
                PointField::new("u32", PrimitiveType::U32, 1, None).unwrap(),
                PointField::new("f32", PrimitiveType::F32, 1, None).unwrap(),
                PointField::new("f64", PrimitiveType::F64, 1, None).unwrap(),
            ])
            .unwrap(),
        );
        let batch = PointBatch::new(
            schema,
            Arc::new(PointFrameMetadata::new(
                Timestamp::new(0, 0).unwrap(),
                "map",
                true,
            )),
            PointDimensions::new(1, 1).unwrap(),
            vec![
                PointColumn::I8(vec![-1]),
                PointColumn::U8(vec![2]),
                PointColumn::I16(vec![-3]),
                PointColumn::U16(vec![4]),
                PointColumn::I32(vec![-5]),
                PointColumn::U32(vec![6]),
                PointColumn::F32(vec![-0.0]),
                PointColumn::F64(vec![f64::INFINITY]),
            ],
        )
        .unwrap();

        for encoding in [Encoding::Binary, Encoding::Ascii] {
            let mut first = Vec::new();
            let mut second = Vec::new();
            write(&mut first, &batch, encoding).unwrap();
            write(&mut second, &batch, encoding).unwrap();
            assert_eq!(first, second);
        }
    }
}
