---
title: リリース
description: 厳密なタグ、検証ゲート、公開先。
---

自動リリースは `vX.Y.Z` または `vX.Y.Z-alpha.N` に厳密一致するタグだけを受け付けます。タグのバージョンは `Cargo.toml` と完全一致しなければなりません。

公開前に formatter、Clippy、typecheck、test、ドキュメント build、Nix check、`cargo package`、展開後 package の build、install smoke test を再実行します。

保護された GitHub `release` environment で承認後、`pcx-cli` を crates.io に公開し、x86_64/aarch64 Linux の closure を公開 Cachix `pcx` に push し、tarball と SHA-256 を GitHub Release に追加します。必要な secret は `CARGO_REGISTRY_TOKEN` と `CACHIX_AUTH_TOKEN` です。
