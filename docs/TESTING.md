# pcx Test Strategy

Status: accepted design. Tests arrive with the product slices they protect.

## Objectives

Tests must prove semantic fidelity, parser safety, bounded managed memory, shell composability, and native behavior on both supported Linux architectures. A test suite that only round-trips through pcx's own encoder and decoder is insufficient.

## Test layers

### Unit tests

Fast, deterministic tests live beside their modules.

- `core`: schemas, capability matching, frame selectors, loss policy, memory planning, exit mapping.
- `mcap`: Channel/Schema association, log times, chunk boundaries, truncation, CRC failures, payload preservation.
- `ros2`: CDR alignment, endian markers, sequence lengths, PointField counts and offsets, organized rows, malformed extents.
- `pcd`: headers, binary/ASCII values, arbitrary fields, non-finite values, deterministic formatting.
- `ply`: bounded headers, both binary byte orders, ASCII values, arbitrary scalar properties, fidelity refusals.
- `ops`: boundaries and invariants for every semantic operator, including
  projection camera, aspect, z-buffer, color, and degenerate-bounds policies.
- `terminal`: cell geometry, explicit color profiles, non-TTY normalization,
  output bounds, and control-sequence confinement for hostile raster metadata.

### Property tests

`proptest` runs in normal pull-request CI for layout arithmetic, CDR sequences, crop invariants, schema/value round-trips, and deterministic frame selection. Minimized regression seeds are committed.

### Integration tests

Integration tests compose real adapters through internal interfaces. Core tests use fake sources and sinks so format dependencies cannot leak into the Planner.

Terminal selection tests inject environment variables, stdin/stdout TTY state,
and typed capability-query results. They do not alter terminal modes or emit
real control sequences. Coverage includes query timeout, SSH, tmux, missing
`TERM`, hostile environment bytes, and redirected stdout.

### CLI contract tests

The compiled process is exercised for:

- stdout/stderr separation;
- exit codes;
- JSON envelope and schema version;
- `--frame`/`--at` exclusion;
- existing-output refusal and `--force`;
- atomic output and Ctrl-C cleanup;
- redirected output and broken pipes.

### End-to-end test

The v0.1 gate is:

```text
fixture.mcap
  -> pcx info
  -> pcx topics --json
  -> pcx extract --topic /lidar/points --frame 0
  -> independent PCD decode
  -> expected schema + bit-level values
```

## Fixtures

Fixtures remain under 1 MiB and need no Git LFS.

1. Hand-authored payloads target exact CDR and PointCloud2 edge cases.
2. Deterministic generators create small integration MCAP/PCD files and are committed with their seeds.
3. At least one fixture produced outside pcx validates interoperability.

Every checked-in fixture records its source, generator command/version, license, expected schema, and whether it contains real sensor data. Real robot logs, personal data, credentials, and device identifiers are prohibited.

## Oracles

| Subject | Oracle |
| --- | --- |
| Lossless point fields | primitive type, count, and value bits; NaN payloads included |
| MCAP output | semantic records, Channel/Schema relationships, message payload and log time |
| PCD header | reviewed golden text |
| PCD points | independently decoded semantic values |
| CPU projection | reviewed synthetic raster cells plus independent policy assertions |
| Unicode terminal output | reviewed LF-only snapshot with ESC made visible as literal `\x1b` |
| CLI text/JSON | reviewed snapshots with volatile values normalized |
| Errors | typed code and structured context, never backtraces |

Whole MCAP files are not byte-golden because legal chunking, indexes, compression metadata, and writer identifiers may differ.

Golden updates are explicit local operations and never automatic in CI. Reviewers inspect the resulting diff.

## Malformed-input matrix

Minimum PointCloud2/CDR cases:

- little and big endian;
- reordered fields and `count > 1`;
- organized data with row padding;
- NaN, infinity, negative zero, and all-zero coordinates;
- field outside `point_step`;
- overlapping and duplicate fields;
- truncated payload and sequence;
- `width * height`, row and field arithmetic overflow;
- inconsistent `row_step`, `point_step`, and data length.

Minimum MCAP cases:

- missing or invalid magic;
- truncated record/chunk;
- invalid CRC;
- unknown record and schema;
- duplicate IDs;
- missing summary;
- zstd/LZ4 decompression failure;
- PointCloud2 Channel whose payload uses the wrong encoding.
- passthrough of attachments, metadata, private records, exact Schema/Channel
  IDs, message payload, sequence, log time, and publish time;
- explicit refusal of unknown future standard records that cannot be
  faithfully rewritten;
- byte-deterministic zstd and LZ4 passthrough output for identical inputs and
  options.

Minimum PLY cases:

- ASCII, little-endian binary, and big-endian binary scalar vertices;
- unknown scalar properties and source order;
- list properties, unsupported scalar types, and non-vertex elements;
- truncated and trailing payloads;
- materialization refusal before column allocation.

## Resource tests

Pull requests block on deterministic managed-memory behavior:

- planned peak never exceeds the configured budget;
- an unplannable job is rejected before output creation;
- retained batch count is constant for a long synthetic stream;
- decompression and output queues remain bounded;
- allowed spooling is explicit and cleaned after failure.

RSS, throughput, allocations, points/s, bytes/s, and output size are measured in scheduled benchmarks. Runner noise makes them reports rather than initial merge gates.

## Fuzzing

`cargo-fuzz` runs on a schedule and manually, targeting MCAP record boundaries, strict CDR, PointCloud2 layouts, and PCD headers. A crash is minimized and added to the regression suite before the fix is merged. Scheduled fuzzing is not itself a release gate.

## CI matrix

| Check | x86_64 Linux | aarch64 Linux |
| --- | --- | --- |
| fmt, clippy, check | Required | Required |
| unit/property/integration | Required | Required |
| CLI and v0.1 E2E | Required | Required |
| Nix package/check | Required | Required |
| Starlight build | Required once | Not duplicated |
| scheduled fuzz/bench | Primary runner | Targeted follow-up |

Coverage is reported but no global percentage initially blocks a change. Critical parser branches and safety contracts are reviewed by scenario, not optimized for a vanity number.

## Required local commands

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo check --all-targets --all-features
cargo test --all-features
nix flake check
npm --prefix docs/pages ci
npm --prefix docs/pages run check
npm --prefix docs/pages run build
```
