use pcx_cli::{
    core::{
        ByteBound, Destination, FidelityLoss, FrameSelector, JobSpec, LossPolicy,
        PipelineMemoryRequirements, Planner, PointRepresentation, SourceSpec,
    },
    mcap::{self, SourceOptions},
    ops::{AxisAlignedCrop, CropBounds, CropPlan},
    pcd::{self, Encoding},
    ros2::pointcloud2,
};
use serde::Serialize;
use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

pub const POINT_COUNT: usize = 65_536;
pub const POINT_BYTES: usize = 16;
pub const EXPECTED_RETAINED_POINTS: usize = 16_384;
pub const BINARY_OUTPUT_BUDGET_BYTES: u64 = 262_343;
pub const ASCII_OUTPUT_BUDGET_BYTES: u64 = 334_790;
pub const PEAK_MANAGED_MEMORY_BUDGET_BYTES: u64 = 3_383_779;

const TOPIC: &str = "/lidar/points";
const MCAP_MAGIC: &[u8; 8] = b"\x89MCAP0\r\n";

pub struct Fixture {
    pub cdr: Arc<[u8]>,
    pub mcap_path: PathBuf,
    pub view: pcx_cli::core::point::PointView,
    pub crop: CropPlan,
    pub cropped: pcx_cli::core::point::PointBatch,
    pub report: BenchmarkReport,
}

impl Fixture {
    pub fn new() -> Self {
        let cdr: Arc<[u8]> = Arc::from(pointcloud2_cdr());
        let view = pointcloud2::decode(Arc::clone(&cdr)).expect("synthetic CDR must decode");
        let crop = AxisAlignedCrop::new(
            CropBounds::new([-64.0, -64.0, -1.0], [64.0, 64.0, 8.0])
                .expect("fixed crop bounds are valid"),
        )
        .plan(
            Arc::new(view.schema().clone()),
            view.layout().dimensions(),
            PointRepresentation::View,
            &LossPolicy::authorize([FidelityLoss::PointSelection]),
        )
        .expect("synthetic schema must satisfy the crop contract");
        let cropped = crop.execute_view(&view).expect("fixed crop must execute");

        let requirements = PipelineMemoryRequirements::for_operator_pipeline(
            &view,
            crop.pipeline(),
            ByteBound::bounded(0),
            ByteBound::bounded(0),
            ByteBound::bounded(0),
        )
        .expect("synthetic pipeline memory must be bounded");
        let job = JobSpec::extract(
            SourceSpec::file("synthetic.mcap").expect("fixed Source path is valid"),
            TOPIC,
            FrameSelector::Index(0),
            Destination::stdout(),
        )
        .expect("fixed benchmark job is valid");
        let memory = Planner::new()
            .plan(job, requirements, u64::MAX)
            .expect("synthetic pipeline must be plannable")
            .memory();

        let mut binary = CountingWriter::default();
        pcd::write(&mut binary, &cropped, Encoding::Binary)
            .expect("synthetic output must encode as binary PCD");
        let mut ascii = CountingWriter::default();
        pcd::write(&mut ascii, &cropped, Encoding::Ascii)
            .expect("synthetic output must encode as ASCII PCD");

        let mcap_path = temporary_mcap_path();
        write_mcap(&mcap_path, &cdr).expect("synthetic MCAP fixture must be writable");
        let mcap_size_bytes = fs::metadata(&mcap_path)
            .expect("synthetic MCAP fixture metadata must be readable")
            .len();

        let report = BenchmarkReport {
            schema_version: 1,
            architecture: std::env::consts::ARCH,
            fixture: FixtureReport {
                kind: "deterministic synthetic ROS 2 PointCloud2 in uncompressed MCAP",
                license: "MIT",
                point_count: POINT_COUNT,
                point_record_bytes: POINT_BYTES,
                cdr_size_bytes: cdr.len() as u64,
                mcap_size_bytes,
                retained_point_count: cropped.dimensions().point_count(),
            },
            measurements: MeasurementReport {
                binary_pcd_output_bytes: binary.bytes,
                ascii_pcd_output_bytes: ascii.bytes,
                declared_peak_managed_memory_bytes: memory.peak_bytes(),
            },
            regression_budgets: RegressionBudgetReport {
                binary_pcd_output_bytes: BINARY_OUTPUT_BUDGET_BYTES,
                ascii_pcd_output_bytes: ASCII_OUTPUT_BUDGET_BYTES,
                declared_peak_managed_memory_bytes: PEAK_MANAGED_MEMORY_BUDGET_BYTES,
            },
        };

        Self {
            cdr,
            mcap_path,
            view,
            crop,
            cropped,
            report,
        }
    }

    #[allow(dead_code)]
    pub fn write_report(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = File::create(path)?;
        serde_json::to_writer_pretty(file, &self.report)?;
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.mcap_path);
    }
}

#[derive(Debug, Serialize)]
pub struct BenchmarkReport {
    pub schema_version: u32,
    pub architecture: &'static str,
    pub fixture: FixtureReport,
    pub measurements: MeasurementReport,
    pub regression_budgets: RegressionBudgetReport,
}

#[derive(Debug, Serialize)]
pub struct FixtureReport {
    pub kind: &'static str,
    pub license: &'static str,
    pub point_count: usize,
    pub point_record_bytes: usize,
    pub cdr_size_bytes: u64,
    pub mcap_size_bytes: u64,
    pub retained_point_count: usize,
}

#[derive(Debug, Serialize)]
pub struct MeasurementReport {
    pub binary_pcd_output_bytes: u64,
    pub ascii_pcd_output_bytes: u64,
    pub declared_peak_managed_memory_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct RegressionBudgetReport {
    pub binary_pcd_output_bytes: u64,
    pub ascii_pcd_output_bytes: u64,
    pub declared_peak_managed_memory_bytes: u64,
}

#[derive(Default)]
pub struct CountingWriter {
    pub bytes: u64,
}

impl Write for CountingWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.bytes = self.bytes.saturating_add(buffer.len() as u64);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub fn probe(path: &Path) -> mcap::Info {
    mcap::inspect(
        File::open(path).expect("benchmark MCAP must open"),
        SourceOptions::default(),
    )
    .expect("benchmark MCAP must probe")
}

fn pointcloud2_cdr() -> Vec<u8> {
    let mut cdr = Cdr::new();
    cdr.i32(1_700_000_000);
    cdr.u32(123_456_789);
    cdr.string("pcx-benchmark");
    cdr.u32(1);
    cdr.u32(POINT_COUNT as u32);
    cdr.u32(5);
    for (name, offset, datatype) in [
        ("x", 0, 7),
        ("y", 4, 7),
        ("z", 8, 7),
        ("intensity", 12, 4),
        ("ring", 14, 4),
    ] {
        cdr.string(name);
        cdr.u32(offset);
        cdr.u8(datatype);
        cdr.u32(1);
    }
    cdr.u8(0);
    cdr.u32(POINT_BYTES as u32);
    cdr.u32((POINT_COUNT * POINT_BYTES) as u32);
    cdr.u32((POINT_COUNT * POINT_BYTES) as u32);
    for point in 0..POINT_COUNT {
        let x = (point % 256) as f32 - 128.0;
        let y = ((point / 256) % 256) as f32 - 128.0;
        let z = (point % 32) as f32 * 0.25;
        cdr.bytes.extend(x.to_le_bytes());
        cdr.bytes.extend(y.to_le_bytes());
        cdr.bytes.extend(z.to_le_bytes());
        cdr.bytes.extend((point as u16).to_le_bytes());
        cdr.bytes.extend(((point / 256) as u16).to_le_bytes());
    }
    cdr.u8(1);
    cdr.bytes
}

struct Cdr {
    bytes: Vec<u8>,
}

impl Cdr {
    fn new() -> Self {
        Self {
            bytes: vec![0, 1, 0, 0],
        }
    }

    fn align(&mut self, alignment: usize) {
        while !self.bytes.len().is_multiple_of(alignment) {
            self.bytes.push(0);
        }
    }

    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn u32(&mut self, value: u32) {
        self.align(4);
        self.bytes.extend(value.to_le_bytes());
    }

    fn i32(&mut self, value: i32) {
        self.align(4);
        self.bytes.extend(value.to_le_bytes());
    }

    fn string(&mut self, value: &str) {
        self.u32((value.len() + 1) as u32);
        self.bytes.extend(value.as_bytes());
        self.bytes.push(0);
    }
}

fn temporary_mcap_path() -> PathBuf {
    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "pcx-performance-{}-{}-{}.mcap",
        std::process::id(),
        std::env::consts::ARCH,
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
    ))
}

fn mcap_string(value: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(4 + value.len());
    bytes.extend((value.len() as u32).to_le_bytes());
    bytes.extend(value.as_bytes());
    bytes
}

fn record(output: &mut File, opcode: u8, body: &[u8]) -> io::Result<()> {
    output.write_all(&[opcode])?;
    output.write_all(&(body.len() as u64).to_le_bytes())?;
    output.write_all(body)
}

fn write_mcap(path: &Path, payload: &[u8]) -> io::Result<()> {
    let mut output = File::create(path)?;
    output.write_all(MCAP_MAGIC)?;

    let mut header = mcap_string("ros2");
    header.extend(mcap_string("pcx-benchmark/1.0.0"));
    record(&mut output, 0x01, &header)?;

    let mut schema = Vec::from(1_u16.to_le_bytes());
    schema.extend(mcap_string("sensor_msgs/msg/PointCloud2"));
    schema.extend(mcap_string("ros2msg"));
    schema.extend(0_u32.to_le_bytes());
    record(&mut output, 0x03, &schema)?;

    let mut channel = Vec::from(1_u16.to_le_bytes());
    channel.extend(1_u16.to_le_bytes());
    channel.extend(mcap_string(TOPIC));
    channel.extend(mcap_string("cdr"));
    channel.extend(0_u32.to_le_bytes());
    record(&mut output, 0x04, &channel)?;

    let message_body_len = 2 + 4 + 8 + 8 + payload.len();
    output.write_all(&[0x05])?;
    output.write_all(&(message_body_len as u64).to_le_bytes())?;
    output.write_all(&1_u16.to_le_bytes())?;
    output.write_all(&0_u32.to_le_bytes())?;
    output.write_all(&1_700_000_000_123_456_789_u64.to_le_bytes())?;
    output.write_all(&1_700_000_000_123_456_789_u64.to_le_bytes())?;
    output.write_all(payload)?;

    record(&mut output, 0x0f, &0_u32.to_le_bytes())?;
    let mut footer = Vec::from(0_u64.to_le_bytes());
    footer.extend(0_u64.to_le_bytes());
    footer.extend(0_u32.to_le_bytes());
    record(&mut output, 0x02, &footer)?;
    output.write_all(MCAP_MAGIC)
}
