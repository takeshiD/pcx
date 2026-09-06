---
title: コントリビューション
description: ローカル開発と変更時の原則。
---

再現可能な Nix 開発環境、または固定済み Rust toolchain を利用します。

```bash
nix develop
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo check --all-targets --all-features
cargo test --all-features
./scripts/generate-assets.sh --check
nix flake check
```

変更はメモリ上限、明示的な忠実性、バイナリ安全なストリームを維持してください。パーサ変更には最小 fixture を追加し、長期的な設計判断の変更には ADR を更新します。

clap grammarまたはpackage versionを変更した場合は`./scripts/generate-assets.sh`を
実行し、`generated/`以下の更新をreviewします。CIは古いassetを検出しますが、
自動では書き換えません。

Pull Request の前に[コントリビューションガイド](https://github.com/takeshiD/pcx/blob/main/CONTRIBUTING.md)を確認してください。
