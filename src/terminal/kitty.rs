//! Bounded Kitty graphics protocol encoding for terminal-neutral CPU rasters.
//!
//! Capability detection remains in the parent module. This encoder emits
//! escapes only after that policy has selected [`Backend::Kitty`].

use std::{fmt, fmt::Write as _, io, num::NonZeroU32};

use crate::{
    core::{ByteBound, Cancellation, ErrorCategory},
    ops::Raster,
    terminal::{Backend, Selection},
};

/// Maximum base64 payload bytes in one Kitty APC command.
pub const KITTY_CHUNK_BYTES: usize = 4096;
/// Raw bytes which encode to exactly one full Kitty payload chunk.
pub const KITTY_RAW_CHUNK_BYTES: usize = KITTY_CHUNK_BYTES / 4 * 3;
const KITTY_CONTROL_BYTES: usize = 128;
const KITTY_COMMAND_BYTES: usize = KITTY_CONTROL_BYTES + KITTY_CHUNK_BYTES + 2;
/// Exact fixed workspace used while streaming one image.
pub const KITTY_ENCODER_BUFFER_BYTES: u64 = (KITTY_RAW_CHUNK_BYTES + KITTY_COMMAND_BYTES) as u64;

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Hard protocol and payload limits checked before the first escape is emitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KittyLimits {
    max_width: usize,
    max_height: usize,
    max_payload_bytes: u64,
}

impl KittyLimits {
    pub const fn new(max_width: usize, max_height: usize, max_payload_bytes: u64) -> Self {
        Self {
            max_width,
            max_height,
            max_payload_bytes,
        }
    }

    pub const fn max_width(self) -> usize {
        self.max_width
    }

    pub const fn max_height(self) -> usize {
        self.max_height
    }

    /// Maximum total base64 image payload, excluding APC control bytes.
    pub const fn max_payload_bytes(self) -> u64 {
        self.max_payload_bytes
    }
}

impl Default for KittyLimits {
    fn default() -> Self {
        Self::new(4096, 4096, 64 * 1024 * 1024)
    }
}

/// A direct-transfer Kitty encoder with caller-owned image identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KittyEncoder {
    image_id: NonZeroU32,
    limits: KittyLimits,
}

impl KittyEncoder {
    pub const fn new(image_id: NonZeroU32, limits: KittyLimits) -> Self {
        Self { image_id, limits }
    }

    pub const fn image_id(self) -> NonZeroU32 {
        self.image_id
    }

    pub const fn limits(self) -> KittyLimits {
        self.limits
    }

    /// Fixed encoder workspace for managed-memory preflight.
    pub const fn memory_bound(self) -> ByteBound {
        ByteBound::bounded(KITTY_ENCODER_BUFFER_BYTES)
    }

    /// Validate dimensions and return exact payload/chunk/output bounds.
    pub fn plan(self, raster: &Raster) -> Result<KittyPlan, KittyError> {
        let dimensions = raster.dimensions();
        if dimensions.width() > self.limits.max_width
            || dimensions.height() > self.limits.max_height
            || dimensions.width() > u32::MAX as usize
            || dimensions.height() > u32::MAX as usize
        {
            return Err(KittyError::DimensionsExceeded {
                width: dimensions.width(),
                height: dimensions.height(),
                max_width: self.limits.max_width.min(u32::MAX as usize),
                max_height: self.limits.max_height.min(u32::MAX as usize),
            });
        }

        let raw_bytes = dimensions
            .pixel_count()
            .checked_mul(4)
            .ok_or(KittyError::PayloadSizeOverflow)?;
        let payload_bytes = base64_length(raw_bytes).ok_or(KittyError::PayloadSizeOverflow)?;
        let payload_bytes_u64 =
            u64::try_from(payload_bytes).map_err(|_| KittyError::PayloadSizeOverflow)?;
        if payload_bytes_u64 > self.limits.max_payload_bytes {
            return Err(KittyError::PayloadLimitExceeded {
                requested: payload_bytes_u64,
                maximum: self.limits.max_payload_bytes,
            });
        }
        let chunks = payload_bytes.div_ceil(KITTY_CHUNK_BYTES);
        let first_control = first_control_length(
            dimensions.width(),
            dimensions.height(),
            self.image_id,
            chunks > 1,
        );
        let continuation_control = continuation_control_length();
        let control_bytes = first_control
            .checked_add(
                continuation_control
                    .checked_mul(chunks.saturating_sub(1))
                    .ok_or(KittyError::PayloadSizeOverflow)?,
            )
            .ok_or(KittyError::PayloadSizeOverflow)?;
        let output_bytes = payload_bytes
            .checked_add(control_bytes)
            .ok_or(KittyError::PayloadSizeOverflow)?;

        Ok(KittyPlan {
            raw_bytes,
            payload_bytes,
            chunks,
            output_bytes,
        })
    }

    /// Stream one image only when the shared capability policy selected Kitty.
    ///
    /// Empty raster cells are transparent RGBA pixels. Occupied cells retain
    /// their projected RGB and use alpha 255. `C=1` preserves cursor position.
    pub fn write<W: io::Write>(
        self,
        selection: Selection,
        raster: &Raster,
        cancellation: &Cancellation,
        writer: &mut W,
    ) -> Result<KittyWriteOutcome, KittyError> {
        if selection.backend() != Backend::Kitty {
            return Ok(KittyWriteOutcome::Fallback(selection.backend()));
        }
        let plan = self.plan(raster)?;
        if cancellation.is_cancelled() {
            return Err(KittyError::Interrupted);
        }

        let mut raw = [0_u8; KITTY_RAW_CHUNK_BYTES];
        let mut command = [0_u8; KITTY_COMMAND_BYTES];
        let mut pixel_index = 0_usize;
        let mut component = 0_usize;
        let mut raw_remaining = plan.raw_bytes;
        let mut transmitted = false;

        for chunk_index in 0..plan.chunks {
            if cancellation.is_cancelled() {
                if transmitted {
                    self.best_effort_delete(writer);
                }
                return Err(KittyError::Interrupted);
            }

            let raw_length = raw_remaining.min(KITTY_RAW_CHUNK_BYTES);
            fill_raw_rgba(
                raster,
                &mut raw[..raw_length],
                &mut pixel_index,
                &mut component,
            );
            raw_remaining -= raw_length;
            let more = chunk_index + 1 < plan.chunks;
            let command_length = build_command(
                &mut command,
                chunk_index == 0,
                more,
                raster,
                self.image_id,
                &raw[..raw_length],
            );
            if let Err(error) = writer.write_all(&command[..command_length]) {
                self.best_effort_delete(writer);
                return Err(KittyError::Io(error));
            }
            transmitted = true;
        }

        if cancellation.is_cancelled() {
            self.best_effort_delete(writer);
            return Err(KittyError::Interrupted);
        }

        Ok(KittyWriteOutcome::Rendered {
            chunks: plan.chunks,
            payload_bytes: plan.payload_bytes,
            output_bytes: plan.output_bytes,
        })
    }

    /// Delete this encoder's image only when Kitty output is selected.
    pub fn delete<W: io::Write>(
        self,
        selection: Selection,
        writer: &mut W,
    ) -> Result<KittyWriteOutcome, KittyError> {
        if selection.backend() != Backend::Kitty {
            return Ok(KittyWriteOutcome::Fallback(selection.backend()));
        }
        write_delete(writer, self.image_id).map_err(KittyError::Io)?;
        Ok(KittyWriteOutcome::Deleted)
    }

    fn best_effort_delete(self, writer: &mut impl io::Write) {
        let _ = write_delete(writer, self.image_id);
        let _ = writer.flush();
    }
}

/// Proven bounds for one encoded raster.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KittyPlan {
    raw_bytes: usize,
    payload_bytes: usize,
    chunks: usize,
    output_bytes: usize,
}

impl KittyPlan {
    pub const fn raw_bytes(self) -> usize {
        self.raw_bytes
    }

    pub const fn payload_bytes(self) -> usize {
        self.payload_bytes
    }

    pub const fn chunks(self) -> usize {
        self.chunks
    }

    pub const fn output_bytes(self) -> usize {
        self.output_bytes
    }
}

/// Observable result without conflating fallback with successful Kitty output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KittyWriteOutcome {
    Rendered {
        chunks: usize,
        payload_bytes: usize,
        output_bytes: usize,
    },
    Deleted,
    /// The caller should invoke the selected portable backend instead.
    Fallback(Backend),
}

#[derive(Debug)]
pub enum KittyError {
    DimensionsExceeded {
        width: usize,
        height: usize,
        max_width: usize,
        max_height: usize,
    },
    PayloadSizeOverflow,
    PayloadLimitExceeded {
        requested: u64,
        maximum: u64,
    },
    Interrupted,
    Io(io::Error),
}

impl KittyError {
    pub const fn category(&self) -> ErrorCategory {
        match self {
            Self::DimensionsExceeded { .. }
            | Self::PayloadSizeOverflow
            | Self::PayloadLimitExceeded { .. } => ErrorCategory::Resource,
            Self::Interrupted => ErrorCategory::Interrupted,
            Self::Io(_) => ErrorCategory::Io,
        }
    }
}

impl fmt::Display for KittyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DimensionsExceeded {
                width,
                height,
                max_width,
                max_height,
            } => write!(
                formatter,
                "Kitty raster {width}x{height} exceeds the configured {max_width}x{max_height} limit"
            ),
            Self::PayloadSizeOverflow => formatter.write_str("Kitty payload size overflowed"),
            Self::PayloadLimitExceeded { requested, maximum } => write!(
                formatter,
                "Kitty base64 payload of {requested} bytes exceeds the configured {maximum}-byte limit"
            ),
            Self::Interrupted => formatter.write_str("Kitty image transmission was interrupted"),
            Self::Io(error) => write!(formatter, "Kitty terminal write failed: {error}"),
        }
    }
}

impl std::error::Error for KittyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

fn fill_raw_rgba(
    raster: &Raster,
    output: &mut [u8],
    pixel_index: &mut usize,
    component: &mut usize,
) {
    for byte in output {
        let rgba = match raster.pixels()[*pixel_index] {
            Some(pixel) => {
                let [red, green, blue] = pixel.color().0;
                [red, green, blue, 255]
            }
            None => [0, 0, 0, 0],
        };
        *byte = rgba[*component];
        *component += 1;
        if *component == 4 {
            *component = 0;
            *pixel_index += 1;
        }
    }
}

fn build_command(
    command: &mut [u8; KITTY_COMMAND_BYTES],
    first: bool,
    more: bool,
    raster: &Raster,
    image_id: NonZeroU32,
    raw: &[u8],
) -> usize {
    let mut cursor = SliceWriter::new(command);
    if first {
        write!(
            cursor,
            "\x1b_Ga=T,f=32,s={},v={},i={},q=2,C=1,m={};",
            raster.dimensions().width(),
            raster.dimensions().height(),
            image_id,
            u8::from(more),
        )
        .expect("fixed Kitty control buffer is sufficient");
    } else {
        write!(cursor, "\x1b_Gm={},q=2;", u8::from(more))
            .expect("fixed Kitty control buffer is sufficient");
    }
    let payload_start = cursor.position;
    let payload_length = encode_base64(raw, &mut cursor.buffer[payload_start..]);
    cursor.position += payload_length;
    cursor
        .push_bytes(b"\x1b\\")
        .expect("fixed Kitty command buffer is sufficient");
    cursor.position
}

fn write_delete(writer: &mut impl io::Write, image_id: NonZeroU32) -> io::Result<()> {
    let mut command = [0_u8; 64];
    let mut cursor = SliceWriter::new(&mut command);
    write!(cursor, "\x1b_Ga=d,d=I,i={image_id},q=2\x1b\\")
        .expect("fixed Kitty delete buffer is sufficient");
    let length = cursor.position;
    writer.write_all(&command[..length])
}

fn encode_base64(input: &[u8], output: &mut [u8]) -> usize {
    let mut source = 0;
    let mut target = 0;
    while source + 3 <= input.len() {
        let bits = (u32::from(input[source]) << 16)
            | (u32::from(input[source + 1]) << 8)
            | u32::from(input[source + 2]);
        output[target] = BASE64[((bits >> 18) & 0x3f) as usize];
        output[target + 1] = BASE64[((bits >> 12) & 0x3f) as usize];
        output[target + 2] = BASE64[((bits >> 6) & 0x3f) as usize];
        output[target + 3] = BASE64[(bits & 0x3f) as usize];
        source += 3;
        target += 4;
    }
    match input.len() - source {
        1 => {
            let bits = u32::from(input[source]) << 16;
            output[target] = BASE64[((bits >> 18) & 0x3f) as usize];
            output[target + 1] = BASE64[((bits >> 12) & 0x3f) as usize];
            output[target + 2] = b'=';
            output[target + 3] = b'=';
            target += 4;
        }
        2 => {
            let bits = (u32::from(input[source]) << 16) | (u32::from(input[source + 1]) << 8);
            output[target] = BASE64[((bits >> 18) & 0x3f) as usize];
            output[target + 1] = BASE64[((bits >> 12) & 0x3f) as usize];
            output[target + 2] = BASE64[((bits >> 6) & 0x3f) as usize];
            output[target + 3] = b'=';
            target += 4;
        }
        _ => {}
    }
    target
}

fn base64_length(raw_bytes: usize) -> Option<usize> {
    raw_bytes.checked_add(2)?.checked_div(3)?.checked_mul(4)
}

fn first_control_length(width: usize, height: usize, image_id: NonZeroU32, more: bool) -> usize {
    b"\x1b_Ga=T,f=32,s=".len()
        + decimal_length(width as u64)
        + b",v=".len()
        + decimal_length(height as u64)
        + b",i=".len()
        + decimal_length(u64::from(image_id.get()))
        + b",q=2,C=1,m=".len()
        + decimal_length(u64::from(more))
        + b";\x1b\\".len()
}

fn continuation_control_length() -> usize {
    b"\x1b_Gm=0,q=2;\x1b\\".len()
}

fn decimal_length(mut value: u64) -> usize {
    let mut length = 1;
    while value >= 10 {
        value /= 10;
        length += 1;
    }
    length
}

struct SliceWriter<'a> {
    buffer: &'a mut [u8],
    position: usize,
}

impl<'a> SliceWriter<'a> {
    fn new(buffer: &'a mut [u8]) -> Self {
        Self {
            buffer,
            position: 0,
        }
    }

    fn push_bytes(&mut self, bytes: &[u8]) -> fmt::Result {
        let end = self.position.checked_add(bytes.len()).ok_or(fmt::Error)?;
        let destination = self.buffer.get_mut(self.position..end).ok_or(fmt::Error)?;
        destination.copy_from_slice(bytes);
        self.position = end;
        Ok(())
    }
}

impl fmt::Write for SliceWriter<'_> {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.push_bytes(value.as_bytes())
    }
}
