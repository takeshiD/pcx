---
title: Installation
description: Current source and Nix installation paths.
---

## From crates.io

```bash
cargo install pcx-cli
pcx --version
```

## From source

```bash
git clone https://github.com/takeshiD/pcx.git
cd pcx
cargo install --path . --locked
pcx --version
```

## With Nix

```bash
nix run github:takeshiD/pcx -- --version
nix develop github:takeshiD/pcx
```

Nix installs Bash, Zsh, and Fish completions in each shell's standard share
directory and installs the `pcx`, `pcx-info`, `pcx-topics`, `pcx-extract`, and
`pcx-passthrough` manual pages. Release archives carry the same files at:

```text
share/bash-completion/completions/pcx
share/zsh/site-functions/_pcx
share/fish/vendor_completions.d/pcx.fish
share/man/man1/pcx*.1
```

When extracting an archive outside a system prefix, source the Bash file or
add the matching Zsh/Fish directory to that shell's completion path. Add
`share/man` to `MANPATH` for the manual pages.

## Regenerating shell help assets

The committed files under `generated/` are generated from the actual clap
command grammar and package version. After changing either, regenerate them:

```bash
./scripts/generate-assets.sh
```

CI runs `./scripts/generate-assets.sh --check` and fails when the committed
completions or manual pages are stale. Review generated diffs alongside the
grammar change; they are not updated automatically in CI.

## Supported targets

| Target | Support |
| --- | --- |
| x86_64 Linux | Native build and full tests |
| aarch64 Linux | Native build and full tests |
| macOS | Not supported; future undecided |
| Windows | Not supported; future undecided |
