//! Deterministic content-based routing for supported Source adapters.

use std::io::{self, Read, Seek, SeekFrom};

const MCAP_MAGIC: &[u8; 8] = b"\x89MCAP0\r\n";
const MAX_PCD_SIGNATURE_BYTES: usize = 64 * 1024;

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
    let mut line = [0_u8; MAX_PCD_SIGNATURE_BYTES];
    let mut line_len = 0_usize;
    let mut consumed = 0_usize;
    loop {
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
        if consumed > MAX_PCD_SIGNATURE_BYTES {
            return Ok(SourceKind::Mcap);
        }
        if byte != b'\n' {
            line[line_len] = byte;
            line_len += 1;
            continue;
        }

        let candidate = trim_ascii(&line[..line_len]);
        line_len = 0;
        if candidate.is_empty() || candidate.starts_with(b"#") {
            continue;
        }
        return Ok(if is_pcd_signature(candidate) {
            SourceKind::Pcd
        } else {
            SourceKind::Mcap
        });
    }
}

fn is_pcd_signature(line: &[u8]) -> bool {
    let mut tokens = line
        .split(u8::is_ascii_whitespace)
        .filter(|token| !token.is_empty());
    tokens.next() == Some(b"VERSION".as_slice())
        && tokens.next() == Some(b"0.7".as_slice())
        && tokens.next().is_none()
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

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Seek};

    use super::{SourceKind, probe_kind};

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
}
