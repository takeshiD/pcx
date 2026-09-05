---
title: Install
description: 現在利用できるsource／Nix install方法。
---

## crates.ioから

```bash
cargo install pcx-cli
pcx --version
```

## Sourceから

```bash
git clone https://github.com/takeshiD/pcx.git
cd pcx
cargo install --path . --locked
pcx --version
```

## Nix

```bash
nix run github:takeshiD/pcx -- --version
nix develop github:takeshiD/pcx
```

## 対応target

| Target | Support |
| --- | --- |
| x86_64 Linux | native buildと全test |
| aarch64 Linux | native buildと全test |
| macOS | 非対応・将来未定 |
| Windows | 非対応・将来未定 |
