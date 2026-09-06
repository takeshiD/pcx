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
pcx extract INPUT.mcap --topic TOPIC (--frame INDEX | --at DURATION) \
  -o OUTPUT.pcd|- [--encoding binary|ascii] [--memory-limit BYTES] [--force]
pcx passthrough INPUT.mcap --topic TOPIC (--frame INDEX | --at DURATION) \
  -o OUTPUT.mcap|- [--compression none|zstd|lz4] [--memory-limit BYTES] [--force]
```

`pcx info` streams through an MCAP Source without decoding point frames. Human output and versioned JSON go to stdout; successful inspection leaves stderr empty.

`pcx topics` lists each MCAP Channel with its user-facing Topic, Schema, encodings, message count, and metadata-based ROS 2 PointCloud2 candidate status. Candidate status does not claim that message payloads have been decoded or validated.

## One-frame extraction

```bash
pcx extract INPUT.mcap --topic TOPIC (--frame INDEX | --at DURATION) \
  -o OUTPUT.pcd|- [--encoding binary|ascii] [--memory-limit BYTES] [--force]
```

`--frame` is zero-based within messages matching the selected topic. `--at` selects the first frame at or after a duration such as `83.2s` from recording start. Exactly one selector and an explicit file or stdout sink are required. Binary PCD is the default. Missing topics, out-of-range frames, malformed messages and operations that cannot satisfy the memory budget fail before producing a committed output.

## Encoded MCAP passthrough

`pcx passthrough` selects one encoded message without PointCloud2 or point-field
decoding. It preserves the Message payload, sequence and times; the exact
Channel and optional Schema relationship; all recording-level attachments and
metadata; and application-private records. Container structure, statistics and
CRCs are regenerated; attachment and metadata indexes are omitted to keep
writer memory bounded. Unknown future standard records fail explicitly because
their preservation semantics are not yet defined. Compression defaults to
single-threaded deterministic zstd; `none` and deterministic LZ4 are explicit
alternatives.

## Streams and exit status

- Human diagnostics and progress use stderr.
- Data and `--json` results use stdout.
- Successful `--json` output uses stdout. Failures from a successfully parsed JSON command leave stdout empty and write a versioned JSON error object to stderr.
- The JSON schemas and compatibility policy are published in [`docs/json-schema`](https://github.com/takeshiD/pcx/tree/main/docs/json-schema). Human-readable output and diagnostic message wording are not compatibility contracts.
- Success is `0`; usage errors, invalid data and resource refusal are non-zero.
- Interrupt handling removes temporary output and returns `130`.
- Existing output is rejected unless `--force` is explicit.
