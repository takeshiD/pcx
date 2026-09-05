---
title: Command design
description: Current interface and accepted v0.1 command contracts.
---

## Available now

```bash
pcx --help
pcx --version
pcx info INPUT.mcap [--json]
pcx topics INPUT.mcap [--json]
```

`pcx info` streams through an MCAP Source without decoding point frames. Human output and versioned JSON go to stdout; successful inspection leaves stderr empty.

`pcx topics` lists each MCAP Channel with its user-facing Topic, Schema, encodings, message count, and metadata-based ROS 2 PointCloud2 candidate status. Candidate status does not claim that message payloads have been decoded or validated.

## Accepted for v0.1

```bash
pcx extract INPUT.mcap --topic TOPIC --frame INDEX [-o OUTPUT.pcd]
```

`--frame` is zero-based within messages matching the selected topic. Exactly one frame is selected. Missing topics, out-of-range frames, malformed messages and operations that cannot satisfy the memory budget fail before producing a committed output.

## Streams and exit status

- Human diagnostics and progress use stderr.
- Data and `--json` results use stdout.
- JSON objects carry a `schema_version`.
- Success is `0`; usage errors, invalid data and resource refusal are non-zero.
- Interrupt handling removes temporary output and returns `130`.
- Existing output is rejected unless `--force` is explicit.

Command examples for planned behavior are always labeled as planned.
