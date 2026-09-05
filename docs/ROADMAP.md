# pcx implementation roadmap

The backlog is tracked in [GitHub Issues](https://github.com/takeshiD/pcx/issues).
Milestones describe delivery order, not calendar commitments. Issue number 3 is
unused because GitHub returned an error during initial backlog creation.

## Foundation

- [#6 Protected delivery environments and repository settings](https://github.com/takeshiD/pcx/issues/6)

## v0.1 — Inspect & Extract

- [#1 Core job, plan, report, and error types](https://github.com/takeshiD/pcx/issues/1)
- [#7 Strict preflight resource planning](https://github.com/takeshiD/pcx/issues/7)
- [#2 Bounded synchronous MCAP source and probe](https://github.com/takeshiD/pcx/issues/2)
- [#5 `pcx info` human and JSON output](https://github.com/takeshiD/pcx/issues/5)
- [#4 `pcx topics` discovery](https://github.com/takeshiD/pcx/issues/4)
- [#8 Checked CDR cursor for ROS 2](https://github.com/takeshiD/pcx/issues/8)
- [#9 ROS 2 PointCloud2 validation](https://github.com/takeshiD/pcx/issues/9)
- [#10 PointView and PointBatch ownership models](https://github.com/takeshiD/pcx/issues/10)
- [#11 Deterministic topic frame selection](https://github.com/takeshiD/pcx/issues/11)
- [#12 Binary and ASCII PCD writer](https://github.com/takeshiD/pcx/issues/12)
- [#13 Atomic file and binary-safe stdout sinks](https://github.com/takeshiD/pcx/issues/13)
- [#14 One-frame MCAP-to-PCD extraction](https://github.com/takeshiD/pcx/issues/14)
- [#15 Valid and malformed format fixtures](https://github.com/takeshiD/pcx/issues/15)
- [#16 Parser properties and fuzz harnesses](https://github.com/takeshiD/pcx/issues/16)

## v0.2 — Reduction

- [#17 Field selection](https://github.com/takeshiD/pcx/issues/17)
- [#18 Frame-local crop](https://github.com/takeshiD/pcx/issues/18)
- [#19 Frame-local statistics](https://github.com/takeshiD/pcx/issues/19)
- [#20 Frame-local voxel reduction](https://github.com/takeshiD/pcx/issues/20)
- [#21 Operator capability and memory contracts](https://github.com/takeshiD/pcx/issues/21)

## v0.3 — Formats

- [#22 Strict PCD reader](https://github.com/takeshiD/pcx/issues/22)
- [#23 MCAP output and container passthrough](https://github.com/takeshiD/pcx/issues/23)
- [#24 PLY I/O](https://github.com/takeshiD/pcx/issues/24)
- [#25 LAS and LAZ I/O](https://github.com/takeshiD/pcx/issues/25)

## v0.4 — Terminal

- [#26 Deterministic CPU projection](https://github.com/takeshiD/pcx/issues/26)
- [#27 Unicode rendering](https://github.com/takeshiD/pcx/issues/27)
- [#28 Kitty graphics rendering](https://github.com/takeshiD/pcx/issues/28) — implemented encoder
- [#29 Capability detection and fallback](https://github.com/takeshiD/pcx/issues/29) — implemented policy seam
- [#30 Sixel rendering](https://github.com/takeshiD/pcx/issues/30)

## Hardening

- [#31 Performance and memory benchmarks](https://github.com/takeshiD/pcx/issues/31)
- [#32 Evaluate static musl artifacts](https://github.com/takeshiD/pcx/issues/32)
- [#33 Version and compatibility-test JSON](https://github.com/takeshiD/pcx/issues/33)
- [#34 Shell completions and manual pages](https://github.com/takeshiD/pcx/issues/34)

Cloud transport, AWS, S3, credential management, daemon, and GUI work are not
part of this roadmap.
