#![no_main]

use std::sync::Arc;

use libfuzzer_sys::fuzz_target;
use pcx_cli::ros2::pointcloud2;

fuzz_target!(|data: &[u8]| {
    let _ = pointcloud2::decode(Arc::from(data));
});
