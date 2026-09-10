---
title: Formats and protocols
description: Accepted format boundaries and fidelity rules.
---

## v0.1 scope

| Boundary | Read | Write | Status |
| --- | --- | --- | --- |
| MCAP container | Metadata and encoded records | One-message container passthrough | Available |
| ROS 2 `sensor_msgs/msg/PointCloud2` | Strict CDR decoding | No | Available |
| PCD | ASCII and little-endian binary | Binary and ASCII | Readable as a Static Cloud through `pcx render` |
| PLY 1.0 | Scalar vertices in ASCII and both binary byte orders | ASCII and both binary byte orders | Adapter available; no CLI command yet |
| LAS/LAZ | Bounded synchronous batches | Bounded synchronous batches | Readable as a Static Cloud through `pcx render` |
| Terminal raster | One selected MCAP Point Frame or PCD/LAS/LAZ Static Cloud | Unicode/ANSI, Kitty, or Sixel | Available through `pcx render` |

LAS/LAZ conversion commands remain later work. The CPU rasterizer,
conservative capability selection, and Unicode, Kitty, and Sixel terminal
backends are available through `pcx render`. AWS/S3 transports and cloud credentials are not
product features.

## Terminal rendering

`pcx render` selects one ROS 2 `PointCloud2` Point Frame from MCAP or reads one
PCD, LAS, or LAZ Static Cloud and projects it into a bounded terminal-neutral raster. MCAP
requires Topic and Point Frame selectors; static Sources reject them. Projection is
synchronous,
orthographic, axis-aligned, fitted to the requested raster, and frame-local. It
does not modify or replace the Source Point Frame.

Unicode rendering packs two vertical raster pixels into one terminal cell with
`▀`, `▄`, and `█`. Interactive color uses ANSI SGR truecolor; `NO_COLOR` and all
non-TTY output use monochrome occupancy instead. Non-TTY bytes contain UTF-8
block glyphs, spaces, and LF only—no ANSI or graphics-protocol escapes.

Kitty streams transparent RGBA through bounded base64 chunks. Sixel streams a
transparent-background, deterministic palette image and refuses more than its
configured palette limit rather than quantizing colors. Both graphics backends
require TTY stdout. Automatic selection never trusts `TERM` alone to authorize
graphics output: Kitty or Sixel must be confirmed by the bounded capability
query, otherwise output falls back to Unicode or safe plain text. See the
[terminal contract](https://github.com/takeshiD/pcx/blob/main/docs/TERMINAL.md)
for exact bounds, detection order, byte alphabets, and interruption cleanup.
The current CLI process query reports unsupported, so `auto` does not yet
authorize Kitty or Sixel; select either explicitly on TTY stdout.

### Sixel terminal protocol

The Sixel adapter streams a deterministic transparent-background image from
the common raster. It refuses dimensions, distinct colors, or exact encoded
payloads beyond caller-supplied limits before output starts. Sixel escapes are
eligible only when the shared capability policy selected Sixel; other selected
backends are refused before DCS entry and use their own renderer. Capability
probing is not inferred by the encoder. See the repository's terminal rendering
contract for the byte, memory, fallback, and interruption rules.

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
For terminal rendering, the header-declared point count becomes the batch bound
and the complete Static Cloud, projection raster, and encoder are planned
together before point decoding. This preserves one global fit; oversized clouds
are refused rather than fitted independently per batch.
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
