# Parser and end-to-end fixtures

This directory contains the minimal synthetic format corpus established by
issue #15, extended with strict PointCloud2 cases for issue #9 and scalar PLY
cases for issue #24. It is test data only: the generator does not parse inputs
and does not implement any `pcx` product adapter.

## Reproduction

Generator version: `pcx-fixture-generator/1.2.0`, compiled with the repository's
pinned Rust 1.97.1 toolchain. The container rewrite uses `mcap-cli 0.0.61`
(`mcap go v1.8.0`).

Generate from a checkout whose `flake.lock` is unchanged, using the official
MCAP CLI version 0.0.61 from Nixpkgs:

```bash
nix develop --command bash tests/fixtures/generate.sh
```

The Rust generator emits deterministic synthetic CDR/PointCloud2, PCD, and PLY
bytes plus a minimal raw MCAP seed. `mcap recover --compression zstd` rewrites that
seed through the official MCAP implementation. `SHA256SUMS` is regenerated
last. Repeating the command must leave the worktree unchanged.

## Provenance and license

All payload values and metadata were created for this repository; none came
from a robot, sensor, recording, customer, or person. The corpus therefore
contains **no private sensor data, personal data, credentials, or device
identifiers**. The generator and original synthetic values are released under
the repository's MIT license. The embedded ROS 2 interface definition is
derived from `common_interfaces/sensor_msgs`, licensed Apache-2.0.

The encodings follow the MCAP v0 specification, ROS 2
`sensor_msgs/msg/PointCloud2` and `sensor_msgs/msg/PointField` definitions,
OMG CDR representation identifiers/alignment, PCD v0.7, and PLY 1.0. The valid MCAP is
produced outside `pcx` by the official `mcap` CLI, providing the independent
interoperability fixture required by the test strategy.

Primary format references:

- [MCAP v0 specification](https://mcap.dev/spec)
- [ROS 2 `sensor_msgs` message definitions](https://docs.ros.org/en/ros2_packages/humble/api/sensor_msgs/)
- [ROS 2 `sensor_msgs` package license](https://raw.githubusercontent.com/ros2/common_interfaces/humble/sensor_msgs/package.xml)
- [OMG DDS-XTypes 1.3](https://www.omg.org/spec/DDS-XTypes/1.3/About-DDS-XTypes/)
- [PCD v0.7 format](https://pointclouds.org/documentation/tutorials/pcd_file_format.html)
- [PLY format description](https://paulbourke.net/dataformats/ply/)

## Valid corpus and oracle checks

All valid PointCloud2/PCD fixtures describe one unorganized Point Frame in
frame `map`, with two points and these ordered Point Fields:

| Point Field | Representation | Point 0 bits/value | Point 1 bits/value |
| --- | --- | --- | --- |
| `x` | `float32` | `0x3f800000` / `1.0` | `0x80000000` / negative zero |
| `y` | `float32` | `0xc0200000` / `-2.5` | `0x7f800000` / positive infinity |
| `z` | `float32` | `0x00000000` / `0.0` | `0x7fc01234` / quiet NaN payload |
| `intensity` | `uint16` | `42` | `65535` |
| `ring` | `uint16` | `7` | `8` |

| Fixture | Provenance | Required oracle |
| --- | --- | --- |
| `valid/pointcloud2.mcap` | Official `mcap` CLI 0.0.61 rewrite of the synthetic seed | `mcap doctor` exits successfully; `mcap info` reports one message on `/lidar/points` with schema `sensor_msgs/msg/PointCloud2 [ros2msg]` |
| `valid/pointcloud2-little-endian.cdr` | Generator 1.0.0 | CDR representation `0x0001`; strict decode yields the schema/value bits above, `height=1`, `width=2`, `point_step=16`, `row_step=32`, and little-endian point bytes |
| `valid/pointcloud2-big-endian.cdr` | Generator 1.0.0 | CDR representation `0x0000`; the same semantic values and layout decode with big-endian point bytes |
| `valid/pointcloud2-organized-row-padding.cdr` | Generator 1.1.0 | Strict decode yields a 2-by-2 organized cloud, `point_step=4`, `row_step=12`, four `float32` x values, and four padding bytes after each row |
| `valid/pointcloud2-reordered-fields-and-count.cdr` | Generator 1.1.0 | Strict decode preserves source field order and a two-element `uint8` field, and exercises every supported PointField datatype |
| `valid/pointcloud2-binary.pcd` | Generator 1.0.0 | Reviewed PCD v0.7 header; independently decoded 32-byte binary body matches the value bits above |
| `valid/pointcloud2-ascii.pcd` | Generator 1.0.0 | Reviewed PCD v0.7 header and two rows; semantic values match above (NaN payload is intentionally not preserved in ASCII) |
| `valid/scalar-vertices-ascii.ply` | Generator 1.2.0 | Reviewed PLY 1.0 header and independently tokenized rows exercise all eight supported scalar types and unknown property order |
| `valid/scalar-vertices-binary-little-endian.ply` | Generator 1.2.0 | Independent offset decoder verifies a 26-byte vertex record and little-endian scalar bits |
| `valid/scalar-vertices-binary-big-endian.ply` | Generator 1.2.0 | Independent offset decoder verifies the same values with big-endian scalar bits |

The PLY fixtures contain two unorganized vertices and these ordered scalar
properties: `signed_byte`, `unsigned_byte`, `signed_short`, `unsigned_short`,
`signed_int`, `unsigned_int`, `x`, and `time`. They cover signed and unsigned
8-, 16-, and 32-bit integers, binary32, binary64, numeric boundaries, and
negative zero. Their property names and order are part of the oracle.

The MCAP message has sequence `0` and log/publish time
`1700000000123456789` ns. Its payload must be byte-identical to
`valid/pointcloud2-little-endian.cdr`.

## Malformed corpus

Each filename states the violated invariant. Parsers must reject these inputs;
the exact error wording is not a golden interface.

| Fixture | Violated invariant | Expected oracle |
| --- | --- | --- |
| `malformed/mcap-leading-magic-must-match.mcap` | The leading bytes must equal MCAP magic | `mcap doctor` fails before accepting a recording |
| `malformed/cdr-representation-identifier-must-be-cdr.cdr` | The encapsulation representation identifier must select supported CDR | Strict CDR decode rejects identifier `0x7fff` |
| `malformed/cdr-point-data-sequence-must-not-be-truncated.cdr` | A declared sequence must fit in the remaining payload | Strict CDR decode reports truncation without allocation from unchecked length |
| `malformed/pointcloud2-field-must-fit-point-step.cdr` | Every Point Field range must fit within `point_step` | PointCloud2 validation rejects `z` at offset 14 with width 4 and `point_step=16` |
| `malformed/pointcloud2-field-names-must-be-unique.cdr` | Point Field names must be unique | PointCloud2 validation rejects the second field named `x` |
| `malformed/pointcloud2-field-ranges-must-not-overlap.cdr` | Point Field byte ranges must not overlap | PointCloud2 validation rejects `x` at bytes 0..4 overlapping `intensity` at bytes 2..4 |
| `malformed/pointcloud2-field-count-must-be-positive.cdr` | A Point Field count must be positive | PointCloud2 validation rejects `x` with `count=0` |
| `malformed/pointcloud2-field-datatype-must-be-supported.cdr` | A Point Field datatype must be one of the eight ROS constants | PointCloud2 validation rejects datatype `9` with field context |
| `malformed/pointcloud2-row-step-must-cover-row.cdr` | `row_step` must cover `width * point_step` | PointCloud2 validation rejects `row_step=7` for two four-byte points |
| `malformed/pointcloud2-data-length-must-equal-height-times-row-step.cdr` | Data length must equal `height * row_step` | PointCloud2 validation rejects 15 bytes for two eight-byte rows |
| `malformed/pointcloud2-timestamp-nanoseconds-must-be-canonical.cdr` | Header nanoseconds must be below one billion | PointCloud2 validation rejects `nanosec=1000000000` |
| `malformed/pointcloud2-height-must-be-positive.cdr` | PointCloud2 height must be positive | PointCloud2 validation rejects `height=0` |
| `malformed/pointcloud2-point-step-must-be-positive.cdr` | A nonempty cloud must have a positive `point_step` | PointCloud2 validation rejects `point_step=0` when `width * height` is nonzero |
| `malformed/pcd-points-must-equal-width-times-height.pcd` | `POINTS` must equal `WIDTH * HEIGHT` | PCD header validation rejects `POINTS=1`, `WIDTH=2`, `HEIGHT=1` |
| `malformed/ply-list-properties-are-unsupported.ply` | The faithful subset accepts scalar vertex properties only | PLY header validation rejects the list before materialization |
| `malformed/ply-int64-properties-are-unsupported.ply` | PLY 1.0 has no supported lossless 64-bit integer mapping | PLY header validation rejects `int64` |
| `malformed/ply-format-endianness-must-be-known.ply` | The format must declare ASCII, little-endian binary, or big-endian binary | PLY header validation rejects an unknown byte order |
| `malformed/ply-non-vertex-elements-are-lossy.ply` | Faces and other elements cannot be represented by the point schema | PLY header validation rejects the element instead of dropping it |
| `malformed/ply-binary-payload-must-not-be-truncated.ply` | Every declared scalar for every vertex must be present | PLY payload decoding reports truncation |

## Review commands

```bash
nix shell nixpkgs#mcap-cli --command mcap doctor tests/fixtures/valid/pointcloud2.mcap
nix shell nixpkgs#mcap-cli --command mcap info tests/fixtures/valid/pointcloud2.mcap
nix shell nixpkgs#mcap-cli --command mcap cat tests/fixtures/valid/pointcloud2.mcap
nix shell nixpkgs#mcap-cli --command mcap doctor tests/fixtures/malformed/mcap-leading-magic-must-match.mcap
nix develop --command bash -c 'cd tests/fixtures && sha256sum --check SHA256SUMS'
nix develop --command du -ch tests/fixtures/valid/* tests/fixtures/malformed/*
```

Binary changes are reviewed with `mcap info`, `mcap doctor`, `xxd`, and the
recorded checksums. Golden updates are explicit local operations only.
