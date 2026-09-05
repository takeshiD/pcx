#![no_main]

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;
use pcx_cli::mcap::{Source, SourceOptions};

fuzz_target!(|data: &[u8]| {
    let read_chunk_bytes = data.first().map_or(1, |byte| usize::from(*byte) + 1);
    let options = SourceOptions {
        read_chunk_bytes,
        max_record_bytes: 1024 * 1024,
    };
    let Ok(mut source) = Source::new(Cursor::new(data), options) else {
        return;
    };

    while let Ok(Some(record)) = source.next_probe() {
        std::hint::black_box(record.retained_bytes());
    }
});
