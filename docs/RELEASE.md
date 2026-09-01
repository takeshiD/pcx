# Release Process

Stable releases use `vX.Y.Z`; pre-releases use `vX.Y.Z-alpha.N`. No other tag starts publication.

## Prerequisites

- The `pcx-cli` package version exactly matches the tag without its leading `v`.
- The tagged commit is on `main` and has passed CI.
- The public Cachix cache is named `pcx`.
- GitHub's protected `release` environment requires manual approval.
- `CARGO_REGISTRY_TOKEN` and `CACHIX_AUTH_TOKEN` are configured only for that environment.

## Automated gate

```text
tag syntax and Cargo version
  -> fmt, clippy, check, test
  -> x86_64 and aarch64 native build/test
  -> cargo package --locked
  -> inspect package contents and size
  -> install the packaged source
  -> packaged pcx --version smoke test
  -> Nix flake check and both packages
  -> Starlight build
  -> protected environment approval
  -> Cachix push
  -> cargo publish pcx-cli
  -> GitHub Release and checksums
```

`cargo publish --no-verify` and `--allow-dirty` are forbidden. crates.io publication is permanent, so it follows every recoverable build/cache operation.

## Artifacts

- `pcx-vX.Y.Z-x86_64-linux.tar.xz`
- `pcx-vX.Y.Z-aarch64-linux.tar.xz`
- `SHA256SUMS`

Each archive contains the `pcx` executable, `LICENSE`, and a minimal README. Alpha tags create a GitHub pre-release and publish the matching Cargo pre-release version.

## Recovery

If Cachix succeeds but crates.io fails, fix only the release configuration and rerun the same tag workflow after verifying the registry does not contain the version. Never move a published tag to different source. A crates.io version cannot be overwritten; a broken published version is yanked and replaced with a new patch/pre-release version.
