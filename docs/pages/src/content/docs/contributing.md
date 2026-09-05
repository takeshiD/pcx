---
title: Contributing
description: Local development and change expectations.
---

Use the Nix development shell for the reproducible toolchain, or install the pinned Rust toolchain directly.

```bash
nix develop
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo check --all-targets --all-features
cargo test --all-features
./scripts/generate-assets.sh --check
nix flake check
```

Changes should preserve bounded memory, explicit fidelity and binary-safe streams. Add the smallest fixture that demonstrates new parser behavior and update an ADR when a durable architectural decision changes.

After changing the clap grammar or package version, run
`./scripts/generate-assets.sh` and review the updated files under `generated/`.
CI detects stale assets but does not rewrite them.

Read the repository [contribution guide](https://github.com/takeshiD/pcx/blob/main/CONTRIBUTING.md) before opening a pull request.
