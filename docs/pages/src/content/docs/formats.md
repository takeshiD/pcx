---
title: Formats and protocols
description: Accepted format boundaries and fidelity rules.
---

## v0.1 scope

| Boundary | Read | Write | Status |
| --- | --- | --- | --- |
| MCAP container | Metadata and encoded records | One-message container passthrough | Available |
| ROS 2 `sensor_msgs/msg/PointCloud2` | Strict CDR decoding | No | Available |
| PCD | ASCII and little-endian binary | Binary and ASCII | Reader adapter available; no input CLI command yet |
| PLY 1.0 | Scalar vertices in ASCII and both binary byte orders | ASCII and both binary byte orders | Adapter available; no CLI command yet |
| LAS/LAZ | Bounded synchronous batches | Bounded synchronous batches | Library adapter available; CLI not yet exposed |

LAS/LAZ CLI integration, Kitty, and Sixel are later work. The CPU rasterizer,
capability selection, and Unicode terminal backend are available internally.
AWS/S3 transports and cloud credentials are not product features.

## Faithful PLY subset

PLY input and output support exactly one `vertex` element with ordered scalar
properties. The accepted PLY 1.0 modes are `ascii`,
`binary_little_endian`, and `binary_big_endian`; binary payloads are decoded
and encoded according to the declared byte order.

| PLY scalar | Common Point Field |
| --- | --- |
| `char` / `int8` | signed 8-bit integer |
| `uchar` / `uint8` | unsigned 8-bit integer |
| `short` / `int16` | signed 16-bit integer |
| `ushort` / `uint16` | unsigned 16-bit integer |
| `int` / `int32` | signed 32-bit integer |
| `uint` / `uint32` | unsigned 32-bit integer |
| `float` / `float32` | IEEE-754 binary32 |
| `double` / `float64` | IEEE-754 binary64 |

Unknown scalar property names and their order are preserved. List properties,
non-vertex elements such as faces, 64-bit integers, Point Fields with
`count > 1`, organized clouds, and semantics that cannot be reconstructed from
the property name are rejected as unsupported or lossy. Binary floats retain
their exact bits. ASCII writing and reading reject NaN and infinity because PLY
1.0 does not define portable spellings for them; finite values, including
negative zero, round-trip.

PLY does not carry Point Frame metadata or an organized shape. A read produces
the documented static-cloud defaults: timestamp zero, empty frame id,
`is_dense = false`, width equal to the vertex count, and height one. Comments
and `obj_info` lines are accepted as non-semantic header annotations but are not
part of the common point schema. Writing requires those same static-cloud
metadata defaults and rejects timestamps, frame identity, density, container
times, or an organized shape instead of silently discarding them.

The reader parses at most a 64 KiB header, reports the exact point-column
allocation, and requires a sufficient materialization budget before allocating
those columns. Payload I/O is synchronous and fixed-buffered; the encoded file
is never loaded wholesale.

MCAP passthrough retains the selected encoded Message and its exact
Channel/Schema relationship, together with recording-level attachments,
metadata, and private records. Derived container structure is rebuilt with a
fixed bounded-memory policy.

## LAS and LAZ mapping

LAS/LAZ coordinates map to semantic `f64` X/Y/Z Point Fields. Their original
per-axis scale and offset remain attached as the Coordinate Transform, and CRS
VLR/EVLR bytes remain in the retained LAS header. Classification is separate
from synthetic, key-point, withheld and overlap flags. Standard point-format
attributes map to named typed Point Fields; Extra Bytes map to the ordered
`u8[count]` field `las_extra_bytes` while their descriptor records are retained.

Reads use a caller-selected maximum points per batch and reject a memory bound
that cannot cover the raw slab, decoded columns and retained header records.
Serial LAZ compression avoids an unbounded parallel queue, and writers require
a maximum point count so the chunk table is planned before output. Writing
refuses coordinate quantization unless representation loss is explicitly
authorized.

## Fidelity contract

`pcx` rejects unsupported layouts and ambiguous conversions. It never silently drops fields, changes numeric types, rewrites coordinate meaning, or discards metadata. A command that intentionally changes the schema must make that change explicit in its arguments and report it in structured output.

## ROS 2 decoding

The decoder accepts only the declared `PointCloud2` schema and validates CDR alignment, endianness, dimensions, field offsets, strides and buffer length before exposing point data. A ROS installation is not required.

## Output ownership

Binary data may be written to stdout. Diagnostics always go to stderr. File output is written to a sibling temporary file and atomically renamed only after the encoder succeeds.
