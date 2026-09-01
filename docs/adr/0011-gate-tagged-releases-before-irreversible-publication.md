# Gate tagged releases before irreversible publication

Only strict `vX.Y.Z` and `vX.Y.Z-alpha.N` tags trigger release validation, and the tag version must equal the `pcx-cli` manifest version. Both native Linux architectures, the packaged source, installed packaged binary, Nix flake, and documentation must pass before a protected release environment permits Cachix publication, crates.io publication, and creation of a GitHub Release with x86_64 and aarch64 archives and checksums; actions use minimal permissions and secrets are unavailable to pull-request jobs.
