//! Deterministic content-based routing for supported Source adapters.

use std::io::{self, Read, Seek, SeekFrom};

const MCAP_MAGIC: &[u8; 8] = b"\x89MCAP0\r\n";
/// Maximum bytes examined while looking for the first meaningful PCD directive.
const MAX_PCD_SIGNATURE_BYTES: usize = 64 * 1024;
const VERSION_KEYWORD: &[u8] = b"VERSION";
const VERSION_VALUE: &[u8] = b"0.7";

/// Format adapter selected from Source content, never from its filename.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SourceKind {
    Mcap,
    Pcd,
}

/// Probe only the bytes needed to route a seekable Source, then rewind it.
///
/// Unknown content retains the historical MCAP path so existing diagnostics
/// and arbitrary MCAP filenames remain compatible. Future static adapters can
/// add a magic/signature variant here without changing render orchestration.
pub(crate) fn probe_kind(source: &mut (impl Read + Seek)) -> io::Result<SourceKind> {
    let start = source.stream_position()?;
    let result = probe_kind_at_current_position(source);
    let rewind = source.seek(SeekFrom::Start(start));
    match (result, rewind) {
        (Ok(kind), Ok(_)) => Ok(kind),
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}

fn probe_kind_at_current_position(source: &mut impl Read) -> io::Result<SourceKind> {
    let mut magic = [0_u8; MCAP_MAGIC.len()];
    let mut magic_len = 0_usize;
    while magic_len < magic.len() {
        let read = source.read(&mut magic[magic_len..])?;
        if read == 0 {
            break;
        }
        magic_len += read;
    }
    if magic_len == magic.len() && &magic == MCAP_MAGIC {
        return Ok(SourceKind::Mcap);
    }

    let mut prefix = magic[..magic_len].iter().copied();
    let mut signature = PcdSignature::Leading;
    let mut consumed = 0_usize;
    loop {
        if consumed == MAX_PCD_SIGNATURE_BYTES {
            return Ok(SourceKind::Mcap);
        }
        let byte = if let Some(byte) = prefix.next() {
            byte
        } else {
            let mut byte = [0_u8; 1];
            if source.read(&mut byte)? == 0 {
                return Ok(SourceKind::Mcap);
            }
            byte[0]
        };
        consumed += 1;
        if let Some(is_pcd) = signature.feed(byte) {
            return Ok(if is_pcd {
                SourceKind::Pcd
            } else {
                SourceKind::Mcap
            });
        }
    }
}

#[derive(Clone, Copy)]
enum PcdSignature {
    Leading,
    Comment,
    Keyword(usize),
    Between,
    Value(usize),
    Trailing,
}

impl PcdSignature {
    /// Return `Some(true)` for PCD and `Some(false)` for a non-PCD first directive.
    fn feed(&mut self, byte: u8) -> Option<bool> {
        if byte == b'\n' {
            return match *self {
                Self::Leading | Self::Comment => {
                    *self = Self::Leading;
                    None
                }
                Self::Trailing => Some(true),
                Self::Value(index) if index == VERSION_VALUE.len() => Some(true),
                Self::Keyword(_) | Self::Between | Self::Value(_) => Some(false),
            };
        }
        match *self {
            Self::Leading if byte.is_ascii_whitespace() => None,
            Self::Leading if byte == b'#' => {
                *self = Self::Comment;
                None
            }
            Self::Leading if byte == VERSION_KEYWORD[0] => {
                *self = Self::Keyword(1);
                None
            }
            Self::Leading => Some(false),
            Self::Comment => None,
            Self::Keyword(index) if index < VERSION_KEYWORD.len() => {
                if byte != VERSION_KEYWORD[index] {
                    return Some(false);
                }
                *self = Self::Keyword(index + 1);
                None
            }
            Self::Keyword(_) if byte.is_ascii_whitespace() => {
                *self = Self::Between;
                None
            }
            Self::Keyword(_) => Some(false),
            Self::Between if byte.is_ascii_whitespace() => None,
            Self::Between if byte == VERSION_VALUE[0] => {
                *self = Self::Value(1);
                None
            }
            Self::Between => Some(false),
            Self::Value(index) if index < VERSION_VALUE.len() => {
                if byte != VERSION_VALUE[index] {
                    return Some(false);
                }
                *self = Self::Value(index + 1);
                None
            }
            Self::Value(_) if byte.is_ascii_whitespace() => {
                *self = Self::Trailing;
                None
            }
            Self::Value(_) => Some(false),
            Self::Trailing if byte.is_ascii_whitespace() => None,
            Self::Trailing => Some(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Seek};

    use super::{MAX_PCD_SIGNATURE_BYTES, SourceKind, probe_kind, probe_kind_at_current_position};

    #[test]
    fn content_wins_over_filename_independent_routing() {
        for (bytes, expected) in [
            (b"\x89MCAP0\r\nrest".as_slice(), SourceKind::Mcap),
            (
                b"# arbitrary comment\nVERSION 0.7\nFIELDS x\n".as_slice(),
                SourceKind::Pcd,
            ),
            (b"not recognized".as_slice(), SourceKind::Mcap),
        ] {
            let mut source = Cursor::new(bytes);
            assert_eq!(probe_kind(&mut source).unwrap(), expected);
            assert_eq!(source.stream_position().unwrap(), 0);
        }
    }

    #[test]
    fn unrecognized_signature_probe_reads_no_more_than_its_fixed_bound() {
        let bytes = vec![b'#'; MAX_PCD_SIGNATURE_BYTES + 100];
        let mut source = Cursor::new(bytes);
        assert_eq!(
            probe_kind_at_current_position(&mut source).unwrap(),
            SourceKind::Mcap
        );
        assert_eq!(
            source.stream_position().unwrap(),
            u64::try_from(MAX_PCD_SIGNATURE_BYTES).unwrap()
        );
    }
}
