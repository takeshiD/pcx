# Release Process

Stable releases use `vX.Y.Z`; pre-releases use `vX.Y.Z-alpha.N`. No other tag starts publication.

## Prerequisites

- The `pcx-cli` package version exactly matches the tag without its leading `v`.
- The tagged commit is on `main` and has passed CI.
- The public Cachix cache is `takeshid` at `https://takeshid.cachix.org`, with public key `takeshid.cachix.org-1:2GsGTUZ3djVzbGzXgeia+SRV1ZJYOXySHyNfBPsEjRA=`.
- GitHub's protected `release` environment requires manual approval.
- `CACHIX_AUTH_TOKEN` is configured only for that environment and can write to `takeshid`.
- For automated releases, crates.io Trusted Publishing identifies repository `takeshiD/pcx`, workflow `release.yml`, and environment `release`; GitHub stores no crates.io token.

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
  -> push both closures to takeshid Cachix
  -> exchange GitHub OIDC for a short-lived crates.io token
  -> cargo publish pcx-cli with the temporary token
  -> GitHub Release and checksums
```

`id-token: write` is scoped to the protected publish job. Pull-request and validation jobs use `takeshid` read-only and receive no publishing credential. `cargo publish --no-verify` and `--allow-dirty` are forbidden. crates.io publication is permanent, so it follows every recoverable build/cache operation.

## First crates.io publication

Trusted Publishing can be configured only after `pcx-cli` exists on crates.io. A maintainer therefore publishes the first version manually from the exact clean `main` commit after all equivalent CI, package, Nix, documentation, and approval gates pass. Do not create an automated release tag for that bootstrap version.

After the first publication, configure the crate's GitHub Trusted Publisher with owner `takeshiD`, repository `pcx`, workflow filename `release.yml`, and environment `release`. Every later release uses a new manifest version and the automated tag workflow; do not add `CARGO_REGISTRY_TOKEN` to GitHub.

## Artifacts

- `pcx-vX.Y.Z-x86_64-linux.tar.xz`
- `pcx-vX.Y.Z-aarch64-linux.tar.xz`
- `SHA256SUMS`

Each archive contains the `pcx` executable, `LICENSE`, `README.md`, shell completions under `share/{bash-completion,zsh,fish}`, and `pcx` manual pages under `share/man/man1`. Alpha tags create a GitHub pre-release and publish the matching Cargo pre-release version.

The supported archives are GNU-linked.
[ADR-0015](./adr/0015-keep-gnu-linux-release-artifacts.md) keeps them in place
until the exact static musl release candidates pass full-feature tests on native
x86_64 and native aarch64 Linux, representative target smoke tests, and an
explicitly accepted size and runtime comparison. A cross-build or emulated run
alone is not native support evidence.

## Recovery

If Cachix succeeds but the OIDC exchange or crates.io publication fails, fix only the Trusted Publisher or release configuration and rerun the same tag workflow after verifying the registry does not contain the version. Never move a published tag to different source. A crates.io version cannot be overwritten; a broken published version is yanked and replaced with a new patch/pre-release version.
