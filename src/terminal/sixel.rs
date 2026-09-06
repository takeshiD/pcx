//! Deterministic, bounded Sixel encoding for the common CPU raster.

use std::{error::Error as StdError, fmt, io, io::Write, mem::size_of};

use crate::{
    core::{ByteBound, Cancellation, ErrorCategory},
    ops::{Raster, Rgb8},
    terminal::{Backend, Selection, SelectionReason},
};

const MAX_SIXEL_COLORS: usize = 256;
const SIXEL_ROWS_PER_BAND: usize = 6;
const DCS_START: &[u8] = b"\x1bP0;1;0q";
const STRING_TERMINATOR: &[u8] = b"\x1b\\";

/// Conservative defaults for a single inline terminal image.
pub const DEFAULT_SIXEL_LIMITS: SixelLimits = SixelLimits {
    maximum_width: 4_096,
    maximum_height: 4_096,
    maximum_palette_colors: 256,
    maximum_payload_bytes: 64 * 1024 * 1024,
};

/// Fixed managed encoder state, available to preflight before rasterization.
pub const SIXEL_ENCODER_MEMORY_BYTES: u64 = size_of::<SixelPlan<'static>>() as u64;

/// Hard limits applied before the first output byte is written.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SixelLimits {
    maximum_width: u32,
    maximum_height: u32,
    maximum_palette_colors: u16,
    maximum_payload_bytes: u64,
}

impl SixelLimits {
    pub fn new(
        maximum_width: u32,
        maximum_height: u32,
        maximum_palette_colors: u16,
        maximum_payload_bytes: u64,
    ) -> Result<Self, SixelError> {
        if maximum_width == 0 || maximum_height == 0 {
            return Err(SixelError::InvalidLimits(
                "Sixel dimension limits must be non-zero",
            ));
        }
        if maximum_palette_colors == 0 || usize::from(maximum_palette_colors) > MAX_SIXEL_COLORS {
            return Err(SixelError::InvalidLimits(
                "Sixel palette limit must be between 1 and 256",
            ));
        }
        Ok(Self {
            maximum_width,
            maximum_height,
            maximum_palette_colors,
            maximum_payload_bytes,
        })
    }

    pub const fn maximum_width(self) -> u32 {
        self.maximum_width
    }

    pub const fn maximum_height(self) -> u32 {
        self.maximum_height
    }

    pub const fn maximum_palette_colors(self) -> u16 {
        self.maximum_palette_colors
    }

    pub const fn maximum_payload_bytes(self) -> u64 {
        self.maximum_payload_bytes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Palette {
    colors: [Rgb8; MAX_SIXEL_COLORS],
    len: u16,
}

impl Palette {
    fn from_raster(raster: &Raster, limit: u16) -> Result<Self, SixelError> {
        let mut palette = Self {
            colors: [Rgb8([0, 0, 0]); MAX_SIXEL_COLORS],
            len: 0,
        };
        for pixel in raster.pixels().iter().flatten() {
            let color = pixel.color();
            if palette.colors().contains(&color) {
                continue;
            }
            if palette.len == limit {
                return Err(SixelError::PaletteLimitExceeded {
                    required_at_least: limit.saturating_add(1),
                    maximum: limit,
                });
            }
            palette.colors[usize::from(palette.len)] = color;
            palette.len += 1;
        }
        Ok(palette)
    }

    fn colors(&self) -> &[Rgb8] {
        &self.colors[..usize::from(self.len)]
    }
}

/// A raster whose dimensions, palette, exact payload, and encoder state are bounded.
#[derive(Debug)]
pub struct SixelPlan<'a> {
    raster: &'a Raster,
    palette: Palette,
    encoded_bytes: u64,
}

impl<'a> SixelPlan<'a> {
    pub fn new(raster: &'a Raster, limits: SixelLimits) -> Result<Self, SixelError> {
        let dimensions = raster.dimensions();
        let width =
            u32::try_from(dimensions.width()).map_err(|_| SixelError::DimensionLimitExceeded {
                width: u64::MAX,
                height: dimensions.height() as u64,
                maximum_width: limits.maximum_width,
                maximum_height: limits.maximum_height,
            })?;
        let height =
            u32::try_from(dimensions.height()).map_err(|_| SixelError::DimensionLimitExceeded {
                width: dimensions.width() as u64,
                height: u64::MAX,
                maximum_width: limits.maximum_width,
                maximum_height: limits.maximum_height,
            })?;
        if width > limits.maximum_width || height > limits.maximum_height {
            return Err(SixelError::DimensionLimitExceeded {
                width: u64::from(width),
                height: u64::from(height),
                maximum_width: limits.maximum_width,
                maximum_height: limits.maximum_height,
            });
        }

        let palette = Palette::from_raster(raster, limits.maximum_palette_colors)?;
        let mut counter = CountingWriter::default();
        emit_sequence(&mut counter, raster, &palette, None).map_err(|error| match error {
            EmitError::Io(error) => SixelError::Io(error),
            EmitError::Interrupted => unreachable!("planning has no cancellation source"),
        })?;
        let encoded_bytes = counter.bytes;
        if encoded_bytes > limits.maximum_payload_bytes {
            return Err(SixelError::PayloadLimitExceeded {
                required: encoded_bytes,
                maximum: limits.maximum_payload_bytes,
            });
        }

        Ok(Self {
            raster,
            palette,
            encoded_bytes,
        })
    }

    pub const fn encoded_bytes(&self) -> u64 {
        self.encoded_bytes
    }

    pub const fn palette_colors(&self) -> u16 {
        self.palette.len
    }

    /// Fixed encoder state; the encoded payload itself is streamed, not buffered.
    pub const fn encoder_memory_bytes() -> u64 {
        SIXEL_ENCODER_MEMORY_BYTES
    }

    pub const fn encoder_memory_bound() -> ByteBound {
        ByteBound::bounded(Self::encoder_memory_bytes())
    }

    /// Write only when the shared capability policy selected Sixel.
    ///
    /// `selection` is produced by [`crate::terminal::select_backend`]. Any
    /// other selected backend is refused before DCS entry so its own renderer
    /// can implement the canonical Unicode/plain fallback path.
    pub fn write_selected(
        &self,
        writer: &mut impl Write,
        cancellation: &Cancellation,
        selection: Selection,
    ) -> Result<(), SixelError> {
        if selection.backend() != Backend::Sixel {
            return Err(SixelError::BackendNotSelected {
                selected: selection.backend(),
                reason: selection.reason(),
            });
        }
        if cancellation.is_cancelled() {
            return Err(SixelError::Interrupted);
        }
        emit_sequence(writer, self.raster, &self.palette, Some(cancellation)).map_err(|error| {
            match error {
                EmitError::Io(error) => SixelError::Io(error),
                EmitError::Interrupted => SixelError::Interrupted,
            }
        })
    }
}

fn emit_sequence(
    writer: &mut impl Write,
    raster: &Raster,
    palette: &Palette,
    cancellation: Option<&Cancellation>,
) -> Result<(), EmitError> {
    // Once entry is attempted, always attempt ST. This includes partial writes,
    // body errors, and cancellation observed after any complete write.
    let start = write_checked(writer, DCS_START, cancellation);
    let body = start.and_then(|()| emit_body(writer, raster, palette, cancellation));
    let terminator = writer.write_all(STRING_TERMINATOR);
    let flush = terminator.and_then(|()| writer.flush());

    match (body, flush) {
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(EmitError::Io(error)),
        (Err(_), Err(cleanup)) => Err(EmitError::Io(io::Error::new(
            cleanup.kind(),
            format!("Sixel stream failed and terminal-state cleanup also failed: {cleanup}"),
        ))),
        (Ok(()), Ok(())) if cancellation.is_some_and(Cancellation::is_cancelled) => {
            Err(EmitError::Interrupted)
        }
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn emit_body(
    writer: &mut impl Write,
    raster: &Raster,
    palette: &Palette,
    cancellation: Option<&Cancellation>,
) -> Result<(), EmitError> {
    let dimensions = raster.dimensions();
    write_decimal_record(
        writer,
        b'"',
        &[1, 1, dimensions.width() as u64, dimensions.height() as u64],
        cancellation,
    )?;

    for (index, color) in palette.colors().iter().copied().enumerate() {
        let [red, green, blue] = color.0;
        write_decimal_record(
            writer,
            b'#',
            &[
                index as u64,
                2,
                u64::from(to_percent(red)),
                u64::from(to_percent(green)),
                u64::from(to_percent(blue)),
            ],
            cancellation,
        )?;
    }

    let band_count = dimensions.height().div_ceil(SIXEL_ROWS_PER_BAND);
    for band in 0..band_count {
        let mut wrote_plane = false;
        for (palette_index, color) in palette.colors().iter().copied().enumerate() {
            let Some(last_column) = last_column_for_color(raster, band, color) else {
                continue;
            };
            if wrote_plane {
                write_checked(writer, b"$", cancellation)?;
            }
            write_decimal_record(writer, b'#', &[palette_index as u64], cancellation)?;
            emit_color_plane(writer, raster, band, color, last_column, cancellation)?;
            wrote_plane = true;
        }
        if band + 1 < band_count {
            write_checked(writer, b"-", cancellation)?;
        }
    }
    Ok(())
}

fn last_column_for_color(raster: &Raster, band: usize, color: Rgb8) -> Option<usize> {
    let dimensions = raster.dimensions();
    (0..dimensions.width()).rev().find(|&column| {
        let first_row = band * SIXEL_ROWS_PER_BAND;
        let last_row = (first_row + SIXEL_ROWS_PER_BAND).min(dimensions.height());
        (first_row..last_row).any(|row| {
            raster
                .pixel(row, column)
                .is_some_and(|pixel| pixel.color() == color)
        })
    })
}

fn emit_color_plane(
    writer: &mut impl Write,
    raster: &Raster,
    band: usize,
    color: Rgb8,
    last_column: usize,
    cancellation: Option<&Cancellation>,
) -> Result<(), EmitError> {
    let mut run_byte = None;
    let mut run_len = 0_usize;
    for column in 0..=last_column {
        let sixel = 63 + color_mask(raster, band, column, color);
        if run_byte == Some(sixel) {
            run_len += 1;
        } else {
            if let Some(byte) = run_byte {
                emit_run(writer, byte, run_len, cancellation)?;
            }
            run_byte = Some(sixel);
            run_len = 1;
        }
    }
    emit_run(
        writer,
        run_byte.expect("a present plane has at least one column"),
        run_len,
        cancellation,
    )
}

fn color_mask(raster: &Raster, band: usize, column: usize, color: Rgb8) -> u8 {
    let first_row = band * SIXEL_ROWS_PER_BAND;
    let mut mask = 0_u8;
    for offset in 0..SIXEL_ROWS_PER_BAND {
        let row = first_row + offset;
        if row >= raster.dimensions().height() {
            break;
        }
        if raster
            .pixel(row, column)
            .is_some_and(|pixel| pixel.color() == color)
        {
            mask |= 1 << offset;
        }
    }
    mask
}

fn emit_run(
    writer: &mut impl Write,
    byte: u8,
    count: usize,
    cancellation: Option<&Cancellation>,
) -> Result<(), EmitError> {
    if count >= 4 {
        write_checked(writer, b"!", cancellation)?;
        write_number(writer, count as u64, cancellation)?;
        write_checked(writer, &[byte], cancellation)
    } else {
        for _ in 0..count {
            write_checked(writer, &[byte], cancellation)?;
        }
        Ok(())
    }
}

fn write_decimal_record(
    writer: &mut impl Write,
    introducer: u8,
    values: &[u64],
    cancellation: Option<&Cancellation>,
) -> Result<(), EmitError> {
    write_checked(writer, &[introducer], cancellation)?;
    for (index, value) in values.iter().copied().enumerate() {
        if index != 0 {
            write_checked(writer, b";", cancellation)?;
        }
        write_number(writer, value, cancellation)?;
    }
    Ok(())
}

fn write_number(
    writer: &mut impl Write,
    value: u64,
    cancellation: Option<&Cancellation>,
) -> Result<(), EmitError> {
    let mut digits = [0_u8; 20];
    let mut cursor = digits.len();
    let mut remaining = value;
    loop {
        cursor -= 1;
        digits[cursor] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    write_checked(writer, &digits[cursor..], cancellation)
}

fn write_checked(
    writer: &mut impl Write,
    bytes: &[u8],
    cancellation: Option<&Cancellation>,
) -> Result<(), EmitError> {
    if cancellation.is_some_and(Cancellation::is_cancelled) {
        return Err(EmitError::Interrupted);
    }
    writer.write_all(bytes).map_err(EmitError::Io)?;
    if cancellation.is_some_and(Cancellation::is_cancelled) {
        Err(EmitError::Interrupted)
    } else {
        Ok(())
    }
}

const fn to_percent(channel: u8) -> u16 {
    (channel as u16 * 100 + 127) / 255
}

#[derive(Debug)]
enum EmitError {
    Io(io::Error),
    Interrupted,
}

#[derive(Default)]
struct CountingWriter {
    bytes: u64,
}

impl Write for CountingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| io::Error::other("Sixel payload length overflowed"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Refusal or write failure from bounded Sixel output.
#[derive(Debug)]
pub enum SixelError {
    InvalidLimits(&'static str),
    DimensionLimitExceeded {
        width: u64,
        height: u64,
        maximum_width: u32,
        maximum_height: u32,
    },
    PaletteLimitExceeded {
        required_at_least: u16,
        maximum: u16,
    },
    PayloadLimitExceeded {
        required: u64,
        maximum: u64,
    },
    BackendNotSelected {
        selected: Backend,
        reason: SelectionReason,
    },
    Interrupted,
    Io(io::Error),
}

impl SixelError {
    pub const fn category(&self) -> ErrorCategory {
        match self {
            Self::InvalidLimits(_) => ErrorCategory::Usage,
            Self::BackendNotSelected { .. } => ErrorCategory::Unsupported,
            Self::DimensionLimitExceeded { .. }
            | Self::PaletteLimitExceeded { .. }
            | Self::PayloadLimitExceeded { .. } => ErrorCategory::Resource,
            Self::Interrupted => ErrorCategory::Interrupted,
            Self::Io(_) => ErrorCategory::Io,
        }
    }
}

impl fmt::Display for SixelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimits(message) => formatter.write_str(message),
            Self::DimensionLimitExceeded {
                width,
                height,
                maximum_width,
                maximum_height,
            } => write!(
                formatter,
                "Sixel raster {width}x{height} exceeds the {maximum_width}x{maximum_height} limit"
            ),
            Self::PaletteLimitExceeded {
                required_at_least,
                maximum,
            } => write!(
                formatter,
                "Sixel raster requires at least {required_at_least} colors, exceeding the {maximum}-color palette limit"
            ),
            Self::PayloadLimitExceeded { required, maximum } => write!(
                formatter,
                "Sixel payload requires {required} bytes, exceeding the {maximum}-byte limit"
            ),
            Self::BackendNotSelected { selected, reason } => write!(
                formatter,
                "Sixel output is disabled because terminal policy selected {selected:?} ({reason:?})"
            ),
            Self::Interrupted => formatter.write_str("Sixel output was interrupted"),
            Self::Io(error) => write!(formatter, "failed to write Sixel output: {error}"),
        }
    }
}

impl StdError for SixelError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{io, sync::Arc};

    use super::{DEFAULT_SIXEL_LIMITS, STRING_TERMINATOR, SixelError, SixelLimits, SixelPlan};
    use crate::{
        core::{
            Cancellation, LossPolicy, PointRepresentation,
            point::{
                PointBatch, PointColumn, PointDimensions, PointField, PointFieldSemantic,
                PointFrameMetadata, PointSchema, PrimitiveType, Timestamp,
            },
        },
        ops::{
            ColorPolicy, DepthPolicy, IntensityRange, InvalidProjectionCoordinatePolicy,
            OrthographicView, Projection, Raster, RasterDimensions, Rgb8,
        },
        terminal::{Backend, Selection, SelectionReason},
    };

    fn selected(backend: Backend, reason: SelectionReason) -> Selection {
        super::super::selection(backend, reason, false)
    }

    fn sixel_selected() -> Selection {
        selected(Backend::Sixel, SelectionReason::Explicit)
    }

    fn raster(points: &[[f64; 4]], width: usize, height: usize) -> Raster {
        let schema = Arc::new(
            PointSchema::new(vec![
                PointField::new("x", PrimitiveType::F64, 1, Some(PointFieldSemantic::X)).unwrap(),
                PointField::new("y", PrimitiveType::F64, 1, Some(PointFieldSemantic::Y)).unwrap(),
                PointField::new("z", PrimitiveType::F64, 1, Some(PointFieldSemantic::Z)).unwrap(),
                PointField::new(
                    "intensity",
                    PrimitiveType::F64,
                    1,
                    Some(PointFieldSemantic::Intensity),
                )
                .unwrap(),
            ])
            .unwrap(),
        );
        let dimensions = PointDimensions::new(points.len(), 1).unwrap();
        let batch = PointBatch::new(
            Arc::clone(&schema),
            Arc::new(PointFrameMetadata::new(
                Timestamp::new(1, 2).unwrap(),
                "hostile-\u{1b}]8;;terminal-metadata",
                false,
            )),
            dimensions,
            vec![
                PointColumn::F64(points.iter().map(|point| point[0]).collect()),
                PointColumn::F64(points.iter().map(|point| point[1]).collect()),
                PointColumn::F64(points.iter().map(|point| point[2]).collect()),
                PointColumn::F64(points.iter().map(|point| point[3]).collect()),
            ],
        )
        .unwrap();
        Projection::new(
            RasterDimensions::new(width, height).unwrap(),
            OrthographicView::xy(),
            DepthPolicy::Nearest,
            InvalidProjectionCoordinatePolicy::Drop,
            ColorPolicy::Intensity {
                range: IntensityRange::new(0.0, 255.0).unwrap(),
                invalid: Rgb8([27, 155, 92]),
            },
        )
        .plan(
            schema,
            dimensions,
            PointRepresentation::Columns,
            &LossPolicy::lossless(),
        )
        .unwrap()
        .execute_batch(&batch)
        .unwrap()
    }

    fn golden_raster() -> Raster {
        raster(&[[-1.0, -1.0, 0.0, 0.0], [1.0, 1.0, 0.0, 255.0]], 4, 6)
    }

    fn decode_hex(input: &str) -> Vec<u8> {
        let input = input.trim();
        assert_eq!(input.len() % 2, 0);
        input
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                let pair = std::str::from_utf8(pair).unwrap();
                u8::from_str_radix(pair, 16).unwrap()
            })
            .collect()
    }

    #[test]
    fn deterministic_bytes_match_reviewed_fixture_and_exact_plan() {
        let raster = golden_raster();
        let plan = SixelPlan::new(&raster, DEFAULT_SIXEL_LIMITS).unwrap();
        let mut actual = Vec::new();
        plan.write_selected(&mut actual, &Cancellation::default(), sixel_selected())
            .unwrap();
        let expected = decode_hex(include_str!("../../tests/golden/sixel_bytes.hex"));

        assert_eq!(actual, expected);
        assert_eq!(plan.encoded_bytes(), actual.len() as u64);
        assert_eq!(plan.palette_colors(), 2);
        assert_eq!(
            SixelPlan::encoder_memory_bound(),
            crate::core::ByteBound::bounded(SixelPlan::encoder_memory_bytes())
        );
    }

    #[test]
    fn dimension_palette_and_payload_limits_refuse_before_output() {
        let raster = golden_raster();
        let dimensions = SixelLimits::new(3, 6, 2, u64::MAX).unwrap();
        assert!(matches!(
            SixelPlan::new(&raster, dimensions),
            Err(SixelError::DimensionLimitExceeded { .. })
        ));

        let palette = SixelLimits::new(4, 6, 1, u64::MAX).unwrap();
        assert!(matches!(
            SixelPlan::new(&raster, palette),
            Err(SixelError::PaletteLimitExceeded {
                required_at_least: 2,
                maximum: 1
            })
        ));

        let exact = SixelPlan::new(&raster, DEFAULT_SIXEL_LIMITS)
            .unwrap()
            .encoded_bytes();
        let payload = SixelLimits::new(4, 6, 2, exact - 1).unwrap();
        assert!(matches!(
            SixelPlan::new(&raster, payload),
            Err(SixelError::PayloadLimitExceeded {
                required,
                maximum
            }) if required == exact && maximum == exact - 1
        ));

        assert!(SixelLimits::new(0, 1, 1, 1).is_err());
        assert!(SixelLimits::new(1, 1, 0, 1).is_err());
        assert!(SixelLimits::new(1, 1, 257, 1).is_err());
    }

    #[test]
    fn every_non_sixel_policy_selection_refuses_before_dcs_entry() {
        let raster = golden_raster();
        let plan = SixelPlan::new(&raster, DEFAULT_SIXEL_LIMITS).unwrap();
        for (backend, reason) in [
            (Backend::Plain, SelectionReason::RedirectedStdout),
            (Backend::Unicode, SelectionReason::QueryUnsupported),
            (Backend::Kitty, SelectionReason::QueryConfirmed),
        ] {
            let mut output = Vec::new();
            let error = plan
                .write_selected(
                    &mut output,
                    &Cancellation::default(),
                    selected(backend, reason),
                )
                .unwrap_err();
            assert!(matches!(
                error,
                SixelError::BackendNotSelected {
                    selected,
                    reason: selected_reason,
                } if selected == backend && selected_reason == reason
            ));
            assert!(output.is_empty());
        }
    }

    #[test]
    fn canonical_redirected_policy_cannot_emit_sixel_escapes() {
        use std::ffi::OsString;

        struct RedirectedContext;
        impl crate::terminal::DetectionContext for RedirectedContext {
            fn stdout_is_terminal(&self) -> bool {
                false
            }

            fn stdin_is_terminal(&self) -> bool {
                true
            }

            fn environment(&self, _name: &str) -> Option<OsString> {
                None
            }
        }

        struct UnusedQuery;
        impl crate::terminal::CapabilityQuery for UnusedQuery {
            fn query(&self, _timeout: std::time::Duration) -> crate::terminal::QueryResult {
                panic!("redirected stdout must not query")
            }
        }

        let selection = crate::terminal::select_backend(
            crate::terminal::BackendChoice::Auto,
            &RedirectedContext,
            Arc::new(UnusedQuery),
        )
        .unwrap();
        let raster = golden_raster();
        let plan = SixelPlan::new(&raster, DEFAULT_SIXEL_LIMITS).unwrap();
        let mut output = Vec::new();
        assert!(matches!(
            plan.write_selected(&mut output, &Cancellation::default(), selection,),
            Err(SixelError::BackendNotSelected {
                selected: Backend::Plain,
                reason: SelectionReason::RedirectedStdout,
            })
        ));
        assert!(output.is_empty());
    }

    #[test]
    fn dcs_contains_only_protocol_ascii_not_metadata_or_raw_color_bytes() {
        let raster = raster(&[[0.0, 0.0, 0.0, f64::NAN]], 1, 1);
        let plan = SixelPlan::new(&raster, DEFAULT_SIXEL_LIMITS).unwrap();
        let mut output = Vec::new();
        plan.write_selected(&mut output, &Cancellation::default(), sixel_selected())
            .unwrap();

        assert_eq!(output.iter().filter(|&&byte| byte == 0x1b).count(), 2);
        assert!(
            output[2..output.len() - 2]
                .iter()
                .all(|byte| (0x20..=0x7e).contains(byte))
        );
        assert!(!output.windows(3).any(|bytes| bytes == [27, 155, 92]));
        assert!(!output.windows(4).any(|bytes| bytes == b"]8;;"));
    }

    struct CancellingWriter {
        bytes: Vec<u8>,
        cancellation: Cancellation,
        cancel_after: usize,
    }

    impl io::Write for CancellingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(bytes);
            if self.bytes.len() >= self.cancel_after {
                self.cancellation.cancel();
            }
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn cancellation_before_entry_writes_nothing_and_after_entry_terminates_dcs() {
        let raster = golden_raster();
        let plan = SixelPlan::new(&raster, DEFAULT_SIXEL_LIMITS).unwrap();

        let before = Cancellation::default();
        before.cancel();
        let mut empty = Vec::new();
        assert!(matches!(
            plan.write_selected(&mut empty, &before, sixel_selected()),
            Err(SixelError::Interrupted)
        ));
        assert!(empty.is_empty());

        let during = Cancellation::default();
        let mut writer = CancellingWriter {
            bytes: Vec::new(),
            cancellation: during.clone(),
            cancel_after: 4,
        };
        assert!(matches!(
            plan.write_selected(&mut writer, &during, sixel_selected()),
            Err(SixelError::Interrupted)
        ));
        assert!(writer.bytes.starts_with(b"\x1bP"));
        assert!(writer.bytes.ends_with(STRING_TERMINATOR));
    }
}
