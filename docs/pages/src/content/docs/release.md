---
title: Release process
description: Strict tags, verification gates and publication targets.
---

Releases are automated only for exact tags:

- Stable: `vX.Y.Z`
- Prerelease: `vX.Y.Z-alpha.N`

The tag version must exactly match `Cargo.toml`. Before publication, CI repeats formatting, Clippy, type checking, tests, documentation build, Nix checks, `cargo package`, an extracted-package build and an install smoke test.

After approval in the protected GitHub `release` environment, the workflow publishes an existing `pcx-cli` package through crates.io Trusted Publishing, pushes Linux closures to the public `takeshid` Cachix cache, and creates a GitHub release with x86_64/aarch64 tarballs and SHA-256 checksums. Each tarball contains the executable, license, README, Bash/Zsh/Fish completions, and manual pages. The cache is `https://takeshid.cachix.org`, with public key `takeshid.cachix.org-1:2GsGTUZ3djVzbGzXgeia+SRV1ZJYOXySHyNfBPsEjRA=`.

The supported Linux tarballs remain GNU-linked. Static musl candidates do not
replace them until the exact release binaries pass the full feature-set suite on
native x86_64 and native aarch64 hardware, representative target smoke tests,
and an explicitly accepted size and runtime comparison. Cross-builds and
emulated runs do not establish native support.

The environment stores only `CACHIX_AUTH_TOKEN`. The protected publish job exchanges GitHub OIDC for a short-lived crates.io token; no long-lived registry token is stored. A maintainer publishes the first crate version manually after equivalent gates, then configures the Trusted Publisher for later automated releases. See the [release runbook](https://github.com/takeshiD/pcx/blob/main/docs/RELEASE.md).
