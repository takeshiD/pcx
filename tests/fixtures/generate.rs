//! Deterministic synthetic parser-fixture generator.
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

fn pointcloud2_cdr_with(
    endian: Endian,
    nanoseconds: u32,
    height: u32,
    width: u32,
    fields: &[(&str, u32, u8, u32)],
    point_step: u32,
    row_step: u32,
    data: &[u8],
) -> Vec<u8> {
    let mut cdr = Cdr::new(endian);
    cdr.i32(1_700_000_000);
    cdr.u32(nanoseconds);
    cdr.string("map");
    cdr.u32(height);
    cdr.u32(width);
    cdr.u32(u32::try_from(fields.len()).unwrap());

    for &(name, offset, datatype, count) in fields {
        cdr.string(name);
        cdr.u32(offset);
        cdr.u8(datatype);
        cdr.u32(count);
    }

    cdr.u8(matches!(endian, Endian::Big) as u8);
    cdr.u32(point_step);
    cdr.u32(row_step);
    cdr.u32(u32::try_from(data.len()).unwrap());
    cdr.bytes.extend_from_slice(data);
    cdr.u8(0); // is_dense
    cdr.bytes
}

fn standard_point_data(endian: Endian) -> Vec<u8> {
    let mut data = Vec::new();
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
        data.extend(float_bytes(x));
        data.extend(float_bytes(y));
        data.extend(float_bytes(z));
        data.extend(short_bytes(intensity));
        data.extend(short_bytes(ring));
    }
    data
}

const STANDARD_FIELDS: &[(&str, u32, u8, u32)] = &[
    ("x", 0, 7, 1),
    ("y", 4, 7, 1),
    ("z", 8, 7, 1),
    ("intensity", 12, 4, 1),
    ("ring", 14, 4, 1),
];

fn pointcloud2_cdr(endian: Endian, invalid_z_offset: bool) -> Vec<u8> {
    let mut fields = STANDARD_FIELDS.to_vec();
    if invalid_z_offset {
        fields[2].1 = 14;
    }
    pointcloud2_cdr_with(
        endian,
        123_456_789,
        1,
        2,
        &fields,
        16,
        32,
        &standard_point_data(endian),
    )
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

fn ply_header(encoding: &str) -> String {
    format!(
        "ply\n\
format {encoding} 1.0\n\
comment generated by pcx\n\
element vertex 2\n\
property char signed_byte\n\
property uchar unsigned_byte\n\
property short signed_short\n\
property ushort unsigned_short\n\
property int signed_int\n\
property uint unsigned_int\n\
property float x\n\
property double time\n\
end_header\n"
    )
}

fn binary_ply(endian: Endian) -> Vec<u8> {
    let encoding = match endian {
        Endian::Little => "binary_little_endian",
        Endian::Big => "binary_big_endian",
    };
    let mut output = ply_header(encoding).into_bytes();
    for (i8_value, u8_value, i16_value, u16_value, i32_value, u32_value, f32_value, f64_value) in [
        (-1_i8, 2_u8, -300_i16, 400_u16, -50_000_i32, 60_000_u32, 1.5_f32, -2.25_f64),
        (i8::MIN, u8::MAX, i16::MIN, u16::MAX, i32::MIN, u32::MAX, -0.0_f32, 3.25_f64),
    ] {
        output.push(i8_value as u8);
        output.push(u8_value);
        macro_rules! scalar {
            ($value:expr) => {
                output.extend(match endian {
                    Endian::Little => $value.to_le_bytes(),
                    Endian::Big => $value.to_be_bytes(),
                })
            };
        }
        scalar!(i16_value);
        scalar!(u16_value);
        scalar!(i32_value);
        scalar!(u32_value);
        scalar!(f32_value.to_bits());
        scalar!(f64_value.to_bits());
    }
    output
}

fn single_field_pcd(
    field_lines: &str,
    width: &str,
    height: &str,
    viewpoint: &str,
    points: &str,
    data: &str,
) -> String {
    format!(
        "# independently generated strict PCD reader fixture\n\
VERSION 0.7\n\
{field_lines}\n\
WIDTH {width}\n\
HEIGHT {height}\n\
VIEWPOINT {viewpoint}\n\
POINTS {points}\n\
DATA {data}\n"
    )
}

fn organized_unknown_fields_pcd() -> String {
    "# independently generated strict PCD reader fixture\n\
VERSION 0.7\n\
FIELDS descriptor quality flag\n\
SIZE 2 8 1\n\
TYPE I F U\n\
COUNT 2 1 1\n\
WIDTH 2\n\
HEIGHT 2\n\
VIEWPOINT 0.0 -0 0 1.0 0 0 0\n\
POINTS 4\n\
DATA ascii\n\
1 2 1.25 1\n\
3 4 -0 0\n\
5 6 -inf 1\n\
7 8 nan 0\n"
        .into()
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
    let mut organized_data = Vec::new();
    for row in [[1.0_f32, 2.0], [3.0, 4.0]] {
        for value in row {
            organized_data.extend(value.to_le_bytes());
        }
        organized_data.extend([0xaa; 4]);
    }
    write(
        root,
        "valid/pointcloud2-organized-row-padding.cdr",
        pointcloud2_cdr_with(
            Endian::Little,
            123_456_789,
            2,
            2,
            &[("x", 0, 7, 1)],
            4,
            12,
            &organized_data,
        ),
    );
    let mut varied_data = Vec::new();
    varied_data.extend([1_u8, 2]);
    varied_data.extend((-3_i8).to_le_bytes());
    varied_data.extend(4_u16.to_le_bytes());
    varied_data.extend((-5_i16).to_le_bytes());
    varied_data.extend(6_u32.to_le_bytes());
    varied_data.extend((-7_i32).to_le_bytes());
    for value in [8.0_f32, 9.0, 10.0] {
        varied_data.extend(value.to_le_bytes());
    }
    varied_data.extend(11.0_f64.to_le_bytes());
    write(
        root,
        "valid/pointcloud2-reordered-fields-and-count.cdr",
        pointcloud2_cdr_with(
            Endian::Little,
            123_456_789,
            1,
            1,
            &[
                ("returns", 0, 2, 2),
                ("i8", 2, 1, 1),
                ("u16", 3, 4, 1),
                ("i16", 5, 3, 1),
                ("u32", 7, 6, 1),
                ("i32", 11, 5, 1),
                ("normal", 15, 7, 3),
                ("time", 27, 8, 1),
            ],
            35,
            35,
            &varied_data,
        ),
    );
    write(root, "valid/pointcloud2-binary.pcd", binary_pcd());
    write(
        root,
        "valid/pointcloud2-ascii.pcd",
        format!(
            "{}1 -2.5 0 42 7\n-0 inf nan 65535 8\n",
            pcd_header(2, "ascii")
        ),
    );
    write(
        root,
        "valid/pcd-organized-unknown-fields-ascii.pcd",
        organized_unknown_fields_pcd(),
    );
    write(
        root,
        "valid/scalar-vertices-ascii.ply",
        format!(
            "{}-1 2 -300 400 -50000 60000 1.5 -2.25\n-128 255 -32768 65535 -2147483648 4294967295 -0 3.25\n",
            ply_header("ascii")
        ),
    );
    write(
        root,
        "valid/scalar-vertices-binary-little-endian.ply",
        binary_ply(Endian::Little),
    );
    write(
        root,
        "valid/scalar-vertices-binary-big-endian.ply",
        binary_ply(Endian::Big),
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
    for (name, fields, point_step) in [
        (
            "pointcloud2-field-names-must-be-unique.cdr",
            vec![("x", 0, 7, 1), ("x", 4, 7, 1)],
            8,
        ),
        (
            "pointcloud2-field-ranges-must-not-overlap.cdr",
            vec![("x", 0, 7, 1), ("intensity", 2, 4, 1)],
            4,
        ),
        (
            "pointcloud2-field-count-must-be-positive.cdr",
            vec![("x", 0, 7, 0)],
            4,
        ),
        (
            "pointcloud2-field-datatype-must-be-supported.cdr",
            vec![("x", 0, 9, 1)],
            4,
        ),
    ] {
        write(
            root,
            &format!("malformed/{name}"),
            pointcloud2_cdr_with(
                Endian::Little,
                123_456_789,
                1,
                1,
                &fields,
                point_step,
                point_step,
                &vec![0; point_step as usize],
            ),
        );
    }
    write(
        root,
        "malformed/pointcloud2-row-step-must-cover-row.cdr",
        pointcloud2_cdr_with(
            Endian::Little,
            123_456_789,
            1,
            2,
            &[("x", 0, 7, 1)],
            4,
            7,
            &[0; 7],
        ),
    );
    write(
        root,
        "malformed/pointcloud2-data-length-must-equal-height-times-row-step.cdr",
        pointcloud2_cdr_with(
            Endian::Little,
            123_456_789,
            2,
            2,
            &[("x", 0, 7, 1)],
            4,
            8,
            &[0; 15],
        ),
    );
    write(
        root,
        "malformed/pointcloud2-timestamp-nanoseconds-must-be-canonical.cdr",
        pointcloud2_cdr_with(
            Endian::Little,
            1_000_000_000,
            1,
            0,
            &[],
            0,
            0,
            &[],
        ),
    );
    write(
        root,
        "malformed/pointcloud2-height-must-be-positive.cdr",
        pointcloud2_cdr_with(
            Endian::Little,
            123_456_789,
            0,
            1,
            &[("x", 0, 7, 1)],
            4,
            4,
            &[],
        ),
    );
    write(
        root,
        "malformed/pointcloud2-point-step-must-be-positive.cdr",
        pointcloud2_cdr_with(
            Endian::Little,
            123_456_789,
            1,
            1,
            &[],
            0,
            0,
            &[],
        ),
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
    write(
        root,
        "malformed/pcd-directives-must-be-ordered.pcd",
        "VERSION 0.7\nSIZE 4\nFIELDS x\nTYPE F\nCOUNT 1\nWIDTH 1\nHEIGHT 1\nVIEWPOINT 0 0 0 1 0 0 0\nPOINTS 1\nDATA ascii\n1\n",
    );
    write(
        root,
        "malformed/pcd-field-vectors-must-align.pcd",
        single_field_pcd(
            "FIELDS x y\nSIZE 4\nTYPE F F\nCOUNT 1 1",
            "1", "1", "0 0 0 1 0 0 0", "1", "ascii",
        ),
    );
    write(
        root,
        "malformed/pcd-field-type-size-must-be-supported.pcd",
        single_field_pcd(
            "FIELDS clock\nSIZE 8\nTYPE U\nCOUNT 1",
            "1", "1", "0 0 0 1 0 0 0", "1", "ascii",
        ),
    );
    write(
        root,
        "malformed/pcd-field-count-must-be-positive.pcd",
        single_field_pcd(
            "FIELDS x\nSIZE 4\nTYPE F\nCOUNT 0",
            "1", "1", "0 0 0 1 0 0 0", "1", "ascii",
        ),
    );
    write(
        root,
        "malformed/pcd-field-names-must-be-unique.pcd",
        single_field_pcd(
            "FIELDS x x\nSIZE 4 4\nTYPE F F\nCOUNT 1 1",
            "1", "1", "0 0 0 1 0 0 0", "1", "ascii",
        ) + "1 2\n",
    );
    write(
        root,
        "malformed/pcd-dimensions-must-not-overflow.pcd",
        single_field_pcd(
            "FIELDS x\nSIZE 4\nTYPE F\nCOUNT 1",
            "18446744073709551615", "2",
            "0 0 0 1 0 0 0", "1", "ascii",
        ),
    );
    write(
        root,
        "malformed/pcd-height-must-be-positive.pcd",
        single_field_pcd(
            "FIELDS x\nSIZE 4\nTYPE F\nCOUNT 1",
            "1", "0", "0 0 0 1 0 0 0", "0", "ascii",
        ),
    );
    write(
        root,
        "malformed/pcd-viewpoint-must-be-preservable.pcd",
        single_field_pcd(
            "FIELDS x\nSIZE 4\nTYPE F\nCOUNT 1",
            "1", "1", "1 0 0 1 0 0 0", "1", "ascii",
        ),
    );
    write(
        root,
        "malformed/pcd-compressed-must-be-rejected.pcd",
        single_field_pcd(
            "FIELDS x\nSIZE 4\nTYPE F\nCOUNT 1",
            "1", "1", "0 0 0 1 0 0 0", "1", "binary_compressed",
        ),
    );
    write(
        root,
        "malformed/pcd-ascii-payload-must-be-complete.pcd",
        single_field_pcd(
            "FIELDS x\nSIZE 4\nTYPE F\nCOUNT 1",
            "2", "1", "0 0 0 1 0 0 0", "2", "ascii",
        ) + "1\n",
    );
    write(
        root,
        "malformed/pcd-ascii-payload-must-not-have-extra-values.pcd",
        single_field_pcd(
            "FIELDS x\nSIZE 4\nTYPE F\nCOUNT 1",
            "1", "1", "0 0 0 1 0 0 0", "1", "ascii",
        ) + "1 2\n",
    );
    let mut short_binary = single_field_pcd(
        "FIELDS x\nSIZE 4\nTYPE F\nCOUNT 1",
        "1", "1", "0 0 0 1 0 0 0", "1", "binary",
    ).into_bytes();
    short_binary.extend([0_u8; 3]);
    write(
        root,
        "malformed/pcd-binary-payload-must-be-exact.pcd",
        short_binary,
    );
    let mut long_binary = single_field_pcd(
        "FIELDS x\nSIZE 4\nTYPE F\nCOUNT 1",
        "1", "1", "0 0 0 1 0 0 0", "1", "binary",
    )
    .into_bytes();
    long_binary.extend(1.0_f32.to_le_bytes());
    long_binary.push(0);
    write(
        root,
        "malformed/pcd-binary-payload-must-not-have-extra-bytes.pcd",
        long_binary,
    );
    write(
        root,
        "malformed/ply-list-properties-are-unsupported.ply",
        "ply\nformat ascii 1.0\nelement vertex 1\nproperty list uchar float samples\nend_header\n0\n",
    );
    write(
        root,
        "malformed/ply-int64-properties-are-unsupported.ply",
        "ply\nformat ascii 1.0\nelement vertex 1\nproperty int64 timestamp\nend_header\n0\n",
    );
    write(
        root,
        "malformed/ply-format-endianness-must-be-known.ply",
        "ply\nformat binary_middle_endian 1.0\nelement vertex 1\nproperty float x\nend_header\n",
    );
    write(
        root,
        "malformed/ply-non-vertex-elements-are-lossy.ply",
        "ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nelement face 0\nend_header\n1\n",
    );
    let mut truncated_ply = binary_ply(Endian::Little);
    truncated_ply.pop();
    write(
        root,
        "malformed/ply-binary-payload-must-not-be-truncated.ply",
        truncated_ply,
    );
}
