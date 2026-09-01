---
title: Formats and protocols
description: Accepted format boundaries and fidelity rules.
---

## v0.1 scope

| Boundary | Read | Write | Status |
| --- | --- | --- | --- |
| MCAP container | Metadata, channels and messages | No | Planned for v0.1 |
| ROS 2 `sensor_msgs/msg/PointCloud2` | Strict CDR decoding | No | Planned for v0.1 |
| PCD | No | Binary and ASCII | Planned for v0.1 |

PLY, LAS/LAZ and terminal rendering are later work. AWS/S3 transports and cloud credentials are not product features.

## Fidelity contract

`pcx` rejects unsupported layouts and ambiguous conversions. It never silently drops fields, changes numeric types, rewrites coordinate meaning, or discards metadata. A command that intentionally changes the schema must make that change explicit in its arguments and report it in structured output.

## ROS 2 decoding

The decoder accepts only the declared `PointCloud2` schema and validates CDR alignment, endianness, dimensions, field offsets, strides and buffer length before exposing point data. A ROS installation is not required.

## Output ownership

Binary data may be written to stdout. Diagnostics always go to stderr. File output is written to a sibling temporary file and atomically renamed only after the encoder succeeds.
