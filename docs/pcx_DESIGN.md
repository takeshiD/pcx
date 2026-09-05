# pcx Design Document

Status: accepted foundation design

Last updated: 2026-09-05

Owner: tkcd

This is the product-level design. Detailed contracts live in `ARCHITECTURE.md`,
`TESTING.md`, `RELEASE.md`, and `docs/adr/`.

## 1. Product vision

`pcx` is a shell-native point-cloud workbench for edge Linux and SSH workflows.
It lets engineers inspect a recording where the data lives, reduce only the
useful part, and transfer the result with existing shell tools.

```text
inspect -> reduce -> transfer with ssh/scp/rsync or another external tool
```

The product is one synchronous executable and requires neither a ROS runtime,
desktop environment, nor resident daemon.

## 2. Product boundary

### 2.1 v0.1

- `pcx info INPUT.mcap` for container metadata;
- `pcx topics INPUT.mcap` for topic and schema discovery;
- `pcx extract INPUT.mcap --topic TOPIC --frame INDEX -o OUTPUT.pcd`;
- strict ROS 2 `sensor_msgs/msg/PointCloud2` CDR decoding;
- binary and ASCII PCD output;
- human-readable and versioned JSON inspection output;
- bounded managed memory and atomic file output.

The v0.1 inspection and one-frame extraction commands are implemented and
covered by committed end-to-end fixtures.

### 2.2 Later phases

Field selection, crop, statistics, frame-local voxel reduction, PCD input,
MCAP output, PLY, LAS/LAZ, and terminal rendering may follow v0.1.

### 2.3 Explicit non-goals

- AWS, S3, or any other cloud-specific client;
- cloud credential management or discovery;
- a network protocol or background daemon;
- a GUI;
- a ROS installation or ROS graph integration;
- PCL or PDAL as a required runtime dependency;
- transparent semantic or metadata loss.

Remote transfer is composition, not a `pcx` feature:

```bash
ssh robot 'pcx extract /data/run.mcap --topic /lidar/points --frame 0 -o -' \
  > frame.pcd
```

## 3. Users and core stories

The primary user is a robotics, autonomy, or LiDAR engineer on a robot,
industrial PC, build server, or remote Linux host.

1. Confirm that a large MCAP contains useful point-cloud data.
2. List candidate topics without loading all messages.
3. Extract exactly one frame over SSH.
4. Refuse malformed data or work whose resource bound cannot be proven.
5. Use stable JSON from scripts without parsing human diagnostics.

## 4. CLI contract

Accepted v0.1 commands are:

```bash
pcx info INPUT.mcap [--json]
pcx topics INPUT.mcap [--json]
pcx extract INPUT.mcap --topic TOPIC (--frame INDEX | --at DURATION) \
  -o PATH|- [--encoding binary|ascii] [--memory-limit BYTES] [--force]
```

`--frame` is zero-based among messages matching the selected topic. `--at`
selects the first frame at or after a duration from recording start. v0.1
extracts exactly one frame, defaults to binary PCD, and requires an explicit
file or stdout sink.

### Streams and failures

- stdout contains data only; stderr contains diagnostics and progress only.
- Binary stdout is never mixed with logs.
- JSON includes a top-level `schema_version`.
- Success is `0`; error categories receive distinct non-zero statuses.
- Interrupt handling returns `130` and removes temporary output.
- Existing output is rejected unless `--force` is explicit.
- File output is atomically renamed from a sibling temporary file after success.

## 5. Package and module topology

The crates.io package is `pcx-cli`; its binary is `pcx`:

```bash
cargo install pcx-cli
```

There is one Rust package, one library target, and one binary target. Core and
format boundaries are deep internal modules, not separately published crates.

```text
src/
├── lib.rs
├── main.rs          # process boundary only
├── cli/             # arguments, presentation, exit mapping
├── core/            # domain types, planning, execution contracts
├── mcap/            # bounded container adapter
├── ros2/            # narrow CDR and PointCloud2 decoder
├── pcd/             # PCD encoder/decoder
├── ply/             # faithful scalar-vertex PLY reader/writer
└── ops/             # point-frame operators

cli -> core <- mcap
              ros2
              pcd
              ply
              ops
```

`core` does not import a concrete format module. The CLI composes adapters but
does not own format logic.

## 6. Control plane

```text
CLI -> JobSpec -> Probe -> Planner -> Plan -> Executor -> Report
```

- `JobSpec` is format-independent requested behavior.
- `Probe` reads only enough source structure to validate and estimate work.
- `Planner` selects a pipeline and calculates conservative resource bounds.
- `Plan` is executable only when every required guarantee holds.
- `Executor` performs no hidden strategy selection.

If a safe upper bound on managed memory cannot be established before execution,
planning fails. Execution does not start optimistically.

## 7. Data planes

Container records and semantic point frames remain separate.

Encoded metadata and records may be inspected or copied without point decoding.
Operations on coordinates or fields use the semantic pipeline:

```text
encoded message -> CDR validation -> PointView -> optional PointBatch
                -> frame-local operators -> encoder
```

A container record is not a point frame; APIs do not interchange those states.

## 8. Point ownership model

`PointView` holds reference-counted source bytes and a validated layout for
low-copy field access. `PointBatch` owns typed, columnar fields for operators
that materialize or change a schema. Conversions are explicit and fallible.

The planner accounts for retained source bytes, decoded columns, operator
scratch, encoder buffers, output buffers, and conservative overhead.

## 9. Format boundaries

### MCAP

Use the official Rust `mcap` crate through its sans-I/O facilities. A bounded,
synchronous adapter owns file reads and buffering. zstd and LZ4 may be enabled;
an async runtime is not introduced.

### ROS 2 PointCloud2

Implement a narrow CDR cursor for the v0.1 message and check representation,
endianness, alignment, checked offsets, dimensions, steps, fields, buffer length,
padding, and organized/unorganized layouts. Unsupported or inconsistent input
is rejected with source context. A ROS installation is not required.

### PCD

The writer supports binary and ASCII modes. Header fields derive from the
validated schema. A representation that would lose information fails unless a
future command exposes an explicit lossy policy.

### PLY

The reader and writer support a documented PLY 1.0 subset with one scalar-only
`vertex` element in ASCII or either binary byte order. All supported scalar
properties and their order map through the common schema. Unsupported elements,
lists, 64-bit integers, vector fields, organized shape, non-default frame
metadata, and non-portable ASCII float values fail before output or
materialization. Header probing is bounded and supplies the common Planner with
an exact column allocation before payload decoding.

## 10. Operators

Operators are frame-local unless a command explicitly states otherwise. Each
declares accepted schemas, whether it materializes, output schema changes,
conservative scratch memory, ordering, and determinism. Initial post-v0.1
operators are field selection, crop, statistics, and voxel reduction.

Axis-aligned crop uses a half-open interval on every axis: a coordinate is
retained when `min <= value < max`. Bounds must be finite and strictly
increasing. Points with a NaN or infinity in X, Y, or Z are discarded. Crop
preserves field order, field types and values, frame metadata, and point order;
if the point count changes, its output is explicitly unorganized (`height = 1`).

## 11. Memory and resource policy

Managed memory includes buffers and collections allocated by `pcx`. Memory maps
and OS caches are not exact resident-memory promises, but mapped ranges and
retained bytes are disclosed. All size arithmetic is checked and every limit is
applied before allocation.

```text
source buffers
+ retained message bytes
+ decoded/materialized columns
+ operator scratch
+ encoder/output buffers
+ conservative overhead
```

There is no fallback that silently weakens fidelity.

## 12. Fidelity and determinism

- Unknown fields and metadata are never silently discarded.
- Numeric types, field order, and coordinate meaning are preserved unless an
  explicit operator changes them.
- Schema changes appear in human and JSON output.
- Identical input and options produce deterministic ordering.
- Floating-point edge cases and invalid coordinates have documented policies.

## 13. Security boundary

Every input file is untrusted. Parsers use checked arithmetic, validate lengths
before slicing, cap counts before allocation, and never panic for malformed
data. The product opens local files and standard streams only; it has no network
or credential-handling surface.

## 14. Platform support

Supported and natively tested:

- `x86_64-linux`;
- `aarch64-linux`.

macOS and Windows are future possibilities without a committed date. Cross-built
artifacts do not count as supported without native tests. MSRV will be defined
later; the current toolchain is pinned for reproducibility.

## 15. Documentation

- `README.md` is English and links to `README.ja.md`.
- GitHub Pages uses Astro and Starlight in English and Japanese.
- Availability labels distinguish implemented, planned, and out-of-scope work.
- Architecture is accessible static HTML/CSS and works without client JS.

## 16. CI and releases

Every pull request runs formatting, Clippy with warnings denied, type checking,
tests, Nix checks, package verification, and documentation checks. Rust and Nix
checks run natively on both supported Linux architectures.

Only exact `vX.Y.Z` and `vX.Y.Z-alpha.N` tags enter release. The tag version must
equal the `pcx-cli` package version. Before publication, CI repeats all gates,
runs `cargo package`, builds the extracted package, and performs an install smoke
test. This catches package-content and build failures before `cargo publish`.

After approval through a protected `release` environment:

- publish existing `pcx-cli` packages to crates.io through Trusted Publishing OIDC;
- push both Nix closures to public Cachix cache `takeshid`;
- create a GitHub release with Linux archives and SHA-256 checksums.

The only release environment secret is `CACHIX_AUTH_TOKEN`. The protected publish job alone receives `id-token: write` and exchanges GitHub's OIDC identity for a short-lived crates.io token. A maintainer manually publishes the first `pcx-cli` version after equivalent gates, then binds Trusted Publishing to this repository, `release.yml`, and the `release` environment.

## 17. Test design

1. Unit tests for checked cursors, schemas, selection, and estimates.
2. Reviewed minimal MCAP/CDR/PCD/PLY fixtures, including malformed inputs.
3. Property tests for layouts, planner monotonicity, and codecs.
4. CLI integration tests for streams, exit status, `--force`, and interruption.
5. End-to-end tests for one-frame MCAP-to-PCD conversion.
6. Scheduled fuzzing for parser entry points.
7. Benchmarks for time, peak managed memory, and output size.

No test requires AWS, S3, cloud credentials, or a remote network service.

## 18. Delivery order

### Foundation

- package metadata, MIT license, and CLI shell;
- bilingual documentation and architecture view;
- Nix flake for both Linux architectures;
- CI, Pages, fuzz schedule, and strict release automation.

### v0.1 inspect and extract

- domain types, error taxonomy, and versioned reports;
- bounded MCAP probe and frame selection;
- strict PointCloud2 CDR decoder;
- `PointView`, `PointBatch`, PCD writer, and atomic sink;
- `info`, `topics`, one-frame `extract`, fixtures, properties, and E2E tests.

### Later work

- frame-local reduction operators;
- additional point-cloud readers and writers;
- container output where fidelity contracts can be met;
- deterministic terminal rendering and packaging hardening.

## 19. Definition of v0.1 success

On both supported Linux architectures, an engineer can inspect an MCAP, list
topics, and extract one ROS 2 PointCloud2 frame to valid PCD using one binary and
no ROS installation. The command either proves its managed-memory bound before
execution and commits a faithful output, or refuses without a partial file.
