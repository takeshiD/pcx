# Publish through shared Cachix and crates.io OIDC

Status: accepted. Fully supersedes ADR-0011.

Only strict `vX.Y.Z` and `vX.Y.Z-alpha.N` tags whose version equals the `pcx-cli` manifest version enter automated release validation. Both native Linux architectures, packaged source, the installed packaged binary, the Nix flake, and documentation must pass before the protected `release` environment permits publication of both Nix closures to the existing public `takeshid` Cachix cache, publication to crates.io through a short-lived Trusted Publishing token, and creation of a GitHub Release with x86_64 and aarch64 archives and checksums; the Cachix write token remains an environment secret, `id-token: write` is granted only to the protected publish job, and pull-request jobs receive no publishing credential.

The public cache identity is `https://takeshid.cachix.org` with signing key `takeshid.cachix.org-1:2GsGTUZ3djVzbGzXgeia+SRV1ZJYOXySHyNfBPsEjRA=`. Because crates.io requires a package to exist before its Trusted Publisher can be configured, a maintainer performs the first `pcx-cli` publication manually only after the equivalent validation and approval gates; that bootstrap version is not an automated tagged release, and later versions use the `takeshiD/pcx` `release.yml` publisher bound to the `release` environment without a stored crates.io token.
