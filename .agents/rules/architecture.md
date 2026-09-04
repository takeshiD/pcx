# Architecture guardrails

These rules translate the accepted ADRs into implementation constraints. Apply them when planning, implementing, reviewing, or refactoring product code, formats, CLI behavior, build configuration, or release automation.

The ADRs remain authoritative. If a proposed change conflicts with a rule or its source ADR, pause implementation and write a new ADR that explicitly supersedes the old decision. Update this file in the same change after that ADR is accepted.

## Product and compatibility boundary

Sources: [ADR-0001](../../docs/adr/0001-prove-the-mcap-workflow-in-v0-1.md), [ADR-0008](../../docs/adr/0008-version-json-and-define-frame-selection.md), and [ADR-0009](../../docs/adr/0009-compose-with-transfer-tools-instead-of-cloud-clients.md).

- Keep the v0.1 releasable slice centered on MCAP information, channel listing, and extraction of one ROS 2 `PointCloud2` frame to PCD.
- Defer static format breadth, voxel reduction, terminal rendering, and LAS/LAZ support until that central slice is proven.
- Treat `inspect -> reduce -> transfer` as the product workflow. Compose transfer from shell tools rather than adding transport to `pcx`.
- Keep AWS, S3, object-storage clients, upload orchestration, and cloud credential handling outside the product.
- Treat only the CLI as a public compatibility surface during v0.x. Internal modules and Rust library APIs may evolve without a stability promise.
- Give every machine-readable command output an explicit schema version. Within one schema version, make additive changes only.
- Treat human-readable output as non-contractual.
- Select a point frame either by zero-based index after Topic selection or by the first MCAP log time at or after a duration from recording start. Keep these selectors mutually exclusive and report absence explicitly.

## Package and module boundaries

Sources: [ADR-0003](../../docs/adr/0003-separate-container-records-from-point-frames.md), [ADR-0005](../../docs/adr/0005-keep-the-processing-core-synchronous.md), [ADR-0010](../../docs/adr/0010-publish-one-rust-package-with-deep-modules.md), and [ADR-0012](../../docs/adr/0012-wrap-the-official-mcap-reader-and-decode-pointcloud2-strictly.md).

- Keep one publishable Rust package named `pcx-cli`, with library and binary targets and an installed executable named `pcx`.
- Keep core processing, MCAP, ROS 2 message handling, and PCD as deep modules behind `src/lib.rs`; preserve test seams without splitting them into separately published crates.
- Do not restore the multi-package workspace from superseded ADR-0006 unless a new ADR supersedes ADR-0010.
- Keep encoded container records separate from decoded point frames.
- Perform container-level channel and time selection without point decoding when records can be preserved directly.
- Enter the semantic point pipeline only after `PointCloud2` decoding. Apply temporal point operations per frame by default.
- Keep planning, container IO, decoding, operators, encoding, and local sinks synchronous and pull-based.
- Express backpressure through bounded batches and byte writes. Keep async runtimes and network adapters out of the execution path.
- Wrap the official Rust `mcap` crate's sans-IO reader with bounded synchronous `Read + Seek`; do not implement the MCAP container or buffer a whole recording.
- Keep Zstandard and LZ4 enabled in the single feature set used by Cargo, Nix, Cachix, and release binaries.
- Decode only ROS 2 `sensor_msgs/msg/PointCloud2` with a small strict CDR decoder. Do not add a ROS runtime or general dynamic message engine without a superseding ADR.

## Point representation and fidelity

Sources: [ADR-0002](../../docs/adr/0002-reject-unplannable-resource-and-data-loss-risks.md), [ADR-0003](../../docs/adr/0003-separate-container-records-from-point-frames.md), and [ADR-0004](../../docs/adr/0004-use-view-and-columnar-point-representations.md).

- Preserve point fields and temporal and spatial metadata. Never discard them silently.
- Require explicit user authorization for lossy conversion and temporary spooling.
- Support a low-copy view over reference-counted source bytes and an owned schema-driven columnar batch.
- Retain views for inspection and direct extraction when possible; materialize columns on demand for point operators.
- Judge semantic equivalence by field names, types, counts, values, timestamps, and frame identity, not padding or byte layout.
- When an operation changes point count, mark an organized cloud as explicitly unorganized.

## Resource and output safety

Sources: [ADR-0002](../../docs/adr/0002-reject-unplannable-resource-and-data-loss-risks.md) and [ADR-0007](../../docs/adr/0007-bound-managed-memory-and-commit-outputs-atomically.md).

- Treat `--memory-limit` as a hard contract for all memory managed by `pcx`, including batches, decoded columns, operator and encoder state, queued output, buffers, and spool indexes.
- Reject a job before execution when peak managed memory or a compatible loss policy cannot be proven. Leave whole-process RSS enforcement to the operating system.
- Write local output to a temporary file beside the destination and rename it atomically only after success.
- Require `--force` before replacing an existing output file.
- On interruption, remove temporary output and exit with status 130.
- Treat an expected downstream broken pipe as normal pipeline termination without noisy diagnostics.

## Release safety

Source: [ADR-0011](../../docs/adr/0011-gate-tagged-releases-before-irreversible-publication.md).

- Trigger release validation only for `vX.Y.Z` and `vX.Y.Z-alpha.N` tags, and require the tag version to equal the `pcx-cli` manifest version.
- Before irreversible publication, validate both native Linux architectures, packaged source, the installed packaged binary, the Nix flake, and documentation.
- Gate Cachix, crates.io, and GitHub Release publication behind a protected release environment.
- Publish x86_64 and aarch64 archives with checksums.
- Give workflows minimal permissions and keep release secrets unavailable to pull-request jobs.

## Completion check

Before completing an affected change, identify every section above that applies and verify the diff against its source ADR. A change is not complete while an applicable invariant is unverified or an ADR conflict is unresolved.
