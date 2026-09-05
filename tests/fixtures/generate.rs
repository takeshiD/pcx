//! Deterministic synthetic fixture generator for issue #15.
//!
//! This is test-data tooling, not a parser or product implementation. The raw
//! MCAP seed is rewritten by the official `mcap` CLI before it is checked in.

use std::{env, fs, path::Path};

const MCAP_MAGIC: &[u8; 8] = b"\x89MCAP0\r\n";

#[derive(Clone, Copy)]
enum Endian {
    Little,
    Big,
}

struct Cdr {
    bytes: Vec<u8>,
    endian: Endian,
}

impl Cdr {
    fn new(endian: Endian) -> Self {
        let bytes = Vec::from(match endian {
            Endian::Little => [0x00, 0x01, 0x00, 0x00],
            Endian::Big => [0x00, 0x00, 0x00, 0x00],
        });
        Self { bytes, endian }
    }

    fn align(&mut self, alignment: usize) {
        while self.bytes.len() % alignment != 0 {
            self.bytes.push(0);
        }
    }

    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn u32(&mut self, value: u32) {
        self.align(4);
        self.bytes.extend(match self.endian {
            Endian::Little => value.to_le_bytes(),
            Endian::Big => value.to_be_bytes(),
        });
    }

    fn i32(&mut self, value: i32) {
        self.align(4);
        self.bytes.extend(match self.endian {
            Endian::Little => value.to_le_bytes(),
            Endian::Big => value.to_be_bytes(),
        });
    }

    fn string(&mut self, value: &str) {
        self.u32(u32::try_from(value.len() + 1).unwrap());
        self.bytes.extend(value.as_bytes());
        self.bytes.push(0);
    }
}

fn pointcloud2_cdr(endian: Endian, invalid_z_offset: bool) -> Vec<u8> {
    let mut cdr = Cdr::new(endian);
    cdr.i32(1_700_000_000);
    cdr.u32(123_456_789);
    cdr.string("map");
    cdr.u32(1); // height
    cdr.u32(2); // width
    cdr.u32(5); // fields length

    for (name, offset, datatype) in [
        ("x", 0, 7),
        ("y", 4, 7),
        ("z", if invalid_z_offset { 14 } else { 8 }, 7),
        ("intensity", 12, 4),
        ("ring", 14, 4),
    ] {
        cdr.string(name);
        cdr.u32(offset);
        cdr.u8(datatype);
        cdr.u32(1);
    }

    cdr.u8(matches!(endian, Endian::Big) as u8);
    cdr.u32(16); // point_step
    cdr.u32(32); // row_step
    cdr.u32(32); // data sequence length

    for (x, y, z, intensity, ring) in [
        (1.0_f32, -2.5_f32, 0.0_f32, 42_u16, 7_u16),
        (
            -0.0_f32,
            f32::INFINITY,
            f32::from_bits(0x7fc0_1234),
            65_535_u16,
            8_u16,
        ),
    ] {
        let float_bytes = |value: f32| match endian {
            Endian::Little => value.to_bits().to_le_bytes(),
            Endian::Big => value.to_bits().to_be_bytes(),
        };
        let short_bytes = |value: u16| match endian {
            Endian::Little => value.to_le_bytes(),
            Endian::Big => value.to_be_bytes(),
        };
        cdr.bytes.extend(float_bytes(x));
        cdr.bytes.extend(float_bytes(y));
        cdr.bytes.extend(float_bytes(z));
        cdr.bytes.extend(short_bytes(intensity));
        cdr.bytes.extend(short_bytes(ring));
    }
    cdr.u8(0); // is_dense: false because the second point contains NaN
    cdr.bytes
}

fn mcap_string(value: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(u32::try_from(value.len()).unwrap().to_le_bytes());
    bytes.extend(value.as_bytes());
    bytes
}

fn mcap_record(output: &mut Vec<u8>, opcode: u8, body: &[u8]) {
    output.push(opcode);
    output.extend(u64::try_from(body.len()).unwrap().to_le_bytes());
    output.extend(body);
}

fn raw_mcap_seed(payload: &[u8]) -> Vec<u8> {
    let mut output = Vec::from(*MCAP_MAGIC);

    let mut header = mcap_string("ros2");
    header.extend(mcap_string("pcx-fixture-seed/1.0.0"));
    mcap_record(&mut output, 0x01, &header);

    let schema_text = b"std_msgs/Header header\n\
uint32 height\n\
uint32 width\n\
sensor_msgs/PointField[] fields\n\
bool is_bigendian\n\
uint32 point_step\n\
uint32 row_step\n\
uint8[] data\n\
bool is_dense\n\
================================================================================\n\
MSG: std_msgs/Header\n\
builtin_interfaces/Time stamp\n\
string frame_id\n\
================================================================================\n\
MSG: builtin_interfaces/Time\n\
int32 sec\n\
uint32 nanosec\n\
================================================================================\n\
MSG: sensor_msgs/PointField\n\
uint8 INT8=1\n\
uint8 UINT8=2\n\
uint8 INT16=3\n\
uint8 UINT16=4\n\
uint8 INT32=5\n\
uint8 UINT32=6\n\
uint8 FLOAT32=7\n\
uint8 FLOAT64=8\n\
string name\n\
uint32 offset\n\
uint8 datatype\n\
uint32 count\n";
    let mut schema = Vec::from(1_u16.to_le_bytes());
    schema.extend(mcap_string("sensor_msgs/msg/PointCloud2"));
    schema.extend(mcap_string("ros2msg"));
    schema.extend(u32::try_from(schema_text.len()).unwrap().to_le_bytes());
    schema.extend(schema_text);
    mcap_record(&mut output, 0x03, &schema);

    let mut channel = Vec::from(1_u16.to_le_bytes());
    channel.extend(1_u16.to_le_bytes());
    channel.extend(mcap_string("/lidar/points"));
    channel.extend(mcap_string("cdr"));
    channel.extend(0_u32.to_le_bytes()); // empty metadata map
    mcap_record(&mut output, 0x04, &channel);

    let mut message = Vec::from(1_u16.to_le_bytes());
    message.extend(0_u32.to_le_bytes());
    message.extend(1_700_000_000_123_456_789_u64.to_le_bytes());
    message.extend(1_700_000_000_123_456_789_u64.to_le_bytes());
    message.extend(payload);
    mcap_record(&mut output, 0x05, &message);

    mcap_record(&mut output, 0x0f, &0_u32.to_le_bytes());
    let mut footer = Vec::from(0_u64.to_le_bytes());
    footer.extend(0_u64.to_le_bytes());
    footer.extend(0_u32.to_le_bytes());
    mcap_record(&mut output, 0x02, &footer);
    output.extend(MCAP_MAGIC);
    output
}

fn pcd_header(points: u32, data: &str) -> String {
    format!(
        "# .PCD v0.7 - Point Cloud Data file format\n\
VERSION 0.7\n\
FIELDS x y z intensity ring\n\
SIZE 4 4 4 2 2\n\
TYPE F F F U U\n\
COUNT 1 1 1 1 1\n\
WIDTH 2\n\
HEIGHT 1\n\
VIEWPOINT 0 0 0 1 0 0 0\n\
POINTS {points}\n\
DATA {data}\n"
    )
}

fn binary_pcd() -> Vec<u8> {
    let mut output = pcd_header(2, "binary").into_bytes();
    for (x, y, z, intensity, ring) in [
        (1.0_f32, -2.5_f32, 0.0_f32, 42_u16, 7_u16),
        (
            -0.0_f32,
            f32::INFINITY,
            f32::from_bits(0x7fc0_1234),
            65_535_u16,
            8_u16,
        ),
    ] {
        output.extend(x.to_le_bytes());
        output.extend(y.to_le_bytes());
        output.extend(z.to_le_bytes());
        output.extend(intensity.to_le_bytes());
        output.extend(ring.to_le_bytes());
    }
    output
}

fn write(root: &Path, relative: &str, bytes: impl AsRef<[u8]>) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn main() {
    let root = env::args()
        .nth(1)
        .expect("usage: generate OUTPUT_DIRECTORY");
    let root = Path::new(&root);
    let little = pointcloud2_cdr(Endian::Little, false);
    let big = pointcloud2_cdr(Endian::Big, false);

    write(root, "valid/pointcloud2-little-endian.cdr", &little);
    write(root, "valid/pointcloud2-big-endian.cdr", &big);
    write(root, "valid/pointcloud2-binary.pcd", binary_pcd());
    write(
        root,
        "valid/pointcloud2-ascii.pcd",
        format!(
            "{}1 -2.5 0 42 7\n-0 inf nan 65535 8\n",
            pcd_header(2, "ascii")
        ),
    );
    write(root, "raw-valid-pointcloud2.mcap", raw_mcap_seed(&little));

    let mut bad_encapsulation = little.clone();
    bad_encapsulation[..2].copy_from_slice(&[0x7f, 0xff]);
    write(
        root,
        "malformed/cdr-representation-identifier-must-be-cdr.cdr",
        bad_encapsulation,
    );
    write(
        root,
        "malformed/pointcloud2-field-must-fit-point-step.cdr",
        pointcloud2_cdr(Endian::Little, true),
    );
    write(
        root,
        "malformed/cdr-point-data-sequence-must-not-be-truncated.cdr",
        &little[..little.len() - 8],
    );
    write(
        root,
        "malformed/pcd-points-must-equal-width-times-height.pcd",
        format!("{}1 -2.5 0 42 7\n", pcd_header(1, "ascii")),
    );
}
