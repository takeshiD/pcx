---
title: Quick start
description: Inspect MCAP metadata and preview the remaining v0.1 workflow.
---

## Available now

```bash
pcx --help
pcx --version
pcx info run.mcap
```

## Planned for v0.1

```bash
pcx topics run.mcap --json
pcx extract run.mcap \
  --topic /lidar/points \
  --frame 0 \
  -o frame.pcd
```

`--frame` uses a zero-based index after Topic selection. `--at 83.2s` selects the first Point Frame whose MCAP log time is at or after the requested duration from recording start. The two selectors are mutually exclusive.

Transfer output with the shell rather than a cloud client:

```bash
ssh robot 'pcx extract /data/run.mcap --topic /lidar/points --frame 0 -o -' > frame.pcd
```
