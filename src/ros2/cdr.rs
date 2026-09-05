//! A checked cursor for the CDR1 representation used by ROS 2 messages.

use std::{error, fmt, str};

const ENCAPSULATION_HEADER_LEN: usize = 4;
const CDR_BE: u16 = 0x0000;
const CDR_LE: u16 = 0x0001;

/// Byte order declared by a supported CDR encapsulation header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Endianness {
    Big,
    Little,
}

/// A malformed or unsupported CDR stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    Truncated {
        offset: usize,
        needed: usize,
        remaining: usize,
    },
    UnsupportedRepresentation {
        identifier: u16,
    },
    UnsupportedEncapsulationOptions {
        options: u16,
    },
    OffsetOverflow {
        offset: usize,
        amount: usize,
    },
    InvalidBoolean {
        offset: usize,
        value: u8,
    },
    InvalidStringLength {
        offset: usize,
        length: u32,
    },
    MissingStringTerminator {
        offset: usize,
    },
    InvalidUtf8 {
        offset: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated {
                offset,
                needed,
                remaining,
            } => write!(
                formatter,
                "CDR data is truncated at byte {offset}: need {needed} bytes, have {remaining}"
            ),
            Self::UnsupportedRepresentation { identifier } => write!(
                formatter,
                "unsupported CDR representation identifier 0x{identifier:04x}"
            ),
            Self::UnsupportedEncapsulationOptions { options } => write!(
                formatter,
                "unsupported CDR encapsulation options 0x{options:04x}"
            ),
            Self::OffsetOverflow { offset, amount } => write!(
                formatter,
                "CDR offset arithmetic overflow at byte {offset} while advancing by {amount}"
            ),
            Self::InvalidBoolean { offset, value } => {
                write!(formatter, "invalid CDR boolean {value} at byte {offset}")
            }
            Self::InvalidStringLength { offset, length } => write!(
                formatter,
                "invalid CDR string length {length} at byte {offset}"
            ),
            Self::MissingStringTerminator { offset } => {
                write!(
                    formatter,
                    "CDR string is not NUL-terminated at byte {offset}"
                )
            }
            Self::InvalidUtf8 { offset } => {
                write!(
                    formatter,
                    "CDR string contains invalid UTF-8 at byte {offset}"
                )
            }
        }
    }
}

impl error::Error for Error {}

/// A zero-copy, forward-only reader for one encapsulated CDR1 stream.
#[derive(Clone, Debug)]
pub struct Cursor<'a> {
    payload: &'a [u8],
    position: usize,
    endianness: Endianness,
}

impl<'a> Cursor<'a> {
    /// Validates the encapsulation header and starts at the CDR payload.
    pub fn new(bytes: &'a [u8]) -> Result<Self, Error> {
        let header = bytes
            .get(..ENCAPSULATION_HEADER_LEN)
            .ok_or(Error::Truncated {
                offset: 0,
                needed: ENCAPSULATION_HEADER_LEN,
                remaining: bytes.len(),
            })?;
        let identifier = u16::from_be_bytes([header[0], header[1]]);
        let endianness = match identifier {
            CDR_BE => Endianness::Big,
            CDR_LE => Endianness::Little,
            _ => return Err(Error::UnsupportedRepresentation { identifier }),
        };
        let options = u16::from_be_bytes([header[2], header[3]]);
        if options != 0 {
            return Err(Error::UnsupportedEncapsulationOptions { options });
        }

        Ok(Self {
            payload: &bytes[ENCAPSULATION_HEADER_LEN..],
            position: 0,
            endianness,
        })
    }

    pub fn endianness(&self) -> Endianness {
        self.endianness
    }

    /// Current byte offset in the complete encapsulated stream.
    pub fn offset(&self) -> usize {
        ENCAPSULATION_HEADER_LEN + self.position
    }

    pub fn remaining(&self) -> usize {
        self.payload.len() - self.position
    }

    pub fn read_bool(&mut self) -> Result<bool, Error> {
        let offset = self.offset();
        match self.read_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(Error::InvalidBoolean { offset, value }),
        }
    }

    pub fn read_u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1, 1)?[0])
    }

    pub fn read_i8(&mut self) -> Result<i8, Error> {
        Ok(self.read_u8()? as i8)
    }

    pub fn read_u16(&mut self) -> Result<u16, Error> {
        let bytes = self.take_array::<2>(2)?;
        Ok(match self.endianness {
            Endianness::Big => u16::from_be_bytes(bytes),
            Endianness::Little => u16::from_le_bytes(bytes),
        })
    }

    pub fn read_i16(&mut self) -> Result<i16, Error> {
        Ok(self.read_u16()? as i16)
    }

    pub fn read_u32(&mut self) -> Result<u32, Error> {
        let bytes = self.take_array::<4>(4)?;
        Ok(match self.endianness {
            Endianness::Big => u32::from_be_bytes(bytes),
            Endianness::Little => u32::from_le_bytes(bytes),
        })
    }

    pub fn read_i32(&mut self) -> Result<i32, Error> {
        Ok(self.read_u32()? as i32)
    }

    pub fn read_u64(&mut self) -> Result<u64, Error> {
        let bytes = self.take_array::<8>(8)?;
        Ok(match self.endianness {
            Endianness::Big => u64::from_be_bytes(bytes),
            Endianness::Little => u64::from_le_bytes(bytes),
        })
    }

    pub fn read_i64(&mut self) -> Result<i64, Error> {
        Ok(self.read_u64()? as i64)
    }

    pub fn read_f32(&mut self) -> Result<f32, Error> {
        Ok(f32::from_bits(self.read_u32()?))
    }

    pub fn read_f64(&mut self) -> Result<f64, Error> {
        Ok(f64::from_bits(self.read_u64()?))
    }

    /// Reads a CDR string without allocating. The encoded length includes NUL.
    pub fn read_string(&mut self) -> Result<&'a str, Error> {
        let length_offset = self.offset();
        let length = self.read_u32()?;
        if length == 0 {
            return Err(Error::InvalidStringLength {
                offset: length_offset,
                length,
            });
        }
        let length = usize::try_from(length).map_err(|_| Error::OffsetOverflow {
            offset: self.offset(),
            amount: usize::MAX,
        })?;
        let string_offset = self.offset();
        let bytes = self.take(length, 1)?;
        let Some((&0, contents)) = bytes.split_last() else {
            return Err(Error::MissingStringTerminator {
                offset: string_offset + length - 1,
            });
        };
        str::from_utf8(contents).map_err(|_| Error::InvalidUtf8 {
            offset: string_offset,
        })
    }

    /// Reads the element count that prefixes a CDR sequence.
    ///
    /// Callers retain control over element parsing and allocation, which keeps
    /// this cursor independent of any particular ROS message shape.
    pub fn read_sequence_len(&mut self) -> Result<u32, Error> {
        self.read_u32()
    }

    /// Reads a `sequence<uint8>` as a borrowed slice.
    pub fn read_byte_sequence(&mut self) -> Result<&'a [u8], Error> {
        let length = self.read_sequence_len()?;
        let length = usize::try_from(length).map_err(|_| Error::OffsetOverflow {
            offset: self.offset(),
            amount: usize::MAX,
        })?;
        self.take(length, 1)
    }

    fn take_array<const N: usize>(&mut self, alignment: usize) -> Result<[u8; N], Error> {
        let bytes = self.take(N, alignment)?;
        let mut result = [0; N];
        result.copy_from_slice(bytes);
        Ok(result)
    }

    fn take(&mut self, length: usize, alignment: usize) -> Result<&'a [u8], Error> {
        let aligned = checked_align(self.position, alignment)?;
        if aligned > self.payload.len() {
            return Err(Error::Truncated {
                offset: self.offset(),
                needed: aligned - self.position,
                remaining: self.remaining(),
            });
        }
        let end = checked_add(aligned, length)?;
        let bytes = self.payload.get(aligned..end).ok_or(Error::Truncated {
            offset: ENCAPSULATION_HEADER_LEN + aligned,
            needed: length,
            remaining: self.payload.len() - aligned,
        })?;
        self.position = end;
        Ok(bytes)
    }
}

fn checked_align(offset: usize, alignment: usize) -> Result<usize, Error> {
    debug_assert!(alignment.is_power_of_two());
    let adjustment = alignment - 1;
    let offset = checked_add(offset, adjustment)?;
    Ok(offset & !adjustment)
}

fn checked_add(offset: usize, amount: usize) -> Result<usize, Error> {
    offset
        .checked_add(amount)
        .ok_or(Error::OffsetOverflow { offset, amount })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    enum TestEndian {
        Big,
        Little,
    }

    struct Encoder {
        bytes: Vec<u8>,
        endian: TestEndian,
    }

    impl Encoder {
        fn new(endian: TestEndian) -> Self {
            let identifier = match endian {
                TestEndian::Big => CDR_BE,
                TestEndian::Little => CDR_LE,
            };
            let mut bytes = Vec::from(identifier.to_be_bytes());
            bytes.extend_from_slice(&[0, 0]);
            Self { bytes, endian }
        }

        fn payload_len(&self) -> usize {
            self.bytes.len() - ENCAPSULATION_HEADER_LEN
        }

        fn align(&mut self, alignment: usize) {
            let padding = (alignment - self.payload_len() % alignment) % alignment;
            self.bytes.resize(self.bytes.len() + padding, 0);
        }

        fn u8(&mut self, value: u8) {
            self.bytes.push(value);
        }

        fn u16(&mut self, value: u16) {
            self.align(2);
            self.bytes.extend_from_slice(&match self.endian {
                TestEndian::Big => value.to_be_bytes(),
                TestEndian::Little => value.to_le_bytes(),
            });
        }

        fn u32(&mut self, value: u32) {
            self.align(4);
            self.bytes.extend_from_slice(&match self.endian {
                TestEndian::Big => value.to_be_bytes(),
                TestEndian::Little => value.to_le_bytes(),
            });
        }

        fn u64(&mut self, value: u64) {
            self.align(8);
            self.bytes.extend_from_slice(&match self.endian {
                TestEndian::Big => value.to_be_bytes(),
                TestEndian::Little => value.to_le_bytes(),
            });
        }

        fn string(&mut self, value: &str) {
            self.u32(u32::try_from(value.len() + 1).unwrap());
            self.bytes.extend_from_slice(value.as_bytes());
            self.bytes.push(0);
        }

        fn byte_sequence(&mut self, values: &[u8]) {
            self.u32(u32::try_from(values.len()).unwrap());
            self.bytes.extend_from_slice(values);
        }
    }

    fn encoded_fixture(endian: TestEndian) -> Vec<u8> {
        let mut encoder = Encoder::new(endian);
        encoder.u8(1);
        encoder.u8(0xfe);
        encoder.u8((-2_i8) as u8);
        encoder.u16(0x1234);
        encoder.u16((-123_i16) as u16);
        encoder.u32(0x1234_5678);
        encoder.u32((-123_456_i32) as u32);
        encoder.u64(0x0123_4567_89ab_cdef);
        encoder.u64((-123_456_789_i64) as u64);
        encoder.u32((-12.5_f32).to_bits());
        encoder.u64(42.25_f64.to_bits());
        encoder.string("frame");
        encoder.u32(2);
        encoder.u16(10);
        encoder.u16(20);
        encoder.byte_sequence(&[3, 1, 4]);
        encoder.bytes
    }

    fn decode_fixture(bytes: &[u8]) -> Result<(), Error> {
        let mut cursor = Cursor::new(bytes)?;
        assert!(cursor.read_bool()?);
        assert_eq!(cursor.read_u8()?, 0xfe);
        assert_eq!(cursor.read_i8()?, -2);
        assert_eq!(cursor.read_u16()?, 0x1234);
        assert_eq!(cursor.read_i16()?, -123);
        assert_eq!(cursor.read_u32()?, 0x1234_5678);
        assert_eq!(cursor.read_i32()?, -123_456);
        assert_eq!(cursor.read_u64()?, 0x0123_4567_89ab_cdef);
        assert_eq!(cursor.read_i64()?, -123_456_789);
        assert_eq!(cursor.read_f32()?, -12.5);
        assert_eq!(cursor.read_f64()?, 42.25);
        assert_eq!(cursor.read_string()?, "frame");
        let length = cursor.read_sequence_len()?;
        assert_eq!(length, 2);
        assert_eq!(cursor.read_u16()?, 10);
        assert_eq!(cursor.read_u16()?, 20);
        assert_eq!(cursor.read_byte_sequence()?, [3, 1, 4]);
        assert_eq!(cursor.remaining(), 0);
        assert_eq!(cursor.offset(), bytes.len());
        Ok(())
    }

    #[test]
    fn reads_all_primitives_strings_and_sequences_in_both_byte_orders() {
        for endian in [TestEndian::Big, TestEndian::Little] {
            let bytes = encoded_fixture(endian);
            decode_fixture(&bytes).unwrap();
        }
    }

    #[test]
    fn reports_declared_endianness() {
        assert_eq!(
            Cursor::new(&[0, 0, 0, 0]).unwrap().endianness(),
            Endianness::Big
        );
        assert_eq!(
            Cursor::new(&[0, 1, 0, 0]).unwrap().endianness(),
            Endianness::Little
        );
    }

    #[test]
    fn rejects_unsupported_representations_explicitly() {
        assert_eq!(
            Cursor::new(&[0, 2, 0, 0]).unwrap_err(),
            Error::UnsupportedRepresentation { identifier: 2 }
        );
        assert_eq!(
            Cursor::new(&[0, 1, 0, 1]).unwrap_err(),
            Error::UnsupportedEncapsulationOptions { options: 1 }
        );
    }

    #[test]
    fn rejects_invalid_scalar_encodings() {
        let mut boolean = Cursor::new(&[0, 1, 0, 0, 2]).unwrap();
        assert_eq!(
            boolean.read_bool().unwrap_err(),
            Error::InvalidBoolean {
                offset: ENCAPSULATION_HEADER_LEN,
                value: 2
            }
        );

        let mut empty_string = Cursor::new(&[0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
        assert!(matches!(
            empty_string.read_string(),
            Err(Error::InvalidStringLength { length: 0, .. })
        ));

        let mut unterminated = Cursor::new(&[0, 1, 0, 0, 2, 0, 0, 0, b'x', b'y']).unwrap();
        assert!(matches!(
            unterminated.read_string(),
            Err(Error::MissingStringTerminator { .. })
        ));

        let mut invalid_utf8 = Cursor::new(&[0, 1, 0, 0, 2, 0, 0, 0, 0xff, 0]).unwrap();
        assert!(matches!(
            invalid_utf8.read_string(),
            Err(Error::InvalidUtf8 { .. })
        ));
    }

    #[test]
    fn every_truncation_boundary_returns_an_error() {
        for endian in [TestEndian::Big, TestEndian::Little] {
            let bytes = encoded_fixture(endian);
            for boundary in 0..bytes.len() {
                let result = std::panic::catch_unwind(|| decode_fixture(&bytes[..boundary]));
                assert!(result.is_ok(), "parser panicked at boundary {boundary}");
                assert!(
                    result.unwrap().is_err(),
                    "parser accepted truncation at boundary {boundary}"
                );
            }
        }
    }

    #[test]
    fn checked_offset_arithmetic_cannot_wrap() {
        assert_eq!(
            checked_add(usize::MAX, 1),
            Err(Error::OffsetOverflow {
                offset: usize::MAX,
                amount: 1
            })
        );
        assert!(matches!(
            checked_align(usize::MAX, 8),
            Err(Error::OffsetOverflow { .. })
        ));
    }
}
