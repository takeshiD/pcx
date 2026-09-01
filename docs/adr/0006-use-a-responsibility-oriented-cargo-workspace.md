---
status: superseded by ADR-0010
---

# Use a responsibility-oriented Cargo workspace

`pcx` will follow the current `typua` repository shape: a virtual Cargo workspace contains responsibility-oriented packages for the CLI, core processing, MCAP, ROS 2 messages, and PCD, and the CLI links them into one `pcx` executable. Format packages depend on `pcx-core`, never the reverse; which workspace packages must be uploaded to a registry is a release-distribution concern and does not change these source boundaries.
