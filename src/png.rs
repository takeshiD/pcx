//! Deterministic, bounded PNG encoding for the common projected raster.

use std::{
    error::Error as StdError,
    fmt,
    io::{self, Write},
};

use crate::{
    core::{ByteBound, Cancellation},
    ops::Raster,
};

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const IHDR: &[u8; 4] = b"IHDR";
const IDAT: &[u8; 4] = b"IDAT";
const IEND: &[u8; 4] = b"IEND";
const ZLIB_HEADER: [u8; 2] = [0x78, 0x01];
const DEFLATE_HEADER_BYTES: usize = 5;
const ADLER_BYTES: usize = 4;

/// Fixed scratch used to emit one bounded IDAT chunk at a time.
pub const PNG_ENCODER_BUFFER_BYTES: u64 = 8 * 1024;
const BUFFER_BYTES: usize = PNG_ENCODER_BUFFER_BYTES as usize;

/// A deterministic RGBA8 PNG encoding plan for one projected raster.
#[derive(Clone, Copy, Debug)]
pub struct PngPlan<'a> {
    raster: &'a Raster,
    row_bytes: usize,
    raw_bytes: usize,
    output_bytes: u64,
}

impl<'a> PngPlan<'a> {
    pub fn new(raster: &'a Raster) -> Result<Self, PngError> {
        let dimensions = raster.dimensions();
        u32::try_from(dimensions.width()).map_err(|_| PngError::DimensionExceeded)?;
        u32::try_from(dimensions.height()).map_err(|_| PngError::DimensionExceeded)?;
        let row_bytes = dimensions
            .width()
            .checked_mul(4)
            .and_then(|bytes| bytes.checked_add(1))
            .ok_or(PngError::SizeOverflow)?;
        let raw_bytes = row_bytes
            .checked_mul(dimensions.height())
            .ok_or(PngError::SizeOverflow)?;
        let output_bytes = encoded_size(raw_bytes)?;
        Ok(Self {
            raster,
            row_bytes,
            raw_bytes,
            output_bytes,
        })
    }

    pub const fn encoder_memory_bound() -> ByteBound {
        ByteBound::bounded(PNG_ENCODER_BUFFER_BYTES)
    }

    pub const fn output_bytes(&self) -> u64 {
        self.output_bytes
    }

    /// Stream a PNG without retaining a second frame-sized image.
    pub fn write(
        &self,
        writer: &mut impl Write,
        cancellation: &Cancellation,
    ) -> Result<(), PngError> {
        if cancellation.is_cancelled() {
            return Err(PngError::Interrupted);
        }
        writer.write_all(PNG_SIGNATURE).map_err(PngError::Io)?;
        let dimensions = self.raster.dimensions();
        let mut header = [0_u8; 13];
        header[..4].copy_from_slice(&(dimensions.width() as u32).to_be_bytes());
        header[4..8].copy_from_slice(&(dimensions.height() as u32).to_be_bytes());
        header[8] = 8; // bit depth
        header[9] = 6; // RGBA
        write_chunk(writer, IHDR, &header).map_err(PngError::Io)?;

        let mut buffer = [0_u8; BUFFER_BYTES];
        let mut raw_offset = 0_usize;
        let mut first = true;
        let mut adler = Adler32::new();
        while raw_offset < self.raw_bytes {
            if cancellation.is_cancelled() {
                return Err(PngError::Interrupted);
            }
            let mut cursor = 0;
            if first {
                buffer[..2].copy_from_slice(&ZLIB_HEADER);
                cursor = 2;
            }
            let remaining = self.raw_bytes - raw_offset;
            let final_capacity = BUFFER_BYTES - cursor - DEFLATE_HEADER_BYTES - ADLER_BYTES;
            let final_block = remaining <= final_capacity;
            let capacity = BUFFER_BYTES
                - cursor
                - DEFLATE_HEADER_BYTES
                - if final_block { ADLER_BYTES } else { 0 };
            let block_bytes = remaining.min(capacity).min(usize::from(u16::MAX));
            buffer[cursor] = u8::from(final_block);
            let length = block_bytes as u16;
            buffer[cursor + 1..cursor + 3].copy_from_slice(&length.to_le_bytes());
            buffer[cursor + 3..cursor + 5].copy_from_slice(&(!length).to_le_bytes());
            cursor += DEFLATE_HEADER_BYTES;
            for slot in &mut buffer[cursor..cursor + block_bytes] {
                let byte = self.raw_byte(raw_offset);
                *slot = byte;
                adler.update(byte);
                raw_offset += 1;
            }
            cursor += block_bytes;
            if final_block {
                buffer[cursor..cursor + ADLER_BYTES].copy_from_slice(&adler.finish().to_be_bytes());
                cursor += ADLER_BYTES;
            }
            write_chunk(writer, IDAT, &buffer[..cursor]).map_err(PngError::Io)?;
            first = false;
        }
        if cancellation.is_cancelled() {
            return Err(PngError::Interrupted);
        }
        write_chunk(writer, IEND, &[]).map_err(PngError::Io)
    }

    fn raw_byte(&self, offset: usize) -> u8 {
        let within_row = offset % self.row_bytes;
        if within_row == 0 {
            return 0; // PNG filter method None
        }
        let row = offset / self.row_bytes;
        let pixel_byte = within_row - 1;
        let column = pixel_byte / 4;
        let channel = pixel_byte % 4;
        match self.raster.pixel(row, column) {
            Some(pixel) if channel < 3 => pixel.color().0[channel],
            Some(_) => 255,
            None => 0,
        }
    }
}

fn encoded_size(raw_bytes: usize) -> Result<u64, PngError> {
    let mut remaining = raw_bytes;
    let mut first = true;
    let mut idat_bytes = 0_u64;
    while remaining > 0 {
        let prefix = if first { ZLIB_HEADER.len() } else { 0 };
        let final_capacity = BUFFER_BYTES - prefix - DEFLATE_HEADER_BYTES - ADLER_BYTES;
        let final_block = remaining <= final_capacity;
        let capacity = BUFFER_BYTES
            - prefix
            - DEFLATE_HEADER_BYTES
            - if final_block { ADLER_BYTES } else { 0 };
        let block_bytes = remaining.min(capacity).min(usize::from(u16::MAX));
        let data_bytes = prefix
            .checked_add(DEFLATE_HEADER_BYTES)
            .and_then(|bytes| bytes.checked_add(block_bytes))
            .and_then(|bytes| bytes.checked_add(if final_block { ADLER_BYTES } else { 0 }))
            .ok_or(PngError::SizeOverflow)?;
        idat_bytes = idat_bytes
            .checked_add(u64::try_from(data_bytes).map_err(|_| PngError::SizeOverflow)?)
            .and_then(|bytes| bytes.checked_add(12))
            .ok_or(PngError::SizeOverflow)?;
        remaining -= block_bytes;
        first = false;
    }
    8_u64
        .checked_add(25)
        .and_then(|bytes| bytes.checked_add(idat_bytes))
        .and_then(|bytes| bytes.checked_add(12))
        .ok_or(PngError::SizeOverflow)
}

fn write_chunk(writer: &mut impl Write, kind: &[u8; 4], data: &[u8]) -> io::Result<()> {
    let length =
        u32::try_from(data.len()).map_err(|_| io::Error::other("PNG chunk length exceeded u32"))?;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(kind)?;
    writer.write_all(data)?;
    let mut crc = Crc32::new();
    crc.update_slice(kind);
    crc.update_slice(data);
    writer.write_all(&crc.finish().to_be_bytes())
}

struct Adler32 {
    a: u32,
    b: u32,
}

impl Adler32 {
    const fn new() -> Self {
        Self { a: 1, b: 0 }
    }
    fn update(&mut self, byte: u8) {
        self.a = (self.a + u32::from(byte)) % 65_521;
        self.b = (self.b + self.a) % 65_521;
    }
    const fn finish(&self) -> u32 {
        (self.b << 16) | self.a
    }
}

struct Crc32(u32);

impl Crc32 {
    const fn new() -> Self {
        Self(0xffff_ffff)
    }
    fn update_slice(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 ^= u32::from(byte);
            for _ in 0..8 {
                self.0 = (self.0 >> 1) ^ (0xedb8_8320 & (0_u32.wrapping_sub(self.0 & 1)));
            }
        }
    }
    const fn finish(self) -> u32 {
        !self.0
    }
}

#[derive(Debug)]
pub enum PngError {
    DimensionExceeded,
    SizeOverflow,
    Interrupted,
    Io(io::Error),
}

impl fmt::Display for PngError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DimensionExceeded => {
                formatter.write_str("PNG dimensions exceed the u32 format limit")
            }
            Self::SizeOverflow => formatter.write_str("PNG encoded size overflowed"),
            Self::Interrupted => formatter.write_str("PNG encoding was interrupted"),
            Self::Io(error) => write!(formatter, "PNG output failed: {error}"),
        }
    }
}

impl StdError for PngError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}
