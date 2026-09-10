---
title: Quick start
description: Inspect MCAP metadata and extract one Point Frame.
---

## Available now

```bash
pcx --help
pcx --version
pcx info run.mcap
```

## Extract one Point Frame

```bash
pcx topics run.mcap --json
pcx extract run.mcap \
  --topic /lidar/points \
  --frame 0 \
  -o frame.pcd
```

`--frame` uses a zero-based index after Topic selection. `--at 83.2s` selects the first Point Frame whose MCAP log time is at or after the requested duration from recording start. The two selectors are mutually exclusive.

## Copy one encoded message to MCAP

```bash
pcx passthrough run.mcap --topic /lidar/points --frame 0 -o selected.mcap
```

This container path preserves encoded message and recording-level records
without PointCloud2 decoding.

## Render one Point Frame in the terminal

```bash
pcx render run.mcap --topic /lidar/points --frame 0
```

The default `--backend auto` falls back conservatively; the current process
query does not auto-authorize Kitty or Sixel.
Use `--backend unicode` for an explicit interactive text-cell rendering. When
stdout is redirected, automatic output is deterministic monochrome Unicode
without ANSI or graphics-protocol escape sequences.

## Render a Static Cloud

```bash
pcx render tests/fixtures/valid/pointcloud2-ascii.pcd --width 32 --height 12
pcx render tests/fixtures/valid/las-pdal.las --width 32 --height 12
pcx render tests/fixtures/valid/las-pdal.laz --width 32 --height 12
```

Static Clouds do not use `--topic`, `--frame`, or `--at`. The readers accept the
supported PCD subset and LAS/LAZ, then apply the same bounded projection and
terminal output policy as MCAP rendering. LAS and LAZ use one global fit and
must fit as a complete declared cloud under `--memory-limit`.

Transfer output with the shell rather than a cloud client:

```bash
ssh robot 'pcx extract /data/run.mcap --topic /lidar/points --frame 0 -o -' > frame.pcd
```
