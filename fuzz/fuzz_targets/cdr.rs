#![no_main]

use libfuzzer_sys::fuzz_target;
use pcx_cli::ros2::cdr::Cursor;

fuzz_target!(|data: &[u8]| {
    let Ok(cursor) = Cursor::new(data) else {
        return;
    };

    let mut scalar = cursor.clone();
    let _ = scalar.read_bool();
    let _ = scalar.read_u8();
    let _ = scalar.read_i8();
    let _ = scalar.read_u16();
    let _ = scalar.read_i16();
    let _ = scalar.read_u32();
    let _ = scalar.read_i32();
    let _ = scalar.read_u64();
    let _ = scalar.read_i64();
    let _ = scalar.read_f32();
    let _ = scalar.read_f64();
    let _ = scalar.read_string();
    let _ = scalar.read_sequence_len();
    let _ = scalar.read_byte_sequence();

    for operation in data.iter().copied().take(64) {
        let result = match operation % 8 {
            0 => cursor.clone().read_bool().map(|_| ()),
            1 => cursor.clone().read_u8().map(|_| ()),
            2 => cursor.clone().read_u16().map(|_| ()),
            3 => cursor.clone().read_u32().map(|_| ()),
            4 => cursor.clone().read_u64().map(|_| ()),
            5 => cursor.clone().read_string().map(|_| ()),
            6 => cursor.clone().read_sequence_len().map(|_| ()),
            _ => cursor.clone().read_byte_sequence().map(|_| ()),
        };
        let _ = std::hint::black_box(result);
    }
});
