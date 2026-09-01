---
title: Installation
description: Current source and Nix installation paths.
---

:::caution[Foundation stage]
The first crates.io release has not happened. `cargo install pcx-cli` is the accepted future command, not a current availability claim.
:::

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

## Supported targets

| Target | Support |
| --- | --- |
| x86_64 Linux | Native build and full tests |
| aarch64 Linux | Native build and full tests |
| macOS | Not supported; future undecided |
| Windows | Not supported; future undecided |
