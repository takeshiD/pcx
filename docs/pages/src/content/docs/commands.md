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
pcx render INPUT.mcap --topic TOPIC (--frame INDEX | --at DURATION) \
  [--backend auto|unicode|kitty|sixel] [--width PIXELS] [--height PIXELS] \
  [--palette-limit COLORS] [--payload-limit BYTES] [--memory-limit BYTES]
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

## Render one Point Frame

```bash
pcx render INPUT.mcap --topic TOPIC (--frame INDEX | --at DURATION) \
  [--backend auto|unicode|kitty|sixel] [--width PIXELS] [--height PIXELS] \
  [--palette-limit COLORS] [--payload-limit BYTES] [--memory-limit BYTES]
```

`pcx render` uses the same Topic and Point Frame selectors as `extract`. It
strictly decodes one ROS 2 `PointCloud2`, performs deterministic frame-local CPU
projection into a bounded raster, and streams one inline rendering to stdout.
It is a one-shot command, not a full-screen viewer or event loop.

`auto` is the default backend. Redirected stdout is never queried and produces
deterministic Unicode occupancy text without terminal control sequences. On a
TTY, missing or `dumb` `TERM` uses plain output; non-interactive stdin, SSH, and
tmux use Unicode without probing. Other interactive sessions may use a bounded
capability query; confirmed Kitty or Sixel support selects that protocol, and
unsupported, malformed, failed, or timed-out detection falls back to Unicode.
The current process query reports unsupported, so the shipped `auto` path does
not select a graphics protocol automatically.

Explicit `unicode`, `kitty`, and `sixel` selections require TTY stdout and skip
detection. An incompatible redirected selection fails before any ANSI, Kitty,
or Sixel escape is written. `NO_COLOR` turns Unicode truecolor output into
monochrome block text; it does not authorize or select a graphics protocol.
Raster dimensions, graphics payload limits, Sixel palette size, encoder state,
and projection storage are checked as applicable before output. The raster
defaults to 80×48 pixels, which Unicode packs into 80 columns by 24 rows.
`--palette-limit` defaults to 256 colors for Sixel; `--payload-limit` defaults
to 64 MiB for Kitty and Sixel. Both graphics backends have a fixed 4096×4096
ceiling. `--memory-limit` defaults to 512 MiB and bounds managed Source,
projection, raster, and encoder memory.

## Streams and exit status

- Human diagnostics and progress use stderr.
- Data and `--json` results use stdout.
- `pcx render` writes its single rendered frame to stdout; redirected automatic
  output contains no terminal control sequences.
- Successful `--json` output uses stdout. Failures from a successfully parsed JSON command leave stdout empty and write a versioned JSON error object to stderr.
- The JSON schemas and compatibility policy are published in [`docs/json-schema`](https://github.com/takeshiD/pcx/tree/main/docs/json-schema). Human-readable output and diagnostic message wording are not compatibility contracts.
- Success is `0`; usage errors, invalid data and resource refusal are non-zero.
- Interrupt handling removes temporary output and returns `130`.
- Existing output is rejected unless `--force` is explicit.
