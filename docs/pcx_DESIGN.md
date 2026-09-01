# pcx Design Document

> Working title: `pcx`  
> Status: Draft / implementation-oriented  
> Primary goal: A single-binary, lightweight CUI/TUI tool for inspecting, extracting, reducing, converting, compressing, rendering, and transferring point-cloud and sensor-log data, especially on edge devices and over SSH.

---

## 1. Product Vision

`pcx` is a **single-binary toolbox for point-cloud and sensor-log investigation on edge devices**.

It is not intended to replace GUI viewers such as RViz, Foxglove, Rerun, or CloudCompare.

Its primary use case is:

> SSH into an edge device, quickly determine whether point-cloud data is being recorded correctly, reduce the data locally, and transfer only the useful subset to a workstation or object storage.

Typical workflow:

```text
sensor / recorder
      |
      v
large MCAP / rosbag / PCAP / PCD / LAS
      |
      |  pcx inspect
      |  pcx view
      |  pcx extract
      |  pcx reduce
      |  pcx compress
      v
small diagnostic artifact
      |
      +--> stdout / local file
      +--> SSH / SCP workflow
      +--> S3
```

The core concept is:

> **Compute where the sensor data lives.**

Avoid transferring multi-GB recordings before knowing which data is needed.

---

## 2. Design Principles

### 2.1 Single binary

The default distribution must be one executable.

Target properties:

- no ROS runtime requirement
- no Python runtime
- no Qt
- no PDAL dependency
- no OpenGL requirement for basic rendering
- no dynamic plugin requirement
- minimal system-library dependencies
- suitable for copying with `scp`

Example:

```bash
scp pcx robot:/tmp/
ssh robot '/tmp/pcx info /data/run.mcap'
```

### 2.2 Streaming first

Large recordings must be processed without loading the entire dataset into memory.

Preferred execution model:

```text
Source
  -> Decode
  -> Select
  -> Transform / Reduce
  -> Encode
  -> Sink
```

Processing should use bounded-memory batches/chunks.

### 2.3 CLI is a frontend, not the architecture

Command parsing must be separated from processing logic.

All commands should compile into a common internal `JobSpec`.

```text
CLI / TUI
   |
   v
JobSpec
   |
   v
Planner
   |
   v
Executor
```

This allows future reuse from:

- Rust API
- FFI
- tests
- one-shot remote execution

No long-running service/daemon is in scope for the initial design.

### 2.4 Preserve semantics where useful

Converting an MCAP point-cloud topic directly to PCD is useful for inspection, but may discard:

- timestamp
- frame_id
- TF
- schema
- topic metadata

Therefore the tool should support both:

1. extracting raw point-cloud frames into point-cloud file formats
2. producing a reduced sensor-log container while preserving temporal metadata

Example:

```bash
pcx extract run.mcap \
  --topic /lidar/points \
  --include-tf \
  -o lidar-only.mcap
```

### 2.5 Reduction before compression

Compression is treated as multiple independent stages:

```text
semantic reduction
  -> representation reduction
  -> codec compression
  -> transport
```

Semantic reduction normally provides the largest benefit.

---

# 3. Target Users

Primary:

- robotics engineers
- autonomous mobile robot developers
- autonomous-driving developers
- field robotics engineers
- LiDAR engineers
- embedded / edge software engineers
- sensor integration engineers
- ROS users diagnosing remote machines

Secondary:

- industrial 3D measurement
- drone mapping
- surveying
- point-cloud researchers

The tool optimizes for engineers operating machines such as:

- Jetson
- x86 industrial PCs
- ARM64 Linux boards
- Yocto Linux devices
- Ubuntu-based robots
- headless servers

---

# 4. Main User Stories

## 4.1 Check whether LiDAR data exists

```bash
ssh robot
pcx info /data/latest.mcap
```

Expected output:

```text
MCAP
duration: 00:18:42.318
size: 24.1 GiB

channels:
  /camera/front       sensor_msgs/Image
  /lidar/points       sensor_msgs/PointCloud2
  /imu/data           sensor_msgs/Imu
  /tf                 tf2_msgs/TFMessage

/lidar/points
  frames: 11,214
  rate: 9.99 Hz
  point fields: x y z intensity ring
```

## 4.2 Inspect one frame over SSH

```bash
pcx view latest.mcap \
  --topic /lidar/points \
  --at 83.2s
```

The renderer chooses:

```text
Kitty
 -> Sixel
 -> Unicode
```

depending on terminal capabilities.

The purpose is not a full GUI.

The user only needs to answer questions such as:

- are points arriving?
- is the shape plausible?
- is the coordinate frame obviously broken?
- is most data NaN/zero?
- is the sensor upside down?
- did the number of points collapse?

## 4.3 Transfer only a useful subset

```bash
ssh robot \
  'pcx extract /data/run.mcap \
      --topic /lidar/points \
      --from 80s \
      --to 90s \
      --voxel 0.05 \
      --format laz \
      -o -' \
  > lidar.laz
```

No temporary local workstation copy of the source recording is required.

## 4.4 Reduce data before S3 upload

```bash
pcx upload run.mcap \
  --topic /lidar/points \
  --from 120s \
  --duration 30s \
  --voxel 0.05 \
  --format mcap \
  --compression zstd \
  s3://robot-debug/robot42/run-120-150.mcap
```

Execution:

```text
MCAP reader
  -> channel selection
  -> time selection
  -> PointCloud2 decode
  -> voxel reduction
  -> MCAP encode
  -> zstd
  -> multipart S3 upload
```

Prefer no intermediate file.

## 4.5 Extract a point-cloud frame

```bash
pcx extract run.mcap \
  --topic /lidar/points \
  --frame 1024 \
  -o frame.pcd
```

## 4.6 Convert point-cloud formats

```bash
pcx convert scan.pcd scan.laz
pcx convert scan.las scan.ply
```

## 4.7 Merge point clouds

```bash
pcx merge a.pcd b.pcd c.pcd -o merged.pcd
```

## 4.8 Inspect raw LiDAR capture

Future:

```bash
pcx info lidar.pcap
pcx extract lidar.pcap --frame 182 -o frame.pcd
pcx view lidar.pcap --frame 182
```

---

# 5. Scope

## 5.1 MVP

Input:

- MCAP
- ROS 2 `sensor_msgs/PointCloud2` inside MCAP
- PCD
- PLY
- LAS
- LAZ

Output:

- MCAP
- PCD
- PLY
- LAS
- LAZ
- stdout

Operations:

- info
- topics/list
- extract
- convert
- merge
- crop
- voxel downsample
- field selection
- basic statistics
- compress/decompress
- terminal snapshot/view
- S3 upload
- output to stdout

Transport/auth:

- local file
- stdout
- S3
- AWS normal credential chain
- AWS IoT Credentials Provider + Role Alias

## 5.2 Phase 2

Input:

- ROS 2 SQLite3 bag (`.db3`)
- ROS 1 bag
- PCAP
- COPC
- E57
- Foxglove PointCloud schema
- TF semantic processing

Operations:

- temporal merge
- split
- advanced diagnostics
- diff
- transform using TF
- lossy coordinate quantization

## 5.3 Non-goals

Do not attempt to initially implement:

- full RViz replacement
- full CloudCompare replacement
- surface reconstruction
- meshing
- SLAM
- ICP-heavy workflows
- advanced segmentation
- ML inference
- complete PDAL filter compatibility
- daemon/service mode
- fleet-management server

---

# 6. CLI Design

Executable:

```text
pcx
```

Commands should be predictable and composable.

## 6.1 General

```bash
pcx info <SOURCE>
pcx topics <SOURCE>
pcx extract <SOURCE> [OPTIONS]
pcx convert <SOURCE> <OUTPUT>
pcx merge <SOURCE>... -o <OUTPUT>
pcx view <SOURCE> [OPTIONS]
pcx stats <SOURCE> [OPTIONS]
pcx upload <SOURCE> [OPTIONS] <DESTINATION>
pcx term
```

Every machine-readable command should support:

```bash
--json
```

Diagnostics go to stderr.

Data goes to stdout when `-o -` is specified.

This rule is important for shell composition.

---

# 7. Source Abstraction

A source is any readable sensor/point-cloud dataset.

```rust
trait Source {
    fn metadata(&mut self) -> Result<SourceMetadata>;

    fn streams(&mut self) -> Result<Vec<StreamDescriptor>>;

    fn open(
        &mut self,
        request: ReadRequest,
    ) -> Result<Box<dyn PointStream>>;
}
```

Candidate source implementations:

```text
McapSource
PcdSource
PlySource
LasSource
LazSource

future:
Rosbag2Source
Rosbag1Source
PcapSource
CopcSource
E57Source
```

`Source` should expose enough capability metadata for planning.

Example:

```rust
struct SourceCapabilities {
    seekable: bool,
    temporal: bool,
    multi_stream: bool,
    random_frame_access: bool,
    spatial_index: bool,
}
```

---

# 8. Internal Point-Cloud Model

Do not use a fixed `PointXYZRGB` representation.

Real sensor clouds commonly contain:

```text
x
y
z
intensity
ring
return_number
timestamp
rgb
normal
classification
custom fields
```

The internal representation should be schema-driven.

Conceptually:

```rust
struct PointSchema {
    fields: Vec<PointField>,
}

struct PointField {
    name: String,
    data_type: DataType,
    semantic: Option<Semantic>,
}

enum DataType {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}
```

A point batch:

```rust
struct PointBatch {
    schema: Arc<PointSchema>,

    columns: ColumnStorage,

    len: usize,

    timestamp: Option<TimeRange>,
    frame_id: Option<String>,
    metadata: BatchMetadata,
}
```

Prefer a columnar/SoA-friendly model internally.

Reasons:

- field removal becomes cheap
- statistics can scan one column
- SIMD-friendly
- compression-friendly
- PointCloud2 maps naturally
- arbitrary attributes remain possible

Do not require every operator to understand every field.

---

# 9. Point Stream

Main processing unit:

```rust
trait PointStream {
    fn schema(&self) -> &PointSchema;

    fn next_batch(
        &mut self,
    ) -> Result<Option<PointBatch>>;
}
```

For temporal containers, preserve frame boundaries when necessary.

A related type may be required:

```rust
struct PointFrame {
    timestamp: Timestamp,
    frame_id: Option<String>,
    points: PointBatch,
}
```

Recommended architecture:

```text
PointEvent
  |
  +-- PointFrame
  +-- Transform
  +-- Metadata
```

This prevents MCAP/ROS semantics from being forced into static point-cloud files.

For the MVP, it is acceptable to keep:

```text
StaticPointStream
TemporalPointStream
```

separate if that substantially simplifies implementation.

---

# 10. Processing Pipeline

Pipeline:

```text
Source
  -> Decoder
  -> Selector
  -> Operator*
  -> Encoder
  -> Sink
```

Example:

```text
run.mcap
  -> MCAP decode
  -> /lidar/points
  -> 80s..90s
  -> field(x,y,z,intensity)
  -> voxel(0.05)
  -> LAZ
  -> stdout
```

Operators should implement a common interface where practical:

```rust
trait Operator {
    fn process(
        &mut self,
        batch: PointBatch,
    ) -> Result<Vec<PointBatch>>;

    fn finish(
        &mut self,
    ) -> Result<Vec<PointBatch>>;
}
```

Some operators may require frame-aware APIs.

---

# 11. Core Operators

## 11.1 SelectFields

```bash
--fields x,y,z,intensity
```

Used to remove unused attributes before transfer.

## 11.2 Crop

```bash
--bounds '-10,-10,-2:10,10,5'
```

Or explicit:

```bash
pcx crop input.pcd \
  --min -10,-10,-2 \
  --max 10,10,5
```

## 11.3 Voxel

```bash
--voxel 0.05
```

Initial implementation can select one representative point per voxel.

Later policies:

```text
first
centroid
nearest-center
highest-intensity
```

## 11.4 Time selection

```bash
--from 80s
--to 90s

--at 83.2s
--duration 10s
```

## 11.5 Topic/channel selection

```bash
--topic /lidar/points
```

Multiple:

```bash
--topic /lidar/points \
--topic /tf \
--topic /tf_static
```

Convenience:

```bash
--include-tf
```

## 11.6 Merge

Static clouds:

```text
cloud A
cloud B
cloud C
  ->
merged cloud
```

Temporal logs should initially use a different semantic operation rather than silently flattening timestamps.

---

# 12. Reduction Model

`pcx` should clearly distinguish the following.

## 12.1 Semantic reduction

Examples:

```text
topic selection
time slicing
field selection
ROI crop
voxel sampling
frame sampling
```

## 12.2 Representation reduction

Potential future options:

```text
F64 -> F32
coordinate quantization
intensity quantization
delta encoding
```

These may be lossy and must require explicit user choice.

Example:

```bash
--quantize xyz=1mm
```

## 12.3 Codec compression

Examples:

```text
zstd
lz4
LAZ
container-native compression
```

The CLI should not call all three layers simply "compression".

---

# 13. Encoder / Writer Model

```rust
trait Encoder {
    fn capabilities(&self) -> EncoderCapabilities;

    fn encode(
        &mut self,
        event: PointEvent,
        sink: &mut dyn Sink,
    ) -> Result<()>;

    fn finish(
        &mut self,
        sink: &mut dyn Sink,
    ) -> Result<()>;
}
```

Capabilities:

```rust
struct EncoderCapabilities {
    streaming: bool,
    requires_seek: bool,
    requires_total_point_count: bool,
    preserves_timestamps: bool,
    preserves_frames: bool,
    preserves_arbitrary_fields: bool,
}
```

The planner must use these capabilities.

---

# 14. Sink Model

```rust
trait Sink {
    fn write(&mut self, bytes: &[u8]) -> Result<()>;
    fn finish(&mut self) -> Result<()>;
}
```

Initial sinks:

```text
FileSink
StdoutSink
S3Sink
```

Future:

```text
SSH sink
HTTP sink
custom cloud storage
```

A sink must not know point-cloud semantics.

It only receives encoded bytes.

---

# 15. JobSpec

All CLI operations compile into a `JobSpec`.

Example conceptual model:

```rust
struct JobSpec {
    source: SourceSpec,

    selection: SelectionSpec,

    operators: Vec<OperatorSpec>,

    output: OutputSpec,

    sink: SinkSpec,
}
```

Example:

```rust
JobSpec {
    source: SourceSpec::File("run.mcap"),

    selection: SelectionSpec {
        streams: vec!["/lidar/points"],
        time_range: Some(TimeRange::new(80.0, 90.0)),
    },

    operators: vec![
        OperatorSpec::SelectFields(
            vec!["x", "y", "z", "intensity"]
        ),

        OperatorSpec::Voxel {
            size: 0.05,
        },
    ],

    output: OutputSpec {
        format: Format::Laz,
        compression: Compression::Default,
    },

    sink: SinkSpec::Stdout,
}
```

`JobSpec` is the stable logical API.

CLI syntax is allowed to evolve independently.

---

# 16. Planner

Before execution, create an execution plan.

Responsibilities:

- validate format compatibility
- determine one-pass vs multi-pass processing
- determine whether seeking is required
- determine whether temporary spool storage is necessary
- estimate memory usage
- choose batch size
- choose S3 multipart strategy
- reject impossible pipelines

Example:

```bash
pcx plan run.mcap \
  --topic /lidar/points \
  --voxel 0.05 \
  --format laz \
  -o -
```

Possible output:

```text
SOURCE
  format: MCAP
  seekable: yes
  size: 28.4 GiB

SELECT
  channel: /lidar/points

OPERATORS
  voxel: 0.05 m

OUTPUT
  format: LAZ

EXECUTION
  passes: 1
  streaming: yes
  temporary storage: none
  estimated peak memory: 92 MiB
```

`pcx plan` may be a later command, but planner support should exist internally from the beginning.

---

# 17. MCAP Support

MCAP is a first-class container.

MVP semantic decoders:

```text
ROS 2 sensor_msgs/msg/PointCloud2
```

Later:

```text
tf2_msgs/msg/TFMessage
Foxglove PointCloud
```

Required MCAP operations:

```text
list schemas
list channels
show statistics
time range selection
channel selection
extract PointCloud2
write reduced MCAP
preserve schema/channel metadata
```

Example:

```bash
pcx topics run.mcap
```

Output:

```text
CHANNEL                  SCHEMA
/lidar/points            sensor_msgs/msg/PointCloud2
/tf                      tf2_msgs/msg/TFMessage
/tf_static               tf2_msgs/msg/TFMessage
/camera/front/image      sensor_msgs/msg/Image
```

---

# 18. PointCloud2 Decoder

Decoder must correctly handle:

```text
height
width
fields
is_bigendian
point_step
row_step
data
is_dense
```

Supported field datatypes should initially include all ROS PointField primitive types needed by typical clouds.

The decoder must not assume:

```text
x,y,z are first
point_step == 12
little endian
unorganized cloud
```

Malformed input should produce useful diagnostics.

Example:

```text
error: PointCloud2 field `z` exceeds point_step
channel: /lidar/points
timestamp: 83.201488s
field offset: 12
field size: 4
point_step: 12
```

---

# 19. Lightweight Diagnostics

`info` and `stats` should detect obvious data problems.

Example checks:

```text
point count
NaN / Inf ratio
XYZ min/max
bounding box
all-zero coordinates
field presence
frame rate
timestamp gaps
frame_id
point count variation
intensity range
```

Example:

```bash
pcx stats run.mcap --topic /lidar/points
```

```text
frames: 1240
rate:
  mean: 9.98 Hz
  min:  7.12 Hz

points/frame:
  mean: 128421
  min:   31204
  max:  129183

xyz:
  nan: 0.18%
  bbox:
    x: -14.2 .. 42.1
    y: -21.0 .. 23.8
    z: -3.1  .. 8.2

intensity:
  min: 0
  p50: 1824
  p99: 49102
  max: 65535
```

A dedicated `doctor` command may be added later.

---

# 20. Terminal Rendering

Rendering is diagnostic, not a GUI replacement.

Command:

```bash
pcx view <SOURCE>
```

Examples:

```bash
pcx view scan.pcd

pcx view run.mcap \
  --topic /lidar/points \
  --at 83.2s
```

Renderer backend order:

```text
Kitty Graphics Protocol
Sixel
Unicode fallback
```

Optional:

```text
iTerm2
```

Renderer architecture:

```text
PointFrame
   |
   v
Camera transform
   |
   v
Point splatting
   |
   v
Depth buffer
   |
   v
RGB framebuffer
   |
   +--> Kitty
   +--> Sixel
   +--> Unicode
```

Basic CPU rendering only is required.

Do not require a GPU.

---

# 21. Rendering Controls

Minimal interactive controls:

```text
drag / arrows     orbit
shift+drag        pan
wheel / +/-       zoom
r                 reset camera
c                 cycle color mode
q                 quit
```

Color modes:

```text
RGB
intensity
height (Z)
distance
single-color
```

For temporal input:

```text
space             play/pause
left/right        previous/next frame
[ / ]             seek backward/forward
```

Do not attempt advanced GUI widgets.

---

# 22. Terminal Capability Detection

Command:

```bash
pcx term
```

Example:

```text
terminal:
  TERM: xterm-kitty
  ssh: yes
  tmux: 3.6

graphics:
  kitty: supported
  sixel: unavailable
  unicode: supported

selected renderer:
  kitty
```

Must gracefully handle:

- SSH
- tmux
- unsupported terminals
- non-interactive stdout

Never emit terminal escape graphics when stdout is redirected unless explicitly requested.

---

# 23. S3 Upload

S3 upload is a one-shot CLI operation.

No background service is required.

Example:

```bash
pcx upload run.mcap \
  --topic /lidar/points \
  --from 120s \
  --duration 30s \
  --voxel 0.05 \
  --format mcap \
  --compression zstd \
  s3://robot-debug/robot42/sample.mcap
```

Pipeline:

```text
Source
  -> processing pipeline
  -> encoder
  -> multipart chunker
  -> S3
```

Prefer streaming upload.

Do not require a complete local output file before upload.

---

# 24. AWS Credential Model

Support at least two credential modes.

## 24.1 Standard AWS credential chain

Useful for developer PCs and EC2-like environments.

Examples:

```text
environment variables
AWS profile
web identity
instance/container credentials
```

Use the official AWS SDK credential provider chain where practical.

## 24.2 AWS IoT Credentials Provider / Role Alias

Intended for edge devices.

Inputs:

```text
credentials endpoint
role alias
thing name
client certificate
private key
root CA
```

Conceptual flow:

```text
device certificate
      |
      | mTLS
      v
AWS IoT Credentials Provider
      |
      | Role Alias
      v
temporary AWS credentials
      |
      v
S3 multipart upload
```

CLI configuration example:

```bash
pcx upload run.mcap \
  ... \
  s3://robot-debug/robot42/sample.mcap \
  --aws-auth iot-role-alias \
  --iot-role-alias RobotLogUploader \
  --iot-credentials-endpoint ... \
  --iot-thing-name robot42 \
  --cert /etc/device/cert.pem \
  --key /etc/device/key.pem
```

Secrets must never be printed.

Avoid logging temporary credentials.

---

# 25. S3 Multipart Upload

Large or streaming outputs should use multipart upload.

Requirements:

- configurable part size
- bounded upload concurrency
- retry per part
- checksums where practical
- abort multipart upload on unrecoverable failure
- upload progress

Example options:

```bash
--s3-part-size 16MiB
--s3-concurrency 2
```

Defaults should be conservative for edge devices.

Do not aggressively consume RAM or bandwidth.

---

# 26. Memory and Resource Constraints

Default behavior should be suitable for embedded/edge machines.

Desired baseline:

```text
idle RSS: as small as practical
processing memory: bounded
CPU threads: limited by default
no full-file buffering
```

Global options:

```bash
--threads N
--memory-limit 256MiB
```

Operators should expose whether their memory usage depends on:

```text
batch size
frame size
global dataset
```

Global-dataset operators should be avoided in the MVP.

---

# 27. Error Handling

Error messages should be actionable.

Bad:

```text
decode failed
```

Good:

```text
failed to decode PointCloud2

source:
  /data/run.mcap

channel:
  /lidar/points

timestamp:
  83.201488s

reason:
  field `z` extends beyond point_step

field:
  offset=12
  size=4

point_step:
  12
```

Exit codes should distinguish at minimum:

```text
0 success
1 general failure
2 invalid CLI arguments
3 unsupported input/format
4 malformed data
5 IO/network failure
6 authentication/authorization failure
```

Exact numeric contract can be finalized later.

---

# 28. Logging

Use stderr for logs.

Global verbosity:

```bash
-q
-v
-vv
-vvv
```

Machine-readable output:

```bash
--json
```

When output bytes are sent to stdout:

```bash
-o -
```

stderr remains safe for logs/progress.

This enables:

```bash
ssh robot 'pcx extract ... -o -' > cloud.laz
```

without corrupting the binary stream.

---

# 29. Configuration

CLI options remain primary.

Optional config locations:

```text
./pcx.toml
~/.config/pcx/config.toml
/etc/pcx/config.toml
```

Do not make configuration mandatory.

Possible content:

```toml
[render]
preferred = "auto"

[resources]
threads = 2
memory_limit = "256MiB"

[aws.iot]
credentials_endpoint = "..."
role_alias = "RobotLogUploader"
thing_name = "robot42"
cert = "/etc/device/cert.pem"
key = "/etc/device/key.pem"
```

Sensitive private-key material should be referenced by path, not embedded.

---

# 30. Rust Workspace Layout

Suggested initial workspace:

```text
pcx/
├── Cargo.toml
├── crates/
│   ├── pcx-cli/
│   │
│   ├── pcx-core/
│   │   ├── job.rs
│   │   ├── planner.rs
│   │   ├── executor.rs
│   │   ├── schema.rs
│   │   ├── batch.rs
│   │   └── stream.rs
│   │
│   ├── pcx-format-mcap/
│   ├── pcx-format-pcd/
│   ├── pcx-format-ply/
│   ├── pcx-format-las/
│   │
│   ├── pcx-ops/
│   │   ├── crop.rs
│   │   ├── fields.rs
│   │   ├── voxel.rs
│   │   └── stats.rs
│   │
│   ├── pcx-render/
│   │   ├── raster.rs
│   │   ├── kitty.rs
│   │   ├── sixel.rs
│   │   └── unicode.rs
│   │
│   └── pcx-s3/
│       ├── sink.rs
│       ├── multipart.rs
│       └── credentials.rs
│
├── tests/
├── fixtures/
└── docs/
```

If this creates too many crates during early development, begin with fewer crates:

```text
pcx-cli
pcx-core
pcx-render
```

and split once boundaries stabilize.

Architecture matters more than crate count.

---

# 31. Dependency Policy

Dependency policy is a product feature.

Before adding a dependency, consider:

1. Does it introduce native system dependencies?
2. Does it significantly increase binary size?
3. Does it complicate cross compilation?
4. Does it require OpenSSL dynamically?
5. Does it require CMake?
6. Does it require ROS/PCL/PDAL?
7. Is there a pure-Rust alternative?

Prefer:

- pure Rust
- rustls
- optional features
- compile-time feature gating

Avoid making cloud functionality force large dependencies on users who only need local point-cloud processing.

Suggested features:

```toml
[features]
default = ["mcap", "pcd", "ply", "las"]

mcap = [...]
las = [...]
render-kitty = [...]
render-sixel = [...]
s3 = [...]
aws-iot-auth = ["s3", ...]
```

Potentially distribute:

```text
pcx
pcx-full
```

later if binary-size pressure becomes significant, while retaining one-binary runtime behavior.

---

# 32. Cross Compilation Targets

High priority:

```text
x86_64-unknown-linux-gnu
aarch64-unknown-linux-gnu
```

Desirable:

```text
x86_64-unknown-linux-musl
aarch64-unknown-linux-musl
```

The musl/static story should be investigated early.

Cross-compilation pain is directly opposed to the product's value proposition.

---

# 33. Performance Strategy

Optimize for:

1. bounded memory
2. sequential IO
3. avoiding copies
4. avoiding unnecessary decoding
5. parallelism only where useful

Important optimization:

If the user requests:

```bash
pcx extract run.mcap \
  --topic /lidar/points \
  --from 10s \
  --to 20s \
  -o reduced.mcap
```

and no point-level operator is requested, avoid decoding PointCloud2 if possible.

Use:

```text
container-level channel/time filtering
```

instead.

Only enter point-cloud semantic decoding when necessary.

This distinction is important.

---

# 34. Zero-Copy / Low-Copy Direction

Not required for first implementation, but data paths should avoid making zero-copy impossible.

Example:

```text
MCAP message bytes
   |
   +--> direct container copy
   |
   +--> PointCloud2 view
           |
           +--> borrowed typed columns where alignment/endian permits
           |
           +--> materialized PointBatch when transformation is required
```

Avoid forcing every source into a fully-owned normalized vector immediately.

---

# 35. Execution Optimization Levels

The planner should conceptually distinguish:

## Container passthrough

Example:

```text
MCAP
 -> select channel/time
 -> MCAP
```

No point decode.

## Semantic streaming

Example:

```text
MCAP PointCloud2
 -> voxel
 -> MCAP PointCloud2
```

Decode and re-encode each frame.

## Static point processing

Example:

```text
PCD
 -> crop
 -> voxel
 -> LAZ
```

This distinction allows performance without exposing complexity to the CLI user.

---

# 36. Security

The tool may run on production robots.

Requirements:

- no implicit network listeners
- no daemon
- no HTTP server for rendering
- no browser requirement
- SSH remains the normal remote-access boundary
- private keys are never logged
- temporary AWS credentials are never logged
- S3 destinations must be explicit
- network functionality should be feature-gated if practical

`pcx view` must render locally in the terminal rather than starting a web service.

---

# 37. UX Philosophy

The tool should feel closer to:

```text
ffmpeg
ffprobe
jq
ripgrep
busybox
timg
```

than to a desktop point-cloud suite.

Properties:

- predictable subcommands
- stdout-friendly
- stderr diagnostics
- scripting-friendly
- useful without configuration
- command completion
- concise default output
- detailed output on request

Example investigation:

```bash
pcx info run.mcap
pcx topics run.mcap
pcx stats run.mcap --topic /lidar/points
pcx view run.mcap --topic /lidar/points --at 43s
pcx extract run.mcap --topic /lidar/points --around 43s --duration 5s --voxel 5cm -o sample.laz
```

---

# 38. Proposed Command Semantics

## `pcx info`

Dataset/container-level metadata.

## `pcx topics`

List streams/channels/topics.

Alias may later become:

```text
pcx ls
```

but one canonical command should be selected.

## `pcx stats`

Point-level statistics.

## `pcx view`

Terminal visualization.

## `pcx extract`

Select a subset and write it elsewhere.

May include:

```text
topic
time
frame
fields
crop
voxel
```

## `pcx convert`

Format-only conversion with optional common transformations.

## `pcx merge`

Merge compatible datasets.

## `pcx upload`

Process and stream output to S3.

## `pcx term`

Terminal diagnostics.

---

# 39. CLI Grammar Consistency

Common selectors should be shared across commands:

```text
--topic
--from
--to
--at
--duration
--frame
--fields
```

Common point operators:

```text
--crop
--voxel
```

Common output options:

```text
-o
--format
--compression
```

Avoid command-specific aliases for identical concepts.

---

# 40. Format Detection

Prefer automatic detection using:

1. magic/header
2. content structure
3. extension as fallback

Do not rely exclusively on extensions.

Example:

```bash
cat data | pcx info -
```

should work for stream-detectable formats where practical.

---

# 41. Compression Commands

Possible explicit commands:

```bash
pcx compress input.pcd -o output.pcd.zst
pcx decompress input.pcd.zst -o output.pcd
```

However, format-native compression should normally be expressed through output format:

```bash
pcx convert scan.pcd scan.laz
```

or:

```bash
pcx extract run.mcap \
  --compression zstd \
  -o reduced.mcap
```

Do not invent a custom point-cloud compression format in the MVP.

---

# 42. AWS Upload Examples

Developer environment:

```bash
AWS_PROFILE=dev \
pcx upload run.mcap \
  --topic /lidar/points \
  --voxel 5cm \
  s3://debug-bucket/sample.mcap
```

Edge device:

```bash
pcx upload run.mcap \
  --topic /lidar/points \
  --from 120s \
  --duration 10s \
  --voxel 5cm \
  s3://robot-debug/robot42/sample.mcap \
  --aws-auth iot-role-alias \
  --iot-role-alias RobotLogUploader \
  --iot-thing-name robot42
```

Configuration can provide certificate paths.

---

# 43. Testing Strategy

## Unit tests

- PointCloud2 field decoding
- endian handling
- schema conversion
- voxel key calculation
- crop boundaries
- PCD parser
- PLY parser
- LAS attributes
- JobSpec validation
- S3 multipart chunking

## Golden tests

Store small fixtures:

```text
simple_xyz.pcd
xyzi.pcd
organized.pcd
simple.ply
simple.las
simple.laz
pointcloud2.mcap
malformed_pointcloud2.mcap
```

Verify deterministic outputs where possible.

## Property tests

Useful for:

- encoding/decoding round trips
- point schema layouts
- crop invariants
- voxel invariants

## Integration tests

Examples:

```text
MCAP -> PCD
PCD -> LAZ
MCAP -> reduced MCAP
MCAP -> voxel -> LAZ
```

## Rendering tests

Separate geometry/rasterizer testing from escape-sequence backend tests.

Snapshot-test framebuffers before terminal encoding.

---

# 44. Benchmarking

Add benchmarks early for realistic sizes:

```text
100k points
1M points
10M points

PointCloud2:
  10 Hz
  100k points/frame
```

Measure:

```text
throughput MB/s
points/s
peak RSS
allocations
output size
CPU time
```

Key benchmarks:

```text
MCAP channel filtering
PointCloud2 decode
voxel
LAZ encode
terminal rasterization
MCAP -> S3 pipeline
```

---

# 45. MVP Implementation Order

## Milestone 0: repository skeleton

- workspace
- CLI
- core error types
- `JobSpec`
- source/output format registry

Acceptance:

```bash
pcx --help
pcx --version
```

work on x86_64 Linux.

## Milestone 1: static point clouds

Implement:

```text
PCD reader/writer
PLY reader/writer
PointSchema
PointBatch
info
stats
convert
```

Acceptance:

```bash
pcx info sample.pcd
pcx convert sample.pcd sample.ply
```

## Milestone 2: operators

Implement:

```text
field selection
crop
voxel
merge
```

Acceptance:

```bash
pcx extract sample.pcd \
  --fields x,y,z \
  --voxel 0.05 \
  -o reduced.pcd
```

Peak memory must stay bounded.

## Milestone 3: LAS/LAZ

Implement:

```text
LAS read/write
LAZ read/write
```

Acceptance:

```bash
pcx convert sample.pcd sample.laz
```

## Milestone 4: MCAP container

Implement:

```text
MCAP metadata
channel listing
time filtering
channel filtering
reduced MCAP output
```

Do not decode PointCloud2 yet unless necessary.

Acceptance:

```bash
pcx topics run.mcap
pcx extract run.mcap \
  --topic /lidar/points \
  --from 10s \
  --to 20s \
  -o reduced.mcap
```

## Milestone 5: PointCloud2 semantics

Implement:

```text
PointCloud2 schema decode
frame extraction
stats
PCD/PLY/LAZ export
```

Acceptance:

```bash
pcx extract run.mcap \
  --topic /lidar/points \
  --at 15s \
  -o frame.pcd
```

## Milestone 6: terminal renderer

Implement:

```text
CPU rasterizer
Unicode backend
Kitty backend
terminal detection
```

Sixel may follow.

Acceptance:

```bash
ssh robot
pcx view frame.pcd
```

works without X forwarding or a web browser.

## Milestone 7: streaming remote workflow

Ensure:

```bash
ssh robot \
  'pcx extract run.mcap \
    --topic /lidar/points \
    --from 10s \
    --duration 5s \
    --voxel 5cm \
    --format laz \
    -o -' \
  > sample.laz
```

works safely.

## Milestone 8: S3

Implement:

```text
S3 sink
multipart upload
standard AWS auth
AWS IoT Role Alias credentials
```

Acceptance:

```bash
pcx upload ...
```

can process and upload without first writing a complete transformed file.

---

# 46. Definition of MVP Success

The MVP is successful when the following real workflow is possible on an edge Linux device:

```bash
scp pcx robot:/tmp/

ssh robot
```

Then:

```bash
/tmp/pcx info /data/run.mcap
```

The user can identify `/lidar/points`.

Then:

```bash
/tmp/pcx stats /data/run.mcap \
  --topic /lidar/points
```

The user can verify point count and coordinate ranges.

Then:

```bash
/tmp/pcx view /data/run.mcap \
  --topic /lidar/points \
  --at 42s
```

The user can visually confirm the cloud.

Then:

```bash
/tmp/pcx extract /data/run.mcap \
  --topic /lidar/points \
  --from 40s \
  --to 45s \
  --voxel 0.05 \
  --format laz \
  -o /tmp/debug.laz
```

The output is substantially smaller than the source.

Finally:

```bash
scp robot:/tmp/debug.laz .
```

or:

```bash
/tmp/pcx upload /data/run.mcap \
  --topic /lidar/points \
  --from 40s \
  --to 45s \
  --voxel 0.05 \
  --format mcap \
  s3://debug-bucket/robot42/debug.mcap
```

works with bounded memory.

If this workflow is reliable and easy, the product already has a clear reason to exist.

---

# 47. Architectural Rules for Codex

When implementing this project:

1. Do not put format-specific logic in CLI command handlers.
2. Do not make `PointXYZ` the universal internal type.
3. Do not load complete large recordings into memory.
4. Keep encoded-byte transport separate from point-cloud semantics.
5. Preserve container-level fast paths when no semantic decode is required.
6. Make stdout binary-safe.
7. Send diagnostics/progress only to stderr.
8. Do not introduce ROS/PCL/PDAL dependencies.
9. Prefer pure-Rust dependencies and feature flags.
10. Keep rendering optional and CPU-capable.
11. Treat MCAP as both an input container and a useful reduced-output container.
12. Do not implement daemon/service behavior.
13. Make S3 a sink, not a special processing pipeline.
14. Make AWS authentication replaceable behind a credential abstraction.
15. Add tests for malformed and unusual PointCloud2 layouts.
16. Optimize resource usage before adding advanced algorithms.

---

# 48. Product Positioning

Do not describe `pcx` as:

> a lighter PDAL

That creates a feature-count competition that is difficult to win.

Preferred positioning:

> **A single-binary toolbox for inspecting, reducing, converting and moving point-cloud recordings on edge devices.**

Alternative:

> **The shell-native workbench for point clouds and sensor recordings.**

Core differentiation:

```text
single binary
+ edge friendly
+ SSH friendly
+ terminal preview
+ multi-container
+ point-cloud semantics
+ streaming reduction
+ stdout pipeline
+ S3 transfer
```

The central workflow is:

> **inspect -> reduce -> transfer**

not:

> edit -> visualize -> model

That boundary should guide feature decisions.
