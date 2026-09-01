---
title: Release process
description: Strict tags, verification gates and publication targets.
---

Releases are automated only for exact tags:

- Stable: `vX.Y.Z`
- Prerelease: `vX.Y.Z-alpha.N`

The tag version must exactly match `Cargo.toml`. Before publication, CI repeats formatting, Clippy, type checking, tests, documentation build, Nix checks, `cargo package`, an extracted-package build and an install smoke test.

After approval in the protected GitHub `release` environment, the workflow publishes the `pcx-cli` package to crates.io, pushes Linux closures to the public `pcx` Cachix cache, and creates a GitHub release with x86_64/aarch64 tarballs and SHA-256 checksums.

Maintainers configure `CARGO_REGISTRY_TOKEN` and `CACHIX_AUTH_TOKEN` as environment secrets. See the [release runbook](https://github.com/takeshiD/pcx/blob/main/docs/RELEASE.md).
