---
title: Status and roadmap
description: What pcx implements today and what comes next.
---

## Foundation — available

- publishable `pcx-cli` package with a `pcx` binary;
- `--help` and `--version`;
- bounded synchronous MCAP reading and `pcx info` metadata output;
- accepted architecture, domain language, ADRs and test strategy;
- Nix, CI, Pages and release automation foundation.

## v0.1 MCAP vertical slice — available

1. Topic, Channel, Schema, and message-count discovery;
2. strict ROS 2 CDR decoding for `sensor_msgs/msg/PointCloud2`;
3. zero-copy `PointView` where possible;
4. select one Point Frame by index or MCAP log time;
5. write binary or ASCII PCD.

## Later milestones

- **Reduction:** field selection, crop, statistics and frame-local voxel sampling.
- **Formats:** PCD input, PLY, LAS/LAZ and reduced MCAP output.
- **Terminal rendering:** deterministic CPU rasterizer, Unicode and Kitty backends; Sixel follows.
- **Hardening:** longer fuzz runs, performance baselines, musl investigation and resource tuning.

## Explicit non-goals

AWS/S3, cloud credentials, a network listener, daemon mode, a desktop GUI, ROS/PCL/PDAL runtime dependencies, SLAM, meshing and ML inference are outside the product boundary.
