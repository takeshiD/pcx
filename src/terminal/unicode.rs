//! Portable Unicode half-block rendering for the terminal-neutral CPU raster.

use std::{error::Error as StdError, fmt, io, io::Write};

use crate::ops::{Raster, RasterDimensions, Rgb8};

const UPPER_HALF_BLOCK: &str = "▀";
const LOWER_HALF_BLOCK: &str = "▄";
const FULL_BLOCK: &str = "█";
const RESET: &[u8] = b"\x1b[0m";
const MONOCHROME_MAX_CELL_BYTES: u64 = 3;
const TRUECOLOR_MAX_CELL_BYTES: u64 = 43;

/// Whether bytes are going to a terminal or a redirected stream.
///
/// This is explicit so the future capability-selection layer can own TTY
/// detection without coupling the deterministic encoder to process globals.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnicodeOutputKind {
    Tty,
    NonTty,
}

/// Requested Unicode color representation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnicodeColorPolicy {
    /// Encode occupancy using block glyphs without terminal control sequences.
    Monochrome,
    /// Encode raster RGB8 values with SGR truecolor parameters.
    ///
    /// [`UnicodeOutputKind::NonTty`] always normalizes this to [`Self::Monochrome`].
    TrueColor,
}

/// Dimensions of the text frame emitted by a Unicode render plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalCellDimensions {
    columns: usize,
    rows: usize,
}

impl TerminalCellDimensions {
    pub const fn columns(self) -> usize {
        self.columns
    }

    pub const fn rows(self) -> usize {
        self.rows
    }
}

/// A validated, deterministic Unicode encoding plan for one raster shape.
///
/// One terminal cell consumes one raster column and two raster rows. The upper
/// pixel is row `2 * cell_row`; the lower pixel is the following row. An odd
/// final raster row is encoded as an upper pixel with an empty lower half.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnicodeRenderPlan {
    raster_dimensions: RasterDimensions,
    cell_dimensions: TerminalCellDimensions,
    color: UnicodeColorPolicy,
}

impl UnicodeRenderPlan {
    /// Checked projection raster dimensions for a terminal viewport.
    ///
    /// A `columns` by `rows` viewport maps to a `columns` by `2 * rows`
    /// raster, so callers can preflight the common projection without an
    /// unchecked height multiplication.
    pub fn raster_dimensions_for_cells(
        columns: usize,
        rows: usize,
    ) -> Result<RasterDimensions, UnicodeRenderError> {
        if columns == 0 || rows == 0 {
            return Err(UnicodeRenderError::ZeroCellDimension);
        }
        let raster_rows = rows
            .checked_mul(2)
            .ok_or(UnicodeRenderError::RasterSizeOverflow)?;
        RasterDimensions::new(columns, raster_rows)
            .map_err(|_| UnicodeRenderError::RasterSizeOverflow)
    }

    pub const fn new(raster_dimensions: RasterDimensions, color: UnicodeColorPolicy) -> Self {
        let height = raster_dimensions.height();
        Self {
            raster_dimensions,
            cell_dimensions: TerminalCellDimensions {
                columns: raster_dimensions.width(),
                rows: height / 2 + height % 2,
            },
            color,
        }
    }

    pub const fn raster_dimensions(self) -> RasterDimensions {
        self.raster_dimensions
    }

    pub const fn cell_dimensions(self) -> TerminalCellDimensions {
        self.cell_dimensions
    }

    pub const fn requested_color_policy(self) -> UnicodeColorPolicy {
        self.color
    }

    /// Effective color policy after applying the non-TTY safety rule.
    pub const fn color_policy_for(self, output_kind: UnicodeOutputKind) -> UnicodeColorPolicy {
        match output_kind {
            UnicodeOutputKind::Tty => self.color,
            UnicodeOutputKind::NonTty => UnicodeColorPolicy::Monochrome,
        }
    }

    /// Encoder-owned scratch is constant and does not grow with raster size.
    pub const fn scratch_bytes(self) -> u64 {
        0
    }

    /// Conservative byte bound for the complete encoded frame.
    ///
    /// This is an output-size bound, not retained encoder memory: rendering
    /// writes each cell directly to the caller's synchronous [`Write`] sink.
    pub fn encoded_size_bound(
        self,
        output_kind: UnicodeOutputKind,
    ) -> Result<u64, UnicodeRenderError> {
        let cells = self
            .cell_dimensions
            .columns
            .checked_mul(self.cell_dimensions.rows)
            .ok_or(UnicodeRenderError::OutputSizeOverflow)?;
        let cells = u64::try_from(cells).map_err(|_| UnicodeRenderError::OutputSizeOverflow)?;
        let rows = u64::try_from(self.cell_dimensions.rows)
            .map_err(|_| UnicodeRenderError::OutputSizeOverflow)?;
        let maximum_cell_bytes = match self.color_policy_for(output_kind) {
            UnicodeColorPolicy::Monochrome => MONOCHROME_MAX_CELL_BYTES,
            UnicodeColorPolicy::TrueColor => TRUECOLOR_MAX_CELL_BYTES,
        };
        cells
            .checked_mul(maximum_cell_bytes)
            .and_then(|bytes| bytes.checked_add(rows))
            .ok_or(UnicodeRenderError::OutputSizeOverflow)
    }

    /// Stream one complete frame in row-major order.
    ///
    /// Every output row ends in LF, including the last. TTY truecolor output
    /// resets SGR after every occupied cell, so no style can leak across cells,
    /// line endings, errors after a complete cell, or subsequent shell output.
    pub fn render(
        self,
        raster: &Raster,
        output_kind: UnicodeOutputKind,
        output: &mut impl Write,
    ) -> Result<(), UnicodeRenderError> {
        if raster.dimensions() != self.raster_dimensions {
            return Err(UnicodeRenderError::RasterDoesNotMatchPlan);
        }
        self.encoded_size_bound(output_kind)?;

        let color = self.color_policy_for(output_kind);
        for cell_row in 0..self.cell_dimensions.rows {
            let upper_row = cell_row * 2;
            let lower_row = upper_row + 1;
            for column in 0..self.cell_dimensions.columns {
                let upper = raster.pixel(upper_row, column).map(|pixel| pixel.color());
                let lower = raster.pixel(lower_row, column).map(|pixel| pixel.color());
                write_cell(output, color, upper, lower)?;
            }
            output.write_all(b"\n")?;
        }
        Ok(())
    }
}

fn write_cell(
    output: &mut impl Write,
    color: UnicodeColorPolicy,
    upper: Option<Rgb8>,
    lower: Option<Rgb8>,
) -> io::Result<()> {
    match color {
        UnicodeColorPolicy::Monochrome => match (upper, lower) {
            (None, None) => output.write_all(b" "),
            (Some(_), None) => output.write_all(UPPER_HALF_BLOCK.as_bytes()),
            (None, Some(_)) => output.write_all(LOWER_HALF_BLOCK.as_bytes()),
            (Some(_), Some(_)) => output.write_all(FULL_BLOCK.as_bytes()),
        },
        UnicodeColorPolicy::TrueColor => match (upper, lower) {
            (None, None) => output.write_all(b" "),
            (Some(color), None) => write_foreground(output, color, UPPER_HALF_BLOCK),
            (None, Some(color)) => write_foreground(output, color, LOWER_HALF_BLOCK),
            (Some(upper), Some(lower)) if upper == lower => {
                write_foreground(output, upper, FULL_BLOCK)
            }
            (Some(upper), Some(lower)) => {
                let [ur, ug, ub] = upper.0;
                let [lr, lg, lb] = lower.0;
                write!(
                    output,
                    "\x1b[38;2;{ur};{ug};{ub};48;2;{lr};{lg};{lb}m{UPPER_HALF_BLOCK}"
                )?;
                output.write_all(RESET)
            }
        },
    }
}

fn write_foreground(output: &mut impl Write, color: Rgb8, glyph: &str) -> io::Result<()> {
    let [red, green, blue] = color.0;
    write!(output, "\x1b[38;2;{red};{green};{blue}m{glyph}")?;
    output.write_all(RESET)
}

/// A deterministic Unicode validation or encoding failure.
#[derive(Debug)]
pub enum UnicodeRenderError {
    ZeroCellDimension,
    RasterSizeOverflow,
    OutputSizeOverflow,
    RasterDoesNotMatchPlan,
    Io(io::Error),
}

impl fmt::Display for UnicodeRenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroCellDimension => {
                formatter.write_str("Unicode terminal columns and rows must be non-zero")
            }
            Self::RasterSizeOverflow => {
                formatter.write_str("Unicode projection raster size overflowed")
            }
            Self::OutputSizeOverflow => formatter.write_str("Unicode output size overflowed"),
            Self::RasterDoesNotMatchPlan => {
                formatter.write_str("Unicode raster dimensions do not match the render plan")
            }
            Self::Io(error) => write!(formatter, "Unicode output failed: {error}"),
        }
    }
}

impl StdError for UnicodeRenderError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::ZeroCellDimension
            | Self::RasterSizeOverflow
            | Self::OutputSizeOverflow
            | Self::RasterDoesNotMatchPlan => None,
        }
    }
}

impl From<io::Error> for UnicodeRenderError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use proptest::prelude::*;

    use super::*;
    use crate::{
        core::{
            LossPolicy, PointRepresentation,
            point::{
                Endianness, PointDimensions, PointField, PointFieldSemantic, PointFrameMetadata,
                PointLayout, PointSchema, PointView, PrimitiveType, Timestamp,
            },
        },
        ops::{
            ColorPolicy, DepthPolicy, IntensityRange, InvalidProjectionCoordinatePolicy,
            OrthographicView, Projection,
        },
    };

    fn colored_raster(frame_id: &str) -> Raster {
        let schema = Arc::new(
            PointSchema::new(vec![
                PointField::new("x", PrimitiveType::F64, 1, Some(PointFieldSemantic::X)).unwrap(),
                PointField::new("y", PrimitiveType::F64, 1, Some(PointFieldSemantic::Y)).unwrap(),
                PointField::new("z", PrimitiveType::F64, 1, Some(PointFieldSemantic::Z)).unwrap(),
                PointField::new(
                    "intensity",
                    PrimitiveType::U8,
                    1,
                    Some(PointFieldSemantic::Intensity),
                )
                .unwrap(),
            ])
            .unwrap(),
        );
        let points = [
            (-1.0_f64, 1.0_f64, 0.0_f64, 0_u8),
            (-1.0, -1.0, 0.0, 255),
            (1.0, 1.0, 0.0, 128),
        ];
        let mut bytes = Vec::new();
        for (x, y, z, intensity) in points {
            bytes.extend_from_slice(&x.to_le_bytes());
            bytes.extend_from_slice(&y.to_le_bytes());
            bytes.extend_from_slice(&z.to_le_bytes());
            bytes.push(intensity);
        }
        let dimensions = PointDimensions::new(points.len(), 1).unwrap();
        let layout = PointLayout::new(
            schema,
            dimensions,
            vec![0, 8, 16, 24],
            25,
            25 * points.len(),
            0,
            Endianness::Little,
        )
        .unwrap();
        let view = PointView::new(
            Arc::from(bytes),
            Arc::new(PointFrameMetadata::new(
                Timestamp::new(1, 2).unwrap(),
                frame_id,
                false,
            )),
            layout,
        )
        .unwrap();
        let projection = Projection::new(
            RasterDimensions::new(2, 2).unwrap(),
            OrthographicView::xy(),
            DepthPolicy::Nearest,
            InvalidProjectionCoordinatePolicy::Reject,
            ColorPolicy::Intensity {
                range: IntensityRange::new(0.0, 255.0).unwrap(),
                invalid: Rgb8([255, 0, 255]),
            },
        );
        projection
            .plan(
                view.shared_schema(),
                view.layout().dimensions(),
                PointRepresentation::View,
                &LossPolicy::lossless(),
            )
            .unwrap()
            .execute_view(&view)
            .unwrap()
    }

    fn normalized_snapshot(bytes: &[u8]) -> String {
        String::from_utf8(bytes.to_vec())
            .unwrap()
            .replace('\x1b', "\\x1b")
    }

    fn contains_only_sgr_and_text(bytes: &[u8]) -> bool {
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'\n' | b' ' => index += 1,
                0x1b => {
                    if bytes.get(index + 1) != Some(&b'[') {
                        return false;
                    }
                    index += 2;
                    let start = index;
                    while bytes.get(index).is_some_and(u8::is_ascii_digit)
                        || bytes.get(index) == Some(&b';')
                    {
                        index += 1;
                    }
                    if index == start || bytes.get(index) != Some(&b'm') {
                        return false;
                    }
                    index += 1;
                }
                byte if byte < 0x20 || byte == 0x7f => return false,
                _ => index += 1,
            }
        }
        true
    }

    #[test]
    fn cell_geometry_halves_raster_height_and_preserves_odd_final_rows() {
        assert_eq!(
            UnicodeRenderPlan::raster_dimensions_for_cells(80, 24).unwrap(),
            RasterDimensions::new(80, 48).unwrap()
        );
        assert!(matches!(
            UnicodeRenderPlan::raster_dimensions_for_cells(0, 24),
            Err(UnicodeRenderError::ZeroCellDimension)
        ));
        assert!(matches!(
            UnicodeRenderPlan::raster_dimensions_for_cells(1, usize::MAX),
            Err(UnicodeRenderError::RasterSizeOverflow)
        ));

        let even = UnicodeRenderPlan::new(
            RasterDimensions::new(80, 48).unwrap(),
            UnicodeColorPolicy::Monochrome,
        );
        assert_eq!(
            even.cell_dimensions(),
            TerminalCellDimensions {
                columns: 80,
                rows: 24
            }
        );

        let odd = UnicodeRenderPlan::new(
            RasterDimensions::new(60, 25).unwrap(),
            UnicodeColorPolicy::Monochrome,
        );
        assert_eq!(odd.cell_dimensions().rows(), 13);
    }

    #[test]
    fn truecolor_frame_matches_the_reviewed_normalized_snapshot() {
        let raster = colored_raster("map");
        let plan = UnicodeRenderPlan::new(raster.dimensions(), UnicodeColorPolicy::TrueColor);
        let mut bytes = Vec::new();
        plan.render(&raster, UnicodeOutputKind::Tty, &mut bytes)
            .unwrap();

        assert!(contains_only_sgr_and_text(&bytes));
        assert!(
            u64::try_from(bytes.len()).unwrap()
                <= plan.encoded_size_bound(UnicodeOutputKind::Tty).unwrap()
        );
        assert_eq!(
            normalized_snapshot(&bytes),
            include_str!("../../tests/golden/unicode_truecolor.txt")
        );
    }

    #[test]
    fn non_tty_output_is_deterministic_monochrome_without_escape_bytes() {
        let raster = colored_raster("map");
        let plan = UnicodeRenderPlan::new(raster.dimensions(), UnicodeColorPolicy::TrueColor);
        let mut first = Vec::new();
        let mut second = Vec::new();
        plan.render(&raster, UnicodeOutputKind::NonTty, &mut first)
            .unwrap();
        plan.render(&raster, UnicodeOutputKind::NonTty, &mut second)
            .unwrap();

        assert_eq!(first, "█▀\n".as_bytes());
        assert_eq!(first, second);
        assert!(first.iter().all(|byte| *byte == b'\n' || *byte >= 0x20));
        assert!(!first.contains(&0x1b));
        assert_eq!(
            plan.color_policy_for(UnicodeOutputKind::NonTty),
            UnicodeColorPolicy::Monochrome
        );
    }

    #[test]
    fn hostile_frame_metadata_cannot_reach_either_output_mode() {
        let hostile = "map\x1b]52;c;YXR0YWNr\x07\x1b[2J\r\nforged";
        let raster = colored_raster(hostile);
        for output_kind in [UnicodeOutputKind::Tty, UnicodeOutputKind::NonTty] {
            let plan = UnicodeRenderPlan::new(raster.dimensions(), UnicodeColorPolicy::TrueColor);
            let mut bytes = Vec::new();
            plan.render(&raster, output_kind, &mut bytes).unwrap();

            assert!(
                !bytes
                    .windows(hostile.len())
                    .any(|window| window == hostile.as_bytes())
            );
            assert!(contains_only_sgr_and_text(&bytes));
            assert!(
                !bytes
                    .windows(2)
                    .any(|window| matches!(window, [0x1b, b']' | b'P' | b'_' | b'^']))
            );
        }
    }

    #[test]
    fn mismatched_raster_is_rejected_before_writing() {
        let raster = colored_raster("map");
        let plan = UnicodeRenderPlan::new(
            RasterDimensions::new(1, 1).unwrap(),
            UnicodeColorPolicy::Monochrome,
        );
        let mut output = Vec::new();
        assert!(matches!(
            plan.render(&raster, UnicodeOutputKind::NonTty, &mut output),
            Err(UnicodeRenderError::RasterDoesNotMatchPlan)
        ));
        assert!(output.is_empty());
    }

    #[test]
    fn declared_cell_byte_bounds_cover_every_occupancy_mapping() {
        let black = Rgb8([0, 0, 0]);
        let white = Rgb8([255, 255, 255]);
        for (upper, lower) in [
            (None, None),
            (Some(black), None),
            (None, Some(black)),
            (Some(black), Some(black)),
            (Some(white), Some(Rgb8([254, 254, 254]))),
        ] {
            let mut monochrome = Vec::new();
            write_cell(
                &mut monochrome,
                UnicodeColorPolicy::Monochrome,
                upper,
                lower,
            )
            .unwrap();
            assert!(monochrome.len() <= usize::try_from(MONOCHROME_MAX_CELL_BYTES).unwrap());

            let mut truecolor = Vec::new();
            write_cell(&mut truecolor, UnicodeColorPolicy::TrueColor, upper, lower).unwrap();
            assert!(truecolor.len() <= usize::try_from(TRUECOLOR_MAX_CELL_BYTES).unwrap());
        }

        let mut longest = Vec::new();
        write_cell(
            &mut longest,
            UnicodeColorPolicy::TrueColor,
            Some(white),
            Some(Rgb8([254, 254, 254])),
        )
        .unwrap();
        assert_eq!(
            longest.len(),
            usize::try_from(TRUECOLOR_MAX_CELL_BYTES).unwrap()
        );
    }

    proptest! {
        #[test]
        fn arbitrary_raster_colors_can_only_form_sgr_sequences(
            upper in proptest::array::uniform3(any::<u8>()),
            lower in proptest::array::uniform3(any::<u8>()),
        ) {
            let mut bytes = Vec::new();
            write_cell(
                &mut bytes,
                UnicodeColorPolicy::TrueColor,
                Some(Rgb8(upper)),
                Some(Rgb8(lower)),
            ).unwrap();

            prop_assert!(contains_only_sgr_and_text(&bytes));
            prop_assert!(!bytes.windows(2).any(|window| matches!(window, [0x1b, b']' | b'P' | b'_' | b'^'])));
            prop_assert!(bytes.len() <= usize::try_from(TRUECOLOR_MAX_CELL_BYTES).unwrap());
        }
    }
}
