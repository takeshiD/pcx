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

Transfer output with the shell rather than a cloud client:

```bash
ssh robot 'pcx extract /data/run.mcap --topic /lidar/points --frame 0 -o -' > frame.pcd
```
